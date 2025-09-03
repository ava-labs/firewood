// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::db::BatchOp;
use crate::merkle::{Merkle, Value};
use crate::v2::api::KeyValuePairIter;
use firewood_storage::{
    BranchNode, Child, FileBacked, FileIoError, ImmutableProposal, LeafNode, MutableProposal,
    NibblesIterator, Node, NodeStore, Parentable, Path,
};
use rayon::{ThreadPool, ThreadPoolBuilder};
use std::iter::once;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, mpsc};

#[derive(Debug)]
struct WorkerState {
    sender: mpsc::Sender<Request>,
}

/// A request to the worker.
#[derive(Debug)]
enum Request {
    Insert { key: Path, value: Value },
    Delete { key: Path },
    DeleteRange { prefix: Path },
    Done,
}

#[derive(Debug)]
enum Response<S> {
    // return the new root of the subtrie at the given nibble
    Root(u8, Box<NodeStore<MutableProposal, S>>),
    Error(FileIoError),
}

/// `ParallelMerkle` safely performs parallel modifications to a Merkle trie. It does this 
/// by creating a worker for each subtrie from the root, and allowing the the workers to 
/// perform inserts and removes to their subtries.
#[derive(Debug)]
pub struct ParallelMerkle {
    workers: [Option<WorkerState>; BranchNode::MAX_CHILDREN],
}

impl Default for ParallelMerkle {
    fn default() -> Self {
        Self::new()
    }
}

impl ParallelMerkle {
    /// Default constructor
    #[must_use]
    pub fn new() -> Self {
        ParallelMerkle {
            workers: [(); BranchNode::MAX_CHILDREN].map(|()| None),
        }
    }

    /// Performs the prepare phase to allow clean separation of the trie into an array
    /// of subtries that can be operated on independently by the worker threads.
    fn prepare_trie(&self, proposal: &mut NodeStore<MutableProposal, FileBacked>) {
        // Prepare phase:
        // --------------
        // There are 3 different cases to handle depending on the value of the root node.
        //
        // 1. If root is None, create a branch node with an empty partial path and a None for
        //    value. Create Nones for all of its children.
        // 2. If the existing root has a partial path, then create a new root with an empty
        //    partial path and a None for a value. Push down the previous root as a child. Note
        //    that this modified Merkle trie is not currently valid and may need to be updated
        //    during the post-processing phase.
        // 3. If the existing root does not have a partial path, then there is nothing we need
        //    to do if it is a branch. If it is a leaf, then convert it into a branch.
        //
        // The result after the prepare phase is that there is a branch node at the root with
        // an empty partial path.
        let root_node = proposal.mut_root().take();
        if let Some(mut node) = root_node {
            // Non-empty root. Check if it has a partial path
            let index_path_opt: Option<(u8, Path)> = node
                .partial_path()
                .split_first()
                .map(|(index, path)| (*index, path.into()));

            // If index_path_opt is not None, then create a new branch that will be the new root with
            // the previous root as the child at the index returned from split_first.
            if let Some((child_index, child_path)) = index_path_opt {
                let mut branch = BranchNode {
                    partial_path: Path::new(),
                    value: None,
                    children: BranchNode::empty_children(),
                };
                node.update_partial_path(child_path);
                branch.update_child(child_index, Some(Child::Node(node)));
                *proposal.mut_root() = Some(branch.into());
            } else {
                // Root has an empty partial path. We need to consider two cases.
                match node {
                    Node::Leaf(mut leaf) => {
                        // Root is a leaf with an empty partial path. Replace it with a branch.
                        let branch = BranchNode {
                            partial_path: Path::new(),
                            value: Some(std::mem::take(&mut leaf.value)),
                            children: BranchNode::empty_children(),
                        };
                        *proposal.mut_root() = Some(branch.into());
                    }
                    Node::Branch(_) => {
                        // Root does not need to be updated since it has an empty partial path and is a
                        // branch. Put it back into the proposal.
                        *proposal.mut_root() = Some(node);
                    }
                }
            }
        } else {
            // Empty trie. Create a branch node with an empty partial path and a None for a value.
            let branch = BranchNode {
                partial_path: Path::new(),
                value: None,
                children: BranchNode::empty_children(),
            };
            *proposal.mut_root() = Some(branch.into());
        }
    }

