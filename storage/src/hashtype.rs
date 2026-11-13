// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::fmt::Display;
use std::fmt::Formatter;
use std::io::Read;

use sha2::Digest as _;
use sha3::Keccak256;

use crate::TrieHash;
use crate::node::ExtendableBytes;
use crate::node::branch::Serializable;

/// The type of a hash. For ethereum compatible hashes, this might be a RLP encoded
/// value if it's small enough to fit in less than 32 bytes. For merkledb compatible
/// hashes, it's always a `TrieHash` (carried as the `Hash` variant).
///
/// This is an unconditional enum so that both hashing schemes can be compiled
/// into a single binary. The on-disk byte layout per scheme is governed by the
/// per-mode codec on [`HashMode`](crate::HashMode), not by this type directly.
#[derive(Clone, Copy)]
pub enum HashType {
    /// A full 32-byte trie hash. This is the only variant produced under
    /// merkledb-compatible hashing.
    Hash(TrieHash),
    /// An inline RLP encoding, used for ethereum-compatible hashes small enough
    /// (less than 32 bytes) to be stored in place of a hash.
    Rlp(InlineRlp),
}

/// An RLP encoding short enough to stand in for its own hash: at most
/// [`InlineRlp::MAX_LEN`] bytes, stored inline.
///
/// The Ethereum scheme refers to a node by its Keccak-256 hash only when the
/// node's encoding is 32 bytes or longer. A shorter encoding is embedded in the
/// parent as is.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct InlineRlp {
    len: u8,
    /// The encoding, followed by zeros. Unused bytes are always zero so the
    /// derived equality compares encodings.
    bytes: [u8; Self::MAX_LEN],
}

/// The error returned when an RLP encoding is too long to be stored inline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("an inline RLP encoding holds at most {max} bytes, got {0}", max = InlineRlp::MAX_LEN)]
#[non_exhaustive]
pub struct InlineRlpTooLong(pub usize);

impl InlineRlp {
    /// The longest encoding that is stored inline. An encoding one byte longer
    /// is replaced by its 32-byte hash.
    pub const MAX_LEN: usize = 31;

    /// Copies `rlp`, or returns `None` if it is longer than
    /// [`MAX_LEN`](Self::MAX_LEN).
    #[must_use]
    pub fn new(rlp: &[u8]) -> Option<Self> {
        let len = u8::try_from(rlp.len()).ok()?;
        let mut bytes = [0; Self::MAX_LEN];
        for (dst, src) in bytes.get_mut(..rlp.len())?.iter_mut().zip(rlp) {
            *dst = *src;
        }
        Some(Self { len, bytes })
    }

    /// Returns the encoding.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        self.bytes
            .get(..usize::from(self.len))
            .unwrap_or(&self.bytes)
    }
}

impl TryFrom<&[u8]> for InlineRlp {
    type Error = InlineRlpTooLong;

    fn try_from(rlp: &[u8]) -> Result<Self, Self::Error> {
        Self::new(rlp).ok_or(InlineRlpTooLong(rlp.len()))
    }
}

impl std::ops::Deref for InlineRlp {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl AsRef<[u8]> for InlineRlp {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl std::fmt::Debug for InlineRlp {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "InlineRlp({})", hex::encode(self.as_slice()))
    }
}

/// Manual implementation of [`Debug`](std::fmt::Debug) so that the RLP bytes
/// are displayed as hex rather than raw bytes, which is more useful for
/// debugging purposes.
impl std::fmt::Debug for HashType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            HashType::Hash(h) => write!(f, "Hash({h})"),
            HashType::Rlp(r) => write!(f, "Rlp({})", hex::encode(r)),
        }
    }
}

impl HashType {
    /// Creates a new `HashType` from the default value, which is all zeros (the
    /// `Hash` variant wrapping a zeroed `TrieHash`).
    ///
    /// ```
    /// assert_eq!(
    ///     firewood_storage::HashType::empty(),
    ///     firewood_storage::HashType::from([0; 32]),
    /// )
    /// ```
    #[must_use]
    pub fn empty() -> Self {
        TrieHash::empty().into()
    }

    /// Returns the underlying bytes as a slice: the raw hash for the `Hash`
    /// variant, or the RLP bytes for the `Rlp` variant.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        self
    }

    /// Converts into a [`TrieHash`], hashing the RLP bytes with Keccak-256 for
    /// the `Rlp` variant.
    #[must_use]
    pub fn into_triehash(self) -> TrieHash {
        self.into()
    }
}

impl PartialEq<TrieHash> for HashType {
    fn eq(&self, other: &TrieHash) -> bool {
        match self {
            HashType::Hash(h) => h == other,
            HashType::Rlp(r) => Keccak256::digest(r.as_ref()).as_slice() == other.as_ref(),
        }
    }
}

impl PartialEq<HashType> for TrieHash {
    fn eq(&self, other: &HashType) -> bool {
        match other {
            HashType::Hash(h) => h == self,
            HashType::Rlp(r) => Keccak256::digest(r.as_ref()).as_slice() == self.as_ref(),
        }
    }
}

impl PartialEq for HashType {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            // if both are hash or rlp, we can skip hashing
            (HashType::Hash(h1), HashType::Hash(h2)) => h1 == h2,
            (HashType::Rlp(r1), HashType::Rlp(r2)) => r1 == r2,
            // otherwise, one is a hash and the other isn't, so hash the rlp and compare
            (HashType::Hash(h), HashType::Rlp(r)) | (HashType::Rlp(r), HashType::Hash(h)) => {
                // avoid copying by comparing the hash directly; copying into a
                // TrieHash adds a noticeable overhead
                Keccak256::digest(r).as_slice() == h.as_ref()
            }
        }
    }
}

