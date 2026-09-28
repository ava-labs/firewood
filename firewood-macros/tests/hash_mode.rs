// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![deny(unfulfilled_lint_expectations)]
#![expect(
    clippy::extra_unused_type_parameters,
    clippy::missing_const_for_fn,
    reason = "macro fixtures exercise test signatures and attributes, not hash-mode behavior"
)]

use firewood_macros::hash_mode;
use firewood_storage::{EthHash, HashMode, MerkleDbHash};
use test_case::test_case;

#[hash_mode]
#[test]
fn test_slow_ordinary<H: HashMode>() {}

#[hash_mode]
#[test]
fn result<H: HashMode>() -> Result<(), String> {
    std::str::from_utf8(b"valid").map_err(|error| error.to_string())?;
    Ok(())
}

#[hash_mode]
#[test_case(1; "one")]
#[test_case(2; "two")]
fn cases<H: HashMode>(number: u8) {
    assert!((1..=2).contains(&number));
}

#[hash_mode]
#[test]
#[should_panic(expected = "hash mode panic")]
fn panics<H: HashMode>() {
    panic!("hash mode panic");
}

#[expect(
    clippy::should_panic_without_expect,
    reason = "exercise bare should_panic routing"
)]
mod bare_panic {
    use super::*;

    #[hash_mode]
    #[test]
    #[should_panic]
    fn panics_any<H: HashMode>() {
        panic!("any panic");
    }
}

#[hash_mode]
#[test]
#[ignore = "explicit ignored-run fixture"]
fn ignored<H: HashMode>() {}

#[hash_mode]
#[test]
#[cfg(any())]
fn disabled<H: HashMode>() {
    nonexistent_function::<H>();
}

#[hash_mode]
#[test]
#[cfg(test)]
fn enabled<H: HashMode>() {}

#[hash_mode]
#[test]
#[expect(clippy::bool_assert_comparison, reason = "exercise expect routing")]
fn lint_expect<H: HashMode>() {
    assert_eq!(std::hint::black_box(true), true);
}

#[hash_mode]
#[test]
#[warn(clippy::bool_assert_comparison)]
#[deny(clippy::unwrap_used)]
fn lint_levels<H: HashMode>() {}
