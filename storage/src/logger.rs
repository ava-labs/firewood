// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

// Supports making the logging operations a true runtime no-op
// Since we're a library, we can't really use the logging level
// static shortcut

#[cfg(feature = "logger")]
pub use firewood_metrics::firewood_debug as debug;
#[cfg(feature = "logger")]
pub use firewood_metrics::firewood_error as error;
#[cfg(feature = "logger")]
pub use firewood_metrics::firewood_info as info;
#[cfg(feature = "logger")]
pub use firewood_metrics::firewood_trace as trace;
#[cfg(feature = "logger")]
pub use firewood_metrics::firewood_warn as warn;

/// Returns true if the trace log level is enabled
#[cfg(feature = "logger")]
#[must_use]
pub fn trace_enabled() -> bool {
    log::log_enabled!(log::Level::Trace)
}

#[cfg(not(feature = "logger"))]
pub use noop_logger::debug;
#[cfg(not(feature = "logger"))]
pub use noop_logger::error;
#[cfg(not(feature = "logger"))]
pub use noop_logger::info;
#[cfg(not(feature = "logger"))]
pub use noop_logger::trace;
#[cfg(not(feature = "logger"))]
pub use noop_logger::trace_enabled;
#[cfg(not(feature = "logger"))]
pub use noop_logger::warn;

#[cfg(not(feature = "logger"))]
mod noop_logger {
    #[macro_export]
    /// A noop logger, when the logger feature is disabled
    macro_rules! noop {
        ($($arg:tt)+) => {
            if false {
                // This is a no-op. If we had an empty macro, the compiler and
                // clippy would generate warnings about variables in the
                // expressions passed into the macro going unused.
                //
                // This is a workaround to avoid that. The `false` branch will
                // never be execute, the expressions passed in will never be
                // evaluated, this string will never be constructed, and the
                // compiler will completely eliminate this branch when any
                // level of optimization is enabled.
                let _ = format!($($arg)+);
            }
        };
    }

    pub use noop as debug;
    pub use noop as error;
    pub use noop as info;
    pub use noop as trace;
    pub use noop as warn;

    /// `trace_enabled` for a noop logger is always false
    #[inline]
    #[must_use]
    pub const fn trace_enabled() -> bool {
        false
    }
}
