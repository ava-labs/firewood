// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Stage configuration loader and template processor.
//!
//! Supports:
//! - Shared stage definitions referenced by ID
//! - Scenarios that compose stages
//! - Variable interpolation: `{{ namespace.key }}`

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Failed to read config file: {0}")]
    Io(#[from] std::io::Error),

    #[error("Failed to parse YAML: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("Stage '{0}' not found in stages")]
    StageNotFound(String),

    #[error("Scenario '{0}' not found in configuration")]
    ScenarioNotFound(String),

    #[error("Unknown variable: {0}")]
    UnknownVariable(String),
}

/// Root configuration structure.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StageConfig {
    #[serde(default)]
    pub packages: Vec<String>,
    #[serde(default)]
    pub users: Vec<UserDefinition>,
    #[serde(default)]
    pub variables: HashMap<String, String>,
    #[serde(default)]
    pub stages: HashMap<String, SharedStage>,
    pub scenarios: HashMap<String, ScenarioDefinition>,
}

pub const DEFAULT_SCENARIO: &str = "default";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SharedStage {
    pub name: String,
    #[serde(default)]
    pub enabled: EnabledValue,
    #[serde(default)]
    pub variables: HashMap<String, String>,
    pub commands: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ScenarioDefinition {
    #[serde(default)]
    pub description: String,
    pub stages: Vec<StageRef>,
}

/// Reference to a stage in a scenario.
///
/// Can be:
/// - A simple string ID: `- rust-install`
/// - A reference with variable overrides: `- id: rust-install\n    variables: {key: value}`
/// - A fully inline stage definition with `name` and `commands`
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum StageRef {
    /// Simple reference by ID
    Ref(String),
    /// Reference with overrides, or fully inline stage
    Override(StageOverride),
}

/// A stage reference with optional overrides, or a fully inline stage.
///
/// If `name` and `commands` are provided, this is a fully inline stage.
/// Otherwise, `id` must reference a shared stage, and `variables` are merged.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StageOverride {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub enabled: Option<EnabledValue>,
    #[serde(default)]
    pub variables: HashMap<String, String>,
    #[serde(default)]
    pub commands: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UserDefinition {
    pub name: String,
    #[serde(default = "default_groups")]
    pub groups: String,
    #[serde(default = "default_shell")]
    pub shell: String,
    #[serde(default = "default_sudo")]
    pub sudo: String,
    #[serde(default)]
    pub ssh_authorized_keys: Vec<String>,
    #[serde(default = "default_lock_passwd")]
    pub lock_passwd: bool,
}

fn default_groups() -> String {
    "users, adm, sudo".into()
}
fn default_shell() -> String {
    "/usr/bin/bash".into()
}
fn default_sudo() -> String {
    "ALL=(ALL) NOPASSWD:ALL".into()
}
fn default_lock_passwd() -> bool {
    true
}

/// A fully resolved stage ready for processing.
#[derive(Debug, Clone)]
pub struct ResolvedStage {
    pub id: String,
    pub name: String,
    pub enabled: EnabledValue,
    pub variables: HashMap<String, String>,
    pub commands: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum EnabledValue {
    Bool(bool),
    Template(String),
}

impl Default for EnabledValue {
    fn default() -> Self {
        Self::Bool(true)
    }
}

/// Template context for variable interpolation.
///
/// Variables are resolved in this order:
/// 1. `variables.*` - from config file and stage-specific variables
/// 2. `args.*` - from CLI arguments
/// 3. `branches.*` - git branch overrides
#[derive(Debug, Clone, Default)]
pub struct TemplateContext {
    pub variables: HashMap<String, String>,
    pub args: HashMap<String, String>,
    pub branches: HashMap<String, String>,
}

const DEFAULT_CONFIG: &str = include_str!("../../../benchmark/launch/launch-stages.yaml");

impl StageConfig {
    pub fn load() -> Result<Self, ConfigError> {
        if let Some(user_path) = user_config_path() {
            if user_path.exists() {
                log::info!("Loading stage config from: {}", user_path.display());
                let content = std::fs::read_to_string(&user_path)?;
                return serde_yaml::from_str(&content).map_err(ConfigError::from);
            }
        }
        log::debug!("Using embedded default stage config");
        serde_yaml::from_str(DEFAULT_CONFIG).map_err(ConfigError::from)
    }

    fn resolve_stage_ref(&self, stage_ref: &StageRef) -> Result<ResolvedStage, ConfigError> {
        match stage_ref {
            StageRef::Ref(id) => {
                let shared = self
                    .stages
                    .get(id)
                    .ok_or_else(|| ConfigError::StageNotFound(id.clone()))?;
                Ok(ResolvedStage {
                    id: id.clone(),
                    name: shared.name.clone(),
                    enabled: shared.enabled.clone(),
                    variables: shared.variables.clone(),
                    commands: shared.commands.clone(),
                })
            }
            StageRef::Override(ovr) => {
                // Check if this references a shared stage or is fully inline
                if let Some(shared) = self.stages.get(&ovr.id) {
                    // Merge overrides with shared stage
                    let mut variables = shared.variables.clone();
                    variables.extend(ovr.variables.clone());

                    Ok(ResolvedStage {
                        id: ovr.id.clone(),
                        name: ovr.name.clone().unwrap_or_else(|| shared.name.clone()),
                        enabled: ovr
                            .enabled
                            .clone()
                            .unwrap_or_else(|| shared.enabled.clone()),
                        variables,
                        commands: ovr
                            .commands
                            .clone()
                            .unwrap_or_else(|| shared.commands.clone()),
                    })
                } else {
                    // Fully inline stage - must have name and commands
                    let name = ovr
                        .name
                        .clone()
                        .ok_or_else(|| ConfigError::StageNotFound(ovr.id.clone()))?;
                    let commands = ovr
                        .commands
                        .clone()
                        .ok_or_else(|| ConfigError::StageNotFound(ovr.id.clone()))?;

                    Ok(ResolvedStage {
                        id: ovr.id.clone(),
                        name,
                        enabled: ovr.enabled.clone().unwrap_or_default(),
                        variables: ovr.variables.clone(),
                        commands,
                    })
                }
            }
        }
    }

    fn get_scenario_stages(&self, scenario_name: &str) -> Result<Vec<ResolvedStage>, ConfigError> {
        let scenario = self
            .scenarios
            .get(scenario_name)
            .ok_or_else(|| ConfigError::ScenarioNotFound(scenario_name.to_string()))?;

        scenario
            .stages
            .iter()
            .map(|r| self.resolve_stage_ref(r))
            .collect()
    }

    pub fn process(
        &self,
        ctx: &TemplateContext,
        scenario_name: &str,
    ) -> Result<Vec<ProcessedStage>, ConfigError> {
        let stages = self.get_scenario_stages(scenario_name)?;
        let mut result = Vec::new();

        // Start with config-level variables, then overlay context variables
        let mut base_vars = self.variables.clone();
        base_vars.extend(ctx.variables.clone());

        for stage in stages {
            // Merge stage-specific variables on top
            let mut stage_vars = base_vars.clone();
            stage_vars.extend(stage.variables.clone());

            let stage_ctx = TemplateContext {
                variables: stage_vars,
                args: ctx.args.clone(),
                branches: ctx.branches.clone(),
            };

            if !is_stage_enabled(&stage.enabled, &stage_ctx) {
                log::debug!("Skipping disabled stage: {}", stage.id);
                continue;
            }

            let commands: Vec<String> = stage
                .commands
                .iter()
                .map(|cmd| process_template(cmd, &stage_ctx))
                .collect::<Result<Vec<_>, _>>()?;

            result.push(ProcessedStage {
                id: stage.id.clone(),
                name: process_template(&stage.name, &stage_ctx)?,
                commands,
            });
        }

        Ok(result)
    }
}

#[derive(Debug, Clone)]
pub struct ProcessedStage {
    pub id: String,
    pub name: String,
    pub commands: Vec<String>,
}

fn is_stage_enabled(enabled: &EnabledValue, ctx: &TemplateContext) -> bool {
    match enabled {
        EnabledValue::Bool(b) => *b,
        EnabledValue::Template(template) => {
            let result = process_template(template, ctx).unwrap_or_default();
            let trimmed = result.trim();
            !trimmed.is_empty() && trimmed != "false" && trimmed != "null" && trimmed != "{{}}"
        }
    }
}

fn process_template(template: &str, ctx: &TemplateContext) -> Result<String, ConfigError> {
    let mut result = template.to_string();
    while let (Some(s), Some(e)) = (result.find("{{"), result.find("}}")) {
        if s >= e {
            break;
        }
        let val = resolve_var(result[s + 2..e].trim(), ctx)?;
        result = format!("{}{}{}", &result[..s], val, &result[e + 2..]);
    }
    Ok(result)
}

fn resolve_var(path: &str, ctx: &TemplateContext) -> Result<String, ConfigError> {
    let err = || ConfigError::UnknownVariable(path.into());
    let (ns, key) = path.split_once('.').ok_or_else(err)?;
    match ns {
        "variables" => ctx.variables.get(key).cloned().ok_or_else(err),
        "branches" => ctx.branches.get(key).cloned().ok_or_else(err),
        "args" => ctx.args.get(key).cloned().ok_or_else(err),
        _ => Err(err()),
    }
}

fn user_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|p| p.join("fwdctl").join("launch-stages.yaml"))
}

