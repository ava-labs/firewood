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
/// # Usage
/// ```ignore
/// firewood_increment!(registry::OP_COUNT, 1);
/// firewood_increment!(registry::OP_COUNT, 1, "label" => "value");
/// firewood_increment!(registry::SLOW_OP_COUNT, 1, expensive);
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
/// # Usage
/// ```ignore
/// let counter = firewood_counter!(registry::OP_COUNT);
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
/// # Usage
/// ```ignore
/// firewood_set!(registry::ACTIVE_REVISIONS, count);
/// firewood_set!(registry::QUEUE_SIZE, size, "queue" => "main");
/// firewood_set!(registry::DETAILED_STAT, value, expensive);
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

/// Describes a counter metric with a human-readable description.
///
/// `$name` must be a string literal so the metrics key can be cached in a
/// per-callsite `static`, avoiding an allocation on every call.
///
/// # Examples
///
/// ```rust
/// firewood_describe_counter!("proposals_created_total", "Number of proposals created");
/// firewood_describe_counter!("io_read_seconds_total", Unit::Seconds, "Time spent on IO reads");
/// ```
#[macro_export]
macro_rules! firewood_describe_counter {
    ($name:literal, $desc:expr) => {
        ::metrics::describe_counter!($name, $desc)
    };
    ($name:literal, $unit:expr, $desc:expr) => {
        ::metrics::describe_counter!($name, $unit, $desc)
    };
}

/// Describes a gauge metric with a human-readable description.
///
/// `$name` must be a string literal so the metrics key can be cached in a
/// per-callsite `static`, avoiding an allocation on every call.
///
/// # Examples
///
/// ```rust
/// firewood_describe_gauge!("active_revisions", "Number of revisions currently held in memory");
/// firewood_describe_gauge!("node_cache_bytes", Unit::Bytes, "Current node cache size");
/// ```
#[macro_export]
macro_rules! firewood_describe_gauge {
    ($name:literal, $desc:expr) => {
        ::metrics::describe_gauge!($name, $desc)
    };
    ($name:literal, $unit:expr, $desc:expr) => {
        ::metrics::describe_gauge!($name, $unit, $desc)
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
