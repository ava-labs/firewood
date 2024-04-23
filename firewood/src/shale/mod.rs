// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

pub(crate) use disk_address::DiskAddress;
use std::fmt::Debug;
use std::ops::DerefMut;

use thiserror::Error;

pub mod disk_address;

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

pub trait SendSyncDerefMut: DerefMut + Send + Sync {}

impl<T: Send + Sync + DerefMut> SendSyncDerefMut for T {}

/// A stored item type that can be decoded from or encoded to on-disk raw bytes. An efficient
/// implementation could be directly transmuting to/from a POD struct. But sometimes necessary
/// compression/decompression is needed to reduce disk I/O and facilitate faster in-memory access.
pub trait Storable {
    fn serialized_len(&self) -> u64;
    fn serialize(&self, to: &mut [u8]) -> Result<(), ShaleError>;
    fn deserialize<T>(addr: usize, mem: &T) -> Result<Self, ShaleError>
    where
        Self: Sized;
}

pub fn to_dehydrated(item: &dyn Storable) -> Result<Vec<u8>, ShaleError> {
    let mut buf = vec![0; item.serialized_len() as usize];
    item.serialize(&mut buf)?;
    Ok(buf)
}
