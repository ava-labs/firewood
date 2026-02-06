// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::Serialize;

use std::collections::HashMap;

use super::DeployOptions;
use super::stage_config::{DEFAULT_SCENARIO, StageConfig, TemplateContext};

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
    pub fn new(opts: &DeployOptions) -> Self {
        let config = StageConfig::load().unwrap_or_else(|e| {
            log::warn!("Failed to load stage config: {e}, using embedded default");
            serde_yaml::from_str(include_str!("../../../benchmark/launch/launch-stages.yaml"))
                .expect("invalid embedded config")
        });
        let template_ctx = TemplateContext {
            variables: config.variables.clone(),
            args: HashMap::from([
                (
                    "end_block".into(),
                    opts.end_block.unwrap_or(1000000).to_string(),
                ),
                ("config".into(), opts.config.clone()),
                ("metrics_server".into(), opts.metrics_server.clone()),
            ]),
            branches: opts
                .branches()
                .into_iter()
                .map(|(k, v)| {
                    (
                        k.into(),
                        v.map(|s| format!("--branch {s}")).unwrap_or_default(),
                    )
                })
                .collect(),
        };
        Self {
            swap_gib: 16,
            template_ctx,
            config,
        }
    }

    pub fn render_yaml(&self) -> Result<String, serde_yaml::Error> {
        let yaml = self.build_yaml()?;
        let mut output = String::from("#cloud-config\n");
        output.push_str(&serde_yaml::to_string(&yaml)?);
        Ok(output)
    }

    pub fn render_base64(&self) -> Result<String, serde_yaml::Error> {
        Ok(BASE64.encode(self.render_yaml()?.as_bytes()))
    }

    fn build_yaml(&self) -> Result<CloudInitYaml, serde_yaml::Error> {
        Ok(CloudInitYaml {
            package_update: true,
            package_upgrade: true,
            packages: self.config.packages.clone(),
            users: self.build_users()?,
            swap: self.build_swap(),
            write_files: self.build_write_files(),
            runcmd: self.build_runcmd(),
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

    fn build_runcmd(&self) -> Vec<String> {
        let stages = match self.config.process(&self.template_ctx, DEFAULT_SCENARIO) {
            Ok(s) => s,
            Err(e) => {
                log::error!("Failed to process stage config: {e}");
                return Vec::new();
            }
        };
        let cmds = stages
            .into_iter()
            .flat_map(|x| x.commands)
            .collect::<Vec<String>>();
        cmds
    }
}
