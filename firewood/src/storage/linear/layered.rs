// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::{collections::BTreeMap, io::Cursor, sync::Arc};

use super::ImmutableLinearStore;

#[derive(Debug)]
pub(super) struct Layer<'a> {
    parent: Arc<ImmutableLinearStore>,
    diffs: &'a BTreeMap<u64, Box<[u8]>>,
}

impl<'a> Layer<'a> {
    pub(super) fn new(
        parent: Arc<ImmutableLinearStore>,
        diffs: &'a BTreeMap<u64, Box<[u8]>>,
    ) -> Self {
        Self { parent, diffs }
    }
}

/// A [LayeredReader] is obtained by calling [Proposed::stream_from]
/// The P type parameter refers to the type of the parent of this layer
/// The M type parameter is not specified here, but should always be
/// read-only, since we do not support mutating parents of another
/// proposal
#[derive(Debug)]
pub(super) struct LayeredReader<'a> {
    offset: u64,
    state: LayeredReaderState<'a>,
    layer: Layer<'a>,
}

impl<'a> LayeredReader<'a> {
    pub(super) const fn new(offset: u64, layer: Layer<'a>) -> Self {
        Self {
            offset,
            state: LayeredReaderState::Initial,
            layer,
        }
    }
}

/// A [LayeredReaderState] keeps track of when the next transition
/// happens for a layer. If you attempt to read bytes past the
/// transition, you'll get what is left, and the state will change
#[derive(Debug)]
enum LayeredReaderState<'a> {
    // we know nothing, we might already be inside a modified area, one might be coming, or there aren't any left
    Initial,
    // we know we are not in a modified area, so find the next modified area
    FindNext,
    // we know there are no more modified areas past our current offset
    NoMoreModifiedAreas,
    BeforeModifiedArea {
        next_offset: u64,
        next_modified_area: &'a [u8],
    },
    InsideModifiedArea {
        area: &'a [u8],
        offset_within: usize,
    },
}

impl<'a> std::io::Read for LayeredReader<'a> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.state {
            LayeredReaderState::Initial => {
                // figure out which of these cases is true:
                //  a. We are inside a delta [LayeredReaderState::InsideModifiedArea]
                //  b. We are before an upcoming delta [LayeredReaderState::BeforeModifiedArea]
                //  c. We are past the last delta [LayeredReaderState::NoMoreModifiedAreas]
                self.state = 'state: {
                    // check for (a) - find the delta in front of (or at) our bytes offset
                    if let Some(delta) = self.layer.diffs.range(..=self.offset).next_back() {
                        // see if the length of the change is inside our address
                        let delta_start = *delta.0;
                        let delta_end = delta_start + delta.1.len() as u64;
                        if self.offset >= delta_start && self.offset < delta_end {
                            // yes, we are inside a modified area
                            break 'state LayeredReaderState::InsideModifiedArea {
                                area: delta.1,
                                offset_within: (self.offset - delta_start) as usize,
                            };
                        }
                    }
                    break 'state LayeredReaderState::FindNext;
                };
                self.read(buf)
            }
            LayeredReaderState::FindNext => {
                // check for (b) - find the next delta and record it
                self.state = if let Some(delta) = self.layer.diffs.range(self.offset..).next() {
                    if self.offset == *delta.0 {
                        LayeredReaderState::InsideModifiedArea {
                            area: delta.1,
                            offset_within: 0,
                        }
                    } else {
                        // case (b) is true
                        LayeredReaderState::BeforeModifiedArea {
                            next_offset: *delta.0,
                            next_modified_area: delta.1,
                        }
                    }
                } else {
                    // (c) nothing even coming up, so all remaining bytes are not in this layer
                    LayeredReaderState::NoMoreModifiedAreas
                };
                self.read(buf)
            }
            LayeredReaderState::BeforeModifiedArea {
                next_offset,
                next_modified_area,
            } => {
                // if the buffer is smaller than the remaining bytes in this change, then
                // restrict the read to only read up to the remaining areas
                let remaining_passthrough: usize = (next_offset - self.offset) as usize;
                let size = if buf.len() > remaining_passthrough {
                    let read_size = self.layer.parent.stream_from(self.offset)?.read(
                        buf.get_mut(0..remaining_passthrough)
                            .expect("length already checked"),
                    )?;
                    if read_size == remaining_passthrough {
                        // almost always true, unless a partial read happened
                        self.state = LayeredReaderState::InsideModifiedArea {
                            area: next_modified_area,
                            offset_within: 0,
                        };
                    }
                    read_size
                } else {
                    self.layer.parent.stream_from(self.offset)?.read(buf)?
                };
                self.offset += size as u64;
                Ok(size)
            }
            LayeredReaderState::InsideModifiedArea {
                area,
                offset_within,
            } => {
                let size = Cursor::new(
                    area.get(offset_within..)
                        .expect("bug: offset_within not in bounds"),
                )
                .read(buf)?;
                self.offset += size as u64;
                debug_assert!(offset_within + size <= area.len());
                self.state = if offset_within + size == area.len() {
                    // we read to the end of this area
                    LayeredReaderState::FindNext
                } else {
                    LayeredReaderState::InsideModifiedArea {
                        area,
                        offset_within: offset_within + size,
                    }
                };
                Ok(size)
            }
            LayeredReaderState::NoMoreModifiedAreas => {
                let size = self.layer.parent.stream_from(self.offset)?.read(buf)?;
                self.offset += size as u64;
                Ok(size)
            }
        }
    }
}
