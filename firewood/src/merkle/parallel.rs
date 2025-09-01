// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//use std::collections::HashMap;
use std::sync::{Arc, OnceLock, mpsc};

//use firewood_storage::{
//    BranchNode, Child, FileBacked, FileIoError, ImmutableProposal, NibblesIterator, Node,
//    NodeStore, Parentable, RootReader,
//};

use firewood_storage::{
    BranchNode, Child, FileBacked, FileIoError, ImmutableProposal, LeafNode, MutableProposal,
    NibblesIterator, Node, NodeStore, Parentable, Path,
};

use rayon::{ThreadPool, ThreadPoolBuilder};
//use sha2::digest::crypto_common::KeyInit;

use crate::db::BatchOp;
//use crate::db;
use crate::merkle::{Merkle, PrefixOverlap, Value};
use crate::v2::api::KeyValuePairIter;
use std::iter::once;

/*
trait DbExt {
    fn propose_sync_parallel(
        &self,
        batch: impl IntoIterator<IntoIter: KeyValuePairIter>,
    ) -> Result<Proposal<'_>, FileIoError>;
}
*/

#[derive(Debug)]
struct WorkerState {
    sender: mpsc::Sender<Request>,
}

/// A request to the worker.
#[derive(Debug)]
enum Request {
    //Insert { key: Key, value: Value },
    Insert { key: Path, value: Value },
    Delete { key: Path },
    DeleteRange { prefix: Path },
    Done,
}

#[derive(Debug)]
enum Response {
    // return the new root of the subtrie at the given nibble
    //Root(u8, Option<Node>),
    Root(u8, Option<Node>),
    Error(FileIoError),
}

static THREADPOOL: OnceLock<ThreadPool> = OnceLock::new();

/// TODO add doc
#[derive(Debug)]
pub struct ParallelMerkle {
    //worker: Option<WorkerState>,
    workers: [Option<WorkerState>; BranchNode::MAX_CHILDREN],
}

impl Default for ParallelMerkle {
    fn default() -> Self {
        Self::new()
    }
}

/*
            let mut branch = new_root
                .as_branch()
                .expect("Should be a branch because of earlier transform");

            let mut children_iter = branch
                .children
                .iter()
                .enumerate()
                .filter_map(|(index, child)| child.as_ref().map(|child| (index, child)));

            let first_child = children_iter.next();

            match first_child {
                None => {
                    // No value. Just delete the root
                    if branch.value.is_none() {
                        return None;
                    }
                    let new_leaf = Node::Leaf(LeafNode {
                        value: branch.value.take()?,
                        partial_path: branch.partial_path.clone(),
                    });
                }
                Some(child) => {}
            }
        }

        None


        if let Some(new_root) = new_root_opt {
            // Root should always be a branch given our earlier transform
            let branch = new_root
                .as_branch()
                .expect("Should be branch because of earlier transform");

            let children_iter = branch
                .children
                .iter()
                .enumerate()
                .filter_map(|(index, child)| child.map(|child| (index, child)));

            let first_child = children_iter.next();

            match first_child {
                None => {
                    // No value. Just delete the root
                    if branch.value.is_none() {
                        return None;
                    }
                }
                Some => {}
            }

            let (child_index, child) = children_iter
                .next()
                .expect("branch node must have children");

            if children_iter.next().is_some() {
                // The branch has more than 1 child so do nothing
                *proposal.mut_root() = new_root;
            } else {
                // The branch's only child becomes the root of this subtrie.
                let mut child = match child {
                    Child::Node(child_node) => std::mem::take(child_node),
                    Child::AddressWithHash(addr, _) => {
                        self.nodestore.read_for_update((*addr).into())?
                    }
                    Child::MaybePersisted(maybe_persisted, _) => {
                        self.nodestore.read_for_update(maybe_persisted.clone())?
                    }
                };


                let mut children_iter =
                    branch
                        .children
                        .iter_mut
                        .enumerate()
                        .filter_map(|(index, child)| {
                            child.as_mut().map(|child| (index, child))
                        });


                //let children = branch.children;

                //if new_root.
            }

            // This branch node has a value.
            // If it has multiple children, return the node as is.
            // Otherwise, its only child becomes the root of this subtrie.
            //let mut children_iter =
            //    branch
            //        .children
            //        .iter_mut()
            //        .enumerate()
            //        .filter_map(|(index, child)| {
            //            child.as_mut().map(|child| (index, child))
            //        });

            //let (child_index, child) = children_iter
            //    .next()
            //    .expect("branch node must have children");

            //if children_iter.next().is_some() {
            //    // The branch has more than 1 child so it can't be removed.
            //    Ok((Some(node), Some(removed_value)))

            //let a = new_root.unwrap();
            *proposal.mut_root() = new_root;
        }



*/

