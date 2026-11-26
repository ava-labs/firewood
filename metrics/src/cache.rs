// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Per-call-site caching of metric handles.

use std::cell::RefCell;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use metrics::SharedString;

/// Entries one call site keeps per thread. Once full, a miss replaces the last entry.
pub const MAX_ENTRIES: usize = 64;

/// Advances whenever a local recorder scope opens or closes, so a cached handle is never
/// reused across scopes even when a later recorder occupies a dropped recorder's address.
static RECORDER_GENERATION: AtomicU64 = AtomicU64::new(0);

/// The current recorder generation.
pub(crate) fn recorder_generation() -> u64 {
    RECORDER_GENERATION.load(Ordering::Relaxed)
}

fn advance_generation() {
    RECORDER_GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// Advances the generation when dropped, so a scope that unwinds still invalidates.
struct AdvanceOnDrop;

impl Drop for AdvanceOnDrop {
    fn drop(&mut self) {
        advance_generation();
    }
}

/// Runs `f` with `recorder` installed as this thread's local recorder.
///
/// Use this instead of [`metrics::with_local_recorder`]. A cached metric handle belongs to the
/// recorder that issued it, and only this function tells the caches that a local recorder
/// scope has opened or closed. A recorder installed directly through `metrics` can receive
/// handles issued by an earlier recorder that lived at the same address.
#[expect(
    clippy::disallowed_methods,
    reason = "this is the wrapper the disallowed-methods entry points to"
)]
pub fn with_local_recorder<T>(recorder: &dyn metrics::Recorder, f: impl FnOnce() -> T) -> T {
    advance_generation();
    let _advance = AdvanceOnDrop;
    metrics::with_local_recorder(recorder, f)
}

/// What a cached handle was registered for.
#[derive(Debug)]
pub(crate) struct EntryKey<'a, const N: usize> {
    /// [`recorder_generation`] read before the handle was registered.
    pub generation: u64,
    /// Data address of the recorder that issued the handle.
    pub recorder: usize,
    /// Call-site label values, in call-site order.
    pub values: &'a [&'static str; N],
    /// The `db_tag` label value, as the thread's metrics context holds it.
    pub db_tag: &'a SharedString,
}

/// Whether `a` and `b` are the same string in memory: same address and length.
fn same_str(a: &str, b: &str) -> bool {
    std::ptr::eq(a, b)
}

#[derive(Debug)]
struct Entry<H, const N: usize> {
    generation: u64,
    recorder: usize,
    values: [&'static str; N],
    db_tag: SharedString,
    handle: H,
}

impl<H, const N: usize> Entry<H, N> {
    const fn same_recorder(&self, key: &EntryKey<'_, N>) -> bool {
        self.generation == key.generation && self.recorder == key.recorder
    }

    /// Matches when every label value and the tag are the same strings in memory. A call
    /// site that passes literals, and a context whose tag the entry shares, always match
    /// this way, without comparing string contents.
    fn matches_in_place(&self, key: &EntryKey<'_, N>) -> bool {
        self.same_recorder(key)
            && self
                .values
                .iter()
                .zip(key.values)
                .all(|(entry, key)| same_str(entry, key))
            && same_str(&self.db_tag, key.db_tag)
    }

    fn matches_by_content(&self, key: &EntryKey<'_, N>) -> bool {
        self.same_recorder(key) && self.values == *key.values && *self.db_tag == **key.db_tag
    }
}

/// The metric handles one call site has registered on this thread.
///
/// A call site declares one in a `thread_local!` and hands it to its metric proxy.
#[doc(hidden)]
#[derive(Debug)]
pub struct HandleCache<H, const N: usize> {
    entries: RefCell<Vec<Entry<H, N>>>,
}

impl<H, const N: usize> Default for HandleCache<H, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<H, const N: usize> HandleCache<H, N> {
    /// An empty cache.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: RefCell::new(Vec::new()),
        }
    }

    /// Calls `hit` with the handle registered for `key`, or returns `None` if there is none.
    ///
    /// Entries are first matched by address, then by content, so equal strings stored at
    /// different addresses still match.
    pub(crate) fn with_hit<R>(
        &self,
        key: &EntryKey<'_, N>,
        hit: impl FnOnce(&H) -> R,
    ) -> Option<R> {
        let entries = self.entries.try_borrow().ok()?;
        entries
            .iter()
            .find(|entry| entry.matches_in_place(key))
            .or_else(|| entries.iter().find(|entry| entry.matches_by_content(key)))
            .map(|entry| hit(&entry.handle))
    }

    /// Stores `handle` as registered for `key`, sharing the key's tag, and replaces the last
    /// entry when the cache is full. Does nothing while a lookup on this cache is in progress.
    pub(crate) fn insert(&self, key: &EntryKey<'_, N>, handle: H) {
        let Ok(mut entries) = self.entries.try_borrow_mut() else {
            return;
        };
        let entry = Entry {
            generation: key.generation,
            recorder: key.recorder,
            values: *key.values,
            db_tag: key.db_tag.clone(),
            handle,
        };
        if entries.len() < MAX_ENTRIES {
            entries.push(entry);
        } else if let Some(last) = entries.last_mut() {
            *last = entry;
        }
    }
}

