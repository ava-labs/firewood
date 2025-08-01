// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::fmt;

use firewood::v2::api;

use crate::value::BorrowedBytes;

/// A `KeyValue` represents a key-value pair, passed to the FFI.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct KeyValuePair<'a> {
    pub key: BorrowedBytes<'a>,
    pub value: BorrowedBytes<'a>,
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
    fn into_key_value(self) -> (Self::Key, Self::Value) {
        (self.key, self.value)
    }
}

impl<'a> api::KeyValuePair for &KeyValuePair<'a> {
    type Key = BorrowedBytes<'a>;
    type Value = BorrowedBytes<'a>;

    #[inline]
    fn into_key_value(self) -> (Self::Key, Self::Value) {
        (*self).into_key_value()
    }
}
