use clap::Args;

#[derive(Debug, Args)]
pub struct Command {}

impl Command {
    pub fn run(&self) -> anyhow::Result<()> {
        todo!("update-rust-dependencies not yet implemented")
    }
}
