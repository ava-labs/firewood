// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! FFI layer metric definitions.

use firewood_metrics::{HistogramBucketConfig, register_histogram_with_buckets};
use metrics::describe_counter;

pub const COMMIT_SECONDS_TOTAL: &str = "ffi.commit_seconds_total";
pub const COMMIT_TOTAL: &str = "ffi.commit_total";
pub const COMMIT_DURATION_SECONDS: &str = "ffi.commit_duration_seconds";

pub const PROPOSE_SECONDS_TOTAL: &str = "ffi.propose_seconds_total";
pub const PROPOSE_TOTAL: &str = "ffi.propose_total";
pub const PROPOSE_DURATION_SECONDS: &str = "ffi.propose_duration_seconds";

pub const BATCH_SECONDS_TOTAL: &str = "ffi.batch_seconds_total";
pub const BATCH_TOTAL: &str = "ffi.batch_total";
pub const BATCH_DURATION_SECONDS: &str = "ffi.batch_duration_seconds";

pub const CACHED_VIEW_MISS_TOTAL: &str = "ffi.cached_view.miss_total";
pub const CACHED_VIEW_HIT_TOTAL: &str = "ffi.cached_view.hit_total";

pub const MERGE_TOTAL: &str = "firewood.ffi.merge_total";

const FFI_TIMING_BUCKETS: &[f64] = &[
    0.0001, 0.00025, 0.0005, 0.00075, 0.001, 0.0015, 0.002, 0.0025, 0.003, 0.004, 0.005, 0.006,
    0.007, 0.008, 0.010, 0.012, 0.015, 0.020, 0.030, 0.050,
];

/// Registers all FFI metric descriptions.
///
/// Histogram bucket configurations are collected into the provided vector.
pub fn register(histogram_configs: &mut Vec<HistogramBucketConfig>) {
    describe_counter!(COMMIT_SECONDS_TOTAL, "Time spent committing via FFI (seconds)");
    describe_counter!(COMMIT_TOTAL, "Count of commit operations via FFI");
    register_histogram_with_buckets(
        histogram_configs,
        COMMIT_DURATION_SECONDS,
        "Commit duration via FFI in seconds",
        FFI_TIMING_BUCKETS,
    );

    describe_counter!(PROPOSE_SECONDS_TOTAL, "Time spent proposing via FFI (seconds)");
    describe_counter!(PROPOSE_TOTAL, "Count of proposal operations via FFI");
    register_histogram_with_buckets(
        histogram_configs,
        PROPOSE_DURATION_SECONDS,
        "Propose duration via FFI in seconds",
        FFI_TIMING_BUCKETS,
    );

    describe_counter!(BATCH_SECONDS_TOTAL, "Time spent processing batches (seconds)");
    describe_counter!(BATCH_TOTAL, "Count of batch operations completed");
    register_histogram_with_buckets(
        histogram_configs,
        BATCH_DURATION_SECONDS,
        "Batch processing duration in seconds",
        FFI_TIMING_BUCKETS,
    );

    describe_counter!(CACHED_VIEW_MISS_TOTAL, "Count of cached view misses");
    describe_counter!(CACHED_VIEW_HIT_TOTAL, "Count of cached view hits");
    describe_counter!(MERGE_TOTAL, "Count of range proof merges via FFI");
}
