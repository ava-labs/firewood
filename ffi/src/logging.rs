// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::BorrowedBytes;

/// Arguments for initializing logging for the Firewood FFI.
#[repr(C)]
#[derive(Debug)]
pub struct LogArgs<'a> {
    /// The file path where logs for this process are stored.
    ///
    /// This is required and must not be empty. Use "/dev/stdout" for stdout logging.
    ///
    /// This is required to be a valid UTF-8 string.
    pub path: BorrowedBytes<'a>,

    /// The filter string in `RUST_LOG` format.
    ///
    /// If empty, `env_logger` defaults will be used.
    /// Common example: "info" to show info-level and above logs.
    /// See <https://docs.rs/env_logger> for full `RUST_LOG` format documentation.
    ///
    /// This is required to be a valid UTF-8 string.
    pub filter_level: BorrowedBytes<'a>,
}

#[cfg(feature = "logger")]
impl LogArgs<'_> {
    fn path(&self) -> std::io::Result<&std::path::Path> {
        let path = self.path.as_str().map_err(|err| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("log path contains invalid utf-8: {err}"),
            )
        })?;
        if path.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "log path must not be empty",
            ));
        }
        Ok(std::path::Path::new(path))
    }

    fn log_filter(&self) -> std::io::Result<&str> {
        let filter = self.filter_level.as_str().map_err(|err| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("log filter contains invalid utf-8: {err}"),
            )
        })?;
        Ok(filter)
    }

    /// Starts logging to the specified file path with the given filter.
    ///
    /// The filter uses `RUST_LOG` format. See `env_logger` documentation for details.
    /// Logging is global per-process and can only be initialized once.
    ///
    /// # Errors
    ///
    /// If the log file cannot be created or opened, if the log filter is invalid,
    /// or if the logger has already been initialized, this will return an error.
    pub fn start_logging(&self) -> std::io::Result<()> {
        use env_logger::Target::Pipe;
        use std::fs::OpenOptions;

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

        let filter = self.log_filter()?;

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)
            .map_err(|e| {
                std::io::Error::new(
                    e.kind(),
                    format!("failed to open log file `{}`: {e}", log_path.display()),
                )
            })?;

        let mut builder = env_logger::Builder::new();
        builder.target(Pipe(Box::new(file)));

        if !filter.is_empty() {
            builder.parse_filters(filter);
        }

        builder
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
