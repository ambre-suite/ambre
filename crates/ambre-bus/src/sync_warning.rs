//! Closed set of `SyncWarning` codes, carried by both
//! [`crate::LogicEvent::SyncWarning`] and [`crate::UiEvent::SyncWarning`].
//!
//! A `SyncWarning` is always **non-fatal**: the Sync thread dropped a message or
//! relieved memory pressure and the session keeps running. The code is the
//! contract that lets the UI map an event to a user-facing notice without
//! parsing the free-text `message` (which stays a developer string). Every
//! warning the Sync thread can raise is enumerated here, in one crate — so the
//! consumer's `match` is exhaustive and the compiler flags any new variant.

/// Why the Sync thread raised a (non-fatal) warning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
}
