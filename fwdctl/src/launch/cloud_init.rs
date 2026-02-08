// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::Serialize;
use serde_json::json;

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
        let total = stages.len();
        let mut runcmd = Vec::with_capacity(total * 4 + 2);

        let stage_names: Vec<_> = stages.iter().map(|stage| stage.name.as_str()).collect();
        let command_map: HashMap<String, Vec<&str>> = stages
            .iter()
            .enumerate()
            .map(|(index, stage)| {
                (
                    (index + 1).to_string(),
                    stage.commands.iter().map(String::as_str).collect(),
                )
            })
            .collect();

        let state = json!({
            "step": 0,
            "total": total,
            "name": "",
            "status": "pending",
            "stages": stage_names,
            "commands": command_map,
            "last_error": serde_json::Value::Null,
        });
        runcmd.push(write_json_file_command(STATE_FILE, &state)?);

        for (stage_idx, stage) in stages.iter().enumerate() {
            let step = stage_idx + 1;
            runcmd.push(jq_update_state_command(
                step,
                total,
                &stage.name,
                "in_progress",
            ));

            for (cmd_idx, cmd) in stage.commands.iter().enumerate() {
                runcmd.push(cmd.clone());
                runcmd.push(jq_mark_failed_command(
                    step,
                    total,
                    &stage.name,
                    cmd_idx + 1,
                ));
            }

            runcmd.push(jq_update_state_command(
                step,
                total,
                &stage.name,
                "completed",
            ));
        }

        Ok(runcmd)
    }
}

fn write_json_file_command<T: Serialize>(path: &str, payload: &T) -> Result<String, LaunchError> {
    let value = serde_json::to_string(payload)?;
    Ok(format!(
        "printf '%s\\n' '{}' > {path}",
        shell_single_quote(&value)
    ))
}

fn jq_update_state_command(step: usize, total: usize, name: &str, status: &str) -> String {
    format!(
        "jq --argjson step {step} --argjson total {total} --arg name '{}' --arg status '{}' '.step=$step | .total=$total | .name=$name | .status=$status | .last_error=null' {STATE_FILE} > {STATE_FILE_TMP} && mv {STATE_FILE_TMP} {STATE_FILE}",
        shell_single_quote(name),
        shell_single_quote(status),
    )
}

fn jq_mark_failed_command(step: usize, total: usize, name: &str, cmd: usize) -> String {
    format!(
        "_ec=$?; if [ $_ec -ne 0 ]; then jq --argjson step {step} --argjson total {total} --arg name '{}' --argjson cmd {cmd} --argjson exit \"$_ec\" '.step=$step | .total=$total | .name=$name | .status=\"failed\" | .last_error={{\"stage\":$step,\"cmd\":$cmd,\"exit\":$exit}}' {STATE_FILE} > {STATE_FILE_TMP} && mv {STATE_FILE_TMP} {STATE_FILE}; exit $_ec; fi",
        shell_single_quote(name),
    )
}

fn shell_single_quote(value: &str) -> String {
    value.replace('\'', "'\\''")
}

pub const STATE_FILE: &str = "/var/log/cloud-init-state.json";
const STATE_FILE_TMP: &str = "/var/log/cloud-init-state.json.tmp";
