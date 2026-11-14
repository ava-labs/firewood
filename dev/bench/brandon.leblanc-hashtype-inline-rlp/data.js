window.BENCHMARK_DATA = {
  "lastUpdate": 1794619940772,
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
          "id": "6e8d74811d98f17163dfc7292292f34ddcc48c01",
          "message": "refactor(storage): derive Copy for TrieHash and HashType\n\nBoth are plain bytes: TrieHash is [u8; 32] and, with InlineRlp, HashType is 33\nbytes with no heap. Deriving Copy lets a hash be passed and stored by value\nwithout a clone call.\n\nEvery other change is clippy's clone_on_copy fix applied by cargo clippy --fix:\nclone() and cloned() on a hash become a copy or a dereference. No behavior\nchanges.",
          "timestamp": "2026-11-13T23:15:19Z",
          "url": "https://github.com/ava-labs/firewood/commit/6e8d74811d98f17163dfc7292292f34ddcc48c01"
        },
        "date": 1794619434858,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 159.5003720323225,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 6269.577852754798,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 112.31340623090009,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 5.83584918504093,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 6146.854834622597,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
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
          "id": "6e8d74811d98f17163dfc7292292f34ddcc48c01",
          "message": "refactor(storage): derive Copy for TrieHash and HashType\n\nBoth are plain bytes: TrieHash is [u8; 32] and, with InlineRlp, HashType is 33\nbytes with no heap. Deriving Copy lets a hash be passed and stored by value\nwithout a clone call.\n\nEvery other change is clippy's clone_on_copy fix applied by cargo clippy --fix:\nclone() and cloned() on a hash become a copy or a dereference. No behavior\nchanges.",
          "timestamp": "2026-11-13T23:15:19Z",
          "url": "https://github.com/ava-labs/firewood/commit/6e8d74811d98f17163dfc7292292f34ddcc48c01"
        },
        "date": 1794619690206,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 154.00747796806206,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 6493.191195607912,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 122.58807943149695,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 6.735640716472667,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 6357.5425213926255,
            "unit": "block_accept_ms/ggas"
          }
        ]
      },
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
          "id": "6e8d74811d98f17163dfc7292292f34ddcc48c01",
          "message": "refactor(storage): derive Copy for TrieHash and HashType\n\nBoth are plain bytes: TrieHash is [u8; 32] and, with InlineRlp, HashType is 33\nbytes with no heap. Deriving Copy lets a hash be passed and stored by value\nwithout a clone call.\n\nEvery other change is clippy's clone_on_copy fix applied by cargo clippy --fix:\nclone() and cloned() on a hash become a copy or a dereference. No behavior\nchanges.",
          "timestamp": "2026-11-13T23:15:19Z",
          "url": "https://github.com/ava-labs/firewood/commit/6e8d74811d98f17163dfc7292292f34ddcc48c01"
        },
        "date": 1794619940062,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 160.23238721471222,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 6240.935539829379,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 103.70275752415343,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 5.23527539109484,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 6128.273126657047,
            "unit": "block_accept_ms/ggas"
          }
        ]
      }
    ]
  }
}