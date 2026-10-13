# Membership Filter

How to build, enable, validate, and monitor the read-path membership filter:
a counting bloom filter consulted by `get_value` so reads of definitely-absent
keys skip the trie walk. It must never produce a false negative; a false
positive only costs one unnecessary walk.

## Build with the feature

The subsystem is gated by the `filter` cargo feature and costs nothing when off:

```bash
cargo build --release -p firewood-ffi --features ethhash,filter   # FFI library
cargo build --release -p firewood-fwdctl --features ethhash,filter # fwdctl
```

## Configure at runtime

The filter is configured by environment variables, so the FFI surface and the
node configuration are unchanged:

| variable | meaning | default |
| --- | --- | --- |
| `FIREWOOD_FILTER_PATH` | checkpoint file; **set to enable** | unset (disabled) |
| `FIREWOOD_FILTER_MODE` | `skip` (use the verdict) or `verify` (walk anyway, audit) | `skip` |
| `FIREWOOD_FILTER_DELETE` | `retain` (sound for every retained revision) or `eager` (latest-only) | `retain` |
| `FIREWOOD_FILTER_CHECKPOINT` | commits between background checkpoints; `0` = only at close | `1000` |
| `FIREWOOD_FILTER_EXPECTED_KEYS` | size a fresh filter for an **empty** database when no checkpoint exists | unset |
| `FIREWOOD_FILTER_COUNTERS_PER_KEY` | counters per key for that fresh filter | `12` |

One database per process may enable the filter: it binds to the first database
opened while `FIREWOOD_FILTER_PATH` is set.

## Lifecycle

1. **Build once** from the database's latest revision (the node must not be
   running against the database while this runs):

   ```bash
   fwdctl build-filter --db <db-dir> -o <db-dir>/membership.fwdfilter
   ```

   Add `--expected-keys N` to skip the counting pass and `--counters-per-key N`
   to trade memory for false positives (12 → ~0.3% fpp at ~4.8 bytes/key). The
   checkpoint is tagged with the root hash it was built from.

2. **Validate** before trusting it: run with `FIREWOOD_FILTER_MODE=verify`.
   Every read still walks the trie, and an "absent" verdict that turns out to be
   present is counted in `firewood_membership_filter_total{verdict="false_negative"}`.
   A correct build yields **zero**.

3. **Run** with `FIREWOOD_FILTER_PATH=<db-dir>/membership.fwdfilter` (skip mode).
   On open the checkpoint is loaded only if its root tag equals the database's
   current root. Every commit updates the filter in memory; a background
   checkpoint is written every `FIREWOOD_FILTER_CHECKPOINT` commits and a final
   one at close, tagged with the root at that moment.

4. **Restart**: a clean shutdown leaves a checkpoint that matches the database,
   so the next open loads it. After a crash, or if another process advanced the
   database, the tag no longer matches and the filter stays **disabled** with a
   warning — reads are correct, just not accelerated. Rebuild with
   `fwdctl build-filter` to re-enable it.

## Choosing the delete policy

- **`retain` (default, recommended):** deletes never decrement. The filter is a
  superset of the keys live in *any* retained revision, so `get_value` may skip
  for reads against every revision. Deleted keys linger as harmless false
  positives; the fill ratio rises slowly with churn and is reset by an occasional
  offline rebuild.
- **`eager`:** decrements on every single-key delete. Lower fpp, but the filter
  then tracks only the **latest** revision: consulting it for an older revision
  can produce a false negative, and a background checkpoint racing with deletes
  in a not-yet-committed proposal can under-approximate the tagged revision. Use
  only for latest-only read workloads.

## Metrics

- `firewood_membership_filter_total{verdict=...}`: `maybe`, `absent`, `insert`,
  `remove`, `false_negative` (verify mode only), `checkpoint_ok`, `checkpoint_fail`.
- `firewood_membership_filter_fill_ratio`: fraction of nonzero counters,
  sampled at each checkpoint. Rebuild with a larger `--counters-per-key` when it
  climbs.
- `firewood_membership_filter_saturated_counters`: pinned counters (saturation
  only raises fpp).

Skip rate among misses is `absent / (absent + maybe)`.

## Safety invariants

- **No false negatives** under `retain` for any retained revision, and under
  `eager` for latest-revision reads.
- A corrupt, missing, or **root-mismatched** checkpoint is never trusted: the
  filter is disabled and reads walk the trie as before.
- An empty filter is only created for an empty database.
- Saturated counters are pinned. A false **positive** costs one unnecessary walk.
