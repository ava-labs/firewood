// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! # Compressed proof body framing
//!
//! Serialized proofs consist of a `Header` followed by the zstd-compressed
//! range or change proof.
//!
//! While the body of the deserialized proof is canonical, compression is not.

use zstd::zstd_safe;

use super::reader::ReadError;
use super::types::ProofError;

/// Hard cap on a decoded proof body, enforced by the decoder before
/// allocating and by the serializer before emitting. 4MiB bounds what one
/// message can force us to allocate while leaving room for real
/// state-sync proofs (target 2MiB). Changing it needs coordination:
/// every decoder must accept a value before any producer emits it.
pub(super) const MAX_DECOMPRESSED_LEN: usize = 4 * 1024 * 1024; // 4 MiB

/// Cap on the uncompressed/compressed length ratio, bounding zstd-bomb
/// amplification to 128× the bytes a peer actually sent. Honest hash-heavy
/// bodies compress ~2–3×. Same coordination caveat as
/// [`MAX_DECOMPRESSED_LEN`].
pub(super) const MAX_COMPRESSION_RATIO: usize = 128;

/// Enforces the pre-allocation bounds (see [`decompress_body`]) and returns
/// the frame's declared content size.
///
/// Exactly one zstd frame must span all of `frame`: the raw decompressor
/// accepts concatenated frames and silently skips skippable ones, which
/// would otherwise be an attacker-controlled padding channel. A lone
/// skippable frame is rejected by the non-zero content-size check.
fn validate_frame(frame: &[u8], frame_offset: usize) -> Result<usize, ReadError> {
    let frame_size =
        zstd_safe::find_frame_compressed_size(frame).map_err(|code| ReadError::InvalidItem {
            item: "compressed body frame",
            offset: frame_offset,
            expected: "a valid zstd frame",
            found: zstd_safe::get_error_name(code).to_owned(),
        })?;
    if frame_size != frame.len() {
        return Err(ReadError::InvalidItem {
            item: "compressed body frame",
            offset: frame_offset,
            expected: "a single zstd frame spanning the entire remainder",
            found: format!("frame ends after {frame_size} of {} bytes", frame.len()),
        });
    }

    let Ok(Some(content_size)) = zstd_safe::get_frame_content_size(frame) else {
        return Err(ReadError::InvalidItem {
            item: "frame content size",
            offset: frame_offset,
            expected: "a content size in the frame header",
            found: "no content size".to_owned(),
        });
    };

    let decoded_len = usize::try_from(content_size)
        .ok()
        .filter(|&n| n != 0 && n <= MAX_DECOMPRESSED_LEN)
        .ok_or_else(|| ReadError::InvalidItem {
            item: "frame content size",
            offset: frame_offset,
            expected: "a non-zero content size within the maximum",
            found: format!("{content_size} not in 1..={MAX_DECOMPRESSED_LEN}"),
        })?;

    // Bound the allocation by bytes the peer actually sent (anti-bomb).
    if decoded_len > frame.len().saturating_mul(MAX_COMPRESSION_RATIO) {
        return Err(ReadError::InvalidItem {
            item: "frame content size",
            offset: frame_offset,
            expected: "content size within the maximum compression ratio of the frame length",
            found: format!("{decoded_len} > {} * {MAX_COMPRESSION_RATIO}", frame.len()),
        });
    }
    Ok(decoded_len)
}

/// Decompresses a framed proof body.
///
/// The frame bytes are attacker-controlled, so every bound below is
/// enforced *before* the body allocation:
///
/// - Exactly one zstd frame must span all of `frame`.
/// - The frame header must declare a content size, capped at
///   [`MAX_DECOMPRESSED_LEN`] and at [`MAX_COMPRESSION_RATIO`] × the
///   compressed frame length.
///
/// `frame_offset` makes error offsets absolute within the wire message;
/// body-parser errors are offsets into the decompressed body.
pub(super) fn decompress_body(frame: &[u8], frame_offset: usize) -> Result<Vec<u8>, ReadError> {
    let decoded_len = validate_frame(frame, frame_offset)?;

    let mut body = vec![0u8; decoded_len];
    zstd::bulk::decompress_to_buffer(frame, &mut body).map_err(|err| ReadError::InvalidItem {
        item: "compressed body frame",
        offset: frame_offset,
        expected: "a valid zstd frame",
        found: err.to_string(),
    })?;
    Ok(body)
}

/// Appends the single zstd frame compressing `body` (the canonical
/// serialized body that follows the header on the wire).
///
/// # Errors
///
/// Returns [`ProofError::Compression`] if the compression fails (resource
/// exhaustion; zstd cannot otherwise fail on in-memory input).
pub(super) fn write_compressed_body(body: &[u8], out: &mut Vec<u8>) -> Result<(), ProofError> {
    let compressed = zstd::bulk::compress(body, zstd::DEFAULT_COMPRESSION_LEVEL)
        .map_err(ProofError::Compression)?;
    out.extend_from_slice(&compressed);
    Ok(())
}