    fn postprocess_trie(
        &self,
        nodestore: &mut NodeStore<MutableProposal, FileBacked>,
        mut new_root: Node,
    ) -> Result<Option<Node>, FileIoError> {
        // Check if the Merkle trie has an extra root node that was added to facilitate efficient
        // parallel modification of the trie. If it does, apply transform to return trie to a
        // valid state by following the steps below:
        //
        // If all of the children from the root has been removed, then
        //     Convert the root into a leaf if it has a value (this should indicate that a
        //     value was inserted for the empty key).
        //     Otherwise, delete the empty root node
        // If more than one child remains, then do nothing
        // If only one child remains and the root has a value, then do nothing
        // If only one child remains and the root doesn’t have a value, then deleted the root,
        // use the child node as the new root, and update the partial path of the new root to
        // include its previous child index

        //let Some(mut new_root) = new_root_opt else {
        //    return Ok(None);
        //};

        // Should always be a branch given the prepare phase.
        let Node::Branch(branch) = &mut new_root else {
            return Ok(None); // TODO: Return some type of error
        };

        let mut children_iter = branch
            .children
            .iter_mut()
            .enumerate()
            .filter_map(|(index, child)| child.as_mut().map(|child| (index, child)));

        let first_child = children_iter.next();
        match first_child {
            None => {
                if let Some(value) = branch.value.take() {
                    // There is a value for the empty key. Create a leaf with the value and return.
                    let new_leaf = Node::Leaf(LeafNode {
                        value,
                        partial_path: Path::new(), // Partial path should be empty
                    });
                    return Ok(Some(new_leaf));
                }
                Ok(None) // No value. Just delete the root
            }
            Some((child_index, child)) => {
                // Check if the root has a value or if there is more than one children. If yes, then
                // just return the root unmodified
                if branch.value.is_some() || children_iter.next().is_some() {
                    return Ok(Some(new_root));
                }

                // Return the child as the new root. Up its partial path to include the index value.
                let mut child = match child {
                    Child::Node(child_node) => std::mem::take(child_node),
                    Child::AddressWithHash(addr, _) => nodestore.read_for_update((*addr).into())?,
                    Child::MaybePersisted(maybe_persisted, _) => {
                        nodestore.read_for_update(maybe_persisted.clone())?
                    }
                };

                // The child's partial path is the concatenation of its (now removed) parent,
                // which should always be empty because of our prepare step, its (former)
                // child index, and its partial path. Because the parent's partial path
                // should always be empty, we replace it here with a Path::new().
                let partial_path = Path::from_nibbles_iterator(
                    Path::new()
                        .iter()
                        .copied()
                        .chain(once(child_index as u8))
                        .chain(child.partial_path().iter().copied()),
                );

                match child {
                    Node::Branch(ref mut child_branch) => {
                        child_branch.partial_path = partial_path;
                    }
                    Node::Leaf(ref mut leaf) => {
                        leaf.partial_path = partial_path;
                    }
                }
                Ok(Some(child))
            }
        }
    }

    fn create_worker(
        pool: &ThreadPool,
        proposal: &NodeStore<MutableProposal, FileBacked>,
        root_branch: &mut Box<BranchNode>,
        first_nibble: u8,
        worker_sender: Sender<Response<FileBacked>>,
    ) -> Result<WorkerState, FileIoError> {
        // No worker state for this nibble yet, so create one.
        // Create a channel for the coordinator (main thread) to send messages to this worker.
        let child_channel = mpsc::channel();

        // get the child node from the proposal
        let child = root_branch
            .children
            .get_mut(first_nibble as usize)
            .expect("index error")
            .take();

        // build a nodestore from the child node
        let worker_nodestore = NodeStore::from_child(proposal, child)?;

        // The worker will send messages to the coordinator using the response channel
        // now tell the threadpool to spawn a worker for this nibble
        pool.spawn(move || {
            let mut merkle = Merkle::from(worker_nodestore);
            loop {
                // Wait for a message on the receiver child channel. Break out of loop if there is an error.
                let request = match child_channel.1.recv() {
                    Ok(r) => r,
                    Err(_err) => break, // Exit recv loop
                };
                match request {
                    // insert a key-value pair into the subtrie
                    Request::Insert { key, value } => {
                        if let Err(err) = merkle.insert_path(key, value.as_ref().into()) {
                            worker_sender
                                .send(Response::Error(err))
                                .expect("send error");
                        }
                    }
                    Request::Delete { key } => {
                        if let Err(err) = merkle.remove_path(key) {
                            worker_sender
                                .send(Response::Error(err))
                                .expect("send error");
                        }
                    }
                    Request::DeleteRange { prefix } => {
                        if let Err(err) = merkle.remove_prefix_path(prefix) {
                            worker_sender
                                .send(Response::Error(err))
                                .expect("send error");
                        }
                    }
                    // sent from the coordinator to the workers to signal that they are done
                    Request::Done => {
                        worker_sender
                            .send(Response::Root(first_nibble, merkle.into_inner().into()))
                            .expect("send error");
                        break;
                    }
                }
            }
        });
        Ok(WorkerState {
            sender: child_channel.0,
        })
    }

