// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::error::Error;
use std::net::Ipv6Addr;
use std::sync::OnceLock;

use crate::rendered_metrics::MapIntoCollection;
use crate::{OwnedRenderedMetrics, jemalloc_metrics};
use firewood_metrics::MetricsContext;
use firewood_metrics::{HistogramConfig, HistogramMetricConfig};
use metrics_exporter_prometheus::{
    Matcher, NativeHistogramConfig, PrometheusBuilder, PrometheusHandle,
};
use oxhttp::Server;
use oxhttp::model::{Body, Response, StatusCode};
use std::net::Ipv4Addr;
use std::time::Duration;

static RECORDER: OnceLock<PrometheusHandle> = OnceLock::new();

/// Trait for types that carry a [`MetricsContext`].
///
/// Implemented for FFI handle types.
/// Concrete impls live in their respective modules (handle, revision, proposal, iterator).
pub(crate) trait MetricsContextExt {
    fn metrics_context(&self) -> Option<MetricsContext>;
}

// some blanket implementations. can't go with Deref approach because of
// tuple handle in range proofs.
impl<T: MetricsContextExt + ?Sized> MetricsContextExt for Box<T> {
    fn metrics_context(&self) -> Option<MetricsContext> {
        (**self).metrics_context()
    }
}

impl<T: MetricsContextExt + ?Sized> MetricsContextExt for &T {
    fn metrics_context(&self) -> Option<MetricsContext> {
        (**self).metrics_context()
    }
}

impl<T: MetricsContextExt + ?Sized> MetricsContextExt for &mut T {
    fn metrics_context(&self) -> Option<MetricsContext> {
        (**self).metrics_context()
    }
}

/// Starts metrics recorder.
/// This happens on a per-process basis, meaning that the metrics system cannot
/// be initialized if it has already been set up in the same process.
pub fn setup_metrics() -> Result<(), Box<dyn Error>> {
    let mut histogram_configs = crate::registry::register();
    histogram_configs.extend(firewood::registry::register());
    histogram_configs.extend(firewood_storage::registry::register());
    #[cfg(feature = "block-replay")]
    histogram_configs.extend(firewood_replay::registry::register());
    jemalloc_metrics::register(); // does not export histogram configs

    let builder = histogram_configs
        .iter()
        .try_fold(PrometheusBuilder::new(), apply_histogram_config)?;
    let handle = builder.install_recorder()?;

    RECORDER
        .set(handle)
        .map_err(|_| "recorder already initialized")?;

    Ok(())
}

fn apply_histogram_config(
    builder: PrometheusBuilder,
    config: &HistogramMetricConfig,
) -> Result<PrometheusBuilder, Box<dyn Error>> {
    let matcher = Matcher::Full(config.name.to_owned());
    Ok(match config.config {
        HistogramConfig::Buckets(ref buckets) => {
            builder
                .set_buckets_for_metric(matcher, buckets)
                .map_err(|e| format!("failed to set buckets for metric {}: {e}", config.name))?
        }
        HistogramConfig::Native {
            scale,
            max_buckets,
            zero_threshold,
        } => builder.set_native_histogram_for_metric(
            matcher,
            NativeHistogramConfig::new(scale, max_buckets, zero_threshold).map_err(|e| {
                format!(
                    "failed to create native histogram config for metric {}: {e}",
                    config.name
                )
            })?,
        ),
    })
}

/// Starts metrics recorder along with an exporter over a specified port.
/// This happens on a per-process basis, meaning that the metrics system
/// cannot be initialized if it has already been set up in the same process.
pub fn setup_metrics_with_exporter(metrics_port: u16) -> Result<(), Box<dyn Error>> {
    setup_metrics()?;

    let recorder = RECORDER.get().ok_or("recorder not initialized")?;
    Server::new(move |request| {
        if request.method() == "GET" {
            jemalloc_metrics::refresh();
            Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "text/plain")
                .body(Body::from(recorder.render()))
                .expect("failed to build response")
        } else {
            Response::builder()
                .status(StatusCode::METHOD_NOT_ALLOWED)
                .body(Body::from("Method not allowed"))
                .expect("failed to build response")
        }
    })
    .bind((Ipv4Addr::LOCALHOST, metrics_port))
    .bind((Ipv6Addr::LOCALHOST, metrics_port))
    .with_global_timeout(Duration::from_secs(60 * 60))
    .with_max_concurrent_connections(2)
    .spawn()?;
    Ok(())
}

/// Returns the latest metrics for this process.
///
/// Refreshes jemalloc stats before rendering so the returned snapshot is current.
pub fn gather_metrics() -> Result<String, String> {
    let Some(recorder) = RECORDER.get() else {
        return Err(String::from("recorder not initialized"));
    };
    jemalloc_metrics::refresh();
    Ok(recorder.render())
}

pub fn gather_rendered_metrics() -> Result<OwnedRenderedMetrics, String> {
    let recorder = RECORDER.get().ok_or("recorder not initialized")?;
    jemalloc_metrics::refresh();
    let start = std::time::Instant::now();
    let result = recorder.render_snapshot_and_descriptions().map_into();
    let elapsed = start.elapsed();
    metrics::histogram!(crate::registry::GATHER_DURATION_SECONDS).record(elapsed.as_secs_f64());
    Ok(result)
}
