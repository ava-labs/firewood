window.BENCHMARK_DATA = {
  "lastUpdate": 1770858164392,
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
          "id": "f81e3eaf2b5a2a88e89f78e28eaefa5b7546748e",
          "message": "feat: defer persist every 'N' commits",
          "timestamp": "2026-02-10T23:11:18Z",
          "url": "https://github.com/ava-labs/firewood/commit/f81e3eaf2b5a2a88e89f78e28eaefa5b7546748e"
        },
        "date": 1770858163533,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 110.72776483770225,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 9031.158548768113,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 451.8172716602097,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 7840.463058069704,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[101,250000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 717.8581011709277,
            "unit": "block_accept_ms/ggas"
          }
        ]
      }
    ]
  }
}