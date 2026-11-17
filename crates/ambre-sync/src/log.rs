//! The log of the Sync worker: every message of this crate reaches `tracing`
//! through one call site per level.
//!
//! A `tracing` macro writes a static callsite at each use: its metadata, its
//! interest cache and its dispatch. Over the 227 messages of this crate, those
//! callsites made the largest share of the worker's growth past its ceiling
//! (`ambre-spec-budgets.md` section 5). The macros below format the message at
//! the call site and pass it to [`emit`], which holds the four callsites
//! alone. The level, the text and the source line of each message stay.

use core::fmt::Arguments;
use core::panic::Location;

/// The level of one message, the one `tracing` level it reaches.
#[derive(Clone, Copy)]
pub(crate) enum Level {
    /// A failure the worker cannot recover from in this operation.
    Error,
    /// A refused or failed step that the worker survives.
    Warn,
    /// A step of normal operation.
    Info,
    /// A detail for a developer.
    Debug,
}

/// Emits one message at `level`, prefixed with the source line of its caller.
///
/// Never inlined: the four `tracing` callsites below must stay the only ones
/// of this crate, or the size the module saves comes back.
#[track_caller]
#[inline(never)]
pub(crate) fn emit(level: Level, message: Arguments<'_>) {
    let at = Location::caller();
    let (file, line) = (at.file(), at.line());
    match level {
        Level::Error => tracing::error!("{file}:{line}: {message}"),
        Level::Warn => tracing::warn!("{file}:{line}: {message}"),
        Level::Info => tracing::info!("{file}:{line}: {message}"),
        Level::Debug => tracing::debug!("{file}:{line}: {message}"),
    }
}

/// Logs a failure the worker cannot recover from, as `tracing::error!` does.
macro_rules! error {
    ($($arg:tt)+) => {
        $crate::log::emit($crate::log::Level::Error, format_args!($($arg)+))
    };
}

/// Logs a refused or failed step, as `tracing::warn!` does.
macro_rules! warning {
    ($($arg:tt)+) => {
        $crate::log::emit($crate::log::Level::Warn, format_args!($($arg)+))
    };
}

/// Logs a step of normal operation, as `tracing::info!` does.
macro_rules! info {
    ($($arg:tt)+) => {
        $crate::log::emit($crate::log::Level::Info, format_args!($($arg)+))
    };
}

/// Logs a detail for a developer, as `tracing::debug!` does.
macro_rules! debug {
    ($($arg:tt)+) => {
        $crate::log::emit($crate::log::Level::Debug, format_args!($($arg)+))
    };
}

pub(crate) use {debug, error, info, warning};
