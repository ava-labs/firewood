// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! # Compressed proof body framing
//!
//! Serialized proofs consist of a `Header` followed by the zstd-compressed
//! range or change proof.
//!
//! While the body of the deserialized proof is canonical, compression is not.

use zstd::zstd_safe;

use super::header::Header;
use super::reader::ReadError;
use crate::db::ProofConfig;

/// Writes the wire message: `header` followed by the single zstd frame
/// compressing `body` (the canonical serialized body).
pub(super) fn write_framed(header: &Header, body: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(bytemuck::bytes_of(header));
    write_compressed_body(body, out);
}

/// Enforces the pre-allocation bounds (see [`decompress_body`]) against
/// `config` and returns the frame's declared content size.
///
/// Exactly one zstd frame must span all of `frame`: the raw decompressor
/// accepts concatenated frames and silently skips skippable ones, which
/// would otherwise be an attacker-controlled padding channel.
fn validate_frame(
    frame: &[u8],
    frame_offset: usize,
    config: &ProofConfig,
) -> Result<usize, ReadError> {
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
        .filter(|&n| n <= config.max_decompressed_len)
        .ok_or_else(|| ReadError::InvalidItem {
            item: "frame content size",
            offset: frame_offset,
            expected: "content size within the configured maximum",
            found: format!("{content_size} > {}", config.max_decompressed_len),
        })?;

    // Bound the allocation by bytes the peer actually sent (anti-bomb).
    if decoded_len > frame.len().saturating_mul(config.max_compression_ratio) {
        return Err(ReadError::InvalidItem {
            item: "frame content size",
            offset: frame_offset,
            expected: "content size within the configured compression ratio of the frame length",
            found: format!(
                "{decoded_len} > {} * {}",
                frame.len(),
                config.max_compression_ratio
            ),
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
///   `config.max_decompressed_len` and at `config.max_compression_ratio` ×
///   the compressed frame length.
///
/// `frame_offset` makes error offsets absolute within the wire message;
/// body-parser errors are offsets into the decompressed body.
pub(super) fn decompress_body(
    frame: &[u8],
    frame_offset: usize,
    config: &ProofConfig,
) -> Result<Vec<u8>, ReadError> {
    let decoded_len = validate_frame(frame, frame_offset, config)?;

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
pub(super) fn write_compressed_body(body: &[u8], out: &mut Vec<u8>) {
    let compressed = zstd::bulk::compress(body, zstd::DEFAULT_COMPRESSION_LEVEL)
        .expect("zstd compressor allocation failed");
    out.extend_from_slice(&compressed);
}
