//! The subscriber of the Main instance: each event the filter keeps becomes
//! one console line, and in a debug build a timing mark.
//!
//! It replaces the `tracing-subscriber` registry and the `tracing-wasm` layer.
//! The Main instance carried about 20 KB of wasm for one filter and one
//! console call (`ambre-spec-budgets.md` section 5). The line keeps the shape
//! the layer printed: the level, the file and line, the message and its fields.
//! A span is refused by a console line of its own, as `ARCH-092` states: the
//! subscriber keeps no span tree.
//!
//! Each method builds what it prints as a value, and `print` hands that value
//! to the browser. A host test reads the same values from `print` instead, so
//! the native tests drive this subscriber and no copy of it.

use std::fmt::{self, Write as _};
use std::sync::atomic::{AtomicU64, Ordering};

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::Interest;
use tracing::{Event, Level, Metadata, Subscriber};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::wasm_bindgen;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = log)]
    fn console_log(line: &str, level_style: &str, origin_style: &str, text_style: &str);
    #[wasm_bindgen(js_namespace = performance, js_name = mark)]
    fn performance_mark(name: &str);
    #[wasm_bindgen(catch, js_namespace = performance, js_name = measure)]
    fn performance_measure(name: &str, start: &str) -> Result<(), wasm_bindgen::JsValue>;
}

/// The build the subscriber runs in, which sets what it prints.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Build {
    /// Our events up to INFO, and no timing mark.
    Release,
    /// Our events up to DEBUG, each with a timing mark. A release binary
    /// cannot build it, so it links no timing code.
    #[cfg(debug_assertions)]
    Debug,
}

/// The subscriber: our crates at the level of the build, every dependency at
/// WARN.
pub struct ConsoleTrace {
    build: Build,
    marks: AtomicU64,
}

/// The target prefix of every crate of the workspace.
const OUR_TARGETS: &str = "ambre";

/// The style of the second segment of a line: the file and line.
const ORIGIN_STYLE: &str = "color: gray; font-style: italic";

/// The style of the third segment of a line: the message and its fields.
const TEXT_STYLE: &str = "color: inherit";

/// One console line: the text with its three `%c` directives, and the style of
/// the first segment. The two other segments always take `ORIGIN_STYLE` and
/// `TEXT_STYLE`.
#[derive(Clone, PartialEq, Eq, Debug)]
struct ConsoleLine {
    text: String,
    level_style: &'static str,
}

/// One timing mark: the name of the mark, then the label of the measure that
/// starts at it.
#[derive(Clone, PartialEq, Eq, Debug)]
struct TimingMark {
    mark: String,
    measure: String,
}

impl ConsoleTrace {
    /// The subscriber of `build`.
    pub const fn new(build: Build) -> Self {
        Self {
            build,
            marks: AtomicU64::new(0),
        }
    }

    /// The most verbose level our own events print at.
    const fn ours(&self) -> Level {
        match self.build {
            Build::Release => Level::INFO,
            #[cfg(debug_assertions)]
            Build::Debug => Level::DEBUG,
        }
    }

    /// The most verbose level `meta` may print at. A target matches by its
    /// prefix, as a `Targets` directive does.
    fn ceiling(&self, meta: &Metadata<'_>) -> Level {
        if meta.target().starts_with(OUR_TARGETS) {
            self.ours()
        } else {
            Level::WARN
        }
    }

    /// The timing mark of the next event, or `None` when timings are off. The
    /// mark names count up from `c0`, in hexadecimal, so no two collide.
    fn timing(&self, meta: &Metadata<'_>, text: &Text) -> Option<TimingMark> {
        // Timing marks are a debug aid, so a release build does not stamp them.
        match self.build {
            Build::Release => None,
            #[cfg(debug_assertions)]
            Build::Debug => Some(TimingMark {
                mark: format!("c{:x}", self.marks.fetch_add(1, Ordering::Relaxed)),
                measure: format!(
                    "{} {}{text}",
                    meta.level(),
                    meta.module_path().unwrap_or("...")
                ),
            }),
        }
    }
}

/// The file and line of a callsite, or nothing when tracing has neither.
struct Origin<'a>(&'a Metadata<'a>);

impl fmt::Display for Origin<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.0.file(), self.0.line()) {
            (Some(file), Some(line)) => write!(f, "{file}:{line}"),
            _ => Ok(()),
        }
    }
}

