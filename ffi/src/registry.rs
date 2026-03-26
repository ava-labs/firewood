// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! FFI layer metric definitions.

use metrics::describe_counter;

pub const CACHED_VIEW_MISS: &str = "ffi.cached_view.miss";
pub const CACHED_VIEW_HIT: &str = "ffi.cached_view.hit";

pub const MERGE_COUNT: &str = "firewood.ffi.merge";

/// Registers all FFI metric descriptions.
pub fn register() {
    describe_counter!(CACHED_VIEW_MISS, "Count of cached view misses");
    describe_counter!(CACHED_VIEW_HIT, "Count of cached view hits");
    describe_counter!(MERGE_COUNT, "Count of range proof merges via FFI");
}
