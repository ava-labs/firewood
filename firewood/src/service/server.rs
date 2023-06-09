// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::atomic::{AtomicU32, Ordering},
};
use tokio::sync::mpsc::Receiver;

use crate::db::{Db, DbConfig, DbError};

use super::{BatchId, BatchRequest, Request, RevId};

macro_rules! get_batch {
    ($active_batch: expr, $handle: ident, $lastid: ident, $respond_to: expr) => {{
        if $handle != $lastid.load(Ordering::Relaxed) - 1 || $active_batch.is_none() {
            let _ = $respond_to.send(Err(DbError::InvalidParams));
            continue;
        }
        $active_batch.take().unwrap()
    }};
}

macro_rules! get_rev {
    ($rev: ident, $handle: ident, $out: expr) => {
        match $rev.get(&$handle) {
            None => {
                let _ = $out.send(Err(DbError::InvalidParams));
                continue;
            }
            Some(x) => x,
        }
    };
}
#[derive(Copy, Debug, Clone)]
pub struct FirewoodService {}

impl FirewoodService {
    pub fn new(mut receiver: Receiver<Request>, owned_path: PathBuf, cfg: DbConfig) -> Self {
        let db = Db::new(owned_path, &cfg).unwrap();
        let mut active_batch: Option<crate::db::WriteBatch> = None;
        let mut revs = HashMap::<RevId, crate::db::Revision>::new();
        let lastid = AtomicU32::new(0);
        loop {
            let msg = match receiver.blocking_recv() {
                Some(msg) => msg,
                None => break,
            };
            match msg {
                Request::NewBatch { respond_to } => {
                    let id: BatchId = lastid.fetch_add(1, Ordering::Relaxed);
                    active_batch = Some(db.new_writebatch());
                    let _ = respond_to.send(id);
                }
                Request::NewRevision {
                    root_hash,
                    cfg,
                    respond_to,
                } => {
                    let id: RevId = lastid.fetch_add(1, Ordering::Relaxed);
                    let msg = match db.get_revision(root_hash, cfg) {
                        Some(rev) => {
                            revs.insert(id, rev);
                            Some(id)
                        }
                        None => None,
                    };
                    let _ = respond_to.send(msg);
                }

                Request::RevRequest(req) => match req {
                    super::RevRequest::Get {
                        handle,
                        key,
                        respond_to,
                    } => {
                        let rev = get_rev!(revs, handle, respond_to);
                        let msg = rev.kv_get(key);
                        let _ = respond_to.send(msg.map_or(Err(DbError::KeyNotFound), Ok));
                    }
                    #[cfg(feature = "proof")]
                    super::RevRequest::Prove {
                        handle,
                        key,
                        respond_to,
                    } => {
                        let msg = revs
                            .get(&handle)
                            .map_or(Err(MerkleError::UnsetInternal), |r| r.prove(key));
                        let _ = respond_to.send(msg);
                    }
                    super::RevRequest::RootHash { handle, respond_to } => {
                        let rev = get_rev!(revs, handle, respond_to);
                        let msg = rev.kv_root_hash();
                        let _ = respond_to.send(msg);
                    }
                    super::RevRequest::Drop { handle } => {
                        revs.remove(&handle);
                    }
                },
                Request::BatchRequest(req) => match req {
                    BatchRequest::Commit { handle, respond_to } => {
                        let batch = get_batch!(active_batch, handle, lastid, respond_to);
                        batch.commit();
                        let _ = respond_to.send(Ok(()));
                    }
                    BatchRequest::KvInsert {
                        handle,
                        key,
                        val,
                        respond_to,
                    } => {
                        let batch = get_batch!(active_batch, handle, lastid, respond_to);
                        let resp = match batch.kv_insert(key, val) {
                            Ok(v) => {
                                active_batch = Some(v);
                                Ok(())
                            }
                            Err(e) => Err(e),
                        };
                        respond_to.send(resp).unwrap();
                    }
                    BatchRequest::KvRemove {
                        handle,
                        key,
                        respond_to,
                    } => {
                        let batch = get_batch!(active_batch, handle, lastid, respond_to);
                        let resp = match batch.kv_remove(key) {
                            Ok(v) => {
                                active_batch = Some(v.0);
                                Ok(v.1)
                            }
                            Err(e) => Err(e),
                        };
                        respond_to.send(resp).unwrap();
                    }
                    #[cfg(feature = "eth")]
                    BatchRequest::SetBalance {
                        handle,
                        key,
                        balance,
                        respond_to,
                    } => {
                        let batch = get_batch!(active_batch, handle, lastid, respond_to);
                        let resp = match batch.set_balance(key.as_ref(), balance) {
                            Ok(v) => {
                                active_batch = Some(v);
                                Ok(())
                            }
                            Err(e) => Err(e),
                        };
                        respond_to.send(resp).unwrap();
                    }
                    #[cfg(feature = "eth")]
                    BatchRequest::SetCode {
                        handle,
                        key,
                        code,
                        respond_to,
                    } => {
                        let batch = get_batch!(active_batch, handle, lastid, respond_to);
                        let resp = match batch.set_code(key.as_ref(), code.as_ref()) {
                            Ok(v) => {
                                active_batch = Some(v);
                                Ok(())
                            }
                            Err(e) => Err(e),
                        };
                        respond_to.send(resp).unwrap();
                    }
                    #[cfg(feature = "eth")]
                    BatchRequest::SetNonce {
                        handle,
                        key,
                        nonce,
                        respond_to,
                    } => {
                        let batch = get_batch!(active_batch, handle, lastid, respond_to);
                        let resp = match batch.set_nonce(key.as_ref(), nonce) {
                            Ok(v) => {
                                active_batch = Some(v);
                                Ok(())
                            }
                            Err(e) => Err(e),
                        };
                        respond_to.send(resp).unwrap();
                    }
                    #[cfg(feature = "eth")]
                    BatchRequest::SetState {
                        handle,
                        key,
                        sub_key,
                        state,
                        respond_to,
                    } => {
                        let batch = get_batch!(active_batch, handle, lastid, respond_to);
                        let resp = match batch.set_state(key.as_ref(), sub_key.as_ref(), state) {
                            Ok(v) => {
                                active_batch = Some(v);
                                Ok(())
                            }
                            Err(e) => Err(e),
                        };
                        respond_to.send(resp).unwrap();
                    }
                    #[cfg(feature = "eth")]
                    BatchRequest::CreateAccount {
                        handle,
                        key,
                        respond_to,
                    } => {
                        let batch = get_batch!(active_batch, handle, lastid, respond_to);
                        let resp = match batch.create_account(key.as_ref()) {
                            Ok(v) => {
                                active_batch = Some(v);
                                Ok(())
                            }
                            Err(e) => Err(e),
                        };
                        respond_to.send(resp).unwrap();
                    }
                },
            }
        }
        FirewoodService {}
    }
}
