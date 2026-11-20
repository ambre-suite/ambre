//! `LogicEvent` - events sent from the Sync thread to the Logic thread.
//!
//! Two groups:
//! - [`LogicEvent::Rest`] -- a [`RestEvent`] service response answering an earlier
//!   [`SyncCommand::Rest`], echoing its [`Request`]. Forwarded verbatim to the
//!   UI (with a small Logic hook, for example `DocumentCreated` records the doc type).
//! - **Server push & errors** -- unsolicited notifications (CRDT deltas,
//!   connection status, member revocation, meeting participants...) and failures
//!   (`AuthError` / `ApiError` / `SetupRequired` / `UpdateRequired`). No originating request, so no
//!   request id; each is handled explicitly by the Logic thread (apply CRDT,
//!   mutate state, ...).
//!
//! [`SyncCommand::Rest`]: crate::SyncCommand::Rest

use serde::{Deserialize, Serialize};
use std::fmt;

use crate::sync_warning::SyncWarningCode;
use crate::{
    ChannelDocument, DocType, DocumentActivity, MemberInfo, Request, RestEvent, Role,
    SealedChannelKey,
};
use ambre_core::id::{
    Channel, Document, Id, Lease, Meeting, Message, Segment, Snapshot, Space, User,
};
use ambre_core::{ApiFailure, FixedBytes, Relay, Seq, SessionNonce};
use ambre_core::{Ms, TimeSpan};

// -----------------------------------------------------------------------------
// LogicEvent
// -----------------------------------------------------------------------------

