window.BENCHMARK_DATA = {
  "lastUpdate": 1775542311455,
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
          "id": "4851d8ab66a7a962b0527f1c7c0ac0762612c592",
          "message": "perf(manager): decouple reader stall from commit backpressure\n\n`commit()` previously held `in_memory_revisions.write()` across all five\nsteps of the critical section, including `persist_worker.persist()` which\nparks the commit thread under backpressure. Every concurrent caller of\n`current_revision()` or `view()` was blocked for the full I/O stall\nduration — potentially hundreds of milliseconds.\n\nIntroduce `commit_lock: Mutex<()>` to serialize concurrent commits for\nthe entire duration of `commit()` without touching `in_memory_revisions`.\nThe backing fields are now held only for brief, non-blocking windows:\n\n- Step 2 (parent check): lock → read back() → unlock\n- Step 3 (reaping): lock → pop_front + hash remove → unlock → reap()\n- Step 4 (persist): no lock held; readers observe the previous revision\n- Step 5 (publish): lock → push_back + hash insert → unlock\n\nBecause `commit_lock` serializes all writers, `in_memory_revisions` and\n`by_hash` are downgraded from `RwLock` to `Mutex`: every critical section\nis nanoseconds (deque/map ops + Arc clone), making concurrent read access\nindistinguishable from serialized access. `RwLock` is removed from the\nimport entirely.",
          "timestamp": "2026-04-07T02:32:50Z",
          "url": "https://github.com/ava-labs/firewood/commit/4851d8ab66a7a962b0527f1c7c0ac0762612c592"
        },
        "date": 1775536518539,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 169.4594957244566,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 5901.115164570143,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 108.23155254075598,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 5716.001474080073,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 74.53929988449408,
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
          "id": "4851d8ab66a7a962b0527f1c7c0ac0762612c592",
          "message": "perf(manager): decouple reader stall from commit backpressure\n\n`commit()` previously held `in_memory_revisions.write()` across all five\nsteps of the critical section, including `persist_worker.persist()` which\nparks the commit thread under backpressure. Every concurrent caller of\n`current_revision()` or `view()` was blocked for the full I/O stall\nduration — potentially hundreds of milliseconds.\n\nIntroduce `commit_lock: Mutex<()>` to serialize concurrent commits for\nthe entire duration of `commit()` without touching `in_memory_revisions`.\nThe backing fields are now held only for brief, non-blocking windows:\n\n- Step 2 (parent check): lock → read back() → unlock\n- Step 3 (reaping): lock → pop_front + hash remove → unlock → reap()\n- Step 4 (persist): no lock held; readers observe the previous revision\n- Step 5 (publish): lock → push_back + hash insert → unlock\n\nBecause `commit_lock` serializes all writers, `in_memory_revisions` and\n`by_hash` are downgraded from `RwLock` to `Mutex`: every critical section\nis nanoseconds (deque/map ops + Arc clone), making concurrent read access\nindistinguishable from serialized access. `RwLock` is removed from the\nimport entirely.",
          "timestamp": "2026-04-07T02:32:50Z",
          "url": "https://github.com/ava-labs/firewood/commit/4851d8ab66a7a962b0527f1c7c0ac0762612c592"
        },
        "date": 1775542310514,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - mgas/s",
            "value": 154.67488851323515,
            "unit": "mgas/s"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - ms/ggas",
            "value": 6465.173562510326,
            "unit": "ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_parse_ms/ggas",
            "value": 118.33943536345016,
            "unit": "block_parse_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_verify_ms/ggas",
            "value": 6255.8359756249465,
            "unit": "block_verify_ms/ggas"
          },
          {
            "name": "BenchmarkReexecuteRange/[40000001,41000000]-Config-firewood-Runner-avago-runner-i4i-2xlarge-local-ssd - block_accept_ms/ggas",
            "value": 87.25335607977237,
            "unit": "block_accept_ms/ggas"
          }
        ]
      }
    ]
  }
}