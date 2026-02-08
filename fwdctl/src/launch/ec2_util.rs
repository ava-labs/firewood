// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use aws_config::BehaviorVersion;
use aws_sdk_ec2::Client as Ec2Client;
use aws_sdk_ec2::types::{
    ArchitectureType, BlockDeviceMapping, EbsBlockDevice, Filter, IamInstanceProfileSpecification,
    InstanceType as Ec2InstanceType, ResourceType, Tag, TagSpecification, VolumeType,
};
use aws_sdk_ssm::Client as SsmClient;
use aws_sdk_ssm::types::{InstanceInformationFilter, InstanceInformationFilterKey, PingStatus};
use aws_sdk_sts::Client as StsClient;
use log::info;
use serde::Deserialize;
use tokio::sync::OnceCell;
use tokio::time::{sleep, timeout};

use super::cloud_init::{COMMANDS_FILE, ERROR_LOG, PROGRESS_FILE, STAGES_FILE};
use super::{DeployOptions, LaunchError};

const WAIT_RUNNING_TIMEOUT: Duration = Duration::from_secs(300);
const POLL_INTERVAL_RUNNING: Duration = Duration::from_secs(3);
const SSM_MAX_RETRIES: u32 = 30;
const SSM_RETRY_DELAY: Duration = Duration::from_secs(5);
const SSM_COMMAND_POLL_INTERVAL: Duration = Duration::from_millis(500);
const SSM_COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
const LOG_POLL_INTERVAL: Duration = Duration::from_secs(3);
const LOG_CHUNK_SIZE: u32 = 500;
const BOOTSTRAP_LOG: &str = "/var/log/bootstrap.log";

static AWS_USERNAME: OnceCell<String> = OnceCell::const_new();

async fn aws_config(region: Option<&str>) -> aws_config::SdkConfig {
    let mut loader = aws_config::defaults(BehaviorVersion::latest());
    if let Some(r) = region {
        loader = loader.region(aws_config::Region::new(r.to_owned()));
    }
    loader.load().await
}

pub async fn ec2_client(region: &str) -> Ec2Client {
    Ec2Client::new(&aws_config(Some(region)).await)
}

pub async fn ssm_client(region: &str) -> SsmClient {
    SsmClient::new(&aws_config(Some(region)).await)
}

async fn get_instance_architecture(
    client: &Ec2Client,
    instance_type: &str,
) -> Result<Option<ArchitectureType>, LaunchError> {
    let parsed = parse_instance_type(instance_type)?;
    let resp = client
        .describe_instance_types()
        .instance_types(parsed)
        .send()
        .await?;

    let arch = resp
        .instance_types()
        .first()
        .and_then(|i| i.processor_info())
        .map(aws_sdk_ec2::types::ProcessorInfo::supported_architectures)
        .and_then(|a| a.first().cloned());

    Ok(arch)
}

pub async fn latest_ubuntu_ami(
    ec2: &Ec2Client,
    instance_type: &str,
) -> Result<String, LaunchError> {
    let arch = match get_instance_architecture(ec2, instance_type).await? {
        Some(ArchitectureType::Arm64) => "arm64",
        Some(ArchitectureType::X8664) => "amd64",
        _ => {
            return Err(LaunchError::InvalidInstanceType(
                instance_type.to_string(),
                "unsupported architecture".to_string(),
            ));
        }
    };
    let name_pattern = format!("ubuntu/images/hvm-ssd-gp3/ubuntu-noble-24.04-{arch}-server-*");

    let response = ec2
        .describe_images()
        .owners("099720109477")
        .filters(Filter::builder().name("name").values(&name_pattern).build())
        .filters(Filter::builder().name("state").values("available").build())
        .send()
        .await?;

    let mut images = response.images().to_vec();
    images.sort_by_key(|img| img.creation_date().unwrap_or_default().to_string());

    images
        .last()
        .and_then(|img| img.image_id())
        .map(String::from)
        .ok_or_else(|| LaunchError::NoMatchingAmi(arch.to_string()))
}