/// Event sent from the Sync thread to the Logic thread. Crosses the Sync-worker
/// `postMessage` boundary, hence postcard-serialized (`Serialize`/`Deserialize`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LogicEvent {
    // -- Service plane (relayed request/response) ------------------------------
    /// A [`RestEvent`] service response echoing the originating [`Request`],
    /// or `None` for an answer no request asked for.
    Rest {
        id: Option<Seq<Request>>,
        evt: RestEvent,
    },

    // -- Calls (read by the UI thread on arrival, never by Logic) ----------------
    /// A signal of the call for the peer connection of the UI thread.
    Call(crate::CallSignal),

    // -- The call message of a channel (read by Logic) --------------------------
    /// The relay seated the starter of `call`: the Logic thread posts its call message in
    /// `channel` (the Murmure calls spec).
    CallStarted {
        call: Id<ambre_core::id::Call>,
        channel: Id<ambre_core::id::Channel>,
        /// The instant of the seat, on the clock of the worker: the time of the message.
        at: ambre_core::Instant<ambre_core::Authored>,
    },
    /// The relay counted this session's leave as the last of `call`: the Logic thread writes
    /// the end into its call message (the Murmure calls spec).
    CallEnded {
        call: Id<ambre_core::id::Call>,
        channel: Id<ambre_core::id::Channel>,
        /// The instant of the answer, on the clock of the worker: the time of the end.
        at: ambre_core::Instant<ambre_core::Authored>,
    },
    /// Who is in `call`, the answer to [`crate::SyncCommand::AskCallRoster`] (the Murmure calls spec).
    CallRoster {
        call: Id<ambre_core::id::Call>,
        channel: Id<ambre_core::id::Channel>,
        members: crate::CallMembers,
    },

    // -- Real-time sync (server push) ------------------------------------------
    /// The identity this session writes under in a document, sent once per open
    /// (`ambre-spec-op-authorship.md`). The Logic thread sets `peer` on the
    /// document, and writes `entry` into its peer registry with its first delta.
    /// It applies no mutating command to the document before this arrives.
    SessionIdentity {
        document_id: Id<Document>,
        /// The PeerID, derived from the signing key and the session nonce.
        peer: ambre_core::PeerId,
        /// The registry entry of `peer`, which the worker signed.
        entry: ambre_core::PeerEntry,
        /// The nonce `peer` derives from. The Logic thread keeps it in the
        /// local store, so a later open can send what this session left
        /// unsent (`AUTHOR-028`).
        nonce: FixedBytes<SessionNonce>,
    },

    /// Decrypted and decompressed CRDT delta, with its signer, ready for `LoroDoc::import()`.
    RemoteUpdate {
        document_id: Id<Document>,
        delta: SignedDelta,
    },

    /// A batch of decrypted CRDT deltas to import as one merge+reseed. Emitted by a server
    /// history replay (`WsServerMsg::MissingUpdates` on (re)open). The Logic thread imports
    /// them all before repainting. A cell that only appears in the last delta does not
    /// flicker in across N intermediate re-renders (one per delta). Order is preserved
    /// (server `seq` order).
    RemoteUpdateBatch {
        document_id: Id<Document>,
        deltas: Vec<SignedDelta>,
    },

    /// Decrypted and decompressed CRDT snapshot.
    RemoteSnapshot {
        document_id: Id<Document>,
        payload: ambre_core::Payload<ambre_core::SnapshotBytes>,
        /// The last delta `seq` it folds in, 0 when unknown. The Logic thread
        /// counts the deltas a later snapshot must hold from it (`AUTHOR-016`).
        covered_seq: Seq<Relay>,
    },

    /// A snapshot to check before anything trusts it
    /// (`ambre-spec-op-authorship.md`, `AUTHOR-016`, `AUTHOR-022`). The Logic
    /// thread compares it with the deltas it holds and answers with
    /// [`SyncCommand::SnapshotVerdict`](crate::SyncCommand::SnapshotVerdict).
    PendingSnapshot {
        document_id: Id<Document>,
        snapshot_id: Id<Snapshot>,
        /// The last delta `seq` it claims to fold in.
        covered_seq: Seq<Relay>,
        /// Decrypted and decompressed, as in `RemoteSnapshot`.
        payload: ambre_core::Payload<ambre_core::SnapshotBytes>,
        purpose: SnapshotPurpose,
    },

    /// A snapshot the relay dropped on a member's report. It names its author
    /// and its reporter, each `None` for a key no member holds.
    /// `after` says whether it was the final snapshot of a segment, which
    /// a member then finalizes again (the Murmure segments spec).
    SnapshotDropped {
        document_id: Id<Document>,
        author: Option<Id<User>>,
        reporter: Option<Id<User>>,
        after: ambre_core::AfterDrop,
    },

    /// A thread this client holds neither in full nor light took a delta
    /// (the Murmure threads spec). The signal names the thread alone: the
    /// Logic thread finds its channel and decides what to load.
    ThreadActivity { document_id: Id<Document> },

    /// Decrypted ephemeral data (cursor, collaborator presence). `from_user_id` is the
    /// sender's canonical `Id<User>`, resolved from the authenticated `pub_key` against the
    /// document's member list before delivery. Presence is therefore keyed by a spoof-proof
    /// per-user identity (`sync`, "Presence identity").
    RemoteEphemeral {
        document_id: Id<Document>,
        from_user_id: Id<User>,
        payload: ambre_core::Payload<ambre_core::EphemeralBytes>,
    },

    /// A peer stopped watching a document (`PRESENCE-014`): they unsubscribed, or their
    /// connection closed. `user_id` is resolved from the departing `pub_key` the relay
    /// named, by the same path an ephemeral takes. A departure therefore cannot be
    /// attributed to a peer who did not leave.
    ///
    /// Best-effort, like presence itself: a lost message leaves the peer to age out of the
    /// live set instead.
    PeerLeft {
        document_id: Id<Document>,
        user_id: Id<User>,
    },

    /// Sync read the members of an open document again, on the relay's word
    /// that they changed (`SRV-177`) or on a subscription (`SYNC-150`). The
    /// one answer that rebuilt the directory (`SYNC-037`) serves a view too,
    /// which asks for nothing more (`SYNC-151`).
    MembersChanged {
        document_id: Id<Document>,
        members: Vec<MemberInfo>,
    },

    /// The log of an open proved the pin of the document's invite link: the
    /// local store keeps it for the sessions after (`SYNC-238`).
    InviterProven {
        document_id: Id<Document>,
        inviter: ambre_core::FixedBytes<ambre_core::InviterPin>,
    },

    /// Initial hydration of a freshly opened document is complete. The Sync thread has
    /// fetched members and the latest snapshot (if any), and emitted it ahead of this
    /// event. Fires exactly once per open, for both the snapshot and the empty/brand-new
    /// paths, so the UI can dismiss its document-loading overlay. An empty document never
    /// produces a snapshot, so the overlay cannot key off `RemoteSnapshot`.
    DocumentReady {
        document_id: Id<Document>,
        /// This client's own role on the document, resolved by Sync from the
        /// members response (`architecture`). Logic keeps it beside
        /// the live `DocState` and drops mutating commands when it is not
        /// writable. The gate is UX only; the server remains the sole authority.
        role: Role,
    },

    /// WebSocket connection status for a document.
    ConnectionStatus {
        document_id: Id<Document>,
        connected: ambre_core::LinkState,
    },

    /// The activity counted for a document not held in memory, read from the
    /// summaries alone (`RESID-005`, `RESID-014`).
    Activity {
        document_id: Id<Document>,
        counters: ambre_core::activity::ActivityCounters,
    },

    /// Another member wrote a call cue into the summary of a document not
    /// held in memory (the Murmure calls spec, the ring). The Logic thread
    /// names its channel to the view.
    CallCue {
        document_id: Id<Document>,
        cue: crate::CallCue,
    },

    /// A released document turned into a light subscription: the relay
    /// acknowledged every frame this client sent of it (`RESID-009`). Its
    /// local copy holds nothing the relay lacks (`RESID-019`).
    Released { document_id: Id<Document> },

    /// Non-fatal warning, such as a quarantine overflow or a decrypt failure.
    SyncWarning {
        document_id: Id<Document>,
        code: SyncWarningCode,
    },

    /// The server requests a snapshot (too many accumulated deltas).
    /// The Logic thread must produce `LoroDoc::export_snapshot()` and respond
    /// with `SyncCommand::SendSnapshot { document_id, payload }`.
    SnapshotNeeded { document_id: Id<Document> },

    /// A member has been revoked and consolidation is complete.
    /// Translated from `WsServerMsg::KeyRotationRequired` by the Sync thread
    /// **after** the rotation is complete (new keys in DB). Server-initiated:
    /// the Logic thread may trigger a snapshot to seal the state.
    MemberRevoked {
        document_id: Id<Document>,
        revoked_user_id: Id<User>,
    },

    /// The relay no longer holds a document this client held: a delta to it
    /// was refused as not found. Sync dropped it. The Logic thread settles
    /// what the document stood for, such as amendments a fold erased.
    DocumentGone { document_id: Id<Document> },

    /// This client rotated the key of a document: a revocation or an owed
    /// rotation it paid. The Logic thread closes the active segment of the
    /// channel whose `meta` it is, so the successor opens under the new key.
    KeyRotated { document_id: Id<Document> },

    /// A document was shared with this user (server push on `user:{id}`,
    /// translated from `WsServerMsg::DocumentShared`). The Logic thread forwards
    /// it to the UI, which refreshes its document list and notifies the user.
    DocumentShared {
        document_id: Id<Document>,
        doc_type: DocType,
    },

    // -- Channels (Murmure) -----------------------------------------------------
    /// The documents a channel groups, answering
    /// [`SyncCommand::OpenChannel`](crate::SyncCommand::OpenChannel).
    ChannelDocuments {
        channel_id: Id<Channel>,
        documents: Vec<ChannelDocument>,
    },
    /// The relay refused the listing for another cause, or it could not be
    /// read.
    ChannelOpenFailed {
        channel_id: Id<Channel>,
        failure: ApiFailure,
    },
    /// The relay refused the listing for want of membership: the user is no
    /// member of the channel any more, or it is gone (`SRV-103`).
    ChannelRefused { channel_id: Id<Channel> },
    /// The relay's activity answer, per segment and thread of every channel
    /// of the user (`SRV-109`), answering [`SyncCommand::FetchActivity`] and
    /// [`SyncCommand::MarkAllRead`](crate::SyncCommand::MarkAllRead).
    ///
    /// [`SyncCommand::FetchActivity`]: crate::SyncCommand::FetchActivity
    ChannelActivity { documents: Vec<DocumentActivity> },
    /// The relay holds the new channel, answering
    /// [`SyncCommand::CreateChannel`](crate::SyncCommand::CreateChannel).
    /// `key` is the channel key sealed under the space key, for the catalogue
    /// of a public channel. Absent for a private one.
    ChannelCreated {
        channel_id: Id<Channel>,
        key: Option<SealedChannelKey>,
    },
    /// The relay closed `closing`, answering
    /// [`SyncCommand::CloseSegment`](crate::SyncCommand::CloseSegment).
    /// `active` is the successor the relay registered. `successor` says
    /// whether it is the one this client asked for. A close that lost its race
    /// names the winner's, and the Sync worker deleted its own.
    SegmentClosed {
        channel_id: Id<Channel>,
        closing: Id<Segment>,
        active: Id<Segment>,
        successor: Successor,
    },
    /// The relay refused the close of `closing`, or never answered it.
    SegmentCloseFailed {
        channel_id: Id<Channel>,
        closing: Id<Segment>,
        failure: ApiFailure,
    },
    /// The relay registered `amendments` for the locked `segment`, answering
    /// [`SyncCommand::CreateAmendments`](crate::SyncCommand::CreateAmendments).
    /// A registration that lost its race names the winner's, and the Sync
    /// worker deleted its own (`SRV-202`).
    AmendmentsLinked {
        channel_id: Id<Channel>,
        segment: Id<Segment>,
        amendments: Id<Document>,
    },
    /// The relay refused the creation or the registration of `amendments`
    /// for `segment`, or never answered it.
    AmendmentsLinkFailed {
        channel_id: Id<Channel>,
        segment: Id<Segment>,
        amendments: Id<Document>,
        failure: ApiFailure,
    },
    /// The relay registered `thread` for `root`, a message of `segment`,
    /// answering [`SyncCommand::CreateThread`](crate::SyncCommand::CreateThread).
    /// A registration that lost its race names the winner's, and the Sync
    /// worker deleted its own (`SRV-214`).
    ThreadLinked {
        channel_id: Id<Channel>,
        segment: Id<Segment>,
        root: Id<Message>,
        thread: Id<Document>,
    },
    /// The relay refused the creation or the registration of `thread` for
    /// `root`, or never answered it.
    ThreadLinkFailed {
        channel_id: Id<Channel>,
        segment: Id<Segment>,
        root: Id<Message>,
        thread: Id<Document>,
        failure: ApiFailure,
    },
    /// The relay's answer to
    /// [`SyncCommand::TakeFinalizeLease`](crate::SyncCommand::TakeFinalizeLease).
    FinalizeLeaseAnswered {
        channel_id: Id<Channel>,
        segment: Id<Segment>,
        answer: LeaseAnswer,
    },
    /// The relay accepted the final snapshot of `segment`, answering
    /// [`SyncCommand::FinalizeSegment`](crate::SyncCommand::FinalizeSegment).
    SegmentFinalized {
        channel_id: Id<Channel>,
        segment: Id<Segment>,
    },
    /// A lease request or a commit of `segment` failed. `end` says whether
    /// this copy holds an edit the relay refused, so it never finalizes the
    /// segment: another member does.
    SegmentFinalizeFailed {
        channel_id: Id<Channel>,
        segment: Id<Segment>,
        end: ambre_core::FinalizeEnd,
        failure: ApiFailure,
    },
    /// The relay's answer to
    /// [`SyncCommand::TakeFoldLease`](crate::SyncCommand::TakeFoldLease).
    FoldLeaseAnswered {
        channel_id: Id<Channel>,
        segment: Id<Segment>,
        answer: FoldAnswer,
    },
    /// The relay took the fold snapshot of `segment`, answering
    /// [`SyncCommand::FoldSegment`](crate::SyncCommand::FoldSegment).
    SegmentFolded {
        channel_id: Id<Channel>,
        segment: Id<Segment>,
    },
    /// A fold lease request or a fold commit of `segment` failed.
    SegmentFoldFailed {
        channel_id: Id<Channel>,
        segment: Id<Segment>,
        failure: ApiFailure,
    },
    /// This client's confirmation made a fold snapshot of the segment
    /// `document_id` final, and the relay erased `amendments`, which it
    /// folds in whole (`SRV-211`).
    AmendmentsFolded {
        document_id: Id<Document>,
        amendments: Id<Document>,
    },
    /// The relay refused deltas of `document_id` for its lock. `deltas` are
    /// the Loro updates this client sent from the first one refused, in the
    /// order they left. None of them reached another member, and none leaves
    /// again: the new messages they carry go to the active segment.
    DeltasLocked {
        document_id: Id<Document>,
        deltas: Vec<ambre_core::Payload<ambre_core::DeltaBytes>>,
    },
    /// A public channel of the space has a new key, sealed under the current
    /// space key, answering
    /// [`SyncCommand::RotateSpace`](crate::SyncCommand::RotateSpace). The
    /// catalogue entry takes it in place of the old one.
    ChannelKeyResealed {
        space_id: Id<Space>,
        channel_id: Id<Channel>,
        key: SealedChannelKey,
    },
    /// The user is a member of the channel, answering
    /// [`SyncCommand::JoinChannel`](crate::SyncCommand::JoinChannel).
    ChannelJoined { channel_id: Id<Channel> },
    /// The relay holds the direct message of the requested members,
    /// answering
    /// [`SyncCommand::CreateDirectMessage`](crate::SyncCommand::CreateDirectMessage).
    /// `channel_id` is the requested one when the relay created it, and the
    /// existing one's otherwise.
    DirectMessageOpened {
        requested: Id<Channel>,
        channel_id: Id<Channel>,
    },
    /// The relay deleted the channel, answering
    /// [`SyncCommand::DeleteChannel`](crate::SyncCommand::DeleteChannel).
    ChannelDeleted { channel_id: Id<Channel> },
    /// The relay removed this user from the channel, answering
    /// [`SyncCommand::LeaveChannel`](crate::SyncCommand::LeaveChannel).
    ChannelLeft { channel_id: Id<Channel> },
    /// A creation, a join, a delete or a leave failed, and the relay changed
    /// nothing.
    ChannelRequestFailed {
        channel_id: Id<Channel>,
        failure: ApiFailure,
    },
    /// The channels the relay holds as public in the space, answering
    /// [`SyncCommand::ListSpaceChannels`](crate::SyncCommand::ListSpaceChannels).
    SpaceChannels {
        space_id: Id<Space>,
        channels: Vec<Id<Channel>>,
    },
    /// Every channel and every space this user is a member of, answering
    /// [`SyncCommand::ListMemberChannels`](crate::SyncCommand::ListMemberChannels).
    /// A space lists here with no channel of the user in it. `names` holds
    /// each name the Sync worker opened beside a `meta`, and `peers` the
    /// other members of each direct message, by display name. `metas`
    /// names the `meta` of each channel whose key the worker holds.
    MemberChannels {
        channels: Vec<crate::MemberChannel>,
        spaces: Vec<Id<Space>>,
        names: Vec<crate::ChannelName>,
        peers: Vec<crate::DirectPeers>,
        metas: Vec<crate::ChannelMeta>,
    },

    // -- Meetings (Rendez-Vous) -- participant push -----------------------------
    /// Another participant joined the meeting.
    MeetingParticipantJoined {
        meeting_id: Id<Meeting>,
        user_id: Id<User>,
    },
    /// Another participant left the meeting.
    MeetingParticipantLeft {
        meeting_id: Id<Meeting>,
        user_id: Id<User>,
    },

    // -- Errors ----------------------------------------------------------------
    /// A document could not open: its wrapped key did not unwrap, or the
    /// session was not ready. Nothing of it is held. The Logic thread ends
    /// the channel or space opening that asked for it, or hands the failed open
    /// to the UI.
    DocumentOpenFailed {
        document_id: Id<Document>,
        cause: OpenFailure,
    },
    /// Authentication error: an incorrect passphrase, corrupted data or a
    /// failed request.
    AuthError { cause: AuthFailure },

    /// Generic error for any REST/WS operation. `request` identifies the source command
    /// (for example, [`ApiRequest::ListDocuments`]), and `failure` says whether the relay
    /// answered and with what.
    ///
    /// `id` echoes the originating [`Request`] when the failed command carried one (a
    /// `SyncCommand::Rest`). The dispatcher can then route a correlated failure back to
    /// the issuing document. In `toile-media`, a failed media upload must reach the
    /// pending import waiting on it. `None` for a spontaneous error with no originating
    /// request.
    ApiError {
        id: Option<Seq<Request>>,
        request: ApiRequest,
        failure: ApiFailure,
    },

    /// A revocation went through, and the new key of `document_id` could not
    /// be wrapped for `members`: their exchange keys do not read. They lost
    /// the document, and the user who revoked reads who (`SYNC-043`).
    KeyNotShared {
        document_id: Id<Document>,
        members: Vec<ambre_core::Validated<ambre_core::DisplayName>>,
    },

    /// The relay refused a request or the socket because the account still owes
    /// its key setup. Stands in for the generic error such a refusal would
    /// otherwise raise (`ambre-spec-sync.md`, `SYNC-134`).
    SetupRequired,

    /// The relay speaks a newer wire version than this build. Only an update
    /// cures it, so the socket stays closed (`ambre-spec-sync.md`, `SYNC-137`).
    UpdateRequired,

    // -- The converter modules -------------------------------------------------
    /// The converter module of `family`, fetched and checked against the
    /// build's digest of that family (`ARCH-080`). Answers
    /// [`crate::SyncCommand::FetchConverter`].
    ConverterFetched {
        family: ambre_core::ConverterFamily,
        module: ambre_core::Payload<ambre_core::ConverterModule>,
    },
    /// The converter module of `family` did not arrive, or its bytes are not
    /// the ones the build names. Answers [`crate::SyncCommand::FetchConverter`].
    ConverterRefused {
        family: ambre_core::ConverterFamily,
        cause: ambre_core::ConverterRefusal,
    },
}

