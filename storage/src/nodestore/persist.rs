// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! # Persist Module
//!
//! This module handles all persistence operations for the nodestore, including writing
//! headers, nodes, and metadata to storage with support for different I/O backends.
//!
//! ## I/O Backend Support
//!
//! This module supports multiple I/O backends through conditional compilation:
//!
//! - **Standard I/O** - `#[cfg(not(feature = "io-uring"))]` - Uses standard file operations
//! - **io-uring** - `#[cfg(feature = "io-uring")]` - Uses Linux io-uring for async I/O
//!
//! This feature flag is automatically enabled when running on Linux, and disabled for all other platforms.
//!
//! The io-uring implementation provides:
//! - Asynchronous batch operations
//! - Reduced system call overhead
//! - Better performance for high-throughput workloads
//!
//! ## Performance Considerations
//!
//! - Nodes are written in batches to minimize I/O overhead
//! - Metrics are collected for flush operation timing
//! - Memory-efficient serialization with pre-allocated buffers
//! - Ring buffer management for io-uring operations

use crate::linear::FileIoError;
use coarsetime::Instant;
use metrics::counter;

#[cfg(feature = "io-uring")]
use crate::logger::trace;

use crate::{FileBacked, MaybePersistedNode, NodeReader, WritableStorage};

use super::header::NodeStoreHeader;
use super::{Committed, NodeStore, RootReader};

impl<T, S: WritableStorage> NodeStore<T, S> {
    /// Persist the header from this proposal to storage.
    ///
    /// # Errors
    ///
    /// Returns a [`FileIoError`] if the header cannot be written.
    pub fn flush_header(&self) -> Result<(), FileIoError> {
        let header_bytes = bytemuck::bytes_of(&self.header);
        self.storage.write(0, header_bytes)?;
        Ok(())
    }

    /// Persist the header, including all the padding
    /// This is only done the first time we write the header
    ///
    /// # Errors
    ///
    /// Returns a [`FileIoError`] if the header cannot be written.
    pub fn flush_header_with_padding(&self) -> Result<(), FileIoError> {
        let header_bytes = bytemuck::bytes_of(&self.header)
            .iter()
            .copied()
            .chain(std::iter::repeat_n(0u8, NodeStoreHeader::EXTRA_BYTES))
            .collect::<Box<[u8]>>();
        debug_assert_eq!(header_bytes.len(), NodeStoreHeader::SIZE as usize);

        self.storage.write(0, &header_bytes)?;
        Ok(())
    }
}

/// Iterator that returns unpersisted nodes in depth first order.
///
/// This iterator assumes the root node is unpersisted and will return it as the
/// last item. It looks at each node and traverses the children in depth first order.
/// A stack of child iterators is maintained to properly handle nested branches.
struct UnPersistedNodeIterator<'a, N> {
    store: &'a N,
    stack: Vec<MaybePersistedNode>,
    child_iter_stack: Vec<Box<dyn Iterator<Item = MaybePersistedNode> + 'a>>,
}

impl<'a, N: NodeReader + RootReader> UnPersistedNodeIterator<'a, N> {
    /// Creates a new iterator over unpersisted nodes in depth-first order.
    fn new(store: &'a N) -> Self {
        let root = store.root_as_maybe_persisted_node();

        // we must have an unpersisted root node to use this iterator
        // It's hard to tell at compile time if this is the case, so we assert it here
        // TODO: can we use another trait or generic to enforce this?
        debug_assert!(root.as_ref().is_none_or(|r| r.unpersisted().is_some()));
        let (child_iter_stack, stack) = if let Some(root) = root {
            if let Some(branch) = root
                .as_shared_node(store)
                .expect("in memory, so no io")
                .as_branch()
            {
                // Create an iterator over unpersisted children
                let unpersisted_children: Vec<MaybePersistedNode> = branch
                    .children
                    .iter()
                    .filter_map(|child_opt| {
                        child_opt
                            .as_ref()
                            .and_then(|child| child.unpersisted().cloned())
                    })
                    .collect();

                (
                    vec![Box::new(unpersisted_children.into_iter())
                        as Box<dyn Iterator<Item = MaybePersistedNode> + 'a>],
                    vec![root],
                )
            } else {
                // root is a leaf
                (vec![], vec![root])
            }
        } else {
            (vec![], vec![])
        };

        Self {
            store,
            stack,
            child_iter_stack,
        }
    }
}

impl<N: NodeReader + RootReader> Iterator for UnPersistedNodeIterator<'_, N> {
    type Item = MaybePersistedNode;