impl ParallelMerkle {
    /// Default constructor
    #[must_use]
    pub fn new() -> Self {
        //ParallelMerkle { workers: [None, None, None, None,
        //                           None, None, None, None,
        //                           None, None, None, None,
        //                           None, None, None, None]}
        //let none_array = [(); BranchNode::MAX_CHILDREN].map(|_| None);
        ParallelMerkle {
            workers: [(); BranchNode::MAX_CHILDREN].map(|()| None),
        }
    }

    fn postprocess_trie(
        &self,
        nodestore: &mut NodeStore<MutableProposal, FileBacked>,
        new_root_opt: Option<Node>,
    ) -> Result<Option<Node>, FileIoError> {
        // Check if the Merkle trie is malformed. If it is, apply transform to create
        // a valid Merkle trie.
        //
        // If all of the children have been removed, then
        //     Convert the root into a leaf if it has a value (this should indicate that a
        //     value was inserted for the empty key).
        //     Otherwise, delete the empty root node
        // If more than one child remains, then do nothing
        // If only one child remains and the root has a value, then do nothing
        // If only one child remains and the root doesn’t have a value, then deleted the root,
        // use the child node as the new root, and update the partial path of the new root to
        // include its previous child index
        println!("In postprocess_trie");

        if let Some(mut new_root) = new_root_opt {
            match &mut new_root {
                Node::Branch(branch) => {
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
                            // No value. Just delete the root
                            return Ok(None);
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
                                Child::AddressWithHash(addr, _) => {
                                    nodestore.read_for_update((*addr).into())?
                                }
                                Child::MaybePersisted(maybe_persisted, _) => {
                                    nodestore.read_for_update(maybe_persisted.clone())?
                                }
                            };

                            // The child's partial path is the concatenation of its (now removed) parent,
                            // its (former) child index, and its partial path. Note that the parent's
                            // partial path should always be empty given the pre-insert transform.
                            //let partial_path = Path::from_nibbles_iterator(
                            //    branch
                            //        .partial_path
                            //        .iter()
                            //        .copied()
                            //        .chain(once(child_index as u8))
                            //        .chain(child.partial_path().iter().copied()),
                            //);
                            println!("Child's partial path: {:?}", child.partial_path());

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
                            return Ok(Some(child));
                        }
                    }
                }
                Node::Leaf(_) => {
                    return Ok(None);
                    // Should never be a leaf
                    //assert!(false);
                }
            }
        }
        Ok(None)
    }

    /// TODO: add doc
    #[allow(clippy::missing_panics_doc)]
    #[allow(clippy::missing_errors_doc)]
    #[allow(clippy::too_many_lines)]
    pub fn create_proposal<T: Parentable>(
        &mut self,
        parent: &NodeStore<T, FileBacked>,
        batch: impl IntoIterator<IntoIter: KeyValuePairIter>,
    ) -> Result<Arc<NodeStore<Arc<ImmutableProposal>, FileBacked>>, FileIoError> {
        // get (or create) a threadpool
        let pool = THREADPOOL.get_or_init(|| {
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

        // keep track of the workers for each nibble
        // TODO: use something better than a hashmap here
        //let mut workers = HashMap::new();
        //self.worker = None;

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

            // For the first version, just pass the key instead of a NibblesIterator
            // Get the first nibble of the key to determine which worker to send the request to
            //let key_nibbles = NibblesIterator::new(op.key().as_ref());
            //let key_path = Path::from_nibbles_iterator(key_nibbles);
            //println!("Key Path: {key_path:?}");

            //let a: &[u8] = key_path.as_ref();
            //let b = a.split_first().unwrap();
            //println!("Index: {:?} Remaining: {:?}", b.0, b.1);

            let key_nibbles = NibblesIterator::new(op.key().as_ref());
            let key_path = Path::from_nibbles_iterator(key_nibbles);

            let empty_path = Path::new();
            let path_overlap = PrefixOverlap::from(key_path.as_ref(), empty_path.as_ref());

            let unique_key = path_overlap.unique_a;
            let unique_node = path_overlap.unique_b;

            println!("unique_key: {unique_key:?}");
            println!("unique_node: {unique_node:?}");

            let a: (u8, Path) = unique_key
                .split_first()
                .map(|(index, path)| (*index, path.into())).expect("todo");
            
            println!("Split: {:?} ------ {:?}", a.0, a.1);

            // TODO: It might be better to call insert_helper instead of insert because of the nibble
            //       taken off the key.


            let mut key_nibbles = NibblesIterator::new(op.key().as_ref());
            //let key_path = Path::from_nibbles_iterator(key_nibbles);
            // TODO: Need to handle empty key.
            let first_nibble = key_nibbles.next().expect("TODO: empty key");
            //key_nibbles.next().expect("testing");

            println!("First nibble: {first_nibble:?}");
            // TODO(bug): we need to send the key_nibbles iterator instead of the key to the child

            //let worker = self.workers[first_nibble as usize];

            let key_path = Path::from_nibbles_iterator(key_nibbles);
            println!("Key Path: {key_path:?}");

            // Find the worker state corresponding to the first nibble. Check for index bounds
            // just in case. Create a new worker state if it doesn't exist.
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

                //let child = std::mem::take(&mut root_branch.children.get_mut(first_nibble as usize).expect("index error"));
                //
                // build a nodestore from the child node
                let worker_nodestore =
                    NodeStore::from_child(&proposal, child).expect("TODO: handle error");

                //let worker_nodestore = NodeStore::new(parent).expect("TODO handle error");

                //let worker_nodestore =
                //    NodeStore::from_proposal(&mut proposal).expect("TODO handle error");

                //let worker_nodestore = NodeStore::new(&proposal).expect("TODO handle error");
                //let mut merkle = Merkle::from(proposal);

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
                                merkle.remove_prefix_path(prefix).expect("TODO: handle error");
                            }
                            // sent from the coordinator to the workers to signal that they are done
                            Request::Done => {
                                worker_sender
                                    .send(Response::Root(
                                        first_nibble,
                                        merkle.into_inner().into_root(),
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

            match &op {
                BatchOp::Put { key: _, value: _} => {
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
                        .send(Request::Delete {
                            key: key_path,
                        })
                        .expect("TODO: handle error");
                }
                BatchOp::DeleteRange { prefix: _ } => {
                    worker
                        .sender
                        .send(Request::DeleteRange {
                            prefix: key_path,
                        })
                        .expect("TODO: handle error");
                }
            }

            /* 
            //key_nibbles.
            // we have the right worker, so send the request to it
            worker
                .sender
                .send(Request::Insert {
                    //key: key_path.as_ref().into(),
                    key: key_path,
                    //key: op.key().as_ref().into(),
                    value: op
                        .value()
                        .as_ref()
                        .map(|v| v.as_ref().into())
                        .unwrap_or_default(),
                })
                .expect("TODO: handle error");
            */
        }

        // Drop the sender response channel from the parent thread.
        drop(response_channel.0);

        // we have processed all the batches, so send a Done message to each worker

        //for worker in workers.values_mut() {
        //    worker.sender.send(Request::Done).expect("TODO: handle error");
        //}

        for worker in self.workers.iter().flatten() {
            worker
                .sender
                .send(Request::Done)
                .expect("TODO: add error handling");
        }

        /*
                for worker in &self.workers {
                    if let Some(worker) = worker {
                        worker
                            .sender
                            .send(Request::Done)
                            .expect("TODO: add error handling");
                    }


                                match worker {
                                    None => {}
                                    Some(worker) => {
                                        worker
                                            .sender
                                            .send(Request::Done)
                                            .expect("TODO: add error handling");
                                    }
                                }

                }
        */
        //self.worker
        //    .expect("TODO add error handling")
        //    .sender
        //    .send(Request::Done)
        //    .expect("TODO: handle error");

        /*
        // collect all the responses from the workers
        while let Ok(response) = response_channel.1.recv() {
            match response {
                Response::Root(nibble, new_root) => {
                    root.children[nibble as usize] = new_root.map(Child::Node);
                }
                Response::Error(_error) => todo!(),
            }
        }
        */

        //let mut proposal = NodeStore::new(&parent)?;

        while let Ok(response) = response_channel.1.recv() {
            match response {
                Response::Root(index, new_root_opt) => {
                    root_branch.children[index as usize] = new_root_opt.map(Child::Node);

                    //*proposal.mut_root() = self
                    //    .postprocess_trie(&mut proposal, new_root_opt)
                    //    .expect("TODO check errors");
                }
                Response::Error(_error) => todo!(),
            }
        }

        *proposal.mut_root() = self
            .postprocess_trie(&mut proposal, Some((*root_branch).into()))
            .expect("TODO check errors");

        // Done with these async tasks. Setting worker to None will allow
        // the next create proposal from the same ParallelMerkle to be reused
        // to spawn new tasks.
        self.workers = [(); BranchNode::MAX_CHILDREN].map(|()| None);

        //*proposal.mut_root() = Some(Node::Branch(root.clone()));
        // impl<S: ReadableStorage> TryFrom<NodeStore<MutableProposal, S>>
        //     for NodeStore<Arc<ImmutableProposal>, S>

        // TODO: Replace Proposal::new with the correct constructor or method for Proposal.
        // The current code will not compile if Proposal::new does not exist.
        let immutable: Arc<NodeStore<Arc<ImmutableProposal>, FileBacked>> =
            Arc::new(proposal.try_into().expect("TODO: handle error"));

        Ok(immutable)
    }
}