/// Why a sign-in, an unlock or a key setup failed (`BUS-046`).
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthFailure {
    /// The passphrase does not open the account data, or the data is
    /// corrupted: the two read the same.
    BadPassphrase,
    /// The account data is corrupted.
    CorruptAccount,
    /// The session ended before the unlock finished.
    SessionEnded,
    /// The master seed did not wrap for storage.
    SeedNotWrapped,
    /// A request of the step failed.
    Api(ApiFailure),
    /// The new passphrase falls under the strength floor of an offline
    /// guess, for the reason it names (`ZK-036`). No request left the worker.
    WeakPassphrase(ambre_core::PassphraseWeakness),
    /// The recovery words do not spell a seed, for the reason it names
    /// (`ZK-043`). No key was derived and no request left the worker.
    BadRecoveryWords(ambre_core::RecoveryWordsRefusal),
    /// The recovery words spell a seed, but not the seed of this account: its
    /// signing key is not the one the relay holds. Nothing was written
    /// (`SYNC-249`).
    ForeignRecoveryWords,
    /// This build holds no development sign-in, so a sign-in by a bare name
    /// derived no key and sent nothing (`ZK-045`).
    NoDevSignIn,
    /// The relay no longer holds the sealed seed a recovery or a passphrase
    /// change read: another device re-sealed it in between. Nothing was
    /// written, and a new try reads the seal again (`SYNC-286`).
    StaleReseal,
}

