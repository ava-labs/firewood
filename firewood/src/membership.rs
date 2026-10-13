// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Integration of the mutable membership filter into Firewood's read path.
//!
//! When enabled, `get_value` consults the filter and skips the trie walk for
//! definitely-absent keys; inserts and removes maintain it incrementally;
//! commits checkpoint it so it never has to be rebuilt from the key-value
//! store on restart.
//!
//! A checkpoint is tagged with the root hash of the revision it covers and is
//! only loaded when the database being opened is at that exact root. Keys
//! committed after a snapshot are missing from it, and a missing key is a
//! false negative, so a filter the database has moved past is refused rather
//! than trusted.
//!
//! The filter is process-wide: one database per process may enable it. The
//! whole subsystem is gated by the `filter` cargo feature. When the feature is
//! off, every entry point is a trivial no-op (see the `stub` module), so the
//! call sites in `db`, `merkle`, and `manager` are feature-independent and
//! zero-cost.

#[cfg(feature = "filter")]
mod imp {
    use std::path::PathBuf;
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::Duration;

    use firewood_metrics::{GaugeExt, firewood_counter, firewood_gauge};
    use firewood_storage::filter::{CountingBloom, MembershipFilter, load_counting};
    use firewood_storage::logger::{info, warn};

    use crate::api::HashKey;

    const ENV_PATH: &str = "FIREWOOD_FILTER_PATH";
    const ENV_MODE: &str = "FIREWOOD_FILTER_MODE"; // skip | verify
    const ENV_DELETE: &str = "FIREWOOD_FILTER_DELETE"; // retain | eager
    const ENV_CHECKPOINT: &str = "FIREWOOD_FILTER_CHECKPOINT"; // commits; 0 = never
    const ENV_EXPECTED_KEYS: &str = "FIREWOOD_FILTER_EXPECTED_KEYS";
    const ENV_COUNTERS_PER_KEY: &str = "FIREWOOD_FILTER_COUNTERS_PER_KEY";

    const DEFAULT_CHECKPOINT: u64 = 1000;
    const DEFAULT_COUNTERS_PER_KEY: u32 = 12;

    /// Whether removes decrement the filter (`eager`, sound for latest-revision
    /// reads only) or are no-ops (`retain`, sound for reads against any retained
    /// revision).
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum DeletePolicy {
        Retain,
        Eager,
    }

    struct State {
        filter: CountingBloom,
        path: PathBuf,
        skip_mode: bool,
        delete: DeletePolicy,
        checkpoint_interval: u64,
        commits: AtomicU64,
        saving: AtomicBool,
    }

    static STATE: OnceLock<Option<State>> = OnceLock::new();

