// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Firewood layer metric definitions.

firewood_metrics::define_metrics! {
    counters: {
        PROPOSALS              = "proposals"              : "Number of proposals created",
        PROPOSALS_CREATED      = "proposals.created"      : "Number of proposals created by base type",
        PROPOSALS_DISCARDED    = "proposals.discarded"    : "Number of proposals dropped without commit",
        INSERT                 = "insert"                 : "Number of insert operations",
        REMOVE                 = "remove"                 : "Number of remove operations",
        CHANGE_PROOF_NEXT      = "change_proof.next"      : "Number of next calls to calculate a change proof",
        COMMIT_LATENCY_MS      = "commit_latency_ms"      : "Commit latency (ms)",
        COMMIT_BLOCKED         = "persist.commit_blocked" : "Number of times commit was blocked"
    },
    gauges: {
        PROPOSALS_UNCOMMITTED  = "proposals.uncommitted"      : "Current number of uncommitted proposals",
        ACTIVE_REVISIONS       = "active_revisions"           : "Current number of active revisions",
        MAX_REVISIONS          = "max_revisions"              : "Maximum number of revisions configured",
        DELETED_LIST_LEN       = "deleted_list_len"           : "Length of deleted list for committed revisions",
        PERMITS_AVAILABLE      = "persist.permits_available"  : "Number of persist permits currently available",
        MAX_PERMITS            = "persist.max_permits"        : "Maximum number of persist permits"
    }
}
