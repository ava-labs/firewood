// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::io::{Error, Read};
use std::marker::PhantomData;
use std::sync::{Arc, RwLock};

use super::layered::{Layer, LayeredReader};
use super::{LinearStoreParent, ReadLinearStore, WriteLinearStore};

#[derive(Debug)]
pub struct Immutable;
#[derive(Debug)]
pub struct Mutable;
/// A shortcut for a [Proposed<Mutable>]
pub type ProposedMutable = Proposed<Mutable>;
/// A shortcut for a [Proposed<Immutable>]
pub type ProposedImmutable = Proposed<Immutable>;

#[derive(Debug)]
/// A proposal backed by a [WriteLinearStore] or a [ReadLinearStore]
/// The generic is either [Mutable] or [Immutable]
pub struct Proposed<M: Send + Sync + Debug> {
    /// The new data in a proposal
    pub new: BTreeMap<u64, Box<[u8]>>,
    /// The old data in a proposal
    pub old: BTreeMap<u64, Box<[u8]>>,
    parent: RwLock<LinearStoreParent>,
    phantom_data: PhantomData<M>,
}

impl ProposedMutable {
    /// Create a new mutable proposal from a given parent [ReadLinearStore]
    pub fn new(parent: LinearStoreParent) -> Self {
        Self {
            parent: RwLock::new(parent),
            new: Default::default(),
            old: Default::default(),
            phantom_data: PhantomData,
        }
    }

    /// Freeze a mutable proposal, consuming it, and making it immutable
    pub fn freeze(self) -> ProposedImmutable {
        ProposedImmutable {
            old: self.old,
            new: self.new,
            parent: self.parent,
            phantom_data: PhantomData,
        }
    }
}

impl ProposedImmutable {
    /// Reparent the proposal
    pub fn reparent(self: &Arc<Self>, parent: LinearStoreParent) {
        *self.parent.write().expect("poisoned lock") = parent;
    }

    /// Retrieve a copy of the parent of the proposal
    pub fn parent(&self) -> LinearStoreParent {
        self.parent.read().expect("poisoned lock").clone()
    }

    /// Check if this parent is the same as the provided one
    pub fn has_parent(&self, candidate: &LinearStoreParent) -> bool {
        *self.parent.read().expect("poisoned lock") == *candidate
    }
}

