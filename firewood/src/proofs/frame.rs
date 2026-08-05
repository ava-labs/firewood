// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! # Compressed proof body framing
//!
//! Serialized proofs carry their body inside a single zstd frame:
//!
//! ```text
//! Header (32 bytes, uncompressed)
//! varint(uncompressed_len)     // exact byte length of the canonical body
//! zstd frame                   // compresses exactly the canonical body bytes
//! ```
//!
//! The canonical body encoding (proof nodes, key-value pairs, batch
//! operations — see `ser.rs`) is unchanged; compression wraps around it.
//! Measured on C-Chain mainnet replays (blocks 50M–56.44M and 75M–76M),
//! compressed proofs at the production state-sync message size
//! (2 MiB − 4 KiB) are 46–57% smaller than their canonical body, which
//! roughly halves sync bandwidth and message count. Compression costs
//! ~3.3 ms and decompression ~0.5 ms per MiB of body on a modern core
//! (measured on c8id.4xlarge instance, Intel Xeon Scalable, 3.9GHz).
//! The `proof_serde` criterion benchmark exercises this path.
//!
//! ## Determinism and canonicality
//!
//! The producer pins the zstd parameters: level [`ZSTD_LEVEL`], single
//! frame, no dictionary, no checksum. For a fixed zstd library version
//! the output is deterministic, but zstd does not guarantee bit-identical
//! output across library upgrades. **The canonical form of a proof is its
//! decompressed body.** Anything that hashes, signs, or byte-compares
//! proofs must operate on the body bytes, never on the compressed wire
//! bytes.
//!
//! ## Decode bounds
//!
//! The wire bytes are attacker-controlled, so every bound below is
//! enforced *before* the body allocation:
//!
//! - The declared `uncompressed_len` is capped at [`MAX_DECOMPRESSED_LEN`].
//! - The remainder must contain exactly one zstd frame spanning all of it
//!   (`ZSTD_findFrameCompressedSize` must equal the remainder length).
//!   `ZSTD_decompressDCtx` would otherwise accept concatenated frames and
//!   silently skip skippable frames, opening an unbounded trailing-bytes
//!   padding channel.
//! - The declared length must equal the content size recorded in the zstd
//!   frame header, so a declared length can never over-allocate beyond
//!   what the frame itself commits to producing.
//! - The declared length may not exceed [`MAX_COMPRESSION_RATIO`] × the
//!   compressed frame length, so the decoder's allocation is bounded by
//!   the bytes the peer actually sent, not by a number it declared.
//! - The decompressed byte count must equal the declared length exactly.
//!
//! The decompressed body then passes through the ordinary body parser
//! with all of its existing validation.

use firewood_metrics::{firewood_counter, firewood_histogram};
use integer_encoding::VarInt;

use super::reader::ReadError;

/// zstd compression level pinned by the producer rule. Level 3 is zstd's
/// default and the best size/CPU trade-off measured on C-Chain proof
/// bodies: higher levels buy <1% smaller output for multiples of the
/// encode cost.
pub(super) const ZSTD_LEVEL: i32 = 3;

/// Upper bound on the declared uncompressed body length.
///
/// Proof bodies in the state-sync regime are ~2 MiB (the production
/// message size is 2 MiB − 4 KiB). 32 MiB is 16× that: enough headroom
/// for oversized proofs from other callers while keeping the worst-case
/// decoder allocation tied to the protocol's message size rather than
/// orders of magnitude beyond it.
pub(super) const MAX_DECOMPRESSED_LEN: usize = 32 * 1024 * 1024;

/// Upper bound on the ratio between the declared uncompressed body length
/// and the compressed frame length.
///
/// Honest proof bodies measure ~2.3× (see the module docs); bodies are
/// dominated by 32-byte hashes and hashed keys, which do not compress.
/// 128× is far beyond anything an honest body reaches while ensuring a
/// peer must actually send `decoded_len / 128` bytes to make the decoder
/// allocate and write `decoded_len` bytes.
pub(super) const MAX_COMPRESSION_RATIO: usize = 128;

// The `expected` strings in `ReadError::InvalidItem` are `&'static str`,
// so the constants above appear in them as literals. Keep them in sync.
const _: () = assert!(
    MAX_DECOMPRESSED_LEN == 33_554_432,
    "update the `expected` message literals below when changing MAX_DECOMPRESSED_LEN"
);
const _: () = assert!(
    MAX_COMPRESSION_RATIO == 128,
    "update the `expected` message literals below when changing MAX_COMPRESSION_RATIO"
);