ambre_core::define_flag! {
    /// Whether a failure refuses the passphrase the person typed.
    pub enum PassphraseVerdict {
        /// Another step failed.
        Other,
        /// A wrong passphrase, or one under the strength floor.
        Refused,
    }
}

impl AuthFailure {
    /// Whether the failure refuses the passphrase the person typed: a wrong
    /// one, or one under the strength floor. The view raises a visual toast for
    /// it beside the line of the face that asked (`CHROME-136`).
    pub fn passphrase_verdict(self) -> PassphraseVerdict {
        match self {
            Self::BadPassphrase | Self::WeakPassphrase(_) => PassphraseVerdict::Refused,
            Self::CorruptAccount
            | Self::SessionEnded
            | Self::SeedNotWrapped
            | Self::Api(_)
            | Self::BadRecoveryWords(_)
            | Self::ForeignRecoveryWords
            | Self::NoDevSignIn
            | Self::StaleReseal => PassphraseVerdict::Other,
        }
    }
}

impl fmt::Display for AuthFailure {
    /// The words a person reads for the failure.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadPassphrase => {
                f.write_str("Unlock failed: incorrect passphrase or corrupted account data")
            }
            Self::CorruptAccount => f.write_str("Unlock failed: account data is corrupted"),
            Self::SessionEnded => f.write_str("Unlock failed: the session ended"),
            Self::SeedNotWrapped => f.write_str("Sign-in failed: the account key did not seal"),
            Self::Api(failure) => write!(f, "Sign-in failed: {failure}"),
            Self::WeakPassphrase(weakness) => f.write_str(weakness.message()),
            Self::BadRecoveryWords(refusal) => {
                f.write_str("Recovery failed: ")?;
                f.write_str(match refusal {
                    ambre_core::RecoveryWordsRefusal::WrongWordCount => {
                        "the recovery phrase holds 24 words."
                    }
                    ambre_core::RecoveryWordsRefusal::UnknownWord => {
                        "a word is not one of the recovery words. Check its spelling."
                    }
                    ambre_core::RecoveryWordsRefusal::BadChecksum => {
                        "the words do not match each other. Check each word and its order."
                    }
                })
            }
            Self::ForeignRecoveryWords => {
                f.write_str("Recovery failed: these recovery words belong to another account.")
            }
            Self::NoDevSignIn => {
                f.write_str("Sign-in failed: this build holds no development sign-in.")
            }
            Self::StaleReseal => f.write_str(
                "Nothing changed: the account keys changed on another device while this ran. \
                 Try again.",
            ),
        }
    }
}

