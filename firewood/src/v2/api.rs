use async_trait::async_trait;
use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::Weak;

/// A KeyType is something that can be cast to a u8 reference,
/// and can be sent and shared across threads. References with
/// lifetimes are not allowed (hence 'static)
pub trait KeyType: AsRef<[u8]> + Send + Sync + Debug + 'static {}

/// A ValueType is the same as a [KeyType]. However, these could
/// be a different type from the [KeyType] on a given API call.
/// For example, you might insert {key: "key", value: vec!\[0u8\]}
/// This also means that the type of all the keys for a single
/// API call must be the same, as well as the type of all values
/// must be the same.
pub trait ValueType: AsRef<[u8]> + Send + Sync + Debug + 'static {}

/// The type and size of a single HashKey
/// These are 256-bit hashes that are used for a variety of reasons:
///  - They identify a version of the datastore at a specific point
///    in time
///  - They are used to provide integrity at different points in a
///    proof
pub type HashKey = [u8; 32];

/// A key/value pair operation. Only put (upsert) and delete are
/// supported
#[derive(Debug)]
pub enum BatchOp<K: KeyType, V: ValueType> {
    Put { key: K, value: V },
    Delete { key: K },
}

/// A list of operations to consist of a batch that
/// can be proposed
pub type Batch<K, V> = Vec<BatchOp<K, V>>;

/// A convenience implementation to convert a vector of key/value
/// pairs into a batch of insert operations
pub fn vec_into_batch<K: KeyType, V: ValueType>(value: Vec<(K, V)>) -> Batch<K, V> {
    value
        .into_iter()
        .map(|(key, value)| BatchOp::Put { key, value })
        .collect()
}

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
/// and a vector of all key/value pairs
#[derive(Debug)]
pub struct RangeProof<K: KeyType, V: ValueType> {
    pub first_key: Proof<V>,
    pub last_key: Proof<V>,
    pub middle: Vec<(K, V)>,
}

/// A proof that a single key is present
#[derive(Debug)]
pub struct Proof<V>(pub HashMap<HashKey, V>);

/// The database interface, which includes a type for a static view of
/// the database (the DbView). The most common implementation of the DbView
/// is the api::DbView trait defined next.
#[async_trait]
pub trait Db {
    type View: DbView;

    /// Get a reference to a specific view based on a hash
    ///
    /// # Arguments
    ///
    /// - `hash` - Identifies the revision for the view
    async fn revision(&self, hash: HashKey) -> Result<Weak<Self::View>, Error>;

    /// Get the hash of the most recently committed version
    async fn root_hash(&self) -> Result<HashKey, Error>;

    /// Propose a change to the database via a batch
    ///
    /// # Arguments
    ///
    /// * `hash` - the HashKey that identifies the revision
    ///            you want to propose this batch against
    /// * `data` - A batch consisting of [BatchOp::Put] and
    ///            [BatchOp::Delete] operations to apply
    ///
    async fn propose<K: KeyType, V: ValueType>(
        &mut self,
        hash: HashKey,
        data: Batch<K, V>,
    ) -> Result<HashKey, Error>;

    /// Commit a specific hash
    ///
    /// # Arguments
    ///
    /// * `hash` - The root this commit must apply against.
    ///            If this is not the latest commit, this
    ///            will return [Error::IncorrectRootHash]
    async fn commit(&mut self, hash: HashKey) -> Result<(), Error>;
}

/// A view of the database at a specific time. These are wrapped with
/// a Weak reference when fetching via a call to [Db::revision], as these
/// can disappear either because they became too old, or are no longer a
/// valid revision due to a [Db::commit].
///
/// You only need a DbView if you need to read from a snapshot at a given
/// root. Don't hold a strong reference to the DbView as it prevents older
/// views from being cleaned up.
#[async_trait]
pub trait DbView {
    /// Get the hash for the current DbView
    async fn hash(&self) -> Result<HashKey, Error>;

    /// Get the value of a specific key
    async fn val<K: KeyType, V: ValueType>(&self, key: K) -> Result<V, Error>;

    /// Obtain a proof for a single key
    async fn single_key_proof<K: KeyType, V: ValueType>(&self, key: K) -> Result<Proof<V>, Error>;

    /// Obtain a range proof over a set of keys
    ///
    /// # Arguments
    ///
    /// * `first_key` - If None, start at the lowest key
    /// * `last_key` - If None, continue to the end of the database
    /// * `limit` - The maximum number of keys in the range proof
    ///
    async fn range_proof<K: KeyType, V: ValueType>(
        &self,
        first_key: Option<K>,
        last_key: Option<K>,
        limit: usize,
    ) -> Result<RangeProof<K, V>, Error>;
}
