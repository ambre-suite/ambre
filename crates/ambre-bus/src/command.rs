//! `SyncCommand` - commands sent from the Logic thread to the Sync thread.
//!
//! Three groups:
//! - [`SyncCommand::Rest`] -- the correlatable service family ([`RestCommand`]),
//!   forwarded verbatim from [`LogicCommand::Rest`]. The Sync thread performs the
//!   REST / crypto work and answers with [`LogicEvent::Rest`].
//! - **Control & content-send** -- document/channel session control and CRDT /
//!   ephemeral byte payloads (`Vec<u8>` = genuine network bytes the Sync thread
//!   encrypts/compresses/transmits).
//! - **Flow control** -- `HoldInbound` and `ReleaseInbound`, which the Main
//!   relay posts when the path back to the Logic thread fills and drains.
//! - **Lifecycle** -- `Shutdown`.
//!
//! Crosses a `postMessage` boundary from the Main instance to the separate Sync
//! worker, so it is postcard-serialized on that hop (hence `Serialize`/
//! `Deserialize`). The `Vec<u8>` payloads are genuine network bytes; large ones
//! ride a Transferable `ArrayBuffer` to stay zero-copy.
//!
//! [`LogicCommand::Rest`]: crate::LogicCommand::Rest
//! [`LogicEvent::Rest`]: crate::LogicEvent::Rest

use ambre_core::{Generation, SessionNonce};
use serde::{Deserialize, Serialize};

use ambre_core::id::{Channel, Document, Id, Lease, Message, Meta, Segment, Snapshot, Space};

use crate::{
    ActivitySummary, DirectMember, DocType, Request, RestCommand, SealedChannelKey, Seq,
    SnapshotPurpose, SnapshotVerdict,
};

