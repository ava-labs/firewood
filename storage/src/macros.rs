
/// Macro to register and use a metric with description and labels.
/// This macro is a wrapper around the `metrics` crate's `counter!` and `describe_counter!`
/// macros. It ensures that the description is registered only once.
///
/// Usage:
///   firewood_metric!("metric_name", "description", value_to_increment_by)
///   firewood_metric!("metric_name", "description", value_to_increment_by, "label" => "value")
#[macro_export]
macro_rules! firewood_metric {
    // With labels
    ($name:expr, $desc:expr, $inc:expr, $($labels:tt)+) => {
        {
            static METRIC: std::sync::LazyLock<()> = std::sync::LazyLock::new(|| {
                metrics::describe_counter!($name, $desc);
            });
            metrics::counter!($name, $($labels)+).increment($inc);
        }
    };
    // No labels
    ($name:expr, $desc:expr, $inc:expr) => {
        {
            static METRIC: std::sync::LazyLock<()> = std::sync::LazyLock::new(|| {
                metrics::describe_counter!($name, $desc);
            });
            metrics::counter!($name).increment($inc);
        }
    };
}