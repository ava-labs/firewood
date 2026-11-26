// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Metric proxies that cache their handle per call site and thread.
//!
//! A proxy names one metric at one call site. Each method finds the handle that call site
//! registered for the thread's `db_tag`, the label values, and the active recorder, registers
//! one on a miss, and updates it in place. The proxies never clone a handle, so an update is
//! the handle's own atomic operations and nothing more.

use std::fmt;
use std::thread::LocalKey;

use metrics::Counter;
use metrics::Gauge;
use metrics::Histogram;
use metrics::IntoF64;
use metrics::Key;
use metrics::Label;
use metrics::Metadata;
use metrics::Recorder;
use metrics::SharedString;

use crate::cache::EntryKey;
use crate::cache::HandleCache;
use crate::cache::recorder_generation;
use crate::with_context;

/// Whether a metric records only when expensive metrics are enabled.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    /// Always records.
    Always,
    /// Records only when the thread's metrics context enables expensive metrics.
    Expensive,
}

/// How a handle type is registered with a recorder.
trait Register: Sized {
    fn register(recorder: &dyn Recorder, key: &Key, metadata: &Metadata<'_>) -> Self;
}

impl Register for Counter {
    fn register(recorder: &dyn Recorder, key: &Key, metadata: &Metadata<'_>) -> Self {
        recorder.register_counter(key, metadata)
    }
}

impl Register for Gauge {
    fn register(recorder: &dyn Recorder, key: &Key, metadata: &Metadata<'_>) -> Self {
        recorder.register_gauge(key, metadata)
    }
}

impl Register for Histogram {
    fn register(recorder: &dyn Recorder, key: &Key, metadata: &Metadata<'_>) -> Self {
        recorder.register_histogram(key, metadata)
    }
}

/// The data address of `recorder`, which identifies it while it is installed.
fn recorder_address(recorder: &dyn Recorder) -> usize {
    std::ptr::from_ref(recorder).cast::<()>().addr()
}

/// One metric at one call site, shared by the three proxies.
struct Site<H: 'static, const N: usize> {
    cache: &'static LocalKey<HandleCache<H, N>>,
    metadata: &'static Metadata<'static>,
    name: &'static str,
    keys: &'static [&'static str; N],
    values: [&'static str; N],
    gate: Gate,
}

impl<H: 'static, const N: usize> Clone for Site<H, N> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<H: 'static, const N: usize> Copy for Site<H, N> {}

impl<H: 'static, const N: usize> fmt::Debug for Site<H, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Site")
            .field("name", &self.name)
            .field("keys", self.keys)
            .field("values", &self.values)
            .field("gate", &self.gate)
            .finish_non_exhaustive()
    }
}

impl<H: Register + 'static, const N: usize> Site<H, N> {
    /// The key today's uncached macros built: `db_tag` first, then the call-site labels.
    fn metric_key(&self, db_tag: SharedString) -> Key {
        let labels = std::iter::once(Label::new("db_tag", db_tag))
            .chain(
                self.keys
                    .iter()
                    .zip(self.values)
                    .map(|(key, value)| Label::from_static_parts(key, value)),
            )
            .collect::<Vec<_>>();
        Key::from_parts(self.name, labels)
    }

    /// Applies `op` to this site's handle for the thread's context and active recorder.
    fn apply(self, op: impl FnOnce(&H)) {
        let mut op = Some(op);

        // Look up the handle while the context's tag is borrowed. A miss copies the tag out,
        // so registration below runs with the context released.
        let miss = with_context(|db_tag, expensive| {
            if self.gate == Gate::Expensive && !expensive {
                return None;
            }
            let key = EntryKey {
                generation: recorder_generation(),
                recorder: metrics::with_recorder(recorder_address),
                values: &self.values,
                db_tag,
            };
            let hit = self
                .cache
                .try_with(|cache| {
                    cache.with_hit(&key, |handle| {
                        if let Some(op) = op.take() {
                            op(handle);
                        }
                    })
                })
                .ok()
                .flatten();
            match hit {
                Some(()) => None,
                None => Some((key.generation, db_tag.clone())),
            }
        });

        let (Some((generation, db_tag)), Some(op)) = (miss, op) else {
            return;
        };
        metrics::with_recorder(|recorder| {
            let handle = H::register(recorder, &self.metric_key(db_tag.clone()), self.metadata);
            op(&handle);
            let key = EntryKey {
                generation,
                recorder: recorder_address(recorder),
                values: &self.values,
                db_tag: &db_tag,
            };
            // During thread teardown the cache is gone and the handle is simply dropped.
            let _ = self.cache.try_with(|cache| cache.insert(&key, handle));
        });
    }
}

