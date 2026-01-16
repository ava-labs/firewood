// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! FFI layer metric definitions.

use metrics::describe_counter;

pub const COMMIT_MS: &str = "ffi.commit_ms";
pub const COMMIT_COUNT: &str = "ffi.commit";

pub const PROPOSE_MS: &str = "ffi.propose_ms";
pub const PROPOSE_COUNT: &str = "ffi.propose";

pub const BATCH_MS: &str = "ffi.batch_ms";
pub const BATCH_COUNT: &str = "ffi.batch";

pub const CACHED_VIEW_MISS: &str = "ffi.cached_view.miss";
pub const CACHED_VIEW_HIT: &str = "ffi.cached_view.hit";

pub const MERGE_COUNT: &str = "ffi.merge";

/// Registers all FFI metric descriptions.
pub fn register() {
    describe_counter!(COMMIT_MS, "Time spent committing via FFI in milliseconds");
    describe_counter!(COMMIT_COUNT, "Count of commit operations via FFI");

    describe_counter!(PROPOSE_MS, "Time spent proposing via FFI in milliseconds");
    describe_counter!(PROPOSE_COUNT, "Count of proposal operations via FFI");

    describe_counter!(BATCH_MS, "Time spent processing batches in milliseconds");
    describe_counter!(BATCH_COUNT, "Count of batch operations completed");

    describe_counter!(CACHED_VIEW_MISS, "Count of cached view misses");
    describe_counter!(CACHED_VIEW_HIT, "Count of cached view hits");
    describe_counter!(MERGE_COUNT, "Count of range proof merges via FFI");
}
