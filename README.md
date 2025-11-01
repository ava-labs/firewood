# Firewood: Compaction-Less Database Optimized for Efficiently Storing Recent Merkleized Blockchain State

![Github Actions](https://github.com/ava-labs/firewood/actions/workflows/ci.yaml/badge.svg?branch=main)
[![Ecosystem license](https://img.shields.io/badge/License-Ecosystem-blue.svg)](./LICENSE.md)

> :warning: Firewood is beta-level software.
> The Firewood API may change with little to no warning.

Firewood is an embedded key-value store, optimized to store recent Merkleized blockchain
state with minimal overhead. Most blockchains, including Avalanche's C-Chain and Ethereum, store their state in Merkle tries to support efficient generation and verification of state proofs.
Firewood is implemented from the ground up to directly store trie nodes on-disk.
Unlike most state management approaches in the field,
it is not built on top of a generic KV store such as LevelDB/RocksDB.
Firewood, like a B+-tree based database, directly uses the trie structure as the index on-disk.
There is no additional “emulation” of the logical trie to flatten out the data structure
to feed into the underlying database that is unaware of the data being stored.
The convenient byproduct of this approach is that iteration is still fast (for serving state sync queries)
but compaction is not required to maintain the index.
Firewood was first conceived to provide a very fast storage layer for the EVM,
but could be used on any blockchain that requires an authenticated state.

Firewood only attempts to store recent revisions on-disk and will actively clean up unused data when revisions expire.
Firewood keeps some configurable number of previous states in memory and on disk to power state sync and APIs
which may occur at a few roots behind the current state.
To do this, a new root is always created for each revision that can reference either new nodes from this revision or nodes from a prior revision.
When creating a revision,
a list of nodes that are no longer needed are computed and saved to disk in a future-delete log (FDL) as well as kept in memory.
When a revision expires, the nodes that were deleted when it was created are returned to the free space.

Hashes are not used to determine where a node is stored on disk in the database file.
Instead space for nodes may be allocated from the end of the file,
or from space freed from expired revision. Free space management algorithmically resembles that of traditional heap memory management, with free lists used to track different-size spaces that can be reused.
The root address of a node is simply the disk offset within the database file,
and each branch node points to the disk offset of that other node.

Firewood guarantees recoverability by not referencing the new nodes in a new revision before they are flushed to disk,
as well as carefully managing the free list during the creation and expiration of revisions.

## Architecture Diagram

![architecture diagram](./docs/assets/architecture.svg)

## Terminology

- `Revision` - A historical point-in-time state/version of the trie. This
  represents the entire trie, including all `Key`/`Value`s at that point
  in time, and all `Node`s.
- `View` - This is the interface to read from a `Revision` or a `Proposal`.
- `Node` - A node is a portion of a trie. A trie consists of nodes that are linked
  together. Nodes can point to other nodes and/or contain `Key`/`Value` pairs.
- `Hash` - In this context, this refers to the merkle hash for a specific node.
- `Root Hash` - The hash of the root node for a specific revision.
- `Key` - Represents an individual byte array used to index into a trie. A `Key`
  usually has a specific `Value`.
- `Value` - Represents a byte array for the value of a specific `Key`. Values can
  contain 0-N bytes. In particular, a zero-length `Value` is valid.
- `Key Proof` - A proof that a `Key` exists within a specific revision of a trie.
  This includes the hash for the node containing the `Key` as well as all parents.
- `Range Proof` - A proof that consists of two `Key Proof`s, one for the start of
  the range, and one for the end of the range, as well as a list of all `Key`/`Value`
  pairs in between the two. A `Range Proof` can be validated independently of an
  actual database by constructing a trie from the `Key`/`Value`s provided.
- `Change Proof` - A proof that consists of a set of all changes between two
  revisions.
- `Put` - An operation for a `Key`/`Value` pair. A put means "create if it doesn't
  exist, or update it if it does. A put operation is how you add a `Value` for a
  specific `Key`.
- `Delete` - An operation indicating that a `Key` should be removed from the trie.
- `Batch Operation` - An operation of either `Put` or `Delete`.
- `Batch` - An ordered set of `Batch Operation`s.
- `Proposal` - A proposal consists of a base `Root Hash` and a `Batch`, but is not
  yet committed to the trie. In Firewood's most recent API, a `Proposal` is required
  to `Commit`.
- `Commit` - The operation of applying one or more `Proposal`s to the most recent
  `Revision`.

## Build

In order to build firewood, the following dependencies must be installed:

- `protoc` See [installation instructions](https://grpc.io/docs/protoc-installation/).
- `cargo` See [installation instructions](https://doc.rust-lang.org/cargo/getting-started/installation.html).
- `make` See [download instructions](https://www.gnu.org/software/make/#download) or run `sudo apt install build-essential` on Linux.

More detailed build instructions, including some scripts,
can be found in the [benchmark setup scripts](benchmark/setup-scripts).

If you want to build and test the ffi layer for another platform,
you can find those instructions in the [ffi README](ffi/README.md).

## Ethereum compatibility

By default, Firewood builds with hashes compatible with [merkledb](https://github.com/ava-labs/avalanchego/tree/master/x/merkledb),
and does not support accounts.
To enable this feature (at the cost of some performance) enable the ethhash [feature flag](https://doc.rust-lang.org/cargo/reference/features.html#command-line-feature-options).

Enabling this feature
changes the hashing algorithm from [sha256](https://docs.rs/sha2/latest/sha2/type.Sha256.html)
to [keccak256](https://docs.rs/sha3/latest/sha3/type.Keccak256.html),
understands that an "account" is actually just a node in the storage tree at a specific depth with a specific RLP-encoded value,
and computes the hash of the account trie as if it were an actual root.

It is worth noting that the hash stored as a value inside the account root RLP is not used.
During hash calculations, we know the hash of the children,
and use that directly to modify the value in-place
when hashing the node.
See [replace\_hash](firewood/storage/src/hashers/ethhash.rs) for more details.

## Run

Example(s) are in the [examples](firewood/examples) directory, that simulate real world
use-cases. Try running the insert example via the command-line, via `cargo run --release
--example insert`.

There is a [fwdctl cli](fwdctl) for command-line operations on a database.

There is also a [benchmark](benchmark) that shows some other example uses.

For maximum runtime performance at the cost of compile time,
use `cargo run --maxperf` instead,
which enables maximum link time compiler optimizations.

## Logging

Firewood provides comprehensive logging capabilities to help with debugging, monitoring, and understanding system behavior. Logging is implemented using the [env_logger](https://docs.rs/env_logger/latest/env_logger/) crate and the [log](https://docs.rs/log/latest/log/) facade.

### Enabling Logging

Logging is controlled by a feature flag and must be explicitly enabled during compilation:

```sh
# Build with logging enabled
cargo build --features logger

# Build and run an example with logging
cargo run --features logger --example insert

# Build the fwdctl CLI with logging
cargo build -p firewood-fwdctl --features logger
```

**Note:** The `logger` feature is **not** enabled by default. This is intentional to ensure zero-overhead when logging is not needed, as Firewood is optimized for performance.

### Setting Log Levels with RUST_LOG

Once logging is enabled at compile time, you control the verbosity at runtime using the `RUST_LOG` environment variable:

```sh
# Set log level to info (default recommended level)
export RUST_LOG=info

# Set log level to debug for more detailed output
export RUST_LOG=debug

# Set log level to trace for maximum verbosity
export RUST_LOG=trace

# Set log level to warn to see only warnings and errors
export RUST_LOG=warn

# Set log level to error to see only errors
export RUST_LOG=error
```

You can also set module-specific log levels for fine-grained control:

```sh
# Enable debug logs only for storage module
export RUST_LOG=firewood_storage=debug

# Enable trace logs for firewood and info for everything else
export RUST_LOG=firewood=trace,info

# Enable debug logs for specific modules
export RUST_LOG=firewood::db=debug,firewood_storage=info
```

### Log Level Recommendations

Choose the appropriate log level based on your scenario:

| Scenario | Recommended Level | Description |
|----------|------------------|-------------|
| **Production** | `warn` or `error` | Minimal logging overhead, only critical issues |
| **Development** | `info` or `debug` | Balanced view of operations without overwhelming detail |
| **Debugging Issues** | `debug` or `trace` | Detailed information for troubleshooting |
| **Performance Testing** | Logging disabled | Compile without `logger` feature for maximum performance |
| **Integration Testing** | `info` | Sufficient context without excessive noise |

### Configuring Log Output Destination

By default, logs are written to **stderr**. For the Rust API, you can configure this using `env_logger` before initializing your database:

```rust
use env_logger::Target;

// Write logs to stdout instead of stderr
env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
    .target(Target::Stdout)
    .init();
```

For the **fwdctl** CLI tool, logs automatically go to stderr and you can redirect them:

```sh
# Redirect logs to a file
RUST_LOG=debug fwdctl dump --db mydb.db 2> firewood.log

# Separate logs from normal output
RUST_LOG=info fwdctl dump --db mydb.db > output.txt 2> logs.txt
```

### Example Log Messages

Firewood emits logs at various points during operation. Here are some examples:

**Debug Level Logs:**

- Database operation details: `"inserting key value pair Options { ... }"`
- Configuration information: `"database configuration parameters: DbConfig { ... }"`
- Operation execution: `"deleting key Options { ... }"`

**Info Level Logs:**

- Database initialization and startup information
- Major state changes and milestones
- Batch operation completions

**Warn Level Logs:**

- Recoverable issues or degraded performance
- Configuration warnings
- Resource constraints

**Error Level Logs:**

- Failed operations
- I/O errors
- Data corruption or validation failures

### Adding Logging to Custom Code

If you're building on top of Firewood or extending it, you can add logging to your code:

```rust
use firewood_storage::logger::{debug, info, warn, error, trace};

fn my_custom_operation() {
    info!("Starting custom operation");
    debug!("Processing with config: {:?}", config);
    
    match process() {
        Ok(result) => {
            trace!("Detailed result: {:?}", result);
            info!("Operation completed successfully");
        }
        Err(e) => {
            error!("Operation failed: {}", e);
        }
    }
}
```

**Important:** When the `logger` feature is not enabled, these macros become no-ops with zero runtime overhead. The unused variable warnings are automatically suppressed.

### FFI-Specific Logging

When using Firewood through the FFI (Foreign Function Interface) layer, logging must be configured from the host language. See the [FFI README](ffi/README.md#logs) for language-specific instructions.

**Key points for FFI logging:**

- Firewood FFI must be compiled with the `logger` feature enabled
- Call `StartLogs(config)` before opening the database
- Logs are written to a file (default: `{TEMP}/firewood-log.txt`)
- Log level defaults to `info` if not specified
- Available log levels: `trace`, `debug`, `info`, `warn`, `error`

**Go FFI Example:**

```go
import ffi "github.com/ava-labs/firewood-go-ethhash/ffi"

func main() {
    // Configure logging before opening database
    logConfig := ffi.LogConfig{
        Path:        "/var/log/firewood.log",
        FilterLevel: "debug",
    }
    ffi.StartLogs(logConfig)
    
    // Now open and use the database
    db, err := ffi.Open("mydb", nil)
    // ...
}
```

### Performance Considerations

Logging has performance implications that you should be aware of:

**Compile-Time Impact:**

- **Without `logger` feature:** Zero overhead. Log macro invocations are completely compiled away.
- **With `logger` feature:** Small binary size increase (~50-100KB) and slightly longer compile times.

**Runtime Impact:**

- **RUST_LOG not set or set to `off`:** Minimal overhead. Log statements are checked but not executed.
- **RUST_LOG=error or warn:** Very low overhead (~1-2% in typical workloads).
- **RUST_LOG=info:** Low overhead (~2-5% depending on workload).
- **RUST_LOG=debug:** Moderate overhead (~5-15% depending on log volume).
- **RUST_LOG=trace:** High overhead (~15-30% or more). Use only for debugging.

**Best Practices:**

1. **Compile without `logger` for production benchmarks** to get true performance numbers
2. **Use `warn` or `error` in production** to minimize overhead while capturing important events
3. **Avoid logging in hot paths** when designing new code
4. **Use conditional logging** for expensive debug information:

```rust
use firewood_storage::logger::{debug, trace_enabled};

// Avoid expensive computation if trace logging is disabled
if trace_enabled() {
    trace!("Expensive debug info: {:?}", compute_expensive_debug_info());
}
```

1. **Test with logging enabled** during development to catch issues early
1. **Profile with realistic log levels** if you'll be running with logging in production

### Troubleshooting

**Logs not appearing?**

1. Ensure you compiled with the `logger` feature flag
2. Verify `RUST_LOG` is set correctly: `echo $RUST_LOG`
3. Check that logs are going to stderr (not stdout)
4. For FFI usage, verify `StartLogs()` was called before opening the database

**Too much log output?**

1. Increase the log level (e.g., from `debug` to `info`)
2. Use module-specific filters: `RUST_LOG=firewood=info,warn`
3. Redirect to a file and use grep: `RUST_LOG=debug cargo run 2>&1 | grep "interesting_term"`

**Log performance impact too high?**

1. Reduce log level to `warn` or `error`
2. Use module-specific filters to only log certain components
3. For maximum performance, recompile without the `logger` feature

### Additional Resources

- [env_logger Documentation](https://docs.rs/env_logger/latest/env_logger/)
- [log Crate Documentation](https://docs.rs/log/latest/log/)
- [FFI Logging Documentation](ffi/README.md#logs)
- [Storage Logger Implementation](storage/src/logger.rs)

## Release

See the [release documentation](./RELEASE.md) for detailed information on how to release Firewood.

## CLI

Firewood comes with a CLI tool called `fwdctl` that enables one to create and interact with a local instance of a Firewood database. For more information, see the [fwdctl README](fwdctl/README.md).

## Test

```sh
cargo test --release
```

## License

Firewood is licensed by the Ecosystem License. For more information, see the
[LICENSE file](./LICENSE.md).
