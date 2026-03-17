# C-Chain Reexecution Benchmarks

Firewood measures performance by re-executing historical C-Chain blocks against
a state snapshot, reporting throughput in mgas/s. Results publish to GitHub
Pages automatically.

For how benchmarks work, what S3 data exists, and how to create snapshots, see
the source:
[**AvalancheGo C-Chain Re-Execution Benchmark README**](https://github.com/ava-labs/avalanchego/blob/master/tests/reexecute/c/README.md)

## Quick Start

Authenticate once:

```bash
nix run ./ffi#gh -- auth login   # or: gh auth login
export GH_TOKEN=$(gh auth token)
```

**Recommended: `firewood-40m-41m`** — contract-heavy mature mainnet, finishes
in <2h. Use this for A/B testing between versions and as the baseline for
performance comparisons. This is also what runs in the daily scheduled job.

```bash
# A/B test your branch vs main
TEST=firewood-40m-41m just bench-cchain

# Pin to a specific commit
FIREWOOD_REF=5406b68 TEST=firewood-40m-41m just bench-cchain

# Same range, custom params (before firewood-40m-41m is available as named test)
START_BLOCK=40000001 END_BLOCK=41000000 \
  BLOCK_DIR_SRC=cchain-mainnet-blocks-40m-50m-ldb \
  CURRENT_STATE_DIR_SRC=cchain-current-state-firewood-40m \
  just bench-cchain
```

See all available named tests: `./scripts/bench-cchain-reexecution.sh tests`

All env vars and options: `./scripts/bench-cchain-reexecution.sh help`

## Scheduled Runs

| Schedule | Test | ~Duration |
|---|---|---|
| Weekdays 05:00 UTC | `firewood-40m-41m` (blocks 40M–41M) | <2h |

## Choosing a Test

| Goal | Use |
|---|---|
| A/B test, daily-style run | `firewood-40m-41m` |
| Quick smoke test | `firewood-101-250k` (~7 min) |
| Deep regression | `firewood-33m-40m` (~7h, needs >6h path) |

If your test takes **>6h**, run it directly in AvalancheGo — `just bench-cchain`
polls via `gh run watch` which has a hard 6-hour limit and will time out before
the run completes. You lose GitHub Pages publishing but the run completes:
[AvalancheGo → Actions → C-Chain Re-Execution Benchmark w/ Container](https://github.com/ava-labs/avalanchego/actions)

## Finding S3 Data

AWS credentials required (Okta → Experimental → Access Keys, region `us-east-2`):

```bash
s5cmd ls s3://avalanchego-bootstrap-testing | grep blocks          # block sources
s5cmd ls s3://avalanchego-bootstrap-testing | grep current-state   # state snapshots
```

Strip the `s3://avalanchego-bootstrap-testing/` prefix and trailing `/` to get
`BLOCK_DIR_SRC` and `CURRENT_STATE_DIR_SRC` values. The naming convention
encodes height: `cchain-current-state-firewood-40m` = state at block 40,000,000.

**Pairing matters.** State snapshot must match the block immediately before
`START_BLOCK`. Wrong pairings silently produce incorrect results.
→ [How to create new snapshots](https://github.com/ava-labs/avalanchego/blob/master/tests/reexecute/c/README.md#how-to-create-and-use-a-re-execution-snapshot)

## Gotchas

1. **Push before triggering.** The workflow builds from the remote branch.
   Unpushed commits benchmark the wrong code — the justfile catches this.

2. **`GH_TOKEN` needs cross-repo access.** Requires a PAT for both
   `ava-labs/firewood` and `ava-labs/avalanchego`. Default `gh auth login`
   tokens scoped to one repo will fail silently.

3. **`AVALANCHEGO_REF` cannot be a commit SHA.** GitHub's `workflow_dispatch`
   only accepts branch or tag names. The justfile validates this.

4. **Monitoring timeout ≠ run failure.** `gh run watch` has a hard 6h limit.
   When it exits, the benchmark may still be running. Check the AvalancheGo
   run URL printed at trigger time before concluding failure.

5. **Results location = triggering branch, not benchmarked code.**
   `github.ref` controls storage, `FIREWOOD_REF` controls what runs:
   - `main` → `bench/` (official history — blocked in justfile, scheduled only)
   - feature branch → `dev/bench/{branch}/`

6. **Block/state pairing is your responsibility in custom mode.** No
   validation — wrong pairs silently produce bad results.

7. **Do not remove `ffi/flake.nix`.** The `track-performance` workflow depends
   on it via the Polyrepo toolchain. It is not dead code.

## Viewing Results

- [Main branch trends](https://ava-labs.github.io/firewood/bench/)
- [Feature branch trends](https://ava-labs.github.io/firewood/dev/bench/)
- Raw data: [benchmark-data/bench/data.js](https://github.com/ava-labs/firewood/blob/benchmark-data/bench/data.js)

## References

- [AvalancheGo C-Chain benchmark README](https://github.com/ava-labs/avalanchego/blob/master/tests/reexecute/c/README.md) — S3 data, snapshot creation, local runs, full config reference
- [Available named tests](https://github.com/ava-labs/avalanchego/blob/master/scripts/benchmark_cchain_range.sh)
