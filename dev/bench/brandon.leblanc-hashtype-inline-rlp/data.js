window.BENCHMARK_DATA = {
  "lastUpdate": 1794953887860,
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
          "id": "6ece35c8204b76e28469ed1e1a4185777cfc6a3b",
          "message": "perf(storage)!: store inline RLP in a fixed 32-byte InlineRlp\n\n`HashType::Rlp` held a `SmallVec` even though an inline encoding is never\nlonger than 31 bytes. `InlineRlp` stores a length byte and a 31-byte array,\nso `HashType` is 33 bytes with no heap allocation and derives `Copy`, as\ndoes `TrieHash`; clones of either across the workspace become copies.\n\n`InlineRlp` holds 1 to 31 bytes. The hasher falls back to a full hash for\nanything longer, as before. `HashType::from_reader` now rejects a stored\ninline length of 32, which it accepted before; the hasher never wrote one.\nThe proof decoder rejects inline children of 0 or more than 31 bytes, with\n`InvalidInlineRlpLength` for construction from a slice. Writers and the\non-disk and proof formats are unchanged.\n\nBREAKING CHANGE: `firewood-storage`'s `HashType::Rlp` holds an `InlineRlp`\nand `HashType` and `TrieHash` are `Copy`. `firewood`'s\n`ProofNode::child_hashes` element type changes with it.",
          "timestamp": "2026-11-17T18:35:24Z",
          "url": "https://github.com/ava-labs/firewood/commit/6ece35c8204b76e28469ed1e1a4185777cfc6a3b"
        },
        "date": 1794953886974,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 167.62156329315724,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 5965.819554200654,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 105.5285647584838,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 5.441866015791137,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 5851.130405001892,
            "unit": "block_accept_ms/ggas"
          }
        ]
      }
    ]
  }
}