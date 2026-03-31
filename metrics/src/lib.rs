// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Shared metric helpers for Firewood.
//!
//! This crate provides:
//! - Recording macros that are simple to use at callsites
//! - Thread-local context for gating expensive metrics
//!
//! Each crate defines its own metric registry (e.g., `ffi::registry`, `storage::registry`).
//!
//! # Usage
//!
//! ```ignore
//! use firewood_metrics::firewood_increment;
//! use crate::registry;
//!
//! registry::register();
//!
//! firewood_increment!(registry::OP_COUNT, 1);
//! ```
//!
//! # Macro overview
//!
//! Recording macros accept an optional trailing `expensive` flag where noted:
//!
//! | Macro | Description |
//! |-------|-------------|
//! | `firewood_increment!(name, value)` | Always increment a counter |
//! | `firewood_increment!(name, value, expensive)` | Increment only if expensive metrics enabled |
//! | `firewood_set!(name, value)` | Always set a gauge value |
//! | `firewood_set!(name, value, expensive)` | Set only if expensive metrics enabled |
//!
//! For registration, use `metrics::describe_*` macros in your crate's `register()` function.

use std::cell::Cell;

/// Metric configuration context for the current thread.
///
/// This is set at API boundaries (e.g., FFI entrypoints) and read when deciding
/// whether to record expensive metrics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MetricsContext {
    expensive_metrics_enabled: bool,
}

impl MetricsContext {
    /// Create a new [`MetricsContext`].
    #[must_use]
    pub const fn new(expensive_metrics_enabled: bool) -> Self {
        Self {
            expensive_metrics_enabled,
        }
    }

    /// Whether expensive metrics are enabled.
    #[must_use]
    pub const fn expensive_metrics_enabled(self) -> bool {
        self.expensive_metrics_enabled
    }
}

thread_local! {
    static METRICS_CONTEXT: Cell<Option<MetricsContext>> = const { Cell::new(None) };
}

/// A guard that restores the previous thread-local [`MetricsContext`].
#[derive(Debug)]
pub struct MetricsContextGuard {
    previous: Option<MetricsContext>,
}

impl Drop for MetricsContextGuard {
    fn drop(&mut self) {
        METRICS_CONTEXT.set(self.previous);
    }
}

/// Sets the thread-local metrics context for the duration of the returned guard.
#[must_use]
pub fn set_metrics_context(context: Option<MetricsContext>) -> MetricsContextGuard {
    let previous = METRICS_CONTEXT.replace(context);
    MetricsContextGuard { previous }
}

/// Returns the current thread-local metrics context, if one is set.
#[must_use]
pub fn current_metrics_context() -> Option<MetricsContext> {
    METRICS_CONTEXT.get()
}

/// Returns whether expensive metrics are enabled for the current thread.
///
/// If no context is set, this returns `false`.
#[must_use]
pub fn expensive_metrics_enabled() -> bool {
    current_metrics_context().is_some_and(MetricsContext::expensive_metrics_enabled)
}

/// Increments a counter metric.
///
/// Use for values that only ever increase: commit counts, error counts, byte
/// totals. Metric names should end in `_total` (e.g. `proposals_created_total`,
/// `bytes_written_total`). For values that can go up or down use
/// [`firewood_set!`] instead.
///
/// # Usage
/// ```ignore
/// firewood_increment!(registry::PROPOSALS_CREATED_TOTAL, 1);
/// firewood_increment!(registry::PROPOSALS_CREATED_TOTAL, 1, "status" => "ok");
/// firewood_increment!(registry::SLOW_PATH_TOTAL, 1, expensive);
/// ```
#[macro_export]
macro_rules! firewood_increment {
    ($name:expr, $value:expr, expensive) => {
        if $crate::expensive_metrics_enabled() {
            ::metrics::counter!($name).increment($value);
        }
    };
    ($name:expr, $value:expr) => {
        ::metrics::counter!($name).increment($value)
    };
    ($name:expr, $value:expr, $($labels:tt)+) => {
        ::metrics::counter!($name, $($labels)+).increment($value)
    };
}

/// Returns a counter handle for advanced operations.
///
/// Prefer [`firewood_increment!`] for simple increment-by-N callsites.
/// Use this when you need to call `.absolute()` or reuse the handle across
/// multiple operations without a repeated name lookup.
///
/// # Usage
/// ```ignore
/// let counter = firewood_counter!(registry::PROPOSALS_CREATED_TOTAL);
/// counter.increment(1);
/// counter.absolute(100);
/// ```
#[macro_export]
macro_rules! firewood_counter {
    ($name:expr) => {
        ::metrics::counter!($name)
    };
    ($name:expr, $($labels:tt)+) => {
        ::metrics::counter!($name, $($labels)+)
    };
}

