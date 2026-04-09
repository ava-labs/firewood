// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Storage layer metric definitions.

firewood_metrics::define_metrics! {
    counters: {
        /// Amount of space reused from free lists (bytes)
        SPACE_REUSED               = "space.reused",
        /// Amount of space allocated from end (bytes)
        SPACE_FROM_END             = "space.from_end",
        /// Amount of space freed (bytes)
        SPACE_FREED                = "space.freed",
        /// Count of deleted nodes
        DELETE_NODE                = "delete_node",
        /// Time spent flushing nodes (ms)
        FLUSH_NODES                = "flush_nodes",
        /// Time spent reaping nodes (ms)
        REAP_NODES                 = "reap_nodes",
        /// Number of node reads
        READ_NODE                  = "read_node",
        /// Number of node cache operations
        CACHE_NODE                 = "cache.node",
        /// Number of freelist cache operations
        CACHE_FREELIST             = "cache.freelist",
        /// IO read timing (ms)
        IO_READ_MS                 = "io.read_ms",
        /// Number of IO read operations
        IO_READ_COUNT              = "io.read",
        /// Number of proposals reparented to committed parent
        REPARENTED_PROPOSAL_COUNT  = "proposals.reparented",
        /// Fetch attempts from the rootstore
        ROOTSTORE_GET              = "rootstore_get",
        /// Count of EAGAIN write retries
        RING_EAGAIN_WRITE_RETRY    = "ring.eagain_write_retry",
        /// Count of ring buffer full events
        RING_FULL                  = "ring.full",
        /// Count of submission queue waits
        RING_SQ_WAIT               = "ring.sq_wait",
        /// Count of partial write retries
        RING_PARTIAL_WRITE_RETRY   = "ring.partial_write_retry"
    },
    gauges: {
        /// Current number of entries in freelist cache
        FREELIST_CACHE_SIZE  = "cache.freelist.size",
        /// Current memory used by node cache (bytes)
        CACHE_MEMORY_USED    = "node.cache.memory_used",
        /// Maximum memory capacity of node cache (bytes)
        CACHE_MEMORY_LIMIT   = "node.cache.memory_limit"
    },
}