    fn end_batch(
        &mut self,
        response_channel: Receiver<Response<FileBacked>>,
        proposal: &mut NodeStore<MutableProposal, FileBacked>,
        root_branch: &mut Box<BranchNode>,
    ) -> Result<(), FileIoError> {
        // We have processed all the ops in the batch, so send a Done message to each worker
        for worker in self.workers.iter().flatten() {
            worker
                .sender
                .send(Request::Done)
                .expect("send to worker error");
        }

        while let Ok(response) = response_channel.recv() {
            match response {
                Response::Root(index, mut child_nodestore) => {
                    // Taking deleted nodes (from calling read_for_update) from child nodestores.
                    proposal.add_deleted_nodes_from_child(&mut child_nodestore);

                    // Set the child at index using the root from the child nodestore.
                    *root_branch
                        .children
                        .get_mut(index as usize)
                        .expect("index error") = child_nodestore.into_root().map(Child::Node);
                }
                Response::Error(err) => {
                    return Err(err); // Early termination.
                }
            }
        }
        Ok(())
    }

    fn get_worker(
        &mut self,
        pool: &ThreadPool,
        proposal: &NodeStore<MutableProposal, FileBacked>,
        root_branch: &mut Box<BranchNode>,
        first_nibble: u8,
        worker_sender: Sender<Response<FileBacked>>,
    ) -> Result<&mut WorkerState, FileIoError> {
        // Find the worker's state corresponding to the first nibble which are stored in an array.
        let worker_option = self
            .workers
            .get_mut(first_nibble as usize)
            .expect("index out of bounds");

        // Create a new worker if it doesn't exist. Not using `get_or_insert_with` with worker_option
        // because it is possible to generate a FileIoError within the closure.
        match worker_option {
            Some(worker) => Ok(worker),
            None => Ok(worker_option.insert(ParallelMerkle::create_worker(
                pool,
                proposal,
                root_branch,
                first_nibble,
                worker_sender,
            )?)),
        }
    }

    fn remove_all_entries(&self, root_branch: &mut Box<BranchNode>) {
        for worker in self.workers.iter().flatten() {
            worker
                .sender
                .send(Request::DeleteRange {
                    prefix: Path::new(), // Empty prefix
                })
                .expect("TODO: handle error");
        }
        // Also set the root value to None but does not delete the root.
        root_branch.value = None;
    }

