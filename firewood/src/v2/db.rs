use crate::v2::api::{self, Batch, KeyType, ValueType};
use async_trait::async_trait;

struct Db;

#[async_trait]
impl api::Db for Db {
    type View = DbView;

    async fn revision(
        &self,
        _hash: api::HashKey,
    ) -> Result<std::sync::Weak<Self::View>, api::Error> {
        todo!()
    }

    async fn root_hash(&self) -> Result<api::HashKey, api::Error> {
        todo!()
    }

    async fn propose<K: KeyType, V: ValueType>(
        &mut self,
        _hash: api::HashKey,
        _data: Batch<K, V>,
    ) -> Result<api::HashKey, api::Error> {
        todo!()
    }

    async fn commit(&mut self, _hash: api::HashKey) -> Result<(), api::Error> {
        todo!()
    }
}

struct DbView;

#[async_trait]
impl api::DbView for DbView {
    async fn hash(&self) -> Result<api::HashKey, api::Error> {
        todo!()
    }

    async fn val<K: KeyType, V: ValueType>(&self, _key: K) -> Result<V, api::Error> {
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
        _first_key: K,
        _last_key: K,
    ) -> Result<api::RangeProof<K, V>, api::Error> {
        todo!()
    }
}
