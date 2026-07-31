// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use clap::Args;
use std::io::{Error, ErrorKind};

use crate::NodeHashAlgorithm;

/// Selects a database key from text, raw hex, or Ethereum account inputs.
#[derive(Debug, Args, Clone)]
pub struct Options {
    /// The key bytes in hexadecimal form. An optional 0x prefix is accepted.
    #[arg(
        long = "key-hex",
        value_name = "KEY_HEX",
        conflicts_with_all = ["account", "storage_key"]
    )]
    pub key_hex: Option<String>,

    /// An Ethereum account address (20 bytes in hexadecimal form).
    #[arg(long, value_name = "ACCOUNT", conflicts_with = "key_hex")]
    pub account: Option<String>,

    /// An Ethereum storage slot (32 bytes in hexadecimal form), combined with --account.
    #[arg(long, value_name = "STORAGE_KEY", requires = "account")]
    pub storage_key: Option<String>,
}

impl Options {
    /// Whether one of the non-text key selectors was provided.
    pub fn has_selector(&self) -> bool {
        self.key_hex.is_some() || self.account.is_some() || self.storage_key.is_some()
    }

    /// Pick a single command key argument, or let the selector flags provide it.
    pub fn raw_key<'a>(&self, args: &'a [String], command: &str) -> Result<Option<&'a str>, Error> {
        if self.has_selector() {
            if !args.is_empty() {
                return Err(invalid(format!(
                    "a key selector cannot be combined with a text KEY for {command}"
                )));
            }
            Ok(None)
        } else if args.len() == 1 {
            Ok(Some(args[0].as_str()))
        } else {
            Err(invalid(format!("{command} requires a KEY argument")))
        }
    }

    /// Pick the key and value arguments for `insert`.
    pub fn insert_args<'a>(&self, args: &'a [String]) -> Result<(Option<&'a str>, &'a str), Error> {
        if self.has_selector() {
            if args.len() == 1 {
                Ok((None, &args[0]))
            } else {
                Err(invalid(
                    "a key selector requires exactly one VALUE argument",
                ))
            }
        } else if args.len() == 2 {
            Ok((Some(args[0].as_str()), &args[1]))
        } else {
            Err(invalid("insert requires KEY and VALUE arguments"))
        }
    }

    /// Resolve the selected input into the bytes used by Firewood.
    pub fn resolve(
        &self,
        raw_key: Option<&str>,
        hash_algorithm: NodeHashAlgorithm,
    ) -> Result<Box<[u8]>, Error> {
        match (raw_key, &self.key_hex, &self.account, &self.storage_key) {
            (Some(key), None, None, None) => Ok(key.as_bytes().into()),
            (None, Some(key_hex), None, None) => decode_hex(key_hex, "key"),
            (None, None, Some(account), storage_key) => {
                resolve_ethereum_key(account, storage_key.as_deref(), hash_algorithm)
            }
            (None, None, None, Some(_)) => Err(invalid("--storage-key requires --account")),
            _ => Err(invalid("provide exactly one key input")),
        }
    }

    /// Return a human-readable representation for command output.
    pub fn display(&self, raw_key: Option<&str>, resolved: &[u8]) -> String {
        raw_key.map_or_else(|| format!("0x{}", hex::encode(resolved)), str::to_owned)
    }
}

fn decode_hex(value: &str, name: &str) -> Result<Box<[u8]>, Error> {
    let value = value.strip_prefix("0x").unwrap_or(value);
    hex::decode(value)
        .map(Vec::into_boxed_slice)
        .map_err(|error| invalid(format!("invalid {name} hex: {error}")))
}

fn decode_fixed(value: &str, name: &str, expected: usize) -> Result<Box<[u8]>, Error> {
    let decoded = decode_hex(value, name)?;
    if decoded.len() != expected {
        return Err(invalid(format!(
            "{name} must be {expected} bytes, got {}",
            decoded.len()
        )));
    }
    Ok(decoded)
}

