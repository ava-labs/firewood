use std::collections::BTreeMap;
use std::fmt::Debug;
use std::io::{Error, Read};
use std::marker::PhantomData;
use std::sync::Arc;

use super::{LinearStore, ReadLinearStore};

/// [Proposed] is a [LinearStore] state that contains a copy of the old and new data.
/// The P type parameter indicates the state of the linear store for it's parent,
/// which could be a another [Proposed] or is [FileBacked](super::filebacked::FileBacked)
/// The M type parameter indicates the mutability of the proposal, either read-write or readonly
#[derive(Debug)]
struct Proposed<P: ReadLinearStore, M> {
    new: BTreeMap<u64, Box<[u8]>>,
    old: BTreeMap<u64, Box<[u8]>>,
    parent: Arc<LinearStore<P>>,
    phantom: PhantomData<M>,
}


/// A [LayeredReader] is obtained by calling [Proposed::stream_from]
/// The P type parameter refers to the type of the parent of this layer
/// The M type parameter is not specified here, but should always be
/// read-only, since we do not support mutating parents of another
/// proposal
#[derive(Debug)]
struct LayeredReader<'a, P: ReadLinearStore, M> {
    offset: u64,
    state: LayeredReaderState<'a>,
    layer: &'a Proposed<P, M>,
}

/// A [LayeredReaderState] keeps track of when the next transition
/// happens for a layer. If you attempt to read bytes past the
/// transition, you'll get what is left, and the state will change
#[derive(Debug)]
enum LayeredReaderState<'a> {
    InitialState,
    BeforeModifiedPart {
        next_offset: u64,
        next_modified_part: &'a [u8],
    },
    InsideModifiedPart {
        part: &'a [u8],
        offset_within: usize,
    },
    NoMoreModifiedParts,
}

impl<P: ReadLinearStore, M: Debug> ReadLinearStore for Proposed<P, M> {
    fn stream_from(&self, addr: u64) -> Result<impl Read, Error> {
        Ok(LayeredReader {
            offset: addr,
            state: LayeredReaderState::InitialState,
            layer: self,
        })
    }
}

// TODO: DiffResolver should also be implemented by Committed
/// The [DiffResolver] trait indicates which field is used from the
/// state of a [LinearStore] based on it's state.
trait DiffResolver<'a> {
    fn diffs(&'a self) -> &'a BTreeMap<u64, Box<[u8]>>;
}

impl<'a, P: ReadLinearStore, M: Debug> DiffResolver<'a> for Proposed<P, M> {
    fn diffs(&'a self) -> &'a BTreeMap<u64, Box<[u8]>> {
        &self.new
    }
}

// TODO: This is not fully implemented as of this commit
// it needs to work across Committed and Proposed types
impl<'a, P: ReadLinearStore, M: Debug> Read for LayeredReader<'a, P, M> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.state {
            LayeredReaderState::InitialState => {
                // figure out which of these cases is true:
                //  a. We are inside a delta [LayeredReaderState::InsideModifiedPart]
                //  b. We are before an upcoming delta [LayeredReaderState::BeforeModifiedPart]
                //  c. We are past the last delta [LayeredReaderState::NoMoreModifiedParts]
                self.state = 'state: {
                    // check for (a) - find the delta in front of (or at) our bytes offset
                    if let Some(delta) = self.layer.diffs().range(..=self.offset).next_back() {
                        // see if the length of the change is inside our address
                        let delta_start = *delta.0;
                        let delta_end = delta_start + delta.1.len() as u64;
                        if self.offset >= delta_start && self.offset < delta_end {
                            // yes, we are inside a modified part
                            break 'state LayeredReaderState::InsideModifiedPart {
                                part: delta.1,
                                offset_within: (self.offset - delta_start) as usize,
                            };
                        }
                    }
                    // check for (b) - find the next delta and record it
                    if let Some(delta) = self.layer.diffs().range(self.offset..).next() {
                        // case (b) is true
                        LayeredReaderState::BeforeModifiedPart {
                            next_offset: *delta.0,
                            next_modified_part: delta.1,
                        }
                    } else {
                        // (c) nothing even coming up, so all remaining bytes are not in this layer
                        LayeredReaderState::NoMoreModifiedParts
                    }
                };
                self.read(buf)
            }
            LayeredReaderState::BeforeModifiedPart {
                next_offset,
                next_modified_part: _,
            } => {
                // see how much we can read until we get to the next modified part
                let remaining_passthrough: usize = (next_offset - self.offset) as usize;
                match buf.len().cmp(&remaining_passthrough) {
                    std::cmp::Ordering::Less => {
                        // remaining_passthrough -= buf.len();
                        todo!()
                    }
                    std::cmp::Ordering::Equal => todo!(),
                    std::cmp::Ordering::Greater => todo!(),
                }
            }
            LayeredReaderState::InsideModifiedPart {
                part: _,
                offset_within: _,
            } => todo!(),
            LayeredReaderState::NoMoreModifiedParts => todo!(),
        }
    }
}
