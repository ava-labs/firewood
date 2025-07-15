// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::io::Read;

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
