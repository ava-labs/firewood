use std::{
    fmt::Debug,
    sync::{Arc, Mutex, Weak},
};

use async_trait::async_trait;

use crate::v2::api::{self, Batch, KeyType, ValueType};

use super::propose;

#[cfg_attr(doc, aquamarine::aquamarine)]
/// ```mermaid
/// graph LR
///     RevRootHash --> DBRevID
///     RevHeight --> DBRevID
///     DBRevID -- Identify --> DbRev
///     Db/Proposal -- propose with batch --> Proposal
///     Proposal -- translate --> DbRev
///     DB -- commit proposal --> DB
/// ```
#[derive(Debug, Default)]
pub struct Db<T: api::DbView> {
    latest_cache: Mutex<Option<Arc<T>>>,
}

#[async_trait]
impl<T> api::Db for Db<T>
where
    T: api::DbView,
    T: Send + Sync,
{
    type Historical = T;

    type Proposal = propose::Proposal<T>;

    async fn revision(&self, _hash: api::HashKey) -> Result<Weak<Self::Historical>, api::Error> {
        todo!()
    }

    async fn root_hash(&self) -> Result<api::HashKey, api::Error> {
        todo!()
    }

    async fn propose<K: KeyType, V: ValueType>(
        self: Arc<Self>,
        data: Batch<K, V>,
    ) -> Result<Arc<Self::Proposal>, api::Error> {
        let mut dbview_latest_cache_guard = self.latest_cache.lock().unwrap();

        if dbview_latest_cache_guard.is_none() {
            // TODO: actually get the latest dbview
            *dbview_latest_cache_guard = Some(Arc::new(T::default()));
        };

        let proposal = propose::Proposal::new(
            propose::ProposalBase::View(dbview_latest_cache_guard.as_ref().unwrap().clone()),
            data,
        );

        Ok(Arc::new(proposal))
    }
}

#[derive(Debug, Default)]
pub struct DbView;

#[async_trait]
impl api::DbView for DbView {
    async fn hash(&self) -> Result<api::HashKey, api::Error> {
        todo!()
    }

    async fn val<K: KeyType>(&self, _key: K) -> Result<Vec<u8>, api::Error> {
        todo!()
    }

    async fn single_key_proof<K: KeyType, V: ValueType>(
        &self,
        _key: K,
    ) -> Result<api::Proof<V>, api::Error> {
        todo!()
    }

    async fn range_proof<K: KeyType, V: ValueType>(
        &self,
        _first_key: Option<K>,
        _last_key: Option<K>,
        _limit: usize,
    ) -> Result<api::RangeProof<K, V>, api::Error> {
        todo!()
    }
}
