window.BENCHMARK_DATA = {
  "lastUpdate": 1769711872509,
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
          "id": "d5b9fca96e30cce1e984343f4761b3a04f4014c7",
          "message": "ci(track-performance): remove JSON configs, simplify benchmark workflow\n\n- Remove `.github/benchmark-schedules.json` in favor of inline configurations.\n- Replace matrix-based strategy with direct output handling for cleaner and more explicit workflow logic.\n- Preserve manual and scheduled benchmark support with optimized input handling.",
          "timestamp": "2026-01-28T18:23:05Z",
          "url": "https://github.com/ava-labs/firewood/commit/d5b9fca96e30cce1e984343f4761b3a04f4014c7"
        },
        "date": 1769628311704,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avalanche-avalanchego-runner-2ti - mgas/s",
            "value": 116.63831163250312,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avalanche-avalanchego-runner-2ti - ms/ggas",
            "value": 8573.512304865479,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avalanche-avalanchego-runner-2ti - block_parse_ms/ggas",
            "value": 351.54217936973646,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avalanche-avalanchego-runner-2ti - block_verify_ms/ggas",
            "value": 7565.670205179929,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avalanche-avalanchego-runner-2ti - block_accept_ms/ggas",
            "value": 635.5436604246512,
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
          "id": "0843424f2452f8942f4b7d2d0c10cb80fc21e464",
          "message": "docs",
          "timestamp": "2026-01-28T19:38:41Z",
          "url": "https://github.com/ava-labs/firewood/commit/0843424f2452f8942f4b7d2d0c10cb80fc21e464"
        },
        "date": 1769711872235,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 108.80969561255482,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 9190.357480281535,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 456.2204092633949,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 7975.911733339908,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 735.6096801629008,
            "unit": "block_accept_ms/ggas"
          }
        ]
      }
    ]
  }
}