use crate::v2::api::{self, Batch, KeyType, ValueType};
use async_trait::async_trait;
use std::sync::Weak;

struct Db;

#[async_trait]
impl api::Db for Db {
    type DbView = DbView;
    type Proposal = Proposal;

    async fn revision(&self, _hash: api::HashKey) -> Result<Weak<Self::DbView>, api::Error> {
        todo!()
    }

    async fn root_hash(&self) -> Result<api::HashKey, api::Error> {
        todo!()
    }

    async fn propose<K: KeyType, V: ValueType>(
        &mut self,
        _data: Batch<K, V>,
    ) -> Result<Proposal, api::Error> {
        todo!()
    }

    async fn commit(
        &mut self,
        _proposal: Self::Proposal,
    ) -> Result<Weak<Self::DbView>, api::Error> {
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
        _first_key: Option<K>,
        _last_key: Option<K>,
        _limit: usize,
    ) -> Result<api::RangeProof<K, V>, api::Error> {
        todo!()
    }
    async fn propose<K: KeyType, V: ValueType>(
        &self,
        _data: Batch<K, V>,
    ) -> Result<Weak<Self>, api::Error> {
        todo!()
    }
}

struct Proposal;

#[async_trait]
impl api::DbView for Proposal {
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
        _first_key: Option<K>,
        _last_key: Option<K>,
        _limit: usize,
    ) -> Result<api::RangeProof<K, V>, api::Error> {
        todo!()
    }
    async fn propose<K: KeyType, V: ValueType>(
        &self,
        _data: Batch<K, V>,
    ) -> Result<Weak<Self>, api::Error> {
        todo!()
    }
}