/// The debug form is the words of [`fmt::Display`]: one table of words, which
/// the Main instance carries once (`ambre-spec-budgets.md section 5`).
impl fmt::Debug for AuthFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Why a document could not open (`SYNC-173`).
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpenFailure {
    /// The key of the document has not reached this device. The wrap for
    /// this user does not open, or the chain does not reach the named key.
    KeyNotReceived,
    /// The relay could not be reached, the session was not ready, or the
    /// replay stopped. A later open may succeed.
    Transient,
    /// The relay answered the open with a refusal of the request itself, a
    /// `4xx`. `code` is the refusal code its body named, if any. Asking again
    /// meets the same answer.
    Refused {
        code: Option<ambre_core::RefusalCode>,
    },
    /// The key the relay served for this device is one no member of the
    /// document's log signed. The relay chose the key, or the log it served,
    /// so a later open meets the same key. This is a fault of the relay,
    /// never a wait (`SYNC-212`).
    KeyUnsigned,
    /// The log of the channel this document belongs to starts at no writer
    /// of the space whose catalogue lists it (audit `N16`). No member of the
    /// space vouches for its key or its deltas, so the channel ends.
    RootOutsideSpace,
}

impl fmt::Display for OpenFailure {
    /// The words a person reads for the failed open.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::KeyNotReceived => {
                f.write_str("the key of the document has not reached this device")
            }
            Self::Transient => f.write_str("the relay could not be reached; try again"),
            Self::Refused { code: Some(code) } => write!(f, "the relay refused the open ({code})"),
            Self::Refused { code: None } => f.write_str("the relay refused the open"),
            Self::KeyUnsigned => {
                f.write_str("the relay served a key no member signed; the relay may be compromised")
            }
            Self::RootOutsideSpace => f.write_str("no member of the space created this channel"),
        }
    }
}

