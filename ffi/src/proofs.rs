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

/// Converts a C `u64` byte budget for the sizer, rejecting one this platform
/// cannot represent rather than silently altering the request.
pub(crate) fn budget_to_usize(budget: u64) -> Result<usize, firewood::api::Error> {
    usize::try_from(budget).map_err(|_| {
        firewood::api::Error::InternalError(
            format!("budget of {budget} bytes exceeds this platform's address space").into(),
        )
    })
}
