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
    pub(crate) fn new(parent: Arc<LinearStore<P>>) -> Self {
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

impl<P: ReadLinearStore, M: Send + Sync + Debug> ReadLinearStore for Proposed<P, M> {
    fn stream_from(&self, addr: u64) -> Result<Box<dyn Read + '_>, Error> {
        Ok(Box::new(LayeredReader {
            offset: addr,
            state: LayeredReaderState::InitialState,
            layer: self,
        }))
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
        debug_assert!(!object.is_empty());
        // search the modifications in front of (or on) this one that can be merged
        // into ours. If we merge, we move modify offset and object, so make mutable
        // copies of them here
        let mut updated_offset = offset;
        let mut updated_object: Box<[u8]> = Box::from(object);

        // we track areas to delete here, which happens when we combine
        let mut merged_offsets_to_delete = Vec::new();

        for existing in self.new.range(..=updated_offset).rev() {
            let old_end = *existing.0 + existing.1.len() as u64;
            // found    smme--------- s=existing.0 e=old_end
            // original ----smmme---- s=new_start e=new_start+object.len() [0 overlap]
            // original --smmmme----- with overlap
            // new      smmmmmmme----
            if updated_offset <= old_end {
                // we have some overlap, so we'll have only one area when we're done
                // mark the original area for deletion and recompute it
                // TODO: can this be done with iter/zip?
                let mut updated = vec![];
                for offset in *existing.0.. {
                    updated.push(
                        if offset >= updated_offset
                            && offset < updated_offset + updated_object.len() as u64
                        {
                            *updated_object
                                .get((offset - updated_offset) as usize)
                                .expect("bounds checked above")
                        } else if let Some(res) = existing.1.get((offset - *existing.0) as usize) {
                            *res
                        } else {
                            break;
                        },
                    );
                }
                updated_offset = *existing.0;
                merged_offsets_to_delete.push(updated_offset);
                updated_object = updated.into_boxed_slice();
            }
        }
        // TODO: search for modifications after this one that can be merged
        for existing in self.new.range(updated_offset + 1..) {
            // existing ----smme---- s=existing.0 e=existing.0+existing.1.len()
            // adding   --smme------ s=updated_offset e=new_end
            let new_end = updated_offset + updated_object.len() as u64;
            if new_end >= *existing.0 {
                // we have some overlap
                let mut updated = vec![];
                for offset in updated_offset.. {
                    updated.push(if offset >= updated_offset && offset < new_end {
                        *updated_object
                            .get((offset - updated_offset) as usize)
                            .expect("bounds checked above")
                    } else if let Some(res) = existing.1.get((offset - *existing.0) as usize) {
                        *res
                    } else {
                        break;
                    });
                }
                merged_offsets_to_delete.push(*existing.0);
                updated_object = updated.into_boxed_slice();
            }
        }
        for delete_offset in merged_offsets_to_delete {
            self.new.remove(&delete_offset);
        }

        self.new.insert(updated_offset, updated_object);
        Ok(object.len())
    }
}

