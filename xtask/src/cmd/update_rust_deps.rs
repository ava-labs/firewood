use std::process::ExitStatus;

use anyhow::Context;
use clap::Args;

use crate::cargo::run_cargo;

#[derive(Debug, Args)]
pub struct Command {
    /// Additional crate names to exclude from the upgrade (in addition to those
    /// automatically excluded because they appear in [patch.crates-io]).
    #[arg(long, value_delimiter = ',', value_name = "CRATE")]
    exclude: Vec<String>,
}

impl Command {
    pub fn run(&self) -> anyhow::Result<()> {
        let mut excluded = patched_crates()?;
        excluded.extend_from_slice(&self.exclude);

        if !excluded.is_empty() {
            eprintln!("Excluding from upgrade: {}", excluded.join(", "));
        }

        let exclude_args: Vec<String> = excluded
            .iter()
            .flat_map(|name| ["--exclude".to_owned(), name.clone()])
            .collect();

        eprintln!("Upgrading compatible Cargo dependencies...");
        let mut upgrade = vec!["upgrade".to_owned()];
        upgrade.extend_from_slice(&exclude_args);
        check_status(run_cargo(&upgrade)?, "cargo upgrade")?;

        eprintln!("Upgrading incompatible Cargo dependencies...");
        eprintln!(
            "NOTE: If this fails, re-run with --exclude <dependency-name> and open a GitHub issue."
        );
        upgrade.push("--incompatible".to_owned());
        check_status(run_cargo(&upgrade)?, "cargo upgrade --incompatible")?;

        eprintln!("Updating Cargo.lock...");
        check_status(run_cargo(["update", "--verbose"])?, "cargo update")?;

        eprintln!("Running tests to verify upgrades...");
        check_status(
            run_cargo(["test", "--workspace", "--all-targets", "--features", "logger"])?,
            "tests with --features logger",
        )?;
        check_status(
            run_cargo([
                "test",
                "--workspace",
                "--all-targets",
                "--features",
                "ethhash,logger",
            ])?,
            "tests with --features ethhash,logger",
        )?;

        Ok(())
    }
}

fn check_status(status: ExitStatus, label: &str) -> anyhow::Result<()> {
    if status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("{label} failed with exit status: {status}"))
    }
}

fn patched_crates() -> anyhow::Result<Vec<String>> {
    let content =
        std::fs::read_to_string("Cargo.toml").context("failed to read Cargo.toml")?;
    let doc: toml::Value =
        toml::from_str(&content).context("failed to parse Cargo.toml")?;

    Ok(doc
        .get("patch")
        .and_then(|p| p.get("crates-io"))
        .and_then(|ci| ci.as_table())
        .map(|t| t.keys().cloned().collect())
        .unwrap_or_default())
}