/// Appends `varint(body_len)` followed by the already-compressed frame,
/// recording the producer-side size metrics.
fn append_frame(body_len: usize, compressed: &[u8], out: &mut Vec<u8>, proof_type: &'static str) {
    let mut buf = [0u8; 10];
    let n = body_len.encode_var(&mut buf);
    #[expect(
        clippy::indexing_slicing,
        reason = "encode_var writes at most buf.len() bytes and returns that count"
    )]
    out.extend_from_slice(&buf[..n]);
    out.extend_from_slice(compressed);
    record_frame_metrics("compress", proof_type, body_len, compressed.len());
}

/// Records the compressed frame size and the body/frame compression ratio.
#[expect(
    clippy::cast_precision_loss,
    reason = "metric observations are approximate; lengths are far below 2^52"
)]
fn record_frame_metrics(
    op: &'static str,
    proof_type: &'static str,
    body_len: usize,
    frame_len: usize,
) {
    firewood_histogram!(cheap: PROOF_COMPRESSED_BYTES, "op" => op, "proof_type" => proof_type)
        .record(frame_len as f64);
    firewood_histogram!(cheap: PROOF_COMPRESSION_RATIO, "op" => op, "proof_type" => proof_type)
        .record(body_len as f64 / (frame_len.max(1)) as f64);
}

/// Appends `varint(body.len())` followed by the zstd frame compressing
/// `body`.
#[cfg(test)]
pub(super) fn write_compressed_body(body: &[u8], out: &mut Vec<u8>, proof_type: &'static str) {
    let compressed = zstd::bulk::compress(body, ZSTD_LEVEL)
        .expect("zstd compression of an in-memory buffer cannot fail");
    append_frame(body.len(), &compressed, out, proof_type);
}

/// Compresses the tail of `out` starting at `body_start` (the canonical
/// body, serialized in place after the header) and replaces it with
/// `varint(body_len) :: zstd frame`.
///
/// Serializing the body directly into `out` and compressing it in place
/// keeps `write_to_vec` down to a single scratch allocation (zstd's
/// output buffer), so callers reusing one output buffer across proofs
/// don't pay a fresh body allocation per call.
pub(super) fn compress_body_in_place(
    out: &mut Vec<u8>,
    body_start: usize,
    proof_type: &'static str,
) {
    let body_len = out
        .len()
        .checked_sub(body_start)
        .expect("body_start is an offset into out");
    #[expect(
        clippy::indexing_slicing,
        reason = "body_start <= out.len() is checked by the subtraction above"
    )]
    let compressed = zstd::bulk::compress(&out[body_start..], ZSTD_LEVEL)
        .expect("zstd compression of an in-memory buffer cannot fail");
    out.truncate(body_start);
    append_frame(body_len, &compressed, out, proof_type);
}

/// Increments the decode-failure counter for `reason` and passes `err`
/// through, so every early return below is recorded.
fn fail(reason: &'static str, err: ReadError) -> ReadError {
    firewood_counter!(PROOF_DECODE_FAILURES, "reason" => reason).increment(1);
    err
}

