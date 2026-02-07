// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::time::Duration;

use aws_config::BehaviorVersion;
use aws_sdk_ec2::Client as Ec2Client;
use aws_sdk_ec2::types::{
    ArchitectureType, BlockDeviceMapping, EbsBlockDevice, Filter, IamInstanceProfileSpecification,
    InstanceType as Ec2InstanceType, ResourceType, Tag, TagSpecification, VolumeType,
};
use aws_sdk_sts::Client as StsClient;
use log::info;
use tokio::sync::OnceCell;
use tokio::time::{sleep, timeout};

use super::{DeployOptions, LaunchError};

const WAIT_RUNNING_TIMEOUT: Duration = Duration::from_secs(300);
const POLL_INTERVAL_RUNNING: Duration = Duration::from_secs(3);
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
    let mut name = format!("{}-fw-{:08X}", opts.name_prefix, rand::random::<u32>());
    for (prefix, (_, branch)) in ["-fw-", "-ag-", "-ce-", "-le-"]
        .into_iter()
        .zip(opts.branches())
    {
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
    for (tag_name, (_, branch)) in [
        "FirewoodBranch",
        "AvalancheGoBranch",
        "CorethBranch",
        "LibEVMBranch",
    ]
    .into_iter()
    .zip(opts.branches())
    {
        if let Some(b) = branch {
            tags.push(tag(tag_name, b));
        }
    }
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

/// Returns the AWS username for the current caller identity.
///
/// Uses STS `GetCallerIdentity` (a global service, region-independent).
/// The result is cached for the lifetime of the process.
async fn get_aws_username() -> String {
    AWS_USERNAME
        .get_or_init(|| async {
            // STS GetCallerIdentity is region-independent; use default config.
            let sts = StsClient::new(&aws_config(None).await);
            sts.get_caller_identity()
                .send()
                .await
                .ok()
                .and_then(|id| {
                    id.arn().map(|arn| {
                        arn.rsplit('/')
                            .next()
                            .or_else(|| arn.rsplit(':').next())
                            .unwrap_or("unknown")
                            .into()
                    })
                })
                .unwrap_or_else(|| {
                    std::env::var("USER")
                        .or_else(|_| std::env::var("USERNAME"))
                        .unwrap_or_else(|_| "unknown".into())
                })
        })
        .await
        .clone()
}
