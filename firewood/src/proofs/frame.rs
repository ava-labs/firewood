// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! # Compressed proof body framing
//!
//! Serialized proofs carry their body inside a single zstd frame:
//!
//! ```text
//! Header (32 bytes, uncompressed)
//! zstd frame                   // compresses exactly the canonical body bytes
//! ```
//!
//! ## Determinism and canonicality
//!
//! The producer pins the zstd parameters: level [`ZSTD_LEVEL`], single
//! frame, no dictionary, no checksum. zstd output is not bit-identical
//! across library versions; **the decompressed body is the canonical
//! form** — hash, sign, or compare that, never the wire bytes.
//!
//! ## Decode bounds
//!
//! The wire bytes are attacker-controlled, so every bound below is
//! enforced *before* the body allocation:
//!
//! - Exactly one zstd frame must span the entire remainder.
//! - The frame header must declare a content size, capped at
//!   [`MAX_DECOMPRESSED_LEN`] and at [`MAX_COMPRESSION_RATIO`] × the
//!   compressed frame length.
//! - The decompressed byte count must equal the declared content size.
//!
//! The decompressed body then passes through the ordinary body parser.
//! Frame-layer errors report offsets into the wire bytes; body-parser
//! errors report offsets into the decompressed body.

use firewood_metrics::{firewood_counter, firewood_histogram};

use super::reader::ReadError;
use super::types::ProofType;

/// zstd compression level pinned by the producer rule.
/// Level 3 is zstd's default and best measured tradeoff.
pub(super) const ZSTD_LEVEL: i32 = 3;

/// Upper bound on the declared uncompressed body length.
pub(super) const MAX_DECOMPRESSED_LEN: usize = 32 * 1024 * 1024;

/// Upper bound on the ratio between the declared uncompressed body length
/// and the compressed frame length. Bounds the decoder's allocation by the
/// bytes the peer actually sent (decompression-bomb defense).
pub(super) const MAX_COMPRESSION_RATIO: usize = 128;

/// Records the compressed frame size and the body/frame compression ratio.
#[expect(
    clippy::cast_precision_loss,
    reason = "metric observations are approximate; lengths are far below 2^52"
)]
fn record_frame_metrics(
    op: &'static str,
    proof_type: ProofType,
    body_len: usize,
    frame_len: usize,
) {
    let kind = proof_type.name();
    firewood_histogram!(cheap: PROOF_COMPRESSED_BYTES, "op" => op, "kind" => kind)
        .record(frame_len as f64);
    firewood_histogram!(cheap: PROOF_COMPRESSION_RATIO, "op" => op, "kind" => kind)
        .record(body_len as f64 / (frame_len.max(1)) as f64);
}

/// Appends the zstd frame compressing `body`.
#[cfg(test)]
pub(super) fn write_compressed_body(body: &[u8], out: &mut Vec<u8>, proof_type: ProofType) {
    let compressed =
        zstd::bulk::compress(body, ZSTD_LEVEL).expect("couldn't allocate zstd context");
    record_frame_metrics("compress", proof_type, body.len(), compressed.len());
    out.extend_from_slice(&compressed);
}

/// Compresses `out[body_start..]` (the canonical body, serialized in place
/// after the header) into a zstd frame, replacing it. Avoids a separate
/// body buffer.
pub(super) fn compress_body_in_place(out: &mut Vec<u8>, body_start: usize, proof_type: ProofType) {
    let body_len = out
        .len()
        .checked_sub(body_start)
        .expect("body_start is an offset into out");
    #[expect(
        clippy::indexing_slicing,
        reason = "body_start <= out.len() is checked by the subtraction above"
    )]
    let compressed = zstd::bulk::compress(&out[body_start..], ZSTD_LEVEL)
        .expect("couldn't allocate zstd context");
    out.truncate(body_start);
    out.extend_from_slice(&compressed);
    record_frame_metrics("compress", proof_type, body_len, compressed.len());
}

/// Increments the decode-failure counter for `reason` and passes `err`
/// through, so every early return below is recorded.
fn fail(reason: &'static str, err: ReadError) -> ReadError {
    firewood_counter!(PROOF_DECODE_FAILURES, "reason" => reason).increment(1);
    err
}