impl Eq for HashType {}

impl std::hash::Hash for HashType {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // contract on `Hash` and `PartialEq` requires that if `a == b` then `hash(a) == hash(b)`
        // and since `PartialEq` may require hashing, we must always convert to `TrieHash` here
        // and use it's hash implementation
        match self {
            Self::Hash(h) => h.hash(state),
            // NB: same `hash` impl as [u8; 32] which TrieHash also uses, prevents copying twice
            Self::Rlp(r) => Keccak256::digest(r.as_ref()).hash(state),
        }
    }
}

impl Serializable for HashType {
    fn write_to<W: ExtendableBytes>(&self, vec: &mut W) {
        match self {
            HashType::Hash(h) => {
                vec.push(0);
                vec.extend_from_slice(h.as_ref());
            }
            HashType::Rlp(r) => {
                debug_assert!(!r.is_empty());
                debug_assert!(r.len() < 32);
                vec.push(r.len() as u8);
                vec.extend_from_slice(r.as_ref());
            }
        }
    }

    fn from_reader<R: Read>(mut reader: R) -> Result<Self, std::io::Error> {
        let mut len = 0;
        reader.read_exact(std::slice::from_mut(&mut len))?;
        if len == 0 {
            // length is zero, so it's a full trie hash
            let mut bytes = [0; 32];
            reader.read_exact(&mut bytes)?;
            return Ok(HashType::Hash(TrieHash::from(bytes)));
        }

        let mut bytes = [0; InlineRlp::MAX_LEN];
        let Some(bytes_mut) = bytes.get_mut(..usize::from(len)) else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid RLP length; expected strictly less than 32, got {len}"),
            ));
        };

        reader.read_exact(bytes_mut)?;
        Ok(HashType::Rlp(InlineRlp { len, bytes }))
    }
}

impl From<HashType> for TrieHash {
    fn from(val: HashType) -> Self {
        match val {
            HashType::Hash(h) => h,
            HashType::Rlp(r) => Keccak256::digest(r).into(),
        }
    }
}

impl From<&HashType> for TrieHash {
    fn from(val: &HashType) -> Self {
        match val {
            HashType::Hash(h) => *h,
            HashType::Rlp(r) => Keccak256::digest(r).into(),
        }
    }
}

impl From<TrieHash> for HashType {
    fn from(val: TrieHash) -> Self {
        HashType::Hash(val)
    }
}

impl From<[u8; 32]> for HashType {
    fn from(value: [u8; 32]) -> Self {
        HashType::Hash(TrieHash::into(value.into()))
    }
}

impl TryFrom<&[u8]> for HashType {
    type Error = crate::InvalidTrieHashLength;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        value.try_into().map(HashType::Hash)
    }
}

impl AsRef<[u8]> for HashType {
    fn as_ref(&self) -> &[u8] {
        match self {
            HashType::Hash(h) => h.as_ref(),
            HashType::Rlp(r) => r.as_ref(),
        }
    }
}

impl std::ops::Deref for HashType {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        match self {
            HashType::Hash(h) => h.as_slice(),
            HashType::Rlp(r) => r,
        }
    }
}

impl Display for HashType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            HashType::Hash(h) => write!(f, "{h}"),
            HashType::Rlp(r) => {
                let width = f.precision().unwrap_or(32);
                write!(f, "{:.*}", width, hex::encode(r))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An encoding of up to `MAX_LEN` bytes round-trips, and one byte more is
    /// rejected by both constructors.
    #[test]
    fn inline_rlp_holds_at_most_max_len_bytes() {
        let longest = [0xAB; InlineRlp::MAX_LEN];
        for len in [0, 1, InlineRlp::MAX_LEN] {
            let rlp = InlineRlp::new(&longest[..len]).expect("fits inline");
            assert_eq!(rlp.as_slice(), &longest[..len]);
        }

        let too_long = [0xAB; InlineRlp::MAX_LEN + 1];
        assert_eq!(InlineRlp::new(&too_long), None);
        assert_eq!(
            InlineRlp::try_from(&too_long[..]),
            Err(InlineRlpTooLong(too_long.len()))
        );
    }

    /// Two encodings are equal exactly when their bytes are, including when
    /// one is a prefix of the other padded with zeros.
    #[test]
    fn inline_rlp_equality_compares_the_encoding() {
        let short = InlineRlp::new(&[0x80]).unwrap();
        let padded = InlineRlp::new(&[0x80, 0x00]).unwrap();
        assert_eq!(short, InlineRlp::new(&[0x80]).unwrap());
        assert_ne!(short, padded);
    }

    /// An inline encoding serializes as its length then its bytes and reads
    /// back unchanged; a length of 32 or more is rejected.
    #[test]
    fn hash_type_rlp_round_trips_and_rejects_long_lengths() {
        let encoding = [0xC2, 0x80, 0x80];
        let rlp = HashType::Rlp(InlineRlp::new(&encoding).unwrap());
        let mut bytes = Vec::new();
        rlp.write_to(&mut bytes);
        assert_eq!(bytes[0] as usize, encoding.len());
        assert_eq!(&bytes[1..], &encoding);
        assert_eq!(HashType::from_reader(&bytes[..]).unwrap(), rlp);

        let too_long = [vec![32u8], vec![0; 32]].concat();
        assert!(HashType::from_reader(&too_long[..]).is_err());
    }

    /// The hash of a child is one byte of tag on top of 32 bytes of payload.
    #[test]
    fn hash_type_is_33_bytes() {
        assert_eq!(size_of::<InlineRlp>(), 32);
        assert_eq!(size_of::<HashType>(), 33);
    }
}
