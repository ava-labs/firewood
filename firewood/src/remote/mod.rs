// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Remote storage support for Firewood.
//!
//! This module provides a client-server mode where the server holds the full
//! Firewood `Db` and the client holds only a truncated in-memory trie of the
//! top K levels, with children below K replaced by hash-only proxy nodes.
//!
//! Every read is verified against an inclusion/exclusion proof. Every commit
//! is verified via witness-based re-execution.

pub mod client;
pub mod truncated_trie;
pub mod witness;

pub use truncated_trie::TruncatedTrie;
