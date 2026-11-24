// ambre-bus host tests (tier `host`, `ambre-spec-message-bus.md`).
// ============================================================================
// Pure, native (`cargo test -p ambre-bus`) coverage of the bus data plane:
//   - the bounded values the shared content plane carries,
//   - the request id issuer invariant (monotonic, one shared issuer),
//   - postcard round-trips of the Logic <-> Sync envelopes. The contract:
//     `SyncCommand` / `LogicEvent`, and the `Rest*` family they carry, survive the
//     `postMessage` hop unchanged, while application payloads stay `Vec<u8>`.
// No DOM, no serialization on the UI <-> Logic path (that link is `ambre_bus::conduit` by
// move and intentionally carries no `Serialize` derive).
// ============================================================================

#![expect(
    clippy::expect_used,
    reason = "a test asserts by panicking (ARCH-028 holds outside the tests)"
)]

use ambre_bus::next_request;
use ambre_bus::{
    ActivityAnchor, ActivityKind, ActivitySummary, Authored, ChannelMeta, ChannelName,
    ChannelPlace, DirectPeers, DocType, FoldAnswer, Instant, LeaseAnswer, LogicEvent,
    MemberChannel, MemberInfo, MemberStanding, OpenFailure, RestCommand, RestEvent, Role,
    SyncCommand,
};
use ambre_core::Seq;
use ambre_core::id::{
    Channel, Document, Id, Lease, Meeting, Message, Meta, Segment, Snapshot, Space, User,
};
use ambre_core::{Ms, TimeSpan};

// -- Service plane: request id issuer -----------------------------------------

#[test]
fn request_id_is_strictly_monotonic() {
    let a = next_request();
    let b = next_request();
    let c = next_request();
    assert!(Seq::ZERO < a, "no request carries the zero id");
    assert!(a < b, "ids must increase");
    assert!(b < c);
}

#[test]
fn request_id_is_unique_under_concurrency() {
    use std::collections::HashSet;

    // Each thread keeps its own ids, so the test shares no lock with them.
    let handles: Vec<_> = (0..8)
        .map(|_| std::thread::spawn(|| (0..1000).map(|_| next_request()).collect::<Vec<_>>()))
        .collect();
    let mut seen = HashSet::new();
    for h in handles {
        for id in h.join().unwrap() {
            assert!(seen.insert(id), "duplicate id {id:?}");
        }
    }
    assert_eq!(seen.len(), 8 * 1000);
}

// -- Logic <-> Sync envelope round-trips (postcard, the postMessage codec) ----

fn roundtrip_sync(cmd: &SyncCommand) {
    let bytes = postcard::to_allocvec(cmd).expect("encode SyncCommand");
    let back: SyncCommand = postcard::from_bytes(&bytes).expect("decode SyncCommand");
    // SyncCommand is not PartialEq (carries RestCommand); compare via Debug.
    assert_eq!(format!("{cmd:?}"), format!("{back:?}"));
}

fn roundtrip_event(evt: &LogicEvent) {
    let bytes = postcard::to_allocvec(evt).expect("encode LogicEvent");
    let back: LogicEvent = postcard::from_bytes(&bytes).expect("decode LogicEvent");
    assert_eq!(format!("{evt:?}"), format!("{back:?}"));
}

#[test]
fn sync_command_roundtrips_through_postcard() {
    let doc = Id::<Document>::mint();
    let chan = Id::<Channel>::mint();
    roundtrip_sync(&SyncCommand::Rest {
        id: Seq::nth(42),
        cmd: RestCommand::CreateDocument {
            doc_type: DocType::Trame,
            name: ambre_core::Validated::<ambre_core::Name>::parse("Budget").unwrap(),
        },
    });
    roundtrip_sync(&SyncCommand::OpenDocument {
        document_id: doc,
        doc_type: DocType::Trame,
        meta: None,
    });
    roundtrip_sync(&SyncCommand::OpenDocument {
        document_id: doc,
        doc_type: DocType::Murmure,
        meta: Some(Id::<ambre_core::id::Meta>::mint()),
    });
    // A genuine network-byte payload survives verbatim.
    roundtrip_sync(&SyncCommand::SendUpdate {
        document_id: doc,
        payload: ambre_core::Payload::<ambre_core::DeltaBytes>::new(vec![0, 1, 2, 250, 255]),
        summary: None,
    });
    roundtrip_sync(&SyncCommand::SendUpdate {
        document_id: doc,
        payload: ambre_core::Payload::<ambre_core::DeltaBytes>::new(vec![7]),
        summary: Some(ActivitySummary {
            kind: ActivityKind::Content,
            mentions: vec![Id::<User>::mint()],
            mentions_all: ambre_bus::MentionScope::All,
            anchor: ActivityAnchor::new(vec![1, 2]),
            call: None,
        }),
    });
    roundtrip_sync(&SyncCommand::SendUnsent {
        document_id: doc,
        payload: ambre_core::Payload::<ambre_core::DeltaBytes>::new(vec![3, 4]),
        nonce: ambre_core::FixedBytes::from_array([5; 16]),
    });
    roundtrip_sync(&SyncCommand::OpenChannel {
        channel_id: chan,
        listed_in: Some(Id::<Space>::mint()),
        catalogue_key: Some(ambre_bus::SealedChannelKey::from_bytes(vec![2; 72])),
    });
    roundtrip_sync(&SyncCommand::Shutdown);
}

