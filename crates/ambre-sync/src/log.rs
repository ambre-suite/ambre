//! The log of the Sync worker: every message of this crate goes through
//! [`emit`], one function for every level.
//!
//! A `tracing` macro writes a static callsite at each use: its metadata, its
//! interest cache and its dispatch. Over the 227 messages of this crate, those
//! callsites made the largest share of the worker's growth past its ceiling
//! (`ambre-spec-budgets.md` section 5). The macros below format the message at
//! the call site and pass it to [`emit`]. The level, the text and the source
//! line of each message stay.
//!
//! In the worker, [`emit`] hands each message to the browser itself: one
//! console line, then a performance mark and a measure (`SYNC-301`). The
//! `tracing-wasm` layer did the same over `tracing` and the
//! `tracing-subscriber` registry: about 47 KB of the worker's wasm for one
//! console call. On the host, [`emit`] reaches `tracing`, which the native
//! tests capture.

use core::fmt::Arguments;
use core::panic::Location;

/// The level of one message.
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

/// Emits one message at `level`, with the source line of its caller.
///
/// Never inlined: this function must stay the only sink of this crate, or
/// the size the module saves comes back.
#[track_caller]
#[inline(never)]
pub(crate) fn emit(level: Level, message: Arguments<'_>) {
    let at = Location::caller();
    #[cfg(target_arch = "wasm32")]
    console::print(&console::Printed::new(
        level,
        at,
        &message.to_string(),
        &console::MARKS,
    ));
    #[cfg(not(target_arch = "wasm32"))]
    {
        let (file, line) = (at.file(), at.line());
        match level {
            Level::Error => tracing::error!("{file}:{line}: {message}"),
            Level::Warn => tracing::warn!("{file}:{line}: {message}"),
            Level::Info => tracing::info!("{file}:{line}: {message}"),
            Level::Debug => tracing::debug!("{file}:{line}: {message}"),
        }
    }
}

/// The browser half of the log: what one message prints in the worker.
///
/// [`Printed::new`] builds every value the browser receives, so a host test
/// reads the same values the worker hands to `console.log` and to the
/// performance profile.
#[cfg(any(test, target_arch = "wasm32"))]
mod console {
    use core::panic::Location;
    use core::sync::atomic::{AtomicU64, Ordering};

    use super::Level;

    /// The number of the next performance mark of the worker.
    #[cfg(target_arch = "wasm32")]
    pub(super) static MARKS: AtomicU64 = AtomicU64::new(0);

    /// The style of the second segment of a line: the file and line.
    pub(super) const ORIGIN_STYLE: &str = "color: gray; font-style: italic";

    /// The style of the third segment of a line: the message.
    pub(super) const TEXT_STYLE: &str = "color: inherit";

    /// What one message hands to the browser.
    #[derive(Clone, PartialEq, Eq, Debug)]
    pub(super) struct Printed {
        /// The console line, with its three `%c` directives.
        pub(super) line: String,
        /// The style of the first segment, the level word.
        pub(super) level_style: &'static str,
        /// The name of the performance mark.
        pub(super) mark: String,
        /// The label of the measure that starts at the mark.
        pub(super) measure: String,
    }

    impl Printed {
        /// The line, the mark and the measure of `message` at `level`, logged
        /// at `at`. The mark takes the next number of `marks`: the names count
        /// up from `c0` in hexadecimal, as the `tracing-wasm` layer named them.
        pub(super) fn new(
            level: Level,
            at: &Location<'_>,
            message: &str,
            marks: &AtomicU64,
        ) -> Self {
            let word = word(level);
            let (file, line) = (at.file(), at.line());
            let mark = marks.fetch_add(1, Ordering::Relaxed);
            Self {
                line: format!("%c{word}%c {file}:{line}%c {message}"),
                level_style: style(level),
                mark: format!("c{mark:x}"),
                measure: format!("{word} {file}:{line} {message}"),
            }
        }
    }

