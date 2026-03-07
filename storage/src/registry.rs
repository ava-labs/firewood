// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Storage layer metric definitions.

use metrics::{describe_counter, describe_gauge};

/// Amount of space reused from free lists (bytes).
pub const SPACE_REUSED_BYTES_TOTAL: &str = "space.reused_bytes_total";
/// Amount of space allocated from end (bytes).
pub const SPACE_FROM_END_BYTES_TOTAL: &str = "space.from_end_bytes_total";
/// Amount of space freed (bytes).
pub const SPACE_FREED_BYTES_TOTAL: &str = "space.freed_bytes_total";
/// Count of deleted nodes.
pub const DELETE_NODE_TOTAL: &str = "delete_node_total";
/// Time spent flushing nodes (seconds, accumulating).
pub const FLUSH_NODES_SECONDS_TOTAL: &str = "flush_nodes_seconds_total";

/// Number of node reads.
pub const READ_NODE_TOTAL: &str = "read_node_total";
/// Number of node cache operations.
pub const CACHE_NODE_TOTAL: &str = "cache.node_total";
/// Number of freelist cache operations.
pub const CACHE_FREELIST_TOTAL: &str = "cache.freelist_total";
/// Current number of entries in the freelist cache.
pub const FREELIST_CACHE_SIZE: &str = "cache.freelist.size";
/// Current memory used by the node cache in bytes.
pub const CACHE_MEMORY_USED: &str = "node.cache.memory_used";
/// Maximum memory capacity of the node cache in bytes.
pub const CACHE_MEMORY_LIMIT: &str = "node.cache.memory_limit";

/// IO read timing in seconds (accumulating).
pub const IO_READ_SECONDS_TOTAL: &str = "io.read_seconds_total";
/// Number of IO read operations.
pub const IO_READ_TOTAL: &str = "io.read_total";

/// Number of proposals reparented to committed parent.
pub const REPARENTED_PROPOSAL_TOTAL: &str = "proposals.reparented_total";

/// `io_uring` specific metrics.
pub mod ring {
    /// Count of EAGAIN write retries.
    pub const EAGAIN_WRITE_RETRY_TOTAL: &str = "ring.eagain_write_retry_total";
    /// Count of ring buffer full events.
    pub const FULL_TOTAL: &str = "ring.full_total";
    /// Count of submission queue waits.
    pub const SQ_WAIT_TOTAL: &str = "ring.sq_wait_total";
    /// Count of partial write retries.
    pub const PARTIAL_WRITE_RETRY_TOTAL: &str = "ring.partial_write_retry_total";
}

/// Registers all storage metric descriptions.
pub fn register() {
    describe_counter!(
        SPACE_REUSED_BYTES_TOTAL,
        "Amount of space reused from free lists (bytes)"
    );
    describe_counter!(
        SPACE_FROM_END_BYTES_TOTAL,
        "Amount of space allocated from end (bytes)"
    );
    describe_counter!(SPACE_FREED_BYTES_TOTAL, "Amount of space freed (bytes)");
    describe_counter!(DELETE_NODE_TOTAL, "Count of deleted nodes");
    describe_counter!(
        FLUSH_NODES_SECONDS_TOTAL,
        "Time spent flushing nodes (seconds)"
    );

    describe_counter!(READ_NODE_TOTAL, "Number of node reads");
    describe_counter!(CACHE_NODE_TOTAL, "Number of node cache operations");
    describe_counter!(CACHE_FREELIST_TOTAL, "Number of freelist cache operations");
    describe_gauge!(
        FREELIST_CACHE_SIZE,
        "Current number of entries in freelist cache"
    );
    describe_gauge!(
        CACHE_MEMORY_USED,
        "Current memory used by node cache (bytes)"
    );
    describe_gauge!(
        CACHE_MEMORY_LIMIT,
        "Maximum memory capacity of node cache (bytes)"
    );

    describe_counter!(IO_READ_SECONDS_TOTAL, "IO read timing (seconds)");
    describe_counter!(IO_READ_TOTAL, "Number of IO read operations");

    describe_counter!(
        REPARENTED_PROPOSAL_TOTAL,
        "Number of proposals reparented to committed parent"
    );

    // Ring metrics
    describe_counter!(
        ring::EAGAIN_WRITE_RETRY_TOTAL,
        "Count of EAGAIN write retries"
    );
    describe_counter!(ring::FULL_TOTAL, "Count of ring buffer full events");
    describe_counter!(ring::SQ_WAIT_TOTAL, "Count of submission queue waits");
    describe_counter!(
        ring::PARTIAL_WRITE_RETRY_TOTAL,
        "Count of partial write retries"
    );
}