#[test]
fn logic_event_roundtrips_through_postcard() {
    let doc = Id::<Document>::mint();
    let user = Id::<User>::mint();
    roundtrip_event(&LogicEvent::Rest {
        id: Some(Seq::nth(7)),
        evt: RestEvent::DocumentCreated {
            document_id: doc,
            doc_type: DocType::Trame,
        },
    });
    roundtrip_event(&LogicEvent::RemoteUpdate {
        document_id: doc,
        delta: ambre_bus::SignedDelta {
            seq: ambre_bus::Seq::<ambre_bus::Relay>::new(8),
            payload: ambre_core::Payload::<ambre_core::DeltaBytes>::new(vec![3, 1, 4, 1, 5, 9]),
            nonce: ambre_core::FixedBytes::from_array([2; 16]),
            peer: ambre_core::PeerId::new(7),
            user: Some(Id::<User>::from_bytes([1; 16])),
            summary: Some(ambre_bus::ActivitySummary {
                kind: ambre_bus::ActivityKind::Content,
                mentions: vec![Id::<User>::from_bytes([3; 16])],
                mentions_all: ambre_bus::MentionScope::Named,
                anchor: None,
                call: None,
            }),
        },
    });
    roundtrip_event(&LogicEvent::SessionIdentity {
        document_id: doc,
        peer: ambre_core::PeerId::new(7),
        entry: an_entry(8),
        nonce: ambre_core::FixedBytes::from_array([9; 16]),
    });
    roundtrip_event(&LogicEvent::ConnectionStatus {
        document_id: doc,
        connected: ambre_core::LinkState::Connected,
    });
    roundtrip_event(&LogicEvent::MemberRevoked {
        document_id: doc,
        revoked_user_id: user,
    });
}

/// The variant's name. The match has no wildcard arm. So a variant added to
/// the envelope does not compile until it is named here. Then
/// `every_envelope_variant_roundtrips_through_postcard` asks for a sample.
fn sync_variant(cmd: &SyncCommand) -> &'static str {
    match cmd {
        SyncCommand::Rest { .. } => "Rest",
        SyncCommand::OpenDocument { .. } => "OpenDocument",
        SyncCommand::PinInviter { .. } => "PinInviter",
        SyncCommand::KeepLog { .. } => "KeepLog",
        SyncCommand::PinMurmureInviter { .. } => "PinMurmureInviter",
        SyncCommand::CloseDocument { .. } => "CloseDocument",
        SyncCommand::ReleaseDocument { .. } => "ReleaseDocument",
        SyncCommand::SendUpdate { .. } => "SendUpdate",
        SyncCommand::SendUnsent { .. } => "SendUnsent",
        SyncCommand::SendSnapshot { .. } => "SendSnapshot",
        SyncCommand::SnapshotVerdict { .. } => "SnapshotVerdict",
        SyncCommand::SendEphemeral { .. } => "SendEphemeral",
        SyncCommand::Reconnect { .. } => "Reconnect",
        SyncCommand::OpenChannel { .. } => "OpenChannel",
        SyncCommand::MarkChannelListed { .. } => "MarkChannelListed",
        SyncCommand::CreateChannel { .. } => "CreateChannel",
        SyncCommand::RotateSpace { .. } => "RotateSpace",
        SyncCommand::CloseSegment { .. } => "CloseSegment",
        SyncCommand::CreateAmendments { .. } => "CreateAmendments",
        SyncCommand::CreateThread { .. } => "CreateThread",
        SyncCommand::TakeFinalizeLease { .. } => "TakeFinalizeLease",
        SyncCommand::FinalizeSegment { .. } => "FinalizeSegment",
        SyncCommand::TakeFoldLease { .. } => "TakeFoldLease",
        SyncCommand::FoldSegment { .. } => "FoldSegment",
        SyncCommand::WatchSegment { .. } => "WatchSegment",
        SyncCommand::WatchThread { .. } => "WatchThread",
        SyncCommand::WatchChannel { .. } => "WatchChannel",
        SyncCommand::NameChannel { .. } => "NameChannel",
        SyncCommand::JoinChannel { .. } => "JoinChannel",
        SyncCommand::DeleteChannel { .. } => "DeleteChannel",
        SyncCommand::LeaveChannel { .. } => "LeaveChannel",
        SyncCommand::RevokeFromChannel { .. } => "RevokeFromChannel",
        SyncCommand::CreateDirectMessage { .. } => "CreateDirectMessage",
        SyncCommand::ListSpaceChannels { .. } => "ListSpaceChannels",
        SyncCommand::ListMemberChannels => "ListMemberChannels",
        SyncCommand::CreateSpace { .. } => "CreateSpace",
        SyncCommand::HoldInbound => "HoldInbound",
        SyncCommand::ReleaseInbound => "ReleaseInbound",
        SyncCommand::MarkRead { .. } => "MarkRead",
        SyncCommand::MoveReadCursors { .. } => "MoveReadCursors",
        SyncCommand::MarkThreadRead { .. } => "MarkThreadRead",
        SyncCommand::MarkAllRead => "MarkAllRead",
        SyncCommand::FetchActivity => "FetchActivity",
        SyncCommand::FetchConverter { .. } => "FetchConverter",
        SyncCommand::JoinCall { .. } => "JoinCall",
        SyncCommand::StartCall { .. } => "StartCall",
        SyncCommand::LeaveCall => "LeaveCall",
        SyncCommand::AskCallRoster { .. } => "AskCallRoster",
        SyncCommand::CallMeta { .. } => "CallMeta",
        SyncCommand::CallDescription(_) => "CallDescription",
        SyncCommand::ShareStarted => "ShareStarted",
        SyncCommand::ChooseVideos(_) => "ChooseVideos",
        SyncCommand::Shutdown => "Shutdown",
    }
}

