// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Replay layer metric definitions.

firewood_metrics::define_metrics! {
    counters: {
        PROPOSE_NS    = "replay.propose_ns" : "Time spent in propose (ns)",
        PROPOSE_COUNT = "replay.propose"    : "Number of propose calls",
        COMMIT_NS     = "replay.commit_ns"  : "Time spent in commit (ns)",
        COMMIT_COUNT  = "replay.commit"     : "Number of commit calls"
    },
    gauges: {}
}
