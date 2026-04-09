// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Firewood layer metric definitions.

firewood_metrics::define_metrics! {
    counters: {
        /// Number of proposals created
        PROPOSALS              = "proposals",
        /// Number of proposals created by base type
        PROPOSALS_CREATED      = "proposals.created",
        /// Number of proposals dropped without commit
        PROPOSALS_DISCARDED    = "proposals.discarded",
        /// Number of insert operations
        INSERT                 = "insert",
        /// Number of remove operations
        REMOVE                 = "remove",
        /// Number of next calls to calculate a change proof
        CHANGE_PROOF_NEXT      = "change_proof.next",
        /// Commit latency (ms)
        COMMIT_LATENCY_MS      = "commit_latency_ms",
        /// Number of times commit was blocked
        COMMIT_BLOCKED         = "persist.commit_blocked",
    },
    gauges: {
        /// Current number of uncommitted proposals
        PROPOSALS_UNCOMMITTED  = "proposals.uncommitted",
        /// Current number of active revisions
        ACTIVE_REVISIONS       = "active_revisions",
        /// Maximum number of revisions configured
        MAX_REVISIONS          = "max_revisions",
        /// Length of deleted list for committed revisions
        DELETED_LIST_LEN       = "deleted_list_len",
        /// Number of persist permits currently available
        PERMITS_AVAILABLE      = "persist.permits_available",
        /// Maximum number of persist permits
        MAX_PERMITS            = "persist.max_permits",
    },
}
