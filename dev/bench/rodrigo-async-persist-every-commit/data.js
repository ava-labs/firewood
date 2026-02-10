window.BENCHMARK_DATA = {
  "lastUpdate": 1770762383849,
  "repoUrl": "https://github.com/ava-labs/firewood",
  "entries": {
    "C-Chain Reexecution with Firewood": [
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
          "id": "7429149c343fdd239d65817442b11ca6b005c676",
          "message": "refactor: shutdown logic",
          "timestamp": "2026-02-10T21:51:37Z",
          "url": "https://github.com/ava-labs/firewood/commit/7429149c343fdd239d65817442b11ca6b005c676"
        },
        "date": 1770762382801,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 110.78094332348692,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 9026.8232964937,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 451.2175084505496,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 7830.47625873533,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 723.3242030966074,
            "unit": "block_accept_ms/ggas"
          }
        ]
      }
    ]
  }
}