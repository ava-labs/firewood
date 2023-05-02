use prometheus_client::encoding::text::encode;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::registry::Registry;

use super::{Metric, MetricRecorder, MetricSet};

pub struct PrometheusMetricRecorder<T> {
    registry: Registry,
    metric_set: MetricSet<T>,
}

impl<T> PrometheusMetricRecorder<T> {
    /// Create a new prometheus metrics instance.
    pub fn new(registry: Option<Registry>) -> Result<Self, std::fmt::Error> {
        Ok(Self{
            registry: registry.unwrap_or_default(),
            metric_set: MetricSet::new(),
        })
    }

    /// Gather the metrics from the registry and encode them.
    pub fn gather(&self) -> Result<String, std::fmt::Error> {
        let mut buffer = String::new();
        encode(&mut buffer, &self.registry)?;
        Ok(buffer)
    }
}

impl<T> MetricRecorder for PrometheusMetricRecorder<T> {
    fn increment(&mut self, _metric: Metric, _count: u64) {
        todo!()
    }
}

#[cfg(test)]
mod test {}
