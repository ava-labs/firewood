// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Extract a subset of revisions from one firewood database to another,
//! preserving copy-on-write node sharing across revisions.
//!
//! State root hashes are read from stdin, one hex-encoded hash per line
//! (with or without a leading `0x`). The last root in the input stream
//! becomes the "tip" (current revision) of the output database.
//!
//! The output is a complete, valid firewood database directory containing
//! `firewood.db` and `root_store/`. Only nodes reachable from the requested
//! roots are copied; nodes shared across roots are written exactly once,
//! preserving copy-on-write structure.

use bytemuck;
use clap::Args;
use fjall::{Config, PartitionCreateOptions, PersistMode};
use firewood_storage::{AreaIndex, Child, LinearAddress, Node, NodeStoreHeader, TrieHash};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, Seek, SeekFrom, Write};
use std::os::unix::fs::FileExt;
use std::path::PathBuf;

use crate::DatabasePath;

#[derive(Debug, Args)]
pub struct Options {
    /// Source firewood database directory (must contain `firewood.db` and `root_store/`)
    #[command(flatten)]
    pub source: DatabasePath,

    /// Output directory for the extracted database.
    /// Will be created if it does not exist.
    #[arg(long, value_name = "OUTPUT_DIR")]
    pub output: PathBuf,
}

pub fn run(opts: &Options) -> Result<(), firewood::api::Error> {
    run_inner(opts).map_err(|e| firewood::api::Error::InternalError(e))
}

fn run_inner(opts: &Options) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let source_db = opts.source.dbpath.join("firewood.db");
    let source_rs = opts.source.dbpath.join("root_store");

    // Open source files
    let source_file = File::open(&source_db)
        .map_err(|e| format!("cannot open source database {source_db:?}: {e}"))?;
    let source_ks = Config::new(&source_rs)
        .open()
        .map_err(|e| format!("cannot open source root_store {source_rs:?}: {e}"))?;
    let source_part =
        source_ks.open_partition("firewood", PartitionCreateOptions::default())?;

    // Create output directory and files
    fs::create_dir_all(&opts.output)?;
    let out_db = opts.output.join("firewood.db");
    let out_rs = opts.output.join("root_store");

    let mut out_file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&out_db)
        .map_err(|e| format!("cannot create output database {out_db:?}: {e}"))?;
    // Reserve space for the header; we will fill it in at the end.
    out_file.write_all(&[0u8; NodeStoreHeader::SIZE as usize])?;
    let mut out_offset: u64 = NodeStoreHeader::SIZE;

    let out_ks = Config::new(&out_rs)
        .open()
        .map_err(|e| format!("cannot create output root_store {out_rs:?}: {e}"))?;
    let out_part = out_ks.open_partition("firewood", PartitionCreateOptions::default())?;

    // Disk-backed old_addr→new_addr mapping, kept in a temp directory.
    let temp_dir = tempfile::tempdir()?;
    let map_ks = Config::new(temp_dir.path().join("mapping")).open()?;
    let mapping = map_ks.open_partition("map", PartitionCreateOptions::default())?;

    let mut last_root: Option<([u8; 32], LinearAddress)> = None;
    let mut roots_done: u64 = 0;
    let mut nodes_done: u64 = 0;

    // Read state root hashes from stdin
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let hash_hex = trimmed.strip_prefix("0x").unwrap_or(trimmed);
        let hash_vec =
            hex::decode(hash_hex).map_err(|e| format!("invalid hex {trimmed:?}: {e}"))?;
        let hash: [u8; 32] = hash_vec
            .try_into()
            .map_err(|_| format!("hash must be exactly 32 bytes: {trimmed:?}"))?;

        let Some(addr_raw) = source_part.get(&hash)? else {
            log::warn!("root {trimmed} not found in source root_store — skipping");
            continue;
        };
        let addr_bytes: [u8; 8] = addr_raw.as_ref().try_into()?;
        let old_root = LinearAddress::new(u64::from_be_bytes(addr_bytes))
            .ok_or_else(|| format!("source root address is zero for {trimmed}"))?;

        let (new_root, copied) =
            copy_subtree(old_root, &source_file, &mut out_file, &mut out_offset, &mapping)?;

        out_part.insert(&hash, &new_root.get().to_be_bytes())?;
        out_ks.persist(PersistMode::Buffer)?;

        last_root = Some((hash, new_root));
        roots_done += 1;
        nodes_done += copied;

        if roots_done % 1_000 == 0 {
            log::info!("progress: {roots_done} roots processed, {nodes_done} nodes copied");
        }
    }

    log::info!("extraction complete: {roots_done} roots, {nodes_done} nodes copied");

    let Some((tip_hash, tip_addr)) = last_root else {
        log::warn!("no roots were processed; the output database will be empty");
        return Ok(());
    };

    // Write the final header at offset 0
    let algo: firewood_storage::NodeHashAlgorithm = opts.source.node_hash_algorithm.into();
    let mut header = NodeStoreHeader::new(algo);
    header.set_size(out_offset);
    header.set_root_location(Some((tip_addr, TrieHash::from(tip_hash))));
    // free_lists remain all-None: nothing has been freed in the freshly written DB.

    out_file.seek(SeekFrom::Start(0))?;
    out_file.write_all(bytemuck::bytes_of(&header))?;
    out_file.flush()?;

    log::info!(
        "wrote output header: tip={}, offset={}",
        hex::encode(tip_hash),
        tip_addr.get()
    );
    Ok(())
}

