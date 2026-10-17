//! IPC message types for inter-thread communication in Ambre.
//!
//! Four thread envelopes, one per queue direction:
//! - `LogicCommand` (UI -> Logic) and `UiEvent` (Logic -> UI) -- see [`ui`].
//! - `SyncCommand` (Logic -> Sync) -- see [`command`].
//! - `LogicEvent` (Sync -> Logic) -- see [`event`].
//!
//! Plus the typed data planes carried inside the envelopes:
//! - **Content** -- the `AppCommand` / `AppEvent` closed sums live next to their
//!   envelopes ([`logic`] / [`ui`]). Each document type's whole data plane is grouped
//!   **one module per app** under [`app`]: its command, its event, its shared value
//!   and display types. Those modules (`app::trame`, `app::toile`) are re-exported
//!   flat at the crate root. Document edits and viewport data, routed by
//!   `DocumentId`. (See `ambre-spec-message-bus.md`.)
//! - **Service** (`RestCommand`/`RestEvent` + `RequestId` in [`rest`]) -- the
//!   correlatable request/response family carried by `...::Rest { id, ... }` on all
//!   four envelopes, defined once and forwarded verbatim.
//!
//! Transport split: `LogicCommand`/`UiEvent` travel UI <-> Logic over the [`conduit`] queues
//! in shared memory (typed Rust values, no serialization). The `SyncCommand`/`LogicEvent`
//! envelope crosses a `postMessage` boundary to the separate Sync worker. It, and the
//! `Rest*` payloads it carries, derives `Serialize`/`Deserialize` and is postcard-encoded
//! on that hop.

pub mod app;
pub mod command;
pub mod conduit;
pub mod event;
pub mod logic;
pub mod lossy;
pub mod outbox;
pub mod rest;
pub mod sink;
pub mod start;
pub mod sync_warning;
pub mod ui;

// Re-export the main types for convenient use.
pub use command::SyncCommand;
pub use event::{LogicEvent, SignedDelta, SnapshotPurpose, SnapshotVerdict};
// Trame data plane (command + event + value types) lives under `app::trame`;
// re-exported flat at the crate root so downstream crates keep `ambre_bus::...`.
pub use app::trame::{
    BORDER_EDGE_ALL, BORDER_EDGE_BOTTOM, BORDER_EDGE_LEFT, BORDER_EDGE_RIGHT, BORDER_EDGE_TOP,
    BorderEdit, BorderPreset, BorderStyle, Borders, CellBorder, CellFormat, CellStyle,
    ClipboardCell, ClipboardPayload, CondFormatId, CondFormatKind, CondStyle, DisplayCell,
    FillMode, FilterKind, FilterOp, FilterViewId, GridRange, HAlign, SelectionKind, SheetId,
    SheetTab, SortKey, StylePatch, TrameCommand, TrameEvent, VAlign, ValidationKind,
    ValidationOnInvalid,
};
pub use logic::{AppCommand, LogicCommand};
pub use lossy::{LossyOutcome, send_lossy};
pub use outbox::{Outbox, SendOutcome};
pub use rest::{
    ChannelSummary, DocumentSummary, MemberInfo, MemberStanding, RequestId, RestCommand, RestEvent,
    SecretBytes, SecretString, UnreadCount, UserInfo,
};
pub use sink::CommandSink;
pub use ui::{AppEvent, UiEvent};

/// App-agnostic clipboard transport carried across the host <-> view seam and, serialized,
/// across the OS clipboard. `text` is the `text/plain` mirror for cross-app paste. `ambre`
/// is the opaque, app-specific payload (postcard of the active app's typed clipboard
/// payload) under the private `application/x-ambre` MIME type. Each app (de)serializes its
/// own payload behind these bytes; a cross-app paste falls back to `text` when the bytes
/// fail to decode.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClipboardBlob {
    pub text: String,
    /// The `text/html` half, for an application that reads formatting but
    /// not Ambre's own bytes. `None` when the active app writes none.
    pub html: Option<String>,
    pub ambre: Option<Vec<u8>>,
    /// The user asked for the plain text alone: a view ignores every richer
    /// half, and the copy it kept in memory too.
    pub plain: bool,
}

impl ClipboardBlob {
    /// The blob a paste inserts: the copy the view kept in memory, or what
    /// the browser read. A plain paste takes the read, whose richer halves the
    /// host already dropped. Otherwise the kept copy wins only while the
    /// clipboard carries this tab's ownership token. A text match never
    /// decides: two tabs holding the same text are two copies
    /// (`UI-THREAD-007`).
    #[must_use]
    pub fn source<'a>(kept: Option<&'a Self>, read: &'a Self, own_copy: bool) -> &'a Self {
        if read.plain {
            return read;
        }
        match kept {
            Some(kept) if own_copy => kept,
            _ => read,
        }
    }

    /// Prefix `html` with the ownership token, for a browser that refuses the
    /// `web ` types and so keeps no token of its own (`UI-THREAD-057`). A
    /// comment, so another editor renders nothing of it.
    #[must_use]
    pub fn mark_html(html: &str, token: &str) -> String {
        format!("{HTML_TOKEN_OPEN}{token}{HTML_TOKEN_CLOSE}{html}")
    }

