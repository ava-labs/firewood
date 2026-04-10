window.BENCHMARK_DATA = {
  "lastUpdate": 1775800184428,
  "repoUrl": "https://github.com/ava-labs/firewood",
  "entries": {
    "C-Chain Reexecution with Firewood": [
      {
        "commit": {
          "author": {
            "name": "Brandon LeBlanc",
            "username": "demosdemon",
            "email": "brandon.leblanc@avalabs.org"
          },
          "committer": {
            "name": "Brandon LeBlanc",
            "username": "demosdemon",
            "email": "brandon.leblanc@avalabs.org"
          },
          "id": "51065a71352f227b286ba63a23f3a2600f1b4fb7",
          "message": "feat(metrics): Overhaul metrics for Prometheus best practices\n\n## Why this should be merged:\nFirewood's metrics were inconsistently named (dot-notation, no namespace\nprefix, abbreviated units like `_ms`/`_ns`), used counters to accumulate\ndurations (preventing percentile calculation), and contained redundant or\nlow-value entries. This brings all metrics into alignment with Prometheus\nconventions and adds coverage for previously uninstrumented paths.\n\n## How this works:\n- Rename all metrics to `firewood_{subsystem}_{noun}_{unit}` with `_total`\n  suffix on counters and base-unit suffixes (`_seconds`, `_bytes`) on\n  histograms/gauges; no dots, no camelCase\n- Replace counter-based duration accumulators (`_ms`, `_ns`) with proper\n  histograms so percentiles (p50/p99) can be computed\n- Remove the vestigial `DbMetrics` struct and `Db::metrics()` public API\n- Add new counters: commits, I/O reads/writes/bytes, nodes allocated/wasted\n  per free-list size class\n- Add new gauges: database file size, free-list entries per size class\n- Add new histograms: propose duration, persist-cycle duration, lock-wait\n  times for `in_memory_revisions` and `by_hash` in `RevisionManager`,\n  persist-worker submission duration\n- Overhaul `#[crate::metrics]` proc macro to accept a registry `Ident`\n  instead of a string literal; the compiler now validates that both the\n  counter (`IDENT`) and histogram (`IDENT_DURATION_SECONDS`) constants\n  exist in `crate::registry`, enabling bucket configuration via\n  `define_metrics!`\n- Add Go-side prometheus registry in `ffi/metrics.go` with histograms for\n  proof marshal/unmarshal operations (`firewood_go_proof_marshal_duration_seconds`,\n  `firewood_go_proof_unmarshal_duration_seconds`); merged into\n  `GatherRenderedMetrics()` alongside Rust-recorder metrics\n- Update `ffi/metrics_test.go` to use renamed metric names and the\n  corrected `firewood_gather_duration_seconds` histogram name\n\n## How this was tested:\n- `cargo nextest run --workspace --features ethhash,logger --all-targets`\n  (529/529 pass)\n- `cargo clippy --workspace --features ethhash,logger --all-targets`\n  (no warnings)\n- `cargo doc --no-deps` (no warnings)\n- `go build ./...` in `ffi/` (clean)\n\n## Breaking Changes:\nAll metric names have changed. Any existing dashboards, alerts, or\nrecording rules referencing the old names (`proposals`, `insert`,\n`flush_nodes`, `io.read_ms`, `replay.propose_ns`, `ffi.gather_duration_seconds`,\n`jemalloc.active_bytes`, etc.) will need to be updated to the new\n`firewood_*` naming scheme.",
          "timestamp": "2026-04-10T03:52:57Z",
          "url": "https://github.com/ava-labs/firewood/commit/51065a71352f227b286ba63a23f3a2600f1b4fb7"
        },
        "date": 1775800183764,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 166.021383245688,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 6023.32049312071,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 113.4398371829434,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 5826.632335040704,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 80.46894421384823,
            "unit": "block_accept_ms/ggas"
          }
        ]
      }
    ]
  }
}