/// The variant's name, with no wildcard arm, as for `sync_variant`.
fn event_variant(evt: &LogicEvent) -> &'static str {
    match evt {
        LogicEvent::Rest { .. } => "Rest",
        LogicEvent::Call(_) => "Call",
        LogicEvent::CallStarted { .. } => "CallStarted",
        LogicEvent::CallEnded { .. } => "CallEnded",
        LogicEvent::CallRoster { .. } => "CallRoster",
        LogicEvent::CallMetaAsked { .. } => "CallMetaAsked",
        LogicEvent::SessionIdentity { .. } => "SessionIdentity",
        LogicEvent::RemoteUpdate { .. } => "RemoteUpdate",
        LogicEvent::RemoteUpdateBatch { .. } => "RemoteUpdateBatch",
        LogicEvent::RemoteSnapshot { .. } => "RemoteSnapshot",
        LogicEvent::PendingSnapshot { .. } => "PendingSnapshot",
        LogicEvent::SnapshotDropped { .. } => "SnapshotDropped",
        LogicEvent::ThreadActivity { .. } => "ThreadActivity",
        LogicEvent::RemoteEphemeral { .. } => "RemoteEphemeral",
        LogicEvent::PeerLeft { .. } => "PeerLeft",
        LogicEvent::MembersChanged { .. } => "MembersChanged",
        LogicEvent::InviterProven { .. } => "InviterProven",
        LogicEvent::LogKept { .. } => "LogKept",
        LogicEvent::DocumentReread { .. } => "DocumentReread",
        LogicEvent::DocumentReady { .. } => "DocumentReady",
        LogicEvent::ConnectionStatus { .. } => "ConnectionStatus",
        LogicEvent::Activity { .. } => "Activity",
        LogicEvent::CallCue { .. } => "CallCue",
        LogicEvent::Released { .. } => "Released",
        LogicEvent::SyncWarning { .. } => "SyncWarning",
        LogicEvent::SnapshotNeeded { .. } => "SnapshotNeeded",
        LogicEvent::MemberRevoked { .. } => "MemberRevoked",
        LogicEvent::DocumentGone { .. } => "DocumentGone",
        LogicEvent::KeyRotated { .. } => "KeyRotated",
        LogicEvent::DocumentShared { .. } => "DocumentShared",
        LogicEvent::ChannelDocuments { .. } => "ChannelDocuments",
        LogicEvent::ChannelOpenFailed { .. } => "ChannelOpenFailed",
        LogicEvent::ChannelRefused { .. } => "ChannelRefused",
        LogicEvent::ChannelActivity { .. } => "ChannelActivity",
        LogicEvent::ChannelCreated { .. } => "ChannelCreated",
        LogicEvent::ChannelKeyResealed { .. } => "ChannelKeyResealed",
        LogicEvent::SegmentClosed { .. } => "SegmentClosed",
        LogicEvent::SegmentCloseFailed { .. } => "SegmentCloseFailed",
        LogicEvent::AmendmentsLinked { .. } => "AmendmentsLinked",
        LogicEvent::AmendmentsLinkFailed { .. } => "AmendmentsLinkFailed",
        LogicEvent::ThreadLinked { .. } => "ThreadLinked",
        LogicEvent::ThreadLinkFailed { .. } => "ThreadLinkFailed",
        LogicEvent::FinalizeLeaseAnswered { .. } => "FinalizeLeaseAnswered",
        LogicEvent::SegmentFinalized { .. } => "SegmentFinalized",
        LogicEvent::SegmentFinalizeFailed { .. } => "SegmentFinalizeFailed",
        LogicEvent::FoldLeaseAnswered { .. } => "FoldLeaseAnswered",
        LogicEvent::SegmentFolded { .. } => "SegmentFolded",
        LogicEvent::SegmentFoldFailed { .. } => "SegmentFoldFailed",
        LogicEvent::AmendmentsFolded { .. } => "AmendmentsFolded",
        LogicEvent::DeltasLocked { .. } => "DeltasLocked",
        LogicEvent::ChannelJoined { .. } => "ChannelJoined",
        LogicEvent::DirectMessageOpened { .. } => "DirectMessageOpened",
        LogicEvent::ChannelDeleted { .. } => "ChannelDeleted",
        LogicEvent::ChannelLeft { .. } => "ChannelLeft",
        LogicEvent::ChannelRequestFailed { .. } => "ChannelRequestFailed",
        LogicEvent::SpaceChannels { .. } => "SpaceChannels",
        LogicEvent::MemberChannels { .. } => "MemberChannels",
        LogicEvent::MeetingParticipantJoined { .. } => "MeetingParticipantJoined",
        LogicEvent::MeetingParticipantLeft { .. } => "MeetingParticipantLeft",
        LogicEvent::DocumentOpenFailed { .. } => "DocumentOpenFailed",
        LogicEvent::AuthError { .. } => "AuthError",
        LogicEvent::KeyNotShared { .. } => "KeyNotShared",
        LogicEvent::ApiError { .. } => "ApiError",
        LogicEvent::SetupRequired => "SetupRequired",
        LogicEvent::UpdateRequired => "UpdateRequired",
        LogicEvent::ConverterFetched { .. } => "ConverterFetched",
        LogicEvent::ConverterRefused { .. } => "ConverterRefused",
    }
}

/// The number of arms of `sync_variant` and of `event_variant`.
const SYNC_VARIANTS: usize = 53;
const EVENT_VARIANTS: usize = 67;

