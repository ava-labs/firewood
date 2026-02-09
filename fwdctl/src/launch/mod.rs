// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

mod cloud_init;
mod ec2_util;
mod ssm_monitor;
pub mod stage_config;

use clap::{Args, Subcommand, ValueEnum};
use log::info;
use thiserror::Error;

type FwdError = firewood::v2::api::Error;

#[derive(Debug, Error)]
pub enum LaunchError {
    #[error("Invalid instance type '{0}'. Valid types: {1}")]
    InvalidInstanceType(String, String),

    #[error("EC2 operation failed: {0}")]
    Ec2(Box<aws_sdk_ec2::Error>),

    #[error("SSM operation failed: {0}")]
    Ssm(Box<aws_sdk_ssm::Error>),

    #[error("AWS SDK error: {0}")]
    AwsSdk(String),

    #[error("{0}")]
    Validation(String),

    #[error("Cloud-init generation failed: {0}")]
    CloudInit(#[from] serde_yaml::Error),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Stage configuration error: {0}")]
    StageConfig(#[from] stage_config::ConfigError),

    #[error("Timeout waiting for {0} after {1} seconds")]
    Timeout(&'static str, u64),

    #[error("EC2 API returned no instance ID")]
    MissingInstanceId,

    #[error("No matching Ubuntu AMI found for architecture '{0}'")]
    NoMatchingAmi(String),

    #[error("Instance '{instance_id}' entered terminal state '{state}' before running")]
    TerminalInstanceState { instance_id: String, state: String },

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Encoded user-data size {actual} exceeds EC2 limit {limit} bytes")]
    UserDataTooLarge { actual: usize, limit: usize },
}

impl From<aws_sdk_ec2::Error> for LaunchError {
    fn from(error: aws_sdk_ec2::Error) -> Self {
        Self::Ec2(Box::new(error))
    }
}

impl From<aws_sdk_ssm::Error> for LaunchError {
    fn from(error: aws_sdk_ssm::Error) -> Self {
        Self::Ssm(Box::new(error))
    }
}

impl<E, R> From<aws_smithy_runtime_api::client::result::SdkError<E, R>> for LaunchError
where
    E: std::error::Error + Send + Sync + 'static,
    R: std::fmt::Debug + 'static,
{
    fn from(e: aws_smithy_runtime_api::client::result::SdkError<E, R>) -> Self {
        log::debug!("AWS SDK error: {e:#?}");
        Self::AwsSdk(format_aws_sdk_error(&e))
    }
}

impl From<aws_sdk_ec2::error::BuildError> for LaunchError {
    fn from(e: aws_sdk_ec2::error::BuildError) -> Self {
        log::debug!("AWS build error: {e:#?}");
        Self::AwsSdk(e.to_string())
    }
}

fn format_aws_sdk_error<E, R>(e: &aws_smithy_runtime_api::client::result::SdkError<E, R>) -> String
where
    E: std::error::Error + Send + Sync + 'static,
    R: std::fmt::Debug + 'static,
{
    let chain = error_chain(e);
    if chain.is_empty() {
        "unknown AWS SDK error".into()
    } else {
        chain.join("/")
    }
}

fn error_chain(err: &(dyn std::error::Error + 'static)) -> Vec<String> {
    let mut messages = Vec::new();
    let mut current = Some(err);
    while let Some(e) = current {
        let msg = e.to_string();
        if !msg.is_empty() && messages.last() != Some(&msg) {
            messages.push(msg);
        }
        current = e.source();
    }
    messages
}

#[derive(Debug, Args)]
pub struct Options {
    #[command(subcommand)]
    pub command: LaunchCommand,
}

#[derive(Debug, Subcommand)]
pub enum LaunchCommand {
    Deploy(Box<DeployOptions>),
    Monitor(MonitorOptions),
    List(ListOptions),
    Kill(KillOptions),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, ValueEnum)]
pub enum NBlocks {
    #[value(name = "10k")]
    TenK = 10_000,
    #[value(name = "1m")]
    OneM = 1_000_000,
    #[value(name = "50m")]
    FiftyM = 50_000_000,
}

impl NBlocks {
    #[must_use]
    pub const fn end_block(self) -> u64 {
        self as u64
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TenK => "10k",
            Self::OneM => "1m",
            Self::FiftyM => "50m",
        }
    }
}

#[derive(Debug, Args)]
pub struct DeployOptions {
    /// EC2 instance type
    #[arg(
        long = "instance-type",
        value_name = "TYPE",
        default_value = "i4g.large"
    )]
    pub instance_type: String,

