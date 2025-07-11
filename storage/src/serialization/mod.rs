// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#[cfg(any(test, not(feature = "serde")))]
pub(crate) mod manual;

#[cfg(feature = "serde")]
pub(crate) mod serde;

use std::io::{Read, Write};

#[cfg(not(feature = "serde"))]
pub(crate) use self::manual::{ReaderExt, SerializeToVec};

#[cfg(feature = "serde")]
pub(crate) use self::serde::{ReaderExt, SerializeToVec};

pub(crate) trait Serializable {
    fn write_to<W: ExtendableBytes>(&self, vec: &mut W);

    fn from_reader<R: Read>(reader: R) -> std::io::Result<Self>
    where
        Self: Sized;
}

// TODO: Unstable extend_reserve re-implemented here
// Extend<A>::extend_reserve is unstable so we implement it here
// see https://github.com/rust-lang/rust/issues/72631
pub trait ExtendableBytes: Write {
    fn extend<T: IntoIterator<Item = u8>>(&mut self, other: T);
    fn reserve(&mut self, reserve: usize) {
        let _ = reserve;
    }
    fn push(&mut self, value: u8);

    fn extend_from_slice(&mut self, other: &[u8]) {
        self.extend(other.iter().copied());
    }
}

impl ExtendableBytes for Vec<u8> {
    fn extend<T: IntoIterator<Item = u8>>(&mut self, other: T) {
        std::iter::Extend::extend(self, other);
    }
    fn reserve(&mut self, reserve: usize) {
        Vec::reserve(self, reserve);
    }
    fn push(&mut self, value: u8) {
        Vec::push(self, value);
    }
    fn extend_from_slice(&mut self, other: &[u8]) {
        Vec::extend_from_slice(self, other);
    }
}
