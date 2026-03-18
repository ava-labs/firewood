# Firewood Benchmarks

Firewood has two categories of benchmarks:

- **Macro benchmarks** — C-Chain re-execution against real mainnet state; measures end-to-end system throughput
- **Micro benchmarks** — synthetic workloads and criterion suites that exercise the Rust API directly, no AvalancheGo required

## What we have

| Benchmark | Use when | Environment |
| --- | --- | --- |
| C-Chain re-execution — GitHub Actions | Tracking performance over time, A/B between versions | Managed CI — scheduled, results published to GitHub Pages |
| C-Chain re-execution — fwdctl launch | Investigating regression, in-depth profiling, tuning | EC2 instance you provision — full root access |
| Rust criterion | Iterating on a specific operation locally; enforces pure-Rust API in CI | Local or CI (`benchmarks.yaml`) |
| Synthetic workloads | Testing Firewood API patterns without AvalancheGo | Local |

## Details

### C-Chain re-execution — GitHub Actions

**When:** tracking performance over time or comparing two versions — runs automatically on a daily schedule and on every trigger, no manual setup needed.

Part of CI — repeatable, versioned, and auditable. Runs on a fixed schedule
and on demand via
[`track-performance.yml`](.github/workflows/track-performance.yml). Each run
is isolated on a dedicated self-hosted runner, keeping variance low enough that
a meaningful difference reflects code, not infrastructure. Results accumulate
on GitHub Pages.

```bash
TEST=firewood-40m-41m just bench-cchain
```

→ [Full guide](docs/cchain-reexecution.md)

### C-Chain re-execution — fwdctl launch

**When:** a GitHub Actions run surfaced a signal worth investigating — SSH into the instance, install any tooling, change code and rebuild freely.

Same workload as GitHub Actions, provisioned on demand on EC2. Full root access
— install `perf`, flamegraphs, or any tooling, change code and rebuild freely.
No CI queue, no constraints.

→ [Full guide](../fwdctl/README.launch.md)

### Rust criterion

**When:** iterating on a specific operation locally — also runs in CI on every push to `main`, catching API breakage and performance regressions before they merge, and enforcing that Firewood stays usable as a standalone Rust library without AvalancheGo.

Criterion benchmarks live in `firewood/benches/` and `storage/benches/`. They
run in seconds with no external dependencies.

```bash
cargo bench --features ethhash,logger
```

### Synthetic workloads

**When:** testing Firewood API patterns in isolation.

A standalone Rust binary exercising the Firewood API directly with synthetic
trie patterns (tenkrandom, zipf, single). No AvalancheGo, no Go, no cloud.

→ [Workload specs](docs/synthetic-workloads.md)
