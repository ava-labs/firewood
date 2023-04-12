// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::api::DB;

pub struct Sender {

}

impl<K: AsRef<[u8]>, V: AsRef<[u8]>> DB<K, V> for Sender {
    fn kv_root_hash(&self) -> Result<crate::merkle::Hash, crate::db::DBError> {
        todo!()
    }

    fn kv_get(&self, key: K) -> Option<Vec<u8>> {
        todo!()
    }

    fn kv_dump<W: std::io::Write>(&self, writer: W) -> Result<(), crate::db::DBError> {
        todo!()
    }

    fn root_hash(&self) -> Result<crate::merkle::Hash, crate::db::DBError> {
        todo!()
    }

    fn dump<W: std::io::Write>(&self, writer: W) -> Result<(), crate::db::DBError> {
        todo!()
    }

    #[cfg(feature = "eth")]
    fn get_account(&self, key: K) -> Result<crate::account::Account, crate::db::DBError> {
        todo!()
    }

    #[cfg(feature = "eth")]
    fn dump_account<W: std::io::Write>(&self, key: K, writer: W) -> Result<(), crate::db::DBError> {
        todo!()
    }

    #[cfg(feature = "eth")]
    fn get_balance(&self, key: K) -> Result<primitive_types::U256, crate::db::DBError> {
        todo!()
    }

    #[cfg(feature = "eth")]
    fn get_code(&self, key: K) -> Result<Vec<u8>, crate::db::DBError> {
        todo!()
    }

    #[cfg(feature = "proof")]
    fn prove(&self, key: K) -> Result<crate::proof::Proof, crate::merkle::MerkleError> {
        todo!()
    }

    #[cfg(feature = "proof")]
    fn verify_range_proof(
        &self,
        proof: crate::proof::Proof,
        first_key: K,
        last_key: K,
        keys: Vec<K>,
        values: Vec<V>,
    ) {
        todo!()
    }

    #[cfg(feature = "eth")]
    fn get_nonce(&self, key: K) -> Result<crate::api::Nonce, crate::db::DBError> {
        todo!()
    }

    #[cfg(feature = "eth")]
    fn get_state(&self, key: K, sub_key: K) -> Result<Vec<u8>, crate::db::DBError> {
        todo!()
    }

    fn exist(&self, key: K) -> Result<bool, crate::db::DBError> {
        todo!()
    }
}