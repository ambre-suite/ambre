//! The guard every test of the db tier logs through (`ZK-030`, `SRV-309`).
//!
//! The gate of `ZK-030` reads the type of each logged value. An identifier
//! first formatted into a text passes it: a foreign error that echoes a key,
//! or a key held as bare text. This guard reads the text instead. It reads
//! every event of the relay at the info level or above. An event whose fields
//! spell a UUID or a long hex run fails the test that logged it.
//!
//! The entry points of the db tier install it once per process. A line that
//! a spawned task logs panics that task alone. So the guard also records the
//! line, and the next entry point fails on it.

#![cfg(test)]

use std::fmt::Write as _;
use std::sync::{Mutex, Once};

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Metadata, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, SubscriberExt as _};

/// The lines the process guard refused, kept for the next entry point.
static REFUSED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Installs the process guard, once, and fails on a line it refused since.
///
/// # Panics
///
/// An earlier test logged an identifier at the info level or above.
pub fn install() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let subscriber = tracing_subscriber::registry().with(NoIdentifier {
            refused: &REFUSED,
            on_refusal: OnRefusal::Panic,
        });
        // A subscriber another harness set first keeps its place.
        let _ = tracing::subscriber::set_global_default(subscriber);
    });
    let refused = REFUSED.lock().unwrap_or_else(|e| e.into_inner());
    assert!(
        refused.is_empty(),
        "a log line at the info level or above names an identifier: {refused:?}"
    );
}

/// What the guard does past the record of a refused line.
#[derive(Clone, Copy)]
pub enum OnRefusal {
    /// Fails the thread that logged it.
    Panic,
    /// Records it alone, for the self-test of the guard.
    Record,
}

/// The layer that refuses an identifier in a line of the relay.
pub struct NoIdentifier {
    /// Where a refused line is kept.
    pub refused: &'static Mutex<Vec<String>>,
    /// What the guard does once it kept the line.
    pub on_refusal: OnRefusal,
}

/// Whether `metadata` is an event of the relay at the info level or above.
fn read(metadata: &Metadata<'_>) -> bool {
    *metadata.level() <= Level::INFO && metadata.target().starts_with("ambre_server")
}

/// The text of each field of an event, as the formatter writes it.
struct Line(String);

impl Visit for Line {
    fn record_str(&mut self, field: &Field, value: &str) {
        let _ = write!(self.0, " {}={value}", field.name());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let _ = write!(self.0, " {}={value:?}", field.name());
    }
}

impl<S: Subscriber> Layer<S> for NoIdentifier {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        if !read(event.metadata()) {
            return;
        }
        let mut line = Line(String::new());
        event.record(&mut line);
        // The redaction of a store failure is the one reader of an
        // identifier in a text: a line it changes names one.
        if crate::error::FaultText::bare(&line.0).to_string() == line.0 {
            return;
        }
        let refused = format!("{}:{}", event.metadata().target(), line.0);
        self.refused
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(refused.clone());
        if let OnRefusal::Panic = self.on_refusal {
            panic!("a log line at the info level or above names an identifier: {refused}");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use tracing_subscriber::layer::SubscriberExt as _;

    use super::{NoIdentifier, OnRefusal};

    /// `SRV-309`: the guard refuses an info, warn or error line of the relay
    /// that names an identifier, in its message or in a field. It
    /// passes a debug line and a line that names none.
    #[test]
    fn the_guard_refuses_an_info_line_that_names_an_identifier() {
        static REFUSED: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let subscriber = tracing_subscriber::registry().with(NoIdentifier {
            refused: &REFUSED,
            on_refusal: OnRefusal::Record,
        });
        let doc = "0192f3a4-5b6c-7d8e-9fab-0123456789ab";
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("flush failed seq=4");
            tracing::warn!(error = %"store down", "a call seat was not refreshed");
            tracing::debug!(document = %doc, "debug names the document");
            tracing::error!("batch put dev/documents/{doc}/batches/1-2: refused");
            tracing::warn!(key = %format!("doc:{{{doc}}}:dirty"), "dirty mark failed");
        });
        let refused = REFUSED.lock().unwrap().clone();
        assert_eq!(refused.len(), 2, "{refused:?}");
        assert!(refused.iter().all(|line| line.contains(doc)), "{refused:?}");
    }
}
