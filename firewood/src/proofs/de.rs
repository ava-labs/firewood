// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#[cfg(feature = "ethhash")]
use firewood_storage::HashType;
use firewood_storage::{BranchNode, TrieHash, ValueDigest};
use integer_encoding::VarInt;

use crate::{
    proof::{Proof, ProofNode},
    proofs::marshaling::{ChildrenMap, Header, ProofType, magic},
    v2::api::FrozenRangeProof,
};

impl FrozenRangeProof {
    /// Parses a `FrozenRangeProof` from the given byte slice.
    ///
    /// Currently only V0 proofs are supported. See [`FrozenRangeProof::write_to_vec`]
    /// for the serialization format.
    ///
    /// # Errors
    ///
    /// Returns a [`ReadError`] if the data is invalid. See the enum variants for
    /// the possible reasons.
    pub fn from_slice(mut data: &[u8]) -> Result<Self, ReadError> {
        let header = data.read_item::<Header>()?;
        header
            .validate(Some(ProofType::Range))
            .map_err(ReadError::InvalidHeader)?;

        let (this, rest) = match header.version {
            0 => Self::parse_v0(&header, data).add_offset(size_of::<Header>()),
            found => Err(ReadError::InvalidHeader(
                InvalidHeader::UnsupportedVersion { found },
            )),
        }?;

        if rest.is_empty() {
            Ok(this)
        } else {
            Err(ReadError::InvalidItem {
                item: "trailing data",
                #[expect(clippy::arithmetic_side_effects)]
                offset: size_of::<Header>() + data.len() - rest.len(),
                expected: "none",
                found: format!("{} bytes", rest.len()),
            })
        }
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

/// Error when validating the header.
#[derive(Debug, thiserror::Error)]
pub enum InvalidHeader {
    /// Expected a static byte string to prefix the input.
    #[error("invalid magic: found {:016x}; expected {:016x}", u64::from_be_bytes(*found), u64::from_be_bytes(*magic::PROOF_HEADER))]
    InvalidMagic {
        /// The actual bytes found in place where the magic header was expected.
        found: [u8; 8],
    },
    /// The proof was encoded with an unrecognized version.
    #[error(
        "unsupported proof version: found {found:02x}; expected {:02x}",
        magic::PROOF_VERSION
    )]
    UnsupportedVersion {
        /// The version byte found instead of a supported version.
        found: u8,
    },
    /// The proof was encoded for an unsupported hash mode.
    #[error(
        "unsupported hash mode: found {found:02x} ({}); expected {:02x} ({})",
        magic::hash_mode_name(*found),
        magic::HASH_MODE,
        magic::hash_mode_name(magic::HASH_MODE)
    )]
    UnsupportedHashMode {
        /// The flag indicating which hash mode created this proof.
        found: u8,
    },
    /// The proof was encoded for an unsupported branching factor.
    #[error(
        "unsupported branch factor: found {}; expected {}",
        magic::widen_branch_factor(*found),
        magic::widen_branch_factor(magic::BRANCH_FACTOR)
    )]
    UnsupportedBranchFactor {
        /// The actual branch factor encoded in the header.
        found: u8,
    },
    /// The header indicated an unexpected or invalid proof type.
    #[error(
        "invalid proof type: found {found:02x} ({}); expected {}",
        ProofType::new(*found).map_or("unknown", ProofType::name),
        DisplayProofType(*expected),
    )]
    InvalidProofType {
        /// The flag from the header.
        found: u8,
        /// The expected type, if any. Otherwise any type was expected and we
        /// found an unknown value.
        expected: Option<ProofType>,
    },
}

struct DisplayProofType(Option<ProofType>);

impl std::fmt::Display for DisplayProofType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Some(pt) => write!(f, "{:02x} ({})", pt as u8, pt.name()),
            None => write!(f, "one of 0x00 (single), 0x01 (range), 0x02 (change)"),
        }
    }
}

