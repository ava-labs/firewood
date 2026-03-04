// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Firewood layer metric definitions.

use metrics::{describe_counter, describe_gauge};

/// Number of proposals created.
pub const PROPOSALS_TOTAL: &str = "proposals_total";

/// Number of proposals created by base type.
pub const PROPOSALS_CREATED_TOTAL: &str = "proposals.created_total";

/// Number of proposals discarded (dropped without commit).
pub const PROPOSALS_DISCARDED_TOTAL: &str = "proposals.discarded_total";

/// Current number of uncommitted proposals.
pub const PROPOSALS_UNCOMMITTED: &str = "proposals.uncommitted";

/// Number of insert operations.
pub const INSERT_TOTAL: &str = "insert_total";

/// Number of remove operations.
pub const REMOVE_TOTAL: &str = "remove_total";

/// Number of next calls to calculate a change proof.
pub const CHANGE_PROOF_NEXT_TOTAL: &str = "change_proof.next_total";

/// Commit latency in seconds (accumulating counter).
pub const COMMIT_LATENCY_SECONDS_TOTAL: &str = "commit_latency_seconds_total";

/// Current number of active revisions.
pub const ACTIVE_REVISIONS: &str = "active_revisions";

/// Maximum number of revisions configured.
pub const MAX_REVISIONS: &str = "max_revisions";

/// Registers all firewood metric descriptions.
pub fn register() {
    describe_counter!(PROPOSALS_TOTAL, "Number of proposals created");
    describe_counter!(
        PROPOSALS_CREATED_TOTAL,
        "Number of proposals created by base type"
    );
    describe_counter!(
        PROPOSALS_DISCARDED_TOTAL,
        "Number of proposals dropped without commit"
    );
    describe_gauge!(
        PROPOSALS_UNCOMMITTED,
        "Current number of uncommitted proposals"
    );
    describe_counter!(INSERT_TOTAL, "Number of insert operations");
    describe_counter!(REMOVE_TOTAL, "Number of remove operations");
    describe_counter!(
        CHANGE_PROOF_NEXT_TOTAL,
        "Number of next calls to calculate a change proof"
    );
    describe_counter!(COMMIT_LATENCY_SECONDS_TOTAL, "Commit latency (seconds)");
    describe_gauge!(ACTIVE_REVISIONS, "Current number of active revisions");
    describe_gauge!(MAX_REVISIONS, "Maximum number of revisions configured");
}
