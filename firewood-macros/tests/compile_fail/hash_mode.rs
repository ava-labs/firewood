// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use firewood_macros::hash_mode;

#[hash_mode(extra)]
fn arguments<H: HashMode>() {}

#[hash_mode]
#[test]
async fn asynchronous<H: HashMode>() {}

#[hash_mode]
#[test]
fn missing_generic() {}

#[hash_mode]
#[test]
fn dependent_type<H: HashMode>(value: H) {}

#[hash_mode]
#[test]
fn where_clause<H>() where H: HashMode {}

#[hash_mode]
fn missing_test<H: HashMode>() {}

#[hash_mode]
#[test]
#[cfg_attr(all(), inline)]
fn conditional_harness<H: HashMode>() {}

#[hash_mode]
#[test]
#[inline]
fn unsupported_attribute<H: HashMode>() {}

fn main() {}