    fn next(&mut self) -> Option<Self::Item> {
        // Try to get the next child from the current child iterator
        while let Some(current_iter) = self.child_iter_stack.last_mut() {
            if let Some(next_child) = current_iter.next() {
                let shared_node = next_child
                    .as_shared_node(self.store)
                    .expect("in memory, so IO is impossible");

                if shared_node.is_leaf() {
                    return Some(next_child);
                }

                // It's a branch, so we need to get its children
                if let Some(branch) = shared_node.as_branch() {
                    // Create an iterator over unpersisted children
                    let unpersisted_children: Vec<MaybePersistedNode> = branch
                        .children
                        .iter()
                        .filter_map(|child_opt| {
                            child_opt
                                .as_ref()
                                .and_then(|child| child.unpersisted().cloned())
                        })
                        .collect();

                    // Push new child iterator to the stack
                    if !unpersisted_children.is_empty() {
                        self.child_iter_stack
                            .push(Box::new(unpersisted_children.into_iter()));
                    }
                }

                self.stack.push(next_child); // visit this node after the children
            } else {
                // Current iterator is exhausted, remove it
                self.child_iter_stack.pop();
            }
        }

        // No more children to process, pop the next node from the stack
        self.stack.pop()
    }
}

impl<S: WritableStorage> NodeStore<Committed, S> {
    /// Persist the freelist from this proposal to storage.
    #[fastrace::trace(short_name = true)]
    pub fn flush_freelist(&self) -> Result<(), FileIoError> {
        // Write the free lists to storage
        let free_list_bytes = bytemuck::bytes_of(self.header.free_lists());
        let free_list_offset = NodeStoreHeader::free_lists_offset();
        self.storage.write(free_list_offset, free_list_bytes)?;
        Ok(())
    }

    /// Persist all the nodes of a proposal to storage.
    #[fastrace::trace(short_name = true)]
    #[cfg(not(feature = "io-uring"))]
    pub fn flush_nodes(&mut self) -> Result<(), FileIoError> {
        let flush_start = Instant::now();

        // Collect all unpersisted nodes first to avoid mutating self while iterating
        // TODO: we can probably do better than this
        let unpersisted_nodes: Vec<MaybePersistedNode> = {
            let unpersisted_iter = UnPersistedNodeIterator::new(self);
            unpersisted_iter.collect()
        };

        // Collect addresses and nodes for caching
        let mut cached_nodes = Vec::new();

        // Now process them with mutable access
        for node in unpersisted_nodes {
            let shared_node = node.as_shared_node(self).expect("in memory, so no IO");
            let mut serialized = Vec::new();
            shared_node.as_bytes(0, &mut serialized);
            let (persisted_address, _) = self.allocate_node(serialized.as_slice())?;
            self.storage
                .write(persisted_address.get(), serialized.as_slice())?;
            node.persist_at(persisted_address);

            // Collect for cache
            cached_nodes.push((persisted_address, shared_node));
        }

        self.storage.write_cached_nodes(&cached_nodes)?;

        let flush_time = flush_start.elapsed().as_millis();
        counter!("firewood.flush_nodes").increment(flush_time);

        Ok(())
    }
}

impl NodeStore<Committed, FileBacked> {
    /// Persist the entire nodestore to storage.
    ///
    /// This method performs a complete persistence operation by:
    /// 1. Flushing all nodes to storage
    /// 2. Setting the root address in the header  
    /// 3. Flushing the header to storage
    ///
    /// # Errors
    ///
    /// Returns a [`FileIoError`] if any of the persistence operations fail.
    #[fastrace::trace(short_name = true)]
    pub fn persist(&mut self) -> Result<(), FileIoError> {
        // First persist all the nodes
        self.flush_nodes()?;

        // Set the root address in the header based on the persisted root
        let root_address = self
            .kind
            .root
            .as_ref()
            .and_then(crate::MaybePersistedNode::as_linear_address);
        self.header.set_root_address(root_address);

        // Finally persist the header
        self.flush_header()?;

        Ok(())
    }