/// Sets a gauge metric value.
///
/// Use for values that can go up or down: active revision counts, queue
/// depths, cache sizes. Metric names should include the unit where applicable
/// (e.g. `node_cache_bytes`, `active_revisions`). Do not use a `_total`
/// suffix — that is reserved for counters. For values that only ever increase
/// use [`firewood_increment!`] instead.
///
/// # Usage
/// ```ignore
/// firewood_set!(registry::ACTIVE_REVISIONS, count);
/// firewood_set!(registry::NODE_CACHE_BYTES, size, "tier" => "l1");
/// firewood_set!(registry::PENDING_PROPOSALS, count, expensive);
/// ```
#[macro_export]
macro_rules! firewood_set {
    ($name:expr, $value:expr, expensive) => {
        if $crate::expensive_metrics_enabled() {
            ::metrics::gauge!($name).set($value as f64);
        }
    };
    ($name:expr, $value:expr) => {
        ::metrics::gauge!($name).set($value as f64)
    };
    ($name:expr, $value:expr, $($labels:tt)+) => {
        ::metrics::gauge!($name, $($labels)+).set($value as f64)
    };
}

/// Returns a gauge handle for advanced operations.
///
/// Prefer [`firewood_set!`] for simple set-value callsites. Use this when
/// you need `.increment()` / `.decrement()` on a gauge or want to reuse the
/// handle across multiple operations.
///
/// # Usage
/// ```ignore
/// let gauge = firewood_gauge!(registry::ACTIVE_REVISIONS);
/// gauge.set(10.0);
/// gauge.increment(1.0);
/// gauge.decrement(1.0);
/// ```
#[macro_export]
macro_rules! firewood_gauge {
    ($name:expr) => {
        ::metrics::gauge!($name)
    };
    ($name:expr, $($labels:tt)+) => {
        ::metrics::gauge!($name, $($labels)+)
    };
}

#[cfg(test)]
mod tests {
    #![expect(clippy::unwrap_used)]

    use super::*;

    fn isolated<F, R>(f: F) -> R
    where
        F: FnOnce() -> R,
    {
        let _guard = set_metrics_context(None);
        f()
    }

    #[test]
    fn context_defaults_to_none() {
        isolated(|| {
            assert_eq!(current_metrics_context(), None);
            assert!(!expensive_metrics_enabled());
        });
    }

    #[test]
    fn nested_guards_restore_in_correct_order() {
        isolated(|| {
            let outer = MetricsContext::new(false);
            let inner = MetricsContext::new(true);

            let guard1 = set_metrics_context(Some(outer));
            {
                let _guard2 = set_metrics_context(Some(inner));
                assert_eq!(current_metrics_context(), Some(inner));
            }
            assert_eq!(current_metrics_context(), Some(outer));

            drop(guard1);
            assert_eq!(current_metrics_context(), None);
        });
    }

    #[test]
    fn spawned_thread_does_not_inherit_context() {
        isolated(|| {
            let ctx = MetricsContext::new(true);
            let _guard = set_metrics_context(Some(ctx));
            assert_eq!(current_metrics_context(), Some(ctx));

            let child_ctx = std::thread::spawn(current_metrics_context).join().unwrap();

            assert_eq!(
                child_ctx, None,
                "thread-local context must not leak to child threads"
            );
        });
    }

    #[test]
    fn capture_and_set_propagates_context_to_spawned_thread() {
        isolated(|| {
            let ctx = MetricsContext::new(true);
            let _guard = set_metrics_context(Some(ctx));

            // Capture on the parent thread, just like persist_worker.rs does.
            let captured = current_metrics_context();

            let child_ctx = std::thread::spawn(move || {
                // Before setting, the child has no context.
                assert_eq!(current_metrics_context(), None);

                let _inner_guard = set_metrics_context(captured);

                // After setting, the child sees the propagated context.
                current_metrics_context()
            })
            .join()
            .unwrap();

            assert_eq!(
                child_ctx,
                Some(ctx),
                "captured context must be available in child thread after set_metrics_context"
            );

            // Parent context is unaffected.
            assert_eq!(current_metrics_context(), Some(ctx));
        });
    }
}
