// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::fmt;

pub struct DisplayHex<'a>(pub(super) &'a [u8]);

impl fmt::Display for DisplayHex<'_> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        #![expect(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

        match f.precision() {
            Some(p) if p < self.0.len() => {
                display_hex_bytes(&self.0[..p], f)?;
                f.write_fmt(format_args!("... ({} remaining bytes)", self.0.len() - p))?;
            }
            _ => display_hex_bytes(self.0, f)?,
        }

        Ok(())
    }
}

#[inline]
fn display_hex_bytes(bytes: &[u8], f: &mut fmt::Formatter<'_>) -> fmt::Result {
    #![expect(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

    const CHUNK: usize = 128;
    let mut buf = [0u8; CHUNK * 2];

    for chunk in bytes.chunks(CHUNK) {
        // cannot fail as we statically allocate enough space
        _ = ::hex::encode_to_slice(chunk, &mut buf[..chunk.len() * 2]);
        // SAFETY: we are guaranteed to have a valid ascii string of hex digits
        // of `chunk.len() * 2` length
        let s = unsafe { std::str::from_utf8_unchecked(&buf[..chunk.len() * 2]) };
        f.write_str(s)?;
    }

    Ok(())
}
