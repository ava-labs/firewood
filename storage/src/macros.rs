// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.
/// Macro to register and use a metric with description and labels.
/// This macro is a wrapper around the `metrics` crate's `counter!` and `describe_counter!`
/// macros. It ensures that the description is registered only once.
///
/// Usage:
///   firewood_metric!("metric_name", "description")
///   firewood_metric!("metric_name", "description", "label" => "value")
///
/// Call `.increment(val)` or `.absolute(val)` on the result as appropriate.
#[macro_export]
macro_rules! firewood_metric {
    // With labels
    ($name:expr, $desc:expr, $($labels:tt)+) => {
        {
            static ONCE: std::sync::Once = std::sync::Once::new();
            ONCE.call_once(|| {
                metrics::describe_counter!($name, $desc);
            });
            metrics::counter!($name, $($labels)+)
        }
    };
    // No labels
    ($name:expr, $desc:expr) => {
        {
            static ONCE: std::sync::Once = std::sync::Once::new();
            ONCE.call_once(|| {
                metrics::describe_counter!($name, $desc);
            });
            metrics::counter!($name)
        }
    };
}