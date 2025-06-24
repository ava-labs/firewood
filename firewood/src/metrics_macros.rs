// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Metrics macros for timing operations

/// Starts a metrics timer by returning the current time.
///
/// This macro generates: `coarsetime::Instant::now()`
///
/// # Example
/// ```rust
/// use firewood::{start_metrics, finish_metrics};
/// let start = start_metrics!();
/// // ... do some work ...
/// let result: Result<(), &str> = Ok(());
/// finish_metrics!("my.operation", result, start);
/// ```
#[macro_export]
macro_rules! start_metrics {
    () => {
        coarsetime::Instant::now()
    };
}

/// Finishes a metrics timer by incrementing two counters:
/// 1. A timing counter with "_ms" suffix that records elapsed time in milliseconds
/// 2. A count counter with the base name that increments by 1
///
/// This macro generates:
/// - `counter!(metric_name, "success" => success_status).increment(1)`
/// - `counter!(metric_name_ms, "success" => success_status).increment(start.elapsed().as_millis())`
///   where `success_status` is "false" if the result is an error, "true" if it's ok.
///
/// Returns the original Result value.
///
/// # Parameters
/// - `metric_name`: The base name of the metric counter
/// - `result`: A `Result<T, E>` value to determine success/failure
/// - `start`: The start time returned by `start_metrics!()`
///
/// # Returns
/// The original `Result<T, E>` that was passed in
///
/// # Example
/// ```rust
/// use firewood::{start_metrics, finish_metrics};
/// fn time_something() -> Result<(), &str>> {
///     let start = start_metrics!();
///     let result_of_work: Result<(), &str> = Ok(());
///     finish_metrics!("example.operations", result_of_work, start)
/// }
/// assert_eq!(time_something(), OK(()));
/// ```
#[macro_export]
macro_rules! finish_metrics {
    ($metric_name:expr, $result:expr, $start:expr) => {{
        let result_value = $result;
        let success_label = if result_value.is_err() { "false" } else { "true" };

        // Increment count counter (base name)
        metrics::counter!($metric_name, "success" => success_label).increment(1);

        // Increment timing counter (base name + "_ms")
        metrics::counter!(concat!($metric_name, "_ms"), "success" => success_label).increment($start.elapsed().as_millis());

        result_value
    }};
}

pub use {finish_metrics, start_metrics};

#[cfg(test)]
mod tests {
    #[test]
    fn metrics_macros() {
        // This test verifies that the macros work together properly
        let start = start_metrics!();

        let result: Result<(), &str> = Ok(());
        let final_result = finish_metrics!("test.metric", result, start);
        assert_eq!(result, final_result);
    }
}
