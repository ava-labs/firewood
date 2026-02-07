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

pub const DEFAULT_SCENARIO: &str = "reexecute";

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
    pub groups: String,
    pub shell: String,
    pub sudo: String,
    pub ssh_authorized_keys: Vec<String>,
    pub lock_passwd: bool,
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
    /// # Errors
    ///
    /// Returns [`ConfigError`] if the config file cannot be read or parsed.
    pub fn load() -> Result<Self, ConfigError> {
        if let Some(user_path) = user_config_path()
            && user_path.exists()
        {
            log::info!("Loading stage config from: {}", user_path.display());
            let content = std::fs::read_to_string(&user_path)?;
            return serde_yaml::from_str(&content).map_err(ConfigError::from);
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

    /// # Errors
    ///
    /// Returns [`ConfigError`] if the scenario is not found, a referenced stage
    /// is missing, or a template variable cannot be resolved.
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

        // Reuse a single mutable context to avoid cloning args/branches per stage
        let mut stage_ctx = TemplateContext {
            variables: HashMap::new(),
            args: ctx.args.clone(),
            branches: ctx.branches.clone(),
        };

        for stage in stages {
            // Reset to base vars + stage vars
            stage_ctx.variables.clone_from(&base_vars);
            stage_ctx.variables.extend(stage.variables);

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

/// Processes `{{ namespace.key }}` placeholders in a template string.
fn process_template(template: &str, ctx: &TemplateContext) -> Result<String, ConfigError> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;

    while let Some((before, after_open)) = rest.split_once("{{") {
        out.push_str(before);
        let Some((expr, after_close)) = after_open.split_once("}}") else {
            // No closing braces: keep the remainder unchanged
            out.push_str("{{");
            out.push_str(after_open);
            return Ok(out);
        };
        let val = resolve_var(expr.trim(), ctx)?;
        out.push_str(&val);
        rest = after_close;
    }

    out.push_str(rest);
    Ok(out)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_config_parses() {
        let config: StageConfig =
            serde_yaml::from_str(DEFAULT_CONFIG).expect("embedded config should parse");
        assert!(
            config.scenarios.contains_key("reexecute"),
            "should contain 'reexecute' scenario"
        );
        assert!(
            !config.stages.is_empty(),
            "should contain shared stage definitions"
        );
    }

    #[test]
    fn template_interpolation() {
        let ctx = TemplateContext {
            variables: HashMap::from([("name".into(), "world".into())]),
            args: HashMap::from([("count".into(), "42".into())]),
            branches: HashMap::new(),
        };
        let result = process_template("hello {{ variables.name }} ({{ args.count }})", &ctx);
        assert_eq!(result.unwrap(), "hello world (42)");
        let result = process_template("{{ variables.missing }}", &ctx);
        assert!(result.is_err());
    }
}
