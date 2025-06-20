
/// Registers and increments a Prometheus counter with description and labels.
/// Usage:
///   firewood_metric!(NAME, DESC, LABELS, [label_values], inc_value)
///   firewood_metric!(NAME, DESC, [], [], inc_value) for no labels
#[macro_export]
macro_rules! firewood_metric {
    // With labels
    ($name:expr, $desc:expr, [$($label:expr),*], [$($value:expr),*], $inc:expr) => {{
        static METRIC: std::sync::LazyLock<prometheus::IntCounterVec> = std::sync::LazyLock::new(|| {
            prometheus::register_int_counter_vec!($name, $desc, &[$($label),*]).expect("metric registration failure")
        });
        METRIC.with_label_values(&[$($value),*]).inc_by($inc);
    }};
    // No labels
    ($name:expr, $desc:expr, [], [], $inc:expr) => {{
        static METRIC: std::sync::LazyLock<prometheus::IntCounter> = std::sync::LazyLock::new(|| {
            prometheus::register_int_counter!($name, $desc).expect("metric registration failure")
        });
        METRIC.inc_by($inc);
    }};
}