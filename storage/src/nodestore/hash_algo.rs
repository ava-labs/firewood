// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

/// An error indicating that an integer could not be converted into a [`NodeHashAlgorithm`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("invalid integer for HashMode: {0}; expected 0 for MerkleDB or 1 for EthereumStateTrie")]
pub struct NodeHashAlgorithmTryFromIntError(u64);

/// The hash algorithm used by the node store.
///
/// This is a per-database runtime choice persisted in the file header. A
/// database created under one scheme is opened under that same scheme; the
/// requested algorithm is validated against the header at open time
/// (`validate_open`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeHashAlgorithm {
    /// Use the MerkleDB node hashing algorithm, with sha256 as the hash function.
    MerkleDB,
    /// Use the Ethereum state trie node hashing algorithm, with keccak256 as the
    /// hash function.
    Ethereum,
}

impl NodeHashAlgorithm {
    /// Returns whether this is the Ethereum hash algorithm (Keccak-256,
    /// account-aware nodes).
    #[must_use]
    pub const fn is_ethereum(self) -> bool {
        matches!(self, NodeHashAlgorithm::Ethereum)
    }

    pub(crate) const fn name(self) -> &'static str {
        match self {
            NodeHashAlgorithm::MerkleDB => "MerkleDB",
            NodeHashAlgorithm::Ethereum => "Ethereum",
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