// this is actually a clippy bug, works on nightly; see
// https://github.com/rust-lang/rust-clippy/issues/12519
#[allow(clippy::unused_io_amount)]
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
                    // TODO: make this work across Committed and Proposed types

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
                    // read to the end of this area
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
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::super::tests::ConstBacked;
    use test_case::test_case;

    use super::*;

    #[test]
    fn smoke_read() -> Result<(), std::io::Error> {
        let sut: Proposed<ConstBacked, Immutable> = ConstBacked::new(ConstBacked::DATA).into();

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
        let mut sut: Proposed<ConstBacked, Mutable> = ConstBacked::new(ConstBacked::DATA).into();

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
        let mut sut: Proposed<ConstBacked, Mutable> = ConstBacked::new(ConstBacked::DATA).into();

        sut.write(pos, delta)?;

        let mut data = [0u8; ConstBacked::DATA.len()];
        sut.stream_from(0).unwrap().read_exact(&mut data).unwrap();
        assert_eq!(data, expected);

        Ok(())
    }

    #[test]
    fn nested() {
        let mut parent: Proposed<ConstBacked, Mutable> = ConstBacked::new(ConstBacked::DATA).into();
        parent.write(1, b"1").unwrap();

        let parent: Arc<LinearStore<Proposed<ConstBacked, Mutable>>> =
            Arc::new(LinearStore { state: parent });
        let mut child: Proposed<Proposed<ConstBacked, Mutable>, Mutable> = Proposed::new(parent);
        child.write(3, b"3").unwrap();

        let mut data = [0u8; ConstBacked::DATA.len()];
        child.stream_from(0).unwrap().read_exact(&mut data).unwrap();
        assert_eq!(&data, b"r1n3om data");
    }

    #[test]
    fn deep_nest() {
        let mut parent: Proposed<ConstBacked, Mutable> = ConstBacked::new(ConstBacked::DATA).into();
        parent.write(1, b"1").unwrap();
        let mut child: Arc<dyn ReadLinearStore> = Arc::new(parent);
        for _ in 0..=200 {
            child = Arc::new(LinearStore { state: child });
        }
        let mut data = [0u8; ConstBacked::DATA.len()];
        child.stream_from(0).unwrap().read_exact(&mut data).unwrap();
        assert_eq!(&data, b"r1ndom data");
    }

    // possible cases (o) represents unmodified data (m) is modified in first
    // modification, M is new modification on top of old
    // oommoo < original state, one modification at offset 2 length 2 (2,2)
    //
    // entire contents before first modified store, space between it
    // oooooo original (this note is not repeated below)
    // oommoo original with first modification (this note is not repeated below)
    // M----- modification at offset 0, length 1
    // Mommoo result: two modified areas (0, 1) and (2, 2)
    #[test_case(&[(2, 2)], (0, 1), b"Mommoo", 2)]
    // adjoints the first modified store, no overlap
    // MM---- modification at offset 0, length 2
    // MMmmoo result: one enlarged modified area (0, 4)
    #[test_case(&[(2, 2)], (0, 2), b"MMmmoo", 1)]
    // starts before and overlaps some of the first modified store
    // MMM--- modification at offset 0, length 3
    // MMMmoo result: one enlarged modified area (0, 4)
    #[test_case(&[(2, 2)], (0, 3), b"MMMmoo", 1)]
    // still starts before and overlaps, modifies all of it
    // MMMM-- modification at offset 0, length 4
    // MMMMoo same result (0, 4)
    #[test_case(&[(2,2)], (0, 4), b"MMMMoo", 1)]
    // starts at same offset, modifies some of it
    // --M--- modification at offset 2, length 1
    // ooMmoo result: one modified area (2, 2)
    #[test_case(&[(2, 2)], (2, 1), b"ooMmoo", 1)]
    // same offset, exact fit
    // --MM-- modification at offset 2, length 2
    // ooMMoo result: one modified area (2, 2)
    #[test_case(&[(2, 2)], (2, 2), b"ooMMoo", 1)]
    // starts at same offset, enlarges
    // --MMM- modification at offset 2, length 3
    // ooMMMo result: one enlarged modified area (2, 3)
    #[test_case(&[(2, 2)], (2, 3), b"ooMMMo", 1)]
    // within existing offset
    // ---M-- modification at offset 3, length 1
    // oomMoo result: one modified area (2, 2)
    #[test_case(&[(2, 2)], (3, 1), b"oomMoo", 1)]
    // starts within original offset, becomes larger
    // ---MM- modification at offset 3, length 2
    // oomMMo result: one modified area (2, 3)
    #[test_case(&[(2, 2)], (3, 2), b"oomMMo", 1)]
    // starts immediately after modified area
    // ----M- modification at offset 4, length 1
    // oommMo result: one modified area (2, 3)
    #[test_case(&[(2, 2)], (4, 1), b"oommMo", 1)]
    // disjoint from modified area
    // -----M modification at offset 5, length 1
    // oommoM result: two modified areas (2, 2) and (5, 1)
    #[test_case(&[(2, 2)], (5, 1), b"oommoM", 2)]
    // consume it all
    // MMMMMM modification at offset 0, length 6
    // MMMMMM result: one modified area
    #[test_case(&[(2, 2)], (0, 6), b"MMMMMM", 1)]
    // consume all
    // starting mods are:
    // omomom
    // MMMMMM modification at offset 0 length 6
    // MMMMMM result, one modified area
    #[test_case(&[(1, 1),(3, 1), (5, 1)], (0, 6), b"MMMMMM", 1)]
    // consume two
    // MMMM-- modificatoin at offset 0, length 4
    // MMMMom result, two modified areas
    #[test_case(&[(1, 1),(3, 1), (5, 1)], (0, 4), b"MMMMom", 2)]
    // consume all, just but don't modify the last one
    #[test_case(&[(1, 1),(3, 1), (5, 1)], (0, 5), b"MMMMMm", 1)]
    // extend store from within original area
    #[test_case(&[(2, 2)], (5, 6), b"oommoMMMMMM", 2)]
    // extend store beyond original area
    #[test_case(&[(2, 2)], (6, 6), b"oommooMMMMMM", 2)]
    // consume multiple before us
    #[test_case(&[(1, 1), (3, 1)], (2, 3), b"omMMMo", 1)]

    fn test_combiner(
        original_mods: &[(u64, usize)],
        new_mod: (u64, usize),
        result: &'static [u8],
        segments: usize,
    ) -> Result<(), Error> {
        let mut proposal: Proposed<ConstBacked, Mutable> = ConstBacked::new(b"oooooo").into();
        for mods in original_mods {
            proposal.write(
                mods.0,
                (0..mods.1).map(|_| b'm').collect::<Vec<_>>().as_slice(),
            )?;
        }

        proposal.write(
            new_mod.0,
            (0..new_mod.1).map(|_| b'M').collect::<Vec<_>>().as_slice(),
        )?;

        // this bleeds the implementation, but I this made debugging the tests way easier...
        assert_eq!(proposal.new.len(), segments);

        let mut data = vec![];
        proposal
            .stream_from(0)
            .unwrap()
            .read_to_end(&mut data)
            .unwrap();

        assert_eq!(data, result);

        Ok(())
    }
}
