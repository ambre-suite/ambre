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
//!   `Id<Document>`. (See `ambre-spec-message-bus.md`.)
//! - **Service** (`RestCommand`/`RestEvent` + `Request` in [`rest`]) -- the
//!   correlatable request/response family carried by `...::Rest { id, ... }` on all
//!   four envelopes, defined once and forwarded verbatim.
//!
//! Transport split: `LogicCommand`/`UiEvent` travel UI <-> Logic over the [`conduit`] queues
//! in shared memory (typed Rust values, no serialization). The `SyncCommand`/`LogicEvent`
//! envelope crosses a `postMessage` boundary to the separate Sync worker. It, and the
//! `Rest*` payloads it carries, derives `Serialize`/`Deserialize` and is postcard-encoded
//! on that hop.

pub mod token;

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
pub use activity::{Activity, ActivityAnchor, ActivityKind, ActivitySummary, MentionScope};
// The Murmure data plane, addressed to a channel rather than a document.
pub use app::murmure::{
    Attachment, Authored, ByteSize, CatalogPosition, CatalogueEntry, CategoryView, ChannelActivity,
    ChannelDocument, ChannelMatches, ChannelMeta, ChannelName, ChannelPlace, ChannelTopic,
    Confirmation, ContentHash, Deletion, DirectMember, DirectMessageDraft, DirectMessageEvent,
    DirectPeers, DocumentActivity, Emoji, HeldReply, HeldSend, MediaType, MemberChannel,
    MemberView, Mention, MentionTarget, Mentions, MessageAt, MessageDraft, MessageView, Messages,
    MurmureCommand, MurmureEvent, Notice, Picture, PictureFrame, Presence, PrivateChannelDraft,
    PublicChannelDraft, Reaction, Replies, ReplyEcho, Revision, SealedChannelKey, SearchMatch,
    Segments, SpaceCommand, SpaceEvent, StyleMark, TextStyle, ThreadHead, Unread, Unreadable,
};
pub use command::SyncCommand;
pub use event::{
    ApiRequest, FoldAnswer, LeaseAnswer, LogicEvent, OpenFailure, SignedDelta, SnapshotPurpose,
    SnapshotVerdict, Successor,
};
// Trame data plane (command + event + value types) lives under `app::trame`;
// re-exported flat at the crate root so downstream crates keep `ambre_bus::...`.
pub use app::trame::{
    BorderEdit, BorderPreset, BorderStyle, Borders, CellBorder, CellFormat, CellPos, CellRect,
    CellStyle, CellValueKind, ClipboardCell, ClipboardPayload, Col, CondFormatKind,
    CondFormatPriority, CondStyle, CsvDelimiter, DateParts, DayOfMonth, Decimals, DecimalsStep,
    DefinedName, DisplayCell, EditSession, FillMode, FilterKind, FilterOp, HAlign, HeaderRow,
    Merge, MergeSpan, Month, MonthCount, Percent, Row, RowFilter, SelectionKind, SheetName,
    SheetNameKey, SheetNameRefusal, SheetTab, Slant, SortKey, SortOrder, Strikethrough, StylePatch,
    TabStrip, TrameCommand, TrameEvent, Underline, VAlign, ValidationKind, ValidationOnInvalid,
    Weight, WordBreak, Year,
};
pub use logic::{AppCommand, LogicCommand, StoreRoom};
pub use lossy::{LossyOutcome, send_lossy};
pub use outbox::{Load, Outbox, SendOutcome};
pub use rest::{
    Confidential, DocumentSummary, DraftKey, KeySetup, MemberInfo, MemberStanding, Plaintext,
    RecoveryWords, Request, RestCommand, RestEvent, RowPlace, SealedRow, UserInfo, next_request,
};
pub use sink::CommandSink;
pub use token::{Token, TokenKind};
pub use ui::{AppEvent, MentionState, UiEvent};

/// App-agnostic clipboard transport carried across the host <-> view seam and, serialized,
/// across the OS clipboard. `text` is the `text/plain` mirror for cross-app paste. `ambre`
/// is the opaque, app-specific payload (postcard of the active app's typed clipboard
/// payload) under the private `application/x-ambre` MIME type. Each app (de)serializes its
/// own payload behind these bytes; a cross-app paste falls back to `text` when the bytes
/// fail to decode.
#[derive(Clone, Default, PartialEq)]
pub struct ClipboardBlob {
    pub text: String,
    /// The `text/html` half, for an application that reads formatting but
    /// not Ambre's own bytes. `None` when the active app writes none.
    pub html: Option<String>,
    pub ambre: Option<ambre_core::Payload<ambre_core::ClipboardBytes>>,
    /// [`PasteForm::Plain`] when the user asked for the plain text alone: a
    /// view ignores every richer half, and the copy it kept in memory too.
    pub form: ambre_core::PasteForm,
}

