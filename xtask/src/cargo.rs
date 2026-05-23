use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};

use anyhow::Context;
use cargo_metadata::{Metadata, MetadataCommand};

/// Returns the path to the `cargo` binary, respecting the `CARGO` environment variable.
pub fn cargo_bin() -> PathBuf {
    std::env::var_os("CARGO").map_or_else(|| PathBuf::from("cargo"), PathBuf::from)
}

/// Runs a cargo subcommand, inheriting stdout/stderr. Returns the exit status.
#[expect(unused, reason = "next PR will use this function")]
pub fn run_cargo<I, S>(args: I) -> anyhow::Result<ExitStatus>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let cargo = cargo_bin();
    let mut cmd = Command::new(&cargo);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    cmd.status()
        .with_context(|| format!("exec: `{}` failed", cargo.display()))
}

/// Loads workspace cargo metadata with `--locked`.
#[expect(unused, reason = "next PR will use this function")]
pub fn load_metadata() -> anyhow::Result<Metadata> {
    MetadataCommand::new()
        .cargo_path(cargo_bin())
        .other_options(["--locked".to_owned()])
        .exec()
        .context("failed to load cargo metadata")
}
