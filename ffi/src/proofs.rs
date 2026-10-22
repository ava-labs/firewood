// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

mod change;
mod code_hash;
mod eth;
mod range;

// Re-export firewood-side types that used to be defined in this crate, so
// downstream FFI code (e.g. `crate::value::results`) can keep importing
// them via `crate::KeyRange`.
pub use firewood::KeyRange;

pub use self::change::*;
pub use self::code_hash::*;
pub use self::eth::*;
pub use self::range::*;

/// Rejects a proof parsed in hash mode `found` unless `db` uses the same mode,
/// so verification never hashes it under the wrong algorithm.
fn check_hash_mode(
    db: &crate::DatabaseHandle,
    found: firewood::NodeHashAlgorithm,
) -> Result<(), firewood::api::Error> {
    let expected = db.node_hash_algorithm();
    if found == expected {
        Ok(())
    } else {
        Err(firewood::api::Error::ProofError(
            firewood::ProofError::HashModeMismatch { expected, found },
        ))
    }
}
