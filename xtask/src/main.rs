#![allow(clippy::missing_errors_doc, reason = "non-public api")]

use clap::Parser;

mod cargo;
pub mod cmd;

#[derive(Debug, Parser)]
#[command(name = "xtask", about = "Firewood development task runner")]
struct Cli {
    #[command(subcommand)]
    command: cmd::Command,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    cli.command.run()
}
