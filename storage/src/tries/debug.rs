// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::Children;

pub(super) struct DebugValue<'a, T: ?Sized> {
    value: Option<&'a T>,
}

impl<'a, T: AsRef<[u8]> + ?Sized> DebugValue<'a, T> {
    pub const fn new(value: Option<&'a T>) -> Self {
        Self { value }
    }
}

impl<T: AsRef<[u8]> + ?Sized> std::fmt::Debug for DebugValue<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        #![expect(clippy::indexing_slicing)]

        const MAX_BYTES: usize = 32;

        let Some(value) = self.value else {
            return write!(f, "None");
        };

        let value = value.as_ref();
        let truncated = &value[..value.len().min(MAX_BYTES)];

        let mut hex_buf = [0u8; MAX_BYTES * 2];
        let hex_buf = &mut hex_buf[..truncated.len().wrapping_mul(2)];
        hex::encode_to_slice(truncated, hex_buf).expect("exact fit");
        let s = str::from_utf8(hex_buf).expect("valid hex");

        if truncated.len() < value.len() {
            write!(f, "0x{s}... (len {})", value.len())
        } else {
            write!(f, "0x{s}")
        }
    }
}

pub(super) struct DebugChildren<'a, T> {
    children: &'a Children<Option<T>>,
}

impl<'a, T: std::fmt::Debug> DebugChildren<'a, T> {
    pub const fn new(children: &'a Children<Option<T>>) -> Self {
        Self { children }
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for DebugChildren<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if f.alternate() {
            // if alternate, debug children as-is (which is pretty and recursive)
            self.children.fmt(f)
        } else {
            // otherwise, replace each child with a continuation marker
            self.children
                .each_ref()
                .map(
                    |_, child| {
                        if child.is_some() { "Some(...)" } else { "None" }
                    },
                )
                .fmt(f)
        }
    }
}
