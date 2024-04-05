// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::io::{Cursor, Error, Read};
use std::marker::PhantomData;
use std::sync::Arc;

use super::{LinearStore, ReadLinearStore, WriteLinearStore};

/// [Proposed] is a [LinearStore] state that contains a copy of the old and new data.
/// The P type parameter indicates the state of the linear store for it's parent,
/// which could be a another [Proposed] or is [FileBacked](super::filebacked::FileBacked)
/// The M type parameter indicates the mutability of the proposal, either read-write or readonly
#[derive(Debug)]
pub(crate) struct Proposed<P: ReadLinearStore, M> {
    new: BTreeMap<u64, Box<[u8]>>,
    old: BTreeMap<u64, Box<[u8]>>,
    parent: Arc<LinearStore<P>>,
    phantom: PhantomData<M>,
}

impl<P: ReadLinearStore, M> Proposed<P, M> {
    fn new(parent: Arc<LinearStore<P>>) -> Self {
        Self {
            parent,
            new: Default::default(),
            old: Default::default(),
            phantom: Default::default(),
        }
    }
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
    // we know nothing, we might already be inside a modified area, one might be coming, or there aren't any left
    InitialState,
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

impl<P: ReadLinearStore, M: Debug> ReadLinearStore for Proposed<P, M> {
    fn stream_from(&self, addr: u64) -> Result<impl Read, Error> {
        Ok(LayeredReader {
            offset: addr,
            state: LayeredReaderState::InitialState,
            layer: self,
        })
    }

    fn size(&self) -> Result<u64, Error> {
        // start with the parent size
        let parent_size = self.parent.state.size()?;
        // look at the last delta, if any, and see if it will extend the file
        Ok(self
            .new
            .range(..)
            .next_back()
            .map(|(k, v)| *k + v.len() as u64)
            .map_or_else(
                || parent_size,
                |delta_end| std::cmp::max(parent_size, delta_end),
            ))
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

/// Marker that the Proposal is mutable
#[derive(Debug)]
pub(crate) struct Mutable;

/// Marker that the Proposal is immutable
#[derive(Debug)]
pub(crate) struct Immutable;

impl<P: ReadLinearStore> WriteLinearStore for Proposed<P, Mutable> {
    fn write(&mut self, offset: u64, object: &[u8]) -> Result<usize, Error> {
        // TODO: naive implementation
        self.new.insert(offset, Box::from(object));
        // TODO: coalesce
        Ok(object.len())
    }
}

// TODO: make this work across Committed and Proposed types
impl<'a, P: ReadLinearStore, M: Debug> Read for LayeredReader<'a, P, M> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.state {
            LayeredReaderState::InitialState => {
                // figure out which of these cases is true:
                //  a. We are inside a delta [LayeredReaderState::InsideModifiedArea]
                //  b. We are before an upcoming delta [LayeredReaderState::BeforeModifiedArea]
                //  c. We are past the last delta [LayeredReaderState::NoMoreModifiedAreas]
                self.state = 'state: {
                    // check for (a) - find the delta in front of (or at) our bytes offset
                    if let Some(delta) = self.layer.diffs().range(..=self.offset).next_back() {
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
                self.state = if let Some(delta) = self.layer.diffs().range(self.offset..).next() {
                    // case (b) is true
                    LayeredReaderState::BeforeModifiedArea {
                        next_offset: *delta.0,
                        next_modified_area: delta.1,
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
                if offset_within + size == area.len() {
                    // read to the end of this area
                    self.state = LayeredReaderState::FindNext;
                }
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
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use test_case::test_case;

    use super::*;

    #[derive(Debug)]
    struct ConstBacked;

    impl ConstBacked {
        const DATA: &'static [u8] = b"random data";

        const fn new() -> Self {
            Self {}
        }
    }

    impl From<ConstBacked> for Arc<LinearStore<ConstBacked>> {
        fn from(state: ConstBacked) -> Self {
            Arc::new(LinearStore { state })
        }
    }

    impl From<ConstBacked> for Proposed<ConstBacked, Mutable> {
        fn from(value: ConstBacked) -> Self {
            Proposed::new(value.into())
        }
    }

    impl From<ConstBacked> for Proposed<ConstBacked, Immutable> {
        fn from(value: ConstBacked) -> Self {
            Proposed::new(value.into())
        }
    }

    impl ReadLinearStore for ConstBacked {
        fn stream_from(&self, addr: u64) -> Result<impl std::io::Read, std::io::Error> {
            Ok(Cursor::new(Self::DATA.get(addr as usize..).unwrap()))
        }

        fn size(&self) -> Result<u64, Error> {
            Ok(Self::DATA.len() as u64)
        }
    }

    #[test]
    fn smoke_read() -> Result<(), std::io::Error> {
        let sut: Proposed<ConstBacked, Immutable> = ConstBacked::new().into();

        // read all
        let mut data = [0u8; ConstBacked::DATA.len()];
        sut.stream_from(0).unwrap().read_exact(&mut data).unwrap();
        assert_eq!(data, ConstBacked::DATA);

        // starting not at beginning
        let mut data = [0u8; ConstBacked::DATA.len()];
        sut.stream_from(1).unwrap().read_exact(&mut data[1..])?;
        assert_eq!(data.get(1..), ConstBacked::DATA.get(1..));

        // ending not at end
        let mut data = [0u8; ConstBacked::DATA.len() - 1];
        sut.stream_from(0).unwrap().read_exact(&mut data)?;
        assert_eq!(Some(data.as_slice()), ConstBacked::DATA.get(..data.len()));

        Ok(())
    }

    #[test]
    fn smoke_mutate() -> Result<(), std::io::Error> {
        let mut sut: Proposed<ConstBacked, Mutable> = ConstBacked::new().into();

        const MUT_DATA: &[u8] = b"data random";

        // mutate the whole thing
        sut.write(0, MUT_DATA)?;

        // read all the changed data
        let mut data = [0u8; MUT_DATA.len()];
        sut.stream_from(0).unwrap().read_exact(&mut data).unwrap();
        assert_eq!(data, MUT_DATA);

        Ok(())
    }

    #[test_case(0, b"1", b"1andom data")]
    #[test_case(1, b"2", b"r2ndom data")]
    #[test_case(10, b"3", b"random dat3")]
    fn partial_mod_full_read(pos: u64, delta: &[u8], expected: &[u8]) -> Result<(), Error> {
        let mut sut: Proposed<ConstBacked, Mutable> = ConstBacked::new().into();

        sut.write(pos, delta)?;

        let mut data = [0u8; ConstBacked::DATA.len()];
        sut.stream_from(0).unwrap().read_exact(&mut data).unwrap();
        assert_eq!(data, expected);

        Ok(())
    }

    #[test]
    fn nested() {
        let mut parent: Proposed<ConstBacked, Mutable> = ConstBacked::new().into();
        parent.write(1, b"1").unwrap();

        let parent: Arc<LinearStore<Proposed<ConstBacked, Mutable>>> =
            Arc::new(LinearStore { state: parent });
        let mut child: Proposed<Proposed<ConstBacked, Mutable>, Mutable> = Proposed::new(parent);
        child.write(3, b"3").unwrap();

        let mut data = [0u8; ConstBacked::DATA.len()];
        child.stream_from(0).unwrap().read_exact(&mut data).unwrap();
        assert_eq!(&data, b"r1n3om data");
    }
}
