window.BENCHMARK_DATA = {
  "lastUpdate": 1775530084692,
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
          "id": "e4aae6790079b15062e7bc59cc8b9a82513817b4",
          "message": "refactor(ffi): remove ArcCache from DatabaseHandle\n\nArcCache was introduced to avoid redundant Db::view() calls during\ncommit by caching the proposal view before committing and clearing it\nafter. Due to subsequent API changes the cache has a 100% miss rate,\nso it provides no benefit while serializing all concurrent FFI view\nlookups behind a Mutex held across the Db::view() factory call.\n\nReplace get_root() with a thin view() method that delegates directly\nto Db::view(), remove clear_cached_view() and the surrounding\ncommit_proposal() bookkeeping, and delete arc_cache.rs along with\nthe CACHED_VIEW_MISS/HIT metrics.",
          "timestamp": "2026-04-06T20:43:48Z",
          "url": "https://github.com/ava-labs/firewood/commit/e4aae6790079b15062e7bc59cc8b9a82513817b4"
        },
        "date": 1775530084409,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 168.69577063090028,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 5927.830889062186,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 110.81818170908966,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 5736.588346848956,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 77.76757938074645,
            "unit": "block_accept_ms/ggas"
          }
        ]
      }
    ]
  }
}