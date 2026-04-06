// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Proof deserialization implementation.
//!
//! This module handles the deserialization of proofs from the binary format.
//! It supports parsing proof headers, proof nodes, and range proofs with full
//! validation of the format.

use firewood_storage::{
    Children, HashType, NodeHashAlgorithm, PathBuf, TrieHash, TriePathFromUnpackedBytes,
    ValueDigest,
};
use integer_encoding::VarInt;

use super::{
    header::{Header, InvalidHeader},
    reader::{ProofReader, ReadError, ReadItem, V0Reader, Version0},
    types::{Proof, ProofNode, ProofType},
};
use crate::merkle::childmask::ChildMask;
use crate::{
    api::{FrozenChangeProof, FrozenRangeProof},
    db::BatchOp,
    merkle::{Key, Value},
    proofs::magic::{BATCH_DELETE, BATCH_DELETE_RANGE, BATCH_PUT},
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
    pub fn from_slice(data: &[u8]) -> Result<Self, ReadError> {
        let mut reader = ProofReader::new(data);

        let header = reader.read_item::<Header>()?;
        header
            .validate(Some(ProofType::Range))
            .map_err(ReadError::InvalidHeader)?;

        match header.version {
            0 => {
                let mut reader = V0Reader::new(reader, header);
                let this = reader.read_v0_item()?;
                if reader.remainder().is_empty() {
                    Ok(this)
                } else {
                    Err(reader.invalid_item(
                        "trailing bytes",
                        "no data after the proof",
                        format!("{} bytes", reader.remainder().len()),
                    ))
                }
            }
            found => Err(ReadError::InvalidHeader(
                InvalidHeader::UnsupportedVersion { found },
            )),
        }
    }
}

impl<T: Version0> Version0 for Box<[T]> {
    fn read_v0_item(reader: &mut V0Reader<'_>) -> Result<Self, ReadError> {
        let num_items = reader
            .read_item::<usize>()
            .map_err(|err| err.set_item("array length"))?;

        // FIXME: we must somehow validate `num_items` matches what is expected
        // An incorrect, or unexpectedly large value could lead to DoS via OOM
        // or panicing
        (0..num_items).map(|_| reader.read_v0_item()).collect()
    }
}

impl FrozenChangeProof {
    /// Parses a `FrozenChangeProof` from the given byte slice.
    ///
    /// Currently only V0 proofs are supported. See [`FrozenChangeProof::write_to_vec`]
    /// for the serialization format.
    ///
    /// # Errors
    ///
    /// Returns a [`ReadError`] if the data is invalid. See the enum variants for
    /// the possible reasons.
    pub fn from_slice(data: &[u8]) -> Result<Self, ReadError> {
        let mut reader = ProofReader::new(data);

        let header = reader.read_item::<Header>()?;
        header
            .validate(Some(ProofType::Change))
            .map_err(ReadError::InvalidHeader)?;

        if header.version != 0 {
            return Err(ReadError::InvalidHeader(
                InvalidHeader::UnsupportedVersion {
                    found: header.version,
                },
            ));
        }

        let mut reader = V0Reader::new(reader, header);
        let this = reader.read_v0_item()?;
        if reader.remainder().is_empty() {
            Ok(this)
        } else {
            Err(reader.invalid_item(
                "trailing bytes",
                "no data after the proof",
                format!("{} bytes", reader.remainder().len()),
            ))
        }
    }
}

impl Version0 for FrozenRangeProof {
    fn read_v0_item(reader: &mut V0Reader<'_>) -> Result<Self, ReadError> {
        let start_proof = reader.read_v0_item()?;
        let end_proof = reader.read_v0_item()?;
        let key_values = reader.read_v0_item()?;
        let node_hash_algorithm = reader
            .header()
            .node_hash_algorithm()
            .map_err(ReadError::InvalidHeader)?;

        Ok(Self::new(
            Proof::new_with_hash_algorithm(node_hash_algorithm, start_proof),
            Proof::new_with_hash_algorithm(node_hash_algorithm, end_proof),
            key_values,
        ))
    }
}

impl Version0 for FrozenChangeProof {
    fn read_v0_item(reader: &mut V0Reader<'_>) -> Result<Self, ReadError> {
        let start_proof = reader.read_v0_item()?;
        let end_proof = reader.read_v0_item()?;
        let key_values = reader.read_v0_item()?;
        let node_hash_algorithm = reader
            .header()
            .node_hash_algorithm()
            .map_err(ReadError::InvalidHeader)?;

        Ok(Self::new(
            Proof::new_with_hash_algorithm(node_hash_algorithm, start_proof),
            Proof::new_with_hash_algorithm(node_hash_algorithm, end_proof),
            key_values,
        ))
    }
}

impl Version0 for BatchOp<Key, Value> {
    fn read_v0_item(reader: &mut V0Reader<'_>) -> Result<Self, ReadError> {
        match reader
            .read_item::<u8>()
            .map_err(|err| err.set_item("option discriminant"))?
        {
            BATCH_PUT => Ok(BatchOp::Put {
                key: reader.read_item()?,
                value: reader.read_item()?,
            }),
            BATCH_DELETE => Ok(BatchOp::Delete {
                key: reader.read_item()?,
            }),
            BATCH_DELETE_RANGE => Ok(BatchOp::DeleteRange {
                prefix: reader.read_item()?,
            }),
            found => Err(reader.invalid_item("option discriminant", "0, 1, or 2", found)),
        }
    }
}

