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
//! # Range Proof Verification Algorithm
//!
//! Range proof verification confirms that a contiguous set of key-value pairs
//! exists within a specific key range of a trie with a given root hash.
//!
//! [`verify_range_proof`] proceeds in two phases:
//!
//! ## Phase 1 — Structural and boundary validation
//!
//! Validates that key-value pairs are strictly ascending and within the
//! requested `[first_key, last_key]` range. Proof nodes carrying in-range
//! values must appear in the key-value list (prevents an attacker from
//! hiding keys that sit on a proof path). Start and end boundary proofs
//! are verified against `root_hash` via hash chain checks.
//!
//! ## Phase 2 — Root hash verification
//!
//! A fresh in-memory proving trie is built from the key-value pairs.
//! Boundary proof nodes are reconciled into it via
//! `reconcile_branch_proof_node`, which inserts branch structure matching
//! the original trie's layout. `compute_root_hash_with_proofs` then
//! computes a hybrid root hash: in-range children are hashed from the
//! proving trie, while out-of-range children (identified by
//! `compute_outside_children`) use hashes from the proof nodes. The
//! result must match `root_hash`.
//!
//! # Change Proof Verification Algorithm
//!
//! Change proof verification confirms that applying a set of `BatchOp`s to a trie
//! with `start_root` produces a trie consistent with `end_root`, without requiring
//! the verifier to hold the full `end_root` trie.
//!
//! Verification proceeds in three phases:
//!
//! ## Phase 1 — Structural validation ([`verify_change_proof_structure`])
//!
//! Validates the proof's internal consistency: key ordering, range bounds,
//! absence of `DeleteRange` ops, and boundary proof hash chain verification
//! against `end_root`. The start proof anchors the left edge of the proven
//! range; the end proof anchors the right edge. Both are standard Merkle
//! inclusion/exclusion proofs.
//!
//! ## Phase 2 — Apply batch ops
//!
//! The verifier applies the proof's `batch_ops` to its own copy of
//! `start_root`, producing a proposal. This is the same commit path used
//! for normal trie operations.
//!
//! ## Phase 3 — Root hash verification ([`verify_change_proof_root_hash`])
//!
//! The proposal's nodestore is forked into a "proving trie" that already
//! contains the correct in-range key-value data. Two reshaping passes make
//! its structure match `end_root`:
//!
//! 1. **Branch reconciliation** — `reconcile_branch_proof_node` ensures
//!    branch nodes exist at every proof-path position with values matching
//!    `end_root`. Out-of-range ancestors (proper prefixes of the boundary
//!    keys) may carry values from `start_root` that differ from `end_root`;
//!    the proof node's value is adopted for these positions.
//!
//! 2. **Branch collapsing** — `collapse_branch_to_path` removes out-of-range
//!    children and flattens single-child branches between consecutive proof
//!    nodes. The proof implies a direct path with no intermediate branches;
//!    the fork may have extra structure from out-of-range keys that must be
//!    stripped so the trie shape matches `end_root`.
//!
//! After reshaping, `compute_root_hash_with_proofs` walks the proving trie,
//! hashing in-range children directly and substituting proof hashes for
//! out-of-range children. The result is compared against `end_root`.
//!
//! # Range vs. Change Proof Verification
//!
//! Both algorithms build a proving trie, reconcile boundary proof nodes into
//! it, and compute a hybrid root hash. They differ in three ways:
//!
//! 1. **Proving trie origin** — Range proofs build a fresh in-memory trie
//!    from the key-value pairs provided in the proof. Change proofs fork
//!    the proposal's nodestore, which already contains the full trie
//!    (in-range *and* out-of-range keys).
//!
//! 2. **Branch collapsing** — Because the change proof's fork contains
//!    out-of-range keys, it may have branch structure that doesn't exist
//!    in `end_root`. `collapse_branch_to_path` strips this extra structure
//!    between consecutive proof nodes. Range proofs skip this step — the
//!    proving trie was built from only in-range keys, so no extra branches
//!    exist.
//!
//! 3. **Out-of-range value trust** — When a proof node's value conflicts
//!    with the proving trie, range proofs unconditionally accept the
//!    proof's value (the proving trie has no out-of-range data to
//!    preserve). Change proofs must be selective: out-of-range ancestors
//!    adopt the proof's value (which reflects `end_root`), but a conflict
//!    at a boundary key itself is an error, since that key is in-range and
//!    should already match.

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
    ChangeProof, ChangeProofVerificationContext, verify_change_proof_structure,
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
