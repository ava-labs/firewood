// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::Serialize;

use std::collections::HashMap;

use super::stage_config::{DEFAULT_SCENARIO, StageConfig, TemplateContext};
use super::{DeployOptions, LaunchError};

#[derive(Serialize)]
struct CloudInitYaml {
    package_update: bool,
    package_upgrade: bool,
    packages: Vec<String>,
    users: Vec<serde_yaml::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    swap: Option<SwapConfig>,
    write_files: Vec<WriteFile>,
    runcmd: Vec<String>,
}

#[derive(Serialize)]
struct SwapConfig {
    filename: &'static str,
    size: String,
    maxsize: String,
}

#[derive(Serialize, Clone)]
struct WriteFile {
    path: &'static str,
    permissions: &'static str,
    content: String,
}

pub struct CloudInitContext {
    swap_gib: u64,
    template_ctx: TemplateContext,
    config: StageConfig,
}

impl CloudInitContext {
    pub fn new(opts: &DeployOptions) -> Result<Self, LaunchError> {
        let config = match StageConfig::load() {
            Ok(c) => c,
            Err(e) => {
                log::warn!("Failed to load stage config: {e}, using embedded default");
                serde_yaml::from_str(include_str!("../../../benchmark/launch/launch-stages.yaml"))?
            }
        };
        let end_block = opts.end_block().to_string();
        let nblocks = opts.nblocks.as_str().to_owned();
        let mut variables = config.variables.clone();
        variables.insert("end_block".into(), end_block.clone());

        let template_ctx = TemplateContext {
            variables,
            args: HashMap::from([
                ("end_block".into(), end_block),
                ("nblocks".into(), nblocks),
                ("config".into(), opts.config.clone()),
                ("metrics_server".into(), opts.metrics_server.to_string()),
            ]),
            branches: opts
                .branches()
                .into_iter()
                .map(|(name, branch)| {
                    (
                        name.into(),
                        branch.map_or_else(String::new, |value| format!("--branch {value}")),
                    )
                })
                .collect(),
        };
        Ok(Self {
            swap_gib: 16,
            template_ctx,
            config,
        })
    }

    pub fn render_yaml(&self) -> Result<String, LaunchError> {
        let yaml = self.build_yaml()?;
        let mut output = String::from("#cloud-config\n");
        output.push_str(&serde_yaml::to_string(&yaml)?);
        Ok(output)
    }

    pub fn render_base64(&self) -> Result<String, LaunchError> {
        Ok(BASE64.encode(self.render_yaml()?.as_bytes()))
    }

    fn build_yaml(&self) -> Result<CloudInitYaml, LaunchError> {
        Ok(CloudInitYaml {
            package_update: true,
            package_upgrade: true,
            packages: self.config.packages.clone(),
            users: self.build_users()?,
            swap: self.build_swap(),
            write_files: self.build_write_files(),
            runcmd: self.build_runcmd()?,
        })
    }

    fn build_users(&self) -> Result<Vec<serde_yaml::Value>, serde_yaml::Error> {
        let mut users = vec![serde_yaml::Value::String("default".into())];
        for user in &self.config.users {
            users.push(serde_yaml::to_value(user)?);
        }
        Ok(users)
    }

    fn build_swap(&self) -> Option<SwapConfig> {
        (self.swap_gib > 0).then(|| SwapConfig {
            filename: "/swapfile",
            size: format!("{}G", self.swap_gib),
            maxsize: format!("{}G", self.swap_gib),
        })
    }

    fn build_write_files(&self) -> Vec<WriteFile> {
        vec![
            WriteFile {
                path: "/etc/sudoers.d/91-cloud-init-enable-D-option",
                permissions: "0440",
                content: "Defaults runcwd=*".into(),
            },
            WriteFile {
                path: "/etc/profile.d/go_path.sh",
                permissions: "0644",
                content: r#"export PATH="$PATH:/usr/local/go/bin""#.into(),
            },
            WriteFile {
                path: "/etc/profile.d/rust_path.sh",
                permissions: "0644",
                content: concat!(
                    "export RUSTUP_HOME=/usr/local/rust\n",
                    r#"export PATH="$PATH:/usr/local/rust/bin""#
                )
                .into(),
            },
        ]
    }

    fn build_runcmd(&self) -> Result<Vec<String>, LaunchError> {
        let stages = self.config.process(&self.template_ctx, DEFAULT_SCENARIO)?;
        Ok(stages.into_iter().flat_map(|x| x.commands).collect())
    }
}
