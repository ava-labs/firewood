# Firewood Metrics Migration Guide

This document maps every old Firewood metric name to its replacement in the
`firewood_*` naming scheme introduced in
[#1912](https://github.com/ava-labs/firewood/pull/1912).

## Prefix applied by the upstream collector

When Firewood is used via the FFI/Go layer in AvalancheGo (coreth / subnet-evm),
the upstream Prometheus scrape job adds the `avalanche_evm_` prefix to every
exposed metric. The internal Firewood metric names therefore appear in dashboards
and alerts as:

```text
avalanche_evm_<firewood-metric-name>
```

For example, the internal name `firewood_commits_total` is scraped as
`avalanche_evm_firewood_commits_total`.

> **Temporary double-prefix:** Until the upstream scrape configuration is
> updated to remove its existing `firewood_` relabelling, some deployments will
> temporarily expose metrics under the name
> `avalanche_evm_firewood_firewood_<suffix>`. This will be resolved once the
> upstream collector is adjusted.

---

## What changed (summary)

- **Naming convention**: All metrics now follow
  `firewood_{subsystem}_{noun}_{unit}`.
- **Counters** always end in `_total`.
- **Histograms** use base-unit suffixes (`_seconds`, `_bytes`).
- **Gauges** carry no `_total` suffix.
- **Duration counters** (old `_ms` / `_ns` suffix) are replaced by **histograms**
  so percentiles (p50/p99) can now be computed.
- **New metrics** have been added with no prior equivalent.
- **Removed metrics** had no replacement (see section below).

---

## Metric-by-metric mapping

### Database layer

#### Proposals

| Old metric           | Old type                  | New metric                                  | New type  | Notes                                    |
| -------------------- | ------------------------- | ------------------------------------------- | --------- | ---------------------------------------- |
| `proposals`          | counter                   | `firewood_proposals_total`                  | counter   | Label `base` added (revision type)       |
| `proposal.create`    | counter (`success` label) | _(removed)_                                 | —         | Superseded by `firewood_proposals_total` |
| `proposal.create_ms` | counter                   | `firewood_propose_duration_seconds`         | histogram | Millisecond counter → seconds histogram  |
| `proposal.commit`    | counter (`success` label) | `firewood_proposal_commits_total`           | counter   | Renamed; same `success` label            |
| `proposal.commit_ms` | counter (`success` label) | `firewood_proposal_commit_duration_seconds` | histogram | Millisecond counter → seconds histogram  |
| _(none)_             | —                         | `firewood_proposals_discarded_total`        | counter   | **New**                                  |
| _(none)_             | —                         | `firewood_proposals_uncommitted`            | gauge     | **New**                                  |
| _(none)_             | —                         | `firewood_proposals_reparented_total`       | counter   | **New**                                  |

#### Revisions

| Old metric         | Old type | New metric                        | New type | Notes   |
| ------------------ | -------- | --------------------------------- | -------- | ------- |
| `active_revisions` | gauge    | `firewood_revisions_active`       | gauge    | Renamed |
| `max_revisions`    | gauge    | `firewood_revisions_limit`        | gauge    | Renamed |
| _(none)_           | —        | `firewood_commits_total`          | counter  | **New** |
| _(none)_           | —        | `firewood_commits_blocked_total`  | counter  | **New** |
| _(none)_           | —        | `firewood_nodes_pending_deletion` | gauge    | **New** |

#### Merkle trie operations

| Old metric | Old type                            | New metric                               | New type | Notes                                |
| ---------- | ----------------------------------- | ---------------------------------------- | -------- | ------------------------------------ |
| `insert`   | counter (`merkle` label)            | `firewood_node_inserts_total`            | counter  | Label renamed `merkle` → `operation` |
| `remove`   | counter (`prefix`, `result` labels) | `firewood_node_removes_total`            | counter  | Labels unchanged                     |
| _(none)_   | —                                   | `firewood_change_proof_iterations_total` | counter  | **New**                              |

#### Persist worker

| Old metric    | Old type                 | New metric                                     | New type  | Notes               |
| ------------- | ------------------------ | ---------------------------------------------- | --------- | ------------------- |
| `flush_nodes` | counter (ms accumulator) | `firewood_flush_duration_seconds`              | histogram | Counter → histogram |
| _(none)_      | —                        | `firewood_persist_cycle_duration_seconds`      | histogram | **New**             |
| _(none)_      | —                        | `firewood_persist_permits_available`           | gauge     | **New**             |
| _(none)_      | —                        | `firewood_persist_permits_limit`               | gauge     | **New**             |
| _(none)_      | —                        | `firewood_persist_root_store_total`            | counter   | **New**             |
| _(none)_      | —                        | `firewood_persist_root_store_duration_seconds` | histogram | **New**             |
| _(none)_      | —                        | `firewood_persist_submit_duration_seconds`     | histogram | **New**             |
| _(none)_      | —                        | `firewood_reap_duration_seconds`               | histogram | **New**             |

#### Lock-contention diagnostics (all new)

| New metric                                    | Type      |
| --------------------------------------------- | --------- |
| `firewood_commit_lock_wait_seconds`           | histogram |
| `firewood_current_revision_lock_wait_seconds` | histogram |
| `firewood_by_hash_lock_wait_seconds`          | histogram |

---

### Storage layer

#### Space allocation

| Old metric       | Old type                | New metric                              | New type | Notes                    |
| ---------------- | ----------------------- | --------------------------------------- | -------- | ------------------------ |
| `space.reused`   | counter (`index` label) | `firewood_storage_bytes_reused_total`   | counter  | Renamed                  |
| `space.from_end` | counter (`index` label) | `firewood_storage_bytes_appended_total` | counter  | Renamed                  |
| `space.freed`    | counter (`index` label) | `firewood_storage_bytes_freed_total`    | counter  | Renamed                  |
| _(none)_         | —                       | `firewood_storage_bytes_wasted_total`   | counter  | **New** (fragmentation)  |
| _(none)_         | —                       | `firewood_nodes_allocated_total`        | counter  | **New** (per size class) |
| `delete_node`    | counter (`index` label) | `firewood_nodes_deleted_total`          | counter  | Renamed                  |
| _(none)_         | —                       | `firewood_free_list_entries`            | gauge    | **New**                  |

#### Node reads and caches

| Old metric            | Old type                        | New metric                               | New type | Notes   |
| --------------------- | ------------------------------- | ---------------------------------------- | -------- | ------- |
| `read_node`           | counter (`from` label)          | `firewood_node_reads_total`              | counter  | Renamed |
| `cache.node`          | counter (`mode`, `type` labels) | `firewood_node_cache_accesses_total`     | counter  | Renamed |
| `cache.freelist`      | counter (`type` label)          | `firewood_freelist_cache_accesses_total` | counter  | Renamed |
| `cache.freelist.size` | gauge                           | `firewood_freelist_cache_entries`        | gauge    | Renamed |
| _(none)_              | —                               | `firewood_node_cache_bytes`              | gauge    | **New** |
| _(none)_              | —                               | `firewood_node_cache_limit_bytes`        | gauge    | **New** |

#### Persistence

| Old metric   | Old type | New metric                          | New type  | Notes               |
| ------------ | -------- | ----------------------------------- | --------- | ------------------- |
| `io.read`    | counter  | `firewood_io_reads_total`           | counter   | Renamed             |
| `io.read_ms` | counter  | `firewood_io_read_duration_seconds` | histogram | Counter → histogram |
| _(none)_     | —        | `firewood_io_writes_total`          | counter   | **New**             |
| _(none)_     | —        | `firewood_io_bytes_read_total`      | counter   | **New**             |
| _(none)_     | —        | `firewood_io_bytes_written_total`   | counter   | **New**             |
| _(none)_     | —        | `firewood_database_size_bytes`      | gauge     | **New**             |
| _(none)_     | —        | `firewood_rootstore_lookups_total`  | counter   | **New**             |

#### io_uring (feature-gated)

| Old metric                 | Old type | New metric                                      | New type | Notes   |
| -------------------------- | -------- | ----------------------------------------------- | -------- | ------- |
| `ring.full`                | counter  | `firewood_io_uring_ring_full_total`             | counter  | Renamed |
| `ring.eagain_write_retry`  | counter  | `firewood_io_uring_eagain_retries_total`        | counter  | Renamed |
| `ring.partial_write_retry` | counter  | `firewood_io_uring_partial_write_retries_total` | counter  | Renamed |
| _(none)_                   | —        | `firewood_io_uring_sq_waits_total`              | counter  | **New** |

---

### FFI / Go layer

| Old metric                    | Old type  | New metric                                     | New type  | Notes                                                      |
| ----------------------------- | --------- | ---------------------------------------------- | --------- | ---------------------------------------------------------- |
| `ffi.batch`                   | counter   | _(removed)_                                    | —         | No direct replacement                                      |
| `ffi.batch_ms`                | counter   | _(removed)_                                    | —         | No direct replacement                                      |
| `ffi.propose`                 | counter   | _(removed)_                                    | —         | Superseded by `firewood_proposals_total`                   |
| `ffi.propose_ms`              | counter   | _(removed)_                                    | —         | Superseded by `firewood_propose_duration_seconds`          |
| `ffi.commit`                  | counter   | _(removed)_                                    | —         | Superseded by `firewood_proposal_commits_total`            |
| `ffi.commit_ms`               | counter   | _(removed)_                                    | —         | Superseded by `firewood_proposal_commit_duration_seconds`  |
| `ffi.cached_view.hit`         | counter   | _(removed)_                                    | —         | No direct replacement                                      |
| `ffi.cached_view.miss`        | counter   | _(removed)_                                    | —         | No direct replacement                                      |
| `ffi.gather_duration_seconds` | histogram | `firewood_gather_duration_seconds`             | histogram | Renamed (dot-notation → underscore; `ffi_` prefix dropped) |
| _(none)_                      | —         | `firewood_proof_merges_total`                  | counter   | **New**                                                    |
| _(none)_                      | —         | `firewood_go_proof_marshal_duration_seconds`   | histogram | **New** (`proof_type` label)                               |
| _(none)_                      | —         | `firewood_go_proof_unmarshal_duration_seconds` | histogram | **New** (`proof_type` label)                               |

---

### Replay layer (all new)

| New metric                                 | Type      |
| ------------------------------------------ | --------- |
| `firewood_replay_propose_duration_seconds` | histogram |
| `firewood_replay_commit_duration_seconds`  | histogram |

Previously, the replay crate exposed:

| Old metric          | Old type | Notes                                         |
| ------------------- | -------- | --------------------------------------------- |
| `replay.propose_ns` | counter  | Nanosecond accumulator; replaced by histogram |
| `replay.commit_ns`  | counter  | Nanosecond accumulator; replaced by histogram |

---

### Allocator (jemalloc — unchanged)

The jemalloc metrics are **not** prefixed with `firewood_` and their names are
unchanged:

| Metric                     | Type  |
| -------------------------- | ----- |
| `jemalloc_active_bytes`    | gauge |
| `jemalloc_allocated_bytes` | gauge |
| `jemalloc_metadata_bytes`  | gauge |
| `jemalloc_mapped_bytes`    | gauge |
| `jemalloc_resident_bytes`  | gauge |
| `jemalloc_retained_bytes`  | gauge |

---

## Removed metrics (no replacement)

These metrics were removed with no direct successor. Any alerts or recording
rules referencing them should be deleted or replaced using the new metrics
described above.

| Removed metric         | Reason                                                   |
| ---------------------- | -------------------------------------------------------- |
| `ffi.batch`            | FFI batch count now implicit in proposal/commit counters |
| `ffi.batch_ms`         | Replaced by proposal/commit duration histograms          |
| `ffi.propose`          | Replaced by `firewood_proposals_total`                   |
| `ffi.propose_ms`       | Replaced by `firewood_propose_duration_seconds`          |
| `ffi.commit`           | Replaced by `firewood_proposal_commits_total`            |
| `ffi.commit_ms`        | Replaced by `firewood_proposal_commit_duration_seconds`  |
| `ffi.cached_view.hit`  | Removed (view caching metrics not re-added)              |
| `ffi.cached_view.miss` | Removed (view caching metrics not re-added)              |
| `proposal.create`      | Superseded by `firewood_proposals_total`                 |
| `proposal.create_ms`   | Replaced by `firewood_propose_duration_seconds`          |

---

## PromQL migration examples

### Proposal commit latency (p99)

```promql
# Before
rate(avalanche_evm_firewood_proposal_commit_ms[5m]) /
rate(avalanche_evm_firewood_proposal_commit[5m])

# After
histogram_quantile(0.99,
  sum(rate(avalanche_evm_firewood_proposal_commit_duration_seconds_bucket[5m]))
  by (le)
)
```

### Metrics gather latency (p90)

`firewood_gather_duration_seconds` is a **native histogram** — query it without
`_bucket` and without `by (le)`.

```promql
# Before
histogram_quantile(0.90,
  sum(rate(avalanche_evm_firewood_ffi_gather_duration_seconds_bucket[5m]))
  by (le)
)

# After (native histogram — no _bucket suffix)
histogram_quantile(0.90,
  sum(rate(avalanche_evm_firewood_gather_duration_seconds[5m]))
)
```

### Commit throughput (commits/sec)

```promql
# Before
rate(avalanche_evm_firewood_proposal_commit{success="true"}[1m])

# After
rate(avalanche_evm_firewood_commits_total[1m])
```

### Node cache hit rate

```promql
# Before
rate(avalanche_evm_firewood_cache_node{type="hit"}[5m]) /
rate(avalanche_evm_firewood_cache_node[5m])

# After
sum(rate(avalanche_evm_firewood_node_cache_accesses_total{type="hit"}[5m]))
  / sum(rate(avalanche_evm_firewood_node_cache_accesses_total[5m]))
```

### I/O read average latency

`firewood_io_read_duration_seconds` is a **native histogram** — query it without
`_bucket` and without `by (le)`.

```promql
# Before
rate(avalanche_evm_firewood_io_read_ms[1m]) /
rate(avalanche_evm_firewood_io_read[1m])

# After (p99, native histogram — no _bucket suffix)
histogram_quantile(0.99,
  sum(rate(avalanche_evm_firewood_io_read_duration_seconds[5m]))
)
```

### Database growth rate (bytes/sec)

```promql
# Before
rate(avalanche_evm_firewood_space_from_end[5m])   # raw node sizes

# After
sum(rate(avalanche_evm_firewood_storage_bytes_appended_total[5m]))
```
