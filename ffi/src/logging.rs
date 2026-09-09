// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::BorrowedBytes;

/// Arguments for initializing logging for the Firewood FFI.
#[repr(C)]
#[derive(Debug)]
pub struct LogArgs<'a> {
    /// The file path where logs for this process are stored.
    ///
    /// If empty, this is set to `${TMPDIR}/firewood-log.txt`.
    ///
    /// This is required to be a valid UTF-8 string.
    pub path: BorrowedBytes<'a>,

    /// The filter level for logs.
    ///
    /// If empty, this is set to `info`.
    ///
    /// This is required to be a valid UTF-8 string.
    pub filter_level: BorrowedBytes<'a>,
}

#[cfg(feature = "logger")]
fn record_db_tag<'a>(record: &'a log::Record<'a>) -> Option<&'a str> {
    use log::kv::Key;

    record
        .key_values()
        .get(Key::from_str("db_tag"))
        .and_then(|value| value.to_borrowed_str())
}

#[cfg(feature = "logger")]
impl LogArgs<'_> {
    fn path(&self) -> std::io::Result<std::borrow::Cow<'_, std::path::Path>> {
        let path = self.path.as_str().map_err(|err| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("log path contains invalid utf-8: {err}"),
            )
        })?;
        if path.is_empty() {
            Ok(std::borrow::Cow::Owned(
                std::env::temp_dir().join("firewood-log.txt"),
            ))
        } else {
            Ok(std::borrow::Cow::Borrowed(std::path::Path::new(path)))
        }
    }

    fn log_level(&self) -> std::io::Result<&str> {
        let level = self.filter_level.as_str().map_err(|err| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("log level contains invalid utf-8: {err}"),
            )
        })?;
        if level.is_empty() {
            Ok("info")
        } else {
            Ok(level)
        }
    }

    /// Starts logging to the specified file path with the given filter level.
    ///
    /// # Errors
    ///
    /// If the log file cannot be created or opened, or if the log level is invalid,
    /// this will return an error.
    pub fn start_logging(&self) -> std::io::Result<()> {
        use env_logger::Target::Pipe;
        use std::fs::OpenOptions;
        use std::io::Write;

        let log_path = self.path()?;

        if let Some(log_dir) = log_path.parent() {
            std::fs::create_dir_all(log_dir).map_err(|e| {
                std::io::Error::new(
                    e.kind(),
                    format!(
                        "failed to create log directory `{}`: {e}",
                        log_dir.display()
                    ),
                )
            })?;
        }

        let level = self.log_level()?;
        let level = level.parse().map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("invalid log level `{level}`: {e}"),
            )
        })?;

        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&log_path)
            .map_err(|e| {
                std::io::Error::new(
                    e.kind(),
                    format!("failed to open log file `{}`: {e}", log_path.display()),
                )
            })?;

        env_logger::Builder::new()
            .filter_level(level)
            .format(move |buffer, record| {
                // matches env_logger's default format style
                // TODO(AminR443): adopt slog format like avalanchego
                // https://github.com/ava-labs/firewood/issues/2227
                let timestamp = buffer.timestamp_millis();
                if let Some(db_tag) = record_db_tag(record) {
                    writeln!(
                        buffer,
                        "[{timestamp} {:<5} {} db_tag={db_tag}] {}",
                        record.level(),
                        record.target(),
                        record.args()
                    )
                } else {
                    writeln!(
                        buffer,
                        "[{timestamp} {:<5} {}] {}",
                        record.level(),
                        record.target(),
                        record.args()
                    )
                }
            })
            .target(Pipe(Box::new(file)))
            .try_init()
            .map_err(|e| std::io::Error::other(format!("failed to initialize logger: {e}")))?;

        Ok(())
    }
}

#[cfg(not(feature = "logger"))]
impl LogArgs<'_> {
    /// Starts logging to the specified file path with the given filter level.
    ///
    /// # Errors
    ///
    /// This method will always return an error because the `logger` feature is not enabled.
    pub fn start_logging(&self) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "firewood-ffi was compiled without the `logger` feature. Logging is not available.",
        ))
    }
}
