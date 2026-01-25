window.BENCHMARK_DATA = {
  "lastUpdate": 1769353432451,
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
      },
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
          "id": "bfc7c8a4a8ac770d2a8d0aac088e2d89775dcd33",
          "message": "ci(gh-pages): preserve benchmark data when deploying docs\n\nGitHub Pages overwrites on each deploy, which would lose benchmark\nhistory stored on the benchmark-data branch. This adds a step to\nmerge benchmark directories (bench/, dev/) into the Pages deployment\nso both docs and performance charts are served together.\n\n- Fetch benchmark-data branch during docs build\n- Extract bench/ (main) and dev/ (feature branches) directories\n- Copy to _site/ before deployment\n- Handle bootstrap case when no benchmarks exist yet",
          "timestamp": "2026-01-25T14:56:51Z",
          "url": "https://github.com/ava-labs/firewood/commit/bfc7c8a4a8ac770d2a8d0aac088e2d89775dcd33"
        },
        "date": 1769353431698,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[1,100]-Config-firewood-Runner-avalanche-avalanchego-runner-2ti - mgas/s",
            "value": 914.8258330625005,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[1,100]-Config-firewood-Runner-avalanche-avalanchego-runner-2ti - ms/ggas",
            "value": 1093.104243298823,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[1,100]-Config-firewood-Runner-avalanche-avalanchego-runner-2ti - block_parse_ms/ggas",
            "value": 81.48662458341437,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[1,100]-Config-firewood-Runner-avalanche-avalanchego-runner-2ti - block_verify_ms/ggas",
            "value": 870.4234013627982,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[1,100]-Config-firewood-Runner-avalanche-avalanchego-runner-2ti - block_accept_ms/ggas",
            "value": 137.78709336697068,
            "unit": "block_accept_ms/ggas"
          }
        ]
      }
    ]
  }
}