// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Replay layer metric definitions.

firewood_metrics::define_metrics! {
    counters: {
        /// Time spent in propose (ns)
        PROPOSE_NS    = "replay.propose_ns",
        /// Number of propose calls
        PROPOSE_COUNT = "replay.propose",
        /// Time spent in commit (ns)
        COMMIT_NS     = "replay.commit_ns",
        /// Number of commit calls
        COMMIT_COUNT  = "replay.commit",
    },
}