impl Header {
    /// Validates the header, returning the discovered proof type if valid.
    ///
    /// If `expected_type` is `Some`, the proof type must match (in which case the return
    /// value can be ignored).
    ///
    /// # Errors
    ///
    /// Returns an [`InvalidHeader`] if the header is invalid. See the enum variants for
    /// possible reasons.
    fn validate(&self, expected_type: Option<ProofType>) -> Result<ProofType, InvalidHeader> {
        if self.magic != *magic::PROOF_HEADER {
            return Err(InvalidHeader::InvalidMagic { found: self.magic });
        }

        if self.version != magic::PROOF_VERSION {
            return Err(InvalidHeader::UnsupportedVersion {
                found: self.version,
            });
        }

        if self.hash_mode != magic::HASH_MODE {
            return Err(InvalidHeader::UnsupportedHashMode {
                found: self.hash_mode,
            });
        }

        if self.branch_factor != magic::BRANCH_FACTOR {
            return Err(InvalidHeader::UnsupportedBranchFactor {
                found: self.branch_factor,
            });
        }

        match (ProofType::new(self.proof_type), expected_type) {
            (None, expected) => Err(InvalidHeader::InvalidProofType {
                found: self.proof_type,
                expected,
            }),
            (Some(found), Some(expected)) if found != expected => {
                Err(InvalidHeader::InvalidProofType {
                    found: self.proof_type,
                    expected: Some(expected),
                })
            }
            (Some(found), _) => Ok(found),
        }
    }
}

trait Version0 {
    fn parse_v0<'a>(header: &Header, data: &'a [u8]) -> Result<(Self, &'a [u8]), ReadError>
    where
        Self: Sized;
}

impl<T: Version0> Version0 for Box<[T]> {
    fn parse_v0<'a>(header: &Header, mut data: &'a [u8]) -> Result<(Self, &'a [u8]), ReadError> {
        let data_len = data.len();
        let num_items = data.read_item::<usize>()?;
        let mut items = Vec::with_capacity(num_items);
        for _ in 0..num_items {
            #[expect(clippy::arithmetic_side_effects)]
            let offset = data_len - data.len();
            let item;
            (item, data) = T::parse_v0(header, data).add_offset(offset)?;
            items.push(item);
        }
        Ok((items.into_boxed_slice(), data))
    }
}

impl Version0 for FrozenRangeProof {
    #[expect(clippy::arithmetic_side_effects)]
    fn parse_v0<'a>(header: &Header, mut data: &'a [u8]) -> Result<(Self, &'a [u8]), ReadError> {
        let data_len = data.len();

        let start_proof;
        let end_proof;

        (start_proof, data) = Version0::parse_v0(header, data)?;

        #[expect(clippy::arithmetic_side_effects)]
        let offset = data_len - data.len();
        (end_proof, data) = Version0::parse_v0(header, data).add_offset(offset)?;

        let offset = data_len - data.len();
        let (key_values, data) = Version0::parse_v0(header, data).add_offset(offset)?;

        Ok((
            Self::new(Proof::new(start_proof), Proof::new(end_proof), key_values),
            data,
        ))
    }
}

impl Version0 for ProofNode {
    #[expect(clippy::arithmetic_side_effects)]
    fn parse_v0<'a>(_: &Header, mut data: &'a [u8]) -> Result<(Self, &'a [u8]), ReadError>
    where
        Self: Sized,
    {
        let data_len = data.len();

        let key = data.read_item()?;

        let offset = data_len - data.len();
        let partial_len = data.read_item().add_offset(offset)?;

        let offset = data_len - data.len();
        let value_digest = data.read_item().add_offset(offset)?;

        let children_map = data.read_item::<ChildrenMap>()?;

        let mut child_hashes = BranchNode::empty_children();
        for idx in children_map.iter_indices() {
            let offset = data_len - data.len();
            #[expect(clippy::indexing_slicing)]
            {
                child_hashes[idx] = Some(data.read_item().add_offset(offset)?);
            }
        }

        Ok((
            ProofNode {
                key,
                partial_len,
                value_digest,
                child_hashes,
            },
            data,
        ))
    }
}