/// The debug form is the words of [`fmt::Display`]: one table of words, which
/// the Main instance carries once (`ambre-spec-budgets.md section 5`).
impl fmt::Debug for OpenFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// The relay's answer to a finalize lease request (the Murmure segments
/// spec).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LeaseAnswer {
    /// This client holds the lease for `expires_in` milliseconds. Its
    /// snapshot covers the deltas up to `horizon`.
    Granted {
        lease: Id<Lease>,
        horizon: Seq<Relay>,
        expires_in: TimeSpan<Ms>,
    },
    /// No lease yet: ask again in `retry_in` milliseconds. The grace period
    /// runs, or another member holds the lease.
    Deferred { retry_in: TimeSpan<Ms> },
}

/// The relay's answer to a fold lease request (`SRV-204`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FoldAnswer {
    /// This client holds the lease for `expires_in` milliseconds. Its
    /// snapshot covers the deltas of the segment up to `horizon`, and folds
    /// `amendments` up to at least `amendments_seq`.
    Granted {
        lease: Id<Lease>,
        horizon: Seq<Relay>,
        amendments: Id<Document>,
        amendments_seq: Seq<Relay>,
        expires_in: TimeSpan<Ms>,
    },
    /// No lease yet: ask again in `retry_in` milliseconds. The fold delay
    /// runs, or another member holds the lease.
    Deferred { retry_in: TimeSpan<Ms> },
}

