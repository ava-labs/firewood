// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

mod counter;
mod hashed;
mod keyvalues;
mod merged;
mod proof;
mod shunt;

pub(crate) use self::hashed::HashedRangeProof;

fn merge_array<T, U, V, E, const N: usize>(
    lhs: [T; N],
    rhs: [U; N],
    mut merge: impl FnMut(T, U) -> Result<Option<V>, E>,
) -> Result<[Option<V>; N], E> {
    let mut output = [const { None::<V> }; N];
    for (slot, (l, r)) in output.iter_mut().zip(lhs.into_iter().zip(rhs)) {
        *slot = merge(l, r)?;
    }
    Ok(output)
}

fn boxed_children<T, const N: usize>(children: [Option<T>; N]) -> [Option<Box<T>>; N] {
    children.map(|maybe| maybe.map(Box::new))
}
