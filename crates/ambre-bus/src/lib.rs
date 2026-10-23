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

mod token;

pub mod activity;
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
pub use activity::{ActivityAnchor, ActivityKind, ActivitySummary};
// The Murmure data plane, addressed to a channel rather than a document.
pub use app::murmure::{
    Attachment, ByteSize, CatalogPosition, CatalogueEntry, CategoryView, ChannelActivity,
    ChannelDocument, ChannelMatches, ChannelMeta, ChannelName, ChannelPlace, DirectMember,
    DirectMessageDraft, DirectMessageEvent, DirectPeers, DocumentActivity, Emoji, HeldReply,
    HeldSend, MediaRef, MediaType, MemberChannel, MemberView, Mention, MentionCount, MentionTarget,
    MessageAt, MessageDraft, MessageView, MurmureCommand, MurmureEvent, Notice, Picture,
    PictureFrame, Presence, PrivateChannelDraft, PublicChannelDraft, Reaction, ReplyEcho,
    SealedChannelKey, SearchMatch, SpaceCommand, SpaceEvent, StyleMark, TextStyle, ThreadHead,
    Timestamp, Topic, UnreadCount, Unreadable,
};
pub use command::SyncCommand;
pub use event::{
    ApiRequest, FoldAnswer, LeaseAnswer, LogicEvent, OpenFailure, SignedDelta, SnapshotPurpose,
    SnapshotVerdict,
};
// Trame data plane (command + event + value types) lives under `app::trame`;
// re-exported flat at the crate root so downstream crates keep `ambre_bus::...`.
pub use app::trame::{
    BorderEdit, BorderPreset, BorderStyle, Borders, CellBorder, CellFormat, CellPos, CellRect,
    CellStyle, ClipboardCell, ClipboardPayload, Col, ColCount, ColDelta, ColSpan, ColVec,
    CondFormatId, CondFormatKind, CondStyle, CsvDelimiter, DefinedName, DisplayCell, EditSessionId,
    FillMode, FilterKind, FilterOp, FilterViewId, HAlign, HeaderRow, MergeSpan, Percent, Row,
    RowCount, RowDelta, RowSpan, RowVec, SelectionKind, SheetId, SheetName, SheetNameKey,
    SheetNameRefusal, SheetTab, SortKey, SortOrder, StylePatch, TrameCommand, TrameEvent, VAlign,
    ValidationKind, ValidationOnInvalid,
};
pub use logic::{AppCommand, LogicCommand};
pub use lossy::{LossyOutcome, send_lossy};
pub use outbox::{Outbox, SendOutcome};
pub use rest::{
    DocumentSummary, DraftKey, MemberInfo, MemberStanding, RequestId, RestCommand, RestEvent,
    RowPlace, SealedRow, SecretBytes, SecretString, UserInfo,
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

/// Whose copy the clipboard holds when a paste reads it. The ownership token
/// decides, never the text (`UI-THREAD-007`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum PasteSource {
    /// Another tab or another program wrote the clipboard last.
    #[default]
    Foreign,
    /// This tab wrote the clipboard last, so the view's kept copy is the same.
    Own,
}

impl ClipboardBlob {
    /// The blob a paste inserts: the copy the view kept in memory, or what
    /// the browser read. A plain paste takes the read, whose richer halves the
    /// host already dropped. Otherwise the kept copy wins only while the
    /// clipboard carries this tab's ownership token. A text match never
    /// decides: two tabs holding the same text are two copies
    /// (`UI-THREAD-007`).
    #[must_use]
    pub fn source<'a>(kept: Option<&'a Self>, read: &'a Self, source: PasteSource) -> &'a Self {
        if read.plain {
            return read;
        }
        match (kept, source) {
            (Some(kept), PasteSource::Own) => kept,
            _ => read,
        }
    }

    /// Prefix `html` with the ownership token, for a browser that refuses the
    /// `web ` types and so keeps no token of its own (`UI-THREAD-057`). A
    /// comment, so another editor renders nothing of it.
    #[must_use]
    pub fn mark_html(html: &str, token: &ClipToken) -> String {
        let token = token.as_str();
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

/// The version of a view's request, echoed by the Logic thread's answer: the
/// view drops an answer older than the last it accepted (`BUS-017`). A
/// distinct type, so a sequence number or a count never takes its place.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(transparent)]
pub struct ViewportVersion(pub u64);

impl ViewportVersion {
    /// The version after this one.
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

/// This tab's clipboard ownership token (`UI-THREAD-007`): written beside a
/// copy, and read back to know a paste is of this tab's own copy. A distinct
/// type, so a text the clipboard carries never stands for it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClipToken(String);

impl ClipToken {
    /// The token the tab drew when its page loaded.
    pub fn new(text: String) -> Self {
        Self(text)
    }

    /// The token, as the clipboard carries it.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether `read`, a token a paste carried, is this one.
    #[must_use]
    pub fn is(&self, read: Option<&str>) -> bool {
        read == Some(self.0.as_str())
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
/// The text offsets, one family per unit, for a crate that reaches the core
/// through this one (`ambre-canvas`).
pub use ambre_core::text_offset::{
    ByteCount, ByteDelta, ByteOffset, ByteSpan, CharCount, CharDelta, CharOffset, CharSpan,
    Utf16Offset, byte_span_to_chars, byte_to_char, char_len, char_span_to_bytes, char_to_byte,
};
pub use ambre_core::{
    ApiFailure, CategoryId, ChannelDocumentKind, ChannelId, CommentId, DocType, DocumentId, Email,
    FootnoteId, HttpStatus, KeyFingerprint, LeaseId, MeetingId, MessageId, MetaId, Millis,
    PageTime, PublicKey, RefusalCode, ReplyId, Role, SegmentId, Seq, SnapshotId, SpaceId,
    SuggestionId, UnreadableCause, UserId, Uuid,
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
        let marked = ClipboardBlob::mark_html("<b>x</b>", &super::ClipToken::new("TOK".to_owned()));
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
        assert_eq!(
            ClipboardBlob::source(Some(&kept), &same_text, super::PasteSource::Own),
            &kept
        );
        // No token: the same text from another tab is another copy.
        assert_eq!(
            ClipboardBlob::source(Some(&kept), &same_text, super::PasteSource::Foreign),
            &same_text
        );
        // Nothing was kept.
        assert_eq!(
            ClipboardBlob::source(None, &same_text, super::PasteSource::Own),
            &same_text
        );
        // A plain paste never takes the kept copy.
        let plain = ClipboardBlob {
            plain: true,
            ..blob("a\tb", 5)
        };
        assert_eq!(
            ClipboardBlob::source(Some(&kept), &plain, super::PasteSource::Own),
            &plain
        );
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
