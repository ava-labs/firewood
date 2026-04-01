// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Cryptographic proof system for Merkle tries.
//!
//! This module provides a complete proof system for verifying the presence or absence
//! of key-value pairs in Firewood's Merkle trie implementation without requiring access
//! to the entire trie structure. The proof system enables efficient verification of trie
//! state using cryptographic hashes.
//!
//! # Overview
//!
//! Firewood's proof system consists of several key components:
//!
//! - **Single-key proofs** ([`Proof`]): Verify that a specific key-value pair exists
//!   (or doesn't exist) in a trie with a given root hash.
//! - **Range proofs** ([`RangeProof`]): Verify that a contiguous set of key-value pairs
//!   exists within a specific key range.
//! - **Change proofs** ([`ChangeProof`]): Verify that a set of key-value changes
//!   between two trie revisions is consistent with a target root hash.
//! - **Serialization format**: A compact binary format for transmitting proofs over the
//!   network or storing them persistently.
//!
//! # Architecture
//!
//! The proof system is organized into several submodules:
//!
//! - `types`: Core proof types including [`Proof`], [`ProofNode`], [`ProofError`], and
//!   [`ProofCollection`].
//! - `range`: Range proof implementation for verifying multiple consecutive keys.
//! - `change`: Change proof type and structural verification.
//! - `header`: Proof format headers and validation.
//! - `reader`: Proof reading and deserialization utilities.
//! - `ser`: Proof serialization implementation (internal).
//! - `de`: Proof deserialization implementation (internal).
//! - `childmask` (in `merkle`): Compact bitmap for tracking present children.
//! - `magic`: Magic constants for proof format identification (internal).
//!
//! # Usage
//!
//! For most use cases, import proof types directly from the top level of the crate:
//!
//! ```rust,ignore
//! use firewood::{Proof, ProofNode, RangeProof};
//!
//! // Verify a single key
//! let proof: Proof<Vec<ProofNode>> = /* ... */;
//! proof.verify(b"key", Some(b"value"), &root_hash)?;
//!
//! // Verify a key range
//! let range_proof: RangeProof<Vec<u8>, Vec<u8>, Vec<ProofNode>> = /* ... */;
//! for (key, value) in &range_proof {
//!     // Process key-value pairs
//! }
//!
//! // Verify a change proof
//! // Step 1: Structural validation and boundary proof hash chain verification.
//! let ctx = verify_change_proof_structure(&proof, end_root, start_key, end_key, max_length)?;
//!
//! // Step 2: Apply batch_ops to the verifier's start_root to produce a proposal.
//! let proposal = db.apply_change_proof_to_parent(&proof, &parent_revision)?;
//!
//! // Step 3: Compare in-range children from the boundary proofs against the
//! //         proposal's trie. Paths are derived from the last proof node in
//! //         each boundary proof via change_proof_boundary_key.
//! verify_change_proof_root_hash(&proof, &ctx, proposal_root.as_ref(), &start_path, &end_path)?;
//! ```
//!
//! # Proof Format
//!
//! Proofs are serialized in a compact binary format that includes:
//!
//! 1. A 32-byte header identifying the proof type, version, hash mode, and branching factor
//! 2. A sequence of proof nodes, each containing:
//!    - The node's key path (variable length)
//!    - The node's value or value hash (if present)
//!    - A bitmap indicating which children are present
//!    - The hash of each present child
//!
//! Change proofs additionally serialize their `batch_ops` as a sequence of tagged
//! Put/Delete operations after the boundary proof nodes. See `ser.rs` for details.
//!
//! The serialization format is versioned to allow for future evolution while maintaining
//! backward compatibility with proof verification.
//!
//! # Formal Verification (Change Proofs)
//!
//! The change proof verification design has been formally verified using TLA+ and
//! the TLC model checker. 35 design properties and 3 protocol properties were
//! exhaustively checked for all trie instances up to branch factor 4 with depth 2,
//! covering boundary classification, hash chain soundness, structural validation,
//! operation consistency, truncated proofs, exclusion proofs, attack detection, and
//! multi-round sync safety under adversarial provers. Two previously fixed bugs
//! were formally reproduced as regression tests — TLC confirms both the vulnerable
//! behavior and the necessity of their fixes. No new bugs were found.

pub(crate) mod change;
pub(super) mod de;
pub(crate) mod header;
pub(crate) mod range;
pub(crate) mod reader;
pub(super) mod ser;
#[cfg(test)]
mod tests;
pub(crate) mod types;

pub use self::change::{
    ChangeProof, ChangeProofVerificationContext, change_proof_boundary_key,
    change_proof_node_byte_key, verify_change_proof_structure,
};
pub use self::header::InvalidHeader;
pub use self::range::RangeProof;
pub use self::reader::ReadError;
pub use self::types::{
    EmptyProofCollection, Proof, ProofCollection, ProofError, ProofNode, ProofType,
};

pub(super) mod magic {
    //! Magic constants for proof format identification.
    //!
    //! These constants are used in proof headers to identify the proof format,
    //! version, hash mode, and branching factor. They enable proof readers to
    //! quickly validate that a proof is compatible with the current implementation.

    /// Magic header bytes identifying a Firewood proof: `b"fwdproof"`
    pub const PROOF_HEADER: &[u8; 8] = b"fwdproof";

    /// Current proof format version: `0`
    pub const PROOF_VERSION: u8 = 0;

    /// Hash mode identifier for SHA-256 hashing
    #[cfg(not(feature = "ethhash"))]
    pub const HASH_MODE: u8 = 0;

    /// Hash mode identifier for Keccak-256 hashing (Ethereum-compatible)
    #[cfg(feature = "ethhash")]
    pub const HASH_MODE: u8 = 1;

    /// Returns the human-readable name for a hash mode identifier.
    pub const fn hash_mode_name(v: u8) -> &'static str {
        match v {
            0 => "sha256",
            1 => "keccak256",
            _ => "unknown",
        }
    }

    /// Branching factor identifier for branch factor 16
    pub const BRANCH_FACTOR: u8 = 16;

    /// `BatchOp` constants for serialization
    pub const BATCH_PUT: u8 = 0;
    pub const BATCH_DELETE: u8 = 1;
    pub const BATCH_DELETE_RANGE: u8 = 2;
}
