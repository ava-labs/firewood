window.BENCHMARK_DATA = {
  "lastUpdate": 1794954095763,
  "repoUrl": "https://github.com/ava-labs/firewood",
  "entries": {
    "C-Chain Reexecution with Firewood": [
      {
        "commit": {
          "author": {
            "name": "rodrigo",
            "username": "RodrigoVillar",
            "email": "77309055+RodrigoVillar@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "ed032a565a7671a9f052d6ef073b13f093a76767",
          "message": "refactor: move fwdctl scripts to fwdctl (#2416)\n\n## Why this should be merged\n\nCloses #1807. `benchmark/` held files that only `fwdctl launch` uses,\nalong with two scripts that nothing used. Moving the launch files under\n`fwdctl/` puts them next to the code that embeds and runs them, and\nleaves `benchmark/` with only the synthetic benchmark crate and docs.\n\nMoving these files into `fwdctl` also makes deprecating `fwdctl launch`\nin favor of internal tooling easier to reason about.\n\n## How this works\n\n- Moves `launch-stages.yaml`, `setup-scripts/`, and\n`Grafana-dashboard.json` from `benchmark/` to `fwdctl/launch/` and\nupdates all references\n(`include_str!` paths, script paths in the stages, the `infra/` path in\nthe installers, the dashboard download URL, and docs).\n- Deletes the unused `build-firewood.sh` and `run-benchmarks.sh`. The\nbenchmark commands are already in\n`benchmark/docs/synthetic-workloads.md`.\n- Adds `fwdctl/launch/**` to the license-header check's no-license list,\nas `benchmark/**` was before.\n\n## How this was tested\n\nCI.\n\n## Breaking Changes\n\n- [ ] firewood\n- [ ] firewood-storage\n- [ ] firewood-ffi (C api)\n- [ ] firewood-go (Go api)\n- [ ] fwdctl",
          "timestamp": "2026-11-17T17:06:01Z",
          "url": "https://github.com/ava-labs/firewood/commit/ed032a565a7671a9f052d6ef073b13f093a76767"
        },
        "date": 1794954094971,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 164.03659510161157,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 6096.200664129584,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 110.00322861460359,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 5.707487403062054,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 5976.125356596256,
            "unit": "block_accept_ms/ggas"
          }
        ]
      }
    ]
  }
}