impl Version0 for (Box<[u8]>, Box<[u8]>) {
    fn parse_v0<'a>(_: &Header, mut data: &'a [u8]) -> Result<(Self, &'a [u8]), ReadError> {
        let data_len = data.len();

        let key = data.read_item()?;

        #[expect(clippy::arithmetic_side_effects)]
        let offset = data_len - data.len();
        let value = data.read_item().add_offset(offset)?;

        Ok(((key, value), data))
    }
}

trait ReadItemExt<'a> {
    fn read_item<T: ReadItem<'a>>(&mut self) -> Result<T, ReadError>;
}

impl<'a> ReadItemExt<'a> for &'a [u8] {
    fn read_item<T: ReadItem<'a>>(&mut self) -> Result<T, ReadError> {
        let (item, rest) = T::read_item(self)?;
        *self = rest;
        Ok(item)
    }
}

trait ReadItem<'a>: Sized {
    fn read_item(data: &'a [u8]) -> Result<(Self, &'a [u8]), ReadError>;
}

impl<'a> ReadItem<'a> for Header {
    fn read_item(data: &'a [u8]) -> Result<(Self, &'a [u8]), ReadError> {
        match data.split_first_chunk::<{ size_of::<Header>() }>() {
            Some((&header, rest)) => Ok((bytemuck::cast(header), rest)),
            None => Err(ReadError::IncompleteItem {
                item: "header",
                offset: 0,
                expected: size_of::<Header>(),
                found: data.len(),
            }),
        }
    }
}

impl<'a> ReadItem<'a> for usize {
    fn read_item(data: &'a [u8]) -> Result<(Self, &'a [u8]), ReadError> {
        match u64::decode_var(data) {
            #[expect(clippy::indexing_slicing)]
            Some((n, size)) => Ok((n as usize, &data[size..])),
            None => Err(ReadError::IncompleteItem {
                item: "varint",
                offset: 0,
                expected: 1, // at least 1 byte is needed
                found: data.len(),
            }),
        }
    }
}

impl<'a> ReadItem<'a> for &'a [u8] {
    fn read_item(mut data: &'a [u8]) -> Result<(Self, &'a [u8]), ReadError> {
        let data_len = data.len();
        let len = data.read_item::<usize>()?;
        if data.len() >= len {
            let (item, rest) = data.split_at(len);
            Ok((item, rest))
        } else {
            Err(ReadError::IncompleteItem {
                item: "byte array",
                #[expect(clippy::arithmetic_side_effects)]
                offset: data_len - data.len(),
                expected: len,
                found: data.len(),
            })
        }
    }
}

impl<'a> ReadItem<'a> for Box<[u8]> {
    fn read_item(mut data: &'a [u8]) -> Result<(Self, &'a [u8]), ReadError> {
        let slice = data.read_item::<&[u8]>()?;
        Ok((slice.to_vec().into_boxed_slice(), data))
    }
}

impl<'a> ReadItem<'a> for u8 {
    fn read_item(data: &'a [u8]) -> Result<(Self, &'a [u8]), ReadError> {
        match data.split_first() {
            Some((&b, rest)) => Ok((b, rest)),
            None => Err(ReadError::IncompleteItem {
                item: "u8",
                offset: 0,
                expected: 1,
                found: 0,
            }),
        }
    }
}