    /// Creating a parallel proposal consists of 4 phases: Prepare, Split, Merge, and Postprocess.
    #[allow(clippy::missing_panics_doc)]
    #[allow(clippy::missing_errors_doc)]
    //#[allow(clippy::too_many_lines)]
    pub fn create_proposal<T: Parentable>(
        &mut self,
        parent: &NodeStore<T, FileBacked>,
        batch: impl IntoIterator<IntoIter: KeyValuePairIter>,
        threadpool: &mut Option<ThreadPool>,
    ) -> Result<Arc<NodeStore<Arc<ImmutableProposal>, FileBacked>>, FileIoError> {
        // get (or create) a threadpool
        let pool = threadpool.get_or_insert_with(|| {
            ThreadPoolBuilder::new()
                .num_threads(BranchNode::MAX_CHILDREN)
                .build()
                .expect("Error in creating threadpool")
        });

        // create a proposal from the parent
        let mut proposal = NodeStore::new(parent)?;

        // Process trie in preparation for performing parallel modifications.
        self.prepare_trie(&mut proposal);

        let mut root_branch = proposal
            .mut_root()
            .take()
            .expect("Should have a root node after prepare phase")
            .into_branch()
            .expect("Root should be a branch after prepare phase");

        // Create a response channel the workers use to send messages back to the coordinator (us)
        let response_channel = mpsc::channel();

        // For each operation in the batch, send a request to the worker related to the first nibble
        for op in batch.into_iter().map_into_batch() {
            // Get the first nibble of the key to determine which worker to send the request to.
            //
            // Need to handle an empty key. Since the partial_path of the root must be empty, an 
            // empty key should always be for the root node. There are 3 cases the consider.
            //
            // Insert: The main thread modifies the value of the root.
            //
            // Remove: The main thread removes any value at the root. However, it should not delete
            //         the root node, which, if necessary later, will be done in post processing.
            //
            // Remove Prefix:
            //         For a remove prefix, we would need to remove everything. We do this by sending
            //         a remove prefix with an empty prefix to all of the children, then removing the
            //         value of the root node.
            let mut key_nibbles = NibblesIterator::new(op.key().as_ref());
            let Some(first_nibble) = key_nibbles.next() else {
                match &op {
                    BatchOp::Put { key: _, value } => {
                        root_branch.value = Some(value.as_ref().into());
                        /*
                        // There should always be a value, even if it is empty.
                        root_branch.value = Some(
                            op.value()
                                .as_ref()
                                .map(|v| v.as_ref().into())
                                .unwrap_or_default(),
                        );
                        */
                    }
                    BatchOp::Delete { key: _ } => {
                        root_branch.value = None;
                    }
                    BatchOp::DeleteRange { prefix: _ } => {
                        // Calling remove prefix with an empty prefix is equivalent to a remove all.
                        self.remove_all_entries(&mut root_branch);
                    }
                }
                continue; // Done with this empty key operation.
            };

            // We send a path to the worker without the first nibble. Sending a nibble iterator to a
            // worker is more difficult as it takes a reference from op.
            let key_path = Path::from_nibbles_iterator(key_nibbles);

            // Get the worker that is responsible for this nibble. The worker will be created if it
            // doesn't already exist.
            let worker = self.get_worker(
                pool,
                &proposal,
                &mut root_branch,
                first_nibble,
                response_channel.0.clone(),
            )?;

            // Send the current operation to the worker.
            match &op {
                BatchOp::Put { key: _, value } => {
                    worker
                        .sender
                        .send(Request::Insert {
                            key: key_path,
                            value: value.as_ref().into(),
                        })
                        .expect("send to worker error");
                    /*
                    worker
                        .sender
                        .send(Request::Insert {
                            key: key_path,
                            value: op
                                .value()
                                .as_ref()
                                .map(|v| v.as_ref().into())
                                .unwrap_or_default(),
                        })
                        .expect("send to worker error");
                    */
                }
                BatchOp::Delete { key: _ } => {
                    worker
                        .sender
                        .send(Request::Delete { key: key_path })
                        .expect("send to worker error");
                }
                BatchOp::DeleteRange { prefix: _ } => {
                    worker
                        .sender
                        .send(Request::DeleteRange { prefix: key_path })
                        .expect("send to worker error");
                }
            }
        }

        // Drop the sender response channel from the parent thread.
        drop(response_channel.0);

        self.end_batch(response_channel.1, &mut proposal, &mut root_branch)?;

        *proposal.mut_root() = self
            .postprocess_trie(&mut proposal, (*root_branch).into())?;

        // Done with these worker states. Setting the workers to None will allow the next create
        // proposal from the same ParallelMerkle to be reused to spawn new states.
        self.workers = [(); BranchNode::MAX_CHILDREN].map(|()| None);

        let immutable: Arc<NodeStore<Arc<ImmutableProposal>, FileBacked>> =
            Arc::new(proposal.try_into().expect("error creating immutable"));

        Ok(immutable)
    }
}
