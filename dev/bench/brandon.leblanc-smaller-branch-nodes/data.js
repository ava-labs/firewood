window.BENCHMARK_DATA = {
  "lastUpdate": 1794612900880,
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
          "id": "20bf9df3a4d47e45e9d62b1189b1eaa339628bfc",
          "message": "test(firewood): add get and remove merkle benchmarks\n\nThe existing Merkle benchmarks insert three keys, which builds one branch and\ncannot show the cost of looking up a child or of a branch gaining or losing\none. MerkleLarge builds a 10,000-key trie and times reading every key from the\nhashed trie and removing every key from the unhashed one.\n\nThey land before BranchNode changes layout so the dense layout has a baseline.\n\nMerkle::get_value and Merkle::remove become pub so the bench can call them.\nMerkle is exported only under the test_utils feature, as insert already\nassumes, so the default public API is unchanged.",
          "timestamp": "2026-11-13T21:45:07Z",
          "url": "https://github.com/ava-labs/firewood/commit/20bf9df3a4d47e45e9d62b1189b1eaa339628bfc"
        },
        "date": 1794612580814,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 166.35460474965276,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 6011.255303121312,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 103.18046115509985,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 5.2100663796992075,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 5899.223185773506,
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
          "id": "20bf9df3a4d47e45e9d62b1189b1eaa339628bfc",
          "message": "test(firewood): add get and remove merkle benchmarks\n\nThe existing Merkle benchmarks insert three keys, which builds one branch and\ncannot show the cost of looking up a child or of a branch gaining or losing\none. MerkleLarge builds a 10,000-key trie and times reading every key from the\nhashed trie and removing every key from the unhashed one.\n\nThey land before BranchNode changes layout so the dense layout has a baseline.\n\nMerkle::get_value and Merkle::remove become pub so the bench can call them.\nMerkle is exported only under the test_utils feature, as insert already\nassumes, so the default public API is unchanged.",
          "timestamp": "2026-11-13T21:45:07Z",
          "url": "https://github.com/ava-labs/firewood/commit/20bf9df3a4d47e45e9d62b1189b1eaa339628bfc"
        },
        "date": 1794612900072,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 158.39246502105357,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 6313.431638727762,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 116.1339148047864,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 6.142307407851713,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 6186.100078467908,
            "unit": "block_accept_ms/ggas"
          }
        ]
      }
    ]
  }
}