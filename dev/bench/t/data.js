window.BENCHMARK_DATA = {
  "lastUpdate": 1773500391303,
  "repoUrl": "https://github.com/ava-labs/firewood",
  "entries": {
    "C-Chain Reexecution with Firewood": [
      {
        "commit": {
          "author": {
            "name": "Elvis",
            "username": "Elvis339",
            "email": "43846394+Elvis339@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "5406b6858e002d783b5a9c32db9b49fd18291c0b",
          "message": "chore(revert): remove nix flake and all references to it (#1788) (#1794)\n\n## Why this should be merged\n\nThe nix flake is still required by the AvalancheGo `polyrepo` build\ntool. Removing it breaks polyrepo which relies on the flake to build\nFirewood and set up the testing infrastructure used for scheduled daily\nbenchmarks, experiments, and A/B testing.\n\nThis revert restores that functionality until the migration to\nAvalancheGo is complete and the flake can be safely removed as part of\nthat transition.\n\n## How this works\n\nThis reverts commit 76a2c8c17e640604ce82d11b5a2779c9dbdd7c04.\n\n## How this was tested\n```\nTEST=firewood-101-250k FIREWOOD_REF=es/revert-remove-nix-flake just bench-cchain\n```\n- Proves just bench-cchain works\n- Successful build at FIREWOOD_REF\nhttps://github.com/ava-labs/avalanchego/actions/runs/23064572050/job/66999562936#step:5:257\n\n## Breaking Changes\n\n- [ ] firewood\n- [ ] firewood-storage\n- [ ] firewood-ffi (C api)\n- [ ] firewood-go (Go api)\n- [ ] fwdctl\n\n---------\n\nCo-authored-by: maru <maru.newby@avalabs.org>",
          "timestamp": "2026-03-14T12:43:41Z",
          "url": "https://github.com/ava-labs/firewood/commit/5406b6858e002d783b5a9c32db9b49fd18291c0b"
        },
        "date": 1773500390956,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 149.4695754975334,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 6690.32474783808,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 109.54098538667233,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 6503.776239395382,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 74.65862391865036,
            "unit": "block_accept_ms/ggas"
          }
        ]
      }
    ]
  }
}