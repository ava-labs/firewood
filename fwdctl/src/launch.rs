// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::{collections::HashMap, sync::OnceLock};

use aws_config::BehaviorVersion;
use aws_sdk_ec2::{
    Client,
    // types::{Tag, TagSpecification},
};
use clap::Args;
use firewood::v2::api;
use log::info;
use saphyr::LoadableYamlNode as _;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct InstanceType {
    // Instance information: architecture:spot:swap:disk:vcpu:memory:notes
    arch: Architecture,
    spot: f64,
    swap: u64,
    disk: u64,
    vcpu: u64,
    memory: u64,
    notes: String,
}

#[derive(Debug, Deserialize)]
struct InstanceTypes(HashMap<String, InstanceType>);

static INSTANCE_DATA: &str = include_str!("launch-types.json");

fn instance_types() -> &'static InstanceTypes {
    static ITEM: OnceLock<InstanceTypes> = OnceLock::new();

    ITEM.get_or_init(|| {
        serde_json::from_str(INSTANCE_DATA).expect("Failed to parse launch-types.json")
    })
}

/// Helper function to get branch name or "default"
fn branchname(branch: Option<&String>) -> &str {
    branch.map_or("default", |s| s.as_str())
}

/// Create an authenticated AWS EC2 client
///
/// This function loads AWS configuration using the standard credential chain:
/// 1. Environment variables (`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, etc.)
/// 2. AWS SSO credentials (if `aws sso login` has been run)
/// 3. AWS credentials file (~/.aws/credentials)
/// 4. IAM instance profile (if running on EC2)
/// 5. ECS task role (if running on ECS)
async fn create_ec2_client(region: Option<&str>) -> Result<Client, Box<dyn std::error::Error>> {
    let mut config_loader = aws_config::defaults(BehaviorVersion::latest());

    if let Some(region) = region {
        config_loader = config_loader.region(aws_config::Region::new(region.to_string()));
    }

    let config = config_loader.load().await;
    let client = Client::new(&config);

    Ok(client)
}

#[derive(Debug, Deserialize)]
enum Architecture {
    Arm64,
    Amd64,
}

#[derive(Debug, Args)]
pub struct Options {
    /// EC2 instance type
    #[arg(
        long = "instance-type",
        value_name = "TYPE",
        default_value = "i4g.large",
        help = "EC2 instance type"
    )]
    pub instance_type: String,

    /// `Firewood` git branch to checkout
    #[arg(
        long = "firewood-branch",
        value_name = "BRANCH",
        help = "Firewood git branch to checkout"
    )]
    pub firewood_branch: Option<String>,

    /// `AvalancheGo` git branch to checkout
    #[arg(
        long = "avalanchego-branch",
        value_name = "BRANCH",
        help = "AvalancheGo git branch to checkout"
    )]
    pub avalanchego_branch: Option<String>,

    /// `Coreth` git branch to checkout
    #[arg(
        long = "coreth-branch",
        value_name = "BRANCH",
        help = "Coreth git branch to checkout"
    )]
    pub coreth_branch: Option<String>,

    /// `LibEVM` git branch to checkout
    #[arg(
        long = "libevm-branch",
        value_name = "BRANCH",
        help = "LibEVM git branch to checkout"
    )]
    pub libevm_branch: Option<String>,

    /// Number of blocks to download
    #[arg(
        long = "nblocks",
        value_name = "BLOCKS",
        default_value = "1m",
        help = "Number of blocks to download",
        value_parser = ["1m", "10m", "50m"]
    )]
    pub nblocks: String,

    /// AWS region
    #[arg(
        long = "region",
        value_name = "REGION",
        default_value = "us-west-2",
        help = "AWS region"
    )]
    pub region: String,

    /// Use spot instance pricing
    #[arg(
        long = "spot",
        help = "Use spot instance pricing (default depends on instance type)"
    )]
    pub spot: bool,

    /// Show the aws command that would be run without executing it
    #[arg(
        long = "dry-run",
        help = "Show the aws command that would be run without executing it"
    )]
    pub dry_run: bool,
}