impl<M: Debug + Send + Sync> ReadLinearStore for Proposed<M> {
    fn stream_from(&self, addr: u64) -> Result<Box<dyn Read + '_>, Error> {
        Ok(Box::new(LayeredReader::new(
            addr,
            Layer::new(
                self.parent.read().expect("poisoned lock").clone(),
                &self.new,
            ),
        )))
    }

    fn size(&self) -> Result<u64, Error> {
        // start with the parent size
        let parent_size = self.parent.read().expect("poisoned lock").size()?;
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

impl WriteLinearStore for ProposedMutable {
    // TODO: we might be able to optimize the case where we keep adding data to
    // the end of the file by not coalescing left. We'd have to change the assumption
    // in the reader that when you reach the end of a modified region, you're always
    // in an unmodified region
    fn write(&mut self, offset: u64, object: &[u8]) -> Result<usize, Error> {
        // the structure of what will eventually be inserted
        struct InsertData {
            offset: u64,
            data: Box<[u8]>,
        }

        debug_assert!(!object.is_empty());
        let mut insert = InsertData {
            offset,
            data: Box::from(object),
        };

        // we track areas to delete here, which happens when we combine
        let mut merged_offsets_to_delete = Vec::new();

        // TODO: combining a change where we constantly append to a file might be too
        // expensive, so consider skipping the relocations if they don't overlap at all
        for existing in self.new.range(..=insert.offset).rev() {
            let old_end = *existing.0 + existing.1.len() as u64;
            // found    smme--------- s=existing.0 e=old_end
            // original ----smmme---- s=new_start e=new_start+object.len() [0 overlap]
            // original --smmmme----- with overlap
            // new      smmmmmmme----
            if insert.offset <= old_end {
                // we have some overlap, so we'll have only one area when we're done
                // mark the original area for deletion and recompute it
                // TODO: can this be done with iter/zip?
                // TODO: can we avoid copying the mutating data?
                let mut new_data = vec![];
                for resulting_bytes_offset in *existing.0.. {
                    new_data.push(
                        if resulting_bytes_offset >= insert.offset
                            && resulting_bytes_offset < insert.offset + insert.data.len() as u64
                        {
                            *insert
                                .data
                                .get((resulting_bytes_offset - insert.offset) as usize)
                                .expect("bounds checked above")
                        } else if let Some(res) = existing
                            .1
                            .get((resulting_bytes_offset - *existing.0) as usize)
                        {
                            *res
                        } else {
                            break;
                        },
                    );
                }
                insert.offset = *existing.0;
                merged_offsets_to_delete.push(insert.offset);
                insert.data = new_data.into_boxed_slice();
            }
        }
        // TODO: search for modifications after this one that can be merged
        for existing in self.new.range(insert.offset + 1..) {
            // existing ----smme---- s=existing.0 e=existing.0+existing.1.len()
            // adding   --smme------ s=updated_offset e=new_end
            let new_end = insert.offset + insert.data.len() as u64;
            if new_end >= *existing.0 {
                // we have some overlap
                let mut new_data = vec![];
                for resulting_bytes_offset in insert.offset.. {
                    new_data.push(
                        if resulting_bytes_offset >= insert.offset
                            && resulting_bytes_offset < new_end
                        {
                            *insert
                                .data
                                .get((resulting_bytes_offset - insert.offset) as usize)
                                .expect("bounds checked above")
                        } else if let Some(res) = existing
                            .1
                            .get((resulting_bytes_offset - *existing.0) as usize)
                        {
                            *res
                        } else {
                            break;
                        },
                    );
                }
                merged_offsets_to_delete.push(*existing.0);
                insert.data = new_data.into_boxed_slice();
            }
        }
        for delete_offset in merged_offsets_to_delete {
            self.new.remove(&delete_offset);
        }

        self.new.insert(insert.offset, insert.data);
        Ok(object.len())
    }

    fn freeze(self) -> ProposedImmutable {
        Proposed {
            new: self.new,
            old: self.old,
            parent: self.parent,
            phantom_data: Default::default(),
        }
    }
}

// this is actually a clippy bug, works on nightly; see
// https://github.com/rust-lang/rust-clippy/issues/12519
#[allow(clippy::unused_io_amount)]
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use crate::MemStore;

    use super::*;
    use rand::Rng;
    use std::time::Instant;
    use test_case::test_case;

    const TEST_DATA: &[u8] = b"random data";

    #[test]
    fn smoke_read() -> Result<(), std::io::Error> {
        let parent = MemStore::new(TEST_DATA.into());

        let proposed = Proposed::new(parent.into());

        // read all
        let mut data = [0u8; TEST_DATA.len()];
        proposed
            .stream_from(0)
            .unwrap()
            .read_exact(&mut data)
            .unwrap();
        assert_eq!(data, TEST_DATA);

        // starting not at beginning
        let mut data = [0u8; TEST_DATA.len()];
        proposed
            .stream_from(1)
            .unwrap()
            .read_exact(&mut data[1..])?;
        assert_eq!(data.get(1..), TEST_DATA.get(1..));

        // ending not at end
        let mut data = [0u8; TEST_DATA.len() - 1];
        proposed.stream_from(0).unwrap().read_exact(&mut data)?;
        assert_eq!(Some(data.as_slice()), TEST_DATA.get(..data.len()));

        Ok(())
    }

    #[test]
    fn smoke_mutate() -> Result<(), std::io::Error> {
        let parent = MemStore::new(TEST_DATA.into());

        const MUT_DATA: &[u8] = b"data random";

        let mut proposed = Proposed::new(parent.into());

        // mutate the whole thing
        proposed.write(0, MUT_DATA)?;

        // read all the changed data
        let mut data = [0u8; MUT_DATA.len()];
        proposed
            .stream_from(0)
            .unwrap()
            .read_exact(&mut data)
            .unwrap();
        assert_eq!(data, MUT_DATA);

        Ok(())
    }

    #[test_case(0, b"1", b"1andom data")]
    #[test_case(1, b"2", b"r2ndom data")]
    #[test_case(10, b"3", b"random dat3")]
    fn partial_mod_full_read(pos: u64, delta: &[u8], expected: &[u8]) -> Result<(), Error> {
        let parent = MemStore::new(TEST_DATA.into());

        let mut proposed = Proposed::new(parent.into());

        proposed.write(pos, delta)?;

        let mut data = [0u8; TEST_DATA.len()];
        proposed
            .stream_from(0)
            .unwrap()
            .read_exact(&mut data)
            .unwrap();
        assert_eq!(data, expected);

        Ok(())
    }

    #[test]
    fn nested() {
        let parent = MemStore::new(TEST_DATA.into());

        let mut proposed = Proposed::new(parent.clone().into());
        proposed.write(1, b"1").unwrap();

        let mut proposed2 = Proposed::new(parent.into());

        proposed2.write(3, b"3").unwrap();

        let mut data = [0u8; TEST_DATA.len()];
        proposed2
            .stream_from(0)
            .unwrap()
            .read_exact(&mut data)
            .unwrap();
        assert_eq!(&data, b"r1n3om data");
    }

    #[test]
    fn deep_nest() {
        let parent = MemStore::new(TEST_DATA.into());

        let mut proposed = Proposed::new(parent.clone().into());
        proposed.write(1, b"1").unwrap();

        let mut child = Proposed::new(parent.into()).freeze();
        for _ in 0..=200 {
            child = Proposed::new(child.into()).freeze();
        }
        let mut data = [0u8; TEST_DATA.len()];
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
    // MMMM-- modification at offset 0, length 4
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
        let parent = MemStore::new(b"oooooo".into());

        let mut proposal = Proposed::new(parent.into());
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

    #[test]
    fn bench_combiner() -> Result<(), Error> {
        const COUNT: usize = 100000;
        const DATALEN: usize = 32;
        const MODIFICATION_AREA_SIZE: u64 = 2048;

        let parent = MemStore::new(TEST_DATA.into());

        let mut proposal = Proposed::new(parent.into());
        let mut rng = rand::thread_rng();
        let start = Instant::now();
        for _ in 0..COUNT {
            let data = rng.gen::<[u8; DATALEN]>();
            proposal.write(rng.gen_range(0..MODIFICATION_AREA_SIZE), &data)?;
        }
        println!(
            "inserted {} of size {} in {}ms",
            COUNT,
            DATALEN,
            Instant::now().duration_since(start).as_millis()
        );
        Ok(())
    }
}
