// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::fmt::{self, Display, Formatter};
use std::sync::OnceLock;

use firewood::db::{BatchOp as DbBatchOp, Db, DbConfig, DbViewSync as _};
use firewood::manager::RevisionManagerConfig;

/// cbindgen:ignore
static DB: OnceLock<Db> = OnceLock::new();

#[derive(Debug)]
#[repr(C)]
pub struct Value {
    pub len: usize,
    pub data: *const u8,
}

impl Display for Value {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "{:?}", self.as_slice())
    }
}

/// Gets the value associated with the given key from the database.
/// Don't forget to call `free_value` to free the memory associated with the returned `Value`.
#[no_mangle]
pub extern "C" fn get(key: Value) -> Value {
    let db = DB.get_or_init(get_db);
    let root = db.root_hash_sync();
    let Ok(Some(root)) = root else {
        return Value {
            len: 0,
            data: std::ptr::null(),
        };
    };
    let rev = db.revision_sync(root).expect("revision should exist");
    let value = rev
        .val_sync(key.as_slice())
        .expect("get should succeed")
        .unwrap_or_default();
    value.into()
}
#[repr(C)]
#[allow(unused)]
#[no_mangle]
pub struct KeyValue {
    key: Value,
    value: Value,
}

/// Puts the given key-value pairs into the database.
///
/// # Returns
///
/// The current root hash of the database, in Value form.
///
/// # Safety
///
/// This function is unsafe because it dereferences raw pointers.
/// The caller must ensure that `values` is a valid pointer and that it points to an array of `KeyValue` structs of length `nkeys`.
#[no_mangle]
pub unsafe extern "C" fn batch(nkeys: usize, values: *const KeyValue) -> Value {
    let db = DB.get_or_init(get_db);
    let mut batch = Vec::with_capacity(nkeys);
    for i in 0..nkeys {
        let kv = unsafe { values.add(i).as_ref() }.expect("values should be non-null");
        if kv.value.len == 0 {
            batch.push(DbBatchOp::Delete {
                key: kv.key.as_slice(),
            });
            continue;
        }
        batch.push(DbBatchOp::Put {
            key: kv.key.as_slice(),
            value: kv.value.as_slice(),
        });
    }
    let proposal = db.propose_sync(batch).expect("proposal should succeed");
    proposal.commit_sync().expect("commit should succeed");
    hash(db)
}


/// Get the root hash of the latest version of the database
/// Don't forget to call `free_value` to free the memory associated with the returned `Value`.
#[no_mangle]
pub extern "C" fn root_hash() -> Value {
    let db = DB.get_or_init(get_db);
    hash(db)
}

/// cbindgen::ignore
///
/// This function is not exposed to the C API.
/// It returns the current hash of an already-fetched database handle
fn hash(db: &Db) -> Value {
    let root = db.root_hash_sync().unwrap_or_default().unwrap_or_default();
    Value::from(root.as_slice())
}

impl Value {
    pub fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.data, self.len) }
    }
}

impl From<&[u8]> for Value {
    fn from(data: &[u8]) -> Self {
        let boxed: Box<[u8]> = data.into();
        boxed.into()
    }
}

impl From<Box<[u8]>> for Value {
    fn from(data: Box<[u8]>) -> Self {
        let len = data.len();
        let data = Box::leak(data).as_ptr();
        Value { len, data }
    }
}

/// Frees the memory associated with a `Value`.
///
/// # Safety
///
/// This function is unsafe because it dereferences raw pointers.
/// The caller must ensure that `value` is a valid pointer.
#[no_mangle]
pub unsafe extern "C" fn free_value(value: *const Value) {
    if (*value).len == 0 {
        return;
    }
    let recreated_box = unsafe {
        Box::from_raw(std::slice::from_raw_parts_mut(
            (*value).data as *mut u8,
            (*value).len,
        ))
    };
    drop(recreated_box);
}

/// Setup the global database handle. You don't need to call this fuction; it is called automatically by the library.
#[no_mangle]
pub extern "C" fn setup_globals() {
    DB.set(get_db()).expect("db should be set once");
}

fn get_db() -> Db {
    const CACHE_SIZE: usize = 1000000;
    const REVISIONS: usize = 100;

    println!("db initialized (1)");
    let mgrcfg = RevisionManagerConfig::builder()
        .node_cache_size(
            CACHE_SIZE
                .try_into()
                .expect("constant will always be non-zero"),
        )
        .max_revisions(REVISIONS)
        .build();
    let cfg = DbConfig::builder().truncate(true).manager(mgrcfg).build();
    Db::new_sync("rev_db", cfg).expect("db initialization should succeed")
}