/// `BUS-003`: every variant of both envelopes survives the hop, field for
/// field. Each arm of the two name functions needs a sample here, and the
/// samples cover each name exactly once.
#[test]
fn every_envelope_variant_roundtrips_through_postcard() {
    use ambre_bus::{
        ChannelDocument, ChannelDocumentKind, DirectMember, DocumentActivity, Relay,
        SealedChannelKey, Seq, SignedDelta, SnapshotPurpose, SnapshotVerdict,
    };
    use ambre_core::FixedBytes;
    use std::collections::BTreeSet;

    let doc = Id::<Document>::mint();
    let chan = Id::<Channel>::mint();
    let direct = Id::<Channel>::mint();
    let space = Id::<Space>::mint();
    let meta = Id::<Meta>::from_document(Id::<Document>::mint());
    let segment = Id::<Segment>::from_document(Id::<Document>::mint());
    let user = Id::<User>::mint();
    let snapshot = Id::<Snapshot>::genesis(doc);
    let delta = SignedDelta {
        seq: ambre_bus::Seq::<ambre_bus::Relay>::new(3),
        payload: ambre_core::Payload::<ambre_core::DeltaBytes>::new(vec![1, 2, 255]),
        nonce: FixedBytes::from_array([4; 16]),
        peer: ambre_core::PeerId::new(9),
        user: Some(user),
        summary: None,
    };
    let commands = [
        SyncCommand::Rest {
            id: Seq::nth(5),
            cmd: RestCommand::CreateDocument {
                doc_type: DocType::Toile,
                name: ambre_core::Validated::<ambre_core::Name>::parse("Plan").unwrap(),
            },
        },
        SyncCommand::OpenDocument {
            document_id: doc,
            doc_type: DocType::Prose,
            meta: None,
        },
        SyncCommand::PinInviter {
            document_id: doc,
            inviter: FixedBytes::from_array([5; 32]),
        },
        SyncCommand::KeepLog {
            document_id: doc,
            kept: kept_log(),
        },
        SyncCommand::PinMurmureInviter {
            inviter: FixedBytes::from_array([6; 32]),
        },
        SyncCommand::CloseDocument { document_id: doc },
        SyncCommand::ReleaseDocument {
            document_id: doc,
            applied_seq: ambre_bus::Seq::<ambre_bus::Relay>::new(12),
        },
        SyncCommand::SendUpdate {
            document_id: doc,
            payload: ambre_core::Payload::<ambre_core::DeltaBytes>::new(vec![0, 7]),
            summary: Some(ActivitySummary {
                kind: ActivityKind::Content,
                mentions: vec![user],
                mentions_all: ambre_bus::MentionScope::Named,
                anchor: ActivityAnchor::new(vec![3]),
                call: None,
            }),
        },
        SyncCommand::SendUnsent {
            document_id: doc,
            payload: ambre_core::Payload::<ambre_core::DeltaBytes>::new(vec![8]),
            nonce: FixedBytes::from_array([6; 16]),
        },
        SyncCommand::SendSnapshot {
            document_id: doc,
            payload: ambre_core::Payload::<ambre_core::SnapshotBytes>::new(vec![1; 4]),
            covered_seq: ambre_bus::Seq::<ambre_bus::Relay>::new(40),
        },
        SyncCommand::SnapshotVerdict {
            document_id: doc,
            snapshot_id: snapshot,
            purpose: SnapshotPurpose::Confirm,
            verdict: SnapshotVerdict::Match {
                extras: vec![(
                    ambre_core::PeerId::new(41),
                    ambre_core::Payload::<ambre_core::PeerEntryBytes>::new(vec![2, 3]),
                )],
            },
        },
        SyncCommand::SendEphemeral {
            document_id: doc,
            payload: ambre_core::Payload::<ambre_core::EphemeralBytes>::new(vec![5]),
        },
        SyncCommand::Reconnect { document_id: doc },
        SyncCommand::OpenChannel {
            channel_id: chan,
            listed_in: None,
            catalogue_key: None,
        },
        SyncCommand::MarkChannelListed {
            channel_id: chan,
            space_id: space,
            catalogue_key: SealedChannelKey::from_bytes(vec![1; 72]),
        },
        SyncCommand::RotateSpace {
            space_id: space,
            channels: vec![(chan, SealedChannelKey::from_bytes(vec![8; 3]))],
        },
        SyncCommand::CreateChannel {
            channel_id: chan,
            space_id: space,
            meta,
            segment,
            visibility: ambre_core::Visibility::Private,
            name: Some(ambre_core::Validated::<ambre_core::Name>::parse("board").unwrap()),
        },
        SyncCommand::NameChannel { meta, name: None },
        SyncCommand::JoinChannel {
            channel_id: chan,
            space_id: space,
            key: SealedChannelKey::from_bytes(vec![6; 3]),
        },
        SyncCommand::CloseSegment {
            channel_id: chan,
            meta,
            closing: segment,
            successor: Id::<Segment>::from_document(Id::<Document>::mint()),
        },
        SyncCommand::CreateAmendments {
            channel_id: chan,
            meta,
            segment,
            amendments: Id::<Document>::mint(),
        },
        SyncCommand::CreateThread {
            channel_id: chan,
            meta,
            segment,
            root: Id::<Message>::mint(),
            thread: Id::<Document>::mint(),
        },
        SyncCommand::TakeFinalizeLease {
            channel_id: chan,
            segment,
        },
        SyncCommand::FinalizeSegment {
            channel_id: chan,
            segment,
            lease: Id::<Lease>::mint(),
            horizon: ambre_bus::Seq::<ambre_bus::Relay>::new(42),
            payload: ambre_core::Payload::<ambre_core::SnapshotBytes>::new(vec![4; 3]),
        },
        SyncCommand::TakeFoldLease {
            channel_id: chan,
            segment,
        },
        SyncCommand::FoldSegment {
            channel_id: chan,
            segment,
            lease: Id::<Lease>::mint(),
            horizon: ambre_bus::Seq::<ambre_bus::Relay>::new(42),
            amendments_seq: ambre_bus::Seq::<ambre_bus::Relay>::new(7),
            payload: ambre_core::Payload::<ambre_core::SnapshotBytes>::new(vec![5; 3]),
        },
        SyncCommand::WatchSegment { segment, meta },
        SyncCommand::WatchThread {
            thread: doc,
            meta,
            after: Seq::<Relay>::new(3),
        },
        SyncCommand::WatchChannel {
            channel_id: chan,
            meta,
            meta_last: Some(Seq::<Relay>::new(2)),
            documents: vec![(doc, Some(Seq::<Relay>::new(3))), (doc, None)],
        },
        SyncCommand::DeleteChannel { channel_id: chan },
        SyncCommand::LeaveChannel {
            channel_id: chan,
            named: vec![doc, doc],
            threads: vec![doc],
        },
        SyncCommand::RevokeFromChannel {
            id: Seq::<ambre_bus::Request>::new(9),
            document_id: doc,
            user_id: user,
            named: vec![doc],
            threads: vec![doc],
        },
        SyncCommand::ListSpaceChannels { space_id: space },
        SyncCommand::ListMemberChannels,
        SyncCommand::CreateSpace { space_id: space },
        SyncCommand::CreateDirectMessage {
            channel_id: chan,
            meta,
            segment,
            members: vec![DirectMember {
                user_id: user,
                exchange_pub_key: ambre_core::FixedBytes::from_array([7; 32]),
            }],
        },
        SyncCommand::HoldInbound,
        SyncCommand::ReleaseInbound,
        SyncCommand::MarkRead {
            live: vec![segment],
            finals: Vec::new(),
        },
        SyncCommand::MoveReadCursors {
            cursors: vec![(segment, Seq::<Relay>::new(4))],
        },
        SyncCommand::MarkThreadRead { thread: doc },
        SyncCommand::MarkAllRead,
        SyncCommand::FetchActivity,
        SyncCommand::FetchConverter {
            family: ambre_core::ConverterFamily::Markup,
        },
        SyncCommand::JoinCall {
            call: Id::<ambre_core::id::Call>::mint(),
            channel: chan,
        },
        SyncCommand::StartCall {
            call: Id::<ambre_core::id::Call>::mint(),
            channel: chan,
        },
        SyncCommand::LeaveCall,
        SyncCommand::ShareStarted,
        SyncCommand::ChooseVideos(
            ambre_protocol::signaling::VideoChoice::new(
                vec![Seq::<ambre_core::SenderIndex>::new(4)],
                None,
                ambre_protocol::signaling::VIDEOS_PER_RECEIVER,
            )
            .unwrap(),
        ),
        SyncCommand::AskCallRoster {
            call: Id::<ambre_core::id::Call>::mint(),
            channel: chan,
            meta: doc,
        },
        SyncCommand::CallMeta {
            call: Id::<ambre_core::id::Call>::mint(),
            meta: Some(doc),
        },
        SyncCommand::CallDescription(ambre_bus::PageDescription::Offer(
            ambre_core::Validated::parse("v=0".to_owned()).unwrap(),
        )),
        SyncCommand::Shutdown,
    ];
    let events = [
        LogicEvent::Call(ambre_bus::CallSignal::Answer(
            ambre_core::Validated::parse("v=0".to_owned()).unwrap(),
        )),
        LogicEvent::CallStarted {
            call: Id::<ambre_core::id::Call>::mint(),
            channel: chan,
            at: Instant::<Authored>::new(5),
        },
        LogicEvent::CallEnded {
            call: Id::<ambre_core::id::Call>::mint(),
            channel: chan,
            at: Instant::<Authored>::new(6),
        },
        LogicEvent::CallRoster {
            call: Id::<ambre_core::id::Call>::mint(),
            channel: chan,
            members: ambre_bus::CallMembers::Live(vec![Id::<User>::mint()]),
        },
        LogicEvent::CallMetaAsked {
            call: Id::<ambre_core::id::Call>::mint(),
            channel: chan,
        },
        LogicEvent::Rest {
            id: Some(Seq::nth(6)),
            evt: RestEvent::DocumentCreated {
                document_id: doc,
                doc_type: DocType::Toile,
            },
        },
        LogicEvent::SessionIdentity {
            document_id: doc,
            peer: ambre_core::PeerId::new(9),
            entry: an_entry(3),
            nonce: FixedBytes::from_array([1; 16]),
        },
        LogicEvent::RemoteUpdate {
            document_id: doc,
            delta: delta.clone(),
        },
        LogicEvent::RemoteUpdateBatch {
            document_id: doc,
            deltas: vec![delta.clone(), delta],
        },
        LogicEvent::RemoteSnapshot {
            document_id: doc,
            payload: ambre_core::Payload::<ambre_core::SnapshotBytes>::new(vec![9; 3]),
            covered_seq: ambre_bus::Seq::<ambre_bus::Relay>::new(30),
        },
        LogicEvent::PendingSnapshot {
            document_id: doc,
            snapshot_id: snapshot,
            covered_seq: ambre_bus::Seq::<ambre_bus::Relay>::new(31),
            payload: ambre_core::Payload::<ambre_core::SnapshotBytes>::new(vec![4]),
            purpose: SnapshotPurpose::Open,
        },
        LogicEvent::SnapshotDropped {
            document_id: doc,
            author: Some(user),
            reporter: None,
            after: ambre_core::AfterDrop::Refinalize,
        },
        LogicEvent::ThreadActivity { document_id: doc },
        LogicEvent::RemoteEphemeral {
            document_id: doc,
            from_user_id: user,
            payload: ambre_core::Payload::<ambre_core::EphemeralBytes>::new(vec![2]),
        },
        LogicEvent::PeerLeft {
            document_id: doc,
            user_id: user,
        },
        LogicEvent::MembersChanged {
            document_id: doc,
            members: vec![MemberInfo {
                user_id: user,
                display_name: ambre_core::Validated::<ambre_core::DisplayName>::parse("Ada")
                    .unwrap(),
                email: "ada@example.org".into(),
                role: Role::Editor,
                standing: MemberStanding::Verified,
            }],
        },
        LogicEvent::InviterProven {
            document_id: doc,
            inviter: FixedBytes::from_array([8; 32]),
        },
        LogicEvent::LogKept {
            document_id: doc,
            kept: kept_log(),
        },
        LogicEvent::DocumentReread { document_id: doc },
        LogicEvent::DocumentReady {
            document_id: doc,
            role: Role::Viewer,
        },
        LogicEvent::ConnectionStatus {
            document_id: doc,
            connected: ambre_core::LinkState::Disconnected,
        },
        LogicEvent::Activity {
            document_id: doc,
            counters: ambre_core::activity::ActivityCounters {
                content: ambre_core::Count::<ambre_core::activity::ContentUpdates>::new(1),
                mentions: ambre_core::Count::<ambre_core::activity::MentionUpdates>::new(2),
                ..Default::default()
            },
        },
        LogicEvent::Released { document_id: doc },
        LogicEvent::SyncWarning {
            document_id: doc,
            code: ambre_bus::sync_warning::SyncWarningCode::DecryptFailed,
        },
        LogicEvent::SnapshotNeeded { document_id: doc },
        LogicEvent::MemberRevoked {
            document_id: doc,
            revoked_user_id: user,
        },
        LogicEvent::DocumentGone { document_id: doc },
        LogicEvent::KeyRotated { document_id: doc },
        LogicEvent::DocumentShared {
            document_id: doc,
            doc_type: DocType::Trame,
        },
        LogicEvent::ChannelDocuments {
            channel_id: chan,
            documents: vec![ChannelDocument {
                document_id: segment.document_id(),
                kind: ChannelDocumentKind::Segment,
            }],
        },
        LogicEvent::ChannelOpenFailed {
            channel_id: chan,
            failure: ambre_bus::ApiFailure::NoAnswer,
        },
        LogicEvent::ChannelRefused { channel_id: chan },
        LogicEvent::ChannelActivity {
            documents: vec![DocumentActivity {
                channel_id: chan,
                document_id: segment.document_id(),
                kind: ChannelDocumentKind::Segment,
                active: ambre_bus::Activity::Active,
                read: Seq::<Relay>::new(2),
                last: Some(Seq::<Relay>::new(5)),
            }],
        },
        LogicEvent::ChannelCreated {
            channel_id: chan,
            key: Some(SealedChannelKey::from_bytes(vec![1; 2])),
        },
        LogicEvent::ChannelKeyResealed {
            space_id: space,
            channel_id: chan,
            key: SealedChannelKey::from_bytes(vec![9; 2]),
        },
        LogicEvent::SegmentClosed {
            channel_id: chan,
            closing: segment,
            active: Id::<Segment>::from_document(Id::<Document>::mint()),
            successor: ambre_bus::Successor::Fetched,
        },
        LogicEvent::SegmentCloseFailed {
            channel_id: chan,
            closing: segment,
            failure: ambre_bus::ApiFailure::Local(ambre_core::LocalFailure::PinMismatch(user)),
        },
        LogicEvent::AmendmentsLinked {
            channel_id: chan,
            segment,
            amendments: Id::<Document>::mint(),
        },
        LogicEvent::AmendmentsLinkFailed {
            channel_id: chan,
            segment,
            amendments: Id::<Document>::mint(),
            failure: ambre_bus::ApiFailure::Local(ambre_core::LocalFailure::PinMismatch(user)),
        },
        LogicEvent::ThreadLinked {
            channel_id: chan,
            segment,
            root: Id::<Message>::mint(),
            thread: Id::<Document>::mint(),
        },
        LogicEvent::ThreadLinkFailed {
            channel_id: chan,
            segment,
            root: Id::<Message>::mint(),
            thread: Id::<Document>::mint(),
            failure: ambre_bus::ApiFailure::Local(ambre_core::LocalFailure::PinMismatch(user)),
        },
        LogicEvent::FinalizeLeaseAnswered {
            channel_id: chan,
            segment,
            answer: LeaseAnswer::Granted {
                lease: Id::<Lease>::mint(),
                horizon: ambre_bus::Seq::<ambre_bus::Relay>::new(42),
                expires_in: TimeSpan::<Ms>::new(30_000),
            },
        },
        LogicEvent::SegmentFinalized {
            channel_id: chan,
            segment,
        },
        LogicEvent::SegmentFinalizeFailed {
            channel_id: chan,
            segment,
            end: ambre_core::FinalizeEnd::Withdrawn,
            failure: ambre_bus::ApiFailure::Local(ambre_core::LocalFailure::PinMismatch(user)),
        },
        LogicEvent::FoldLeaseAnswered {
            channel_id: chan,
            segment,
            answer: FoldAnswer::Granted {
                lease: Id::<Lease>::mint(),
                horizon: ambre_bus::Seq::<ambre_bus::Relay>::new(42),
                amendments: Id::<Document>::mint(),
                amendments_seq: ambre_bus::Seq::<ambre_bus::Relay>::new(7),
                expires_in: TimeSpan::<Ms>::new(30_000),
            },
        },
        LogicEvent::SegmentFolded {
            channel_id: chan,
            segment,
        },
        LogicEvent::SegmentFoldFailed {
            channel_id: chan,
            segment,
            failure: ambre_bus::ApiFailure::Local(ambre_core::LocalFailure::PinMismatch(user)),
        },
        LogicEvent::AmendmentsFolded {
            document_id: segment.document_id(),
            amendments: Id::<Document>::mint(),
        },
        LogicEvent::DeltasLocked {
            document_id: segment.document_id(),
            deltas: vec![b"{}".to_vec().into()],
        },
        LogicEvent::ChannelJoined { channel_id: chan },
        LogicEvent::DirectMessageOpened {
            requested: chan,
            channel_id: Id::<Channel>::mint(),
        },
        LogicEvent::ChannelDeleted { channel_id: chan },
        LogicEvent::ChannelLeft { channel_id: chan },
        LogicEvent::ChannelRequestFailed {
            channel_id: chan,
            failure: ambre_bus::ApiFailure::Local(ambre_core::LocalFailure::PinMismatch(user)),
        },
        LogicEvent::SpaceChannels {
            space_id: space,
            channels: vec![chan],
        },
        LogicEvent::MemberChannels {
            channels: vec![
                MemberChannel {
                    channel_id: chan,
                    place: ChannelPlace::Private(space),
                },
                MemberChannel {
                    channel_id: direct,
                    place: ChannelPlace::Direct,
                },
            ],
            spaces: vec![space],
            names: vec![ChannelName {
                channel_id: chan,
                name: ambre_core::Validated::<ambre_core::Name>::parse("board").unwrap(),
            }],
            peers: vec![DirectPeers {
                channel_id: direct,
                names: vec![
                    ambre_core::Validated::<ambre_core::DisplayName>::parse("Bob").unwrap(),
                ],
            }],
            metas: vec![ChannelMeta {
                channel_id: chan,
                meta: Id::<Meta>::from_document(doc),
                last: Some(Seq::<Relay>::new(5)),
            }],
        },
        LogicEvent::MeetingParticipantJoined {
            meeting_id: Id::<Meeting>::mint(),
            user_id: user,
        },
        LogicEvent::MeetingParticipantLeft {
            meeting_id: Id::<Meeting>::mint(),
            user_id: user,
        },
        LogicEvent::DocumentOpenFailed {
            document_id: doc,
            cause: OpenFailure::KeyNotReceived,
        },
        LogicEvent::AuthError {
            cause: ambre_bus::AuthFailure::Api(ambre_bus::ApiFailure::Unreadable {
                cause: ambre_bus::UnreadableCause::UnknownWord,
            }),
        },
        LogicEvent::ApiError {
            id: Some(Seq::nth(8)),
            request: ambre_bus::ApiRequest::InviteToDocument,
            failure: ambre_bus::ApiFailure::refused(ambre_bus::RefusalCode::MembersChanged),
        },
        LogicEvent::KeyNotShared {
            document_id: doc,
            members: vec![
                ambre_core::Validated::<ambre_core::DisplayName>::parse("Carol").unwrap(),
            ],
        },
        LogicEvent::SetupRequired,
        LogicEvent::UpdateRequired,
        LogicEvent::ConverterFetched {
            family: ambre_core::ConverterFamily::Tables,
            module: ambre_core::Payload::new(b"\0asm".to_vec()),
        },
        LogicEvent::CallCue {
            document_id: doc,
            cue: ambre_bus::CallCue {
                call: Id::<ambre_core::id::Call>::from_bytes([3; 16]),
                kind: ambre_bus::CueKind::Ring,
                at: Instant::<Authored>::new(42),
            },
        },
        LogicEvent::ConverterRefused {
            family: ambre_core::ConverterFamily::Images,
            cause: ambre_core::ConverterRefusal::DigestMismatch,
        },
    ];
    for cmd in &commands {
        roundtrip_sync(cmd);
    }
    for evt in &events {
        roundtrip_event(evt);
    }
    let named: BTreeSet<_> = commands.iter().map(sync_variant).collect();
    assert_eq!(
        (commands.len(), named.len()),
        (SYNC_VARIANTS, SYNC_VARIANTS)
    );
    let named: BTreeSet<_> = events.iter().map(event_variant).collect();
    assert_eq!(
        (events.len(), named.len()),
        (EVENT_VARIANTS, EVENT_VARIANTS)
    );
}

