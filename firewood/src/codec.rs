// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use bincode::Options;
use thiserror::Error;

const MIN_BYTES_LEN: usize = 1;

#[derive(Debug, Error)]
pub enum CodecError {
    #[error("bincode error")]
    BincodeError(#[from] bincode::Error),
    #[error("no such node")]
    UnexpectedEOFError,
    #[error("from utf8 error")]
    FromUtf8Error(#[from] std::string::FromUtf8Error),
}

use serde::Deserialize;

#[derive(Deserialize, Debug)]
struct Data {
    len: i64,

    #[serde(skip)]
    trailing: Vec<u8>,
}

pub fn encode_int(val: i64) -> Result<Vec<u8>, CodecError> {
    Ok(bincode::DefaultOptions::new().serialize(&val)?)
}

pub fn decode_int(src: &[u8]) -> Result<i64, CodecError> {
    Ok(bincode::DefaultOptions::new().deserialize(src)?)
}

pub fn encode_str(val: &[u8]) -> Result<Vec<u8>, CodecError> {
    let res = encode_int(val.len() as i64)?;
    Ok([&res[..], val].concat())
}

pub fn decode_str(src: &[u8]) -> Result<String, CodecError> {
    if src.len() < MIN_BYTES_LEN {
        return Err(CodecError::UnexpectedEOFError);
    }

    let mut cursor = src;
    let mut data: Data = bincode::DefaultOptions::new().deserialize_from(&mut cursor)?;
    data.trailing = cursor.to_owned();

    let len = data.len;
    if len < 0 {
        return Err(CodecError::UnexpectedEOFError);
    } else if len == 0 {
        return Ok(String::default());
    } else if len as usize > src.len() {
        return Err(CodecError::UnexpectedEOFError);
    }
    Ok(String::from_utf8(data.trailing)?)
}
