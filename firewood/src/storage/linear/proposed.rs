use std::collections::BTreeMap;
use std::io::Read;
use std::marker::PhantomData;
use std::sync::Arc;

use super::LinearStore;

#[derive(Debug)]
struct Proposed<P, T> {
    new: BTreeMap<u64, Box<[u8]>>,
    old: BTreeMap<u64, Box<[u8]>>,
    parent: Arc<LinearStore<P>>,
    phantom: PhantomData<T>,
}

#[derive(Debug)]
struct LayeredReader<'a, T> {
    offset: u64,
    state: LayeredReaderState<'a>,
    layer: &'a LinearStore<T>,
}

impl<'a, T> Read for LayeredReader<'a, T> {
    fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
        todo!()
    }
}

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

/*
impl<'a, P, T> Read for LayeredReader<'a, Proposed<P, T>> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.state {
            LayeredReaderState::InitialState => {

                // figure out which of these cases is true:
                //  a. We are inside a delta [LayeredReaderState::InsideModifiedPart]
                //  b. We are before an upcoming delta [LayeredReaderState::BeforeModifiedPart]
                //  c. We are past the last delta [LayeredReaderState::NoMoreModifiedParts]
                self.state = 'state: {
                    // check for (a) - find the delta in front of (or at) our bytes offset
                    if let Some(delta) = self.layer.state.new.range(..=self.offset).next_back() {
                        // see if the length of the change is inside our address
                        let delta_start = *delta.0;
                        let delta_end = delta_start + delta.1.len() as u64;
                        if self.offset >= delta_start && self.offset < delta_end {
                            // yes, we are inside a modified part
                            break 'state LayeredReaderState::InsideModifiedPart {
                                part: &delta.1,
                                offset_within: (self.offset - delta_start) as usize,
                            };
                        }
                    }
                    // check for (b) - find the next delta and record it
                    if let Some(delta) = self.layer.state.new.range(self.offset..).next() {
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
                next_modified_part,
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
                part,
                offset_within,
            } => todo!(),
            LayeredReaderState::NoMoreModifiedParts => todo!(),
        }
    }
}



impl<Type: Debug> ReadOnlyLinearStore for LinearStore<Type>
where for<'a> LayeredReader<'a, Type>: Read + Debug {
    fn stream_from(&self, addr: u64) -> Result<impl Read, Error> {
        // Find out whether a change in this store touches
        Ok(LayeredReader {
            offset: addr,
            state: LayeredReaderState::InitialState,
            layer: self,
        })
    }
}
*/