    /// Firewood git branch to checkout
    #[arg(long = "firewood-branch", value_name = "BRANCH")]
    pub firewood_branch: Option<String>,

    /// `AvalancheGo` git branch to checkout
    #[arg(long = "avalanchego-branch", value_name = "BRANCH")]
    pub avalanchego_branch: Option<String>,

    /// `LibEVM` git branch to checkout
    #[arg(long = "libevm-branch", value_name = "BRANCH")]
    pub libevm_branch: Option<String>,

    /// Dataset size to download from S3 (also sets the execution end block)
    #[arg(long = "nblocks", value_name = "SIZE", value_enum, default_value_t = NBlocks::OneM)]
    pub nblocks: NBlocks,

    /// Launch scenario from `benchmark/launch/launch-stages.yaml`
    #[arg(long = "scenario", value_name = "SCENARIO", default_value = "reexecute")]
    pub scenario: String,

    /// VM reexecution config (firewood, hashdb, pathdb, etc.)
    #[arg(long = "config", value_name = "CONFIG", default_value = "firewood")]
    pub config: String,

    /// Enable metrics server during execution (`--metrics-server=false` disables)
    #[arg(
        long = "metrics-server",
        value_name = "BOOL",
        default_value_t = true,
        action = clap::ArgAction::Set
    )]
    pub metrics_server: bool,

    /// AWS region
    #[arg(long = "region", value_name = "REGION", default_value = "us-west-2")]
    pub region: String,

    /// EC2 key pair name to attach
    #[arg(long = "key-name", value_name = "KEY")]
    pub key_name: Option<String>,

    /// Security group ID to attach
    #[arg(
        long = "sg",
        value_name = "SG_ID",
        default_value = "sg-0ac5ceb1761087d04"
    )]
    pub security_group_id: String,

    /// IAM instance profile name
    #[arg(
        long = "iam-instance-profile",
        value_name = "NAME",
        default_value = "s3-readonly-with-ssm"
    )]
    pub iam_instance_profile_name: String,

    /// Name prefix for the instance Name tag
    #[arg(long = "name-prefix", value_name = "STR", default_value = "fw")]
    pub name_prefix: String,

    /// Custom tag to identify this instance (e.g., "pathdb-test", "pr-123")
    #[arg(long = "tag", value_name = "TAG")]
    pub custom_tag: Option<String>,

    /// Follow cloud-init stage progress via SSM after launch
    #[arg(long = "follow")]
    pub follow_logs: bool,

    /// Monitor bootstrap re-execution progress from `/var/log/bootstrap.log` (requires `--follow`)
    #[arg(long = "observe", requires = "follow_logs")]
    pub observe: bool,

    /// Don't launch
    #[arg(long = "dry-run")]
    pub dry_run: bool,
}

impl DeployOptions {
    #[must_use]
    pub fn branches(&self) -> [(&str, Option<&str>); 3] {
        [
            ("firewood", self.firewood_branch.as_deref()),
            ("avalanchego", self.avalanchego_branch.as_deref()),
            ("libevm", self.libevm_branch.as_deref()),
        ]
    }

    #[must_use]
    pub const fn end_block(&self) -> u64 {
        self.nblocks.end_block()
    }

    #[must_use]
    pub fn scenario_name(&self) -> &str {
        &self.scenario
    }
}

#[derive(Debug, Args)]
pub struct MonitorOptions {
    /// Instance ID to monitor
    #[arg(value_name = "INSTANCE_ID")]
    pub instance_id: String,

    /// AWS region
    #[arg(long = "region", value_name = "REGION", default_value = "us-west-2")]
    pub region: String,

    /// Monitor bootstrap re-execution progress from `/var/log/bootstrap.log`
    #[arg(long = "observe")]
    pub observe: bool,
}

#[derive(Debug, Args)]
pub struct ListOptions {
    /// AWS region
    #[arg(long = "region", value_name = "REGION", default_value = "us-west-2")]
    pub region: String,

    /// Show only running/pending instances
    #[arg(long = "running")]
    pub running_only: bool,

    /// Show only instances launched by your AWS identity
    #[arg(long = "mine")]
    pub mine_only: bool,
}

