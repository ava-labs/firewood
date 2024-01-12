// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#[cfg(feature = "logger")]
#[macro_export]
macro_rules! trace {
    // trace!(target: "my_target", key1 = 42, key2 = true; "a {} event", "log")
    // trace!(target: "my_target", "a {} event", "log")
    (target: $target:expr, $($arg:tt)+) => (log::log!(target: $target, log::Level::Trace, $($arg)+));

    // trace!("a {} event", "log")
    ($($arg:tt)+) => (log::log!(log::Level::Trace, $($arg)+))
}
#[cfg(not(feature = "logger"))]
#[macro_export]
macro_rules! trace {
    (target: $target:expr, $($arg:tt)+) => {};
    ($($arg:tt)+) => {};
}

#[cfg(feature = "logger")]
#[macro_export]
macro_rules! debug {
    // trace!(target: "my_target", key1 = 42, key2 = true; "a {} event", "log")
    // trace!(target: "my_target", "a {} event", "log")
    (target: $target:expr, $($arg:tt)+) => (log::log!(target: $target, log::Level::Debug, $($arg)+));

    // trace!("a {} event", "log")
    ($($arg:tt)+) => (log::log!(log::Level::Debug, $($arg)+))
}
#[cfg(not(feature = "logger"))]
#[macro_export]
macro_rules! debug {
    (target: $target:expr, $($arg:tt)+) => {};
    ($($arg:tt)+) => {};
}

#[cfg(feature = "logger")]
#[macro_export]
macro_rules! info {
    // trace!(target: "my_target", key1 = 42, key2 = true; "a {} event", "log")
    // trace!(target: "my_target", "a {} event", "log")
    (target: $target:expr, $($arg:tt)+) => (log::log!(target: $target, log::Level::Info, $($arg)+));

    // trace!("a {} event", "log")
    ($($arg:tt)+) => (log::log!(log::Level::Info, $($arg)+))
}
#[cfg(not(feature = "logger"))]
#[macro_export]
macro_rules! info {
    (target: $target:expr, $($arg:tt)+) => {};
    ($($arg:tt)+) => {};
}

#[cfg(feature = "logger")]
#[macro_export]
macro_rules! warn {
    // trace!(target: "my_target", key1 = 42, key2 = true; "a {} event", "log")
    // trace!(target: "my_target", "a {} event", "log")
    (target: $target:expr, $($arg:tt)+) => (log::log!(target: $target, log::Level::Warn, $($arg)+));

    // trace!("a {} event", "log")
    ($($arg:tt)+) => (log::log!(log::Level::Warn, $($arg)+))
}
#[cfg(not(feature = "logger"))]
#[macro_export]
macro_rules! warn {
    (target: $target:expr, $($arg:tt)+) => {};
    ($($arg:tt)+) => {};
}

#[cfg(feature = "logger")]
#[macro_export]
macro_rules! error {
    // trace!(target: "my_target", key1 = 42, key2 = true; "a {} event", "log")
    // trace!(target: "my_target", "a {} event", "log")
    (target: $target:expr, $($arg:tt)+) => (log::log!(target: $target, log::Level::Error, $($arg)+));

    // trace!("a {} event", "log")
    ($($arg:tt)+) => (log::log!(log::Level::Error, $($arg)+))
}
#[cfg(not(feature = "logger"))]
#[macro_export]
macro_rules! error {
    (target: $target:expr, $($arg:tt)+) => {};
    ($($arg:tt)+) => {};
}
