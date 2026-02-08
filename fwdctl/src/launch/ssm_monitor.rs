// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use aws_config::BehaviorVersion;
use aws_sdk_ssm::Client as SsmClient;
use aws_sdk_ssm::types::{InstanceInformationFilter, InstanceInformationFilterKey, PingStatus};
use log::info;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tokio::time::sleep;

use super::LaunchError;
use super::cloud_init::STATE_FILE;

const SSM_MAX_RETRIES: u32 = 30;
const SSM_RETRY_DELAY: Duration = Duration::from_secs(5);
const SSM_COMMAND_POLL_INTERVAL: Duration = Duration::from_millis(500);
const SSM_COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
const LOG_POLL_INTERVAL: Duration = Duration::from_secs(3);
const LOG_CHUNK_SIZE: u64 = 500;
const BOOTSTRAP_LOG: &str = "/var/log/bootstrap.log";

async fn aws_config(region: Option<&str>) -> aws_config::SdkConfig {
    let mut loader = aws_config::defaults(BehaviorVersion::latest());
    if let Some(r) = region {
        loader = loader.region(aws_config::Region::new(r.to_owned()));
    }
    loader.load().await
}

pub async fn ssm_client(region: &str) -> SsmClient {
    SsmClient::new(&aws_config(Some(region)).await)
}

pub async fn wait_for_ssm_registration(
    ssm: &SsmClient,
    instance_id: &str,
) -> Result<(), LaunchError> {
    for attempt in 1..=SSM_MAX_RETRIES {
        let filter = InstanceInformationFilter::builder()
            .key(InstanceInformationFilterKey::InstanceIds)
            .value_set(instance_id)
            .build()
            .map_err(|e| LaunchError::AwsSdk(format!("failed to build SSM filter: {e}")))?;

        if ssm
            .describe_instance_information()
            .instance_information_filter_list(filter)
            .send()
            .await
            .ok()
            .map(|info| {
                info.instance_information_list()
                    .iter()
                    .any(|i| i.ping_status() == Some(&PingStatus::Online))
            })
            .unwrap_or(false)
        {
            return Ok(());
        }

        log::debug!("Waiting for SSM registration ({attempt}/{SSM_MAX_RETRIES})");
        sleep(SSM_RETRY_DELAY).await;
    }

    Err(LaunchError::AwsSdk(format!(
        "instance {instance_id} did not register with SSM"
    )))
}

pub async fn stream_logs_via_ssm(
    ssm: &SsmClient,
    instance_id: &str,
    observe: bool,
) -> Result<(), LaunchError> {
    let mut tracker = StageTracker::default();

    loop {
        if let Some(state) =
            read_optional_json::<CloudInitState>(ssm, instance_id, STATE_FILE).await?
        {
            tracker.update(&state);
            if state.status == "failed" {
                return Err(LaunchError::AwsSdk(failure_context(&state)));
            }
            if state.status == "completed" && state.step == state.total {
                info!("");
                info!("All {} stages completed.", state.total);
                break;
            }
        }

        sleep(LOG_POLL_INTERVAL).await;
    }

    if observe {
        info!("Observing re-execution progress from {}...", BOOTSTRAP_LOG);
        stream_bootstrap_log(ssm, instance_id).await?;
    }

    Ok(())
}

async fn stream_bootstrap_log(ssm: &SsmClient, instance_id: &str) -> Result<(), LaunchError> {
    let mut last_line = 0_u64;
    let mut observer = LogObserver::default();

    loop {
        let (output, lines_read) =
            read_log_chunk(ssm, instance_id, BOOTSTRAP_LOG, last_line + 1).await?;
        if !output.is_empty() {
            for line in output.lines() {
                if observer.process_line(line) {
                    observer.print_summary();
                    return Ok(());
                }
            }
            last_line += lines_read;
        }
        sleep(LOG_POLL_INTERVAL).await;
    }
}

#[derive(Debug, Deserialize)]
struct CloudInitState {
    step: usize,
    total: usize,
    name: String,
    status: String,
    #[serde(default)]
    stages: Vec<String>,
    #[serde(default)]
    commands: HashMap<String, Vec<String>>,
    #[serde(default)]
    last_error: Option<CommandError>,
}

#[derive(Debug, Deserialize)]
struct CommandError {
    stage: usize,
    cmd: usize,
    exit: i64,
}

#[derive(Default)]
struct StageTracker {
    shown_step: usize,
    shown_completed: bool,
}

impl StageTracker {
    fn update(&mut self, state: &CloudInitState) {
        for skipped in (self.shown_step + 1)..state.step {
            info!(
                "[{:>2}/{}] ✓ {}",
                skipped,
                state.total,
                stage_name(state, skipped)
            );
        }

        let is_complete = state.status == "completed";
        let should_print = state.step > self.shown_step
            || (state.step == self.shown_step && is_complete && !self.shown_completed);
        if should_print {
            let symbol = match state.status.as_str() {
                "completed" => "✓",
                "failed" => "✗",
                _ => "…",
            };
            info!(
                "[{:>2}/{}] {} {}",
                state.step, state.total, symbol, state.name
            );
            self.shown_step = state.step;
            self.shown_completed = is_complete;
        }
    }
}

fn failure_context(state: &CloudInitState) -> String {
    let Some(err) = &state.last_error else {
        return format!("stage {} [{}] failed", state.step, state.name);
    };
    let command = state
        .commands
        .get(&err.stage.to_string())
        .and_then(|commands| commands.get(err.cmd.saturating_sub(1)))
        .map(String::as_str)
        .unwrap_or("?");
    format!(
        "stage {} [{}] failed (exit={}) while running: {}",
        err.stage,
        stage_name(state, err.stage),
        err.exit,
        command
    )
}

