#include <stdarg.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>


/**
 * A handle to the database, returned by `fwd_create_db` and `fwd_open_db`.
 *
 * These handles are passed to the other FFI functions.
 *
 */
typedef struct DatabaseHandle DatabaseHandle;

/**
 * A value returned by the FFI.
 *
 * This is used in several different ways, including:
 * * An C-style string.
 * * An ID for a proposal.
 * * A byte slice containing data.
 *
 * For more details on how the data may be stored, refer to the function signature
 * that returned it or the `From` implementations.
 *
 * The data stored in this struct (if `data` is not null) must be manually freed
 * by the caller using `fwd_free_value`.
 *
 */
typedef struct Value {
  size_t len;
  uint8_t *data;
} Value;

/**
 * A `KeyValue` represents a key-value pair, passed to the FFI.
 */
typedef struct KeyValue {
  struct Value key;
  struct Value value;
} KeyValue;

/**
 * Struct returned by `fwd_create_db` and `fwd_open_db`
 */
typedef struct DatabaseCreationResult {
  struct DatabaseHandle *db;
  uint8_t *error_str;
} DatabaseCreationResult;

typedef uint32_t ProposalId;

/**
 * Common arguments, accepted by both `fwd_create_db()` and `fwd_open_db()`.
 *
 * * `path` - The path to the database file, which will be truncated if passed to `fwd_create_db()`
 *   otherwise should exist if passed to `fwd_open_db()`.
 * * `cache_size` - The size of the node cache, returns an error if <= 0
 * * `free_list_cache_size` - The size of the free list cache, returns an error if <= 0
 * * `revisions` - The maximum number of revisions to keep; firewood currently requires this to be at least 2.
 * * `strategy` - The cache read strategy to use, 0 for writes only,
 *   1 for branch reads, and 2 for all reads.
 * * `truncate` - Whether to truncate the database file if it exists.
 *   Returns an error if the value is not 0, 1, or 2.
 */
typedef struct CreateOrOpenArgs {
  const char *path;
  size_t cache_size;
  size_t free_list_cache_size;
  size_t revisions;
  uint8_t strategy;
  bool truncate;
} CreateOrOpenArgs;

/**
 * Arguments for logging
 *
 * * `path` - The file path where logs for this process are stored. By
 *   default, this is set to /tmp/firewood-log.txt
 * * `filter_level` - The filter level for logs. By default, this is set to info.
 */
typedef struct LogArgs {
  const char *path;
  const char *filter_level;
} LogArgs;

/**
 * Puts the given key-value pairs into the database.
 *
 * # Arguments
 *
 * * `db` - The database handle returned by `open_db`
 * * `nkeys` - The number of key-value pairs to put
 * * `values` - A pointer to an array of `KeyValue` structs
 *
 * # Returns
 *
 * The new root hash of the database, in Value form.
 * A `Value` containing {0, "error message"} if the commit failed.
 *
 * # Errors
 *
 * * `"key-value pair is null"` - A `KeyValue` struct is null
 * * `"db should be non-null"` - The database handle is null
 * * `"couldn't get key-value pair"` - A `KeyValue` struct is null
 * * `"proposed revision is empty"` - The proposed revision is empty
 *
 * # Safety
 *
 * This function is unsafe because it dereferences raw pointers.
 * The caller must:
 *  * ensure that `db` is a valid pointer returned by `open_db`
 *  * ensure that `values` is a valid pointer and that it points to an array of `KeyValue` structs of length `nkeys`.
 *  * ensure that the `Value` fields of the `KeyValue` structs are valid pointers.
 *
 */
struct Value fwd_batch(const struct DatabaseHandle *db,
                       size_t nkeys,
                       const struct KeyValue *values);

/**
 * Close and free the memory for a database handle
 *
 * # Safety
 *
 * This function uses raw pointers so it is unsafe.
 * It is the caller's responsibility to ensure that the database handle is valid.
 * Using the db after calling this function is undefined behavior
 *
 * # Arguments
 *
 * * `db` - The database handle to close, previously returned from a call to `open_db()`
 *
 * # Panics
 *
 * This function panics if:
 * * `db` is `None` (null pointer)
 * * A lock is poisoned
 */
void fwd_close_db(struct DatabaseHandle *db);

/**
 * Commits a proposal to the database.
 *
 * # Arguments
 *
 * * `db` - The database handle returned by `open_db`
 * * `proposal_id` - The ID of the proposal to commit
 *
 * # Returns
 *
 * A `Value` containing {0, null} if the commit was successful.
 * A `Value` containing {0, "error message"} if the commit failed.
 *
 * # Safety
 *
 * This function is unsafe because it dereferences raw pointers.
 * The caller must ensure that `db` is a valid pointer returned by `open_db`
 *
 */
