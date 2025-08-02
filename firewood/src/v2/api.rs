// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::manager::RevisionManagerError;
use crate::merkle::{Key, Value};
use crate::proof::{Proof, ProofError, ProofNode};
pub use crate::range_proof::RangeProof;
use async_trait::async_trait;
use firewood_storage::{FileIoError, TrieHash};
use futures::Stream;
use std::fmt::Debug;
use std::num::NonZeroUsize;
use std::sync::Arc;

/// A `KeyType` is something that can be xcast to a u8 reference,
/// and can be sent and shared across threads. References with
/// lifetimes are not allowed (hence 'static)
pub trait KeyType: AsRef<[u8]> + Send + Sync + Debug {}

impl<T> KeyType for T where T: AsRef<[u8]> + Send + Sync + Debug {}

/// A `ValueType` is the same as a `KeyType`. However, these could
/// be a different type from the `KeyType` on a given API call.
/// For example, you might insert `{key: "key", value: [0u8]}`
/// This also means that the type of all the keys for a single
/// API call must be the same, as well as the type of all values
/// must be the same.
pub trait ValueType: AsRef<[u8]> + Send + Sync + Debug {}

impl<T> ValueType for T where T: AsRef<[u8]> + Send + Sync + Debug {}

/// The type and size of a single hash key
/// These are 256-bit hashes that are used for a variety of reasons:
///  - They identify a version of the datastore at a specific point
///    in time
///  - They are used to provide integrity at different points in a
///    proof
pub type HashKey = firewood_storage::TrieHash;

/// A frozen proof is a proof that is stored in immutable memory.
pub type FrozenRangeProof = RangeProof<Key, Value, Box<[ProofNode]>>;

/// A frozen proof uses an immutable collection of proof nodes.
pub type FrozenProof = Proof<Box<[ProofNode]>>;

/// A key/value pair operation. Only put (upsert) and delete are
/// supported
#[derive(Debug, Clone)]
pub enum BatchOp<K: KeyType, V: ValueType> {
    /// Upsert a key/value pair
    Put {
        /// the key
        key: K,
        /// the value
        value: V,
    },

    /// Delete a key
    Delete {
        /// The key
        key: K,
    },

    /// Delete a range of keys by prefix
    DeleteRange {
        /// The prefix of the keys to delete
        prefix: K,
    },
}

impl<K: KeyType, V: ValueType> BatchOp<K, V> {
    /// Get the key of this operation
    #[must_use]
    pub const fn key(&self) -> &K {
        match self {
            BatchOp::Put { key, .. }
            | BatchOp::Delete { key }
            | BatchOp::DeleteRange { prefix: key } => key,
        }
    }

    /// Get the value of this operation
    #[must_use]
    pub const fn value(&self) -> Option<&V> {
        match self {
            BatchOp::Put { value, .. } => Some(value),
            _ => None,
        }
    }
}

impl<K: KeyType, V: ValueType> PartialOrd for BatchOp<K, V> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        use BatchOp::{Delete, DeleteRange, Put};
        use std::cmp::Ordering::Equal;
        match <[u8] as Ord>::cmp(self.key().as_ref(), other.key().as_ref()) {
            Equal => {}
            ord => return Some(ord),
        }

        match (self, other) {
            (Put { value: v1, .. }, Put { value: v2, .. }) => {
                Some(<[u8]>::cmp(v1.as_ref(), v2.as_ref()))
            }
            (Delete { .. }, Delete { .. }) | (DeleteRange { .. }, DeleteRange { .. }) => {
                Some(Equal)
            }
            // if the keys are the same but the operations differ, we cannot apply a
            // total ordering, so we return None. Anything attempting to sort two
            // differently typed operations with the same key must use a stable sort to
            // ensure that their relative order is preserved, otherwise they should fail.
            _ => None,
        }
    }
}

impl<K: KeyType, V: ValueType> PartialEq for BatchOp<K, V> {
    fn eq(&self, other: &Self) -> bool {
        // unlike ordering, we can always compare two operations with the same key.
        // they are equivalent if they have the same key and value (or absence of
        // value). Therefore, Delete and DeleteRange operations are equivalent if
        // they have the same key, regardless of the operation type.
        <[u8]>::eq(self.key().as_ref(), other.key().as_ref())
            && <Option<&[u8]>>::eq(
                &self.value().map(AsRef::as_ref),
                &other.value().map(AsRef::as_ref),
            )
    }
}

/// A key/value pair that can be used in a batch.
pub trait KeyValuePair {
    /// The key type
    type Key: KeyType;

    /// The value type
    type Value: ValueType;

    /// Convert this into a key/value tuple
    fn into_key_value(self) -> (Self::Key, Self::Value);