#[derive(Debug, Args)]
pub struct KillOptions {
    /// Instance ID to terminate
    #[arg(
        value_name = "INSTANCE_ID",
        conflicts_with_all = ["all", "mine"]
    )]
    pub instance_id: Option<String>,

    /// Terminate all `fwdctl` managed instances in this region
    #[arg(long = "all", conflicts_with = "mine")]
    pub all: bool,

    /// Terminate all instances launched by your AWS identity
    #[arg(long = "mine", conflicts_with = "all")]
    pub mine: bool,

    /// AWS region
    #[arg(long = "region", value_name = "REGION", default_value = "us-west-2")]
    pub region: String,

    /// Skip termination confirmation
    #[arg(long = "yes", short = 'y')]
    pub skip_confirm: bool,
}

pub(super) fn run(opts: &Options) -> Result<(), FwdError> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(run_internal(opts))
}

async fn run_internal(opts: &Options) -> Result<(), FwdError> {
    log::debug!("launch command {opts:?}");
    match &opts.command {
        LaunchCommand::Deploy(deploy) => run_deploy(deploy)
            .await
            .map_err(|e| FwdError::InternalError(Box::from(e))),
        LaunchCommand::Monitor(monitor) => run_monitor(monitor)
            .await
            .map_err(|e| FwdError::InternalError(Box::from(e))),
        LaunchCommand::List(list) => run_list(list)
            .await
            .map_err(|e| FwdError::InternalError(Box::from(e))),
        LaunchCommand::Kill(kill) => run_kill(kill)
            .await
            .map_err(|e| FwdError::InternalError(Box::from(e))),
    }
}

async fn run_deploy(opts: &DeployOptions) -> Result<(), LaunchError> {
    const EC2_USER_DATA_B64_LIMIT: usize = 25_600;

    log_launch_config(opts);

    let ctx = cloud_init::CloudInitContext::new(opts)?;

    if opts.dry_run {
        let c = ctx.render_yaml()?;
        println!("{}", c);
        return Ok(());
    }

    let user_data_b64 = ctx.render_base64()?;
    let user_data_size = user_data_b64.len();
    info!("Cloud-init user-data size: {user_data_size} bytes (base64)");
    if user_data_size > EC2_USER_DATA_B64_LIMIT {
        return Err(LaunchError::UserDataTooLarge {
            actual: user_data_size,
            limit: EC2_USER_DATA_B64_LIMIT,
        });
    }

    let ec2 = ec2_util::ec2_client(&opts.region).await;
    let ssm = ssm_monitor::ssm_client(&opts.region).await;

    let ami_id = ec2_util::latest_ubuntu_ami(&ec2, &opts.instance_type).await?;
    info!("Using AMI: {ami_id}");

    let instance_id = ec2_util::launch_instance(&ec2, &ami_id, opts, &user_data_b64).await?;

    ec2_util::wait_for_running(&ec2, &instance_id).await?;

    let (public_ip, private_ip) = ec2_util::describe_ips(&ec2, &instance_id).await?;

    info!("=== Instance Launched ===");
    info!("Instance ID: {instance_id}");
    if let Some(ip) = &public_ip {
        info!("Public IP:   {ip}");
    }
    if let Some(ip) = &private_ip {
        info!("Private IP:  {ip}");
    }
    info!("");
    info!("To monitor:  fwdctl launch monitor {instance_id}");
    info!("To list:     fwdctl launch list");
    info!("To kill:     fwdctl launch kill {instance_id} --yes");

    if opts.follow_logs {
        ssm_monitor::wait_for_ssm_registration(&ssm, &instance_id).await?;
        info!("");
        info!("Following cloud-init stage progress via SSM...");
        ssm_monitor::stream_logs_via_ssm(&ssm, &instance_id, opts.observe).await?;
    }

    Ok(())
}

async fn run_monitor(opts: &MonitorOptions) -> Result<(), LaunchError> {
    let ssm = ssm_monitor::ssm_client(&opts.region).await;

    info!("Monitoring instance: {}", opts.instance_id);
    if opts.observe {
        info!("Observe mode: tracking bootstrap re-execution after cloud-init stages.");
    }
    info!("");

    ssm_monitor::wait_for_ssm_registration(&ssm, &opts.instance_id).await?;
    ssm_monitor::stream_logs_via_ssm(&ssm, &opts.instance_id, opts.observe).await
}

