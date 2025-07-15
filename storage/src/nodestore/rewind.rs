// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! This module provides a reader that can be rewound to the beginning. This is useful
//! when you need to rewind the reader after failing to deserialize a value (e.g., a back
//! tracking deserializer).

use std::io::Read;

/// A reader that can be rewound to the beginning of when the reader was created.
///
/// All data read from the reader is buffered in memory so that it can be read
/// again if rewound. This is necessary when the underlying reader does not
/// implement [`std::io::Seek`].
pub(crate) struct RewindReader<R> {
    buffer: Vec<u8>,
    cursor: usize,
    reader: R,
}

impl<R: Read> RewindReader<R> {
    #[must_use]
    pub const fn new(reader: R) -> Self {
        Self {
            buffer: Vec::new(),
            cursor: 0,
            reader,
        }
    }

    /// Rewinds the reader to the beginning of the stream.
    ///
    /// Subsequent reads will start from where the reader was first created.
    pub const fn rewind(&mut self) {
        self.cursor = 0;
    }
}

impl<R: Read> Read for RewindReader<R> {
    #[expect(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.cursor >= self.buffer.len() {
            // we are at the end of the buffer, so read more data from the
            // underlying reader. Copy it into the buffer, then return.

            let n = self.reader.read(buf)?;
            if n > 0 {
                self.buffer.extend_from_slice(&buf[..n]);
                self.cursor += n;
            }

            Ok(n)
        } else {
            let len = self.buffer.len() - self.cursor;
            let n = Ord::min(len, buf.len());
            buf[..n].copy_from_slice(&self.buffer[self.cursor..self.cursor + n]);
            self.cursor += n;
            Ok(n)
        }
    }
}
