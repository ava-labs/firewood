// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

pub(crate) use disk_address::DiskAddress;
use std::fmt::Debug;
use std::ops::{Deref, DerefMut};

use thiserror::Error;

pub mod disk_address;
pub mod in_mem;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ShaleError {
    #[error("obj invalid: {addr:?} obj: {obj_type:?} error: {error:?}")]
    InvalidObj {
        addr: usize,
        obj_type: &'static str,
        error: &'static str,
    },
    #[error("invalid address length expected: {expected:?} found: {found:?})")]
    InvalidAddressLength { expected: u64, found: u64 },
    #[error("invalid node type")]
    InvalidNodeType,
    #[error("invalid node metadata")]
    InvalidNodeMeta,
    #[error("failed to create view: offset: {offset:?} size: {size:?}")]
    InvalidCacheView { offset: usize, size: u64 },
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Write on immutable cache")]
    ImmutableWrite,
}

// TODO:
// this could probably included with ShaleError,
// but keeping it separate for now as Obj/ObjRef might change in the near future
#[derive(Debug, Error)]
#[error("object cannot be written in the store provided")]
pub struct ObjWriteSizeError;

pub type StoreId = u8;
pub const INVALID_STORE_ID: StoreId = 0xff;

/// A handle that pins and provides a readable access to a portion of a [LinearStore].
pub trait LinearStoreView {
    type DerefReturn: Deref<Target = [u8]>;
    fn as_deref(&self) -> Self::DerefReturn;
}

pub trait SendSyncDerefMut: DerefMut + Send + Sync {}

impl<T: Send + Sync + DerefMut> SendSyncDerefMut for T {}

/// In-memory store that offers access to intervals from a linear byte store, which is usually
/// backed by a cached/memory-mapped pool of the accessed intervals from the underlying linear
/// persistent store. Reads may trigger disk reads to bring data into memory, but writes will
/// *only* be visible in memory -- they do not write to disk.
pub trait LinearStore: Debug + Send + Sync {
    /// Returns a view containing `length` bytes starting from `offset` from this
    /// store. The returned view is pinned.
    fn get_view(
        &self,
        offset: usize,
        length: u64,
    ) -> Option<Box<dyn LinearStoreView<DerefReturn = Vec<u8>>>>;

    /// Returns a handle that allows shared access to this store.
    fn get_shared(&self) -> Box<dyn SendSyncDerefMut<Target = dyn LinearStore>>;

    /// Write the `change` to the linear store starting at `offset`. The change should
    /// be immediately visible to all `LinearStoreView` associated with this linear store.
    fn write(&mut self, offset: usize, change: &[u8]) -> Result<(), ShaleError>;

    /// Returns the identifier of this store.
    fn id(&self) -> StoreId;

    /// Returns whether or not this store is writable
    fn is_writeable(&self) -> bool;
}

/// A stored item type that can be decoded from or encoded to on-disk raw bytes. An efficient
/// implementation could be directly transmuting to/from a POD struct. But sometimes necessary
/// compression/decompression is needed to reduce disk I/O and facilitate faster in-memory access.
pub trait Storable {
    fn serialized_len(&self) -> u64;
    fn serialize(&self, to: &mut [u8]) -> Result<(), ShaleError>;
    fn deserialize<T: LinearStore>(addr: usize, mem: &T) -> Result<Self, ShaleError>
    where
        Self: Sized;
}

pub fn to_dehydrated(item: &dyn Storable) -> Result<Vec<u8>, ShaleError> {
    let mut buf = vec![0; item.serialized_len() as usize];
    item.serialize(&mut buf)?;
    Ok(buf)
}

// TODO remove or delete the code below
// /// A stored view of any [Storable]
// pub struct StoredView<T> {
//     /// The item this stores.
//     item: T,
//     mem: Box<dyn SendSyncDerefMut<Target = dyn LinearStore>>,
//     offset: usize,
//     /// If the serialized length of `item` is greater than this,
//     /// `serialized_len` will return `None`.
//     len_limit: u64,
// }

// impl<T: Debug> Debug for StoredView<T> {
//     fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
//         let StoredView {
//             item,
//             offset,
//             len_limit,
//             mem: _,
//         } = self;
//         f.debug_struct("StoredView")
//             .field("item", item)
//             .field("offset", offset)
//             .field("len_limit", len_limit)
//             .finish()
//     }
// }

// impl<T: Storable> Deref for StoredView<T> {
//     type Target = T;
//     fn deref(&self) -> &T {
//         &self.item
//     }
// }

// impl<T: Storable> StoredView<T> {
//     const fn get_offset(&self) -> usize {
//         self.offset
//     }

//     fn get_mem_store(&self) -> &dyn LinearStore {
//         &**self.mem
//     }

//     fn get_mut_mem_store(&mut self) -> &mut dyn LinearStore {
//         &mut **self.mem
//     }

//     /// Returns the serialized length of the item if it's less than the limit, otherwise `None`.
//     fn serialized_len(&self) -> Option<u64> {
//         let len = self.item.serialized_len();
//         if len > self.len_limit {
//             None
//         } else {
//             Some(len)
//         }
//     }

//     fn serialize(&self, mem_image: &mut [u8]) -> Result<(), ShaleError> {
//         self.item.serialize(mem_image)
//     }

//     fn mut_item_ref(&mut self) -> &mut T {
//         &mut self.item
//     }
// }

// impl<T: Storable + 'static> StoredView<T> {
//     #[inline(always)]
//     fn new<U: LinearStore>(offset: usize, len_limit: u64, store: &U) -> Result<Self, ShaleError> {
//         let item = T::deserialize(offset, store)?;