async fn run_list(opts: &ListOptions) -> Result<(), LaunchError> {
    let ec2 = ec2_util::ec2_client(&opts.region).await;
    let me = ec2_util::get_aws_username().await;

    let mut instances = ec2_util::list_instances(&ec2, opts.running_only).await?;
    if opts.mine_only {
        instances.retain(|instance| instance.launched_by.as_deref() == Some(me.as_str()));
    }

    if instances.is_empty() {
        if opts.mine_only {
            info!("No instances launched by '{me}' were found.");
        } else {
            info!("No fwdctl-managed instances found.");
        }
        return Ok(());
    }

    info!(
        "{:<20} {:<11} {:<11} {:<12} {:<16} {:<12} NAME",
        "INSTANCE_ID", "USER", "STATE", "TYPE", "IP", "TAG"
    );
    info!("{}", "-".repeat(110));

    for instance in &instances {
        let user = instance.launched_by.as_deref().unwrap_or("-");
        let user_with_marker = if user == me.as_str() {
            format!("{user}*")
        } else {
            user.to_owned()
        };
        let ip = instance
            .public_ip
            .as_deref()
            .or(instance.private_ip.as_deref())
            .unwrap_or("-");
        info!(
            "{:<20} {:<11} {:<11} {:<12} {:<16} {:<12} {}",
            instance.instance_id,
            user_with_marker,
            instance.state,
            instance.instance_type,
            ip,
            instance.custom_tag.as_deref().unwrap_or("-"),
            instance.name
        );
    }

    let my_count = instances
        .iter()
        .filter(|instance| instance.launched_by.as_deref() == Some(me.as_str()))
        .count();
    info!("");
    if opts.mine_only {
        info!("Total: {} instance(s)", instances.len());
    } else {
        info!(
            "Total: {} instance(s), {} yours (*)",
            instances.len(),
            my_count
        );
    }
    Ok(())
}

async fn run_kill(opts: &KillOptions) -> Result<(), LaunchError> {
    let ec2 = ec2_util::ec2_client(&opts.region).await;
    let me = ec2_util::get_aws_username().await;
    let instances = ec2_util::list_instances(&ec2, false).await?;

    let to_kill = if opts.all {
        instances
            .iter()
            .map(|instance| instance.instance_id.clone())
            .collect()
    } else if opts.mine {
        instances
            .iter()
            .filter(|instance| instance.launched_by.as_deref() == Some(me.as_str()))
            .map(|instance| instance.instance_id.clone())
            .collect()
    } else if let Some(instance_id) = &opts.instance_id {
        if instances
            .iter()
            .any(|instance| instance.instance_id == *instance_id)
        {
            vec![instance_id.clone()]
        } else {
            return Err(LaunchError::Validation(format!(
                "Instance '{instance_id}' is not managed by fwdctl in region '{}'",
                opts.region
            )));
        }
    } else {
        info!("Specify an instance ID, `--mine`, or `--all`.");
        info!("Use `fwdctl launch list` to discover active instances.");
        return Ok(());
    };

    if to_kill.is_empty() {
        info!("No matching instances to terminate.");
        return Ok(());
    }

    if !opts.skip_confirm {
        info!("Will terminate {} instance(s):", to_kill.len());
        for instance_id in &to_kill {
            info!("  - {instance_id}");
        }
        return Err(LaunchError::Validation(
            "Confirmation required. Re-run with `--yes` (`-y`).".to_owned(),
        ));
    }

    info!("Terminating {} instance(s)...", to_kill.len());
    ec2_util::terminate_instances(&ec2, &to_kill).await?;
    for instance_id in &to_kill {
        info!("  Terminated: {instance_id}");
    }
    Ok(())
}

fn log_launch_config(opts: &DeployOptions) {
    info!("Launch configuration:");
    info!("\t{:24}{}", "Instance Type:", opts.instance_type);
    for (label, value) in opts.branches() {
        info!(
            "\t{:24}{}",
            format!("{label} branch:"),
            value.unwrap_or("default")
        );
    }
    info!("\t{:24}{}", "Scenario:", opts.scenario_name());
    info!("\t{:24}{}", "Blocks:", opts.nblocks.as_str());
    info!("\t{:24}{}", "Config:", opts.config);
    info!("\t{:24}{}", "Metrics Server:", opts.metrics_server);
    info!("\t{:24}{}", "Region:", opts.region);
    info!("\t{:24}{}", "Follow logs:", opts.follow_logs);
    info!("\t{:24}{}", "Observe progress:", opts.observe);
}