    fn state() -> Option<&'static State> {
        STATE.get().and_then(Option::as_ref)
    }

    /// Whether a filter is bound and consulted by the read path. Diagnostic for
    /// embedders and tests; the filter itself is configured by environment.
    #[must_use]
    pub fn is_enabled() -> bool {
        state().is_some()
    }

    fn describe(root: Option<&HashKey>) -> String {
        root.map_or_else(|| "<empty>".to_owned(), ToString::to_string)
    }

    fn env_u64(name: &str) -> Option<u64> {
        std::env::var(name).ok().and_then(|v| v.parse().ok())
    }

    /// Bind the filter to a database whose current root is `current_root`.
    /// Called once when the database is opened; a second call is ignored.
    ///
    /// The filter stays disabled unless an existing checkpoint is tagged with
    /// exactly `current_root`, or the database is empty and a fresh filter is
    /// requested through `FIREWOOD_FILTER_EXPECTED_KEYS`. Anything else could
    /// under-approximate the key set and produce false negatives.
    pub(crate) fn open(current_root: Option<&HashKey>) {
        if STATE.get().is_some() {
            if std::env::var_os(ENV_PATH).is_some() {
                warn!("membership filter: already bound to a database; ignoring second open");
            }
            return;
        }
        let _ = STATE.set(init(current_root));
    }

    fn init(current_root: Option<&HashKey>) -> Option<State> {
        let path = std::env::var_os(ENV_PATH).map(PathBuf::from)?;
        let filter = if path.exists() {
            let (filter, tag) = match load_counting(&path) {
                Ok(loaded) => loaded,
                Err(err) => {
                    // A corrupt filter could produce false negatives; never trust it.
                    warn!("membership filter: load failed ({err}); disabled");
                    return None;
                }
            };
            if tag.as_ref() != current_root {
                warn!(
                    "membership filter: checkpoint {} covers root {} but the database is at {}; \
                     disabled (rebuild with `fwdctl build-filter`)",
                    path.display(),
                    describe(tag.as_ref()),
                    describe(current_root),
                );
                return None;
            }
            info!(
                "membership filter: loaded {} ({} MiB, fill {:.4}) for root {}",
                path.display(),
                filter.size_bytes() >> 20,
                filter.stats().fill_ratio(),
                describe(current_root),
            );
            filter
        } else if let Some(expected) = env_u64(ENV_EXPECTED_KEYS) {
            if current_root.is_some() {
                warn!(
                    "membership filter: {ENV_EXPECTED_KEYS} creates an empty filter, which is only sound for an \
                     empty database; disabled (build one with `fwdctl build-filter`)"
                );
                return None;
            }
            let counters_per_key = env_u64(ENV_COUNTERS_PER_KEY)
                .and_then(|v| u32::try_from(v).ok())
                .unwrap_or(DEFAULT_COUNTERS_PER_KEY);
            let filter = CountingBloom::new(expected, counters_per_key);
            info!(
                "membership filter: created empty ({} MiB) for ~{expected} keys",
                filter.size_bytes() >> 20
            );
            filter
        } else {
            warn!(
                "membership filter: {ENV_PATH} set but file missing and {ENV_EXPECTED_KEYS} unset; disabled"
            );
            return None;
        };

        let skip_mode = std::env::var(ENV_MODE).ok().as_deref() != Some("verify");
        let delete = match std::env::var(ENV_DELETE).ok().as_deref() {
            Some("eager") => DeletePolicy::Eager,
            _ => DeletePolicy::Retain,
        };
        let checkpoint_interval = env_u64(ENV_CHECKPOINT).unwrap_or(DEFAULT_CHECKPOINT);

        info!(
            "membership filter: mode={} delete={} checkpoint={checkpoint_interval}",
            if skip_mode { "skip" } else { "verify" },
            if delete == DeletePolicy::Eager {
                "eager"
            } else {
                "retain"
            },
        );
        Some(State {
            filter,
            path,
            skip_mode,
            delete,
            checkpoint_interval,
            commits: AtomicU64::new(0),
            saving: AtomicBool::new(false),
        })
    }

    /// In skip mode, returns true iff `key` is definitely absent, so the caller
    /// may skip the trie walk. Always false in verify mode. Records the verdict.
    pub(crate) fn definitely_absent(key: &[u8]) -> bool {
        let Some(st) = state() else {
            return false;
        };
        if st.filter.contains(key) {
            firewood_counter!(FILTER, "verdict" => "maybe").increment(1);
            false
        } else {
            firewood_counter!(FILTER, "verdict" => "absent").increment(1);
            st.skip_mode
        }
    }

    /// In verify mode, count a false negative: the filter said absent but the
    /// walk found the key. Used to validate a filter before trusting skip mode.
    pub(crate) fn audit_false_negative(key: &[u8], found: bool) {
        if let Some(st) = state()
            && found
            && !st.skip_mode
            && !st.filter.contains(key)
        {
            firewood_counter!(FILTER, "verdict" => "false_negative").increment(1);
        }
    }

    pub(crate) fn insert(key: &[u8]) {
        if let Some(st) = state() {
            st.filter.insert(key);
            firewood_counter!(FILTER, "verdict" => "insert").increment(1);
        }
    }

    /// Callers must only report genuine removals of a present key (the
    /// matched-remove contract); the filter ignores them under `retain`.
    pub(crate) fn remove(key: &[u8]) {
        if let Some(st) = state()
            && st.delete == DeletePolicy::Eager
        {
            st.filter.remove(key);
            firewood_counter!(FILTER, "verdict" => "remove").increment(1);
        }
    }

    /// Called once per committed revision with its root: checkpoints the
    /// filter in the background every `checkpoint_interval` commits.
    ///
    /// Every key of the committed revision was inserted while its proposal was
    /// built, so a snapshot started after the commit covers that root. Keys of
    /// proposals still being built may also be captured; they are false
    /// positives, which are safe.
    pub(crate) fn on_commit(root: Option<&HashKey>) {
        let Some(st) = state() else {
            return;
        };
        let n = st.commits.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        // `checkpoint_interval == 0` means never checkpoint.
        if n.checked_rem(st.checkpoint_interval) != Some(0) {
            return;
        }
        // Never block the commit; skip if a checkpoint is already in flight.
        if st.saving.swap(true, Ordering::AcqRel) {
            return;
        }
        let root = root.cloned();
        let spawned = std::thread::Builder::new()
            .name("fwd-filter-ckpt".to_owned())
            .spawn(move || {
                checkpoint(st, root.as_ref());
                st.saving.store(false, Ordering::Release);
            });
        if spawned.is_err() {
            st.saving.store(false, Ordering::Release);
        }
    }

    /// Called when the database closes: waits for any in-flight background
    /// checkpoint, then writes a final one tagged with the closing root so the
    /// next open finds a checkpoint that matches the database.
    pub(crate) fn checkpoint_on_close(root: Option<&HashKey>) {
        let Some(st) = state() else {
            return;
        };
        while st.saving.swap(true, Ordering::AcqRel) {
            std::thread::sleep(Duration::from_millis(5));
        }
        checkpoint(st, root);
        st.saving.store(false, Ordering::Release);
    }

    fn checkpoint(st: &State, root: Option<&HashKey>) {
        let stats = st.filter.stats();
        firewood_gauge!(FILTER_FILL).set(stats.fill_ratio());
        firewood_gauge!(FILTER_SATURATED).set_integer(stats.saturated);
        match st.filter.save(&st.path, root) {
            Ok(()) => {
                firewood_counter!(FILTER, "verdict" => "checkpoint_ok").increment(1);
            }
            Err(err) => {
                warn!("membership filter: checkpoint failed: {err}");
                firewood_counter!(FILTER, "verdict" => "checkpoint_fail").increment(1);
            }
        }
    }
}

#[cfg(not(feature = "filter"))]
mod stub {
    use crate::api::HashKey;

    #[inline]
    pub(crate) const fn open(_current_root: Option<&HashKey>) {}
    #[inline]
    pub(crate) const fn definitely_absent(_key: &[u8]) -> bool {
        false
    }
    #[inline]
    pub(crate) const fn audit_false_negative(_key: &[u8], _found: bool) {}
    #[inline]
    pub(crate) const fn insert(_key: &[u8]) {}
    #[inline]
    pub(crate) const fn remove(_key: &[u8]) {}
    #[inline]
    pub(crate) const fn on_commit(_root: Option<&HashKey>) {}
    #[inline]
    pub(crate) const fn checkpoint_on_close(_root: Option<&HashKey>) {}
}

#[cfg(feature = "filter")]
pub use imp::is_enabled as membership_filter_enabled;
#[cfg(feature = "filter")]
pub(crate) use imp::*;
#[cfg(not(feature = "filter"))]
pub(crate) use stub::*;
