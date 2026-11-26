// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! The cost of one metric update through the Firewood macros, with the Prometheus collector
//! installed, against a pre-registered handle and the uncached `metrics::counter!` call the
//! macros used to make.

use std::hint::black_box;

use criterion::Criterion;
use criterion::criterion_group;
use criterion::criterion_main;
use firewood_metrics::__private::MAX_ENTRIES;
use firewood_metrics::firewood_counter;
use metrics_collector_prometheus::PrometheusCollectorBuilder;

/// Metric names; the macros resolve `crate::registry::NAME`.
mod registry {
    pub const BENCH_FIRST: &str = "bench_first_total";
    pub const BENCH_LABELLED: &str = "bench_labelled_total";
    pub const BENCH_SPILL: &str = "bench_spill_total";
    pub const BENCH_UNCACHED: &str = "bench_uncached_total";
}

/// Label values; the cache compares them by content, so any `&'static str` works.
const VALUES: [&str; 8] = ["v0", "v1", "v2", "v3", "v4", "v5", "v6", "v7"];

fn labelled(value: &'static str) {
    firewood_counter!(BENCH_LABELLED, "v" => value).increment(1);
}

fn spill(value: &'static str) {
    firewood_counter!(BENCH_SPILL, "v" => value).increment(1);
}

fn bench_handle_cache(criterion: &mut Criterion) {
    let _collector = PrometheusCollectorBuilder::new()
        .install()
        .expect("install the Prometheus collector");

    let mut group = criterion.benchmark_group("metric_update");

    group.bench_function("cached/first_entry", |b| {
        b.iter(|| firewood_counter!(BENCH_FIRST).increment(black_box(1)));
    });

    // Fill eight entries, then hit the last one, so every hit scans all eight.
    for value in VALUES {
        labelled(value);
    }
    let [.., last] = VALUES;
    group.bench_function("cached/eighth_entry", |b| {
        b.iter(|| labelled(black_box(last)));
    });

    // Fill all but one entry, then alternate two more values: each call scans a full list,
    // misses, registers, and replaces the last entry.
    let fill: Vec<&'static str> = (0..MAX_ENTRIES - 1)
        .map(|i| &*String::leak(format!("fill-{i}")))
        .collect();
    for &value in &fill {
        spill(value);
    }
    let mut alternating = ["spill-a", "spill-b"].into_iter().cycle();
    group.bench_function("cached/full_miss", |b| {
        b.iter(|| {
            if let Some(value) = alternating.next() {
                spill(black_box(value));
            }
        });
    });

    let handle = metrics::counter!("bench_pre_registered_total", "db_tag" => "untagged");
    group.bench_function("handle/pre_registered", |b| {
        b.iter(|| handle.increment(black_box(1)));
    });

    group.bench_function("uncached", |b| {
        b.iter(|| {
            metrics::counter!(
                registry::BENCH_UNCACHED,
                "db_tag" => firewood_metrics::current_db_tag_label()
            )
            .increment(black_box(1));
        });
    });

    group.finish();
}

criterion_group!(benches, bench_handle_cache);
criterion_main!(benches);