/// Enforces the pre-allocation frame bounds: exactly one zstd frame
/// spanning the entire remainder, a frame-header content size equal to
/// the declared length, and the declared length within
/// [`MAX_COMPRESSION_RATIO`] of the frame length.
fn validate_frame(
    frame: &[u8],
    decoded_len: usize,
    wire_offset: usize,
    frame_offset: usize,
) -> Result<(), ReadError> {
    // Exactly one zstd frame must span the entire remainder. The raw
    // decompressor accepts concatenated frames and silently skips
    // skippable frames, so anything after the first frame would be an
    // attacker-controlled padding channel.
    let frame_size = zstd::zstd_safe::find_frame_compressed_size(frame).map_err(|code| {
        fail(
            "invalid_frame",
            ReadError::InvalidItem {
                item: "compressed body frame",
                offset: frame_offset,
                expected: "a valid zstd frame",
                found: zstd::zstd_safe::get_error_name(code).to_owned(),
            },
        )
    })?;
    if frame_size != frame.len() {
        return Err(fail(
            "trailing_data",
            ReadError::InvalidItem {
                item: "compressed body frame",
                offset: frame_offset,
                expected: "a single zstd frame spanning the entire remainder",
                found: format!("frame ends after {frame_size} of {} bytes", frame.len()),
            },
        ));
    }

    // The declared varint is pure attacker input; the producer always
    // records the content size in the frame header, so the two must
    // agree. This kills over-allocation via an inflated declaration.
    match zstd::zstd_safe::get_frame_content_size(frame) {
        Ok(Some(content_size)) if content_size == decoded_len as u64 => {}
        Ok(content_size) => {
            return Err(fail(
                "content_size_mismatch",
                ReadError::InvalidItem {
                    item: "compressed body length",
                    offset: wire_offset,
                    expected: "declared length equal to the zstd frame header's content size",
                    found: match content_size {
                        Some(n) => format!("declared {decoded_len}, frame header says {n}"),
                        None => format!("declared {decoded_len}, frame header has no content size"),
                    },
                },
            ));
        }
        Err(_) => {
            return Err(fail(
                "invalid_frame",
                ReadError::InvalidItem {
                    item: "compressed body frame",
                    offset: frame_offset,
                    expected: "a zstd frame with a readable header",
                    found: "unreadable frame header".to_owned(),
                },
            ));
        }
    }

    // Bound the allocation by bytes the peer actually sent, not by the
    // (cross-checked but still peer-chosen) declared length.
    if decoded_len > frame.len().saturating_mul(MAX_COMPRESSION_RATIO) {
        return Err(fail(
            "ratio_exceeded",
            ReadError::InvalidItem {
                item: "compressed body length",
                offset: wire_offset,
                expected: "declared length <= 128x (MAX_COMPRESSION_RATIO) the frame length",
                found: format!("{decoded_len} > {} * 128", frame.len()),
            },
        ));
    }
    Ok(())
}

/// Decompresses a framed proof body (`varint(uncompressed_len) :: zstd
/// frame`), enforcing the decode bounds described in the module docs.
///
/// `wire_offset` is the offset of `data` within the full wire message
/// (the length of the uncompressed header), so reported error offsets
/// line up with a hexdump of the wire bytes.
pub(super) fn decompress_body(
    data: &[u8],
    wire_offset: usize,
    proof_type: &'static str,
) -> Result<Vec<u8>, ReadError> {
    let (decoded_len, varint_len) = usize::decode_var(data).ok_or_else(|| {
        fail(
            "incomplete_length",
            ReadError::IncompleteItem {
                item: "compressed body length",
                offset: wire_offset,
                expected: 1,
                found: 0,
            },
        )
    })?;
    if decoded_len > MAX_DECOMPRESSED_LEN {
        return Err(fail(
            "over_cap",
            ReadError::InvalidItem {
                item: "compressed body length",
                offset: wire_offset,
                expected: "<= 33554432 bytes (MAX_DECOMPRESSED_LEN, 32 MiB)",
                found: decoded_len.to_string(),
            },
        ));
    }
    let frame_offset = wire_offset.saturating_add(varint_len);
    #[expect(
        clippy::indexing_slicing,
        reason = "decode_var consumes at most data.len() bytes"
    )]
    let frame = &data[varint_len..];
    if frame.is_empty() {
        return Err(fail(
            "missing_frame",
            ReadError::IncompleteItem {
                item: "compressed body frame",
                offset: frame_offset,
                expected: 1,
                found: 0,
            },
        ));
    }

    validate_frame(frame, decoded_len, wire_offset, frame_offset)?;

    let mut body = vec![0u8; decoded_len];
    let written = zstd::bulk::decompress_to_buffer(frame, &mut body).map_err(|err| {
        fail(
            "invalid_frame",
            ReadError::InvalidItem {
                item: "compressed body frame",
                offset: frame_offset,
                expected: "a valid zstd frame with no trailing bytes",
                found: err.to_string(),
            },
        )
    })?;
    if written != decoded_len {
        return Err(fail(
            "length_mismatch",
            ReadError::InvalidItem {
                item: "compressed body length",
                offset: wire_offset,
                expected: "declared length equal to decompressed length",
                found: format!("declared {decoded_len}, got {written}"),
            },
        ));
    }
    record_frame_metrics("decompress", proof_type, decoded_len, frame.len());
    Ok(body)
}