/// Whether `T` can be encoded. The inherent constant applies when `T`
/// implements `Serialize`, and the trait's default applies otherwise.
struct Encodable<T>(std::marker::PhantomData<T>);

trait NotEncodable {
    const ENCODES: bool = false;
}

impl<T> NotEncodable for Encodable<T> {}

impl<T: serde::Serialize> Encodable<T> {
    const ENCODES: bool = true;
}

/// `BUS-001`: nothing on the UI to Logic path can be encoded. The two
/// envelopes that cross it implement no `Serialize`, whatever document command
/// or event they carry, so no encoder has a value to take. The probe reads
/// true for an envelope of the worker hop. The Logic runtime probes the two
/// sums it declares the same way.
#[test]
fn the_ui_to_logic_path_carries_nothing_an_encoder_takes() {
    use ambre_bus::{LogicCommand, UiEvent};

    // Each probe is a constant, so a derive added tomorrow fails the build.
    const {
        assert!(!Encodable::<LogicCommand>::ENCODES);
        assert!(!Encodable::<UiEvent>::ENCODES);
        assert!(Encodable::<SyncCommand>::ENCODES);
    }
}

#[test]
fn request_id_echo_is_preserved_across_the_hop() {
    // The id allocated by the UI must be byte-identical after a Sync round-trip
    // (the whole point of the correlation token).
    let id = next_request();
    let cmd = SyncCommand::Rest {
        id,
        cmd: RestCommand::InviteToDocument {
            document_id: Id::<Document>::mint(),
            invitee_email: Some(ambre_bus::Email::from("x@y.z")),
            invitee: Id::<User>::mint(),
            role: Role::Editor,
            expect_exchange_pub_key: ambre_core::FixedBytes::from_array([0u8; 32]),
            now: Instant::<Authored>::new(0),
        },
    };
    let bytes = postcard::to_allocvec(&cmd).unwrap();
    let SyncCommand::Rest { id: back, .. } = postcard::from_bytes(&bytes).unwrap() else {
        panic!("variant changed");
    };
    assert_eq!(id, back);
}