fn stage_name(state: &CloudInitState, stage: usize) -> &str {
    state
        .stages
        .get(stage.saturating_sub(1))
        .map(String::as_str)
        .unwrap_or("?")
}

#[derive(Default)]
struct LogObserver {
    last_progress: Option<(u64, f64)>,
    results: Vec<(String, String)>,
}

impl LogObserver {
    /// Process a line and return `true` once re-execution has finished.
    fn process_line(&mut self, line: &str) -> bool {
        if line.contains("executing block") && line.contains("progress_pct") {
            if let Some((height, pct, eta)) = Self::parse_progress(line)
                && self.should_show(height, pct)
            {
                eprint!("\r[{:>6.1}%] block {:>10} | eta: {:>8}", pct, height, eta);
                self.last_progress = Some((height, pct));
            }
            return false;
        }

        if line.contains("BenchmarkReexecuteRange") && line.contains("result") {
            if let Some((metric, value)) = Self::parse_result(line) {
                self.results.push((metric, value));
            }
            return false;
        }

        if line.contains("finished executing sequence") {
            if self.last_progress.is_some() {
                eprintln!();
            }
            return true;
        }

        false
    }

    fn should_show(&self, height: u64, pct: f64) -> bool {
        match self.last_progress {
            None => true,
            Some((h, p)) => (pct - p).abs() >= 0.5 || height.saturating_sub(h) >= 50_000,
        }
    }

    fn parse_progress(line: &str) -> Option<(u64, f64, String)> {
        #[derive(Deserialize)]
        struct ProgressLine {
            height: u64,
            progress_pct: f64,
            #[serde(default)]
            eta: String,
        }

        let parsed: ProgressLine = parse_embedded_json(line)?;
        let eta = if parsed.eta.is_empty() {
            "-".into()
        } else {
            parsed.eta
        };
        Some((parsed.height, parsed.progress_pct, eta))
    }

    fn parse_result(line: &str) -> Option<(String, String)> {
        #[derive(Deserialize)]
        struct ResultLine {
            result: String,
        }

        let parsed: ResultLine = parse_embedded_json(line)?;
        let mut parts = parsed.result.split_whitespace();
        let value = parts.next()?.to_owned();
        let metric = parts.collect::<Vec<_>>().join(" ");
        Some(if metric.is_empty() {
            ("result".into(), value)
        } else {
            (metric, value)
        })
    }

    fn print_summary(&self) {
        if self.results.is_empty() {
            return;
        }
        info!("");
        info!("=== Benchmark Results ===");
        for (metric, value) in &self.results {
            info!("  {:<30} {}", metric, value);
        }
    }
}

fn parse_embedded_json<T: DeserializeOwned>(line: &str) -> Option<T> {
    let json_start = line.find('{')?;
    let json_end = line.rfind('}')?;
    serde_json::from_str(&line[json_start..=json_end]).ok()
}

async fn read_optional_json<T: DeserializeOwned>(
    ssm: &SsmClient,
    instance_id: &str,
    path: &str,
) -> Result<Option<T>, LaunchError> {
    let output =
        run_ssm_command(ssm, instance_id, &format!("cat {path} 2>/dev/null || true")).await?;
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    match serde_json::from_str(trimmed) {
        Ok(value) => Ok(Some(value)),
        Err(err) => {
            log::debug!("Ignoring malformed JSON in {path}: {err}");
            Ok(None)
        }
    }
}

async fn read_log_chunk(
    ssm: &SsmClient,
    instance_id: &str,
    log_path: &str,
    start_line: u64,
) -> Result<(String, u64), LaunchError> {
    let command = format!(
        "sudo sed -n '{start},{end}p' {log_path} 2>/dev/null || true",
        start = start_line,
        end = start_line + LOG_CHUNK_SIZE - 1,
    );
    let output = run_ssm_command(ssm, instance_id, &command).await?;
    let lines_read = output.lines().count() as u64;
    Ok((output, lines_read))
}

async fn run_ssm_command(
    ssm: &SsmClient,
    instance_id: &str,
    command: &str,
) -> Result<String, LaunchError> {
    let resp = ssm
        .send_command()
        .document_name("AWS-RunShellScript")
        .instance_ids(instance_id)
        .parameters("commands", vec![command.to_owned()])
        .send()
        .await?;

    let command_id = resp
        .command()
        .and_then(|c| c.command_id())
        .map(str::to_owned)
        .ok_or_else(|| LaunchError::AwsSdk("missing SSM command ID".into()))?;

    let started = Instant::now();
    loop {
        if started.elapsed() > SSM_COMMAND_TIMEOUT {
            return Err(LaunchError::Timeout(
                "ssm command",
                SSM_COMMAND_TIMEOUT.as_secs(),
            ));
        }

        sleep(SSM_COMMAND_POLL_INTERVAL).await;
        let invocation = ssm
            .get_command_invocation()
            .command_id(&command_id)
            .instance_id(instance_id)
            .send()
            .await;

        let resp = match invocation {
            Ok(resp) => resp,
            Err(err) if err.to_string().contains("InvocationDoesNotExist") => continue,
            Err(err) => return Err(err.into()),
        };

        let status = resp.status().map(|s| s.as_str()).unwrap_or("Pending");
        match status {
            "Pending" | "InProgress" | "Delayed" => continue,
            "Success" => return Ok(resp.standard_output_content().unwrap_or("").to_owned()),
            _ => {
                let detail = resp
                    .standard_error_content()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .or_else(|| {
                        resp.standard_output_content()
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                    })
                    .unwrap_or("no output");
                return Err(LaunchError::AwsSdk(format!(
                    "SSM command failed with status '{status}': {detail}"
                )));
            }
        }
    }
}
