// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Proof reading utilities and traits.
//!
//! This module provides low-level utilities for reading binary proof data,
//! including the `ProofReader` type for sequential reading and traits for
//! deserializing individual proof components.

use firewood_storage::{DefaultHashMode, HashMode, NodeHashAlgorithm};

use super::header::{Header, InvalidHeader};
use std::num::NonZeroUsize;
pub(super) trait ReadItem<'a>: Sized {
    /// Reads an item from the given reader, or terrminates with an error.
    fn read_item(data: &mut ProofReader<'a>) -> Result<Self, ReadError>;
}

pub(super) trait Version0: Sized {
    /// The minimum number of bytes required to deserialize a single instance of this type.
    const MIN_BYTES_PER_ITEM: NonZeroUsize = NonZeroUsize::new(1).unwrap();

    fn read_v0_item(reader: &mut V0Reader<'_>) -> Result<Self, ReadError>;
}

pub(super) struct ProofReader<'a> {
    data: &'a [u8],
    offset: usize,
    /// The hash algorithm the proof being read was encoded with, resolved from
    /// the self-describing `hash_mode` header byte. Leaf reads whose wire layout
    /// differs per mode (e.g. [`HashType`](firewood_storage::HashType)) dispatch
    /// on this value so a single binary reads either wire format.
    node_hash_algorithm: NodeHashAlgorithm,
}

impl<'a> ProofReader<'a> {
    /// Creates a reader for parsing the fixed-size header, before the proof's
    /// own hash mode is known.
    ///
    /// The algorithm is seeded to the compile default; it is only consulted by
    /// body reads (after the header is parsed and the reader is re-seeded via
    /// [`ProofReader::set_node_hash_algorithm`]), so this placeholder never
    /// influences header parsing.
    #[must_use]
    pub const fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            offset: 0,
            node_hash_algorithm: DefaultHashMode::ALGORITHM,
        }
    }

    /// Re-seeds the reader with the hash algorithm resolved from the validated
    /// header so subsequent body reads dispatch on the proof's own mode.
    pub const fn set_node_hash_algorithm(&mut self, algorithm: NodeHashAlgorithm) {
        self.node_hash_algorithm = algorithm;
    }

    /// The hash algorithm the proof being read was encoded with.
    #[must_use]
    pub const fn node_hash_algorithm(&self) -> NodeHashAlgorithm {
        self.node_hash_algorithm
    }

    pub fn read_chunk<const N: usize>(&mut self) -> Result<&'a [u8; N], ReadError> {
        if let Some((chunk, _)) = self.remainder().split_first_chunk::<N>() {
            #[expect(clippy::arithmetic_side_effects)]
            {
                self.offset += N;
            }
            Ok(chunk)
        } else {
            Err(self.incomplete_item(std::any::type_name::<[u8; N]>(), N))
        }
    }

    pub fn read_slice(&mut self, n: usize) -> Result<&'a [u8], ReadError> {
        if self.remainder().len() >= n {
            #[expect(
                clippy::disallowed_methods,
                reason = "n is checked against remainder().len() on the line above"
            )]
            let (slice, _) = self.remainder().split_at(n);
            #[expect(clippy::arithmetic_side_effects)]
            {
                self.offset += n;
            }
            Ok(slice)
        } else {
            Err(self.incomplete_item("byte slice", n))
        }
    }

    pub(super) fn read_item<T: ReadItem<'a>>(&mut self) -> Result<T, ReadError> {
        T::read_item(self)
    }

    pub fn remainder(&self) -> &'a [u8] {
        #![expect(clippy::indexing_slicing)]
        &self.data[self.offset..]
    }

    pub fn advance(&mut self, n: usize) {
        #![expect(clippy::arithmetic_side_effects)]
        debug_assert!(self.offset + n <= self.data.len());
        self.offset += n;
    }

    #[must_use]
    pub const fn incomplete_item(&self, item: &'static str, expected: usize) -> ReadError {
        ReadError::IncompleteItem {
            item,
            offset: self.offset,
            expected,
            #[expect(clippy::arithmetic_side_effects)]
            found: self.data.len() - self.offset,
        }
    }

    #[must_use]
    pub fn invalid_item(
        &self,
        item: &'static str,
        expected: &'static str,
        found: impl ToString,
    ) -> ReadError {
        ReadError::InvalidItem {
            item,
            offset: self.offset,
            expected,
            found: found.to_string(),
        }
    }
}

pub(super) struct V0Reader<'a> {
    inner: ProofReader<'a>,
    header: Header,
}

impl<'a> V0Reader<'a> {
    #[must_use]
    pub fn new(inner: ProofReader<'a>, header: Header) -> Self {
        let this = Self { inner, header };
        debug_assert_eq!(this.header().version, 0);
        this
    }

    pub fn read_v0_item<T: Version0>(&mut self) -> Result<T, ReadError> {
        T::read_v0_item(self)
    }

    pub const fn header(&self) -> &Header {
        &self.header
    }
}

impl<'a> std::ops::Deref for V0Reader<'a> {
    type Target = ProofReader<'a>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl std::ops::DerefMut for V0Reader<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

/// Error that ocurred while reading an item from the byte stream.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    /// Insufficient data in the byte stream.
    #[error("incomplete {item} at offset {offset}: expected {expected} bytes, but found {found}")]
    IncompleteItem {
        /// The specific item that was trying to parse.
        item: &'static str,
        /// The offset in the byte stream where the error ocurred.
        offset: usize,
        /// The expected length of the input (for this item).
        expected: usize,
        /// The number of bytes found in the byte stream.
        found: usize,
    },
    /// An item was invalid after parsing.
    #[error("invalid {item} at offset {offset}: expected {expected}, but found {found}")]
    InvalidItem {
        /// The item that was trying to parse.
        item: &'static str,
        /// The offset in the byte stream where the error ocurred.
        offset: usize,
        /// A hint at what was expected.
        expected: &'static str,
        /// Message indicating what was actually found.
        found: String,
    },
    /// Failed to validate the header.
    #[error("invalid header: {0}")]
    InvalidHeader(InvalidHeader),
}

impl ReadError {
    pub(super) const fn set_item(mut self, item: &'static str) -> Self {
        match &mut self {
            Self::IncompleteItem { item: e_item, .. } | Self::InvalidItem { item: e_item, .. } => {
                *e_item = item;
            }
            Self::InvalidHeader(_) => {}
        }
        self
    }
}
