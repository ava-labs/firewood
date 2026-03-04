// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Replay layer metric definitions.

/// Time spent in propose (seconds, accumulating).
pub const PROPOSE_SECONDS_TOTAL: &str = "replay.propose_seconds_total";

/// Number of propose calls.
pub const PROPOSE_TOTAL: &str = "replay.propose_total";

/// Time spent in commit (seconds, accumulating).
pub const COMMIT_SECONDS_TOTAL: &str = "replay.commit_seconds_total";

/// Number of commit calls.
pub const COMMIT_TOTAL: &str = "replay.commit_total";

/// Registers all replay metric descriptions.
pub fn register() {
    use metrics::describe_counter;

    describe_counter!(PROPOSE_SECONDS_TOTAL, "Time spent in propose (seconds)");
    describe_counter!(PROPOSE_TOTAL, "Number of propose calls");
    describe_counter!(COMMIT_SECONDS_TOTAL, "Time spent in commit (seconds)");
    describe_counter!(COMMIT_TOTAL, "Number of commit calls");
}
