use clap::Subcommand;

pub mod publish_crates;
pub mod update_rust_deps;

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Update the workspace Cargo dependencies, excluding patched crates.
    UpdateRustDependencies(update_rust_deps::Command),
    /// Publish all publishable workspace crates to crates.io.
    PublishCrates(publish_crates::Command),
}

impl Command {
    pub fn run(&self) -> anyhow::Result<()> {
        match self {
            Self::UpdateRustDependencies(cmd) => cmd.run(),
            Self::PublishCrates(cmd) => cmd.run(),
        }
    }
}
