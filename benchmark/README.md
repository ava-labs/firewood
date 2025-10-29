# Firewood Benchmark

Welcome to the Firewood Benchmark! This repository contains the benchmarking
code and resources for the Firewood project.

## Table of Contents

- [Introduction](#introduction)
- [Benchmark Description](#benchmark-description)
- [Installation](#installation)
- [Usage](#usage)
- [Understanding Metrics](#understanding-metrics)

## Introduction

The Firewood Benchmark is a performance testing suite designed to evaluate the performance of the Firewood project. It includes a set of benchmarks that measure various aspects of the project's performance, such as execution time, memory usage, and scalability.

## Benchmark Description

There are currently three different benchmarks, as follows:

1. `tenkrandom` which does transactions of size 10k, 5k updates, 2.5k inserts, and 2.5k deletes
2. `zipf` which uses a zipf distribution across the database to perform updates
3. `single` which only updates a single row (row 1) repeatedly in a tiny transaction

There is also a `create` benchmark which creates the database to begin with. The defaults will create
a 10M row database. If you want a larger one, increase the number of batches.

## Benchmark Specification

This describes each benchmark in detail, and forms a standard for how to test merkle databases. The pseudo-code for each
benchmark is provided.

Note that the `commit` operation is expected to persist the database and compute the root hash for each merkle.

### create

Inserts are done starting from an empty database, 10,000 rows at a time, starting with row id 0 and increasing by 1 for each row.
The key and data consist of the SHA-256 of the row id, in native endian format.

```rust
  for key in (0..N) {
    let key_and_data = sha256(key.to_ne_bytes());
    testdb.insert(key_and_data, key_and_data);
    if key % 10000 == 9999 {
      testdb.commit();
    }
  }
```

Exception: when creating the 1B row database, 100,000 rows at a time are added.

### tenkrandom

This test uses two variables, low and high, which initially start off at 0 and N, where N is the number of rows in the database.
A batch consists of deleting 2,500 rows from low to low+2499, 2,500 new insertions from high to high+2499, and 5,000 updates from (low+high)/2 to (low+high)/2+4999. The key is computed based on the sha256 as when the database is created, but the data is set to the sha256 of the value of 'low', to ensure that the data is updated. Once a batch is completed, low and high are increased by 2,500.

```rust
  let low = 0;
  let high = N;
  loop {
    let hashed_low = sha256(low.to_ne_bytes());
    for key in (low..low+2499) {
      testdb.delete(hashed_key);
      let hashed_key = sha256(key.to_ne_bytes());
    }
    for key in (high..high+2499) {
      let hashed_key = sha256(key.to_ne_bytes());
      testdb.insert(hashed_key, hashed_low);
    }
    for key in ((low+high)/2, (low+high)/2+4999) {
      let hashed_key = sha256(key.to_ne_bytes());
      testdb.upsert(hashed_key, hashed_low);
    }
    testdb.commit();
  }
```

### zipf

A zipf distribution with an exponent of 1.2 on the total number of inserted rows is used to compute which rows to update with a batch of 10,000 rows. Note that this results in duplicates -- the duplicates are passed to the database for resolution.

```rust
   for id in (0..N) {
    let key = zipf(1.2, N);
    let hashed_key = sha256(key.to_ne_bytes());
    let hashed_data = sha256(id.to_ne_bytes());
    testdb.upsert(hashed_key, hashed_data);
    if id % 10000 == 9999 {
      testdb.commit();
    }
   }
  
```

### single

This test repeatedly updates the first row.

```rust
  let hashed_key = sha256(0.to_ne_bytes());
  for id in (0..) {
    let hashed_data = sha256(id.to_ne_bytes());
    testdb.upsert(hashed_key, hashed_data);
    testdb.commit(); 
  }
```

## Installation

To install the Firewood Benchmark, follow these steps:

1. Install the build prerequisites: `sudo bash setup-scripts/build-environment.sh`
2. \[Optional\] Install grafana: `sudo bash setup-scripts/install-grafana.sh`
3. Build firewood: `bash setup-scripts/build-firewood.sh`
4. Create the benchmark database: `nohup time cargo run --profile maxperf --bin benchmark -- create`. For a larger database, add `--number-of-batches=10000` before the subcommand 'create' for a 100M row database (each batch by default is 10K rows). Additional options are documented in setup-scripts/run-benchmarks.sh

If you want to install grafana and prometheus on an AWS host (using Ubuntu as a base), do the following:

1. Log in to the AWS EC2 console
2. Launch a new instance:
   1. Name: Firewood Benchmark
   2. Click on 'Ubuntu' in the quick start section
   3. Set the instance type to m5d.2xlarge
   4. Set your key pair
   5. Open SSH, HTTP, and port 3000 so prometheus can be probed remotely (we use a 'firewood-bench' security group)
   6. Configure storage to 20GiB disk
   7. \[optional] Save money by selecting 'spot instance' in advanced
   8. Launch the instance
3. ssh ubuntu@AWS-IP
4. Run the scripts as described above, including the grafana installation.
5. Log in to Grafana at <http://YOUR-AWS-IP>
   a. Username: `admin`, password: `firewood_is_fast`
6. A Prometheus data source, the Firewood dashboard, and a [system statistics dashboard](https://grafana.com/grafana/dashboards/1860-node-exporter-full/) are preconfigured and ready to use.

### Updating the dashboard

If you want to update the dashboard and commit it, do not enable "Export for sharing externally" when exporting. These dashboards are provisioned and reference the default Prometheus data source, with the option to pick from available data sources.

## Usage

Since the benchmark is in two phases, you may want to create the database first and then
examine the steady-state performance second. This can easily be accomplished with a few
command line options.

To create test databases, use the following command:

```sh
    nohup time cargo run --profile maxperf --bin benchmark -- -n 10000 create
```

Then, you can look at nohup.out and see how long the database took to initialize. Then, to run
the second phase, use:

```sh
    nohup time cargo run --profile maxperf --bin benchmark -- -n 10000 zipf
```

If you're looking for detailed logging, there are some command line options to enable it. For example, to enable debug logging for the single benchmark, you can use the following:

```sh
    cargo run --profile release --bin benchmark -- -l debug -n 10000 single
```

## Using opentelemetry

To use the opentelemetry server and record timings, just run a docker image that collects the data using:

```sh
docker run   -p 127.0.0.1:4318:4318   -p 127.0.0.1:55679:55679   otel/opentelemetry-collector-contrib:0.97.0   2>&1
```

Then, pass the `-e` option to the benchmark.

## Understanding Metrics

This section explains the metrics collected during benchmarks, how to interpret them, and how to identify performance characteristics and bottlenecks.

### Overview

Firewood exposes Prometheus metrics that are visualized in the Grafana dashboard. These metrics track cache performance, database operations, throughput, and resource usage. Understanding these metrics is essential for analyzing benchmark results and identifying performance issues.

### Key Metrics Categories

#### Cache Performance Metrics

##### Node Cache Misses (read+deserialize)

- **Metric**: `firewood_cache_node{type="miss", mode!="open"}`
- **What it measures**: Rate of cache misses when reading nodes from storage, broken down by operation mode (e.g., read, write)
- **Unit**: Operations per second
- **Interpretation**:
  - Lower values indicate better cache performance
  - High miss rates suggest the cache is too small or access patterns are not cache-friendly
  - Different modes (read/write) can have different miss patterns
- **Good performance**: Miss rate should be low relative to total operations (< 10% of total cache accesses)
- **Poor performance**: Consistently high miss rates (> 50%) indicate inadequate cache size or poor locality

##### Cache Hit Rate

- **Metrics**:
  - Node cache: `firewood_cache_node{type="hit"}` / `firewood_cache_node`
  - Freelist cache: `firewood_cache_freelist{type="hit"}` / `firewood_cache_freelist`
- **What it measures**: Percentage of cache accesses that find data in cache (node cache and freelist cache tracked separately)
- **Unit**: Percentage (0-100%)
- **Interpretation**:
  - Node cache hit rate: How often node reads find data in memory
  - Freelist cache hit rate: How often free space lookups succeed in cache
  - Higher is better - indicates effective caching
- **Good performance**:
  - Node cache: > 90% for steady-state workloads
  - Freelist cache: > 80% is typical
- **Poor performance**:
  - Node cache: < 70% indicates cache thrashing
  - May need to increase `--cache-size` parameter

##### Reads per Insert

- **Metric**: `sum(firewood_cache_node) / sum(firewood_insert)`
- **What it measures**: Average number of node reads required per insert operation
- **Unit**: Ratio (reads/insert)
- **Interpretation**:
  - Indicates the I/O amplification factor for write operations
  - Lower values mean more efficient inserts
  - Varies by benchmark type due to different access patterns
- **Expected values**:
  - `single`: ~2-5 (minimal tree traversal)
  - `zipf`: ~5-15 (skewed access pattern reduces tree depth)
  - `tenkrandom`: ~10-30 (uniform random access requires deeper traversals)
- **Poor performance**: Values 2-3x higher than expected may indicate cache or index issues

#### Operation Rate Metrics

##### Operation Rate

- **Metrics**:
  - `firewood_remove{prefix="true/false"}` - Removals (by prefix or exact key)
  - `firewood_insert{merkle!="update"}` - New insertions
  - `firewood_insert{merkle="update"}` - Updates to existing keys
- **What it measures**: Rate of database operations per second
- **Unit**: Operations per second
- **Interpretation**:
  - Shows the throughput of different operation types
  - Removal operations split by whether they're prefix-based or exact-key
  - Inserts split between new keys and updates
- **Good performance**:
  - Depends on hardware and benchmark type
  - Rates should be stable over time during steady-state
  - `single` benchmark: 10k-50k ops/sec (limited by commit overhead)
  - `zipf`/`tenkrandom`: 50k-200k ops/sec (batched commits)
- **Poor performance**:
  - Declining operation rates over time
  - High variance in operation rates

##### Insert Merkle Ops by Type

- **Metric**: `firewood_insert{merkle="update/above/below/split"}`
- **What it measures**: Categorizes insertions by the type of merkle tree operation performed
- **Unit**: Operations per second
- **Interpretation**:
  - `update`: Updating an existing leaf node (fastest)
  - `above`: Inserting above an existing node in the tree
  - `below`: Inserting below an existing node
  - `split`: Splitting a node to accommodate new data (most expensive)
- **Expected distribution**:
  - `single`: Almost all `update` operations
  - `zipf`: Mostly `update` with occasional `split`
  - `tenkrandom`: Mix of all types, more `split` operations
- **Poor performance**: Excessive `split` operations indicate tree fragmentation

#### Throughput Metrics

##### Proposals Submitted (Blocks Processed)

- **Metric**: `firewood_proposals`
- **What it measures**: Total number of proposals (transactions/batches) committed to the database
- **Unit**: Cumulative count
- **Interpretation**:
  - Each proposal represents a committed batch of operations
  - Rate of increase shows throughput in terms of batches per second
  - Use `irate()` or `rate()` to see proposals per second
- **Good performance**: Steady linear increase over time
- **Poor performance**: Stalls, drops, or irregular patterns indicate commit issues

#### System Resource Metrics

##### Dirty Bytes (OS pending write to disk)

- **Metric**: `node_memory_Dirty_bytes`
- **What it measures**: Amount of modified data in OS page cache waiting to be written to disk
- **Unit**: Bytes
- **Interpretation**:
  - Shows backlog of data waiting to be flushed to storage
  - High values indicate write pressure on storage subsystem
  - Can cause latency spikes when kernel forces flushes
- **Good performance**: Stays relatively stable and bounded
- **Poor performance**: Constantly growing or very high values (> 1GB) indicate I/O bottleneck

### Metric Correlation and Analysis

Understanding how metrics relate to each other helps identify root causes:

#### Cache Performance ↔ Operation Rate

- Low cache hit rate typically correlates with lower operation rates
- Improving cache size/strategy should improve both metrics

#### Operation Rate ↔ Dirty Bytes

- High operation rates increase dirty bytes
- Storage can become bottleneck if dirty bytes grow unbounded

#### Reads per Insert ↔ Cache Hit Rate

- Poor cache hit rate increases reads per insert
- Both metrics should improve together with cache tuning

### Benchmark-Specific Metrics Interpretation

Different benchmarks exhibit different metric patterns:

#### `create` Benchmark

- **Focus**: Initial database population throughput
- **Key metrics**: Operation rate, proposals per second
- **Expected pattern**: Steady operation rate, increasing proposals count
- **Notes**: Cache less relevant as there's no existing data to cache

#### `single` Benchmark

- **Focus**: Minimal transaction overhead, single-row update performance
- **Key metrics**: Proposals per second, update operation rate
- **Expected pattern**:
  - Nearly all operations are `update` type
  - Very high cache hit rate (> 99%)
  - Low reads per insert (~2)
  - Highest proposals/sec rate
- **What constitutes good performance**: 10k-50k commits/sec depending on hardware

#### `zipf` Benchmark

- **Focus**: Realistic skewed workload (hot keys get more updates)
- **Key metrics**: Cache hit rate, operation rate, insert types distribution
- **Expected pattern**:
  - High cache hit rate (90%+) due to hot key concentration
  - Mostly `update` operations with some `split`
  - Medium reads per insert (~5-15)
- **What constitutes good performance**: 50k-150k ops/sec with > 90% cache hit rate

#### `tenkrandom` Benchmark

- **Focus**: Uniform random access with mixed operations (insert/update/delete)
- **Key metrics**: All metrics relevant, cache hit rate critical
- **Expected pattern**:
  - Moderate cache hit rate (depends on cache size vs. database size)
  - Mix of all insert types
  - Higher reads per insert (~10-30)
  - Balanced insert/update/delete rates
- **What constitutes good performance**: 50k-100k ops/sec with stable cache metrics

### Comparing Results Across Runs

To effectively compare benchmark results:

1. **Ensure consistent configuration**:
   - Same `--cache-size`, `--batch-size`, `--number-of-batches`
   - Same database size for steady-state tests
   - Same hardware or account for differences

2. **Compare steady-state metrics**:
   - Ignore warm-up period (first few minutes)
   - Compare averages over stable time windows
   - Look for trends (improving/degrading over time)

3. **Key comparison metrics**:
   - Operation rate (ops/sec)
   - Proposals per second
   - Cache hit rates
   - Reads per insert ratio

4. **Use Grafana time range selection**:
   - Select the same time window for comparison
   - Use "Compare" feature to overlay different runs

### Identifying Performance Bottlenecks

#### Symptom: Low operation rates

- Check: Cache hit rate
  - If low (< 70%): Increase cache size
- Check: Dirty bytes
  - If high and growing: Storage I/O bottleneck, consider faster storage
- Check: Reads per insert
  - If very high: Possible index fragmentation or suboptimal access patterns

#### Symptom: Declining performance over time

- Check: Cache hit rate trend
  - If declining: Database growing beyond cache effectiveness
- Check: Insert merkle ops distribution
  - If increasing `split` operations: Tree fragmentation increasing
- Check: Dirty bytes trend
  - If growing: Storage falling behind

#### Symptom: High variability in metrics

- Check: System resources (CPU, memory, I/O)
- May indicate interference from other processes
- Consider dedicated benchmark environment

#### Symptom: Poor cache hit rate despite large cache

- Check: Reads per insert
  - If very high: Access pattern is not cache-friendly
- Consider: Different cache read strategy (see `--cache-read-strategy` option)
- Check: Database size vs. cache size ratio

### Understanding Grafana Dashboard Panels

The Grafana dashboard organizes metrics into logical groups:

1. **Cache** section: Node cache misses and hit rates - start here to assess cache efficiency
2. **Throughput** section: Overall system throughput (proposals submitted)
3. **Operation Rate** section: Detailed breakdown of operation types and rates
4. **Internals** section: Deep-dive metrics like reads per insert
5. **System** section: OS-level metrics like dirty bytes

**Recommended analysis workflow**:

1. Start with "Proposals Submitted" to see overall throughput
2. Check "Cache hit rate" to assess cache effectiveness
3. Review "Operation Rate" to understand workload composition
4. Examine "Reads per insert" for write amplification
5. Monitor "Dirty bytes" for storage pressure

### Troubleshooting Anomalous Results

#### Sudden drops in operation rate

- Possible causes:
  - Garbage collection pause (if using default allocator)
  - Kernel forced flush of dirty pages
  - Background OS operations
- Solutions:
  - Use jemalloc allocator (already configured in benchmark)
  - Tune kernel write-back parameters
  - Ensure dedicated benchmark environment

#### Cache hit rate suddenly drops

- Possible causes:
  - Database size crossed cache capacity threshold
  - Benchmark switched phases (e.g., from warm-up to steady-state)
  - Cache eviction policy pressure
- Solutions:
  - Increase `--cache-size`
  - Reduce `--number-of-batches` for smaller database
  - Check if pattern is expected for benchmark type

#### Irregular/spiky metrics

- Possible causes:
  - Insufficient warm-up time
  - Interference from other processes
  - Storage device issues (e.g., SSD garbage collection)
- Solutions:
  - Allow longer warm-up period
  - Use dedicated hardware
  - Monitor system-level metrics

#### Very high dirty bytes

- Possible causes:
  - Storage I/O bottleneck
  - Kernel writeback settings too permissive
- Solutions:
  - Use faster storage (NVMe SSD)
  - Tune kernel parameters: `/proc/sys/vm/dirty_*`
  - Reduce operation rate if necessary

### Tips for Optimal Benchmarking

1. **Warm-up period**: Run for 5-10 minutes before measuring to allow caches to stabilize
2. **Duration**: Run for at least 30 minutes to get representative results
3. **Isolation**: Minimize other processes on benchmark machine
4. **Consistency**: Keep configuration parameters consistent across comparison runs
5. **Monitoring**: Always monitor system resources (CPU, memory, I/O) alongside Firewood metrics
6. **Documentation**: Record configuration parameters with results for reproducibility
