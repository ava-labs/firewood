use std::{
    borrow::Borrow,
    collections::BTreeMap,
    fmt::Debug,
    sync::{Arc, Mutex, Weak},
};

use async_trait::async_trait;

use crate::v2::api::{self, Batch};

#[derive(Debug, Default)]
pub struct Db {
    latest_cache: Mutex<Option<Arc<DbView>>>,
}

#[async_trait]
impl api::Db for Db {
    type Historical = DbView;
    type Proposal = Proposal;

    async fn revision(&self, _hash: api::HashKey) -> Result<Weak<Self::Historical>, api::Error> {
        todo!()
    }

    async fn root_hash(&self) -> Result<api::HashKey, api::Error> {
        todo!()
    }

    async fn propose<
        K: AsRef<[u8]> + Send + Sync + Debug + 'static,
        V: AsRef<[u8]> + Send + Sync + Debug + 'static,
    >(
        &mut self,
        data: Batch<K, V>,
    ) -> Result<Weak<Proposal>, api::Error> {
        let mut dbview_cache_guard = self.latest_cache.lock().unwrap();
        if dbview_cache_guard.is_none() {
            // TODO: actually get the latest dbview
            *dbview_cache_guard = Some(Arc::new(DbView {
                proposals: Mutex::new(vec![]),
            }));
        };
        let mut proposal_guard = dbview_cache_guard
            .as_ref()
            .unwrap()
            .proposals
            .lock()
            .unwrap();
        let proposal = Arc::new(Proposal::new(
            ProposalBase::View(dbview_cache_guard.clone().unwrap()),
            data,
        ));
        proposal_guard.push(proposal.clone());
        Ok(Arc::downgrade(&proposal))
    }
}

#[derive(Debug)]
pub struct DbView {
    proposals: Mutex<Vec<Arc<Proposal>>>,
}

#[async_trait]
impl api::DbView for DbView {
    async fn hash(&self) -> Result<api::HashKey, api::Error> {
        todo!()
    }

    async fn val<K: AsRef<[u8]> + Send + Sync + Debug + 'static>(
        &self,
        _key: K,
    ) -> Result<Vec<u8>, api::Error> {
        todo!()
    }

    async fn single_key_proof<
        K: AsRef<[u8]> + Send + Sync + Debug + 'static,
        V: AsRef<[u8]> + Send + Sync + Debug + 'static,
    >(
        &self,
        _key: K,
    ) -> Result<api::Proof<V>, api::Error> {
        todo!()
    }

    async fn range_proof<
        K: AsRef<[u8]> + Send + Sync + Debug + 'static,
        V: AsRef<[u8]> + Send + Sync + Debug + 'static,
    >(
        &self,
        _first_key: Option<K>,
        _last_key: Option<K>,
        _limit: usize,
    ) -> Result<api::RangeProof<K, V>, api::Error> {
        todo!()
    }
}

#[derive(Debug)]
enum ProposalBase {
    Proposal(Arc<Proposal>),
    View(Arc<DbView>),
}

#[derive(Debug)]
pub struct Proposal {
    base: ProposalBase,
    delta: BTreeMap<Vec<u8>, Option<Vec<u8>>>,
    children: Mutex<Vec<Arc<Proposal>>>,
}
impl Proposal {
    fn new(
        base: ProposalBase,
        batch: Batch<
            impl AsRef<[u8]> + Send + Sync + Debug + 'static,
            impl AsRef<[u8]> + Send + Sync + Debug + 'static,
        >,
    ) -> Self {
        let delta: BTreeMap<Vec<u8>, Option<Vec<u8>>> = batch
            .iter()
            .map(|op| match op {
                api::BatchOp::Put { key, value } => {
                    (key.as_ref().to_vec(), Some(value.as_ref().to_vec()))
                }
                api::BatchOp::Delete { key } => (key.as_ref().to_vec(), None),
            })
            .collect();
        Self {
            base,
            delta,
            children: Mutex::new(vec![]),
        }
    }
}

#[async_trait]
impl api::DbView for Proposal {
    async fn hash(&self) -> Result<api::HashKey, api::Error> {
        todo!()
    }

