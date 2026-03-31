// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! FFI layer metric definitions.

firewood_metrics::define_metrics! {
    counters: {
        CACHED_VIEW_MISS = "ffi.cached_view.miss" : "Count of cached view misses",
        CACHED_VIEW_HIT  = "ffi.cached_view.hit"  : "Count of cached view hits",
        MERGE_COUNT      = "firewood.ffi.merge"    : "Count of range proof merges via FFI"
    },
    gauges: {}
}