/// Why a snapshot is checked (`ambre-spec-op-authorship.md` section 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SnapshotPurpose {
    /// A pending snapshot, which a match confirms (`AUTHOR-016`). Every delta
    /// it covers must be held, or the check proves nothing.
    Confirm,
    /// A snapshot only its author confirmed, read on open (`AUTHOR-022`). The
    /// relay trimmed that author's deltas, so gaps in the seqs are expected.
    Open,
}

/// What the Logic thread found when it checked a snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SnapshotVerdict {
    /// Every operation both sides hold is the same. `extras` names each PeerID
    /// only the snapshot holds, with its registry entry. The snapshot passes
    /// only if each entry resolves to its author (`AUTHOR-021`).
    Match {
        extras: Vec<(
            ambre_core::PeerId,
            ambre_core::Payload<ambre_core::PeerEntryBytes>,
        )>,
    },
    /// The snapshot contradicts a delta it covers (`AUTHOR-017`).
    Mismatch,
    /// The client lacks what a verdict needs, and says nothing.
    Unchecked,
}

/// One opened update and what its signature proves about it
/// (`ambre-spec-sealed-envelope.md`, `SYNC-114`).
///
/// The Sync worker derives `peer` from the signer key and `nonce`. The Logic
/// thread refuses the delta when one of its operations carries another PeerID
/// (`ambre-spec-op-authorship.md`, `AUTHOR-012`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedDelta {
    /// The number the relay gave it. The Logic thread's snapshot check reads
    /// which deltas a horizon covers from it (`AUTHOR-016`).
    pub seq: Seq<Relay>,
    /// The Loro update, decrypted and decompressed.
    pub payload: ambre_core::Payload<ambre_core::DeltaBytes>,
    /// The session nonce of its author, opened from inside the ciphertext.
    pub nonce: FixedBytes<SessionNonce>,
    /// The only PeerID its operations may carry.
    pub peer: ambre_core::PeerId,
    /// The member the signing key resolves to, `None` when the directory does
    /// not name it (`AUTHOR-013`).
    pub user: Option<Id<User>>,
    /// What the update says of itself, when its author wrote a summary
    /// (`ambre-spec-sealed-envelope.md`, `SYNC-144`).
    pub summary: Option<crate::ActivitySummary>,
}

