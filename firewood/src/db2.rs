use crate::api2::{self, Batch, KeyType, ValueType};
use async_trait::async_trait;

struct DB;

#[async_trait]
impl api2::DB for DB {
    type View = DBView;

    async fn revision(
        &self,
        _hash: api2::HashKey,
    ) -> Result<std::sync::Weak<Self::View>, api2::Error> {
        todo!()
    }

    async fn root_hash(&self) -> Result<api2::HashKey, api2::Error> {
        todo!()
    }

    async fn propose<K: KeyType, V: ValueType>(
        &mut self,
        _hash: api2::HashKey,
        _data: Batch<K, V>,
    ) -> Result<api2::HashKey, api2::Error> {
        todo!()
    }

    async fn commit(&mut self, _hash: api2::HashKey) -> Result<(), api2::Error> {
        todo!()
    }
}

struct DBView;

#[async_trait]
impl api2::DBView for DBView {
    async fn hash(&self) -> Result<api2::HashKey, api2::Error> {
        todo!()
    }

    async fn val<K: KeyType, V: ValueType>(&self, _key: K) -> Result<V, api2::Error> {
        todo!()
    }

    async fn single_key_proof<K: KeyType, V: ValueType>(
        &self,
        _key: K,
    ) -> Result<api2::Proof<V>, api2::Error> {
        todo!()
    }

    async fn range_proof<K: KeyType, V: ValueType>(
        &self,
        _first_key: K,
        _last_key: K,
    ) -> Result<api2::RangeProof<K, V>, api2::Error> {
        todo!()
    }
}