    /// Persist all the nodes of a proposal to storage.
    #[fastrace::trace(short_name = true)]
    #[cfg(feature = "io-uring")]
    pub fn flush_nodes(&mut self) -> Result<(), FileIoError> {
        use std::pin::Pin;

        #[derive(Clone, Debug)]
        struct PinnedBufferEntry {
            pinned_buffer: Pin<Box<[u8]>>,
            offset: Option<u64>,
        }

        /// Helper function to handle completion queue entries and check for errors
        fn handle_completion_queue(
            storage: &FileBacked,
            completion_queue: io_uring::cqueue::CompletionQueue<'_>,
            saved_pinned_buffers: &mut [PinnedBufferEntry],
        ) -> Result<(), FileIoError> {
            for entry in completion_queue {
                let item = entry.user_data() as usize;
                let pbe = saved_pinned_buffers
                    .get_mut(item)
                    .expect("should be an index into the array");

                if entry.result()
                    != pbe
                        .pinned_buffer
                        .len()
                        .try_into()
                        .expect("buffer should be small enough")
                {
                    let error = if entry.result() >= 0 {
                        std::io::Error::other("Partial write")
                    } else {
                        std::io::Error::from_raw_os_error(0 - entry.result())
                    };
                    return Err(storage.file_io_error(
                        error,
                        pbe.offset.expect("offset should be Some"),
                        Some("write failure".to_string()),
                    ));
                }
                pbe.offset = None;
            }
            Ok(())
        }

        const RINGSIZE: usize = FileBacked::RINGSIZE as usize;

        let flush_start = Instant::now();

        // Collect all unpersisted nodes first to avoid mutating self while iterating
        let unpersisted_nodes: Vec<MaybePersistedNode> = {
            let unpersisted_iter = UnPersistedNodeIterator::new(self);
            unpersisted_iter.collect()
        };

        // Collect addresses and nodes for caching
        let mut cached_nodes = Vec::new();

        let mut ring = self.storage.ring.lock().expect("poisoned lock");
        let mut saved_pinned_buffers = vec![
            PinnedBufferEntry {
                pinned_buffer: Pin::new(Box::new([0; 0])),
                offset: None,
            };
            RINGSIZE
        ];

        // Process each unpersisted node
        for node in unpersisted_nodes {
            let shared_node = node.as_shared_node(self).expect("in memory, so no IO");
            let mut serialized = Vec::with_capacity(100); // TODO: better size? we can guess branches are larger
            shared_node.as_bytes(0, &mut serialized);
            let (persisted_address, _) = self.allocate_node(serialized.as_slice())?;
            let mut serialized = serialized.into_boxed_slice();

            loop {
                // Find the first available write buffer, enumerate to get the position for marking it completed
                if let Some((pos, pbe)) = saved_pinned_buffers
                    .iter_mut()
                    .enumerate()
                    .find(|(_, pbe)| pbe.offset.is_none())
                {
                    pbe.pinned_buffer = std::pin::Pin::new(std::mem::take(&mut serialized));
                    pbe.offset = Some(persisted_address.get());

                    let submission_queue_entry = self
                        .storage
                        .make_op(&pbe.pinned_buffer)
                        .offset(persisted_address.get())
                        .build()
                        .user_data(pos as u64);

                    // SAFETY: the submission_queue_entry's found buffer must not move or go out of scope
                    // until the operation has been completed. This is ensured by having a Some(offset)
                    // and not marking it None until the kernel has said it's done below.
                    #[expect(unsafe_code)]
                    while unsafe { ring.submission().push(&submission_queue_entry) }.is_err() {
                        ring.submitter().squeue_wait().map_err(|e| {
                            self.storage.file_io_error(
                                e,
                                persisted_address.get(),
                                Some("io-uring squeue_wait".to_string()),
                            )
                        })?;
                        trace!("submission queue is full");
                        counter!("ring.full").increment(1);
                    }
                    break;
                }
                // if we get here, that means we couldn't find a place to queue the request, so wait for at least one operation
                // to complete, then handle the completion queue
                counter!("ring.full").increment(1);
                ring.submit_and_wait(1).map_err(|e| {
                    self.storage
                        .file_io_error(e, 0, Some("io-uring submit_and_wait".to_string()))
                })?;
                let completion_queue = ring.completion();
                trace!("competion queue length: {}", completion_queue.len());
                handle_completion_queue(
                    &self.storage,
                    completion_queue,
                    &mut saved_pinned_buffers,
                )?;
            }

            // Mark node as persisted and collect for cache
            node.persist_at(persisted_address);
            cached_nodes.push((persisted_address, shared_node));
        }
        let pending = saved_pinned_buffers
            .iter()
            .filter(|pbe| pbe.offset.is_some())
            .count();
        ring.submit_and_wait(pending).map_err(|e| {
            self.storage
                .file_io_error(e, 0, Some("io-uring final submit_and_wait".to_string()))
        })?;

        handle_completion_queue(&self.storage, ring.completion(), &mut saved_pinned_buffers)?;

        debug_assert!(
            !saved_pinned_buffers.iter().any(|pbe| pbe.offset.is_some()),
            "Found entry with offset still set: {:?}",
            saved_pinned_buffers.iter().find(|pbe| pbe.offset.is_some())
        );

        self.storage.write_cached_nodes(&cached_nodes)?;
        debug_assert!(ring.completion().is_empty());

        let flush_time = flush_start.elapsed().as_millis();
        counter!("firewood.flush_nodes").increment(flush_time);

        Ok(())
    }
}