/// Copy the trie rooted at `root_old_addr` from `source` to `output`, reusing
/// any nodes that were already copied by a previous call (shared subtrees).
///
/// Returns `(new_root_address, number_of_newly_copied_nodes)`.
fn copy_subtree(
    root_old_addr: LinearAddress,
    source: &File,
    output: &mut File,
    out_offset: &mut u64,
    mapping: &fjall::PartitionHandle,
) -> Result<(LinearAddress, u64), Box<dyn std::error::Error + Send + Sync>> {
    enum Phase {
        Discover,
        Process,
    }

    let mut stack: Vec<(LinearAddress, Phase)> = vec![(root_old_addr, Phase::Discover)];
    let mut nodes_copied = 0u64;

    while let Some((old_addr, phase)) = stack.pop() {
        let key = old_addr.get().to_be_bytes();

        match phase {
            Phase::Discover => {
                if mapping.get(&key)?.is_some() {
                    // Already copied by a previous root — nothing to do.
                    continue;
                }
                // Push this node back so we process it after all children are done.
                stack.push((old_addr, Phase::Process));
                // Discover children (pushed in reverse so DFS is left-to-right).
                for child_addr in child_addresses(source, old_addr)?.into_iter().rev() {
                    if mapping.get(&child_addr.get().to_be_bytes())?.is_none() {
                        stack.push((child_addr, Phase::Discover));
                    }
                }
            }
            Phase::Process => {
                if mapping.get(&key)?.is_some() {
                    // Became reachable via another path while on the stack.
                    continue;
                }
                // All children have already been copied and are in `mapping`.
                let new_addr =
                    copy_node(old_addr, source, output, out_offset, mapping)?;
                mapping.insert(&key, &new_addr.get().to_be_bytes())?;
                nodes_copied += 1;
            }
        }
    }

    let new_root_raw = mapping
        .get(&root_old_addr.get().to_be_bytes())?
        .ok_or("root node was not copied (internal error)")?;
    let new_root_bytes: [u8; 8] = new_root_raw.as_ref().try_into()?;
    let new_root = LinearAddress::new(u64::from_be_bytes(new_root_bytes))
        .ok_or("new root address is zero (internal error)")?;

    Ok((new_root, nodes_copied))
}

/// Return the `LinearAddress` of every persisted child of the node at `addr`.
fn child_addresses(
    source: &File,
    addr: LinearAddress,
) -> Result<Vec<LinearAddress>, Box<dyn std::error::Error + Send + Sync>> {
    match read_node(source, addr)? {
        Node::Leaf(_) => Ok(vec![]),
        Node::Branch(b) => Ok(b
            .children
            .iter_present()
            .filter_map(|(_, child)| child.persisted_address())
            .collect()),
    }
}

/// Deserialize the node stored at `addr` in the source file.
fn read_node(
    source: &File,
    addr: LinearAddress,
) -> Result<Node, Box<dyn std::error::Error + Send + Sync>> {
    // Read just the area-index byte first so we know how many bytes to read.
    let mut one = [0u8; 1];
    source.read_at(&mut one, addr.get())?;
    let area_index = AreaIndex::new(one[0])
        .ok_or_else(|| format!("invalid area index {} at address {}", one[0], addr.get()))?;

    // Read the full area block: [area_index:1][node_bytes:n][padding:p]
    let area_size = area_index.size() as usize;
    let mut buf = vec![0u8; area_size];
    source.read_at(&mut buf, addr.get())?;

    // `Node::from_reader` consumes the BranchFirstByte / LeafFirstByte as its
    // first byte, so we skip the area-index byte.
    let node = Node::from_reader(&mut &buf[1..])?;
    Ok(node)
}

/// Read the node at `old_addr`, replace every child `LinearAddress` with the
/// corresponding new address from `mapping`, re-encode, write to `output`, and
/// return the new address.
///
/// # Panics / Errors
///
/// Errors if a child address is not yet in `mapping`, which would indicate a
/// bug in the DFS ordering (children must be written before parents).
fn copy_node(
    old_addr: LinearAddress,
    source: &File,
    output: &mut File,
    out_offset: &mut u64,
    mapping: &fjall::PartitionHandle,
) -> Result<LinearAddress, Box<dyn std::error::Error + Send + Sync>> {
    let mut node = read_node(source, old_addr)?;

    // Remap child addresses.  Branch nodes carry up to 16 children, each
    // stored as an AddressWithHash once persisted to disk.
    if let Node::Branch(ref mut branch) = node {
        for (_, child_opt) in branch.children.iter_mut() {
            if let Some(child) = child_opt {
                if let Child::AddressWithHash(addr, _) = child {
                    let mapped = mapping
                        .get(&addr.get().to_be_bytes())?
                        .ok_or_else(|| {
                            format!(
                                "child address {} not in mapping — DFS ordering bug",
                                addr.get()
                            )
                        })?;
                    let mapped_bytes: [u8; 8] = mapped.as_ref().try_into()?;
                    *addr = LinearAddress::new(u64::from_be_bytes(mapped_bytes))
                        .ok_or("remapped child address is zero")?;
                }
            }
        }
    }

    // Re-encode.  Because child LinearAddresses are always 8 bytes regardless
    // of their value, the re-encoded size equals the original size and the
    // AreaIndex is unchanged.
    let mut encoded: Vec<u8> = Vec::with_capacity(256);
    let area_index = node.as_bytes(&mut encoded)?;
    let area_size = area_index.size() as usize;
    encoded.resize(area_size, 0); // zero-pad to fill the area

    let new_addr =
        LinearAddress::new(*out_offset).ok_or("output offset is zero (internal error)")?;
    output.write_all(&encoded)?;
    *out_offset += area_size as u64;

    Ok(new_addr)
}
