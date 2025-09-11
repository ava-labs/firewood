// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::db::BatchOp;
use crate::merkle::{Key, Merkle, Value};
use crate::v2::api::KeyValuePairIter;
use firewood_storage::{
    BranchNode, Child, FileBacked, FileIoError, ImmutableProposal, LeafNode, MutableProposal,
    NibblesIterator, Node, NodeReader, NodeStore, Parentable, Path,
};
use rayon::{ThreadPool, ThreadPoolBuilder};
use std::iter::once;
use std::ops::Deref;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, OnceLock, mpsc};

#[derive(Debug)]
struct WorkerSender(mpsc::Sender<Request>);

impl std::ops::Deref for WorkerSender {
    type Target = mpsc::Sender<Request>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// A request to the worker.
#[derive(Debug)]
enum Request {
    Insert { key: Key, value: Value },
    Delete { key: Key },
    DeleteRange { prefix: Key },
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
    workers: [Option<WorkerSender>; BranchNode::MAX_CHILDREN],
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

    /// Normalize the root to allow clean separation of the trie into an array of subtries that
    /// can be operated on independently by the worker threads.
    fn normalize_root(&self, proposal: &mut NodeStore<MutableProposal, FileBacked>) {
        // There are 3 different cases to handle depending on the value of the root node.
        //
        // 1. If root is None, create a branch node with an empty partial path and a None for
        //    value. Create Nones for all of its children.
        // 2. If the existing root has a partial path, then create a new root with an empty
        //    partial path and a None for a value. Push down the previous root as a child. Note
        //    that this modified Merkle trie is not currently valid and may need to be updated
        //    during the post-processing step.
        // 3. If the existing root does not have a partial path, then there is nothing we need
        //    to do if it is a branch. If it is a leaf, then convert it into a branch.
        //
        // Cases 2 and 3 are handled by `normalize_for_insert`. The result after normalization
        // is that there is a branch node at the root with an empty partial path.
        let root_node = proposal.root_mut().take();
        if let Some(node) = root_node {
            *proposal.root_mut() = Some(node.normalize_for_insert());
        } else {
            // Empty trie. Create a branch node with an empty partial path and a None for a value.
            let branch = BranchNode {
                partial_path: Path::new(),
                value: None,
                children: BranchNode::empty_children(),
            };
            *proposal.root_mut() = Some(branch.into());
        }
    }

