# xtask

Firewood's development task runner. Built as a Rust binary using the
[cargo xtask pattern](https://github.com/matklad/cargo-xtask).

## Usage

```shell
cargo xtask <subcommand> [options]
```

## Available subcommands

| Subcommand | Purpose |
| ---------- | ------- |
| `update-rust-dependencies` | Upgrade Cargo deps, excluding patched crates |
| `publish-crates` | Publish unpublished workspace crates to crates.io |

## Implementing a new subcommand

1. Create `xtask/src/cmd/<name>.rs` with a `Command` struct that derives
   `clap::Args` and implements `pub fn run(&self) -> anyhow::Result<()>`.

   ```rust
   use clap::Args;

   #[derive(Debug, Args)]
   pub struct Command {
       /// Example flag.
       #[arg(long)]
       dry_run: bool,
   }

   impl Command {
       pub fn run(&self) -> anyhow::Result<()> {
           // implementation
           Ok(())
       }
   }
   ```

2. Add a variant to `cmd::Command` in `xtask/src/cmd/mod.rs`:

   ```rust
   pub mod my_new_cmd;

   #[derive(Debug, Subcommand)]
   pub enum Command {
       // ...existing variants...
       /// One-line description shown in --help.
       MyNewCmd(my_new_cmd::Command),
   }

   impl Command {
       pub fn run(&self) -> anyhow::Result<()> {
           match self {
               // ...existing arms...
               Self::MyNewCmd(cmd) => cmd.run(),
           }
       }
   }
   ```

3. Use `crate::cargo::run_cargo(args)` to invoke cargo subcommands.
   Use `crate::cargo::load_metadata()` to access the workspace dependency graph.

## Cargo environment variable

When `cargo xtask` invokes child `cargo` commands, it reads the `CARGO`
environment variable for the path to the cargo binary. This is set
automatically when running inside `cargo run` or `cargo build`.
