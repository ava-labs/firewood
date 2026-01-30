window.BENCHMARK_DATA = {
  "lastUpdate": 1769799945966,
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
          "id": "d028428a24a1be2f331bc682a4a43a9b9cb064b7",
          "message": "ci(track-performance): remove preset runner options",
          "timestamp": "2026-01-29T19:29:14Z",
          "url": "https://github.com/ava-labs/firewood/commit/d028428a24a1be2f331bc682a4a43a9b9cb064b7"
        },
        "date": 1769715851453,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 108.07331734095278,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 9252.977743295982,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 462.05494887583995,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 8024.646865876763,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 743.2626064300606,
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
          "id": "42a1d9d05eb856d247cfbddcdc5960970334a013",
          "message": "ci(gh-pages): repository_dispatch",
          "timestamp": "2026-01-29T20:00:35Z",
          "url": "https://github.com/ava-labs/firewood/commit/42a1d9d05eb856d247cfbddcdc5960970334a013"
        },
        "date": 1769717768471,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 109.93907641110958,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 9095.946888443641,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 451.9697902569987,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 7893.3045825166155,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 728.0097779136032,
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
          "id": "07ce5ca5ff69939aebbcf20a9aace89516a4ee35",
          "message": "test",
          "timestamp": "2026-01-29T20:22:29Z",
          "url": "https://github.com/ava-labs/firewood/commit/07ce5ca5ff69939aebbcf20a9aace89516a4ee35"
        },
        "date": 1769718870809,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 109.1275692450096,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 9163.587230233572,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 461.3560722443497,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 7942.648189857963,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 736.3434911719924,
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
          "id": "22e2be1338e2d3f28b8840b39f33d20d795a2ee7",
          "message": "ci(github-pages): auto rebuild benchmarks when results are pushed",
          "timestamp": "2026-01-30T18:29:28Z",
          "url": "https://github.com/ava-labs/firewood/commit/22e2be1338e2d3f28b8840b39f33d20d795a2ee7"
        },
        "date": 1769798697410,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 110.5280748007236,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 9047.475058286758,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 448.74784140235846,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 7851.926650204512,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 724.1648426782667,
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
          "id": "a44160d23ac32c3be457ef719add2b5742c537a8",
          "message": "ci(gh-pages): auto deploy",
          "timestamp": "2026-01-30T18:53:11Z",
          "url": "https://github.com/ava-labs/firewood/commit/a44160d23ac32c3be457ef719add2b5742c537a8"
        },
        "date": 1769799945213,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 108.78673675299297,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 9192.297056125159,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 463.9912738454248,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 7963.594776606748,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 742.0582175613531,
            "unit": "block_accept_ms/ggas"
          }
        ]
      }
    ]
  }
}