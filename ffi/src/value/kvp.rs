// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::fmt;

use crate::value::BorrowedBytes;
use crate::{OwnedBytes, OwnedSlice};
use firewood::v2::api;

/// A type alias for a rust-owned byte slice.
pub type OwnedKeyValueBatch = OwnedSlice<OwnedKeyValuePair>;

/// Tag indicating the type of batch operation.
///
/// This is used by the FFI to explicitly distinguish between different
/// operation types instead of relying on nil vs empty pointer semantics.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchOpTag {
    /// Insert or update a key with a value.
    /// The value may be empty (zero-length).
    Put = 0,
    /// Delete a specific key.
    /// The value field is ignored for this operation.
    Delete = 1,
    /// Delete all keys with a given prefix.
    /// The key field is used as the prefix.
    /// The value field is ignored for this operation.
    DeleteRange = 2,
}

/// A `KeyValue` represents a key-value pair with an operation tag, passed to the FFI.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct KeyValuePair<'a> {
    pub key: BorrowedBytes<'a>,
    pub value: BorrowedBytes<'a>,
    /// The operation type for this key-value pair.
    pub op: BatchOpTag,
}

impl<'a> KeyValuePair<'a> {
    /// Creates a new `KeyValuePair` with a `Put` operation.
    pub fn new((key, value): &'a (impl AsRef<[u8]>, impl AsRef<[u8]>)) -> Self {
        Self {
            key: BorrowedBytes::from_slice(key.as_ref()),
            value: BorrowedBytes::from_slice(value.as_ref()),
            op: BatchOpTag::Put,
        }
    }
}

impl fmt::Display for KeyValuePair<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let precision = f.precision().unwrap_or(64);
        write!(
            f,
            "Key: {:.precision$}, Value: {:.precision$}",
            self.key, self.value
        )
    }
}

impl<'a> api::TryIntoBatch for KeyValuePair<'a> {
    type Key = BorrowedBytes<'a>;
    type Value = BorrowedBytes<'a>;
    type Error = std::convert::Infallible;

    #[inline]
    fn try_into_batch(self) -> Result<api::BatchOp<Self::Key, Self::Value>, Self::Error> {
        Ok(match self.op {
            BatchOpTag::Put => api::BatchOp::Put {
                key: self.key,
                value: self.value,
            },
            BatchOpTag::Delete => api::BatchOp::Delete { key: self.key },
            BatchOpTag::DeleteRange => api::BatchOp::DeleteRange { prefix: self.key },
        })
    }
}

impl api::KeyValuePair for KeyValuePair<'_> {
    #[inline]
    fn try_into_tuple(self) -> Result<(Self::Key, Self::Value), Self::Error> {
        Ok((self.key, self.value))
    }
}

impl<'a> api::TryIntoBatch for &KeyValuePair<'a> {
    type Key = BorrowedBytes<'a>;
    type Value = BorrowedBytes<'a>;
    type Error = std::convert::Infallible;

    #[inline]
    fn try_into_batch(self) -> Result<api::BatchOp<Self::Key, Self::Value>, Self::Error> {
        (*self).try_into_batch()
    }
}

impl api::KeyValuePair for &KeyValuePair<'_> {
    #[inline]
    fn try_into_tuple(self) -> Result<(Self::Key, Self::Value), Self::Error> {
        (*self).try_into_tuple()
    }
}

/// Owned version of `KeyValuePair`, returned to ffi callers.
///
/// C callers must free this using [`crate::fwd_free_owned_kv_pair`],
/// not the C standard library's `free` function.
#[repr(C)]
#[derive(Debug, Clone)]
pub struct OwnedKeyValuePair {
    pub key: OwnedBytes,
    pub value: OwnedBytes,
}

impl From<(Box<[u8]>, Box<[u8]>)> for OwnedKeyValuePair {
    fn from(value: (Box<[u8]>, Box<[u8]>)) -> Self {
        OwnedKeyValuePair {
            key: value.0.into(),
            value: value.1.into(),
        }
    }
}
