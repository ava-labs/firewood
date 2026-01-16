// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Storage layer metric definitions.

use metrics::describe_counter;

/// Amount of space reused from free lists.
pub const SPACE_REUSED: &str = "space.reused";
/// Amount of space allocated from end.
pub const SPACE_FROM_END: &str = "space.from_end";
/// Amount of space freed.
pub const SPACE_FREED: &str = "space.freed";
/// Count of deleted nodes.
pub const DELETE_NODE: &str = "delete_node";
/// Time spent flushing nodes.
pub const FLUSH_NODES: &str = "flush_nodes";

/// Number of node reads.
pub const READ_NODE: &str = "read_node";
/// Number of node cache operations.
pub const CACHE_NODE: &str = "cache.node";
/// Number of freelist cache operations.
pub const CACHE_FREELIST: &str = "cache.freelist";

/// IO read timing in milliseconds.
pub const IO_READ_MS: &str = "io.read_ms";
/// Number of IO read operations.
pub const IO_READ_COUNT: &str = "io.read";

/// Number of proposals reparented to committed parent.
pub const REPARENTED_PROPOSAL_COUNT: &str = "proposals.reparented";

/// `io_uring` specific metrics.
pub mod ring {
    /// Count of EAGAIN write retries.
    pub const EAGAIN_WRITE_RETRY: &str = "ring.eagain_write_retry";
    /// Count of ring buffer full events.
    pub const FULL: &str = "ring.full";
    /// Count of submission queue waits.
    pub const SQ_WAIT: &str = "ring.sq_wait";
    /// Count of partial write retries.
    pub const PARTIAL_WRITE_RETRY: &str = "ring.partial_write_retry";
}

/// Registers all storage metric descriptions.
pub fn register() {
    describe_counter!(SPACE_REUSED, "Amount of space reused from free lists");
    describe_counter!(SPACE_FROM_END, "Amount of space allocated from end");
    describe_counter!(SPACE_FREED, "Amount of space freed");
    describe_counter!(DELETE_NODE, "Count of deleted nodes");
    describe_counter!(FLUSH_NODES, "Time spent flushing nodes");

    describe_counter!(READ_NODE, "Number of node reads");
    describe_counter!(CACHE_NODE, "Number of node cache operations");
    describe_counter!(CACHE_FREELIST, "Number of freelist cache operations");

    describe_counter!(IO_READ_MS, "IO read timing in milliseconds");
    describe_counter!(IO_READ_COUNT, "Number of IO read operations");

    describe_counter!(
        REPARENTED_PROPOSAL_COUNT,
        "Number of proposals reparented to committed parent"
    );

    // Ring metrics
    describe_counter!(ring::EAGAIN_WRITE_RETRY, "Count of EAGAIN write retries");
    describe_counter!(ring::FULL, "Count of ring buffer full events");
    describe_counter!(ring::SQ_WAIT, "Count of submission queue waits");
    describe_counter!(ring::PARTIAL_WRITE_RETRY, "Count of partial write retries");
}
