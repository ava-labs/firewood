// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::{
    collections::HashMap,
    sync::{Arc, OnceLock, mpsc},
};

use firewood_storage::{
    BranchNode, Child, FileBacked, FileIoError, ImmutableProposal, NibblesIterator, Node,
    NodeStore, Parentable,
};
use rayon::{ThreadPool, ThreadPoolBuilder};

use crate::{
    db::{Db, Proposal},
    merkle::{Key, Merkle, Value},
    v2::api::KeyValuePairIter,
};

#[derive(Debug)]
struct ParallelMerkle {
    workers: HashMap<u8, Worker>,
}

#[derive(Debug)]
struct Worker {
    sender: mpsc::Sender<Request>,
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
enum Response {
    // return the new root of the subtrie at the given nibble
    Root(u8, Option<Node>),
    Error(FileIoError),
}

// TODO: move this to the database
static THREADPOOL: OnceLock<ThreadPool> = OnceLock::new();

impl ParallelMerkle {
    // TODO: move this as a method on `Db``, like `Db::propose`
    pub fn create_proposal<'db, T: Parentable>(
        db: &'db Db,
        parent: &NodeStore<T, FileBacked>,
        batch: impl IntoIterator<IntoIter: KeyValuePairIter> + Send,
    ) -> Result<Proposal<'db>, FileIoError> {
        
        // get (or create) a threadpool
        let pool = THREADPOOL.get_or_init(|| {
            ThreadPoolBuilder::new()
                .num_threads(BranchNode::MAX_CHILDREN)
                .build()
                .expect("TODO: handle error")
        });

        // create a proposal from the parent
        let mut proposal = NodeStore::new(parent)?;

        // Get the root node of that proposal. For now, this must exist and be a branch node.
        let root_node = proposal.mut_root().clone().expect("TODO: empty root");
        let mut root = root_node.as_branch().expect("TODO: root is a leaf").clone();

        // keep track of the workers for each nibble
        // TODO: use something better than a hashmap here
        let mut workers = HashMap::new();

        // Create a response channel the workers use to send messages back to the coordinator (us)
        let response_channel = mpsc::channel();

        // for each operation in the batch, send a request to the worker related to the first nibble
        for op in batch.into_iter().map_into_batch() {
            // Get the first nibble of the key to determine which worker to send the request to
            let mut key_nibbles = NibblesIterator::new(op.key().as_ref());
            let first_nibble = key_nibbles.next().expect("TODO: empty key");
            // TODO(bug): we can need to send the key_nibbles iterator instead of the key to the child

            // Find the worker for the first nibble, or create a new one if it doesn't exist
            let worker = workers.entry(first_nibble).or_insert_with(|| {
                // no worker yet, so create one

                // create a channel for the coordinator to send messages to the worker
                let child_channel = mpsc::channel();
                // the worker will send messages to the coordinator using the response channel
                // we copy and (poorly) name the variables here to move them to the worker
                let (worker_receiver, worker_sender) =
                    (child_channel.1, response_channel.0.clone());

                // get the child node from the proposal
                let child = std::mem::take(&mut root.children[first_nibble as usize]);

                // build a nodestore from the child node
                let child_nodestore =
                    NodeStore::from_child(&proposal, child).expect("TODO: handle error");

                // now tell the threadpool to spawn a worker for this nibble
                pool.spawn(move || {
                    let mut merkle = Merkle::from(child_nodestore);
                    loop {
                        let request = worker_receiver.recv().expect("TODO: handle error");
                        match request {
                            // insert a key-value pair into the subtrie
                            Request::Insert { key, value } => {
                                // TODO: we still have a bug here, we need to remove the first nibble from the key :(
                                merkle
                                    .insert(key.as_ref(), value.as_ref().into())
                                    .expect("TODO: handle error");
                            }
                            // these should be easy to implement...
                            Request::Delete { key: _ } => todo!(),
                            Request::DeleteRange { prefix: _ } => todo!(),
                            
                            // sent from the coordinator to the workers to signal that they are done
                            Request::Done => {
                                worker_sender
                                    .send(Response::Root(
                                        first_nibble,
                                        merkle.into_inner().into_root(),
                                    ))
                                    .expect("TODO: handle error");
                                break;
                            }
                        }
                    }
                });
                Worker {
                    sender: child_channel.0,
                }
            });

            // we have the right worker, so send the request to it
            worker
                .sender
                .send(Request::Insert {
                    key: op.key().as_ref().into(),
                    value: op
                        .value()
                        .as_ref()
                        .map(|v| v.as_ref().into())
                        .unwrap_or_default(),
                })
                .expect("TODO: handle error");
        }

        // we have processed all the batches, so send a Done message to each worker

        for worker in workers.values_mut() {
            worker.sender.send(Request::Done).expect("TODO: handle error");
        }

        // collect all the responses from the workers
        while let Ok(response) = response_channel.1.recv() {
            match response {
                Response::Root(nibble, new_root) => {
                    root.children[nibble as usize] = new_root.map(Child::Node);
                }
                Response::Error(_error) => todo!(),
            }
        }

        *proposal.mut_root() = Some(Node::Branch(root.clone()));
        // impl<S: ReadableStorage> TryFrom<NodeStore<MutableProposal, S>>
        //     for NodeStore<Arc<ImmutableProposal>, S>

        // TODO: Replace Proposal::new with the correct constructor or method for Proposal.
        // The current code will not compile if Proposal::new does not exist.
        let immutable: Arc<NodeStore<Arc<ImmutableProposal>, FileBacked>> =
            Arc::new(proposal.try_into().expect("TODO: handle error"));

        let immutable_proposal = Proposal::new(immutable, db);
        Ok(immutable_proposal)
    }
}
/*
let parent = self.manager.current_revision();
        let proposal = NodeStore::new(&parent)?;
        let mut merkle = Merkle::from(proposal);
        let span = fastrace::Span::enter_with_local_parent("merkleops");
        for op in batch.into_iter().map_into_batch() {
            match op {
                BatchOp::Put { key, value } => {
                    merkle.insert(key.as_ref(), value.as_ref().into())?;
                }
                BatchOp::Delete { key } => {
                    merkle.remove(key.as_ref())?;
                }
                BatchOp::DeleteRange { prefix } => {
                    merkle.remove_prefix(prefix.as_ref())?;
                }
            }
        }
*/
