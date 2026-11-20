//! Closed set of `SyncWarning` codes, carried by both [`crate::LogicEvent::SyncWarning`]
//! and [`crate::UiEvent::SyncWarning`].
//!
//! A `SyncWarning` is always **non-fatal**: the session keeps running, and so does the
//! document it names. The code is the contract that lets the UI map an event to a
//! user-facing notice. The event carries no text: the thread that raised it logs the
//! detail, and the view makes its words from the code (`BUS-046`). Every warning the Sync and Logic threads can raise is enumerated
//! here, in one crate. The consumer's `match` is therefore exhaustive, and the compiler
//! flags any new variant.

/// Why the Sync or Logic thread raised a (non-fatal) warning.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[repr(u16)]
pub enum SyncWarningCode {
    /// A single inbound message failed to decrypt/decode and was dropped.
    /// Transient and self-correcting (the CRDT reconverges from later updates).
    DecryptFailed = 1,

    /// The per-document quarantine hit its hard cap and the oldest held update
    /// was dropped to bound the shared heap. Convergent: the server still holds
    /// the history, re-fetched via `RequestMissing` once the parent snapshot
    /// arrives.
    QuarantineOverflow = 2,

    /// Raised by Logic. A delta waits on an operation this client never wrote
    /// and no delta it holds carries, such as another build's scaffolding.
    /// The document shows what it could read, and the rest never arrives
    /// (`ambre-spec-op-authorship.md`, `AUTHOR-025`).
    DocumentUnreadable = 3,

    /// Raised by Logic. The browser refuses to store a change the relay has
    /// not acknowledged. Editing pauses until the change is stored or the
    /// relay holds it (`ambre-spec-residency.md`, `RESID-018`).
    LocalStoreFull = 4,

    /// A local update named a document with no open session, so it never left
    /// for the relay. Said aloud rather than dropped in silence (`SYNC-069`).
    UpdateRefused = 5,

    /// An edit of this document will never reach the relay, for one of two
    /// causes. The relay refused it before it took a place in its history
    /// (`ambre-spec-sync.md`, `SYNC-155`). The client dropped the edits it held
    /// while the log of a channel walked again past the bound of that walk
    /// (`ambre-spec-membership.md`, `SYNC-234`). The edit stays on this client
    /// only, and nothing more is sent until the document reopens.
    EditRefused = 6,

    /// A single inbound message carried no valid signature of its author and
    /// was dropped (`SYNC-091`).
    SignatureRefused = 7,

    /// A single inbound message was sealed under a key this client does not
    /// hold, such as one a missed rotation brought, and was dropped
    /// (`SYNC-091`).
    KeyNotHeld = 8,

    /// A member signed a head of the membership log, and the log the relay
    /// serves does not pass through it. The relay withholds an entry from
    /// this client, such as a revoke and its rotation, or a member signed a
    /// head of no log (`SYNC-207`). The edits of this client wait on this
    /// device until a log that passes through the head is served
    /// (`SYNC-220`).
    LogEntryWithheld = 9,
    /// A single inbound delta or snapshot was signed by a key the verified
    /// membership log never granted a writing role, and was dropped
    /// (`SYNC-211`).
    WriterRefused = 10,
    /// A member's delta opened under a key a rotation retired. The relay
    /// numbered it after a delta under the current key. Its log head comes
    /// before the removal that rotation answered. Either the relay withholds
    /// that removal from the member, or the member has not read it yet. The
    /// member who left may read that delta (`SYNC-286`).
    WriterBehindLog = 11,
}