#[cfg(test)]
mod tests {
    use metrics::NoopRecorder;

    use super::*;
    use crate::MetricsContext;
    use crate::set_metrics_context;
    use crate::with_context;

    static TAG: SharedString = SharedString::const_str("db-a");

    fn key<'a>(values: &'a [&'static str; 1], db_tag: &'a SharedString) -> EntryKey<'a, 1> {
        EntryKey {
            generation: 7,
            recorder: 0x1000,
            values,
            db_tag,
        }
    }

    fn lookup(cache: &HandleCache<u32, 1>, key: &EntryKey<'_, 1>) -> Option<u32> {
        cache.with_hit(key, |handle| *handle)
    }

    /// Changing any one field of the key misses.
    #[test]
    fn each_field_is_part_of_the_key() {
        let cache = HandleCache::new();
        let (values, other_values) = (["hit"], ["miss"]);
        let other_tag = SharedString::const_str("db-b");
        cache.insert(&key(&values, &TAG), 1);

        let mut stale_generation = key(&values, &TAG);
        stale_generation.generation = 8;
        let mut other_recorder = key(&values, &TAG);
        other_recorder.recorder = 0x2000;
        for (field, changed) in [
            ("generation", stale_generation),
            ("recorder", other_recorder),
            ("label values", key(&other_values, &TAG)),
            ("db_tag", key(&values, &other_tag)),
        ] {
            assert_eq!(lookup(&cache, &changed), None, "{field} was ignored");
        }
    }

    /// Label values and tags held at different addresses match by content.
    #[test]
    fn equal_strings_at_different_addresses_match() {
        let cache = HandleCache::new();
        let literal = ["hit"];
        let copy: [&'static str; 1] = [String::leak("hit".to_owned())];
        let owned_tag = SharedString::from_owned("db-a".to_owned());
        assert!(
            !same_str(literal[0], copy[0]),
            "the copy shares the literal's address"
        );
        assert!(
            !same_str(&owned_tag, &TAG),
            "the owned tag shares the static's address"
        );
        cache.insert(&key(&literal, &TAG), 1);
        assert_eq!(lookup(&cache, &key(&copy, &owned_tag)), Some(1));
    }

    /// Inserting while a lookup on the same cache is in progress is skipped instead of
    /// panicking.
    #[test]
    fn insert_during_lookup_is_skipped() {
        let cache = HandleCache::new();
        let (values, other) = (["hit"], ["other"]);
        cache.insert(&key(&values, &TAG), 1);
        let nested = cache.with_hit(&key(&values, &TAG), |_| {
            cache.insert(&key(&other, &TAG), 2);
            lookup(&cache, &key(&values, &TAG))
        });
        assert_eq!(nested, Some(Some(1)), "a nested lookup should still hit");
        assert_eq!(
            lookup(&cache, &key(&other, &TAG)),
            None,
            "the nested insert was stored"
        );
    }

    /// Entering and leaving a local recorder scope each advance the generation, for nested
    /// scopes too.
    #[test]
    fn local_recorder_scopes_advance_generation() {
        with_local_recorder(&NoopRecorder, || {
            let outer = recorder_generation();
            let inner = with_local_recorder(&NoopRecorder, recorder_generation);
            assert!(
                inner > outer,
                "entering a scope did not advance the generation"
            );
            assert!(
                recorder_generation() > inner,
                "leaving a scope did not advance it"
            );
        });
    }

    /// A scope that panics still advances the generation when it unwinds.
    #[test]
    fn panicking_local_recorder_scope_advances_generation() {
        let before = recorder_generation();
        let unwound = std::panic::catch_unwind(|| {
            with_local_recorder(&NoopRecorder, || panic!("scope body panics"));
        });
        assert!(unwound.is_err(), "the scope body should have panicked");
        assert!(
            recorder_generation() >= before.saturating_add(2),
            "an unwinding scope did not advance the generation on exit"
        );
    }

    /// `with_context` passes the thread's tag and expensive flag, or `"untagged"` and `false`
    /// when no context is set.
    #[test]
    fn with_context_reads_tag_and_flag() {
        let read = || with_context(|tag, expensive| (tag.to_string(), expensive));
        assert_eq!(read(), ("untagged".to_owned(), false));
        let _guard = set_metrics_context(Some(
            MetricsContext::new(true).with_db_tag(Some("db-a".into())),
        ));
        assert_eq!(read(), ("db-a".to_owned(), true));
    }
}