/// The request an [`LogicEvent::ApiError`] or a [`crate::UiEvent::ApiError`]
/// reports the failure of. A closed set, so a consumer matches the request
/// and never its spelling. [`fmt::Display`] writes the name a log reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ApiRequest {
    /// Creating a space.
    CreateSpace,
    /// Adding a category to a space.
    AddCategory,
    /// Creating a public channel.
    CreatePublicChannel,
    /// Creating a private channel.
    CreatePrivateChannel,
    /// Renaming a channel.
    NameChannel,
    /// Inviting a member to a document.
    InviteToDocument,
    /// Revoking a member of a document.
    RevokeFromDocument,
    /// Leaving a document.
    LeaveDocument,
    /// Changing the role of a member of a document.
    UpdateDocumentMemberRole,
    /// Fetching the keys of the user an invitation names.
    FetchInviteeKey,
    /// Fetching the keys of a member.
    FetchMemberKey,
    /// Listing the user's documents.
    ListDocuments,
    /// Listing the members of a document.
    ListDocumentMembers,
    /// Uploading a media blob.
    UploadMedia,
    /// Downloading a media blob.
    DownloadMedia,
    /// Erasing a media blob.
    EraseMedia,
    /// The socket, for a refusal it carried.
    Websocket,
    /// A request of the service plane.
    Rest,
}

impl ApiRequest {
    /// The name a log and a toast read.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CreateSpace => "CreateSpace",
            Self::AddCategory => "AddCategory",
            Self::CreatePublicChannel => "CreatePublicChannel",
            Self::CreatePrivateChannel => "CreatePrivateChannel",
            Self::NameChannel => "NameChannel",
            Self::InviteToDocument => "InviteToDocument",
            Self::RevokeFromDocument => "RevokeFromDocument",
            Self::LeaveDocument => "LeaveDocument",
            Self::UpdateDocumentMemberRole => "UpdateDocumentMemberRole",
            Self::FetchInviteeKey => "FetchInviteeKey",
            Self::FetchMemberKey => "FetchMemberKey",
            Self::ListDocuments => "ListDocuments",
            Self::ListDocumentMembers => "ListDocumentMembers",
            Self::UploadMedia => "UploadMedia",
            Self::DownloadMedia => "DownloadMedia",
            Self::EraseMedia => "EraseMedia",
            Self::Websocket => "websocket",
            Self::Rest => "rest",
        }
    }
}

impl fmt::Display for ApiRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Who registered the segment a close moves to.
///
/// The variant that was `false`, when the relay's answer said whether this
/// client created it, comes first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Successor {
    /// Another member won the race: the successor is fetched as they wrote
    /// it.
    Fetched,
    /// This client registered it: the channel holds it at once.
    Created,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `CHROME-136`: a wrong passphrase and each weakness refuse the
    /// passphrase. No other failure does, so no toast doubles its face.
    #[test]
    fn only_a_wrong_or_weak_passphrase_refuses_the_passphrase() {
        use ambre_core::PassphraseWeakness as Weak;
        let refusing = [
            AuthFailure::BadPassphrase,
            AuthFailure::WeakPassphrase(Weak::CommonPassword),
            AuthFailure::WeakPassphrase(Weak::Repetition),
            AuthFailure::WeakPassphrase(Weak::Sequence),
            AuthFailure::WeakPassphrase(Weak::TooShort),
        ];
        assert!(
            refusing
                .iter()
                .all(|cause| cause.passphrase_verdict() == PassphraseVerdict::Refused)
        );
        let others = [
            AuthFailure::CorruptAccount,
            AuthFailure::SessionEnded,
            AuthFailure::SeedNotWrapped,
            AuthFailure::Api(ApiFailure::Local(ambre_core::LocalFailure::NoRelay)),
            AuthFailure::BadRecoveryWords(ambre_core::RecoveryWordsRefusal::BadChecksum),
            AuthFailure::ForeignRecoveryWords,
        ];
        assert!(
            others
                .iter()
                .all(|cause| cause.passphrase_verdict() == PassphraseVerdict::Other)
        );
    }
}