impl<'a, T: ReadItem<'a>> ReadItem<'a> for Option<T> {
    fn read_item(mut data: &'a [u8]) -> Result<(Self, &'a [u8]), ReadError> {
        match data.read_item::<u8>()? {
            0 => Ok((None, data)),
            1 => Ok((Some(data.read_item::<T>().add_offset(1)?), data)),
            found => Err(ReadError::InvalidItem {
                item: "option discriminant",
                offset: 0,
                expected: "0 or 1",
                found: found.to_string(),
            }),
        }
    }
}

impl<'a> ReadItem<'a> for ValueDigest<&'a [u8]> {
    fn read_item(mut data: &'a [u8]) -> Result<(Self, &'a [u8]), ReadError> {
        match data.read_item::<u8>()? {
            0 => Ok((ValueDigest::Value(data.read_item().add_offset(1)?), data)),
            #[cfg(not(feature = "ethhash"))]
            1 => Ok((ValueDigest::Hash(data.read_item().add_offset(1)?), data)),
            found => Err(ReadError::InvalidItem {
                item: "value digest type",
                offset: 0,
                expected: "0 (value) or 1 (hash)",
                found: found.to_string(),
            }),
        }
    }
}

impl<'a> ReadItem<'a> for ValueDigest<Box<[u8]>> {
    fn read_item(data: &'a [u8]) -> Result<(Self, &'a [u8]), ReadError> {
        match ValueDigest::<&[u8]>::read_item(data) {
            Ok((ValueDigest::Value(v), rest)) => Ok((ValueDigest::Value(v.into()), rest)),
            #[cfg(not(feature = "ethhash"))]
            Ok((ValueDigest::Hash(h), rest)) => Ok((ValueDigest::Hash(h), rest)),
            Err(e) => Err(e),
        }
    }
}

impl<'a> ReadItem<'a> for TrieHash {
    fn read_item(data: &'a [u8]) -> Result<(Self, &'a [u8]), ReadError> {
        match data.split_first_chunk::<32>() {
            Some((&hash, rest)) => Ok((hash.into(), rest)),
            None => Err(ReadError::IncompleteItem {
                item: "triehash",
                offset: 0,
                expected: 32,
                found: data.len(),
            }),
        }
    }
}

impl<'a> ReadItem<'a> for ChildrenMap {
    fn read_item(data: &'a [u8]) -> Result<(Self, &'a [u8]), ReadError> {
        match data.split_first_chunk::<{ size_of::<ChildrenMap>() }>() {
            Some((&cm, rest)) => Ok((bytemuck::cast(cm), rest)),
            None => Err(ReadError::IncompleteItem {
                item: "children map",
                offset: 0,
                expected: size_of::<ChildrenMap>(),
                found: data.len(),
            }),
        }
    }
}

#[cfg(feature = "ethhash")]
impl<'a> ReadItem<'a> for HashType {
    fn read_item(mut data: &'a [u8]) -> Result<(Self, &'a [u8]), ReadError> {
        match data.read_item::<u8>()? {
            0 => Ok((HashType::Hash(data.read_item().add_offset(1)?), data)),
            1 => Ok((
                HashType::Rlp(data.read_item::<&[u8]>().add_offset(1)?.into()),
                data,
            )),
            found => Err(ReadError::InvalidItem {
                item: "hash type",
                offset: 0,
                expected: "0 (hash) or 1 (rlp)",
                found: found.to_string(),
            }),
        }
    }
}

trait AddOffset {
    fn add_offset(self, offset: usize) -> Self;
}

impl AddOffset for ReadError {
    fn add_offset(mut self, offset: usize) -> Self {
        match &mut self {
            ReadError::IncompleteItem {
                offset: err_offset, ..
            }
            | ReadError::InvalidItem {
                offset: err_offset, ..
            } => {
                *err_offset = err_offset.saturating_add(offset);
            }
            ReadError::InvalidHeader(_) => {}
        }

        self
    }
}

impl<T, E: AddOffset> AddOffset for Result<T, E> {
    fn add_offset(self, offset: usize) -> Self {
        self.map_err(|e| e.add_offset(offset))
    }
}