/// The line of an event: the level word, the origin, then the message and the
/// fields, each segment opened by a `%c` directive.
fn event_line(meta: &Metadata<'_>, text: &Text) -> ConsoleLine {
    let level = *meta.level();
    ConsoleLine {
        text: format!("%c{level}%c {}%c{text}", Origin(meta)),
        level_style: level_style(level),
    }
}

/// The line that refuses a span: the word `SPAN` in the colour of a warning,
/// the origin, then the name. It records no field, so a refused span costs
/// no visitor.
fn span_line(meta: &Metadata<'_>) -> ConsoleLine {
    ConsoleLine {
        text: format!("%cSPAN%c {}%c {}: refused", Origin(meta), meta.name()),
        level_style: level_style(Level::WARN),
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
            self.ours().max(Level::WARN),
        ))
    }

    fn new_span(&self, attrs: &Attributes<'_>) -> Id {
        print_line(&span_line(attrs.metadata()));
        Id::from_u64(1)
    }

    fn record(&self, _: &Id, _: &Record<'_>) {}

    fn record_follows_from(&self, _: &Id, _: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let mut text = Text::default();
        event.record(&mut text);
        let meta = event.metadata();
        print_line(&event_line(meta, &text));
        if let Some(mark) = self.timing(meta, &text) {
            print_mark(&mark);
        }
    }

    fn enter(&self, _: &Id) {}

    fn exit(&self, _: &Id) {}
}

/// Hands a line to the browser console.
#[cfg(target_arch = "wasm32")]
fn print_line(line: &ConsoleLine) {
    console_log(&line.text, line.level_style, ORIGIN_STYLE, TEXT_STYLE);
}

/// Hands a timing mark to the browser profile.
#[cfg(target_arch = "wasm32")]
fn print_mark(mark: &TimingMark) {
    performance_mark(&mark.mark);
    // A measure the browser refuses loses a blip in the profile, and
    // nothing else.
    let _ = performance_measure(&mark.measure, &mark.mark);
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
        // The fields open on their own space, so a line without a message
        // keeps one space before them.
        if !self.message.is_empty() {
            write!(f, " {}", self.message)?;
        }
        f.write_str(&self.fields)
    }
}

