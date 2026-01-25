window.BENCHMARK_DATA = {
  "lastUpdate": 1769350676084,
  "repoUrl": "https://github.com/ava-labs/firewood",
  "entries": {
    "C-Chain Reexecution with Firewood": [
      {
        "commit": {
          "author": {
            "name": "Elvis S.",
            "username": "Elvis339",
            "email": "elvissabanovic3@gmail.com"
          },
          "committer": {
            "name": "Elvis S.",
            "username": "Elvis339",
            "email": "elvissabanovic3@gmail.com"
          },
          "id": "e19e9510812ba2ac7df19b84296b4bc67783b7a8",
          "message": "fix(track-performance): improve concurrent trigger handling\n\nAdd input verification to reliably identify workflow runs when multiple\ntriggers occur simultaneously. Previously, the script picked the newest\nrun after a timestamp which could select the wrong run in concurrent\nscenarios.\n\nChanges:\n- Add verify_run_inputs() to match runs by job name parameters\n  (test name, config, start-block, runner)\n- Iterate candidate runs oldest-first and verify inputs match\n- Increase run list limit from 5 to 10 for better coverage\n- Document dependency on AvalancheGo's job naming convention\n\nThis partially handles concurrent triggers: runs triggered seconds apart\nare now correctly identified. Same-second collisions remain ambiguous\nbut are rare in practice.",
          "timestamp": "2026-01-25T14:06:30Z",
          "url": "https://github.com/ava-labs/firewood/commit/e19e9510812ba2ac7df19b84296b4bc67783b7a8"
        },
        "date": 1769350675809,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[1,100]-Config-firewood-Runner-avalanche-avalanchego-runner-2ti - mgas/s",
            "value": 990.7323405320444,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[1,100]-Config-firewood-Runner-avalanche-avalanchego-runner-2ti - ms/ggas",
            "value": 1009.3543524207341,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[1,100]-Config-firewood-Runner-avalanche-avalanchego-runner-2ti - block_parse_ms/ggas",
            "value": 81.88533894991315,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[1,100]-Config-firewood-Runner-avalanche-avalanchego-runner-2ti - block_verify_ms/ggas",
            "value": 792.5684374580309,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[1,100]-Config-firewood-Runner-avalanche-avalanchego-runner-2ti - block_accept_ms/ggas",
            "value": 131.47544028985476,
            "unit": "block_accept_ms/ggas"
          }
        ]
      }
    ]
  }
}