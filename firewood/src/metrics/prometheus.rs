use prometheus_client::metrics::counter::Counter;
use prometheus_client::registry::Registry;

use std::collections::HashMap;
use strum::IntoEnumIterator;

use super::{Metric, MetricRecorder};

pub struct PrometheusMetricRecorder {
    registry: Registry,
    hm: HashMap<Metric, PrometheusStat>,
}

struct PrometheusStat {
    name: String,
    help: String,
}

impl PrometheusStat {
    fn new(name: String, help: String) -> Self {
        Self { name, help }
    }
}

impl PrometheusMetricRecorder {
    /// Create a new prometheus metrics instance.
    pub fn new(registry: Option<Registry>) -> Result<Self, std::fmt::Error> {
        let hm = Metric::iter()
            .map(|metric| {
                (
                    metric.clone(),
                    PrometheusStat::new(
                        metric.to_string().to_lowercase(),
                        metric.help().to_string(),
                    ),
                )
            })
            .collect();

        let registry = registry.unwrap_or_default();

        for (metric, stat) in hm {
            let counter = Counter::default();
            registry.register(stat.name, stat.help, counter.clone());
        }

        Ok(Self { registry, hm })
    }
}

impl MetricRecorder for PrometheusMetricRecorder {
    fn increment(&mut self, _metric: Metric, _count: u64) {
        todo!()
    }
}

#[cfg(test)]
mod test {}
