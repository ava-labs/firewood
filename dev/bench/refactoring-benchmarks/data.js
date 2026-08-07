window.BENCHMARK_DATA = {
  "lastUpdate": 1786111071427,
  "repoUrl": "https://github.com/ava-labs/firewood",
  "entries": {
    "Rust Benchmarks": [
      {
        "commit": {
          "author": {
            "name": "Rodrigo Villar",
            "username": "RodrigoVillar",
            "email": "rodrigo.villar@avalabs.org"
          },
          "committer": {
            "name": "Rodrigo Villar",
            "username": "RodrigoVillar",
            "email": "rodrigo.villar@avalabs.org"
          },
          "id": "4024ecd401d4b79854011013beb241aa9ebbdabf",
          "message": "ci: publish Rust benchmarks to GH page\n\nChange-Id: Iad06b3ebb459bf0d97ead16b6f14689f7da44966",
          "timestamp": "2026-08-07T13:48:35Z",
          "url": "https://github.com/ava-labs/firewood/commit/4024ecd401d4b79854011013beb241aa9ebbdabf"
        },
        "date": 1786111069907,
        "tool": "cargo",
        "benches": [
          {
            "name": "Merkle/insert",
            "value": 4148,
            "range": "± 88",
            "unit": "ns/iter"
          },
          {
            "name": "Merkle/insert #2",
            "value": 7133,
            "range": "± 118",
            "unit": "ns/iter"
          },
          {
            "name": "Db/commit",
            "value": 2368188,
            "range": "± 358772",
            "unit": "ns/iter"
          },
          {
            "name": "deferred_persistence/commit_count_1",
            "value": 37601755,
            "range": "± 2145066",
            "unit": "ns/iter"
          },
          {
            "name": "deferred_persistence/commit_count_10",
            "value": 20795886,
            "range": "± 243632",
            "unit": "ns/iter"
          },
          {
            "name": "deferred_persistence/commit_count_100",
            "value": 16069071,
            "range": "± 212752",
            "unit": "ns/iter"
          },
          {
            "name": "deferred_persistence/commit_count_1000",
            "value": 14838332,
            "range": "± 308960",
            "unit": "ns/iter"
          },
          {
            "name": "leaf/manual",
            "value": 59,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "leaf/from_reader",
            "value": 64,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "has_value/manual",
            "value": 112,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "has_value/from_reader",
            "value": 220,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "1_child/manual",
            "value": 83,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "1_child/from_reader",
            "value": 243,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "2_child/manual",
            "value": 108,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "2_child/from_reader",
            "value": 226,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "16_child/manual",
            "value": 170,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "16_child/from_reader",
            "value": 728,
            "range": "± 4",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}