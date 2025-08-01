// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::fmt;

/// A root database hash key, used in FFI functions that provide or return
/// a root hash. This type requires no allocation and can be copied freely
/// and droppped without any additional overhead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(transparent)]
pub struct HashKey([u8; 32]);

impl fmt::Display for HashKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        super::DisplayHex(&self.0).fmt(f)
    }
}

impl From<firewood::v2::api::HashKey> for HashKey {
    fn from(value: firewood::v2::api::HashKey) -> Self {
        Self(value.into())
    }
}

impl From<HashKey> for firewood::v2::api::HashKey {
    fn from(value: HashKey) -> Self {
        value.0.into()
    }
}
