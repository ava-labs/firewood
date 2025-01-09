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

void free_value(struct Value value);

struct Value get(struct Value key);

struct Value root_hash(void);

void setup_globals(void);