    /// The ownership token [`Self::mark_html`] left in `html`, wherever the
    /// browser moved the comment.
    #[must_use]
    pub fn html_token(html: &str) -> Option<&str> {
        let start = html.find(HTML_TOKEN_OPEN)? + HTML_TOKEN_OPEN.len();
        let rest = html.get(start..)?;
        rest.get(..rest.find(HTML_TOKEN_CLOSE)?)
    }
}

const HTML_TOKEN_OPEN: &str = "<!--ambre-token:";
const HTML_TOKEN_CLOSE: &str = "-->";

/// What a `copy`/`cut` on the focused view resolves to synchronously, inside the
/// native clipboard event (`trame-clipboard`). The authoritative payload
/// is built asynchronously on the Logic thread, so the sync event only decides
/// how the host should treat the browser default:
/// - `Nothing` -- nothing selected; let the browser default proceed.
/// - `Text` -- a synchronous in-cell / inline text selection the view already
///   holds; the host `setData`s it and prevents the default.
/// - `Deferred` -- the view emitted the authoritative copy command to Logic. The
///   host prevents the default, so the browser does not clobber the OS clipboard
///   with empty content. One frame later it drains
///   [`poll_clipboard`](crate::ClipboardBlob) into the async clipboard write.
#[derive(Clone, Debug, PartialEq)]
pub enum CopyOutcome {
    Nothing,
    Text(String),
    Deferred,
}
// Re-export core types so that users don't need to depend on ambre-core directly.
pub use ambre_core::{
    ChannelId, DocType, DocumentId, MeetingId, PublicKey, Role, SnapshotId, UserId, Uuid,
};

#[cfg(test)]
mod clipboard_tests {
    use super::ClipboardBlob;

    fn blob(text: &str, ambre: u8) -> ClipboardBlob {
        ClipboardBlob {
            text: text.into(),
            ambre: Some(vec![ambre]),
            ..ClipboardBlob::default()
        }
    }

    /// `UI-THREAD-057`: the token survives in the HTML as Firefox 156 hands it
    /// back, the comment moved before the `<html>` wrapper it adds.
    #[test]
    fn the_token_reads_back_from_the_marked_html() {
        let marked = ClipboardBlob::mark_html("<b>x</b>", "TOK");
        assert_eq!(ClipboardBlob::html_token(&marked), Some("TOK"));
        let moved = "<!--ambre-token:TOK-->\n<html><head></head><body><b>x</b></body></html>";
        assert_eq!(ClipboardBlob::html_token(moved), Some("TOK"));
        assert_eq!(ClipboardBlob::html_token("<b>x</b>"), None);
        assert_eq!(ClipboardBlob::html_token("<!--ambre-token:TOK"), None);
    }

    /// `UI-THREAD-053`: one choice between the kept copy and the read, shared
    /// by every view, and decided by the token alone (`UI-THREAD-007`).
    #[test]
    fn a_paste_takes_the_kept_copy_only_on_the_token() {
        let kept = blob("a\tb", 1);
        let same_text = blob("a\tb", 2);
        // The token says the clipboard is ours.
        assert_eq!(ClipboardBlob::source(Some(&kept), &same_text, true), &kept);
        // No token: the same text from another tab is another copy.
        assert_eq!(
            ClipboardBlob::source(Some(&kept), &same_text, false),
            &same_text
        );
        // Nothing was kept.
        assert_eq!(ClipboardBlob::source(None, &same_text, true), &same_text);
        // A plain paste never takes the kept copy.
        let plain = ClipboardBlob {
            plain: true,
            ..blob("a\tb", 5)
        };
        assert_eq!(ClipboardBlob::source(Some(&kept), &plain, true), &plain);
    }

    /// Every `.rs` file under `dir`, at any depth.
    fn rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for path in entries.flatten().map(|entry| entry.path()) {
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }

    /// `UI-THREAD-043`: no source but the launcher's clipboard module spells a
    /// clipboard type, so no application can declare one of its own.
    #[test]
    fn only_the_launcher_names_a_clipboard_type() {
        // A private type is an `x-` type or a `web ` one. Comments may name
        // them; code may not. The needles are built from pieces, so this file
        // does not match its own scan.
        let needles = [concat!("/", "x-"), concat!("\"web", " ")];
        let spells = |source: String| {
            source
                .lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .any(|line| needles.iter().any(|n| line.contains(n)))
        };
        let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let owner = std::path::Path::new("ambre-app/src/host/mod.rs");
        // The browser belt reads the types the page wrote; it declares none.
        let belt = crates.join("ambre-belt");
        let mut files = Vec::new();
        rust_files(&crates, &mut files);
        assert!(files.len() > 100, "the scan reads the workspace");
        let offenders: Vec<_> = files
            .iter()
            .filter(|path| !path.ends_with(owner) && !path.starts_with(&belt))
            .filter(|path| std::fs::read_to_string(path).is_ok_and(spells))
            .collect();
        assert!(
            offenders.is_empty(),
            "clipboard types named in {offenders:?}"
        );
    }
}