pub async fn launch_instance(
    ec2: &Ec2Client,
    ami_id: &str,
    opts: &DeployOptions,
    user_data_b64: &str,
) -> Result<String, LaunchError> {
    let instance_type = parse_instance_type(&opts.instance_type)?;
    let instance_name = build_instance_name(opts);
    let username = get_aws_username().await;
    let tags = build_tags(opts, &instance_name, &username);

    let root_volume = BlockDeviceMapping::builder()
        .device_name("/dev/sda1")
        .ebs(
            EbsBlockDevice::builder()
                .volume_size(50)
                .volume_type(VolumeType::Gp3)
                .build(),
        )
        .build();

    let mut request = ec2
        .run_instances()
        .image_id(ami_id)
        .instance_type(instance_type)
        .user_data(user_data_b64)
        .min_count(1)
        .max_count(1)
        .block_device_mappings(root_volume)
        .tag_specifications(
            TagSpecification::builder()
                .resource_type(ResourceType::Instance)
                .set_tags(Some(tags))
                .build(),
        );

    if let Some(key) = &opts.key_name {
        request = request.key_name(key);
    }
    if !opts.security_group_ids.is_empty() {
        request = request.set_security_group_ids(Some(opts.security_group_ids.clone()));
    }
    if !opts.iam_instance_profile_name.is_empty() {
        request = request.iam_instance_profile(
            IamInstanceProfileSpecification::builder()
                .name(&opts.iam_instance_profile_name)
                .build(),
        );
    }

    info!("Requesting EC2 instance: {}", opts.instance_type);
    let response = request.send().await?;

    response
        .instances()
        .first()
        .and_then(|i| i.instance_id())
        .map(String::from)
        .ok_or(LaunchError::MissingInstanceId)
}

fn parse_instance_type(s: &str) -> Result<Ec2InstanceType, LaunchError> {
    use std::str::FromStr;
    Ec2InstanceType::from_str(s)
        .map_err(|_| LaunchError::InvalidInstanceType(s.to_owned(), "SDK parse error".to_owned()))
}

fn build_instance_name(opts: &DeployOptions) -> String {
    let mut name = format!("{}-{:08X}", opts.name_prefix, rand::random::<u32>());
    for (prefix, (_, branch)) in ["-fw-", "-ag-", "-le-"].into_iter().zip(opts.branches()) {
        if let Some(b) = branch {
            name.push_str(prefix);
            name.push_str(b);
        }
    }
    name
}

fn build_tags(opts: &DeployOptions, instance_name: &str, username: &str) -> Vec<Tag> {
    let tag = |k: &str, v: &str| Tag::builder().key(k).value(v).build();
    let mut tags = vec![
        tag("Name", instance_name),
        tag("Component", "firewood"),
        tag("ManagedBy", "fwdctl"),
        tag("LaunchedBy", username),
    ];
    if let Some(t) = &opts.custom_tag {
        tags.push(tag("CustomTag", t));
    }
    tags.extend(
        ["FirewoodBranch", "AvalancheGoBranch", "LibEVMBranch"]
            .into_iter()
            .zip(opts.branches())
            .filter_map(|(tag_name, (_, branch))| branch.map(|value| tag(tag_name, value))),
    );
    tags
}

pub async fn wait_for_running(ec2: &Ec2Client, instance_id: &str) -> Result<(), LaunchError> {
    info!("Waiting for instance to enter 'running' state...");

    timeout(WAIT_RUNNING_TIMEOUT, async {
        loop {
            let state = get_instance_state(ec2, instance_id).await?;
            match state.as_str() {
                "running" => return Ok(()),
                "terminated" | "shutting-down" | "stopped" | "stopping" => {
                    return Err(LaunchError::TerminalInstanceState {
                        instance_id: instance_id.to_owned(),
                        state,
                    });
                }
                _ => sleep(POLL_INTERVAL_RUNNING).await,
            }
        }
    })
    .await
    .map_err(|_| LaunchError::Timeout("instance running", WAIT_RUNNING_TIMEOUT.as_secs()))?
}

async fn get_instance_state(ec2: &Ec2Client, instance_id: &str) -> Result<String, LaunchError> {
    let response = ec2
        .describe_instances()
        .instance_ids(instance_id)
        .send()
        .await?;

    Ok(response
        .reservations()
        .first()
        .and_then(|r| r.instances().first())
        .and_then(|i| i.state())
        .and_then(|s| s.name())
        .map_or("unknown", aws_sdk_ec2::types::InstanceStateName::as_str)
        .to_owned())
}

