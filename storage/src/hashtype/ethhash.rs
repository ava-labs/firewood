// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use sha2::Digest as _;
use sha3::Keccak256;
use std::{
    fmt::{Display, Formatter},
    io::Read,
};

use crate::{
    TrieHash,
    node::{ExtendableBytes, branch::Serializable},
};

const RLP_MAX_LEN: usize = 31;

/// Compact inline representation for short RLP bytes.
#[derive(Clone, Debug)]
pub struct RlpBytes {
    len: u8,
    bytes: [u8; RLP_MAX_LEN],
}

impl RlpBytes {
    /// Maximum encoded byte length for inline Ethereum RLP payloads.
    pub const MAX_LEN: usize = RLP_MAX_LEN;

    #[must_use]
    #[inline]
    pub(crate) fn from_buf_and_len(bytes: [u8; RLP_MAX_LEN], len: u8) -> Self {
        debug_assert!(len > 0);
        debug_assert!((len as usize) <= Self::MAX_LEN);
        Self { len, bytes }
    }

    /// Creates inline RLP bytes when the length is in the valid `1..=31` range.
    #[must_use]
    #[inline]
    pub fn try_from_slice(bytes: &[u8]) -> Option<Self> {
        if bytes.is_empty() || bytes.len() > Self::MAX_LEN {
            return None;
        }

        let mut buf = [0; RLP_MAX_LEN];
        let out = buf.get_mut(..bytes.len())?;
        out.iter_mut().zip(bytes).for_each(|(dst, src)| *dst = *src);
        Some(Self {
            len: bytes.len() as u8,
            bytes: buf,
        })
    }

    /// Creates inline RLP bytes, panicking if the length is outside `1..=31`.
    ///
    /// # Panics
    ///
    /// Panics when `bytes.len()` is `0` or greater than `31`.
    #[must_use]
    #[track_caller]
    #[inline]
    pub fn from_slice(bytes: &[u8]) -> Self {
        Self::try_from_slice(bytes).expect("RLP length must be 1..=31 bytes")
    }

    /// Returns the encoded byte length.
    #[must_use]
    #[inline]
    pub const fn len_u8(&self) -> u8 {
        self.len
    }

    /// Returns the encoded bytes.
    ///
    /// # Panics
    ///
    /// Panics only if the internal length invariant is violated. All constructors
    /// bound `len` to `1..=MAX_LEN`.
    #[must_use]
    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        self.bytes
            .get(..usize::from(self.len))
            .expect("RlpBytes len is bounded by inline buffer")
    }
}

impl PartialEq for RlpBytes {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl Eq for RlpBytes {}

impl std::hash::Hash for RlpBytes {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.as_slice().hash(state);
    }
}

impl AsRef<[u8]> for RlpBytes {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl std::ops::Deref for RlpBytes {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

#[derive(Clone)]
pub enum HashOrRlp {
    Hash(TrieHash),
    Rlp(RlpBytes),
}

/// Manual implementation of [`Debug`](std::fmt::Debug) so that the RLP bytes
/// are displayed as hex rather than raw bytes, which is more useful for
/// debugging purposes.
impl std::fmt::Debug for HashOrRlp {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            HashOrRlp::Hash(h) => write!(f, "Hash({h})"),
            HashOrRlp::Rlp(r) => write!(f, "Rlp({})", hex::encode(r)),
        }
    }
}

impl HashOrRlp {
    /// Creates a new `TrieHash` from the default value, which is the all zeros.
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

    /// Creates a hash type from inline RLP bytes.
    ///
    /// # Panics
    ///
    /// Panics when `bytes.len()` is `0` or greater than `31`.
    #[must_use]
    #[inline]
    pub fn from_rlp_slice(bytes: &[u8]) -> Self {
        HashOrRlp::Rlp(RlpBytes::from_slice(bytes))
    }

    /// Creates a hash type from inline RLP bytes when the length is valid.
    #[must_use]
    #[inline]
    pub fn try_from_rlp_slice(bytes: &[u8]) -> Option<Self> {
        RlpBytes::try_from_slice(bytes).map(HashOrRlp::Rlp)
    }

    pub fn as_slice(&self) -> &[u8] {
        self
    }

    pub fn into_triehash(self) -> TrieHash {
        self.into()
    }
}

impl PartialEq<TrieHash> for HashOrRlp {
    fn eq(&self, other: &TrieHash) -> bool {
        match self {
            HashOrRlp::Hash(h) => h == other,
            HashOrRlp::Rlp(r) => Keccak256::digest(r.as_ref()).as_slice() == other.as_ref(),
        }
    }
}

impl PartialEq<HashOrRlp> for TrieHash {
    fn eq(&self, other: &HashOrRlp) -> bool {
        match other {
            HashOrRlp::Hash(h) => h == self,
            HashOrRlp::Rlp(r) => Keccak256::digest(r.as_ref()).as_slice() == self.as_ref(),
        }
    }
}

