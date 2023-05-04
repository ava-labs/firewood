use super::{Metric, MetricRecorder};

#[derive(Debug, Default)]
pub struct NoopMetricRecorder;

impl NoopMetricRecorder {
    pub fn new() -> Self {
        Self {}
    }
}

impl MetricRecorder for NoopMetricRecorder {
    /// Intentional no-op that drops the metric
    fn increment(&mut self, _metric: Metric, _count: u64) {}
}
