window.BENCHMARK_DATA = {
  "lastUpdate": 1787604236803,
  "repoUrl": "https://github.com/ava-labs/firewood",
  "entries": {
    "Rust Microbenchmarks": [
      {
        "commit": {
          "author": {
            "name": "rodrigo.villar",
            "username": "RodrigoVillar",
            "email": "rodrigojvillarg@gmail.com"
          },
          "committer": {
            "name": "rodrigo.villar",
            "username": "RodrigoVillar",
            "email": "rodrigojvillarg@gmail.com"
          },
          "id": "aa1b32da72c72eb89872ffdcb86de9d738b2dd65",
          "message": "ci: publish Rust benchmarks to the C-Chain benchmark page on the same cron cadence\n\nWhy this should be merged: the Criterion benches only uploaded an artifact,\nso there was no trend history and no way to correlate them with the C-Chain\nreexecution numbers.\n\nHow this works: benchmarks.yaml now emits bencher-format output and publishes\nit with github-action-benchmark into the same benchmark-data directory the\nC-Chain workflow uses (bench on main, dev/bench/{branch} otherwise) under a\ndistinct name, so the charts land as an extra section on the same page. Triggers\nmove from push-to-main to a weekday 05:10 UTC cron; a shared, non-cancelling\nconcurrency group serializes publishes against the same data.js. gh-pages also\nredeploys after firewood-benchmarks.\n\nCo-Authored-By: Devin AI <158243242+devin-ai-integration[bot]@users.noreply.github.com>",
          "timestamp": "2026-08-24T19:55:24Z",
          "url": "https://github.com/ava-labs/firewood/commit/aa1b32da72c72eb89872ffdcb86de9d738b2dd65"
        },
        "date": 1787602153665,
        "tool": "cargo",
        "benches": [
          {
            "name": "Merkle/insert",
            "value": 2979,
            "range": "± 25",
            "unit": "ns/iter"
          },
          {
            "name": "Merkle/insert #2",
            "value": 4972,
            "range": "± 75",
            "unit": "ns/iter"
          },
          {
            "name": "Db/commit",
            "value": 2089785,
            "range": "± 348858",
            "unit": "ns/iter"
          },
          {
            "name": "deferred_persistence/commit_count_1",
            "value": 32699404,
            "range": "± 3377265",
            "unit": "ns/iter"
          },
          {
            "name": "deferred_persistence/commit_count_10",
            "value": 21032991,
            "range": "± 421866",
            "unit": "ns/iter"
          },
          {
            "name": "deferred_persistence/commit_count_100",
            "value": 15381238,
            "range": "± 231228",
            "unit": "ns/iter"
          },
          {
            "name": "deferred_persistence/commit_count_1000",
            "value": 14553221,
            "range": "± 190333",
            "unit": "ns/iter"
          },
          {
            "name": "leaf/manual",
            "value": 61,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "leaf/from_reader",
            "value": 63,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "has_value/manual",
            "value": 107,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "has_value/from_reader",
            "value": 215,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "1_child/manual",
            "value": 86,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "1_child/from_reader",
            "value": 192,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "2_child/manual",
            "value": 91,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "2_child/from_reader",
            "value": 234,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "16_child/manual",
            "value": 173,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "16_child/from_reader",
            "value": 832,
            "range": "± 5",
            "unit": "ns/iter"
          }
        ]
      }
    ],
    "C-Chain Reexecution with Firewood": [
      {
        "commit": {
          "author": {
            "name": "rodrigo.villar",
            "username": "RodrigoVillar",
            "email": "rodrigojvillarg@gmail.com"
          },
          "committer": {
            "name": "rodrigo.villar",
            "username": "RodrigoVillar",
            "email": "rodrigojvillarg@gmail.com"
          },
          "id": "aa1b32da72c72eb89872ffdcb86de9d738b2dd65",
          "message": "ci: publish Rust benchmarks to the C-Chain benchmark page on the same cron cadence\n\nWhy this should be merged: the Criterion benches only uploaded an artifact,\nso there was no trend history and no way to correlate them with the C-Chain\nreexecution numbers.\n\nHow this works: benchmarks.yaml now emits bencher-format output and publishes\nit with github-action-benchmark into the same benchmark-data directory the\nC-Chain workflow uses (bench on main, dev/bench/{branch} otherwise) under a\ndistinct name, so the charts land as an extra section on the same page. Triggers\nmove from push-to-main to a weekday 05:10 UTC cron; a shared, non-cancelling\nconcurrency group serializes publishes against the same data.js. gh-pages also\nredeploys after firewood-benchmarks.\n\nCo-Authored-By: Devin AI <158243242+devin-ai-integration[bot]@users.noreply.github.com>",
          "timestamp": "2026-08-24T19:55:24Z",
          "url": "https://github.com/ava-labs/firewood/commit/aa1b32da72c72eb89872ffdcb86de9d738b2dd65"
        },
        "date": 1787604235390,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avalanche-avalanchego-runner-2ti - mgas/s",
            "value": 115.37773676678351,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avalanche-avalanchego-runner-2ti - ms/ggas",
            "value": 8667.183358097325,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avalanche-avalanchego-runner-2ti - block_parse_ms/ggas",
            "value": 444.4314643279584,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avalanche-avalanchego-runner-2ti - block_verify_ms/ggas",
            "value": 7479.061160388931,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avalanche-avalanchego-runner-2ti - block_accept_ms/ggas",
            "value": 719.7775900472496,
            "unit": "block_accept_ms/ggas"
          }
        ]
      }
    ]
  }
}