/// A registry entry whose every byte is `fill`; no test checks its signature.
fn an_entry(fill: u8) -> ambre_core::PeerEntry {
    ambre_core::PeerEntry {
        user: Id::<User>::from_bytes([fill; 16]),
        nonce: ambre_core::FixedBytes::from_array([fill; 16]),
        signature: ambre_core::FixedBytes::from_array([fill; 64]),
    }
}

/// `BUS-046`: every error an event of the bus carries is a closed set. Each
/// type is `Copy`, so no variant of it can own a `String`, and a new text
/// field fails to compile here. The events name these types alone, and the
/// exhaustive round-trip above names every event.
#[test]
fn every_error_of_the_bus_is_a_closed_set() {
    fn closed<T: Copy + std::fmt::Debug>(_: T) {}
    closed(ambre_bus::ApiFailure::Local(
        ambre_core::LocalFailure::PinMismatch(Id::<User>::mint()),
    ));
    closed(ambre_bus::AuthFailure::Api(ambre_bus::ApiFailure::NoAnswer));
    closed(OpenFailure::Transient);
    closed(ambre_bus::sync_warning::SyncWarningCode::KeyNotHeld);
    closed(ambre_core::ConverterRefusal::DigestMismatch);
    closed(ambre_bus::FatalCause::DoorbellTaken);
    closed(ambre_core::SchemaRefusal::VersionUnreadable);
    closed(ambre_bus::interop::InteropError::Stopped(
        ambre_core::StopCause::UploadFailed(ambre_bus::ApiFailure::NoAnswer),
    ));
    closed(ambre_bus::interop::InteropError::ParseFailed(
        ambre_core::FileRefusal::Malformed,
    ));
}

