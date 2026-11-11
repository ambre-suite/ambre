//! The subscriber of the Main instance: each event the filter keeps becomes
//! one console line, and in a debug build a timing mark.
//!
//! It replaces the `tracing-subscriber` registry and the `tracing-wasm` layer.
//! The Main instance carried about 20 KB of wasm for one filter and one
//! console call (`ambre-spec-budgets.md` section 5). The line keeps the shape
//! the layer printed: the level, the file and line, the message and its fields.
//! No client code opens a span, so a span records nothing.

use std::fmt::{self, Write as _};
use std::sync::atomic::{AtomicU64, Ordering};

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::Interest;
use tracing::{Event, Level, Metadata, Subscriber};
use wasm_bindgen::prelude::wasm_bindgen;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = log)]
    fn console_log(line: &str, level_style: &str, origin_style: &str, text_style: &str);
    #[wasm_bindgen(js_namespace = performance, js_name = mark)]
    fn performance_mark(name: &str);
    #[wasm_bindgen(catch, js_namespace = performance, js_name = measure)]
    fn performance_measure(name: &str, start: &str) -> Result<(), wasm_bindgen::JsValue>;
}

/// Whether each event also leaves a timing mark in the browser profile.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Timings {
    Off,
    Marked,
}

/// The subscriber: our crates at `ours`, every dependency at WARN.
pub struct ConsoleTrace {
    ours: Level,
    timings: Timings,
    marks: AtomicU64,
}

/// The target prefix of every crate of the workspace.
const OUR_TARGETS: &str = "ambre";

impl ConsoleTrace {
    /// A subscriber that prints our events up to `ours`, and the events of a
    /// dependency up to WARN.
    pub const fn new(ours: Level, timings: Timings) -> Self {
        Self {
            ours,
            timings,
            marks: AtomicU64::new(0),
        }
    }

    /// The most verbose level `meta` may print at. A target matches by its
    /// prefix, as a `Targets` directive does.
    fn ceiling(&self, meta: &Metadata<'_>) -> Level {
        if meta.target().starts_with(OUR_TARGETS) {
            self.ours
        } else {
            Level::WARN
        }
    }
}

impl Subscriber for ConsoleTrace {
    fn register_callsite(&self, meta: &'static Metadata<'static>) -> Interest {
        if self.enabled(meta) {
            Interest::always()
        } else {
            Interest::never()
        }
    }

    fn enabled(&self, meta: &Metadata<'_>) -> bool {
        meta.level() <= &self.ceiling(meta)
    }

    fn max_level_hint(&self) -> Option<tracing::level_filters::LevelFilter> {
        Some(tracing::level_filters::LevelFilter::from_level(
            self.ours.max(Level::WARN),
        ))
    }

    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _: &Id, _: &Record<'_>) {}

    fn record_follows_from(&self, _: &Id, _: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let mut text = Text::default();
        event.record(&mut text);
        let meta = event.metadata();
        let level = meta.level();
        let origin = meta
            .file()
            .and_then(|file| meta.line().map(|line| format!("{file}:{line}")))
            .unwrap_or_default();
        console_log(
            &format!("%c{level}%c {origin}%c{text}"),
            level_style(*level),
            "color: gray; font-style: italic",
            "color: inherit",
        );
        if self.timings == Timings::Marked {
            let mark = format!("c{:x}", self.marks.fetch_add(1, Ordering::Relaxed));
            performance_mark(&mark);
            let module = meta.module_path().unwrap_or("...");
            // A measure the browser refuses loses a blip in the profile, and
            // nothing else.
            let _ = performance_measure(&format!("{level} {module}{text}"), &mark);
        }
    }

    fn enter(&self, _: &Id) {}

    fn exit(&self, _: &Id) {}
}

/// The colour of the level word, as the layer drew it.
const fn level_style(level: Level) -> &'static str {
    match level {
        Level::TRACE => "color: dodgerblue; background: #444",
        Level::DEBUG => "color: lawngreen; background: #444",
        Level::INFO => "color: whitesmoke; background: #444",
        Level::WARN => "color: orange; background: #444",
        Level::ERROR => "color: red; background: #444",
    }
}

/// The message of an event, then each field as `name = value;`, one per line.
#[derive(Default)]
struct Text {
    message: String,
    fields: String,
}

impl Visit for Text {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        // Writing into a `String` cannot fail.
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
        } else {
            let separator = if self.fields.is_empty() { " " } else { "\n" };
            let _ = write!(self.fields, "{separator}{} = {value:?};", field.name());
        }
    }
}

impl fmt::Display for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.message.is_empty(), self.fields.is_empty()) {
            (true, true) => Ok(()),
            (false, true) => write!(f, " {}", self.message),
            (true, false) => write!(f, " {}", self.fields),
            (false, false) => write!(f, " {}{}", self.message, self.fields),
        }
    }
}