    /// Convert this key-value pair into a [`BatchOp`]. Other implementations may
    /// override this to provide more specific behavior like using [`BatchOp::Delete`]
    /// when appropriate.
    #[inline]
    #[must_use]
    fn into_batch(self) -> BatchOp<Self::Key, Self::Value>
    where
        Self: Sized,
    {
        let (key, value) = self.into_key_value();
        if value.as_ref().is_empty() {
            BatchOp::DeleteRange { prefix: key }
        } else {
            BatchOp::Put { key, value }
        }
    }
}

impl<K: KeyType, V: ValueType> KeyValuePair for (K, V) {
    type Key = K;
    type Value = V;

    fn into_key_value(self) -> (Self::Key, Self::Value) {
        self
    }
}

/// An extension trait for iterators that yield [`KeyValuePair`]s.
pub trait KeyValuePairIter: Iterator<Item: KeyValuePair> {
    /// Maps the items of this iterator into [`BatchOp`]s.
    #[inline]
    fn map_into_batch(self) -> KeyValuePairIterMapIntoBatch<Self>
    where
        Self: Sized,
        Self::Item: KeyValuePair,
    {
        self.map(KeyValuePair::into_batch)
    }
}

impl<I: Iterator<Item: KeyValuePair>> KeyValuePairIter for I {}

/// An iterator that converts a [`KeyValuePair`] into a [`BatchOp`] on yielded items.
// gnarly type-alias instead of a newtype to avoid needing to implement low level
// iterator traits.
pub type KeyValuePairIterMapIntoBatch<I> = std::iter::Map<
    I,
    fn(
        <I as Iterator>::Item,
    ) -> BatchOp<
        <<I as Iterator>::Item as KeyValuePair>::Key,
        <<I as Iterator>::Item as KeyValuePair>::Value,
    >,
>;

