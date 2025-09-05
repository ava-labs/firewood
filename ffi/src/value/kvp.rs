// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::fmt;

use crate::OwnedBytes;
use crate::value::BorrowedBytes;
use firewood::v2::api;

/// A `KeyValue` represents a key-value pair, passed to the FFI.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct KeyValuePair<'a> {
    pub key: BorrowedBytes<'a>,
    pub value: BorrowedBytes<'a>,
}

/// Owned version of `KeyValuePair`, returned to the FFI.
#[repr(C)]
#[derive(Debug, Clone)]
pub struct OwnedKeyValuePair {
    pub key: OwnedBytes,
    pub value: OwnedBytes,
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

impl<'a> api::KeyValuePair for KeyValuePair<'a> {
    type Key = BorrowedBytes<'a>;
    type Value = BorrowedBytes<'a>;

    #[inline]
    fn into_batch(self) -> api::BatchOp<Self::Key, Self::Value> {
        if self.value.is_empty() {
            api::BatchOp::DeleteRange { prefix: self.key }
        } else {
            api::BatchOp::Put {
                key: self.key,
                value: self.value,
            }
        }
    }
}

impl<'a> api::KeyValuePair for &KeyValuePair<'a> {
    type Key = BorrowedBytes<'a>;
    type Value = BorrowedBytes<'a>;

    #[inline]
    fn into_batch(self) -> api::BatchOp<Self::Key, Self::Value> {
        (*self).into_batch()
    }
}

impl From<(Box<[u8]>, Box<[u8]>)> for OwnedKeyValuePair {
    fn from(value: (Box<[u8]>, Box<[u8]>)) -> Self {
        OwnedKeyValuePair {
            key: value.0.into(),
            value: value.1.into(),
        }
    }
}