    async fn val<K: AsRef<[u8]> + Send + Sync + Debug + 'static>(
        &self,
        key: K,
    ) -> Result<Vec<u8>, api::Error> {
        // see if this key is in this proposal
        match self.delta.get(key.as_ref()) {
            Some(change) => match change {
                // key in proposal, check for Put or Delete
                Some(val) => Ok(val.clone()),
                None => Err(api::Error::KeyNotFound), // key was deleted in this proposal
            },
            None => match &self.base {
                // key not in this proposal, so delegate to base
                ProposalBase::Proposal(p) => p.val(key).await,
                ProposalBase::View(view) => view.val(key).await,
            },
        }
    }

    async fn single_key_proof<
        K: AsRef<[u8]> + Send + Sync + Debug + 'static,
        V: AsRef<[u8]> + Send + Sync + Debug + 'static,
    >(
        &self,
        _key: K,
    ) -> Result<api::Proof<V>, api::Error> {
        todo!()
    }

    async fn range_proof<
        KT: AsRef<[u8]> + Send + Sync + Debug + 'static,
        VT: AsRef<[u8]> + Send + Sync + Debug + 'static,
    >(
        &self,
        _first_key: Option<KT>,
        _last_key: Option<KT>,
        _limit: usize,
    ) -> Result<api::RangeProof<KT, VT>, api::Error> {
        todo!()
    }
}

#[async_trait]
impl api::Proposal<DbView> for Proposal {
    async fn propose<
        K: AsRef<[u8]> + Send + Sync + Debug + 'static,
        V: AsRef<[u8]> + Send + Sync + Debug + 'static,
    >(
        &self,
        data: Batch<K, V>,
    ) -> Result<Weak<Self>, api::Error> {
        // find the Arc for this base proposal from the parent
        let guard = match &self.base {
            ProposalBase::Proposal(p) => p.children.lock().unwrap(),
            ProposalBase::View(v) => v.proposals.lock().unwrap(),
        };
        let arc = guard
            .iter()
            .find(|&c| std::ptr::eq(c.borrow() as *const _, self as *const _));

        if arc.is_none() {
            return Err(api::Error::InvalidProposal);
        }
        let proposal = Arc::new(Proposal::new(
            ProposalBase::Proposal(arc.unwrap().clone()),
            data,
        ));
        self.children.lock().unwrap().push(proposal.clone());
        Ok(Arc::downgrade(&proposal))
    }
    async fn commit(self) -> Result<Weak<DbView>, api::Error> {
        todo!()
    }
}
#[cfg(test)]
mod test {
    use super::*;
    use crate::v2::api::Db as _;
    use crate::v2::api::DbView as _;
    use crate::v2::api::Proposal;
    use api::BatchOp;
    #[tokio::test]
    async fn test_basic_proposal() -> Result<(), crate::v2::api::Error> {
        let mut db = Db::default();
        let batch = vec![
            BatchOp::Put {
                key: b"k",
                value: b"v",
            },
            BatchOp::Delete { key: b"z" },
        ];
        let proposal = db.propose(batch).await?.upgrade().unwrap();
        assert_eq!(proposal.val(b"k").await.unwrap(), b"v");
        assert!(matches!(
            proposal.val(b"z").await.unwrap_err(),
            crate::v2::api::Error::KeyNotFound
        ));
        Ok(())
    }
    #[tokio::test]
    async fn test_nested_proposal() -> Result<(), crate::v2::api::Error> {
        let mut db = Db::default();
        let batch = vec![
            BatchOp::Put {
                key: b"k",
                value: b"v",
            },
            BatchOp::Delete { key: b"z" },
        ];
        let proposal1 = db.propose(batch).await?.upgrade().unwrap();
        let proposal2 = proposal1
            .propose(vec![BatchOp::Put {
                key: b"z",
                value: "undo",
            }])
            .await?
            .upgrade()
            .unwrap();
        assert_eq!(proposal1.val(b"k").await.unwrap(), b"v");
        assert_eq!(proposal2.val(b"k").await.unwrap(), b"v");
        assert!(matches!(
            proposal1.val(b"z").await.unwrap_err(),
            crate::v2::api::Error::KeyNotFound
        ));
        assert_eq!(proposal2.val(b"z").await.unwrap(), b"undo");
        Ok(())
    }
}
