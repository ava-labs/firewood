// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Storage layer metric definitions.

firewood_metrics::define_metrics! {
    counters: {
        SPACE_REUSED               = "space.reused"              : "Amount of space reused from free lists (bytes)",
        SPACE_FROM_END             = "space.from_end"            : "Amount of space allocated from end (bytes)",
        SPACE_FREED                = "space.freed"               : "Amount of space freed (bytes)",
        DELETE_NODE                = "delete_node"               : "Count of deleted nodes",
        FLUSH_NODES                = "flush_nodes"               : "Time spent flushing nodes (ms)",
        REAP_NODES                 = "reap_nodes"                : "Time spent reaping nodes (ms)",
        READ_NODE                  = "read_node"                 : "Number of node reads",
        CACHE_NODE                 = "cache.node"                : "Number of node cache operations",
        CACHE_FREELIST             = "cache.freelist"            : "Number of freelist cache operations",
        IO_READ_MS                 = "io.read_ms"                : "IO read timing (ms)",
        IO_READ_COUNT              = "io.read"                   : "Number of IO read operations",
        REPARENTED_PROPOSAL_COUNT  = "proposals.reparented"      : "Number of proposals reparented to committed parent",
        ROOTSTORE_GET              = "rootstore_get"             : "Fetch attempts from the rootstore",
        RING_EAGAIN_WRITE_RETRY    = "ring.eagain_write_retry"   : "Count of EAGAIN write retries",
        RING_FULL                  = "ring.full"                 : "Count of ring buffer full events",
        RING_SQ_WAIT               = "ring.sq_wait"              : "Count of submission queue waits",
        RING_PARTIAL_WRITE_RETRY   = "ring.partial_write_retry"  : "Count of partial write retries"
    },
    gauges: {
        FREELIST_CACHE_SIZE  = "cache.freelist.size"      : "Current number of entries in freelist cache",
        CACHE_MEMORY_USED    = "node.cache.memory_used"   : "Current memory used by node cache (bytes)",
        CACHE_MEMORY_LIMIT   = "node.cache.memory_limit"  : "Maximum memory capacity of node cache (bytes)"
    }
}