/// `SYNC-212`: a key no member signed crosses the hop as a failure of its own.
/// Neither the frame nor the words read as a key still to arrive.
#[test]
fn a_key_no_member_signed_crosses_the_bus_as_its_own_failure() {
    let doc = Id::<Document>::mint();
    let failed = |cause| LogicEvent::DocumentOpenFailed {
        document_id: doc,
        cause,
    };
    let unsigned = failed(OpenFailure::KeyUnsigned);
    let bytes = postcard::to_allocvec(&unsigned).expect("encode LogicEvent");
    let back: LogicEvent = postcard::from_bytes(&bytes).expect("decode LogicEvent");
    assert!(matches!(
        back,
        LogicEvent::DocumentOpenFailed {
            cause: OpenFailure::KeyUnsigned,
            ..
        }
    ));
    let waiting = postcard::to_allocvec(&failed(OpenFailure::KeyNotReceived)).expect("encode");
    assert_ne!(bytes, waiting);
    assert_ne!(
        OpenFailure::KeyUnsigned.to_string(),
        OpenFailure::KeyNotReceived.to_string()
    );
}

/// A kept log whose every field holds a distinct value, so a codec that
/// drops or swaps a field fails the round trip.
fn kept_log() -> ambre_bus::KeptLog {
    ambre_bus::KeptLog {
        anchor: ambre_core::FixedBytes::from_array([9; 32]),
        head: ambre_bus::LogHead {
            position: ambre_core::Seq::new(3),
            previous: Some(ambre_core::FixedBytes::from_array([10; 32])),
        },
        key: ambre_core::FixedBytes::from_array([11; 16]),
        links: vec![ambre_protocol::KeyLink {
            document_id: Id::<Document>::from_bytes([12; 16]),
            key: ambre_core::FixedBytes::from_array([11; 16]),
            previous: ambre_core::FixedBytes::from_array([13; 16]),
            head: ambre_bus::LogHead {
                position: ambre_core::Seq::new(2),
                previous: Some(ambre_core::FixedBytes::from_array([14; 32])),
            },
            sealed: ambre_core::Payload::new(vec![15; 40]),
        }],
    }
}