struct Value fwd_commit(const struct DatabaseHandle *db, uint32_t proposal_id);

/**
 * Drops a proposal from the database.
 * The propopsal's data is now inaccessible, and can be freed by the `RevisionManager`.
 *
 * # Arguments
 *
 * * `db` - The database handle returned by `open_db`
 * * `proposal_id` - The ID of the proposal to drop
 *
 * # Safety
 *
 * This function is unsafe because it dereferences raw pointers.
 * The caller must ensure that `db` is a valid pointer returned by `open_db`
 *
 */
struct Value fwd_drop_proposal(const struct DatabaseHandle *db, uint32_t proposal_id);

/**
 * Frees the memory associated with a `DatabaseCreationResult`.
 * This only needs to be called if the `error_str` field is non-null.
 *
 * # Arguments
 *
 * * `result` - The `DatabaseCreationResult` to free, previously returned from `fwd_create_db` or `fwd_open_db`.
 *
 * # Safety
 *
 * This function is unsafe because it dereferences raw pointers.
 * The caller must ensure that `result` is a valid pointer.
 *
 * # Panics
 *
 * This function panics if `result` is `null`.
 *
 */
void fwd_free_database_error_result(struct DatabaseCreationResult *result);

/**
 * Frees the memory associated with a `Value`.
 *
 * # Arguments
 *
 * * `value` - The `Value` to free, previously returned from any Rust function.
 *
 * # Safety
 *
 * This function is unsafe because it dereferences raw pointers.
 * The caller must ensure that `value` is a valid pointer.
 *
 * # Panics
 *
 * This function panics if `value` is `null`.
 *
 */
void fwd_free_value(struct Value *value);

/**
 * Gather latest metrics for this process.
 *
 * # Returns
 *
 * A `Value` containing {len, bytes} representing the latest metrics for this process.
 * A `Value` containing {0, "error message"} if unable to get the latest metrics.
 */
struct Value fwd_gather(void);

/**
 * Gets the value associated with the given key from the proposal provided.
 *
 * # Arguments
 *
 * * `db` - The database handle returned by `open_db`
 * * `id` - The ID of the proposal to get the value from
 * * `key` - The key to look up, in `Value` form
 *
 * # Returns
 *
 * A `Value` containing the requested value.
 * A `Value` containing {0, "error message"} if the get failed.
 *
 * # Safety
 *
 * The caller must:
 *  * ensure that `db` is a valid pointer returned by `open_db`
 *  * ensure that `key` is a valid pointer to a `Value` struct
 *  * call `free_value` to free the memory associated with the returned `Value`
 *
 */
struct Value fwd_get_from_proposal(const struct DatabaseHandle *db,
                                   ProposalId id,
                                   struct Value key);

/**
 * Gets a value assoicated with the given root hash and key.
 *
 * The hash may refer to a historical revision or an existing proposal.
 *
 * # Arguments
 *
 * * `db` - The database handle returned by `open_db`
 * * `root` - The root hash to look up, in `Value` form
 * * `key` - The key to look up, in `Value` form
 *
 * # Returns
 *
 * A `Value` containing the requested value.
 * A `Value` containing {0, "error message"} if the get failed.
 *
 * # Safety
 *
 * The caller must:
 * * ensure that `db` is a valid pointer returned by `open_db`
 * * ensure that `key` is a valid pointer to a `Value` struct
 * * ensure that `root` is a valid pointer to a `Value` struct
 * * call `free_value` to free the memory associated with the returned `Value`
 *
 */
struct Value fwd_get_from_root(const struct DatabaseHandle *db,
                               struct Value root,
                               struct Value key);

/**
 * Gets the value associated with the given key from the database.
 *
 * # Arguments
 *
 * * `db` - The database handle returned by `open_db`
 * * `key` - The key to look up, in `Value` form
 *
 * # Returns
 *
 * A `Value` containing the requested value.
 * A `Value` containing {0, "error message"} if the get failed.
 * There is one error case that may be expected to be null by the caller,
 * but should be handled externally: The database has no entries - "IO error: Root hash not found"
 * This is expected behavior if the database is empty.
 *
 * # Safety
 *
 * The caller must:
 *  * ensure that `db` is a valid pointer returned by `open_db`
 *  * ensure that `key` is a valid pointer to a `Value` struct
 *  * call `free_value` to free the memory associated with the returned `Value`
 *
 */
