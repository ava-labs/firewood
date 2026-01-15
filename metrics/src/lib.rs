// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
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
//! // At startup
//! registry::register();
//!
//! // Always increment
//! firewood_increment!(registry::COMMIT_COUNT, 1);
//!
//! // Expensive histogram (only records if expensive metrics enabled)
//! firewood_record!(registry::COMMIT_MS_BUCKET, elapsed_ms, expensive);
//! ```
//!
//! # Macro Overview
//!
//! All recording macros accept an optional trailing `expensive` flag:
//!
//! | Macro | Description |
//! |-------|-------------|
//! | `firewood_increment!(name, value)` | Always increment a counter |
//! | `firewood_increment!(name, value, expensive)` | Increment only if expensive metrics enabled |
//! | `firewood_set!(name, value)` | Always set a gauge value |
//! | `firewood_set!(name, value, expensive)` | Set only if expensive metrics enabled |
//! | `firewood_record!(name, value)` | Always record to histogram |
//! | `firewood_record!(name, value, expensive)` | Record only if expensive metrics enabled |
//!
//! For registration, use `describe_*` or `describe_*_expensive` macros.

use std::cell::Cell;

// Re-export metrics crate macros for registration
pub use metrics::{describe_counter, describe_gauge, describe_histogram};

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

/// An RAII guard that restores the previous thread-local [`MetricsContext`].
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
/// firewood_increment!(registry::COMMIT_COUNT, 1);
/// firewood_increment!(registry::COMMIT_COUNT, 1, "label" => "value");
/// firewood_increment!(registry::SLOW_OP_COUNT, 1, expensive);
/// ```
#[macro_export]
macro_rules! firewood_increment {
    ($name:expr, $value:expr, expensive) => {
        if $crate::expensive_metrics_enabled() {
            metrics::counter!($name).increment($value);
        }
    };
    ($name:expr, $value:expr) => {
        metrics::counter!($name).increment($value)
    };
    ($name:expr, $value:expr, $($labels:tt)+) => {
        metrics::counter!($name, $($labels)+).increment($value)
    };
}

/// Returns a counter handle for advanced operations.
///
/// # Usage
/// ```ignore
/// let counter = firewood_counter!(registry::COMMIT_COUNT);
/// counter.increment(1);
/// counter.absolute(100);
/// ```
#[macro_export]
macro_rules! firewood_counter {
    ($name:expr) => {
        metrics::counter!($name)
    };
    ($name:expr, $($labels:tt)+) => {
        metrics::counter!($name, $($labels)+)
    };
}

/// Sets a gauge metric value.
///
/// # Usage
/// ```ignore
/// firewood_set!(registry::ACTIVE_REVISIONS, count as f64);
/// firewood_set!(registry::QUEUE_SIZE, size, "queue" => "main");
/// firewood_set!(registry::DETAILED_STAT, value, expensive);
/// ```
#[macro_export]
macro_rules! firewood_set {
    ($name:expr, $value:expr, expensive) => {
        if $crate::expensive_metrics_enabled() {
            metrics::gauge!($name).set($value);
        }
    };
    ($name:expr, $value:expr) => {
        metrics::gauge!($name).set($value)
    };
    ($name:expr, $value:expr, $($labels:tt)+) => {
        metrics::gauge!($name, $($labels)+).set($value)
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
        metrics::gauge!($name)
    };
    ($name:expr, $($labels:tt)+) => {
        metrics::gauge!($name, $($labels)+)
    };
}

/// Records a value to a histogram metric.
///
/// # Usage
/// ```ignore
/// firewood_record!(registry::LATENCY_MS, elapsed_ms);
/// firewood_record!(registry::LATENCY_MS, elapsed_ms, "op" => "read");
/// firewood_record!(registry::COMMIT_MS_BUCKET, elapsed_ms, expensive);
/// ```
#[macro_export]
macro_rules! firewood_record {
    ($name:expr, $value:expr, expensive) => {
        if $crate::expensive_metrics_enabled() {
            metrics::histogram!($name).record($value);
        }
    };
    ($name:expr, $value:expr) => {
        metrics::histogram!($name).record($value)
    };
    ($name:expr, $value:expr, $($labels:tt)+) => {
        metrics::histogram!($name, $($labels)+).record($value)
    };
}

/// Returns a histogram handle for advanced operations.
///
/// # Usage
/// ```ignore
/// let histogram = firewood_histogram!(registry::LATENCY_MS);
/// histogram.record(elapsed_ms);
/// ```
#[macro_export]
macro_rules! firewood_histogram {
    ($name:expr) => {
        metrics::histogram!($name)
    };
    ($name:expr, $($labels:tt)+) => {
        metrics::histogram!($name, $($labels)+)
    };
}

/// Describes a counter metric marked as expensive.
///
/// Appends " (expensive)" to the description for clarity.
///
/// # Usage
/// ```ignore
/// describe_counter_expensive!(SLOW_OP_COUNT, "Count of slow operations");
/// ```
#[macro_export]
macro_rules! describe_counter_expensive {
    ($name:expr, $description:expr) => {
        metrics::describe_counter!($name, concat!($description, " (expensive)"))
    };
}

/// Describes a gauge metric marked as expensive.
#[macro_export]
macro_rules! describe_gauge_expensive {
    ($name:expr, $description:expr) => {
        metrics::describe_gauge!($name, concat!($description, " (expensive)"))
    };
}

/// Describes a histogram metric marked as expensive.
#[macro_export]
macro_rules! describe_histogram_expensive {
    ($name:expr, $description:expr) => {
        metrics::describe_histogram!($name, concat!($description, " (expensive)"))
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_defaults_to_none() {
        METRICS_CONTEXT.set(None);
        assert_eq!(current_metrics_context(), None);
        assert!(!expensive_metrics_enabled());
    }

    #[test]
    fn context_is_set_and_restored() {
        METRICS_CONTEXT.set(None);
        let ctx1 = MetricsContext::new(false);
        let ctx2 = MetricsContext::new(true);

        let guard1 = set_metrics_context(Some(ctx1));
        assert_eq!(current_metrics_context(), Some(ctx1));
        assert!(!expensive_metrics_enabled());

        {
            let _guard2 = set_metrics_context(Some(ctx2));
            assert_eq!(current_metrics_context(), Some(ctx2));
            assert!(expensive_metrics_enabled());
        }

        assert_eq!(current_metrics_context(), Some(ctx1));
        assert!(!expensive_metrics_enabled());

        drop(guard1);
        assert_eq!(current_metrics_context(), None);
        assert!(!expensive_metrics_enabled());
    }

    #[test]
    fn expensive_gating() {
        METRICS_CONTEXT.set(None);
        assert!(!expensive_metrics_enabled());

        let _guard = set_metrics_context(Some(MetricsContext::new(true)));
        assert!(expensive_metrics_enabled());
    }
}