/// The fields of a kept log as the store wrote them before it kept the
/// chain.
#[derive(serde::Serialize)]
struct KeptLogBeforeTheChain {
    anchor: ambre_core::FixedBytes<ambre_core::EntryDigest>,
    head: ambre_bus::LogHead,
    key: ambre_core::FixedBytes<ambre_core::KeyId>,
}

/// `SYNC-290`: a kept log reads back from the row the local store keeps,
/// and bytes that are no kept log read as none.
#[test]
fn a_kept_log_reads_back_from_its_row() {
    let kept = kept_log();
    let row = kept.to_row().unwrap();
    assert_eq!(ambre_bus::KeptLog::from_row(&row), Some(kept));
    let damaged = ambre_core::Payload::<ambre_bus::KeptLogRow>::new(vec![1]);
    assert_eq!(ambre_bus::KeptLog::from_row(&damaged), None);
}

/// `SYNC-308`: the row keeps the chain beside the head, so a reload reads
/// both back. A row of the form before the chain reads as none. So does a
/// row with bytes past the kept log. The store drops either, and the next
/// open fetches the links again.
#[test]
fn a_kept_chain_reads_back_and_an_older_row_reads_as_none() {
    let kept = kept_log();
    let row = kept.to_row().unwrap();
    let read = ambre_bus::KeptLog::from_row(&row).unwrap();
    assert_eq!(read.links, kept.links);
    let older = KeptLogBeforeTheChain {
        anchor: kept.anchor,
        head: kept.head,
        key: kept.key,
    };
    let older =
        ambre_core::Payload::<ambre_bus::KeptLogRow>::new(postcard::to_allocvec(&older).unwrap());
    assert_eq!(ambre_bus::KeptLog::from_row(&older), None);
    let mut longer = row.as_bytes().to_vec();
    longer.push(0);
    let longer = ambre_core::Payload::<ambre_bus::KeptLogRow>::new(longer);
    assert_eq!(ambre_bus::KeptLog::from_row(&longer), None);
}
