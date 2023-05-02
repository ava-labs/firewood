use std::collections::HashMap;

pub mod noop;
pub mod prometheus;

/// Defines the behavior of metric recorders that can be used with firewood.
/// Each MetricRecorder has its own implementation of the MetricRecorder trait.
/// Options include Prometheus, Console (print to stdout), and Noop (do nothing).
pub trait MetricRecorder {
    /// Increment the counter metric by count
    fn increment(&mut self, metric: Metric, count: u64);
}

#[non_exhaustive]
#[derive(Clone, Debug)]
/// The set of metrics for firewood to track.
pub enum Metric {
    IOKeyRead,
    IOKeyWrite,
    HashCalculated,
}

pub type MetricSet<T> = HashMap<Metric, T>;