macro_rules! proxy {
    ($(#[$doc:meta])* $proxy:ident, $handle:ty) => {
        $(#[$doc])*
        #[doc(hidden)]
        #[must_use = "a metric proxy records nothing until one of its methods is called"]
        #[derive(Clone, Copy, Debug)]
        pub struct $proxy<const N: usize>(Site<$handle, N>);

        impl<const N: usize> $proxy<N> {
            /// A proxy for the metric `name` at the call site that owns `cache`.
            pub const fn new(
                cache: &'static LocalKey<HandleCache<$handle, N>>,
                metadata: &'static Metadata<'static>,
                name: &'static str,
                keys: &'static [&'static str; N],
                values: [&'static str; N],
                gate: Gate,
            ) -> Self {
                Self(Site {
                    cache,
                    metadata,
                    name,
                    keys,
                    values,
                    gate,
                })
            }
        }
    };
}

proxy!(
    /// A counter at one call site; see [`firewood_counter!`](crate::firewood_counter).
    CachedCounter,
    Counter
);
proxy!(
    /// A gauge at one call site; see [`firewood_gauge!`](crate::firewood_gauge).
    CachedGauge,
    Gauge
);
proxy!(
    /// A histogram at one call site; see [`firewood_histogram!`](crate::firewood_histogram).
    CachedHistogram,
    Histogram
);

impl<const N: usize> CachedCounter<N> {
    /// Increments the counter by `value`.
    pub fn increment(self, value: u64) {
        self.0.apply(|counter| counter.increment(value));
    }

    /// Sets the counter to `value` if it is larger than the current value.
    pub fn absolute(self, value: u64) {
        self.0.apply(|counter| counter.absolute(value));
    }
}

impl<const N: usize> CachedGauge<N> {
    /// Increments the gauge by `value`.
    pub fn increment<T: IntoF64>(self, value: T) {
        self.0.apply(|gauge| gauge.increment(value));
    }

    /// Decrements the gauge by `value`.
    pub fn decrement<T: IntoF64>(self, value: T) {
        self.0.apply(|gauge| gauge.decrement(value));
    }

    /// Sets the gauge to `value`.
    pub fn set<T: IntoF64>(self, value: T) {
        self.0.apply(|gauge| gauge.set(value));
    }
}

impl<const N: usize> CachedHistogram<N> {
    /// Records one observation of `value`.
    pub fn record<T: IntoF64>(self, value: T) {
        self.0.apply(|histogram| histogram.record(value));
    }

    /// Records `count` observations of `value`.
    pub fn record_many<T: IntoF64>(self, value: T, count: usize) {
        self.0
            .apply(|histogram| histogram.record_many(value, count));
    }
}

impl<const N: usize> crate::CounterExt for CachedCounter<N> {
    fn increment_f64(&self, value: f64) {
        self.0.apply(|counter| counter.increment_f64(value));
    }
}

impl<const N: usize> crate::GaugeExt for CachedGauge<N> {
    fn set_integer<T: crate::gauge_value::GaugeValue>(&self, value: T) {
        self.0.apply(|gauge| gauge.set_integer(value));
    }
}

impl<const N: usize> crate::HistogramExt for CachedHistogram<N> {
    fn record_integer<T: crate::gauge_value::GaugeValue>(&self, value: T) {
        self.0.apply(|histogram| histogram.record_integer(value));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use metrics_util::CompositeKey;
    use metrics_util::MetricKind;
    use metrics_util::debugging::DebugValue;
    use metrics_util::debugging::DebuggingRecorder;

    use super::*;
    use crate::CounterExt;
    use crate::GaugeExt;
    use crate::HistogramExt;
    use crate::MetricsContext;
    use crate::MetricsContextGuard;
    use crate::cache::MAX_ENTRIES;
    use crate::current_db_tag_label;
    use crate::set_metrics_context;
    use crate::with_local_recorder;

    /// Builds a proxy by hand, as the macros do, so each test names its own metric.
    macro_rules! site {
        ($proxy:ident, $handle:ty, $name:literal, $gate:expr $(, $k:literal => $v:expr)*) => {{
            const N: usize = <[&str]>::len(&[$($k),*]);
            thread_local! {
                static CACHE: HandleCache<$handle, N> = const { HandleCache::new() };
            }
            static METADATA: Metadata<'static> =
                Metadata::new(module_path!(), metrics::Level::INFO, Some(module_path!()));
            static KEYS: [&str; N] = [$($k),*];
            $proxy::<N>::new(&CACHE, &METADATA, $name, &KEYS, [$($v),*], $gate)
        }};
    }

    fn tagged(db_tag: &'static str) -> MetricsContextGuard {
        set_metrics_context(Some(
            MetricsContext::new(false).with_db_tag(Some(db_tag.into())),
        ))
    }

    const UNTAGGED: (&str, &str) = ("db_tag", "untagged");

    /// Everything a recorder received while `f` ran with it installed. Reading a
    /// `DebuggingRecorder` resets it, so this reads it exactly once.
    struct Recorded(Vec<(CompositeKey, DebugValue)>);

    impl Recorded {
        fn during(f: impl FnOnce()) -> Self {
            let recorder = DebuggingRecorder::new();
            let snapshotter = recorder.snapshotter();
            with_local_recorder(&recorder, f);
            let values = snapshotter.snapshot().into_vec().into_iter();
            Self(values.map(|(key, _, _, value)| (key, value)).collect())
        }

        /// The value recorded for `name` with exactly `labels`, in order.
        fn get(
            &self,
            kind: MetricKind,
            name: &str,
            labels: &[(&str, &str)],
        ) -> Option<&DebugValue> {
            self.0.iter().find_map(|(key, value)| {
                let mut recorded = key.key().labels().map(|label| (label.key(), label.value()));
                (key.kind() == kind
                    && key.key().name() == name
                    && recorded.by_ref().eq(labels.iter().copied()))
                .then_some(value)
            })
        }

        fn counter(&self, name: &str, labels: &[(&str, &str)]) -> u64 {
            match self.get(MetricKind::Counter, name, labels) {
                Some(DebugValue::Counter(count)) => *count,
                _ => 0,
            }
        }
    }

    /// Updates under two `db_tag`s on one thread land in two series with their own counts.
    #[test]
    fn db_tags_record_separate_series() {
        let updates = [("db-a", 3), ("db-b", 2)];
        let recorded = Recorded::during(|| {
            for (db_tag, count) in updates {
                let _guard = tagged(db_tag);
                for _ in 0..count {
                    site!(CachedCounter, Counter, "ops", Gate::Always).increment(1);
                }
            }
        });
        for (db_tag, count) in updates {
            assert_eq!(
                recorded.counter("ops", &[("db_tag", db_tag)]),
                count,
                "{db_tag}"
            );
        }
    }

    /// Runtime label values record one series per value, after the `db_tag`, including more
    /// combinations than one call site caches.
    #[test]
    fn runtime_label_values_record_separate_series() {
        const ROUNDS: u64 = 2;
        let values: Vec<&'static str> = (0..MAX_ENTRIES + 8)
            .map(|i| &*String::leak(format!("v{i}")))
            .collect();
        let recorded = Recorded::during(|| {
            for _ in 0..ROUNDS {
                for &value in &values {
                    site!(CachedCounter, Counter, "spread", Gate::Always, "k" => "fixed", "v" => value)
                        .increment(1);
                }
            }
        });
        for value in values {
            let labels = [UNTAGGED, ("k", "fixed"), ("v", value)];
            assert_eq!(recorded.counter("spread", &labels), ROUNDS, "{value}");
        }
    }

    /// Each recorder sees only the updates made while it was active: one installed after the
    /// call site's first update, two used one after the other, and one nested in another.
    #[test]
    fn each_recorder_sees_only_its_own_updates() {
        fn bump() {
            site!(CachedCounter, Counter, "scoped", Gate::Always).increment(1);
        }
        bump();
        for round in 0..2 {
            let recorded = Recorded::during(bump);
            assert_eq!(recorded.counter("scoped", &[UNTAGGED]), 1, "round {round}");
        }
        let mut inner = None;
        let outer = Recorded::during(|| {
            bump();
            inner = Some(Recorded::during(bump));
            bump();
        });
        let inner = inner.expect("the inner scope ran");
        assert_eq!(outer.counter("scoped", &[UNTAGGED]), 2);
        assert_eq!(inner.counter("scoped", &[UNTAGGED]), 1);
    }

    /// An expensive metric records only when the context enables expensive metrics.
    #[test]
    fn expensive_metrics_follow_the_context() {
        let recorded = Recorded::during(|| {
            for enabled in [false, true] {
                let _guard = set_metrics_context(Some(MetricsContext::new(enabled)));
                site!(CachedCounter, Counter, "costly", Gate::Expensive).increment(1);
            }
        });
        assert_eq!(recorded.counter("costly", &[UNTAGGED]), 1);
    }

    /// Every counter, gauge, and histogram operation reaches the handle, including the
    /// extension-trait conversions.
    #[test]
    fn all_operations_reach_the_handle() {
        let recorded = Recorded::during(|| {
            site!(CachedCounter, Counter, "c", Gate::Always).increment(2);
            site!(CachedCounter, Counter, "c", Gate::Always).increment_f64(3.9);
            site!(CachedCounter, Counter, "c_abs", Gate::Always).absolute(10);
            site!(CachedGauge, Gauge, "g", Gate::Always).set(4.0);
            site!(CachedGauge, Gauge, "g", Gate::Always).increment(2.0);
            site!(CachedGauge, Gauge, "g", Gate::Always).decrement(1.0);
            site!(CachedGauge, Gauge, "g_int", Gate::Always).set_integer(7_u64);
            site!(CachedHistogram, Histogram, "h", Gate::Always).record(1.5);
            site!(CachedHistogram, Histogram, "h", Gate::Always).record_many(2.5, 2);
            site!(CachedHistogram, Histogram, "h", Gate::Always).record_integer(3_usize);
        });
        let gauge = |name| recorded.get(MetricKind::Gauge, name, &[UNTAGGED]);
        let histogram = recorded.get(MetricKind::Histogram, "h", &[UNTAGGED]);
        assert_eq!(recorded.counter("c", &[UNTAGGED]), 5);
        assert_eq!(recorded.counter("c_abs", &[UNTAGGED]), 10);
        assert_eq!(gauge("g"), Some(&DebugValue::Gauge(5.0.into())));
        assert_eq!(gauge("g_int"), Some(&DebugValue::Gauge(7.0.into())));
        let expected = [1.5, 2.5, 2.5, 3.0].map(Into::into).to_vec();
        assert_eq!(histogram, Some(&DebugValue::Histogram(expected)));
    }

    /// Updating a metric and reading the context from a thread-local destructor, after the
    /// call site's cache and the metrics context are destroyed, falls back to untagged
    /// instead of panicking.
    #[test]
    fn metrics_during_thread_teardown_do_not_panic() {
        fn bump() {
            site!(CachedCounter, Counter, "teardown", Gate::Always).increment(1);
        }
        struct Probe(mpsc::Sender<String>);
        impl Drop for Probe {
            fn drop(&mut self) {
                bump();
                let _ = self.0.send(current_db_tag_label().to_string());
            }
        }
        thread_local! {
            static PROBE: std::cell::RefCell<Option<Probe>> =
                const { std::cell::RefCell::new(None) };
        }

        let (report, seen) = mpsc::channel();
        std::thread::spawn(move || {
            // Thread-locals are destroyed in reverse order of first use, so registering the
            // probe first makes it run after the context and the call site's cache are gone.
            PROBE.with(|probe| *probe.borrow_mut() = Some(Probe(report)));
            let _guard = tagged("db-a");
            bump();
        })
        .join()
        .expect("the probe thread panicked");
        assert_eq!(
            seen.recv().as_deref(),
            Ok("untagged"),
            "the probe read a stale context"
        );
    }
}
