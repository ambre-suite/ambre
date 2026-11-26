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

use ambre_core::{FixedBytes, SessionNonce};
use serde::{Deserialize, Serialize};

use ambre_core::id::{Channel, Document, Id, Lease, Message, Meta, Segment, Snapshot, Space, User};

use crate::{
    ActivitySummary, DirectMember, DocType, Relay, Request, RestCommand, SealedChannelKey, Seq,
    SnapshotPurpose, SnapshotVerdict,
};

/// Command sent from the Logic thread to the Sync thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SyncCommand {
    // -- Service plane (relayed request/response) ------------------------------
    /// A [`RestCommand`] service operation forwarded from the Logic thread. The
    /// Sync thread answers with [`crate::LogicEvent::Rest`] echoing `id`.
    Rest {
        id: Seq<Request>,
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
        /// The `meta` of the channel that groups the document, read from the
        /// sealed state of the channel, never from the relay's listing.
        /// `None` for a document no channel groups. The log and the key links
        /// of that `meta` then vouch for the key and the writers of the
        /// document (`SYNC-229`).
        meta: Option<Id<Meta>>,
    },

    /// Pin the inviter that the invite link of `document_id` names
    /// (`SYNC-205`). Every later open of the document in the session checks
    /// the log the relay serves against it.
    PinInviter {
        document_id: Id<Document>,
        inviter: ambre_core::FixedBytes<ambre_core::InviterPin>,
    },

    /// Hold `kept`, where the log of `document_id` stood in an earlier
    /// session, as the footing of every later open of the session
    /// (`SYNC-290`). The Main thread sends it from the local store before
    /// any open.
    KeepLog {
        document_id: Id<Document>,
        kept: crate::KeptLog,
    },

    /// Pin the inviter that a Murmure invite link names (`SYNC-236`). A
    /// Murmure page binds no document, so the pin roots every Murmure open
    /// of the session (`SYNC-235`).
    PinMurmureInviter {
        inviter: ambre_core::FixedBytes<ambre_core::InviterPin>,
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
        applied_seq: Seq<Relay>,
    },

    /// Raw CRDT delta produced by `LoroDoc::export_updates()`.
    SendUpdate {
        document_id: Id<Document>,
        payload: ambre_core::Payload<ambre_core::DeltaBytes>,
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
        payload: ambre_core::Payload<ambre_core::DeltaBytes>,
        nonce: FixedBytes<SessionNonce>,
    },

    /// Full CRDT state for compaction (presigned URL -> S3).
    SendSnapshot {
        document_id: Id<Document>,
        payload: ambre_core::Payload<ambre_core::SnapshotBytes>,
        /// The last delta `seq` the Logic thread holds with no gap below it.
        /// The snapshot claims to fold in every delta up to it (`SRV-083`).
        covered_seq: Seq<Relay>,
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
        payload: ambre_core::Payload<ambre_core::EphemeralBytes>,
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
        /// The channel key that catalogue holds, sealed under the space key.
        /// The Sync worker pins its id, and an open of the channel trusts
        /// that key. It trusts a newer one only when the log links it to the
        /// pin through signed rotations (`SYNC-299`).
        catalogue_key: Option<SealedChannelKey>,
    },
    /// The catalogue of `space_id` lists `channel_id`, which this client
    /// opened before that space, or while its listing was in flight. Its log
    /// then admits a self-join (`SYNC-106`), and the Sync worker walks it
    /// again under the rule of the space (`SYNC-230`). Answered by nothing.
    MarkChannelListed {
        channel_id: Id<Channel>,
        space_id: Id<Space>,
        /// The channel key the catalogue holds, which the Sync worker pins
        /// (`SYNC-299`).
        catalogue_key: SealedChannelKey,
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
        name: Option<ambre_core::Validated<ambre_core::Name>>,
    },
    /// Seal the name just written into the `meta` of an open channel beside
    /// it on the relay, under the channel key. `None` seals an empty name: the
    /// channel is unnamed. A failure is logged, and the next name replaces it.
    NameChannel {
        meta: Id<Meta>,
        name: Option<ambre_core::Validated<ambre_core::Name>>,
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
    /// A reply in `thread`, the thread of `root`, a message of `segment`.
    /// The root holds `receipts`. When none counts under the link key of
    /// this client, ask the relay for the receipt of that link (`SYNC-334`).
    /// Answered by [`crate::LogicEvent::ThreadLinked`], or by nothing.
    FreshenReceipt {
        channel_id: Id<Channel>,
        segment: Id<Segment>,
        root: Id<Message>,
        thread: Id<Document>,
        receipts: Vec<crate::LinkReceipt>,
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
        horizon: Seq<Relay>,
        payload: ambre_core::Payload<ambre_core::SnapshotBytes>,
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
        horizon: Seq<Relay>,
        amendments_seq: Seq<Relay>,
        payload: ambre_core::Payload<ambre_core::SnapshotBytes>,
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
        after: Seq<Relay>,
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
        meta_last: Option<Seq<Relay>>,
        documents: Vec<(Id<Document>, Option<Seq<Relay>>)>,
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
    ///
    /// `named` lists the documents the sealed `meta` names, which the Logic
    /// thread read. The census refuses a relay listing that leaves one out,
    /// and an empty `named` (`SYNC-320`). `threads` lists the threads those
    /// documents name. A listing that leaves one out passes only under an
    /// owner's erasure of it (`SYNC-321`, `SYNC-322`). `receipts` lists the
    /// relay's receipt of each link those documents hold: a thread under a
    /// valid one takes no erasure (`SYNC-330`).
    ///
    /// `erase` lists the hidden threads with no proof whose erasure the owner
    /// confirmed (`SYNC-333`). The census signs the erasure of no other.
    LeaveChannel {
        channel_id: Id<Channel>,
        named: Vec<Id<Document>>,
        threads: Vec<Id<Document>>,
        receipts: Vec<crate::LinkReceipt>,
        erase: Vec<Id<Document>>,
    },
    /// Revoke `user_id` from the channel whose `meta` is `document_id`, as
    /// [`RestCommand::RevokeFromDocument`] does. It carries what
    /// [`SyncCommand::LeaveChannel`] carries (`SYNC-320`). Answered under
    /// `id` as that request is.
    RevokeFromChannel {
        id: Seq<Request>,
        document_id: Id<Document>,
        user_id: Id<User>,
        named: Vec<Id<Document>>,
        threads: Vec<Id<Document>>,
        receipts: Vec<crate::LinkReceipt>,
        erase: Vec<Id<Document>>,
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

    // -- Calls (posted by the UI thread, never by Logic) ------------------------
    /// Join the call `call` of the channel `channel` (`CALL-065`). The UI thread posts it with
    /// the port of the call worker in the transfer list. The Sync worker hands each key of the
    /// call on that port alone (`CALL-064`).
    JoinCall {
        call: Id<ambre_core::id::Call>,
        channel: Id<Channel>,
    },
    /// Start the call `call` in the channel `channel`, as [`SyncCommand::JoinCall`] joins one.
    /// The relay creates the state of the call (`CALL-068`). The Sync worker answers
    /// [`crate::LogicEvent::CallStarted`] once seated.
    StartCall {
        call: Id<ambre_core::id::Call>,
        channel: Id<Channel>,
    },
    /// Leave the call: the relay answers whether the seat was the last, both sockets close,
    /// and the call worker drops every key (`CALL-029`, `CALL-069`).
    LeaveCall,
    /// Ask the relay who is in `call` of `channel`, whose `meta` document is `meta`. Posted by
    /// the Logic thread on the tick of a shown channel, and answered by
    /// [`crate::LogicEvent::CallRoster`] (the Murmure calls spec).
    AskCallRoster {
        call: Id<ambre_core::id::Call>,
        channel: Id<Channel>,
        meta: Id<Document>,
    },
    /// The `meta` document of the channel of a waiting join, the answer to
    /// [`crate::LogicEvent::CallMetaAsked`]. Posted by the Logic thread from the channel state
    /// it opened, whose sealed header names the channel. `None` when it holds no such state,
    /// and the join then ends (`CALL-149`, audit `P24`).
    CallMeta {
        call: Id<ambre_core::id::Call>,
        meta: Option<Id<Document>>,
    },
    /// A session description of the peer connection, for the socket of the SFU.
    CallDescription(crate::PageDescription),
    /// The screen sender of the page took a track: a share starts, which the socket of the SFU
    /// alone carries (`CALL-106`, `ZK-032`).
    ShareStarted,
    /// The page chose the videos it receives: the socket of the SFU alone carries the choice
    /// (`CALL-132`, `ZK-044`).
    ChooseVideos(ambre_protocol::signaling::VideoChoice),

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
        cursors: Vec<(Id<Segment>, Seq<Relay>)>,
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

    // -- The converter modules -------------------------------------------------
    /// Fetch the converter module of `family` from the page's origin and check
    /// it against the digest this build carries for that family (`ARCH-080`).
    /// The Logic thread asks on the first command of the family. The family
    /// is the one field: no byte of the file to convert rides along
    /// (`ARCH-083`). Answered by [`crate::LogicEvent::ConverterFetched`] or
    /// [`crate::LogicEvent::ConverterRefused`].
    FetchConverter {
        family: ambre_core::ConverterFamily,
    },

    // -- Lifecycle -------------------------------------------------------------
    Shutdown,
}