// The two text halves are the author's: a log carries their length alone.
impl std::fmt::Debug for ClipboardBlob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClipboardBlob")
            .field("text", &ambre_core::RedactedChars::of(&self.text))
            .field(
                "html",
                &self.html.as_deref().map(ambre_core::RedactedChars::of),
            )
            .field("ambre", &self.ambre)
            .field("form", &self.form)
            .finish()
    }
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
        if read.form == ambre_core::PasteForm::Plain {
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
    /// browser moved the comment. An empty comment carries none.
    #[must_use]
    pub fn html_token(html: &str) -> Option<ClipToken> {
        let start = html.find(HTML_TOKEN_OPEN)? + HTML_TOKEN_OPEN.len();
        let rest = html.get(start..)?;
        ClipToken::new(rest.get(..rest.find(HTML_TOKEN_CLOSE)?)?.to_owned())
    }
}

/// The version of a view's request, echoed by the Logic thread's answer: the
/// view drops an answer older than the last it accepted (`BUS-017`). A
/// distinct family, so a sequence number or a count never takes its place.
/// It crosses the bus, so its serde form is the bare number. It saturates
/// rather than wraps, so a wrapped answer never reads as an older one.
pub enum ViewportVersion {}

impl ambre_core::SeqKind for ViewportVersion {
    type IssueRight = ();
    const NAME: &'static str = "ViewportVersion";
}

impl ambre_core::seq::Wire for ViewportVersion {}

/// This tab's clipboard ownership token (`UI-THREAD-007`): written beside a
/// copy, and read back to know a paste is of this tab's own copy. A distinct
/// type, so a text the clipboard carries never stands for it. It is never
/// empty, so an empty read never matches it: a paste compares two tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipToken(String);

impl ClipToken {
    /// A token the clipboard carried, or `None` when it is empty.
    #[must_use]
    pub fn new(text: String) -> Option<Self> {
        (!text.is_empty()).then_some(Self(text))
    }

    /// The token a tab draws when its page loads, from a `random` draw.
    #[must_use]
    pub fn draw(random: u64) -> Self {
        Self(format!("ambre-{random:x}"))
    }

    /// The token, as the clipboard carries it.
    pub fn as_str(&self) -> &str {
        &self.0
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
#[derive(Clone, PartialEq)]
pub enum CopyOutcome {
    Nothing,
    Text(String),
    Deferred,
}

// The selected text is the author's: a log carries its length alone.
impl std::fmt::Debug for CopyOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Nothing => f.write_str("Nothing"),
            Self::Text(text) => f
                .debug_tuple("Text")
                .field(&ambre_core::RedactedChars::of(text))
                .finish(),
            Self::Deferred => f.write_str("Deferred"),
        }
    }
}
// Re-export core types so that users don't need to depend on ambre-core directly.
/// The text offsets, one family per unit, for a crate that reaches the core
/// through this one (`ambre-canvas`).
pub use ambre_core::text_offset::{
    Byte, Char, Utf16, byte_span_to_chars, byte_to_char, char_len, char_span_to_bytes, char_to_byte,
};
pub use ambre_core::{
    ApiFailure, ChannelDocumentKind, DocType, Email, FixedBytes, FracMs, HttpStatus, Instant, Ms,
    PageLoad, PublicKey, RefusalCode, Relay, Role, Seq, TimeSpan, UnreadableCause, Uuid,
};

#[cfg(test)]
mod clipboard_tests {
    use super::ClipboardBlob;

    fn blob(text: &str, ambre: u8) -> ClipboardBlob {
        ClipboardBlob {
            text: text.into(),
            ambre: Some(ambre_core::Payload::new(vec![ambre])),
            ..ClipboardBlob::default()
        }
    }

    /// A blob and a copied selection print the length of their text alone.
    #[test]
    fn a_copied_text_prints_its_length_alone() {
        let secret = "Q3 layoffs";
        let mut copied = blob(secret, 1);
        copied.html = Some(secret.to_owned());
        let printed = [
            format!("{copied:?}"),
            format!("{:?}", super::CopyOutcome::Text(secret.to_owned())),
        ];
        for line in printed {
            assert!(line.contains("10 chars"), "{line}");
            assert!(!line.contains(secret), "{line}");
        }
    }

    /// `UI-THREAD-007`: an empty token is no token, so an empty read or an
    /// empty comment never stands for this tab's copy.
    #[test]
    fn an_empty_token_never_matches() {
        assert_eq!(super::ClipToken::new(String::new()), None);
        assert_eq!(
            ClipboardBlob::html_token("<!--ambre-token:--><b>x</b>"),
            None
        );
        assert_ne!(super::ClipToken::draw(0).as_str(), "");
    }

    /// `UI-THREAD-057`: the token survives in the HTML as Firefox 156 hands it
    /// back, the comment moved before the `<html>` wrapper it adds.
    #[test]
    fn the_token_reads_back_from_the_marked_html() {
        let tok = super::ClipToken::new("TOK".to_owned()).unwrap();
        let marked = ClipboardBlob::mark_html("<b>x</b>", &tok);
        assert_eq!(ClipboardBlob::html_token(&marked), Some(tok.clone()));
        let moved = "<!--ambre-token:TOK-->\n<html><head></head><body><b>x</b></body></html>";
        assert_eq!(ClipboardBlob::html_token(moved), Some(tok));
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
            form: ambre_core::PasteForm::Plain,
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