/// Builds a recorded [`ReadError::InvalidItem`] — [`fail`] with the error
/// construction folded in, since every invalid-frame path below reports
/// the same error shape.
fn invalid(
    reason: &'static str,
    item: &'static str,
    offset: usize,
    expected: &'static str,
    found: impl ToString,
) -> ReadError {
    fail(
        reason,
        ReadError::InvalidItem {
            item,
            offset,
            expected,
            found: found.to_string(),
        },
    )
}

/// Enforces the pre-allocation frame bounds and returns the frame's
/// declared content size: exactly one zstd frame spanning the entire
/// remainder, with a content size within [`MAX_DECOMPRESSED_LEN`] and
/// within [`MAX_COMPRESSION_RATIO`] of the frame length.
fn validate_frame(frame: &[u8], frame_offset: usize) -> Result<usize, ReadError> {
    // `find_frame_compressed_size` measures only the *first* frame.
    // Requiring it to span the entire remainder leaves no room for
    // trailing concatenated or skippable frames, which the raw
    // decompressor would otherwise silently accept — an
    // attacker-controlled padding channel.
    let frame_size = zstd::zstd_safe::find_frame_compressed_size(frame).map_err(|code| {
        invalid(
            "invalid_frame",
            "compressed body frame",
            frame_offset,
            "a valid zstd frame",
            zstd::zstd_safe::get_error_name(code),
        )
    })?;
    if frame_size != frame.len() {
        return Err(invalid(
            "trailing_data",
            "compressed body frame",
            frame_offset,
            "a single zstd frame spanning the entire remainder",
            format!("frame ends after {frame_size} of {} bytes", frame.len()),
        ));
    }

    let content_size = match zstd::zstd_safe::get_frame_content_size(frame) {
        Ok(Some(content_size)) => content_size,
        Ok(None) => {
            return Err(invalid(
                "missing_content_size",
                "compressed body length",
                frame_offset,
                "a zstd frame header with a content size",
                "no content size",
            ));
        }
        Err(_) => {
            return Err(invalid(
                "invalid_frame",
                "compressed body frame",
                frame_offset,
                "a zstd frame with a readable header",
                "unreadable frame header",
            ));
        }
    };

    let decoded_len = usize::try_from(content_size)
        .ok()
        .filter(|&n| n <= MAX_DECOMPRESSED_LEN)
        .ok_or_else(|| {
            invalid(
                "over_cap",
                "compressed body length",
                frame_offset,
                "content size within MAX_DECOMPRESSED_LEN",
                format!("{content_size} > {MAX_DECOMPRESSED_LEN}"),
            )
        })?;

    // Bound the allocation by bytes the peer actually sent (anti-bomb).
    if decoded_len > frame.len().saturating_mul(MAX_COMPRESSION_RATIO) {
        return Err(invalid(
            "ratio_exceeded",
            "compressed body length",
            frame_offset,
            "content size within MAX_COMPRESSION_RATIO of the frame length",
            format!("{decoded_len} > {} * {MAX_COMPRESSION_RATIO}", frame.len()),
        ));
    }
    Ok(decoded_len)
}

/// Decompresses a framed proof body, enforcing the decode bounds described
/// in the module docs.
///
/// `frame_offset` is the offset of `frame` within the full wire message
/// (the length of the uncompressed header), so reported error offsets line
/// up with a hexdump of the wire bytes.
pub(super) fn decompress_body(
    frame: &[u8],
    frame_offset: usize,
    proof_type: ProofType,
) -> Result<Vec<u8>, ReadError> {
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

    let decoded_len = validate_frame(frame, frame_offset)?;

    let mut body = vec![0u8; decoded_len];
    let written = zstd::bulk::decompress_to_buffer(frame, &mut body).map_err(|err| {
        invalid(
            "invalid_frame",
            "compressed body frame",
            frame_offset,
            "a valid zstd frame with no trailing bytes",
            err,
        )
    })?;
    if written != decoded_len {
        return Err(invalid(
            "length_mismatch",
            "compressed body length",
            frame_offset,
            "content size equal to decompressed length",
            format!("declared {decoded_len}, got {written}"),
        ));
    }
    record_frame_metrics("decompress", proof_type, decoded_len, frame.len());
    Ok(body)
}
