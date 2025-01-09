#include <stdarg.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>


typedef struct Value {
  size_t len;
  const uint8_t *data;
} Value;

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
 * The caller must ensure that `values` is a valid pointer and that it points to an array of `KeyValue` structs of length `nkeys`.
 */
struct Value batch(size_t nkeys,
                   const struct KeyValue *values);

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
 * Don't forget to call `free_value` to free the memory associated with the returned `Value`.
 */
struct Value get(struct Value key);

/**
 * Get the root hash of the latest version of the database
 * Don't forget to call `free_value` to free the memory associated with the returned `Value`.
 */
struct Value root_hash(void);

/**
 * Setup the global database handle. You don't need to call this fuction; it is called automatically by the library.
 */
void setup_globals(void);