/// Command sent from the Logic thread to the Sync thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SyncCommand {
    // -- Service plane (relayed request/response) ------------------------------
    /// A [`RestCommand`] service operation forwarded from the Logic thread. The
    /// Sync thread answers with [`crate::LogicEvent::Rest`] echoing `id`.
    Rest {
        id: Generation<Request>,
        cmd: RestCommand,
    },

    // -- Document sync session (control) ---------------------------------------
    /// Open a sync session for a document.
    ///
    /// The Sync thread fetches members + `doc_key_enc`, decrypts the DocumentKey,
    /// loads snapshot + deltas, opens a WebSocket subscription.
    OpenDocument {
        document_id: Id<Document>,
        /// Document kind, used to lazily create the document server-side (with the
        /// right type) on its first modification if it does not yet exist.
        doc_type: DocType,
    },

    /// Close the sync session. Unsubscribe, zeroize the DocumentKey.
    CloseDocument {
        document_id: Id<Document>,
    },

    /// The Logic thread evicted a document from memory (`RESID-006`). Sync
    /// keeps a light subscription in place of the full session, which counts
    /// its activity and resumes a later open after `applied_seq`
    /// (`RESID-004`, `RESID-010`).
    ReleaseDocument {
        document_id: Id<Document>,
        /// The highest relay number the evicted copy holds.
        applied_seq: Seq,
    },

    /// Raw CRDT delta produced by `LoroDoc::export_updates()`.
    SendUpdate {
        document_id: Id<Document>,
        payload: ambre_core::DeltaBytes,
        /// What the delta holds, when the application wrote a summary
        /// (`ambre-spec-residency.md`). A Murmure message carries one
        /// (the Murmure messages spec). The Sync worker seals it inside the ciphertext
        /// (`SYNC-113`), or drops it while that seal is not built.
        summary: Option<ActivitySummary>,
    },

    /// The operations an earlier session of this user wrote and never sent.
    /// They are sealed with that session's nonce, so a reader derives the
    /// PeerID that wrote them (`AUTHOR-028`).
    SendUnsent {
        document_id: Id<Document>,
        payload: ambre_core::DeltaBytes,
        nonce: SessionNonce,
    },

    /// Full CRDT state for compaction (presigned URL -> S3).
    SendSnapshot {
        document_id: Id<Document>,
        payload: ambre_core::SnapshotBytes,
        /// The last delta `seq` the Logic thread holds with no gap below it.
        /// The snapshot claims to fold in every delta up to it (`SRV-083`).
        covered_seq: Seq,
    },

    /// The Logic thread's verdict on a [`LogicEvent::PendingSnapshot`]
    /// (`ambre-spec-op-authorship.md`, `AUTHOR-016`, `AUTHOR-017`).
    ///
    /// [`LogicEvent::PendingSnapshot`]: crate::LogicEvent::PendingSnapshot
    SnapshotVerdict {
        document_id: Id<Document>,
        snapshot_id: Id<Snapshot>,
        purpose: SnapshotPurpose,
        verdict: SnapshotVerdict,
    },

    /// Cursor / presence data (encrypted by the Sync thread before sending).
    SendEphemeral {
        document_id: Id<Document>,
        payload: ambre_core::EphemeralBytes,
    },

    /// Reconnect the WebSocket after a disconnect.
    Reconnect {
        document_id: Id<Document>,
    },

    // -- Channel (control) -----------------------------------------------------
    /// List the documents a channel groups. Answered by
    /// [`crate::LogicEvent::ChannelDocuments`], by
    /// [`crate::LogicEvent::ChannelRefused`], or by
    /// [`crate::LogicEvent::ChannelOpenFailed`]. Each document then opens
    /// through [`SyncCommand::OpenDocument`]: a channel has no session of its
    /// own (`SRV-102`).
    OpenChannel {
        channel_id: Id<Channel>,
        /// The open space whose catalogue lists the channel, if one does. Its
        /// log then admits a self-join (`SYNC-106`), and the log of the space
        /// verifies its removals and its key links. The Logic thread knows
        /// the catalogues; the Sync worker does not.
        listed_in: Option<Id<Space>>,
    },
    /// Rotate what a leave left owed in an open space. The space key goes
    /// first, when the relay says the space owes it. Then each public
    /// channel whose sealed key predates the current space key gets a new
    /// key. `channels` is the catalogue: each entry's channel and its sealed
    /// key. Each new key is answered by
    /// [`crate::LogicEvent::ChannelKeyResealed`]; a failure is logged, and
    /// the next open of the space tries again.
    RotateSpace {
        space_id: Id<Space>,
        channels: Vec<(Id<Channel>, SealedChannelKey)>,
    },
    /// Create a channel of a space on the relay: one channel key, its `meta`
    /// and first segment documents under it, and the channel record.
    /// Answered by [`crate::LogicEvent::ChannelCreated`] or by
    /// [`crate::LogicEvent::ChannelRequestFailed`]. A public channel's answer
    /// carries the key sealed under the key of the space, for the catalogue.
    /// Its space document must then have an open session, which holds the
    /// space key. A private channel is listed nowhere, and gets no sealed key.
    ///
    /// A private channel's `name` is sealed beside `meta`, so it shows before
    /// the channel opens. A public channel's name is in the catalogue, so it
    /// has none.
    CreateChannel {
        channel_id: Id<Channel>,
        space_id: Id<Space>,
        meta: Id<Meta>,
        segment: Id<Segment>,
        visibility: ambre_core::Visibility,
        name: Option<ambre_core::Name>,
    },
    /// Seal the name just written into the `meta` of an open channel beside
    /// it on the relay, under the channel key. `None` seals an empty name: the
    /// channel is unnamed. A failure is logged, and the next name replaces it.
    NameChannel {
        meta: Id<Meta>,
        name: Option<ambre_core::Name>,
    },
    /// Close the active segment `closing` of an open channel into
    /// `successor` (the Murmure segments spec). The Sync worker creates the
    /// successor on the relay under the current key of `meta`, then asks the
    /// relay to close. Answered by [`crate::LogicEvent::SegmentClosed`] or
    /// by [`crate::LogicEvent::SegmentCloseFailed`].
    CloseSegment {
        channel_id: Id<Channel>,
        meta: Id<Meta>,
        closing: Id<Segment>,
        successor: Id<Segment>,
    },
    /// Create `amendments` on the relay under the current key of `meta`,
    /// then register it as the amendments of the locked `segment`
    /// (`SRV-201`). Answered by [`crate::LogicEvent::AmendmentsLinked`] or
    /// by [`crate::LogicEvent::AmendmentsLinkFailed`].
    CreateAmendments {
        channel_id: Id<Channel>,
        meta: Id<Meta>,
        segment: Id<Segment>,
        amendments: Id<Document>,
    },
    /// Create `thread` on the relay under the current key of `meta`, then
    /// register it as the thread of `root`, a message of `segment`
    /// (`SRV-213`). Answered by [`crate::LogicEvent::ThreadLinked`] or by
    /// [`crate::LogicEvent::ThreadLinkFailed`].
    CreateThread {
        channel_id: Id<Channel>,
        meta: Id<Meta>,
        segment: Id<Segment>,
        root: Id<Message>,
        thread: Id<Document>,
    },
    /// Ask the relay for the finalize lease of the closing segment `segment`
    /// (the Murmure segments spec). Answered by
    /// [`crate::LogicEvent::FinalizeLeaseAnswered`] or by
    /// [`crate::LogicEvent::SegmentFinalizeFailed`].
    TakeFinalizeLease {
        channel_id: Id<Channel>,
        segment: Id<Segment>,
    },
    /// Upload `payload`, the final snapshot of `segment` up to `horizon`,
    /// then commit it under `lease`. Answered by
    /// [`crate::LogicEvent::SegmentFinalized`] or by
    /// [`crate::LogicEvent::SegmentFinalizeFailed`].
    FinalizeSegment {
        channel_id: Id<Channel>,
        segment: Id<Segment>,
        lease: Id<Lease>,
        horizon: Seq,
        payload: ambre_core::SnapshotBytes,
    },
    /// Ask the relay for the lease that folds the amendments of the final
    /// segment `segment` into it (`SRV-204`). Answered by
    /// [`crate::LogicEvent::FoldLeaseAnswered`] or by
    /// [`crate::LogicEvent::SegmentFoldFailed`].
    TakeFoldLease {
        channel_id: Id<Channel>,
        segment: Id<Segment>,
    },
    /// Upload `payload`, the fold snapshot of `segment` up to `horizon`,
    /// then commit it under `lease` as reaching the delta `amendments_seq`
    /// of its amendments (`SRV-207`). Answered by
    /// [`crate::LogicEvent::SegmentFolded`] or by
    /// [`crate::LogicEvent::SegmentFoldFailed`].
    FoldSegment {
        channel_id: Id<Channel>,
        segment: Id<Segment>,
        lease: Id<Lease>,
        horizon: Seq,
        amendments_seq: Seq,
        payload: ambre_core::SnapshotBytes,
    },
    /// Count a segment of a closed channel as a light subscription, from its
    /// first number. It opens under the key of the light subscription of
    /// `meta`, as every document of the channel does.
    WatchSegment {
        segment: Id<Segment>,
        meta: Id<Meta>,
    },
    /// Count a thread whose root is not on screen as a light subscription,
    /// from the number after `after` (the Murmure threads spec). It opens
    /// under the key of `meta`, and no document is loaded.
    WatchThread {
        thread: Id<Document>,
        meta: Id<Meta>,
        after: Seq,
    },
    /// Count a channel this session never opened, as light subscriptions
    /// (the Murmure notifications spec). Each of `documents` counts after
    /// the last number beside it, the activity answer's, and from the next
    /// delta without one. Its `meta` counts after `meta_last`, the listing's,
    /// the same way. They open under the key of `meta` the listing wrapped,
    /// and an open fetches them whole.
    WatchChannel {
        channel_id: Id<Channel>,
        meta: Id<Meta>,
        meta_last: Option<Seq>,
        documents: Vec<(Id<Document>, Option<Seq>)>,
    },
    /// Join a public channel alone. The Sync worker unseals `key` with the
    /// space key, wraps it for this user alone, and asks for the member row.
    /// Answered by [`crate::LogicEvent::ChannelJoined`] or
    /// [`crate::LogicEvent::ChannelRequestFailed`].
    JoinChannel {
        channel_id: Id<Channel>,
        space_id: Id<Space>,
        key: SealedChannelKey,
    },
    /// Delete a channel with every document it groups (`SRV-142`). Answered
    /// by [`crate::LogicEvent::ChannelDeleted`] or by
    /// [`crate::LogicEvent::ChannelRequestFailed`].
    DeleteChannel {
        channel_id: Id<Channel>,
    },
    /// Remove this user from a channel: a signed `Revoke` of themselves on
    /// its `meta`, posted with no rotation (`SYNC-127`). Answered by
    /// [`crate::LogicEvent::ChannelLeft`] or by
    /// [`crate::LogicEvent::ChannelRequestFailed`].
    LeaveChannel {
        channel_id: Id<Channel>,
    },
    /// Open or create the direct message of this user and `members`, in one
    /// request. It carries one channel key wrapped for each member alone, and
    /// a log in which nobody stays owner. Answered by
    /// [`crate::LogicEvent::DirectMessageOpened`] or
    /// [`crate::LogicEvent::ChannelRequestFailed`] under `channel_id`.
    CreateDirectMessage {
        channel_id: Id<Channel>,
        meta: Id<Meta>,
        segment: Id<Segment>,
        members: Vec<DirectMember>,
    },
    /// List the channels the relay holds as public in a space (`SRV-189`).
    /// Answered by [`crate::LogicEvent::SpaceChannels`]; a failure is logged
    /// and answers nothing, since the next open of the space asks again.
    ListSpaceChannels {
        space_id: Id<Space>,
    },
    /// List every channel this user is a member of, private channels and
    /// direct messages included (`SRV-191`). Answered by
    /// [`crate::LogicEvent::MemberChannels`]; a failure is logged and answers
    /// nothing, since the next share of a Murmure document asks again.
    ListMemberChannels,
    /// Open a new space, as [`SyncCommand::OpenDocument`] does for its
    /// document. Its lazy creation marks it a space, so each member lists it
    /// (`SYNC-170`).
    CreateSpace {
        space_id: Id<Space>,
    },

    // -- Flow control (posted by the Main relay, never by Logic) ---------------
    /// The path back to the Logic thread is full: stop reading the socket
    /// until [`SyncCommand::ReleaseInbound`] (`SYNC-059`).
    HoldInbound,
    /// The path back has room again: reopen the socket and catch up.
    ReleaseInbound,

    // -- Read cursors (Murmure) --------------------------------------------------
    /// Move the read cursors of one channel to its latest message
    /// (the Murmure unread spec). Each `live` segment moves to the last sequence number
    /// this worker delivered, so a message sequenced and not yet shown stays
    /// unread. Each `finals` segment no longer changes, so the relay moves it
    /// to its own last number.
    MarkRead {
        live: Vec<Id<Segment>>,
        finals: Vec<Id<Segment>>,
    },
    /// Move the cursor of each listed segment to the number beside it, back
    /// as well as forward: marking a message unread (the Murmure unread
    /// spec). Only the listed segments move.
    MoveReadCursors {
        cursors: Vec<(Id<Segment>, Seq)>,
    },
    /// Move the cursor of one thread to the last sequence number this worker
    /// delivered on it (the Murmure threads spec). A reply sequenced and not
    /// yet shown stays past it.
    MarkThreadRead {
        thread: Id<Document>,
    },
    /// Move every read cursor of the user to its document's last number
    /// (the Murmure unread spec), then answer with a fresh
    /// [`crate::LogicEvent::ChannelActivity`].
    MarkAllRead,
    /// Ask the relay which channel documents hold something past the user's
    /// cursors (`SRV-109`). Answered by [`crate::LogicEvent::ChannelActivity`].
    FetchActivity,

    // -- Lifecycle -------------------------------------------------------------
    Shutdown,
}
