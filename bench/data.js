window.BENCHMARK_DATA = {
  "lastUpdate": 1769802936938,
  "repoUrl": "https://github.com/ava-labs/firewood",
  "entries": {
    "C-Chain Reexecution with Firewood": [
      {
        "commit": {
          "author": {
            "name": "bernard-avalabs",
            "username": "bernard-avalabs",
            "email": "53795885+bernard-avalabs@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "718ae46a3883e7f0564181fdccc485e5c099ecb8",
          "message": "chore: Add Bernard as codeowner (#1647)\n\n## Why this should be merged\n\nAdding myself to codeowners to help with code review. @rkuris also\nsuggested that the \"/.github\" be removed.\n\n## How this works\n\nAdded @bernard-avalabs to \"*\" and \"/ffi\" and removed \"/.github\"\n\n## How this was tested\n\nCI",
          "timestamp": "2026-01-30T17:27:59Z",
          "url": "https://github.com/ava-labs/firewood/commit/718ae46a3883e7f0564181fdccc485e5c099ecb8"
        },
        "date": 1769802936441,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avalanche-avalanchego-runner-2ti - mgas/s",
            "value": 103.55308243832812,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avalanche-avalanchego-runner-2ti - ms/ggas",
            "value": 9656.882986516197,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avalanche-avalanchego-runner-2ti - block_parse_ms/ggas",
            "value": 406.37812496970713,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avalanche-avalanchego-runner-2ti - block_verify_ms/ggas",
            "value": 8505.896964239892,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avalanche-avalanchego-runner-2ti - block_accept_ms/ggas",
            "value": 721.2011097869388,
            "unit": "block_accept_ms/ggas"
          }
        ]
      }
    ]
  }
}