    /// The level word, as `tracing` spells it.
    const fn word(level: Level) -> &'static str {
        match level {
            Level::Error => "ERROR",
            Level::Warn => "WARN",
            Level::Info => "INFO",
            Level::Debug => "DEBUG",
        }
    }

    /// The colour of the level word, as the `tracing-wasm` layer drew it.
    const fn style(level: Level) -> &'static str {
        match level {
            Level::Error => "color: red; background: #444",
            Level::Warn => "color: orange; background: #444",
            Level::Info => "color: whitesmoke; background: #444",
            Level::Debug => "color: lawngreen; background: #444",
        }
    }

    #[cfg(target_arch = "wasm32")]
    #[wasm_bindgen::prelude::wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = console, js_name = log)]
        fn console_log(line: &str, level_style: &str, origin_style: &str, text_style: &str);
        #[wasm_bindgen(js_namespace = performance, js_name = mark)]
        fn performance_mark(name: &str);
        #[wasm_bindgen(catch, js_namespace = performance, js_name = measure)]
        fn performance_measure(name: &str, start: &str) -> Result<(), wasm_bindgen::JsValue>;
    }

    /// Hands one message to the console, then to the performance profile.
    #[cfg(target_arch = "wasm32")]
    pub(super) fn print(printed: &Printed) {
        console_log(&printed.line, printed.level_style, ORIGIN_STYLE, TEXT_STYLE);
        performance_mark(&printed.mark);
        // A measure the browser refuses loses a blip in the profile, and
        // nothing else.
        let _ = performance_measure(&printed.measure, &printed.mark);
    }
}

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
    use super::console::{self, Printed};
    use super::{Level, emit};
    use core::panic::Location;
    use core::sync::atomic::AtomicU64;
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

    /// The worker prints one line of three styled segments: the level word,
    /// the file and line of the caller, then the message (`SYNC-301`).
    #[test]
    fn a_worker_line_holds_the_level_the_caller_and_the_message() {
        let at = Location::caller();
        let printed = Printed::new(Level::Warn, at, "refused 7", &AtomicU64::new(0));
        assert_eq!(
            printed.line,
            format!("%cWARN%c {}:{}%c refused 7", at.file(), at.line())
        );
        assert_eq!(printed.level_style, "color: orange; background: #444");
        assert_eq!(console::ORIGIN_STYLE, "color: gray; font-style: italic");
        assert_eq!(console::TEXT_STYLE, "color: inherit");
    }

    /// Each level prints its own word in the colour the layer drew.
    #[test]
    fn each_level_prints_its_word_in_the_colour_of_the_layer() {
        let at = Location::caller();
        let printed: Vec<(String, &str)> = [Level::Error, Level::Warn, Level::Info, Level::Debug]
            .into_iter()
            .map(|level| {
                let printed = Printed::new(level, at, "m", &AtomicU64::new(0));
                let word = printed
                    .measure
                    .split(' ')
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                (word, printed.level_style)
            })
            .collect();
        assert_eq!(
            printed,
            [
                ("ERROR".to_owned(), "color: red; background: #444"),
                ("WARN".to_owned(), "color: orange; background: #444"),
                ("INFO".to_owned(), "color: whitesmoke; background: #444"),
                ("DEBUG".to_owned(), "color: lawngreen; background: #444"),
            ]
        );
    }

    /// Each message leaves a mark named by its number in hexadecimal, and a
    /// measure labelled with its level, its caller and its text (`SYNC-301`).
    #[test]
    fn a_worker_message_marks_the_profile_with_a_measure_of_its_text() {
        let at = Location::caller();
        let marks = AtomicU64::new(26);
        let printed = Printed::new(Level::Info, at, "opened", &marks);
        assert_eq!(printed.mark, "c1a");
        let next = Printed::new(Level::Info, at, "opened", &marks);
        assert_eq!(next.mark, "c1b");
        assert_eq!(
            printed.measure,
            format!("INFO {}:{} opened", at.file(), at.line())
        );
    }
}
