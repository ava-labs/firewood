#include <stdarg.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>


typedef struct Value {
  size_t len;
  const uint8_t *data;
} Value;

/**
 * A `KeyValue` struct that represents a key-value pair in the database.
 */
typedef struct KeyValue {
  struct Value key;
  struct Value value;
} KeyValue;

/**
 * Puts the given key-value pairs into the database.
 *
 * # Returns
 *
 * The current root hash of the database, in Value form.
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
struct Value batch(void *db,
                   size_t nkeys,
                   const struct KeyValue *values);

/**
 * Create a database with the given cache size and maximum number of revisions
 *
 * # Arguments
 *
 * * `path` - The path to the database file, which will be overwritten
 * * `cache_size` - The size of the node cache, panics if <= 0
 * * `revisions` - The maximum number of revisions to keep; firewood currently requires this to be at least 2
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
void *create_db(const char *path,
                size_t cache_size,
                size_t revisions);

/**
 * Frees the memory associated with a `Value`.
 *
 * # Safety
 *
 * This function is unsafe because it dereferences raw pointers.
 * The caller must ensure that `value` is a valid pointer.
 */
void free_value(const struct Value *value);

/**
 * Gets the value associated with the given key from the database.
 *
 * # Arguments
 *
 * * `db` - The database handle returned by `open_db`
 *
 * # Safety
 *
 * The caller must:
 *  * ensure that `db` is a valid pointer returned by `open_db`
 *  * ensure that `key` is a valid pointer to a `Value` struct
 *  * call `free_value` to free the memory associated with the returned `Value`
 */
struct Value get(void *db, struct Value key);

/**
 * Open a database with the given cache size and maximum number of revisions
 *
 * # Arguments
 *
 * * `path` - The path to the database file, which should exist
 * * `cache_size` - The size of the node cache, panics if <= 0
 * * `revisions` - The maximum number of revisions to keep; firewood currently requires this to be at least 2
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
void *open_db(const char *path,
              size_t cache_size,
              size_t revisions);

/**
 * Get the root hash of the latest version of the database
 * Don't forget to call `free_value` to free the memory associated with the returned `Value`.
 *
 * # Safety
 *
 * This function is unsafe because it dereferences raw pointers.
 * The caller must ensure that `db` is a valid pointer returned by `open_db`
 */
struct Value root_hash(void *db);
