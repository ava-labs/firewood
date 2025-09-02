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
use std::sync::{Arc, OnceLock, mpsc};

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

//static THREADPOOL: OnceLock<ThreadPool> = OnceLock::new();

/// TODO add doc
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

    fn postprocess_trie(
        &self,
        nodestore: &mut NodeStore<MutableProposal, FileBacked>,
        new_root_opt: Option<Node>,
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
        println!("In postprocess_trie");

        let Some(mut new_root) = new_root_opt else {
            return Ok(None);
        };

        // Should always be a branch given the prepare phase.
        let Node::Branch(branch) = &mut new_root else {
            return Ok(None); // TODO: Return some type of error
        };

        println!("Root is a branch");
        println!("Root node: {branch:?}");
        let mut children_iter = branch
            .children
            .iter_mut()
            .enumerate()
            .filter_map(|(index, child)| child.as_mut().map(|child| (index, child)));

        let first_child = children_iter.next();
        match first_child {
            None => {
                println!("First child is None");
                if let Some(value) = branch.value.take() {
                    // There is a value for the empty key. Create a leaf with the value and return.
                    let new_leaf = Node::Leaf(LeafNode {
                        value,
                        partial_path: Path::new(), // Partial path should be empty
                                                   //partial_path: branch.partial_path.clone(),
                    });
                    return Ok(Some(new_leaf));
                }
                Ok(None) // No value. Just delete the root
            }
            Some((child_index, child)) => {
                // Check if the root has a value or if there is more than one children. If yes, then
                // just return the root unmodified
                if branch.value.is_some() || children_iter.next().is_some() {
                    println!("root has a value or has more than one child");
                    return Ok(Some(new_root));
                }

                // Return the child as the new root. Need to update its partial path to include the
                // index value. Copied from remove_helper. Should move to a shared function to
                // increase code reuse.
                //
                // The branch's only child becomes the root of this subtrie.
                println!("root only has one child");
                let mut child = match child {
                    Child::Node(child_node) => std::mem::take(child_node),
                    Child::AddressWithHash(addr, _) => nodestore.read_for_update((*addr).into())?,
                    Child::MaybePersisted(maybe_persisted, _) => {
                        nodestore.read_for_update(maybe_persisted.clone())?
                    }
                };

                println!("Child's partial path: {:?}", child.partial_path());
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
                println!("Old root: {branch:?} New root: {child:?}");
                Ok(Some(child))
            }
        }
    }

    /// TODO: add doc
    #[allow(clippy::missing_panics_doc)]
    #[allow(clippy::missing_errors_doc)]
    #[allow(clippy::too_many_lines)]
    pub fn create_proposal<T: Parentable>(
        &mut self,
        parent: &NodeStore<T, FileBacked>,
        batch: impl IntoIterator<IntoIter: KeyValuePairIter>,
        threadpool: &mut OnceLock<ThreadPool>,
    ) -> Result<Arc<NodeStore<Arc<ImmutableProposal>, FileBacked>>, FileIoError> {
        // get (or create) a threadpool
        let pool = threadpool.get_or_init(|| {
            ThreadPoolBuilder::new()
                .num_threads(BranchNode::MAX_CHILDREN)
                .build()
                .expect("TODO: handle error")
        });

        // create a proposal from the parent
        let mut proposal = NodeStore::new(parent)?;

        // Creating a parallel proposal consists of 4 phases: Prepare, Split, Merge, and Postprocess.
        //
        // Prepare phase:
        // --------------
        // There are 3 different cases to handle depending on the value of the root node.
        //
        // 1. If root is None, create a branch node with an empty partial path and a None for
        //    value. Create Nones for all of its children.
        // 2. If the existing root has a partial path, then create a new root with an empty
        //    partial path and a None for a value. Push down the previous root as a child. This
        //    will be a malformed Merkle trie and will need to be fixed afterwards.
        // 3. If the existing root does not have a partial path, then there is nothing we need
        //    to do.
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

            println!("Split index from non empty root: {index_path_opt:?}");

            // If index_path_opt is not None, then create a new branch that will be the new root with
            // the previous root as the child at the index returned from split_first.
            if let Some((child_index, child_path)) = index_path_opt {
                println!(
                    "Creating empty branch node for non-empty trie. child_index: {child_index:?} child_path: {child_path:?}"
                );
                let mut branch = BranchNode {
                    partial_path: Path::new(),
                    value: None,
                    children: BranchNode::empty_children(),
                };
                node.update_partial_path(child_path);
                branch.update_child(child_index, Some(Child::Node(node)));

                println!("New root: {branch:?}");
                *proposal.mut_root() = Some(branch.into());
            } else {
                // Root does not need to be updated since it has an empty partial path. Put it
                // back into the proposal.
                *proposal.mut_root() = Some(node);
            }
        } else {
            // Create a branch node with an empty partial path and a None for a value
            println!("Creating empty branch node for empty trie");
            let branch = BranchNode {
                partial_path: Path::new(),
                value: None,
                children: BranchNode::empty_children(),
            };
            *proposal.mut_root() = Some(branch.into());
        }

        let root_node = proposal
            .mut_root()
            .take()
            .expect("Should have a root node after transform");
        let mut root_branch = root_node.into_branch().expect("Should be branch");

        // Create a response channel the workers use to send messages back to the coordinator (us)
        let response_channel = mpsc::channel();

        // For each operation in the batch, send a request to the worker related to the first nibble
        for op in batch.into_iter().map_into_batch() {
            println!("Key: {:?}", op.key().as_ref());

            let mut key_nibbles = NibblesIterator::new(op.key().as_ref());

            // Get the first nibble of the key to determine which worker to send the request to.
            //
            // Need to handle empty key. Since the partial_path of the root must be empty, an empty
            // key should always be for the root node. There are 3 cases the consider.
            //
            // Insert: The main thread modifies the value of the root.
            //
            // Remove: The main thread removes any value at the root. However, it should not delete
            //         the root node, which, if necessary later, will be done in post processing.
            //
            // Remove Prefix:
            //         For a remove prefix, we would need to remove everything, which cannot safely
            //         be done in parallel with other operations. Calling a remove prefix on an
            //         empty key should never happen during normal operations. We handle this case
            //         by returning an error. The caller will then need to recreate the proposal
            //         serially.
            let Some(first_nibble) = key_nibbles.next() else {
                match &op {
                    BatchOp::Put { key: _, value: _ } => {
                        // There should always be a value, even if it is empty.
                        root_branch.value = Some(
                            op.value()
                                .as_ref()
                                .map(|v| v.as_ref().into())
                                .unwrap_or_default(),
                        );
                    }
                    BatchOp::Delete { key: _ } => {
                        // Delete the value for this key.
                        root_branch.value = None;
                    }
                    BatchOp::DeleteRange { prefix: _ } => {
                        todo!();
                    }
                }
                continue; // Done with this operation.
            };

            println!("First nibble: {first_nibble:?}");

            // We send a path to the worker without the first nibble. Sending a nibble iterator to a
            // worker is more difficult as it takes a reference from op.
            let key_path = Path::from_nibbles_iterator(key_nibbles);
            println!("Key Path: {key_path:?}");

            // Find the worker's state corresponding to the first nibble which are stored in an array.
            // Create a new worker state if it doesn't exist.
            let worker_option = self
                .workers
                .get_mut(first_nibble as usize)
                .expect("index out of bounds");
            let worker = worker_option.get_or_insert_with(|| {
                println!("Creating worker for nibble: {first_nibble:?}");

                // No worker state for this nibble yet, so create one.
                // Create a channel for the coordinator (main thread) to send messages to this worker.
                let child_channel = mpsc::channel();

                // The worker will send messages to the coordinator using the response channel
                // we copy and (poorly) name the variables here to move them to the worker
                let (worker_receiver, worker_sender) =
                    (child_channel.1, response_channel.0.clone());

                // get the child node from the proposal
                let child = root_branch
                    .children
                    .get_mut(first_nibble as usize)
                    .expect("index error")
                    .take();

                // build a nodestore from the child node
                let worker_nodestore =
                    NodeStore::from_child(&proposal, child).expect("TODO: handle error");

                // now tell the threadpool to spawn a worker for this nibble
                pool.spawn(move || {
                    //let mut merkle = Merkle::from(child_nodestore);
                    let mut merkle = Merkle::from(worker_nodestore);
                    loop {
                        let request = worker_receiver.recv().expect("TODO: handle error");
                        match request {
                            // insert a key-value pair into the subtrie
                            Request::Insert { key, value } => {
                                // TODO: we still have a bug here, we need to remove the first nibble from the key :(
                                println!("### In Worker Thread: Inserting: {key:?} and {value:?}");
                                merkle
                                    //.insert(key_path.as_ref(), value.as_ref().into())
                                    //.insert(key.as_ref(), value.as_ref().into())
                                    .insert_path(key, value.as_ref().into())
                                    .expect("TODO: handle error");
                            }
                            // these should be easy to implement...
                            Request::Delete { key } => {
                                merkle.remove_path(key).expect("TODO: handle error");
                            }
                            Request::DeleteRange { prefix } => {
                                merkle
                                    .remove_prefix_path(prefix)
                                    .expect("TODO: handle error");
                            }
                            // sent from the coordinator to the workers to signal that they are done
                            Request::Done => {
                                worker_sender
                                    .send(Response::Root(
                                        first_nibble,
                                        //merkle.into_inner().into_root(),
                                        merkle.into_inner().into(),
                                        //merkle.into_inner().mut_root().take(),
                                    ))
                                    .expect("TODO: handle error");
                                break;
                            }
                        }
                    }
                });
                WorkerState {
                    sender: child_channel.0,
                }
            });

            println!("Worker Send (key_path): {:?}", key_path.as_ref());

            // we have the right worker, so send the request to it
            match &op {
                BatchOp::Put { key: _, value: _ } => {
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
                        .expect("TODO: handle error");
                }
                BatchOp::Delete { key: _ } => {
                    worker
                        .sender
                        .send(Request::Delete { key: key_path })
                        .expect("TODO: handle error");
                }
                BatchOp::DeleteRange { prefix: _ } => {
                    worker
                        .sender
                        .send(Request::DeleteRange { prefix: key_path })
                        .expect("TODO: handle error");
                }
            }
        }

        // Drop the sender response channel from the parent thread.
        drop(response_channel.0);

        // we have processed all the ops in the batch, so send a Done message to each worker
        for worker in self.workers.iter().flatten() {
            worker
                .sender
                .send(Request::Done)
                .expect("TODO: add error handling");
        }

        //let mut proposal = NodeStore::new(&parent)?;
        while let Ok(response) = response_channel.1.recv() {
            match response {
                Response::Root(index, mut child_nodestore) => {
                    // Taking deleted nodes (from calling read_for_update) from child nodestores.
                    proposal.add_deleted_nodes_from_child(&mut child_nodestore);

                    // Set the child at index using the root from the child nodestore.
                    *root_branch
                        .children
                        .get_mut(index as usize)
                        .expect("index error") = child_nodestore.into_root().map(Child::Node);

                    //root_branch.children[index as usize] = child_nodestore.into_root().map(Child::Node);
                    // TODO: Need to combine the deletes from the children.
                    //proposal.add_deleted_nodes_from_child(&mut child_nodestore);
                }
                Response::Error(_error) => todo!(),
            }
        }

        *proposal.mut_root() = self
            .postprocess_trie(&mut proposal, Some((*root_branch).into()))
            .expect("TODO check errors");

        // Done with these worker states. Setting the workers to None will allow the next create
        // proposal from the same ParallelMerkle to be reused to spawn new states.
        self.workers = [(); BranchNode::MAX_CHILDREN].map(|()| None);

        let immutable: Arc<NodeStore<Arc<ImmutableProposal>, FileBacked>> =
            Arc::new(proposal.try_into().expect("TODO: handle error"));

        Ok(immutable)
    }
}