/// Errors returned through the API
#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum Error {
    /// A given hash key is not available in the database
    #[error("Hash not found for key: {provided:?}")]
    HashNotFound {
        /// the provided hash key
        provided: HashKey,
    },

    /// Incorrect root hash for commit
    #[error("Incorrect root hash for commit: {provided:?} != {expected:?}")]
    IncorrectRootHash {
        /// the provided root hash
        provided: HashKey,
        /// the expected root hash
        expected: HashKey,
    },

    /// Invalid range
    #[error("Invalid range: {start_key:?} > {end_key:?}")]
    InvalidRange {
        /// The provided starting key
        start_key: Key,
        /// The provided ending key
        end_key: Key,
    },

    #[error("IO error: {0}")]
    /// An IO error occurred
    IO(#[from] std::io::Error),

    #[error("File IO error: {0}")]
    /// A file I/O error occurred
    FileIO(#[from] FileIoError),

    /// Cannot commit a committed proposal
    #[error("Cannot commit a committed proposal")]
    AlreadyCommitted,

    /// Internal error
    #[error("Internal error")]
    InternalError(Box<dyn std::error::Error + Send>),

    /// Range too small
    #[error("Range too small")]
    RangeTooSmall,

    /// Request `RangeProof` for empty trie
    #[error("request RangeProof for empty trie")]
    RangeProofOnEmptyTrie,

    /// Request `RangeProof` for empty range
    #[error("the latest revision is empty and has no root hash")]
    LatestIsEmpty,

    /// Sibling already committed
    #[error("sibling already committed")]
    SiblingCommitted,

    /// Proof error
    #[error("proof error")]
    ProofError(#[from] ProofError),

    /// An invalid root hash was provided
    #[error("provided root hash is not a valid hash key")]
    InvalidRootHash(#[from] firewood_storage::InvalidTrieHashLength),
}

impl From<RevisionManagerError> for Error {
    fn from(err: RevisionManagerError) -> Self {
        match err {
            RevisionManagerError::FileIoError(io_err) => Error::FileIO(io_err),
            RevisionManagerError::NotLatest { provided, expected } => {
                Error::IncorrectRootHash { provided, expected }
            }
            RevisionManagerError::RevisionNotFound { provided } => Error::HashNotFound { provided },
        }
    }
}

impl From<crate::db::DbError> for Error {
    fn from(value: crate::db::DbError) -> Self {
        match value {
            crate::db::DbError::FileIo(err) => Error::FileIO(err),
        }
    }
}

/// The database interface. The methods here operate on the most
/// recently committed revision, and allow the creation of a new
/// [Proposal] or a new [DbView] based on a specific historical
/// revision.
#[async_trait]
pub trait Db {
    /// The type of a historical revision
    type Historical: DbView;

    /// The type of a proposal
    type Proposal<'db>: DbView + Proposal
    where
        Self: 'db;

    /// Get a reference to a specific view based on a hash
    ///
    /// # Arguments
    ///
    /// - `hash` - Identifies the revision for the view
    async fn revision(&self, hash: TrieHash) -> Result<Arc<Self::Historical>, Error>;

    /// Get the hash of the most recently committed version
    ///
    /// # Note
    ///
    /// If the database is empty, this will return None, unless the ethhash feature is enabled.
    /// In that case, we return the special ethhash compatible empty trie hash.
    async fn root_hash(&self) -> Result<Option<TrieHash>, Error>;

    /// Get all the hashes available
    async fn all_hashes(&self) -> Result<Vec<TrieHash>, Error>;

    /// Propose a change to the database via a batch
    ///
    /// This proposal assumes it is based off the most recently
    /// committed transaction
    ///
    /// # Arguments
    ///
    /// * `data` - A batch consisting of [BatchOp::Put] and
    ///            [BatchOp::Delete] operations to apply
    ///
    async fn propose<'db, K: KeyType, V: ValueType>(
        &'db self,
        data: (impl IntoIterator<Item = BatchOp<K, V>> + Send),
    ) -> Result<Self::Proposal<'db>, Error>
    where
        Self: 'db;
}

/// A view of the database at a specific time.
///
/// There are a few ways to create a [DbView]:
/// 1. From [Db::revision] which gives you a view for a specific
///    historical revision
/// 2. From [Db::propose] which is a view on top of the most recently
///    committed revision with changes applied; or
/// 3. From [Proposal::propose] which is a view on top of another proposal.
#[async_trait]
pub trait DbView {
    /// The type of a stream of key/value pairs
    type Stream<'view>: Stream<Item = Result<(Key, Value), Error>>
    where
        Self: 'view;

    /// Get the root hash for the current DbView
    ///
    /// # Note
    ///
    /// If the database is empty, this will return None, unless the ethhash feature is enabled.
    /// In that case, we return the special ethhash compatible empty trie hash.
    async fn root_hash(&self) -> Result<Option<HashKey>, Error>;

    /// Get the value of a specific key
    async fn val<K: KeyType>(&self, key: K) -> Result<Option<Value>, Error>;

    /// Obtain a proof for a single key
    async fn single_key_proof<K: KeyType>(&self, key: K) -> Result<FrozenProof, Error>;

    /// Obtain a range proof over a set of keys
    ///
    /// # Arguments
    ///
    /// * `first_key` - If None, start at the lowest key
    /// * `last_key` - If None, continue to the end of the database
    /// * `limit` - The maximum number of keys in the range proof
    ///
    async fn range_proof<K: KeyType>(
        &self,
        first_key: Option<K>,
        last_key: Option<K>,
        limit: Option<NonZeroUsize>,
    ) -> Result<FrozenRangeProof, Error>;

    /// Obtain a stream over the keys/values of this view, using an optional starting point
    ///
    /// # Arguments
    ///
    /// * `first_key` - If None, start at the lowest key
    ///
    /// # Note
    ///
    /// If you always want to start at the beginning, [DbView::iter] is easier to use
    /// If you always provide a key, [DbView::iter_from] is easier to use
    ///
    #[expect(clippy::missing_errors_doc)]
    fn iter_option<K: KeyType>(&self, first_key: Option<K>) -> Result<Self::Stream<'_>, Error>;

    /// Obtain a stream over the keys/values of this view, starting from the beginning
    #[expect(clippy::missing_errors_doc)]
    #[expect(clippy::iter_not_returning_iterator)]
    fn iter(&self) -> Result<Self::Stream<'_>, Error> {
        self.iter_option(Option::<Key>::None)
    }

    /// Obtain a stream over the key/values, starting at a specific key
    #[expect(clippy::missing_errors_doc)]
    fn iter_from<K: KeyType + 'static>(&self, first_key: K) -> Result<Self::Stream<'_>, Error> {
        self.iter_option(Some(first_key))
    }
}

/// A proposal for a new revision of the database.
///
/// A proposal may be committed, which consumes the
/// [Proposal] and return the generic type T, which
/// is the same thing you get if you call [Db::root_hash]
/// immediately after committing, and then call
/// [Db::revision] with the returned revision.
///
/// A proposal type must also implement everything in a
/// [DbView], which means you can fetch values from it or
/// obtain proofs.
#[async_trait]
pub trait Proposal: DbView + Send + Sync {
    /// The type of a proposal
    type Proposal: DbView + Proposal;

    /// Commit this revision
    async fn commit(self) -> Result<(), Error>;

    /// Propose a new revision on top of an existing proposal
    ///
    /// # Arguments
    ///
    /// * `data` - the batch changes to apply
    ///
    /// # Return value
    ///
    /// A new proposal
    ///
    async fn propose<K: KeyType, V: ValueType>(
        &self,
        data: (impl IntoIterator<Item = BatchOp<K, V>> + Send),
    ) -> Result<Self::Proposal, Error>;
}