impl PartialEq for HashOrRlp {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            // if both are hash or rlp, we can skip hashing
            (HashOrRlp::Hash(h1), HashOrRlp::Hash(h2)) => h1 == h2,
            (HashOrRlp::Rlp(r1), HashOrRlp::Rlp(r2)) => r1 == r2,
            // otherwise, one is a hash and the other isn't, so hash the rlp and compare
            (HashOrRlp::Hash(h), HashOrRlp::Rlp(r)) | (HashOrRlp::Rlp(r), HashOrRlp::Hash(h)) => {
                // avoid copying by comparing the hash directly; copying into a
                // TrieHash adds a noticeable overhead
                Keccak256::digest(r).as_slice() == h.as_ref()
            }
        }
    }
}

impl Eq for HashOrRlp {}

impl std::hash::Hash for HashOrRlp {
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

impl Serializable for HashOrRlp {
    fn write_to<W: ExtendableBytes>(&self, vec: &mut W) {
        match self {
            HashOrRlp::Hash(h) => {
                vec.push(0);
                vec.extend_from_slice(h.as_ref());
            }
            HashOrRlp::Rlp(r) => {
                debug_assert!(!r.is_empty());
                vec.push(r.len_u8());
                vec.extend_from_slice(r.as_ref());
            }
        }
    }

    fn from_reader<R: Read>(mut reader: R) -> Result<Self, std::io::Error> {
        let mut first = [0u8; 1];
        reader.read_exact(&mut first)?;
        let Some(len) = std::num::NonZeroU8::new(first[0]) else {
            // length is zero, so it's a full trie hash
            let mut hash = [0u8; 32];
            reader.read_exact(&mut hash)?;
            return Ok(HashOrRlp::Hash(TrieHash::from(hash)));
        };

        let len = len.get() as usize;
        if len > RlpBytes::MAX_LEN {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid RLP length; expected 1..=31, got {len}"),
            ));
        }

        let mut bytes = [0u8; RLP_MAX_LEN];
        let out = bytes
            .get_mut(..len)
            .expect("length is checked before allocation");
        reader.read_exact(out)?;
        Ok(HashOrRlp::Rlp(RlpBytes::from_buf_and_len(bytes, len as u8)))
    }
}

impl From<HashOrRlp> for TrieHash {
    fn from(val: HashOrRlp) -> Self {
        match val {
            HashOrRlp::Hash(h) => h,
            HashOrRlp::Rlp(r) => Keccak256::digest(&r).into(),
        }
    }
}

impl From<&HashOrRlp> for TrieHash {
    fn from(val: &HashOrRlp) -> Self {
        match val {
            HashOrRlp::Hash(h) => h.clone(),
            HashOrRlp::Rlp(r) => Keccak256::digest(r).into(),
        }
    }
}

impl From<TrieHash> for HashOrRlp {
    fn from(val: TrieHash) -> Self {
        HashOrRlp::Hash(val)
    }
}

impl From<[u8; 32]> for HashOrRlp {
    fn from(value: [u8; 32]) -> Self {
        HashOrRlp::Hash(TrieHash::into(value.into()))
    }
}

impl TryFrom<&[u8]> for HashOrRlp {
    type Error = crate::InvalidTrieHashLength;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        value.try_into().map(HashOrRlp::Hash)
    }
}

impl AsRef<[u8]> for HashOrRlp {
    fn as_ref(&self) -> &[u8] {
        match self {
            HashOrRlp::Hash(h) => h.as_ref(),
            HashOrRlp::Rlp(r) => r.as_ref(),
        }
    }
}

impl std::ops::Deref for HashOrRlp {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl Display for HashOrRlp {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            HashOrRlp::Hash(h) => write!(f, "{h}"),
            HashOrRlp::Rlp(r) => {
                let width = f.precision().unwrap_or(32);
                write!(f, "{:.*}", width, hex::encode(r))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{HashOrRlp, RlpBytes};
    use crate::node::branch::Serializable;

    #[test]
    fn rlp_bytes_enforces_inline_bounds() {
        assert!(RlpBytes::try_from_slice(&[]).is_none());
        assert!(RlpBytes::try_from_slice(&[0u8; RlpBytes::MAX_LEN + 1]).is_none());

        let bytes = [0xabu8; RlpBytes::MAX_LEN];
        let rlp = RlpBytes::try_from_slice(&bytes).expect("31-byte RLP is valid");

        assert_eq!(rlp.len_u8(), RlpBytes::MAX_LEN as u8);
        assert_eq!(rlp.as_slice(), bytes);
    }

    #[test]
    fn hash_or_rlp_rejects_serialized_rlp_at_hash_length() {
        let serialized = [32u8; 33];
        let err = HashOrRlp::from_reader(serialized.as_slice())
            .expect_err("32-byte inline RLP must be rejected");

        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }
}