//         Ok(Self {
//             offset,
//             item,
//             mem: store.get_shared(),
//             len_limit,
//         })
//     }

//     #[inline(always)]
//     fn from_hydrated(
//         offset: usize,
//         len_limit: u64,
//         item: T,
//         store: &dyn LinearStore,
//     ) -> Result<Self, ShaleError> {
//         Ok(Self {
//             offset,
//             item,
//             mem: store.get_shared(),
//             len_limit,
//         })
//     }

//     #[inline(always)]
//     pub fn addr_to_obj<U: LinearStore>(
//         store: &U,
//         ptr: DiskAddress,
//         len_limit: u64,
//     ) -> Result<Obj<T>, ShaleError> {
//         Ok(Obj::from_stored_view(Self::new(
//             ptr.get(),
//             len_limit,
//             store,
//         )?))
//     }

//     #[inline(always)]
//     pub fn item_to_obj(
//         store: &dyn LinearStore,
//         addr: usize,
//         len_limit: u64,
//         item: T,
//     ) -> Result<Obj<T>, ShaleError> {
//         Ok(Obj::from_stored_view(Self::from_hydrated(
//             addr, len_limit, item, store,
//         )?))
//     }
// }

// impl<T: Storable> StoredView<T> {
//     fn new_from_slice(
//         offset: usize,
//         len_limit: u64,
//         item: T,
//         store: &dyn LinearStore,
//     ) -> Result<Self, ShaleError> {
//         Ok(Self {
//             offset,
//             item,
//             mem: store.get_shared(),
//             len_limit,
//         })
//     }

//     pub fn slice<U: Storable + 'static>(
//         s: &Obj<T>,
//         offset: usize,
//         length: u64,
//         item: U,
//     ) -> Result<Obj<U>, ShaleError> {
//         let addr_ = s.value.get_offset() + offset;
//         if s.dirty.is_some() {
//             return Err(ShaleError::InvalidObj {
//                 addr: offset,
//                 obj_type: type_name::<T>(),
//                 error: "dirty write",
//             });
//         }
//         let r = StoredView::new_from_slice(addr_, length, item, s.value.get_mem_store())?;
//         Ok(Obj {
//             value: r,
//             dirty: None,
//         })
//     }
// }

// #[derive(Debug)]
// pub struct ObjCacheInner<T: Storable> {
//     cached: lru::LruCache<DiskAddress, Obj<T>>,
//     pinned: HashMap<DiskAddress, bool>,
//     dirty: HashSet<DiskAddress>,
// }

// /// [ObjRef] pool that is used by [compact::Store] to construct [ObjRef]s.
// #[derive(Debug)]
// pub struct ObjCache<T: Storable>(Arc<RwLock<ObjCacheInner<T>>>);

// impl<T: Storable> ObjCache<T> {
//     pub fn new(capacity: usize) -> Self {
//         Self(Arc::new(RwLock::new(ObjCacheInner {
//             cached: lru::LruCache::new(NonZeroUsize::new(capacity).expect("non-zero cache size")),
//             pinned: HashMap::new(),
//             dirty: HashSet::new(),
//         })))
//     }

//     fn lock(&self) -> RwLockWriteGuard<ObjCacheInner<T>> {
//         #[allow(clippy::unwrap_used)]
//         self.0.write().unwrap()
//     }

//     #[inline(always)]
//     fn get(&self, ptr: DiskAddress) -> Result<Option<Obj<T>>, ShaleError> {
//         #[allow(clippy::unwrap_used)]
//         let mut inner = self.0.write().unwrap();

//         let obj_ref = inner.cached.pop(&ptr).map(|r| {
//             // insert and set to `false` if you can
//             // When using `get` in parallel, one should not `write` to the same address
//             inner
//                 .pinned
//                 .entry(ptr)
//                 .and_modify(|is_pinned| *is_pinned = false)
//                 .or_insert(false);

//             // if we need to re-enable this code, it has to return from the outer function
//             //
//             // return if inner.pinned.insert(ptr, false).is_some() {
//             //     Err(ShaleError::InvalidObj {
//             //         addr: ptr.addr(),
//             //         obj_type: type_name::<T>(),
//             //         error: "address already in use",
//             //     })
//             // } else {
//             //     Ok(Some(ObjRef {
//             //         inner: Some(r),
//             //         cache: Self(self.0.clone()),
//             //         _life: PhantomData,
//             //     }))
//             // };

//             // always return instead of the code above
//             r
//         });

//         Ok(obj_ref)
//     }

//     #[inline(always)]
//     fn put(&self, inner: Obj<T>) -> Obj<T> {
//         let ptr = inner.as_addr();
//         self.lock().pinned.insert(ptr, false);
//         inner
//     }

//     #[inline(always)]
//     pub fn pop(&self, ptr: DiskAddress) {
//         let mut inner = self.lock();
//         if let Some(f) = inner.pinned.get_mut(&ptr) {
//             *f = true
//         }
//         if let Some(mut r) = inner.cached.pop(&ptr) {
//             r.dirty = None
//         }
//         inner.dirty.remove(&ptr);
//     }

//     pub fn flush_dirty(&self) -> Option<()> {
//         let mut inner = self.lock();
//         if !inner.pinned.is_empty() {
//             return None;
//         }
//         for ptr in std::mem::take(&mut inner.dirty) {
//             if let Some(r) = inner.cached.peek_mut(&ptr) {
//                 r.flush_dirty()
//             }
//         }
//         Some(())
//     }
// }