impl Version0 for ProofNode {
    fn read_v0_item(reader: &mut V0Reader<'_>) -> Result<Self, ReadError> {
        let node_hash_algorithm = reader
            .header()
            .node_hash_algorithm()
            .map_err(ReadError::InvalidHeader)?;
        let key = reader.read_v0_item()?;
        let partial_len = reader.read_item()?;
        let value_digest = read_value_digest_boxed(reader, node_hash_algorithm)?;

        let children_map = reader.read_item::<ChildMask>()?;

        let mut child_hashes = Children::new();
        for idx in children_map.iter_indices() {
            child_hashes[idx] = Some(read_hash_type(reader, node_hash_algorithm)?);
        }

        Ok(ProofNode {
            key,
            partial_len,
            value_digest,
            child_hashes,
        })
    }
}

impl Version0 for PathBuf {
    fn read_v0_item(reader: &mut V0Reader<'_>) -> Result<Self, ReadError> {
        let bytes = reader.read_item::<&[u8]>()?;
        TriePathFromUnpackedBytes::path_from_unpacked_bytes(bytes).map_err(|_| {
            reader.invalid_item(
                "path",
                "valid nibbles",
                format!("invalid nibbles: {}", hex::encode(bytes)),
            )
        })
    }
}

impl Version0 for (Box<[u8]>, Box<[u8]>) {
    fn read_v0_item(reader: &mut V0Reader<'_>) -> Result<Self, ReadError> {
        Ok((reader.read_item()?, reader.read_item()?))
    }
}

impl<'a> ReadItem<'a> for Header {
    fn read_item(reader: &mut ProofReader<'a>) -> Result<Self, ReadError> {
        reader
            .read_chunk::<{ size_of::<Header>() }>()
            .map_err(|err| err.set_item("header"))
            .copied()
            .map(bytemuck::cast)
    }
}

impl<'a> ReadItem<'a> for usize {
    fn read_item(reader: &mut ProofReader<'a>) -> Result<Self, ReadError> {
        match u64::decode_var(reader.remainder()) {
            Some((n, size)) => {
                reader.advance(size);
                Ok(n as usize)
            }
            None if reader.remainder().is_empty() => Err(reader.incomplete_item("varint", 1)),
            #[expect(clippy::indexing_slicing)]
            None => Err(reader.invalid_item(
                "varint",
                "byte with no MSB within 9 bytes",
                format!(
                    "{:?}",
                    &reader.remainder()[..reader.remainder().len().min(10)]
                ),
            )),
        }
    }
}

impl<'a> ReadItem<'a> for &'a [u8] {
    fn read_item(reader: &mut ProofReader<'a>) -> Result<Self, ReadError> {
        let len = reader.read_item::<usize>()?;
        reader.read_slice(len)
    }
}

impl<'a> ReadItem<'a> for Box<[u8]> {
    fn read_item(reader: &mut ProofReader<'a>) -> Result<Self, ReadError> {
        reader.read_item::<&[u8]>().map(Box::from)
    }
}

impl<'a> ReadItem<'a> for u8 {
    fn read_item(reader: &mut ProofReader<'a>) -> Result<Self, ReadError> {
        reader
            .read_chunk::<1>()
            .map(|&[b]| b)
            .map_err(|err| err.set_item("u8"))
    }
}

impl<'a, T: ReadItem<'a>> ReadItem<'a> for Option<T> {
    fn read_item(reader: &mut ProofReader<'a>) -> Result<Self, ReadError> {
        match reader
            .read_item::<u8>()
            .map_err(|err| err.set_item("option discriminant"))?
        {
            0 => Ok(None),
            1 => Ok(Some(reader.read_item::<T>()?)),
            found => Err(reader.invalid_item("option discriminant", "0 or 1", found)),
        }
    }
}

impl<'a> ReadItem<'a> for TrieHash {
    fn read_item(reader: &mut ProofReader<'a>) -> Result<Self, ReadError> {
        reader
            .read_chunk::<{ size_of::<TrieHash>() }>()
            .map_err(|err| err.set_item("trie hash"))
            .copied()
            .map(TrieHash::from)
    }
}

impl<'a> ReadItem<'a> for ChildMask {
    fn read_item(reader: &mut ProofReader<'a>) -> Result<Self, ReadError> {
        reader
            .read_chunk::<2>()
            .map_err(|err| err.set_item("children map"))
            .copied()
            .map(ChildMask::from_le_bytes)
    }
}

fn read_hash_type(
    reader: &mut ProofReader<'_>,
    node_hash_algorithm: NodeHashAlgorithm,
) -> Result<HashType, ReadError> {
    HashType::from_reader_for_node_hash_algorithm(&mut *reader, node_hash_algorithm)
        .map_err(|err| reader.invalid_item("hash type", "valid hash encoding", err))
}

fn read_value_digest_boxed(
    reader: &mut ProofReader<'_>,
    node_hash_algorithm: NodeHashAlgorithm,
) -> Result<Option<ValueDigest<Box<[u8]>>>, ReadError> {
    match reader
        .read_item::<u8>()
        .map_err(|err| err.set_item("option discriminant"))?
    {
        0 => Ok(None),
        1 => Ok(Some(
            read_value_digest(reader, node_hash_algorithm)?.map(Into::into),
        )),
        found => Err(reader.invalid_item("option discriminant", "0 or 1", found)),
    }
}

fn read_value_digest<'a>(
    reader: &mut ProofReader<'a>,
    node_hash_algorithm: NodeHashAlgorithm,
) -> Result<ValueDigest<&'a [u8]>, ReadError> {
    match reader
        .read_item::<u8>()
        .map_err(|err| err.set_item("value digest discriminant"))?
    {
        0 => Ok(ValueDigest::Value(reader.read_item()?)),
        1 => Ok(ValueDigest::Hash(read_hash_type(
            reader,
            node_hash_algorithm,
        )?)),
        found => {
            Err(reader.invalid_item("value digest discriminant", "0 (value) or 1 (hash)", found))
        }
    }
}