pub(super) fn run(opts: &Options) -> Result<(), api::Error> {
    log::debug!("launch AWS instance {opts:?}");

    // Use tokio to run the async function
    let rt = tokio::runtime::Runtime::new().map_err(|e| {
        api::Error::InternalError(format!("Failed to create tokio runtime: {e}").into())
    })?;

    rt.block_on(run_async(opts))
        .map_err(|e| api::Error::InternalError(format!("Launch failed: {e}").into()))
}

async fn run_async(opts: &Options) -> Result<(), Box<dyn std::error::Error>> {
    info!("Launch command called with options:");
    info!("  Instance Type: {}", opts.instance_type);
    info!(
        "  Firewood Branch: {}",
        branchname(opts.firewood_branch.as_ref())
    );
    info!(
        "  AvalancheGo Branch: {}",
        branchname(opts.avalanchego_branch.as_ref())
    );
    info!(
        "  Coreth Branch: {}",
        branchname(opts.coreth_branch.as_ref())
    );
    info!(
        "  LibEVM Branch: {}",
        branchname(opts.libevm_branch.as_ref())
    );
    info!("  Number of Blocks: {}", opts.nblocks);
    info!("  Region: {}", opts.region);
    info!("  Spot Instance: {}", opts.spot);
    info!("  Dry Run: {}", opts.dry_run);

    // Validate instance type
    let instance_info = instance_types();
    if let Some(instance) = instance_info.0.get(&opts.instance_type) {
        info!("📋 Instance specifications:");
        info!("  Architecture: {:?}", instance.arch);
        info!("  Spot Price: ${:.4}/hr", instance.spot);
        info!("  Swap: {}GB", instance.swap);
        info!("  Disk: {}GB", instance.disk);
        info!("  vCPU: {}", instance.vcpu);
        info!("  Memory: {}GB", instance.memory);
        info!("  Notes: {}", instance.notes);
    } else {
        return Err(format!("Invalid instance type: {}", opts.instance_type).into());
    }

    // TODO: this should not be part of the source, instead it should read it from
    // a file in some config directory for this tool
    let mut yamls = saphyr::Yaml::load_from_str(include_str!("cloud-init.yaml"))?;
    let /*mut*/ cloud_init = yamls.first_mut().expect("should at least have one yaml");
    // TODO: mutate the cloud_init file to insert the swap config
    // maybe something like this
    // for mut seq in cloud_init.as_mapping_mut().expect("not a mapping?") {
    //     if let (k, saphyr::Yaml::Tagged(tag, value)) = seq {
    //         // TODO: use the correct mapping
    //         seq = (&saphyr::Yaml::Mapping(Mapping::new()), &mut saphyr::Yaml::Mapping(Mapping::new()));
    //     }
    // }

    let mut yaml = String::new();
    let mut emitter = saphyr::YamlEmitter::new(&mut yaml);
    emitter.dump(cloud_init)?;
    // this needs to get sent to the launcher
    println!("{yaml}");

    if opts.dry_run {
        info!("🏃 Dry run mode - would launch instance here");
    } else {
        let _client = create_ec2_client(Some(&opts.region)).await?;
        // TODO: base64 encode the user data
        /* client
        .run_instances()
        //.region(opts.region)
        //.image_id(image_id)
        .instance_type(InstanceType::builder().instance_type(opts.instance_type).build())
        .key_name("rkuris")
        .security_groups(vec!["rkuris-starlink-only".to_string()])
        .iam_instance_profile(Some("Name=s3-readonly"))
        //.user_data(user_data)
        .tag_specifications(vec![
            TagSpecification::builder()
                .resource_type("instance")
                .tags(
                    Tag::builder().key("Name").value(instance_name).build(),
                )
                .build(),
        ])
        .block_device_mappings(vec![
            BlockDeviceMapping::builder()
                .device_name("/dev/sda1")
                .ebs(
                    EbsBlockDevice::builder()
                        .volume_size(50)
                        .volume_type("gp3")
                        .build(),
                )
                .build(),
        ])
        .send()
        .await?; */
        info!("🚀 Would launch instance here (not implemented yet)");
    }

    Ok(())
}