    /// After performing parallel modifications, it may be necessary to perform post processing
    /// to return the Merkle trie to the correct canonical form.
    fn postprocess_trie(
        &self,
        nodestore: &mut NodeStore<MutableProposal, FileBacked>,
        mut branch: Box<BranchNode>,
    ) -> Result<Option<Node>, FileIoError> {
        // Check if the Merkle trie has an extra root node. If it does, apply transform to
        // return trie to a valid state by following the steps below:
        //
        // If the root node has:
        // 0 children and no value, the trie is empty. Just delete the root.
        // 0 children and a value (from an empty key), the root should be a leaf
        // 1 child and no value, the child should be the root (need to update partial path)
        // In all other cases, the root is already correct.

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
                    Ok(Some(Node::Leaf(LeafNode {
                        value,
                        partial_path: Path::new(), // Partial path should be empty
                    })))
                } else {
                    Ok(None) // No value. Just delete the root
                }
            }
            Some((child_index, child)) => {
                // Check if the root has a value or if there is more than one child. If yes, then
                // just return the root unmodified
                if branch.value.is_some() || children_iter.next().is_some() {
                    return Ok(Some((*branch).into()));
                }

                // Return the child as the new root. Update its partial path to include the index value.
                let mut child = match child {
                    Child::Node(child_node) => std::mem::take(child_node),
                    Child::AddressWithHash(addr, _) => nodestore.read_for_update((*addr).into())?,
                    Child::MaybePersisted(maybe_persisted, _) => {
                        nodestore.read_for_update(maybe_persisted.clone())?
                    }
                };

                // The child's partial path is the concatenation of its (now removed) parent, which
                // should always be empty because of our prepare step, its (former) child index, and
                // its partial path. Because the parent's partial path should always be empty, we
                // can omit it and start with the `child_index`.
                let partial_path = Path::from_nibbles_iterator(
                    once(child_index as u8).chain(child.partial_path().iter().copied()),
                );
                child.update_partial_path(partial_path);
                Ok(Some(child))
            }
        }
    }

    /// Creates a worker for performing operations on a subtrie, with the subtrie being determined
    /// by the value of the `first_nibble`.
    fn create_worker(
        pool: &ThreadPool,
        proposal: &NodeStore<MutableProposal, FileBacked>,
        root_branch: &mut Box<BranchNode>,
        first_nibble: u8,
        worker_sender: Sender<Response<FileBacked>>,
    ) -> Result<WorkerSender, FileIoError> {
        // Create a channel for the coordinator (main thread) to send messages to this worker.
        let (child_sender, child_receiver) = mpsc::channel();

        // The root's child becomes the root node of the worker
        let child = root_branch
            .children
            .get_mut(first_nibble as usize)
            .expect("index error")
            .take();

        let child_root = child
            .map(|child| match child {
                Child::Node(node) => Ok(node),
                Child::AddressWithHash(address, _) => {
                    Ok(proposal.read_node(address)?.deref().clone())
                }
                Child::MaybePersisted(maybe_persisted, _) => {
                    Ok(maybe_persisted.as_shared_node(proposal)?.deref().clone())
                }
            })
            .transpose()?;

        // Build a nodestore from the child node
        let worker_nodestore = NodeStore::from_root(proposal, child_root);

        // Spawn a worker from the threadpool for this nibble. The worker will send messages to the coordinator
        // using `worker_sender`.
        pool.spawn(move || {
            let mut merkle = Merkle::from(worker_nodestore);

            // Wait for a message on the receiver child channel. Break out of loop if there is an error.
            while let Ok(request) = child_receiver.recv() {
                match request {
                    // insert a key-value pair into the subtrie
                    Request::Insert { key, value } => {
                        let mut nibbles_iter = NibblesIterator::new(&key);
                        nibbles_iter.next(); // Skip the first nibble
                        if let Err(err) =
                            merkle.insert_from_iter(nibbles_iter, value.as_ref().into())
                        {
                            worker_sender
                                .send(Response::Error(err))
                                .expect("send from worker error");
                        }
                    }
                    Request::Delete { key } => {
                        let mut nibbles_iter = NibblesIterator::new(&key);
                        nibbles_iter.next(); // Skip the first nibble
                        if let Err(err) = merkle.remove_from_iter(nibbles_iter) {
                            worker_sender
                                .send(Response::Error(err))
                                .expect("send from worker error");
                        }
                    }
                    Request::DeleteRange { prefix } => {
                        let mut nibbles_iter = NibblesIterator::new(&prefix);
                        nibbles_iter.next(); // Skip the first nibble
                        if let Err(err) = merkle.remove_prefix_from_iter(nibbles_iter) {
                            worker_sender
                                .send(Response::Error(err))
                                .expect("send from worker error");
                        }
                    }
                    // Sent from the coordinator to the workers to signal that the batch is done.
                    Request::Done => {
                        worker_sender
                            .send(Response::Root(first_nibble, merkle.into_inner().into()))
                            .expect("send from worker error");
                        break; // Allow the worker to return to the thread pool.
                    }
                }
            }
        });
        Ok(WorkerSender(child_sender))
    }

    // Send a done message to all of the workers. Collect the responses, each representing the root of a
    // subtrie, and merge them into the root node of the main trie.
    fn merge_children(
        &mut self,
        response_channel: Receiver<Response<FileBacked>>,
        proposal: &mut NodeStore<MutableProposal, FileBacked>,
        root_branch: &mut Box<BranchNode>,
    ) -> Result<(), FileIoError> {
        // We have processed all the ops in the batch, so send a Done message to each worker
        for worker in self.workers.iter().flatten() {
            worker.send(Request::Done).expect("send to worker error");
        }

        while let Ok(response) = response_channel.recv() {
            match response {
                Response::Root(index, child_nodestore) => {
                    // Adding deleted nodes (from calling read_for_update) from child nodestores.
                    proposal.delete_nodes(child_nodestore.deleted_as_slice());

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

    /// Get a worker from the worker pool based on the `first_nibble` value. Create a worker if
    /// it doesn't exist already.
    fn get_worker(
        &mut self,
        pool: &ThreadPool,
        proposal: &NodeStore<MutableProposal, FileBacked>,
        root_branch: &mut Box<BranchNode>,
        first_nibble: u8,
        worker_sender: Sender<Response<FileBacked>>,
    ) -> Result<&mut WorkerSender, FileIoError> {
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

    /// Removes all of the entries in the trie. For the root entry, the value is removed but the
    /// root itself will remain. An empty root will only be removed during post processing.
    fn remove_all_entries(&self, root_branch: &mut Box<BranchNode>) {
        for worker in self.workers.iter().flatten() {
            worker
                .send(Request::DeleteRange {
                    prefix: Box::default(), // Empty prefix
                })
                .expect("TODO: handle error");
        }
        // Also set the root value to None but does not delete the root.
        root_branch.value = None;
    }

    /// Creates a parallel proposal in 4 steps: Prepare, Split, Merge, and Post-process. In the
    /// Prepare step, the trie is modified to ensure that the root is a branch node with no
    /// partial path. In the split step, entries from the batch are sent to workers that
    /// independently modify their sub-tries. In the merge step, the sub-tries are merged back
    /// to the main trie. Finally, in the post-processing step, the trie is returned to its
    /// canonical form.
    ///
    /// # Errors
    ///
    /// Can return a `FileIoError` when it fetches nodes from storage.
    ///
    /// # Panics
    ///
    /// Panics on errors when sending or receiving messages from workers. Can also panic if the
    /// thread pool cannot be created, or logic errors regarding the state of the trie.
    pub fn create_proposal<T: Parentable>(
        &mut self,
        parent: &NodeStore<T, FileBacked>,
        batch: impl IntoIterator<IntoIter: KeyValuePairIter>,
        threadpool: &OnceLock<ThreadPool>,
    ) -> Result<Arc<NodeStore<Arc<ImmutableProposal>, FileBacked>>, FileIoError> {
        // Get (or create) a threadpool
        let pool = threadpool.get_or_init(|| {
            ThreadPoolBuilder::new()
                .num_threads(BranchNode::MAX_CHILDREN)
                .build()
                .expect("Error in creating threadpool")
        });

        // Create a proposal from the parent
        let mut proposal = NodeStore::new(parent)?;

        // Prepare step: process trie in preparation for performing parallel modifications.
        self.normalize_root(&mut proposal);

        let mut root_branch = proposal
            .root_mut()
            .take()
            .expect("Should have a root node after prepare step")
            .into_branch()
            .expect("Root should be a branch after prepare step");

        // Create a response channel the workers use to send messages back to the coordinator (us)
        let response_channel = mpsc::channel();

        // Split step: for each operation in the batch, send a request to the worker that is
        // responsible for the sub-trie corresponding to the operation's first nibble.
        for op in batch.into_iter().map_into_batch() {
            // Get the first nibble of the key to determine which worker to send the request to.
            //
            // Need to handle an empty key. Since the partial_path of the root must be empty, an
            // empty key should always be for the root node. There are 3 cases to consider.
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
                        .send(Request::Insert {
                            key: op.key().as_ref().into(),
                            value: value.as_ref().into(),
                        })
                        .expect("send to worker error");
                }
                BatchOp::Delete { key: _ } => {
                    worker
                        .send(Request::Delete {
                            key: op.key().as_ref().into(),
                        })
                        .expect("send to worker error");
                }
                BatchOp::DeleteRange { prefix: _ } => {
                    worker
                        .send(Request::DeleteRange {
                            prefix: op.key().as_ref().into(),
                        })
                        .expect("send to worker error");
                }
            }
        }

        // Drop the sender response channel from the parent thread.
        drop(response_channel.0);

        // Merge step: send a done message to all of the workers to indicate that the batch is complete.
        // Collect the results from the workers and merge them as children to the root.
        self.merge_children(response_channel.1, &mut proposal, &mut root_branch)?;

        // Post-process step: return the trie to its canonical form.
        *proposal.root_mut() = self.postprocess_trie(&mut proposal, root_branch)?;

        // Done with these worker senders. Setting the workers to None will allow the next create
        // proposal from the same ParallelMerkle to reuse the thread pool.
        self.workers = [(); BranchNode::MAX_CHILDREN].map(|()| None);

        let immutable: Arc<NodeStore<Arc<ImmutableProposal>, FileBacked>> =
            Arc::new(proposal.try_into().expect("error creating immutable"));

        Ok(immutable)
    }
}