/// What a host test reads in place of the browser. It holds every line and
/// every mark of one thread, in the order the subscriber printed them.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod printed {
    use std::cell::RefCell;

    use super::{ConsoleLine, TimingMark};

    #[derive(Clone, PartialEq, Eq, Debug)]
    pub(super) enum Printed {
        Line(ConsoleLine),
        Mark(TimingMark),
    }

    thread_local! {
        static PRINTED: RefCell<Vec<Printed>> = const { RefCell::new(Vec::new()) };
    }

    pub(super) fn push(printed: Printed) {
        PRINTED.with(|all| all.borrow_mut().push(printed));
    }

    /// Everything printed on this thread since the last call.
    pub(super) fn take() -> Vec<Printed> {
        PRINTED.with(|all| all.take())
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
fn print_line(line: &ConsoleLine) {
    printed::push(printed::Printed::Line(line.clone()));
}

#[cfg(all(test, not(target_arch = "wasm32")))]
fn print_mark(mark: &TimingMark) {
    printed::push(printed::Printed::Mark(mark.clone()));
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use tracing::Level;

    use super::printed::{self, Printed};
    use super::{Build, ConsoleLine, ConsoleTrace, TimingMark, level_style};

    /// Runs `body` under the subscriber of `build`, and returns what it printed.
    fn printed_under(build: Build, body: impl FnOnce()) -> Vec<Printed> {
        printed::take();
        tracing::subscriber::with_default(ConsoleTrace::new(build), body);
        printed::take()
    }

    fn lines(printed: &[Printed]) -> Vec<&ConsoleLine> {
        printed
            .iter()
            .filter_map(|p| match p {
                Printed::Line(line) => Some(line),
                Printed::Mark(_) => None,
            })
            .collect()
    }

    #[test]
    fn our_targets_print_at_the_build_level_and_a_dependency_at_warn() {
        let printed = printed_under(Build::Debug, || {
            tracing::debug!(target: "ambre_logic::dispatch", "ours at debug");
            tracing::trace!(target: "ambre_logic::dispatch", "ours at trace");
            tracing::info!(target: "loro_internal::encoding", "dependency at info");
            tracing::warn!(target: "loro_internal::encoding", "dependency at warn");
            tracing::error!(target: "wgpu_core", "dependency at error");
        });
        let texts: Vec<_> = lines(&printed).iter().map(|l| l.text.clone()).collect();
        assert_eq!(texts.len(), 3, "{texts:?}");
        assert!(texts[0].ends_with(" ours at debug"));
        assert!(texts[1].ends_with(" dependency at warn"));
        assert!(texts[2].ends_with(" dependency at error"));
    }

    #[test]
    fn a_dropped_event_formats_nothing() {
        use std::cell::Cell;
        use std::fmt;

        /// A field that counts each time a subscriber formats it.
        struct Counted<'a>(&'a Cell<u32>);
        impl fmt::Debug for Counted<'_> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.set(self.0.get() + 1);
                f.write_str("counted")
            }
        }

        let formatted = Cell::new(0);
        let printed = printed_under(Build::Release, || {
            tracing::info!(target: "loro_internal::encoding", field = ?Counted(&formatted));
            tracing::debug!(target: "ambre_logic", field = ?Counted(&formatted), "dropped");
        });
        assert!(printed.is_empty(), "{printed:?}");
        assert_eq!(formatted.get(), 0);
    }

    #[test]
    fn a_release_build_drops_our_debug_events() {
        let printed = printed_under(Build::Release, || {
            tracing::debug!(target: "ambre_app", "dropped");
            tracing::info!(target: "ambre_app", "kept");
        });
        let texts: Vec<_> = lines(&printed).iter().map(|l| l.text.clone()).collect();
        assert_eq!(texts.len(), 1, "{texts:?}");
        assert!(texts[0].ends_with(" kept"));
    }

    #[test]
    fn the_level_hint_is_the_build_level() {
        use tracing::Subscriber as _;
        use tracing::level_filters::LevelFilter;
        let hint = |build| ConsoleTrace::new(build).max_level_hint();
        assert_eq!(hint(Build::Debug), Some(LevelFilter::DEBUG));
        assert_eq!(hint(Build::Release), Some(LevelFilter::INFO));
    }

    #[test]
    fn an_event_prints_one_styled_line_of_level_origin_message_and_fields() {
        let line = line!() + 2;
        let printed = printed_under(Build::Debug, || {
            tracing::warn!(target: "ambre_sync", seq = 7, doc = "d1", "late delta");
        });
        assert_eq!(
            lines(&printed),
            vec![&ConsoleLine {
                text: format!(
                    "%cWARN%c {}:{line}%c late delta seq = 7;\ndoc = \"d1\";",
                    file!()
                ),
                level_style: "color: orange; background: #444",
            }]
        );
    }

    #[test]
    fn each_level_word_takes_the_colour_the_layer_drew() {
        let printed = printed_under(Build::Debug, || {
            tracing::debug!(target: "ambre_x", "d");
            tracing::info!(target: "ambre_x", "i");
            tracing::warn!(target: "ambre_x", "w");
            tracing::error!(target: "ambre_x", "e");
        });
        let styles: Vec<_> = lines(&printed).iter().map(|l| l.level_style).collect();
        assert_eq!(
            styles,
            [
                "color: lawngreen; background: #444",
                "color: whitesmoke; background: #444",
                "color: orange; background: #444",
                "color: red; background: #444",
            ]
        );
        // No build prints our TRACE events; the colour stays the layer's.
        assert_eq!(
            level_style(Level::TRACE),
            "color: dodgerblue; background: #444"
        );
    }

    #[test]
    fn an_event_without_a_message_prints_its_fields_alone() {
        let printed = printed_under(Build::Debug, || {
            tracing::info!(target: "ambre_x", n = 3);
        });
        let texts: Vec<_> = lines(&printed).iter().map(|l| l.text.clone()).collect();
        assert_eq!(texts.len(), 1);
        assert!(texts[0].ends_with("%c n = 3;"), "{texts:?}");
    }

    #[test]
    fn a_debug_build_marks_each_event_with_a_measure_of_its_text() {
        let printed = printed_under(Build::Debug, || {
            tracing::info!(target: "ambre_x", "first");
            tracing::info!(target: "ambre_x", k = 1, "second");
        });
        let module = module_path!();
        let marks: Vec<_> = printed
            .iter()
            .filter_map(|p| match p {
                Printed::Mark(mark) => Some(mark.clone()),
                Printed::Line(_) => None,
            })
            .collect();
        assert_eq!(
            marks,
            [
                TimingMark {
                    mark: "c0".into(),
                    measure: format!("INFO {module} first"),
                },
                TimingMark {
                    mark: "c1".into(),
                    measure: format!("INFO {module} second k = 1;"),
                },
            ]
        );
        // The mark follows the line of its own event.
        assert!(matches!(printed[0], Printed::Line(_)));
        assert!(matches!(printed[1], Printed::Mark(_)));
    }

    #[test]
    fn a_release_build_leaves_no_timing_mark() {
        let printed = printed_under(Build::Release, || {
            tracing::info!(target: "ambre_x", "no mark");
        });
        assert!(printed.iter().all(|p| matches!(p, Printed::Line(_))));
        assert_eq!(printed.len(), 1);
    }

    #[test]
    fn a_span_opened_on_the_client_prints_a_line_that_refuses_it() {
        let line = line!() + 2;
        let printed = printed_under(Build::Debug, || {
            let span = tracing::info_span!(target: "ambre_logic", "keystroke_pass", n = 1);
            let _entered = span.enter();
            tracing::info!(target: "ambre_logic", "inside");
        });
        assert_eq!(
            printed.first(),
            Some(&Printed::Line(ConsoleLine {
                text: format!("%cSPAN%c {}:{line}%c keystroke_pass: refused", file!()),
                level_style: "color: orange; background: #444",
            }))
        );
        // The event inside the refused span still prints.
        assert_eq!(lines(&printed).len(), 2);
    }

    #[test]
    fn a_span_the_filter_drops_prints_nothing() {
        let printed = printed_under(Build::Release, || {
            let _span = tracing::debug_span!(target: "ambre_logic", "quiet").entered();
            let _dep = tracing::info_span!(target: "loro_internal", "import").entered();
        });
        assert!(printed.is_empty(), "{printed:?}");
    }
}

