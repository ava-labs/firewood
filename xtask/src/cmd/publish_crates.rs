use clap::Args;

#[derive(Debug, Args)]
pub struct Command {}

impl Command {
    pub fn run(&self) -> anyhow::Result<()> {
        todo!("publish-crates not yet implemented")
    }
}
