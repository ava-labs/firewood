// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use firewood_metrics::MetricsContext;

use crate::VoidResult;

/// A sized request's budget and, after a chunk, its calibration for the next
/// one. Create with [`fwd_sizing_hint_new`]; each sized request consumes the
/// hint it is given and returns a new one with its result. Free an unused
/// hint with [`fwd_free_sizing_hint`].
#[derive(Debug)]
pub struct SizingHint(pub(crate) firewood::SizingHint);

/// Creates a hint for chunks whose serialized size never exceeds `budget`
/// bytes.
///
/// # Returns
///
/// A [`SizingHint`] to pass to a sized proof request, which consumes it, or to
/// free with [`fwd_free_sizing_hint`].
#[unsafe(no_mangle)]
pub extern "C" fn fwd_sizing_hint_new(budget: usize) -> Box<SizingHint> {
    Box::new(SizingHint(firewood::SizingHint::new(budget)))
}

/// Frees the memory associated with a [`SizingHint`] that was not passed to
/// a sized proof request.
///
/// # Arguments
///
/// * `hint` - The [`SizingHint`] to free, previously returned from any Rust function.
///
/// # Returns
///
/// - [`VoidResult::Ok`] if the memory was successfully freed.
/// - [`VoidResult::Err`] if the process panics while freeing the memory.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_free_sizing_hint(hint: Option<Box<SizingHint>>) -> VoidResult {
    crate::invoke_with_handle(hint, drop)
}

impl crate::MetricsContextExt for SizingHint {
    fn metrics_context(&self) -> Option<MetricsContext> {
        None
    }
}
