// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

/// An error indicating that an integer could not be converted into a [`NodeHashAlgorithm`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("invalid integer for HashMode: {0}; expected 0 for MerkleDB or 1 for EthereumStateTrie")]
pub struct NodeHashAlgorithmTryFromIntError(u64);

/// The hash algorithm used by the node store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeHashAlgorithm {
    /// Use the MerkleDB node hashing algorithm, with sha256 as the hash function.
    MerkleDB,
    /// Use the Ethereum state trie node hashing algorithm, with keccak256 as the
    /// hash function.
    Ethereum,
}

impl NodeHashAlgorithm {
    /// Returns `true` if this algorithm uses Ethereum state trie hashing.
    #[must_use]
    pub const fn is_ethereum(self) -> bool {
        matches!(self, Self::Ethereum)
    }

    /// Returns the proof-header discriminator for this hash algorithm.
    #[must_use]
    pub const fn proof_hash_mode(self) -> u8 {
        match self {
            Self::MerkleDB => 0,
            Self::Ethereum => 1,
        }
    }

    pub(crate) const fn name(self) -> &'static str {
        match self {
            NodeHashAlgorithm::MerkleDB => "MerkleDB",
            NodeHashAlgorithm::Ethereum => "Ethereum",
        }
    }

    /// Returns the canonical empty-root hash for this algorithm, if it has one.
    #[must_use]
    pub fn default_root_hash(self) -> Option<crate::TrieHash> {
        match self {
            Self::MerkleDB => None,
            Self::Ethereum => Some(
                [
                    0x56, 0xe8, 0x1f, 0x17, 0x1b, 0xcc, 0x55, 0xa6, 0xff, 0x83, 0x45, 0xe6, 0x92,
                    0xc0, 0xf8, 0x6e, 0x5b, 0x48, 0xe0, 0x1b, 0x99, 0x6c, 0xad, 0xc0, 0x01, 0x62,
                    0x2f, 0xb5, 0xe3, 0x63, 0xb4, 0x21,
                ]
                .into(),
            ),
        }
    }

    pub(crate) fn validate_open(self, expected: NodeHashAlgorithm) -> std::io::Result<()> {
        if self == expected {
            Ok(())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                format!(
                    "node store hash algorithm mismatch: want to open with {}, \
                     but file header indicates {}",
                    expected.name(),
                    self.name()
                ),
            ))
        }
    }
}

impl TryFrom<u8> for NodeHashAlgorithm {
    type Error = NodeHashAlgorithmTryFromIntError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Self::try_from(u64::from(value))
    }
}

impl TryFrom<u64> for NodeHashAlgorithm {
    type Error = NodeHashAlgorithmTryFromIntError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(NodeHashAlgorithm::MerkleDB),
            1 => Ok(NodeHashAlgorithm::Ethereum),
            other => Err(NodeHashAlgorithmTryFromIntError(other)),
        }
    }
}
