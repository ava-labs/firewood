use async_trait::async_trait;
use std::fmt::Debug;
use std::{collections::HashMap, sync::Weak};

/// A KeyType is something that can be cast to a u8 reference,
/// and can be sent and shared across threads. References with
/// lifetimes are not allowed (hence 'static)
pub trait KeyType: AsRef<[u8]> + Send + Sync + Debug + 'static {}

/// A ValueType is the same as a KeyType. However, these could
/// be a different type from the KeyType on a given API call.
/// For example, you might insert {key: "key", value: vec![0u8]}
/// This also means that the type of all the keys for a single
/// API call must be the same, as well as the type of all values
/// must be the same.
pub trait ValueType: AsRef<[u8]> + Send + Sync + Debug + 'static {}

/// The type and size of a single HashKey
pub type HashKey = [u8; 32];

/// A key/value pair operation. Only upsert and delete are
/// supported
#[derive(Debug)]
pub enum DBOp<K: KeyType, V: ValueType> {
    Put { key: K, value: V },
    Delete { key: K },
}

/// A list of operations to consist of a batch that
/// can be proposed
pub type Batch<K, V> = Vec<DBOp<K, V>>;

/// Errors returned through the API
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// A given hash key is not available in the database
    HashNotFound {
        provided: HashKey,
    },
    /// Incorrect root hash for commit
    IncorrectRootHash {
        provided: HashKey,
        current: HashKey,
    },
    /// Key not found
    KeyNotFound,
    IO(std::io::Error),
}

/// A range proof, consisting of a proof of the first key and the last key,
/// and a vector of all key
#[derive(Debug)]
pub struct RangeProof<K: KeyType, V: ValueType> {
    pub first_key: Proof<V>,
    pub last_key: Proof<V>,
    pub middle: Batch<K, V>,
}

/// A proof that a single key is present
#[derive(Debug)]
pub struct Proof<V>(pub HashMap<HashKey, V>);

/// The database interface, which includes a type for a static view of
/// the database (the DBView). The most common implementation of the DBView
/// is the api::DBView trait defined next.
#[async_trait]
pub trait DB {
    type View: DBView;

    /// Get a reference to a specific view based on a hash
    async fn revision(&self, hash: HashKey) -> Result<Weak<Self::View>, Error>;

    /// Get the hash of the most recently committed version
    async fn root_hash(&self) -> Result<HashKey, Error>;

    /// Propose a change to the database via a batch, providing back the
    /// hash for the proposed value
    async fn propose<K: KeyType, V: ValueType>(
        &mut self,
        hash: HashKey,
        data: Batch<K, V>,
    ) -> Result<HashKey, Error>;

    /// Commit a specific hash, that must have been proposed against the
    /// current root
    async fn commit(&mut self, hash: HashKey) -> Result<(), Error>;
}

/// A view of the database at a specific time. These are typically wrapped with
/// a Weak reference, as these can disappear when idle.
#[async_trait]
pub trait DBView {
    /// Get the hash for the current DBView
    async fn hash(&self) -> Result<HashKey, Error>;

    /// Get the value of a specific key
    async fn val<K: KeyType, V: ValueType>(&self, key: K) -> Result<V, Error>;

    /// Obtain a proof for a single key
    async fn single_key_proof<K: KeyType, V: ValueType>(&self, key: K) -> Result<Proof<V>, Error>;

    /// Obtain a range proof over a set of keys
    async fn range_proof<K: KeyType, V: ValueType>(
        &self,
        first_key: K,
        last_key: K,
    ) -> Result<RangeProof<K, V>, Error>;
}
