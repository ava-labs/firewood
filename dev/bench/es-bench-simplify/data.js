window.BENCHMARK_DATA = {
  "lastUpdate": 1773760028491,
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
          "id": "0d19dc2f2d81b8d9ec90a5e6822e8b63d00469a3",
          "message": "feat(benchmark): add firewood-40m-41m test and harden bench-cchain\n\nUse named test in daily schedule, fix runner default, add unpushed\ncommits guard, default timeout, and improve timeout error messages.",
          "timestamp": "2026-03-17T14:48:00Z",
          "url": "https://github.com/ava-labs/firewood/commit/0d19dc2f2d81b8d9ec90a5e6822e8b63d00469a3"
        },
        "date": 1773760028071,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 105.89746145189261,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 9443.096994863117,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 474.55304825100814,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 8230.839336790286,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 715.01540767677,
            "unit": "block_accept_ms/ggas"
          }
        ]
      }
    ]
  }
}