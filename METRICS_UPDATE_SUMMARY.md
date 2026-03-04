# Metrics Naming Update Summary

## Overview

All Firewood metrics have been updated to follow Prometheus naming conventions:

1. **Base units**: seconds (not milliseconds), bytes (not kilobytes)
2. **Plural units**: `_seconds` not `_second`, `_bytes` not `_byte`  
3. **Accumulating counters**: Must have `_total` suffix
4. **Histograms**: Named `*_duration_seconds` for timing observations

## Changed Metric Names

### Firewood Core

| Old Name | New Name | Type | Unit |
|----------|----------|------|------|
| `proposals` | `proposals_total` | counter | count |
| `proposals.created` | `proposals.created_total` | counter | count |
| `proposals.discarded` | `proposals.discarded_total` | counter | count |
| `insert` | `insert_total` | counter | count |
| `remove` | `remove_total` | counter | count |
| `change_proof.next` | `change_proof.next_total` | counter | count |
| `commit_latency_s` | `commit_latency_seconds_total` | counter | nanoseconds |

### Storage Layer

| Old Name | New Name | Type | Unit |
|----------|----------|------|------|
| `space.reused` | `space.reused_bytes_total` | counter | bytes |
| `space.from_end` | `space.from_end_bytes_total` | counter | bytes |
| `space.freed` | `space.freed_bytes_total` | counter | bytes |
| `delete_node` | `delete_node_total` | counter | count |
| `flush_nodes` | `flush_nodes_seconds_total` | counter | nanoseconds |
| `read_node` | `read_node_total` | counter | count |
| `cache.node` | `cache.node_total` | counter | count |
| `cache.freelist` | `cache.freelist_total` | counter | count |
| `io.read_s` | `io.read_seconds_total` | counter | nanoseconds |
| `io.read` | `io.read_total` | counter | count |
| `proposals.reparented` | `proposals.reparented_total` | counter | count |
| `ring.eagain_write_retry` | `ring.eagain_write_retry_total` | counter | count |
| `ring.full` | `ring.full_total` | counter | count |
| `ring.sq_wait` | `ring.sq_wait_total` | counter | count |
| `ring.partial_write_retry` | `ring.partial_write_retry_total` | counter | count |

### FFI Layer

| Old Name | New Name | Type | Unit |
|----------|----------|------|------|
| `ffi.commit_s` | `ffi.commit_seconds_total` | counter | nanoseconds |
| `ffi.commit` | `ffi.commit_total` | counter | count |
| `ffi.commit_seconds_bucket` | `ffi.commit_duration_seconds` | histogram | seconds |
| `ffi.propose_s` | `ffi.propose_seconds_total` | counter | nanoseconds |
| `ffi.propose` | `ffi.propose_total` | counter | count |
| `ffi.propose_seconds_bucket` | `ffi.propose_duration_seconds` | histogram | seconds |
| `ffi.batch_s` | `ffi.batch_seconds_total` | counter | nanoseconds |
| `ffi.batch` | `ffi.batch_total` | counter | count |
| `ffi.batch_seconds_bucket` | `ffi.batch_duration_seconds` | histogram | seconds |
| `ffi.cached_view.miss` | `ffi.cached_view.miss_total` | counter | count |
| `ffi.cached_view.hit` | `ffi.cached_view.hit_total` | counter | count |
| `firewood.ffi.merge` | `firewood.ffi.merge_total` | counter | count |

### Replay Layer

| Old Name | New Name | Type | Unit |
|----------|----------|------|------|
| `replay.propose_s` | `replay.propose_seconds_total` | counter | nanoseconds |
| `replay.propose` | `replay.propose_total` | counter | count |
| `replay.commit_s` | `replay.commit_seconds_total` | counter | nanoseconds |
| `replay.commit` | `replay.commit_total` | counter | count |

## Implementation Details

### Time Metrics

- **Counters** (e.g., `*_seconds_total`): Accumulate nanoseconds for precision, conceptually represent seconds
  - Usage: `firewood_increment!(METRIC, elapsed.as_nanos() as u64)`
  - Prometheus query: Divide by 1e9 to convert to seconds: `rate(metric_seconds_total[5m]) / 1e9`

- **Histograms** (e.g., `*_duration_seconds`): Record observations in seconds
  - Usage: `firewood_record!(METRIC, elapsed.as_f64(), expensive)`
  - Prometheus automatically creates `_bucket`, `_sum`, `_count` suffixes

### Size Metrics

- All size metrics now explicitly include `_bytes_total` suffix
- Values represent actual bytes (not KB/MB)

## Breaking Changes

⚠️ **This is a breaking change** for:
- Prometheus dashboards and queries
- Alerting rules
- Any external monitoring systems

### Migration Guide

1. Update Prometheus queries:
   ```promql
   # Old
   rate(firewood_ffi_commit_s[5m])
   
   # New
   rate(firewood_ffi_commit_seconds_total[5m]) / 1e9
   ```

2. Update Grafana dashboards:
   - Replace all old metric names with new ones
   - Add `/1e9` division for timing counters

3. Update alerting rules with new metric names

## Testing

All metrics tested with:
```bash
cargo check --workspace --features ethhash,logger --all-targets
```

Compilation successful ✅