/// The browser half: the line and the mark reach the two browser APIs. Each
/// test swaps the API it reads for a recorder, then puts it back.
#[cfg(all(test, target_arch = "wasm32"))]
mod wasm_tests {
    use js_sys::{Array, Function, Reflect};
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::{Build, ConsoleTrace, ORIGIN_STYLE, TEXT_STYLE};

    fn global(name: &str) -> JsValue {
        Reflect::get(&js_sys::global(), &name.into()).unwrap_or(JsValue::UNDEFINED)
    }

    #[wasm_bindgen_test]
    fn a_printed_line_reaches_console_log_with_its_three_styles() {
        let console = global("console");
        let original = Reflect::get(&console, &"log".into()).unwrap();
        let calls = Array::new();
        let recorder = Function::new_with_args("...args", "this.push(args)").bind(&calls);
        Reflect::set(&console, &"log".into(), &recorder).unwrap();
        tracing::subscriber::with_default(ConsoleTrace::new(Build::Release), || {
            tracing::warn!(target: "ambre_x", "to the console");
        });
        Reflect::set(&console, &"log".into(), &original).unwrap();

        assert_eq!(calls.length(), 1);
        let args: Array = calls.get(0).unchecked_into();
        let arg = |i| args.get(i).as_string().unwrap_or_default();
        assert!(arg(0).starts_with("%cWARN%c "), "{}", arg(0));
        assert!(arg(0).ends_with("%c to the console"), "{}", arg(0));
        assert_eq!(arg(1), "color: orange; background: #444");
        assert_eq!(arg(2), ORIGIN_STYLE);
        assert_eq!(arg(3), TEXT_STYLE);
    }

    #[wasm_bindgen_test]
    fn a_debug_build_leaves_its_measure_in_the_browser_profile() {
        let label = format!("INFO {} profiled", module_path!());
        tracing::subscriber::with_default(ConsoleTrace::new(Build::Debug), || {
            tracing::info!(target: "ambre_x", "profiled");
        });
        let performance = global("performance");
        let by_name: Function = Reflect::get(&performance, &"getEntriesByName".into())
            .unwrap()
            .unchecked_into();
        let entries: Array = by_name
            .call1(&performance, &label.as_str().into())
            .unwrap()
            .unchecked_into();
        assert_eq!(entries.length(), 1);
    }
}