struct Value fwd_get_latest(const struct DatabaseHandle *db, struct Value key);

/**
 * Open a database with the given cache size and maximum number of revisions
 *
 * # Arguments
 *
 * See `CreateOrOpenArgs`.
 *
 * # Returns
 *
 * A database handle, or panics if it cannot be created
 *
 * # Safety
 *
 * This function uses raw pointers so it is unsafe.
 * It is the caller's responsibility to ensure that path is a valid pointer to a null-terminated string.
 * The caller must also ensure that the cache size is greater than 0 and that the number of revisions is at least 2.
 * The caller must call `close` to free the memory associated with the returned database handle.
 *
 */
struct DatabaseCreationResult fwd_open_db(struct CreateOrOpenArgs args);

/**
 * Proposes a batch of operations to the database.
 *
 * # Arguments
 *
 * * `db` - The database handle returned by `open_db`
 * * `nkeys` - The number of key-value pairs to put
 * * `values` - A pointer to an array of `KeyValue` structs
 *
 * # Returns
 *
 * On success, a `Value` containing {len=id, data=hash}. In this case, the
 * hash will always be 32 bytes, and the id will be non-zero.
 * On failure, a `Value` containing {0, "error message"}.
 *
 * # Safety
 *
 * This function is unsafe because it dereferences raw pointers.
 * The caller must:
 *  * ensure that `db` is a valid pointer returned by `open_db`
 *  * ensure that `values` is a valid pointer and that it points to an array of `KeyValue` structs of length `nkeys`.
 *  * ensure that the `Value` fields of the `KeyValue` structs are valid pointers.
 *
 */
struct Value fwd_propose_on_db(const struct DatabaseHandle *db,
                               size_t nkeys,
                               const struct KeyValue *values);

/**
 * Proposes a batch of operations to the database on top of an existing proposal.
 *
 * # Arguments
 *
 * * `db` - The database handle returned by `open_db`
 * * `proposal_id` - The ID of the proposal to propose on
 * * `nkeys` - The number of key-value pairs to put
 * * `values` - A pointer to an array of `KeyValue` structs
 *
 * # Returns
 *
 * On success, a `Value` containing {len=id, data=hash}. In this case, the
 * hash will always be 32 bytes, and the id will be non-zero.
 * On failure, a `Value` containing {0, "error message"}.
 *
 * # Safety
 *
 * This function is unsafe because it dereferences raw pointers.
 * The caller must:
 *  * ensure that `db` is a valid pointer returned by `open_db`
 *  * ensure that `values` is a valid pointer and that it points to an array of `KeyValue` structs of length `nkeys`.
 *  * ensure that the `Value` fields of the `KeyValue` structs are valid pointers.
 *
 */
struct Value fwd_propose_on_proposal(const struct DatabaseHandle *db,
                                     ProposalId proposal_id,
                                     size_t nkeys,
                                     const struct KeyValue *values);

/**
 * Get the root hash of the latest version of the database
 *
 * # Argument
 *
 * * `db` - The database handle returned by `open_db`
 *
 * # Returns
 *
 * A `Value` containing the root hash of the database.
 * A `Value` containing {0, "error message"} if the root hash could not be retrieved.
 * One expected error is "IO error: Root hash not found" if the database is empty.
 * This should be handled by the caller.
 *
 * # Safety
 *
 * This function is unsafe because it dereferences raw pointers.
 * The caller must ensure that `db` is a valid pointer returned by `open_db`
 *
 */
struct Value fwd_root_hash(const struct DatabaseHandle *db);

/**
 * Start logs for this process.
 *
 * # Arguments
 *
 * See `LogArgs`.
 *
 * # Returns
 *
 * A `Value` containing {0, null} if the global logger was initialized.
 * A `Value` containing {0, "error message"} if an error occurs.
 */
struct Value fwd_start_logs(const struct LogArgs *args);

/**
 * Start metrics recorder for this process.
 *
 * # Returns
 *
 * A `Value` containing {0, null} if the metrics recorder was initialized.
 * A `Value` containing {0, "error message"} if an error occurs.
 */
struct Value fwd_start_metrics(void);

/**
 * Start metrics recorder and exporter for this process.
 *
 * * `metrics_port` - the port where metrics will be exposed at
 *
 * # Returns
 *
 * A `Value` containing {0, null} if the metrics recorder was initialized and
 * the exporter was started.
 * A `Value` containing {0, "error message"} if an error occurs.
 */
struct Value fwd_start_metrics_with_exporter(uint16_t metrics_port);
