// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use super::historical::Historical;
use super::{LinearStore, ReadLinearStore};

/// A linear store used for the current revision. The difference betwen this
/// and a historical revision is that it has a mutable parent. It is reparented
/// when another proposal commits
///
/// A [Current] [LinearStore] supports read operations only
#[derive(Debug)]
pub(crate) struct Current<P: ReadLinearStore> {
    old: BTreeMap<u64, Box<[u8]>>,
    parent: Mutex<Arc<LinearStore<P>>>,
}

impl<P: ReadLinearStore> Current<P> {
    pub(crate) fn new(parent: Arc<LinearStore<P>>) -> Self {
        Self {
            parent: Mutex::new(parent),
            old: Default::default(),
        }
    }

    pub(crate) fn reparent(self, parent: Arc<LinearStore<P>>) -> Historical<P> {
        Historical::from_current(self.old, parent)
    }
}

impl<P: ReadLinearStore> ReadLinearStore for Current<P> {
    fn stream_from(
        &self,
        addr: u64,
    ) -> Result<Box<dyn std::io::prelude::Read + '_>, std::io::Error> {
        let parent = self.parent.lock().expect("poisoned lock").clone();
        Ok(Box::new(CurrentStream { parent, addr }))
    }

    fn size(&self) -> Result<u64, std::io::Error> {
        self.parent.lock().expect("poisoned lock").size()
    }
}

struct CurrentStream<P: ReadLinearStore> {
    parent: Arc<LinearStore<P>>,
    addr: u64,
}

impl<P: ReadLinearStore> std::io::prelude::Read for CurrentStream<P> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let size = self.parent.stream_from(self.addr)?.read(buf)?;
        self.addr += size as u64;
        Ok(size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::linear::tests::ConstBacked;
    use std::io::prelude::*;
    use test_case::test_case;

    #[test]
    fn test_current_stream_from() {
        let parent = Arc::new(LinearStore::<ConstBacked> {
            state: ConstBacked::new(&[0, 1, 2]),
        });

        let current = Current::new(parent.clone());
        let mut stream = current.stream_from(0).unwrap();

        // Read the first byte
        let mut buf = vec![0; 1];
        stream.read_exact(&mut buf).unwrap();
        assert_eq!(buf, vec![0]);

        // Read the last two bytes
        let mut buf = vec![0; 2];
        stream.read_exact(&mut buf).unwrap();
        assert_eq!(buf, vec![1, 2]);

        // Read past the end of the stream
        let mut buf = vec![0; 1];
        assert_eq!(stream.read(&mut buf).unwrap(), 0);
    }

    #[test_case(&[]; "0 elements")]
    #[test_case(&[1]; "1 element")]
    #[test_case(&[1,2]; "2 elements")]
    fn test_current_size(data: &'static [u8]) {
        let inner_store = ConstBacked::new(data);

        let parent = Arc::new(LinearStore { state: inner_store });

        let current = Current::new(parent);

        assert_eq!(current.size().unwrap(), data.len() as u64);
    }
}
