# Operations & Benchmarking

This section covers running and measuring Firewood.

## Deployment

- [Deploying AvalancheGo with Firewood](deploying-avalanchego.md) — build, configure,
  provision, and operate an AvalancheGo node that stores EVM state in Firewood.

## `fwdctl`

`fwdctl` is the command-line tool for operating on a Firewood database. See its
[README](https://github.com/ava-labs/firewood/blob/main/fwdctl/README.md) for the
available commands.

## Benchmarking

Firewood tracks performance with a C-Chain reexecution benchmark and synthetic
workloads:

- [C-Chain reexecution benchmark](https://github.com/ava-labs/firewood/blob/main/benchmark/docs/cchain-reexecution.md)
- [Synthetic workloads](https://github.com/ava-labs/firewood/blob/main/benchmark/docs/synthetic-workloads.md)

Live benchmark dashboards are published at [`/bench/` ↗](/firewood/bench/) (resolves
only on the deployed site).