#[cfg(feature = "ethhash")]
fn resolve_ethereum_key(
    account: &str,
    storage_key: Option<&str>,
    hash_algorithm: NodeHashAlgorithm,
) -> Result<Box<[u8]>, Error> {
    use sha3::{Digest, Keccak256};

    if hash_algorithm != NodeHashAlgorithm::Ethereum {
        return Err(invalid(
            "--account and --storage-key require --node-hash-algorithm ethereum",
        ));
    }

    let account = decode_fixed(account, "account", 20)?;
    let account_hash = Keccak256::digest(&account);
    let Some(storage_key) = storage_key else {
        return Ok(account_hash.to_vec().into_boxed_slice());
    };

    let storage_key = decode_fixed(storage_key, "storage key", 32)?;
    let storage_hash = Keccak256::digest(&storage_key);
    Ok(account_hash
        .iter()
        .chain(storage_hash.iter())
        .copied()
        .collect::<Vec<_>>()
        .into_boxed_slice())
}

#[cfg(not(feature = "ethhash"))]
fn resolve_ethereum_key(
    _account: &str,
    _storage_key: Option<&str>,
    _hash_algorithm: NodeHashAlgorithm,
) -> Result<Box<[u8]>, Error> {
    Err(invalid(
        "--account and --storage-key require fwdctl built with the ethhash feature",
    ))
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidInput, message.into())
}

#[cfg(test)]
mod tests {
    use super::Options;
    use crate::NodeHashAlgorithm;

    #[test]
    fn resolves_text_and_hex_keys() {
        let text = Options {
            key_hex: None,
            account: None,
            storage_key: None,
        };
        assert_eq!(
            &*text
                .resolve(Some("hello"), NodeHashAlgorithm::MerkleDB)
                .unwrap(),
            b"hello"
        );

        let hex = Options {
            key_hex: Some("0x00ff".into()),
            account: None,
            storage_key: None,
        };
        assert_eq!(
            &*hex.resolve(None, NodeHashAlgorithm::MerkleDB).unwrap(),
            &[0, 255]
        );
    }

    #[test]
    fn rejects_invalid_hex_and_combinations() {
        let invalid_hex = Options {
            key_hex: Some("not-hex".into()),
            account: None,
            storage_key: None,
        };
        assert!(
            invalid_hex
                .resolve(None, NodeHashAlgorithm::MerkleDB)
                .is_err()
        );

        let missing = Options {
            key_hex: None,
            account: None,
            storage_key: None,
        };
        assert!(missing.resolve(None, NodeHashAlgorithm::MerkleDB).is_err());
    }

    #[test]
    fn separates_legacy_and_selector_arguments() {
        let text = Options {
            key_hex: None,
            account: None,
            storage_key: None,
        };
        let args = vec!["key".into(), "value".into()];
        assert_eq!(text.insert_args(&args).unwrap(), (Some("key"), "value"));
        assert_eq!(text.raw_key(&args[..1], "get").unwrap(), Some("key"));

        let hex = Options {
            key_hex: Some("00".into()),
            account: None,
            storage_key: None,
        };
        let value = vec!["value".into()];
        assert_eq!(hex.insert_args(&value).unwrap(), (None, "value"));
        assert_eq!(hex.raw_key(&[], "get").unwrap(), None);
    }

    #[cfg(feature = "ethhash")]
    #[test]
    fn resolves_ethereum_account_and_storage_keys() {
        let account = Options {
            key_hex: None,
            account: Some("11".repeat(20)),
            storage_key: Some("22".repeat(32)),
        };
        let key = account.resolve(None, NodeHashAlgorithm::Ethereum).unwrap();
        assert_eq!(key.len(), 64);

        let account_only = Options {
            key_hex: None,
            account: Some("11".repeat(20)),
            storage_key: None,
        };
        assert_eq!(
            account_only
                .resolve(None, NodeHashAlgorithm::Ethereum)
                .unwrap()
                .len(),
            32
        );
    }

    #[cfg(feature = "ethhash")]
    #[test]
    fn rejects_wrong_ethereum_lengths() {
        let account = Options {
            key_hex: None,
            account: Some("11".repeat(19)),
            storage_key: None,
        };
        assert!(account.resolve(None, NodeHashAlgorithm::Ethereum).is_err());
    }
}
