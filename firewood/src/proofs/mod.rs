// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

mod bitmap;
mod de;
mod header;
mod path;
mod proof_type;
mod reader;
mod ser;
#[cfg(test)]
mod tests;
mod trie;

use crate::{
    proof::{ProofCollection, ProofNode},
    range_proof::RangeProof,
    v2::api::{HashKey, KeyType, ValueType},
};

pub use self::header::InvalidHeader;
pub(crate) use self::path::BytesIter;
pub use self::path::CollectedNibbles;
pub use self::reader::ReadError;
pub use self::trie::MissingKeys;

mod magic {
    pub const PROOF_HEADER: &[u8; 8] = b"fwdproof";

    pub const PROOF_VERSION: u8 = 0;

    #[cfg(not(feature = "ethhash"))]
    pub const HASH_MODE: u8 = 0;
    #[cfg(feature = "ethhash")]
    pub const HASH_MODE: u8 = 1;

    pub const fn hash_mode_name(v: u8) -> &'static str {
        match v {
            0 => "sha256",
            1 => "keccak256",
            _ => "unknown",
        }
    }

    #[cfg(not(feature = "branch_factor_256"))]
    pub const BRANCH_FACTOR: u8 = 16;
    #[cfg(feature = "branch_factor_256")]
    pub const BRANCH_FACTOR: u8 = 0; // 256 wrapped to 0

    pub const fn widen_branch_factor(v: u8) -> u16 {
        match v {
            0 => 256,
            _ => v as u16,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct VerifyRangeProofArguments<'a, K, V, H> {
    lower_bound: Option<&'a [u8]>,
    upper_bound: Option<&'a [u8]>,
    expected_root: &'a HashKey,
    range_proof: &'a RangeProof<K, V, H>,
}

impl<'a, K, V, H> VerifyRangeProofArguments<'a, K, V, H>
where
    K: KeyType,
    V: ValueType,
    H: ProofCollection<Node = ProofNode>,
{
    pub(crate) const fn new(
        lower_bound: Option<&'a [u8]>,
        upper_bound: Option<&'a [u8]>,
        expected_root: &'a HashKey,
        range_proof: &'a RangeProof<K, V, H>,
    ) -> Self {
        Self {
            lower_bound,
            upper_bound,
            expected_root,
            range_proof,
        }
    }
}
