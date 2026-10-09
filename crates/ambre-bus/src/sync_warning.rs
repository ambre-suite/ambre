//! Closed set of `SyncWarning` codes, carried by both [`crate::LogicEvent::SyncWarning`]
//! and [`crate::UiEvent::SyncWarning`].
//!
//! A `SyncWarning` is always **non-fatal**: the session keeps running, and so does the
//! document it names. The code is the contract that lets the UI map an event to a
//! user-facing notice. It spares the UI from parsing the free-text `message`, which stays
//! a developer string. Every warning the Sync and Logic threads can raise is enumerated
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
}
