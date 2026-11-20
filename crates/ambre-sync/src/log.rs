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

#[cfg(test)]
pub(crate) mod tests {
    use super::{Level, emit};
    use std::sync::{Arc, Mutex};

    /// The level and the message of each event `tracing` receives.
    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<(tracing::Level, String)>>>);

    /// The messages `run` logs, once one of them contains `marker`.
    ///
    /// Another test thread may leave a callsite's interest stale, as the test
    /// below states. So `run` runs again after a rebuild until the marker
    /// shows, a bounded number of times. An absence read off the result is
    /// then read off messages that did reach the capture.
    pub(crate) fn logged_by(marker: &str, mut run: impl FnMut()) -> Vec<String> {
        let capture = Capture::default();
        tracing::subscriber::with_default(capture.clone(), || {
            for _ in 0..50 {
                tracing::callsite::rebuild_interest_cache();
                if let Ok(mut held) = capture.0.lock() {
                    held.clear();
                }
                run();
                let seen: Vec<String> = capture
                    .0
                    .lock()
                    .map(|held| held.iter().map(|(_, message)| message.clone()).collect())
                    .unwrap_or_default();
                if seen.iter().any(|message| message.contains(marker)) {
                    return seen;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            panic!("no message contains {marker}");
        })
    }

    /// The `message` field of one event.
    struct Message(String);

    impl tracing::field::Visit for Message {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn core::fmt::Debug) {
            if field.name() == "message" {
                self.0 = format!("{value:?}");
            }
        }
    }

    impl tracing::Subscriber for Capture {
        fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            let mut message = Message(String::new());
            event.record(&mut message);
            if let Ok(mut seen) = self.0.lock() {
                seen.push((*event.metadata().level(), message.0));
            }
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }

    /// Each message reaches `tracing` once, at the level it names, prefixed
    /// with the source line of its caller.
    #[test]
    fn a_message_reaches_tracing_at_its_level_with_the_line_of_its_caller() {
        let capture = Capture::default();
        let levels = [
            (Level::Error, tracing::Level::ERROR),
            (Level::Warn, tracing::Level::WARN),
            (Level::Info, tracing::Level::INFO),
            (Level::Debug, tracing::Level::DEBUG),
        ];
        let expected = |line: u32| -> Vec<(tracing::Level, String)> {
            levels
                .iter()
                .map(|(_, level)| (*level, format!("{}:{line}: refused 7", file!())))
                .collect()
        };
        tracing::subscriber::with_default(capture.clone(), || {
            // Another test thread may register a callsite of `emit` before this
            // capture exists. Preempted between the two steps of its registration, it
            // stores the interest as never after the capture rebuilt it. A later
            // rebuild corrects it, so the test emits again after one, a bounded number
            // of times.
            let mut seen = Vec::new();
            for _ in 0..50 {
                tracing::callsite::rebuild_interest_cache();
                if let Ok(mut held) = capture.0.lock() {
                    held.clear();
                }
                let line = line!() + 2;
                for (level, _) in levels {
                    emit(level, format_args!("refused {}", 7));
                }
                seen = capture.0.lock().expect("the capture").clone();
                if seen == expected(line) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            panic!("the messages reached tracing as {seen:?}");
        });
    }
}
