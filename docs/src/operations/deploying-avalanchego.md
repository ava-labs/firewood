# Deploying AvalancheGo with Firewood

This guide takes an operator from nothing to a running AvalancheGo node whose EVM
chain stores its Merkle trie in Firewood. It assumes a green-field stack: no existing
node images, no existing chain data, no existing storage or scheduling conventions to
inherit.

Everything here is derived from the [AvalancheGo](https://github.com/ava-labs/avalanchego)
and Firewood sources. Where a choice is genuinely a judgement call, the guide says so
and gives the reasoning rather than a bare number.

## What Firewood replaces (and what it does not)

Firewood is a drop-in backend for the EVM **trie node store** only. Selecting it
changes where account and storage trie nodes live; it changes nothing else about an
AvalancheGo deployment.

| Data | Backend | Location |
| --- | --- | --- |
| EVM state trie nodes | **Firewood** | `--chain-data-dir` |
| EVM blocks, receipts, tx index, chain metadata | AvalancheGo base database (LevelDB/PebbleDB) | `--db-dir` |
| P-Chain, X-Chain, platform state | AvalancheGo base database | `--db-dir` |
| Consensus, networking, staking keys | unchanged | `--staking-tls-cert-file`, etc. |

Two consequences follow immediately:

- **Firewood data does not live under `--db-dir`.** It lives under `--chain-data-dir`,
  which defaults to `$AVALANCHEGO_DATA_DIR/chainData` — a *sibling* of the default
  `--db-dir` (`$AVALANCHEGO_DATA_DIR/db`), not a child. A deployment that mounts
  persistent storage only at `--db-dir` will write the entire state trie to whatever
  backs the container root filesystem. This is the single most common way to get a
  Firewood deployment wrong. See [Provision storage](#step-4-provision-storage).
- **You still need a fast base database.** Firewood does not reduce the block and
  receipt storage requirement, which for a mainnet C-Chain archive node is
  substantial in its own right.

The scheme applies to any libevm-based chain in AvalancheGo — the C-Chain
(`graft/coreth`) and Subnet-EVM L1s (`graft/subnet-evm`) share the same wiring. The
examples below use the C-Chain; substitute the chain alias or blockchain ID
throughout for an L1.

## Step 1: Pin versions

Firewood ships to Go consumers as a prebuilt static library in the
`github.com/ava-labs/firewood-go-ethhash/ffi` module, which AvalancheGo pins in its
`go.mod`. See [AvalancheGo & EVM Integration](../integration/README.md) for how that
module is produced.

**Pin an exact AvalancheGo tag for the whole fleet and let it choose the Firewood
version.** Do not override the `firewood-go-ethhash/ffi` requirement independently:
the Go wrapper, the C header, and the Rust static library are built and released as
one unit, and the AvalancheGo integration code is written against a specific FFI
surface.

Two version-compatibility rules matter operationally:

- **The on-disk format is versioned and validated at open time.** Firewood writes a
  version identifier into the database header and refuses to open a file whose
  version it does not recognize. An upgrade that changes the format is a resync, not
  a restart.
- **The data *path* has changed before.** AvalancheGo v1.14.1 moved the Firewood
  directory as part of enabling non-pruning mode, and its release notes state that
  nodes using Firewood had to resync. Read the AvalancheGo release notes for every
  version you skip, not just the one you land on.

> [!IMPORTANT]
> Plan every Firewood version bump as a potential resync. Budget the bootstrap or
> snapshot-restore time before you schedule the upgrade, and roll one canary through
> the new version before the fleet.

## Step 2: Build a node binary and image

Firewood is Rust code linked into AvalancheGo through cgo. That imposes real build
constraints.

### Requirements

- **`CGO_ENABLED=1`.** AvalancheGo's `scripts/constants.sh` already exports this, but
  any custom build path must preserve it. A `CGO_ENABLED=0` build fails to link.
- **A supported target triple.** For deployment that means
  `x86_64-unknown-linux-gnu` or `aarch64-unknown-linux-gnu`; the module also ships
  macOS libraries for local development. Anything else requires building Firewood
  from source and redirecting the module with `go mod edit -replace`. Check the
  `libs/` directory of the pinned module version for the authoritative list.
- **glibc, not musl.** The Linux archives are `-gnu` targets. AvalancheGo's
  `STATIC_COMPILATION=1` path switches the compiler to `musl-gcc` and will not link
  against them. Build and run on a glibc base image (the stock AvalancheGo
  `Dockerfile` uses `golang:*-bookworm` to build and `debian:12-slim` to run — both
  are fine). **musl-based runtime images such as Alpine do not work.**
- **A cross-compiler when the build and target architectures differ.** The stock
  `Dockerfile` installs `gcc-aarch64-linux-gnu` or `gcc-x86-64-linux-gnu` and sets
  `CC` accordingly. Keep that behaviour in any derived image.

There is no build tag to enable Firewood and no separate binary: a standard
AvalancheGo build of a version that pins the FFI module already contains it. The
scheme is selected purely by configuration.

### Things that are *not* requirements

- **No `io_uring`.** The published static libraries are built with the `ethhash` and
  `logger` features only. The `io-uring` feature is off, so there is no kernel
  version floor and no seccomp allowance to add beyond what AvalancheGo already
  needs.
- **No runtime shared library.** Firewood is linked statically. The only additional
  link-time dependencies are `libm` and `libdl`, both present in any glibc base image.

### Build-system notes

The Linux static libraries are roughly 50 MB each, so the module download is large
compared to a pure-Go dependency. Cache the Go module cache in CI, and expect a cold
`go mod download` to move a few hundred megabytes.

Firewood uses the Ethereum-compatible Keccak hashing (`ethhash`) in the published
module. Firewood's native hashing is a different, incompatible on-disk format; if you
build from source for an EVM chain, you must enable the `ethhash` feature.

## Step 3: Configure the chain

Firewood is enabled by the EVM chain config, not by a node flag. AvalancheGo reads
per-chain configuration from either:

- `--chain-config-dir=<dir>`, with the chain's JSON at `<dir>/C/config.json`
  (directory name is the chain alias or blockchain ID); or
- `--chain-config-content=<base64>`, a base64-encoded JSON object mapping chain alias
  to config. This takes precedence over `--chain-config-dir` and is convenient for
  container platforms that inject configuration as environment variables
  (`AVAGO_CHAIN_CONFIG_CONTENT`).

### Minimal configuration

A pruning node:

```json
{
  "state-scheme": "firewood",
  "snapshot-cache": 0,
  "pruning-enabled": true,
  "state-sync-enabled": false,
  "commit-interval": 4096,
  "state-history": 8192
}
```

An archive node:

```json
{
  "state-scheme": "firewood",
  "snapshot-cache": 0,
  "pruning-enabled": false,
  "state-sync-enabled": false
}
```

> [!CAUTION]
> `"state-sync-enabled": false` is **required**, not a default worth copying by
> accident. Firewood state sync is not production ready, and omitting the key enables
> state sync on an empty chain. See
> [State sync](#state-sync--not-production-ready).

### Hard constraints

The node refuses to start if any of these is violated. Encode them as validation in
whatever renders your config, so a bad value fails at deploy time rather than at
container start.

| Setting | Required value | Failure |
| --- | --- | --- |
| `snapshot-cache` | `0` | `snapshot cache must be disabled for Firewood` |
| `offline-pruning-enabled` | unset / `false` | `offline pruning is not supported for Firewood` |
| `populate-missing-tries` | unset | `missing trie repopulation is not supported for Firewood` |
| existing chain data | must not already hold hashdb or pathdb state | `state scheme conflict` |

`snapshot-cache` must be zero because the flat snapshot layer requires iteration over
the state trie, which Firewood does not provide. AvalancheGo forces the internal
snapshot limit to zero for the Firewood scheme regardless, but the explicit
configuration check runs first and is fatal.

The state-scheme conflict check is one-way and deliberate: **you cannot convert an
existing hashdb chain data directory to Firewood in place.** Adopting Firewood on a
node that has already synced means starting from an empty chain data directory. See
[Bootstrapping](#step-5-bootstrap-the-node).

### Tuning knobs

These four EVM config keys are what actually configure the Firewood database. The
mapping is not obvious from their names.

| EVM config key | Firewood parameter | Default | Meaning |
| --- | --- | --- | --- |
| `state-history` | `RevisionsInMemory` | `32` | Number of revisions Firewood keeps live. Must be ≥ 2. |
| `commit-interval` | `DeferredCommitInterval` | `4096` | How many commits Firewood defers before persisting. |
| `trie-clean-cache` | node cache size (MB → bytes) | `512` | Firewood's read cache, allocated Rust-side. |
| `pruning-enabled` | `Archive` (inverted) | `true` | `false` enables archive mode and the on-disk root store. |

Three non-obvious behaviours:

- **`commit-interval` is silently clamped.** Firewood requires
  `DeferredCommitInterval < RevisionsInMemory`, so the effective value is
  `min(commit-interval, state-history - 1)`. With the default `state-history` of 32,
  a configured `commit-interval` of 4096 becomes 31. If you want deferred persistence
  to actually defer, **raise `state-history` above `commit-interval`** — that is why
  the pruning example above pairs `commit-interval: 4096` with `state-history: 8192`.
- **Firewood is the only scheme allowed a non-default `commit-interval` on production
  networks.** AvalancheGo rejects a non-default `commit-interval` on mainnet and Fuji
  for every other scheme. This is what makes commit-interval a legitimate tuning axis
  for Firewood fleets.
- **Archive mode costs extra disk and memory.** `pruning-enabled: false` turns on
  Firewood's root store, an on-disk index of historical roots that lives beside the
  main database file.

`commit-interval` is the main latency/durability dial. A larger value means fewer,
larger persists and less write amplification, at the cost of more blocks to
re-execute after an unclean shutdown. Start at the defaults, measure, then move one
axis at a time.

## Step 4: Provision storage

### Layout

For a chain with blockchain ID `<id>`, Firewood's files are:

```text
<chain-data-dir>/<id>/firewood/firewood.db      # the database — one large file
<chain-data-dir>/<id>/firewood/root_store/      # archive mode only
```

Mount persistent storage such that **both** `--db-dir` and `--chain-data-dir` land on
it. The simplest correct arrangement is to point `--data-dir` at a single mount and
let both defaults fall underneath it.

### Media

Firewood is a random-read, random-write workload against a single large file, and the
trie walk on every state read is latency-bound rather than throughput-bound.

- **Local NVMe (instance store) is strongly preferred.** This is the configuration
  Firewood's own C-Chain re-execution benchmarks target — see the
  [C-Chain reexecution benchmark](https://github.com/ava-labs/firewood/blob/main/benchmark/docs/cchain-reexecution.md),
  which uses `i4i`-class instances with local NVMe SSD as its default runner.
- **Network block storage works** but you are paying its round-trip on every trie
  node read. If you use it, provision high, *provisioned* IOPS rather than a
  burst-credit tier, and expect materially lower throughput than local NVMe.
- **Do not use network filesystems** (NFS, EFS, and similar). Firewood assumes local
  file semantics.

Use `ext4` or `xfs`. Nothing in Firewood requires a specific filesystem feature.

### Sizing and growth

Firewood is compaction-less. Superseded nodes are tracked in a future-delete log and
their space returns to size-classed free lists once the referencing revision expires,
so space *is* reused — but the file itself only ever grows. **On-disk size is a
high-water mark, not a measure of the live set, and there is no online shrink.**

Practical consequences:

- Size for the workload's peak, then add headroom. A mainnet C-Chain archive node
  needs multiple terabytes; a pruning node needs substantially less but still grows
  continuously with chain history.
- **Enable automatic volume expansion** where the platform supports it, and set the
  ceiling well above the current requirement. Discovering the limit during an
  incident is expensive.
- Alert on free space on the chain-data volume as a first-class signal. A full volume
  stops the node.
- The only way to reclaim the high-water mark is to rebuild the database from
  scratch.

### One node per device

If you use local NVMe, **exactly one AvalancheGo process may own a given device.**
Two nodes sharing one local disk will corrupt each other's data. On a scheduler this
is not a convention, it is a constraint you must enforce (see
[Kubernetes](#kubernetes)).

## Step 5: Bootstrap the node

A Firewood chain data directory must start empty. There are two supported ways to
reach a synced node, plus one that is **not** production ready.

### Bootstrap from the network

The default, and the one with no additional moving parts. AvalancheGo downloads and
executes every block. It is slow — days for a mainnet C-Chain archive node — but it
needs no external artefacts and no trust in a snapshot.

### Restore from a snapshot

The fastest path if you already operate Firewood nodes. Copy a consistent
`--db-dir` + `--chain-data-dir` pair from a node that was cleanly stopped.

> [!WARNING]
> Snapshots are **scheme-specific and configuration-specific**. A hashdb data
> directory is not a Firewood data directory, and a Firewood database written with
> one `commit-interval`/`state-history` pair should not be handed to a node
> configured with another. Key your snapshot storage by scheme and by the tuning
> parameters that produced it, and never let a restore job pick a snapshot from a
> different lineage. Restore both directories together — they are a matched pair.

### State sync — not production ready

> [!CAUTION]
> **Firewood state sync is not production ready. Set `"state-sync-enabled": false`
> on every Firewood node.**
>
> Code exists and is wired end to end — a client-side syncer that requests range
> proofs, and a server-side proof handler registered on the P2P network when the
> Firewood scheme is active — but it is still under active development and has not
> been qualified for production use. A node that syncs incorrectly does not
> necessarily fail loudly; it can reach an incorrect state root and then diverge from
> the network.

Leave it disabled and use one of the two paths above. This applies to both roles:

- **Serving.** The proof handler is registered whenever the Firewood scheme is
  active, independent of `state-sync-enabled`. Do not point other nodes at a Firewood
  node as a state sync source.
- **Syncing.** When `state-sync-enabled` is unset, AvalancheGo enables state sync if
  the chain is empty — which is exactly the state of a brand-new Firewood node. So
  **omitting the key is not the same as disabling it**, and a fresh node that omits
  it will attempt to state sync. Set it explicitly to `false`.

This guide will be updated when Firewood state sync is qualified. Until then, treat
any deployment that enables it as experimental, keep it off mainnet, and verify the
resulting state root against an independently bootstrapped node before trusting the
node with any traffic.

## Step 6: Size memory

Firewood allocates through jemalloc on the Rust side of the FFI boundary. **That
memory is invisible to the Go runtime and to `GOMEMLIMIT`.** A container sized as if
the Go heap were the whole process will OOM.

Budget, per node:

- the Go heap (blocks, receipts, caches, networking, consensus);
- Firewood's node cache — `trie-clean-cache`, in megabytes, allocated Rust-side;
- jemalloc overhead and fragmentation on top of that;
- in-flight proposals and live revisions, which scale with `state-history`.

Then:

1. Set the container memory limit to the total.
2. Set `GOMEMLIMIT` to the *Go* portion, leaving explicit headroom for everything
   Rust-side. Do not set it to the container limit.

`trie-clean-cache` is the largest knob you control directly and the right place to
start when a node is memory-pressured. It is rounded up to the nearest 64 MB.

## Step 7: Deploy

### Kubernetes

Nothing about Firewood requires Kubernetes, but if you use it, four things are
specific to this workload:

**Give each node a stable identity and stable storage.** A StatefulSet, or an
equivalent per-node application, so that a restarted pod reattaches to its own chain
data rather than bootstrapping fresh.

**Enforce one pod per NVMe host.** When the database lives on instance-store NVMe,
schedule with a *required* pod anti-affinity keyed on `kubernetes.io/hostname` and a
label that identifies Firewood-on-NVMe pods:

```yaml
affinity:
  podAntiAffinity:
    requiredDuringSchedulingIgnoredDuringExecution:
      - labelSelector:
          matchExpressions:
            - key: storage
              operator: In
              values: ["nvme"]
        topologyKey: kubernetes.io/hostname
```

Preferred anti-affinity is not sufficient. Two pods on one host means two processes
writing one device.

**Wire up the instance store explicitly.** Instance-store NVMe is not a PersistentVolume.
The usual arrangement is:

1. A dedicated node group of storage-optimized instances with local NVMe.
2. A boot-time step that initializes and mounts the ephemeral disk at a known path.
   Some node OSes provide this directly; otherwise a systemd unit or bootstrap script
   formats and mounts it.
3. Node labels recording the storage type and size, plus taints so only workloads
   that ask for local NVMe land there.
4. A `hostPath` (or local PersistentVolume) mount into the pod at that path, in place
   of a PVC.

When you use network block storage instead, keep the PVC and enable expansion — see
[Sizing and growth](#sizing-and-growth).

**Allow a real shutdown.** Firewood defers persistence, so the last
`commit-interval` window of state lives in memory. A clean shutdown persists it; a
`SIGKILL` does not. AvalancheGo recovers either way — on startup it logs
`Re-executing blocks to generate state for last accepted block` and replays from the
last persisted revision — but that replay is downtime proportional to the deferred
window. Set `terminationGracePeriodSeconds` generously, and remember that a larger
`commit-interval` makes an unclean shutdown more expensive, not less.

### Plain virtual machines

The same constraints, minus the scheduler: one node per host, chain data on local
NVMe, a systemd unit with a generous `TimeoutStopSec`, and monitoring on free space.

## Step 8: Observe

Firewood exposes Prometheus metrics through the FFI. AvalancheGo registers them under
a `firewood` prefix inside the chain's metric namespace.

> [!NOTE]
> Because Firewood's own metric names already begin with `firewood_`, the registration
> prefix produces a **doubled segment** in the exported name — for example
> `avalanche_<chain-namespace>_firewood_firewood_revisions_active`. This is expected.
> Write dashboard queries against the exported names, not the names in Firewood's
> metric reference.

Three metric families are worth wiring into dashboards from day one:

| Source | Prefix | Covers |
| --- | --- | --- |
| Firewood (Rust, via FFI) | `firewood_…` | proposals, revisions, commits, cache, persistence, I/O |
| AvalancheGo Firewood TrieDB | `firewood/triedb/…` | hash and commit counts and timings, proposal bookkeeping |
| Firewood state syncer | `sync_firewood` registry | state sync progress — should stay silent; see [State sync](#state-sync--not-production-ready) |

Signals to alert on:

- **Free space on the chain-data volume.** The database only grows; see
  [Sizing and growth](#sizing-and-growth).
- **Revisions active vs. the configured limit.** Sustained pressure at the limit
  means revision reaping is blocked by a live reference.
- **Commits blocked waiting for a persist permit.** The persist worker is not keeping
  up with the commit rate — usually a storage problem.
- **Commit duration.** The clearest early indicator of storage degradation.

Firewood's full metric reference lives in
[`METRICS.md`](https://github.com/ava-labs/firewood/blob/main/METRICS.md), including
the distinction between cheap metrics (always recorded) and expensive metrics (gated
per call).

At startup a Firewood node logs, in order:

```text
Firewood state scheme is enabled
This is untested in production, use at your own risk
```

Treat the absence of those lines as a failed rollout: the node started on the default
hashdb scheme.

## Step 9: Verify

Before sending traffic:

1. **Confirm the scheme took effect.** The two startup warnings above appear in the
   node log.
2. **Confirm the files exist where you expect.** `<chain-data-dir>/<id>/firewood/firewood.db`
   is present and growing, and it is on the volume you provisioned — not on the
   container root filesystem. `df` the path; do not infer it.
3. **Confirm metrics are flowing.** At least one
   `…_firewood_firewood_…` series is present on the metrics endpoint.
4. **Confirm the node reaches the chain tip** and that its state root matches a
   reference node at the same height.
5. **Test a restart.** Stop the node cleanly, start it, and confirm it resumes without
   a long replay. Then kill it uncleanly and confirm it recovers — this is the
   behaviour you will depend on during an incident, and it is better discovered on
   purpose.

## Known limitations

Current as of the AvalancheGo integration described here. Verify against the release
you deploy.

**Unsupported EVM configuration:**

- Offline pruning (`offline-pruning-enabled`).
- Missing-trie repopulation (`populate-missing-tries`).
- The flat snapshot layer (`snapshot-cache` must be `0`).
- The `path` state scheme is unsupported by AvalancheGo generally; use `hash` or
  `firewood`.

**State sync is not production ready.** Keep `state-sync-enabled` set to `false`, and
do not use Firewood nodes as a state sync source for others. See
[State sync](#state-sync--not-production-ready).

**Unsupported RPC methods.** Several `debug` APIs return
`firewood triedb scheme does not yet support this operation`, including state dumps
and storage-range queries. If your consumers rely on those, Firewood is not yet a
drop-in replacement for them. Historical state *reads* (`eth_getBalance` and friends
at an old block) do work: AvalancheGo reconstructs historical state by opening a
Firewood revision and re-executing forward.

**No in-place scheme conversion.** Switching to or from Firewood requires a fresh
chain data directory.

## Rolling back

Because the schemes cannot be converted in place, rollback is a redeploy onto empty
chain data, not a config flip.

Make it cheap in advance:

- Keep a current hashdb snapshot for the same network and AvalancheGo version, so a
  rollback is a restore rather than a full bootstrap.
- Never overwrite a hashdb snapshot lineage with a Firewood one. Keep them in separate
  namespaces.
- Roll out canary-first, and keep enough non-Firewood capacity to absorb the canary's
  traffic while it is out.

## See also

- [AvalancheGo & EVM Integration](../integration/README.md) — how the Go module is
  produced and versioned.
- [Concepts & Architecture](../concepts/README.md) — revisions, proposals, the
  future-delete log, and free lists.
- [Operations & Benchmarking](README.md) — `fwdctl` and the benchmark suites.
- [Firewood metric reference](https://github.com/ava-labs/firewood/blob/main/METRICS.md).
