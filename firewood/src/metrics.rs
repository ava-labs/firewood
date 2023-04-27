#![allow(dead_code)]

use prometheus_client::encoding::text::encode;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::registry::Registry;

/// Defines the behavior of metrics that Firewood will track.
pub trait CounterMetricRecorder {
    fn increment(&mut self, metric: Metric);
}

#[non_exhaustive]
#[derive(Clone, Debug)]
pub enum Metric {
    IoKeyRead(Counter),
    IoKeyWrite(Counter),
    HashCalculated(Counter),
}

pub type MetricSet = Vec<Metric>;

pub struct PrometheusMetricRecorder {
    registry: Registry,
    metric_set: MetricSet,
}

impl PrometheusMetricRecorder {
    /// Create a new Firewood metrics instance.
    fn new(registry: Option<Registry>) -> Result<Self, std::fmt::Error> {
        let mut registry = registry.unwrap_or_default();
        let mut metric_set = MetricSet::default();

        let io_key_read = Counter::<u64>::default();
        metric_set.push(Metric::IoKeyRead(io_key_read.clone()));
        let io_key_write = Counter::<u64>::default();
        metric_set.push(Metric::IoKeyWrite(io_key_write.clone()));
        let hash_calculated = Counter::<u64>::default();
        metric_set.push(Metric::HashCalculated(hash_calculated.clone()));

        registry.register("io_key_read", "Number of keys read from disk", io_key_read);

        registry.register(
            "io_key_write",
            "Number of keys written to disk",
            io_key_write,
        );

        registry.register(
            "hash_calculated",
            "Number of hashes calculated",
            hash_calculated,
        );

        Ok(Self {
            registry,
            metric_set,
        })
    }

    /// Gather the metrics from the registry and encode them.
    fn gather(&self) -> Result<String, std::fmt::Error> {
        let mut buffer = String::new();
        encode(&mut buffer, &self.registry)?;
        Ok(buffer)
    }
}

impl CounterMetricRecorder for PrometheusMetricRecorder {
    fn increment(&mut self, metric: Metric) {
        match metric {
            Metric::IoKeyRead(c) => c.inc(),
            Metric::IoKeyWrite(c) => c.inc(),
            Metric::HashCalculated(c) => c.inc(),
        };
    }
}

#[cfg(test)]
mod test {}
