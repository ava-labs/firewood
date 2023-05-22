use prometheus_client::metrics::counter::Counter;
use prometheus_client::registry::Registry;

use std::{collections::HashMap, sync::atomic::AtomicU64};
use strum::IntoEnumIterator;

use super::{Metric, MetricRecorder};

#[derive(Debug)]
pub struct PrometheusMetricRecorder(HashMap<Metric, Counter>);

impl PrometheusMetricRecorder {
    /// Create a new prometheus metrics instance.
    pub fn new(mut registry: Registry) -> Result<Self, std::fmt::Error> {
        let hm: HashMap<Metric, Counter> = Metric::iter()
            .map(|metric| {
                let counter = Counter::<u64, AtomicU64>::default();
                let name = metric.to_string().to_lowercase();
                let help = metric.help().to_string();
                registry.register(name, help, counter.clone());
                (metric, counter)
            })
            .collect();

        Ok(Self(hm))
    }
}

impl MetricRecorder for PrometheusMetricRecorder {
    fn increment(&mut self, metric: Metric, count: u64) {
        let counter = self.0.get(&metric).expect("missing metric?");
        counter.inc_by(count);
    }
}
