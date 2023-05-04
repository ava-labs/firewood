use strum_macros::{Display, EnumIter};

pub mod noop;
pub mod prometheus;

/// Defines the behavior of metric recorders that can be used with firewood.
/// Each MetricRecorder has its own implementation of the MetricRecorder trait.
/// Options include Prometheus, Console (print to stdout), and Noop (do nothing).
pub trait MetricRecorder {
    /// Increment the counter metric by count
    fn increment(&mut self, metric: Metric, count: u64);
}

#[derive(Display, Debug, PartialEq, Eq, Hash, EnumIter, Clone)]
#[strum(serialize_all = "snake_case")]
pub enum Metric {
    IOKeyRead,
    IOKeyWrite,
    HashCalculated,
}

impl Metric {
    fn help(&self) -> &'static str {
        match self {
            Metric::IOKeyRead => "Number of keys read from the input file",
            Metric::IOKeyWrite => "Number of keys written to the output file",
            Metric::HashCalculated => "Number of hashes calculated",
        }
    }
}