pub async fn describe_ips(
    ec2: &Ec2Client,
    instance_id: &str,
) -> Result<(Option<String>, Option<String>), LaunchError> {
    let response = ec2
        .describe_instances()
        .instance_ids(instance_id)
        .send()
        .await?;
    let i = response
        .reservations()
        .first()
        .and_then(|r| r.instances().first());
    Ok(i.map(|i| {
        (
            i.public_ip_address().map(Into::into),
            i.private_ip_address().map(Into::into),
        )
    })
    .unwrap_or_default())
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

        let resp = ssm
            .describe_instance_information()
            .instance_information_filter_list(filter)
            .send()
            .await;

        if let Ok(info) = resp {
            let registered = info
                .instance_information_list()
                .iter()
                .any(|i| i.ping_status() == Some(&PingStatus::Online));
            if registered {
                return Ok(());
            }
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
    let mut tracker = StageTracker::new();

    loop {
        if tracker.stage_names.is_empty()
            && let Ok(names) = read_stage_names(ssm, instance_id).await
        {
            tracker.stage_names = names;
        }

        if tracker.commands.is_empty()
            && let Ok(commands) = read_commands(ssm, instance_id).await
        {
            tracker.commands = commands;
        }

        if let Some(progress) = read_progress(ssm, instance_id).await? {
            tracker.update(&progress);
            if progress.status == "failed" {
                let context = read_latest_error(ssm, instance_id)
                    .await?
                    .map(|line| tracker.context_from_error_line(&line))
                    .or_else(|| tracker.failure_context_from_progress(&progress))
                    .unwrap_or_else(|| {
                        format!("stage {} [{}] failed", progress.step, progress.name)
                    });
                return Err(LaunchError::AwsSdk(context));
            }

            if progress.status == "completed" && progress.step == progress.total {
                info!("");
                info!("All {} stages completed.", progress.total);
                break;
            }
        }

        if let Some(line) = read_latest_error(ssm, instance_id).await? {
            return Err(LaunchError::AwsSdk(tracker.context_from_error_line(&line)));
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
    let mut last_line: u64 = 0;
    let mut observer = LogObserver::new();

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

#[derive(Debug, Deserialize, Default)]
struct StageProgress {
    step: usize,
    total: usize,
    name: String,
    status: String,
}

struct StageTracker {
    shown_step: usize,
    shown_completed: bool,
    stage_names: Vec<String>,
    commands: HashMap<String, Vec<String>>,
}

impl StageTracker {
    fn new() -> Self {
        Self {
            shown_step: 0,
            shown_completed: false,
            stage_names: Vec::new(),
            commands: HashMap::new(),
        }
    }

    fn update(&mut self, progress: &StageProgress) {
        for skipped in (self.shown_step + 1)..progress.step {
            let name = self
                .stage_names
                .get(skipped.saturating_sub(1))
                .map(String::as_str)
                .unwrap_or("?");
            info!("[{:>2}/{}] ✓ {}", skipped, progress.total, name);
        }

        let is_complete = progress.status == "completed";
        let is_new_step = progress.step > self.shown_step;
        let just_completed =
            progress.step == self.shown_step && is_complete && !self.shown_completed;

        if is_new_step || just_completed {
            let symbol = match progress.status.as_str() {
                "completed" => "✓",
                "failed" => "✗",
                _ => "…",
            };
            info!(
                "[{:>2}/{}] {} {}",
                progress.step, progress.total, symbol, progress.name
            );
            self.shown_step = progress.step;
            self.shown_completed = is_complete;
        }
    }

    fn failure_context_from_progress(&self, progress: &StageProgress) -> Option<String> {
        let stage_key = progress.step.to_string();
        let stage_name = self
            .stage_names
            .get(progress.step.saturating_sub(1))
            .map(String::as_str)
            .unwrap_or(progress.name.as_str());

        self.commands.get(&stage_key).and_then(|commands| {
            commands.first().map(|command| {
                format!(
                    "stage {} [{}] failed while running: {}",
                    progress.step, stage_name, command
                )
            })
        })
    }

    fn context_from_error_line(&self, line: &str) -> String {
        let Some(stage) = extract_num(line, "stage=") else {
            return line.to_owned();
        };
        let cmd = extract_num(line, "cmd=");
        let exit = extract_num(line, "exit=").unwrap_or_default();

        let stage_name = self
            .stage_names
            .get(stage.saturating_sub(1))
            .map(String::as_str)
            .unwrap_or("?");
        let command = cmd
            .and_then(|cmd_idx| {
                self.commands
                    .get(&stage.to_string())
                    .and_then(|commands| commands.get(cmd_idx.saturating_sub(1)))
            })
            .map(String::as_str)
            .unwrap_or("?");

        format!("stage {stage} [{stage_name}] failed (exit={exit}) while running: {command}")
    }
}

fn extract_num(line: &str, prefix: &str) -> Option<usize> {
    line.find(prefix).and_then(|start| {
        line[start + prefix.len()..]
            .split(|c: char| !c.is_ascii_digit())
            .next()
            .and_then(|value| value.parse().ok())
    })
}

pub struct LogObserver {
    last_progress: Option<(u64, f64)>,
    results: Vec<(String, String)>,
}

impl LogObserver {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            last_progress: None,
            results: Vec::new(),
        }
    }

    /// Process a line and return `true` once re-execution has finished.
    pub fn process_line(&mut self, line: &str) -> bool {
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
        let json_start = line.find('{')?;
        let json_end = line.rfind('}')?;
        let json_str = &line[json_start..=json_end];

        #[derive(Deserialize)]
        struct ProgressLine {
            height: u64,
            progress_pct: f64,
            #[serde(default)]
            eta: String,
        }

        let parsed: ProgressLine = serde_json::from_str(json_str).ok()?;
        let eta = if parsed.eta.is_empty() {
            "-".into()
        } else {
            parsed.eta
        };
        Some((parsed.height, parsed.progress_pct, eta))
    }

    fn parse_result(line: &str) -> Option<(String, String)> {
        let json_start = line.find('{')?;
        let json_end = line.rfind('}')?;
        let json_str = &line[json_start..=json_end];

        #[derive(Deserialize)]
        struct ResultLine {
            result: String,
        }

        let parsed: ResultLine = serde_json::from_str(json_str).ok()?;
        let mut parts = parsed.result.split_whitespace();
        let value = parts.next()?.to_owned();
        let metric = parts.collect::<Vec<_>>().join(" ");
        if metric.is_empty() {
            Some(("result".into(), value))
        } else {
            Some((metric, value))
        }
    }

    pub fn print_summary(&self) {
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

async fn read_progress(
    ssm: &SsmClient,
    instance_id: &str,
) -> Result<Option<StageProgress>, LaunchError> {
    let output = run_ssm_command(
        ssm,
        instance_id,
        &format!("cat {PROGRESS_FILE} 2>/dev/null || true"),
    )
    .await?;
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    Ok(serde_json::from_str(trimmed).ok())
}

async fn read_stage_names(ssm: &SsmClient, instance_id: &str) -> Result<Vec<String>, LaunchError> {
    let output = run_ssm_command(
        ssm,
        instance_id,
        &format!("cat {STAGES_FILE} 2>/dev/null || true"),
    )
    .await?;
    if output.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(output.trim()).map_err(LaunchError::from)
}

async fn read_commands(
    ssm: &SsmClient,
    instance_id: &str,
) -> Result<HashMap<String, Vec<String>>, LaunchError> {
    let output = run_ssm_command(
        ssm,
        instance_id,
        &format!("cat {COMMANDS_FILE} 2>/dev/null || true"),
    )
    .await?;
    if output.trim().is_empty() {
        return Ok(HashMap::new());
    }
    serde_json::from_str(output.trim()).map_err(LaunchError::from)
}

async fn read_latest_error(
    ssm: &SsmClient,
    instance_id: &str,
) -> Result<Option<String>, LaunchError> {
    let output = run_ssm_command(
        ssm,
        instance_id,
        &format!("tail -n 1 {ERROR_LOG} 2>/dev/null || true"),
    )
    .await?;
    let line = output.trim();
    if line.is_empty() {
        return Ok(None);
    }
    Ok(Some(line.to_owned()))
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
        end = start_line + u64::from(LOG_CHUNK_SIZE) - 1,
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
        .parameters("commands", vec![command.to_string()])
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
            Err(err) => {
                if err.to_string().contains("InvocationDoesNotExist") {
                    continue;
                }
                return Err(err.into());
            }
        };

        let status = resp.status().map(|s| s.as_str()).unwrap_or("Pending");
        match status {
            "Pending" | "InProgress" | "Delayed" => continue,
            "Success" => return Ok(resp.standard_output_content().unwrap_or("").to_string()),
            _ => {
                let stderr = resp.standard_error_content().unwrap_or("").trim();
                let stdout = resp.standard_output_content().unwrap_or("").trim();
                let detail = if !stderr.is_empty() {
                    stderr
                } else if !stdout.is_empty() {
                    stdout
                } else {
                    "no output"
                };
                return Err(LaunchError::AwsSdk(format!(
                    "SSM command failed with status '{status}': {detail}"
                )));
            }
        }
    }
}

/// Returns the AWS username for the current caller identity.
async fn get_aws_username() -> String {
    AWS_USERNAME
        .get_or_init(|| async {
            let fallback_username = || {
                std::env::var("USER")
                    .or_else(|_| std::env::var("USERNAME"))
                    .unwrap_or_else(|_| "unknown".into())
            };

            // STS GetCallerIdentity is region-independent; use default config.
            let sts = StsClient::new(&aws_config(None).await);
            match sts.get_caller_identity().send().await {
                Ok(id) => id
                    .arn()
                    .and_then(|arn| {
                        arn.rsplit('/')
                            .next()
                            .or_else(|| arn.rsplit(':').next())
                            .map(ToOwned::to_owned)
                    })
                    .unwrap_or_else(fallback_username),
                Err(e) => {
                    log::debug!("STS GetCallerIdentity failed: {e}");
                    fallback_username()
                }
            }
        })
        .await
        .clone()
}
