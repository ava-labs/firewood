// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

mod cloud_init;
mod ec2_util;
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
    Ec2(#[from] aws_sdk_ec2::Error),

    #[error("AWS SDK error: {0}")]
    AwsSdk(String),

    #[error("Cloud-init generation failed: {0}")]
    CloudInit(#[from] serde_yaml::Error),

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
}

impl<E, R> From<aws_smithy_runtime_api::client::result::SdkError<E, R>> for LaunchError
where
    E: std::error::Error + Send + Sync + 'static,
    R: std::fmt::Debug,
{
    fn from(e: aws_smithy_runtime_api::client::result::SdkError<E, R>) -> Self {
        log::debug!("AWS SDK error: {e:#?}");
        Self::AwsSdk(e.to_string())
    }
}

impl From<aws_sdk_ec2::error::BuildError> for LaunchError {
    fn from(e: aws_sdk_ec2::error::BuildError) -> Self {
        log::debug!("AWS build error: {e:#?}");
        Self::AwsSdk(e.to_string())
    }
}

#[derive(Debug, Args)]
pub struct Options {
    #[command(subcommand)]
    pub command: LaunchCommand,
}

#[derive(Debug, Subcommand)]
pub enum LaunchCommand {
    Deploy(DeployOptions),
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

    /// Security group IDs to attach (repeatable)
    #[arg(long = "sg", value_name = "SG_ID", num_args = 0..)]
    pub security_group_ids: Vec<String>,

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
}

pub(super) fn run(opts: &Options) -> Result<(), FwdError> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(run_internal(opts))
}

async fn run_internal(opts: &Options) -> Result<(), FwdError> {
    log::debug!("launch command {opts:?}");

    match &opts.command {
        LaunchCommand::Deploy(o) => run_deploy(o)
            .await
            .map_err(|e| FwdError::InternalError(Box::from(e))),
    }
}

async fn run_deploy(opts: &DeployOptions) -> Result<(), LaunchError> {
    log_launch_config(opts);

    let ctx = cloud_init::CloudInitContext::new(opts)?;

    let user_data_b64 = ctx.render_base64()?;

    let ec2 = ec2_util::ec2_client(&opts.region).await;

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

    Ok(())
}

fn log_launch_config(opts: &DeployOptions) {
    info!("Launch configuration:");
    info!("  Instance Type:     {}", opts.instance_type);
    info!(
        "  Firewood Branch:   {}",
        opts.firewood_branch.as_deref().unwrap_or("default")
    );
    info!(
        "  AvalancheGo:       {}",
        opts.avalanchego_branch.as_deref().unwrap_or("default")
    );
    info!(
        "  LibEVM:            {}",
        opts.libevm_branch.as_deref().unwrap_or("default")
    );
    info!("  Blocks:            {}", opts.nblocks.as_str());
    info!("  Config:            {}", opts.config);
    info!("  Metrics Server:    {}", opts.metrics_server);
    info!("  Region:            {}", opts.region);
}
