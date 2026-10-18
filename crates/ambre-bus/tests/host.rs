// ambre-bus host tests (tier `host`, `ambre-spec-message-bus.md`).
// ============================================================================
// Pure, native (`cargo test -p ambre-bus`) coverage of the bus data plane:
//   - the two pure helpers carried on the content plane (`to_tsv`,
//     `has_decimals`),
//   - the `RequestId` allocator invariant (monotonic, shared `AtomicU64`),
//   - postcard round-trips of the Logic <-> Sync envelopes. The contract:
//     `SyncCommand` / `LogicEvent`, and the `Rest*` family they carry, survive the
//     `postMessage` hop unchanged, while application payloads stay `Vec<u8>`.
// No DOM, no serialization on the UI <-> Logic path (that link is `ambre_bus::conduit` by
// move and intentionally carries no `Serialize` derive).
// ============================================================================

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "a test asserts by panicking (ARCH-028 holds outside the tests)"
)]

use ambre_bus::app::toile::{ElementId, PresencePayload, ToileCommand, Viewport as ToileViewport};
use ambre_bus::{
    ActivityAnchor, ActivityKind, ActivitySummary, AppCommand, CellFormat, ChannelId, ChannelPlace,
    ClipboardCell, ClipboardPayload, DocType, DocumentId, FoldAnswer, LeaseAnswer, LeaseId,
    LogicEvent, MemberChannel, MemberInfo, MemberStanding, RequestId, RestCommand, RestEvent, Role,
    SelectionKind, SyncCommand, Timestamp, TrameCommand, UserId,
};

// -- Content plane: ClipboardPayload::to_tsv ---------------------------------

fn cell(dr: u32, dc: u32, raw: &str) -> ClipboardCell {
    ClipboardCell {
        dr,
        dc,
        raw: raw.to_string(),
        ..Default::default()
    }
}

#[test]
fn to_tsv_full_grid() {
    let payload = ClipboardPayload {
        rows: 2,
        cols: 2,
        cells: vec![
            cell(0, 0, "a"),
            cell(0, 1, "b"),
            cell(1, 0, "c"),
            cell(1, 1, "d"),
        ],
        ..Default::default()
    };
    assert_eq!(payload.to_tsv(), "a\tb\nc\td");
}

#[test]
fn to_tsv_gaps_render_as_empty_fields() {
    // Blank source cells are omitted from `cells`; they must appear as empty
    // fields, not collapse the row.
    let payload = ClipboardPayload {
        rows: 2,
        cols: 2,
        cells: vec![cell(0, 0, "a"), cell(1, 1, "d")],
        ..Default::default()
    };
    assert_eq!(payload.to_tsv(), "a\t\n\td");
}

#[test]
fn to_tsv_formula_cell_keeps_raw_text() {
    let payload = ClipboardPayload {
        rows: 1,
        cols: 1,
        cells: vec![cell(0, 0, "=SUM(A1:B1)")],
        ..Default::default()
    };
    assert_eq!(payload.to_tsv(), "=SUM(A1:B1)");
}

#[test]
fn to_tsv_zero_dimensions_clamp_to_one() {
    // `rows`/`cols` of 0 still produce a 1x1 grid (the `.max(1)`), never panic.
    let payload = ClipboardPayload::default();
    assert_eq!(payload.to_tsv(), "");
}

#[test]
fn to_tsv_out_of_bounds_offsets_are_ignored() {
    // A cell whose offset exceeds the declared range must not panic or grow the
    // grid; `get_mut` returns `None` and the cell is dropped.
    let payload = ClipboardPayload {
        rows: 1,
        cols: 1,
        cells: vec![cell(0, 0, "a"), cell(5, 5, "stray")],
        ..Default::default()
    };
    assert_eq!(payload.to_tsv(), "a");
}

// -- Content plane: CellFormat::has_decimals (exhaustive) ---------------------

#[test]
fn has_decimals_is_true_only_for_numeric_families() {
    for fmt in [
        CellFormat::Number,
        CellFormat::Currency,
        CellFormat::Percent,
        CellFormat::Scientific,
    ] {
        assert!(fmt.has_decimals(), "{fmt:?} should honour decimals");
    }
    for fmt in [
        CellFormat::General,
        CellFormat::Fraction,
        CellFormat::Date,
        CellFormat::Time,
        CellFormat::Text,
        CellFormat::Custom("dd/mm".to_string()),
    ] {
        assert!(!fmt.has_decimals(), "{fmt:?} should ignore decimals");
    }
}

// -- Service plane: RequestId allocator --------------------------------------

#[test]
fn request_id_is_strictly_monotonic() {
    let a = RequestId::next();
    let b = RequestId::next();
    let c = RequestId::next();
    assert!(a.0 < b.0, "ids must increase");
    assert!(b.0 < c.0);
}

#[test]
fn request_id_is_unique_under_concurrency() {
    use std::collections::HashSet;

    // Each thread keeps its own ids, so the test shares no lock with them.
    let handles: Vec<_> = (0..8)
        .map(|_| std::thread::spawn(|| (0..1000).map(|_| RequestId::next().0).collect::<Vec<_>>()))
        .collect();
    let mut seen = HashSet::new();
    for h in handles {
        for id in h.join().unwrap() {
            assert!(seen.insert(id), "duplicate id {id}");
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
    let doc = DocumentId::new();
    let chan = ChannelId::new();
    roundtrip_sync(&SyncCommand::Rest {
        id: RequestId(42),
        cmd: RestCommand::CreateDocument {
            doc_type: DocType::Trame,
            name: "Budget".to_string(),
        },
    });
    roundtrip_sync(&SyncCommand::OpenDocument {
        document_id: doc,
        doc_type: DocType::Trame,
    });
    // A genuine network-byte payload survives verbatim.
    roundtrip_sync(&SyncCommand::SendUpdate {
        document_id: doc,
        payload: vec![0, 1, 2, 250, 255],
        summary: None,
    });
    roundtrip_sync(&SyncCommand::SendUpdate {
        document_id: doc,
        payload: vec![7],
        summary: Some(ActivitySummary {
            kind: ActivityKind::Content,
            mentions: vec![UserId::new()],
            mentions_all: true,
            anchor: Some(ActivityAnchor(vec![1, 2])),
        }),
    });
    roundtrip_sync(&SyncCommand::SendUnsent {
        document_id: doc,
        payload: vec![3, 4],
        nonce: ambre_core::SessionNonce([5; 16]),
    });
    roundtrip_sync(&SyncCommand::OpenChannel {
        channel_id: chan,
        listed_in: Some(ambre_core::SpaceId::new()),
    });
    roundtrip_sync(&SyncCommand::Shutdown);
}

#[test]
fn logic_event_roundtrips_through_postcard() {
    let doc = DocumentId::new();
    let user = UserId::new();
    roundtrip_event(&LogicEvent::Rest {
        id: RequestId(7),
        evt: RestEvent::DocumentCreated {
            document_id: doc,
            doc_type: DocType::Trame,
        },
    });
    roundtrip_event(&LogicEvent::RemoteUpdate {
        document_id: doc,
        delta: ambre_bus::SignedDelta {
            seq: 8,
            payload: vec![3, 1, 4, 1, 5, 9],
            nonce: ambre_core::SessionNonce([2; 16]),
            peer: 7,
            user: Some(UserId::from_bytes([1; 16])),
            summary: Some(ambre_bus::ActivitySummary {
                kind: ambre_bus::ActivityKind::Content,
                mentions: vec![UserId::from_bytes([3; 16])],
                mentions_all: false,
                anchor: None,
            }),
        },
    });
    roundtrip_event(&LogicEvent::SessionIdentity {
        document_id: doc,
        peer: 7,
        entry: vec![8; 96],
        nonce: ambre_core::SessionNonce([9; 16]),
    });
    roundtrip_event(&LogicEvent::ConnectionStatus {
        document_id: doc,
        connected: true,
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
        SyncCommand::CloseDocument { .. } => "CloseDocument",
        SyncCommand::ReleaseDocument { .. } => "ReleaseDocument",
        SyncCommand::SendUpdate { .. } => "SendUpdate",
        SyncCommand::SendUnsent { .. } => "SendUnsent",
        SyncCommand::SendSnapshot { .. } => "SendSnapshot",
        SyncCommand::SnapshotVerdict { .. } => "SnapshotVerdict",
        SyncCommand::SendEphemeral { .. } => "SendEphemeral",
        SyncCommand::Reconnect { .. } => "Reconnect",
        SyncCommand::OpenChannel { .. } => "OpenChannel",
        SyncCommand::CreateChannel { .. } => "CreateChannel",
        SyncCommand::RotateSpace { .. } => "RotateSpace",
        SyncCommand::CloseSegment { .. } => "CloseSegment",
        SyncCommand::CreateAmendments { .. } => "CreateAmendments",
        SyncCommand::TakeFinalizeLease { .. } => "TakeFinalizeLease",
        SyncCommand::FinalizeSegment { .. } => "FinalizeSegment",
        SyncCommand::TakeFoldLease { .. } => "TakeFoldLease",
        SyncCommand::FoldSegment { .. } => "FoldSegment",
        SyncCommand::WatchSegment { .. } => "WatchSegment",
        SyncCommand::JoinChannel { .. } => "JoinChannel",
        SyncCommand::DeleteChannel { .. } => "DeleteChannel",
        SyncCommand::LeaveChannel { .. } => "LeaveChannel",
        SyncCommand::CreateDirectMessage { .. } => "CreateDirectMessage",
        SyncCommand::ListSpaceChannels { .. } => "ListSpaceChannels",
        SyncCommand::ListMemberChannels => "ListMemberChannels",
        SyncCommand::HoldInbound => "HoldInbound",
        SyncCommand::ReleaseInbound => "ReleaseInbound",
        SyncCommand::MarkRead { .. } => "MarkRead",
        SyncCommand::MoveReadCursors { .. } => "MoveReadCursors",
        SyncCommand::MarkAllRead => "MarkAllRead",
        SyncCommand::FetchActivity => "FetchActivity",
        SyncCommand::Shutdown => "Shutdown",
    }
}

/// The variant's name, with no wildcard arm, as for `sync_variant`.
fn event_variant(evt: &LogicEvent) -> &'static str {
    match evt {
        LogicEvent::Rest { .. } => "Rest",
        LogicEvent::SessionIdentity { .. } => "SessionIdentity",
        LogicEvent::RemoteUpdate { .. } => "RemoteUpdate",
        LogicEvent::RemoteUpdateBatch { .. } => "RemoteUpdateBatch",
        LogicEvent::RemoteSnapshot { .. } => "RemoteSnapshot",
        LogicEvent::PendingSnapshot { .. } => "PendingSnapshot",
        LogicEvent::SnapshotDropped { .. } => "SnapshotDropped",
        LogicEvent::RemoteEphemeral { .. } => "RemoteEphemeral",
        LogicEvent::PeerLeft { .. } => "PeerLeft",
        LogicEvent::MembersChanged { .. } => "MembersChanged",
        LogicEvent::DocumentReady { .. } => "DocumentReady",
        LogicEvent::ConnectionStatus { .. } => "ConnectionStatus",
        LogicEvent::Activity { .. } => "Activity",
        LogicEvent::Released { .. } => "Released",
        LogicEvent::SyncWarning { .. } => "SyncWarning",
        LogicEvent::SnapshotNeeded { .. } => "SnapshotNeeded",
        LogicEvent::MemberRevoked { .. } => "MemberRevoked",
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
        LogicEvent::ApiError { .. } => "ApiError",
        LogicEvent::SetupRequired => "SetupRequired",
        LogicEvent::UpdateRequired => "UpdateRequired",
    }
}

/// The number of arms of `sync_variant` and of `event_variant`.
const SYNC_VARIANTS: usize = 33;
const EVENT_VARIANTS: usize = 50;

/// `BUS-003`: every variant of both envelopes survives the hop, field for
/// field. Each arm of the two name functions needs a sample here, and the
/// samples cover each name exactly once.
#[test]
fn every_envelope_variant_roundtrips_through_postcard() {
    use ambre_bus::{
        ChannelDocument, ChannelDocumentKind, DirectMember, DocumentActivity, MeetingId, MetaId,
        SealedChannelKey, SegmentId, Seq, SignedDelta, SnapshotId, SnapshotPurpose,
        SnapshotVerdict, SpaceId,
    };
    use ambre_core::SessionNonce;
    use std::collections::BTreeSet;

    let doc = DocumentId::new();
    let chan = ChannelId::new();
    let space = SpaceId::new();
    let meta = MetaId::from_document(DocumentId::new());
    let segment = SegmentId::from_document(DocumentId::new());
    let user = UserId::new();
    let snapshot = SnapshotId::genesis(doc);
    let delta = SignedDelta {
        seq: 3,
        payload: vec![1, 2, 255],
        nonce: SessionNonce([4; 16]),
        peer: 9,
        user: Some(user),
        summary: None,
    };
    let commands = [
        SyncCommand::Rest {
            id: RequestId(5),
            cmd: RestCommand::CreateDocument {
                doc_type: DocType::Toile,
                name: "Plan".to_owned(),
            },
        },
        SyncCommand::OpenDocument {
            document_id: doc,
            doc_type: DocType::Prose,
        },
        SyncCommand::CloseDocument { document_id: doc },
        SyncCommand::ReleaseDocument {
            document_id: doc,
            applied_seq: 12,
        },
        SyncCommand::SendUpdate {
            document_id: doc,
            payload: vec![0, 7],
            summary: Some(ActivitySummary {
                kind: ActivityKind::Content,
                mentions: vec![user],
                mentions_all: false,
                anchor: Some(ActivityAnchor(vec![3])),
            }),
        },
        SyncCommand::SendUnsent {
            document_id: doc,
            payload: vec![8],
            nonce: SessionNonce([6; 16]),
        },
        SyncCommand::SendSnapshot {
            document_id: doc,
            payload: vec![1; 4],
            covered_seq: 40,
        },
        SyncCommand::SnapshotVerdict {
            document_id: doc,
            snapshot_id: snapshot,
            purpose: SnapshotPurpose::Confirm,
            verdict: SnapshotVerdict::Match {
                extras: vec![(41, vec![2, 3])],
            },
        },
        SyncCommand::SendEphemeral {
            document_id: doc,
            payload: vec![5],
        },
        SyncCommand::Reconnect { document_id: doc },
        SyncCommand::OpenChannel {
            channel_id: chan,
            listed_in: None,
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
            public: true,
        },
        SyncCommand::JoinChannel {
            channel_id: chan,
            space_id: space,
            key: SealedChannelKey::from_bytes(vec![6; 3]),
        },
        SyncCommand::CloseSegment {
            channel_id: chan,
            meta,
            closing: segment,
            successor: SegmentId::from_document(DocumentId::new()),
        },
        SyncCommand::CreateAmendments {
            channel_id: chan,
            meta,
            segment,
            amendments: DocumentId::new(),
        },
        SyncCommand::TakeFinalizeLease {
            channel_id: chan,
            segment,
        },
        SyncCommand::FinalizeSegment {
            channel_id: chan,
            segment,
            lease: LeaseId::new(),
            horizon: 42,
            payload: vec![4; 3],
        },
        SyncCommand::TakeFoldLease {
            channel_id: chan,
            segment,
        },
        SyncCommand::FoldSegment {
            channel_id: chan,
            segment,
            lease: LeaseId::new(),
            horizon: 42,
            amendments_seq: 7,
            payload: vec![5; 3],
        },
        SyncCommand::WatchSegment { segment, meta },
        SyncCommand::DeleteChannel { channel_id: chan },
        SyncCommand::LeaveChannel { channel_id: chan },
        SyncCommand::ListSpaceChannels { space_id: space },
        SyncCommand::ListMemberChannels,
        SyncCommand::CreateDirectMessage {
            channel_id: chan,
            meta,
            segment,
            members: vec![DirectMember {
                user_id: user,
                exchange_pub_key: [7; 32],
            }],
        },
        SyncCommand::HoldInbound,
        SyncCommand::ReleaseInbound,
        SyncCommand::MarkRead {
            live: vec![segment],
            finals: Vec::new(),
        },
        SyncCommand::MoveReadCursors {
            cursors: vec![(segment, Seq(4))],
        },
        SyncCommand::MarkAllRead,
        SyncCommand::FetchActivity,
        SyncCommand::Shutdown,
    ];
    let events = [
        LogicEvent::Rest {
            id: RequestId(6),
            evt: RestEvent::DocumentCreated {
                document_id: doc,
                doc_type: DocType::Toile,
            },
        },
        LogicEvent::SessionIdentity {
            document_id: doc,
            peer: 9,
            entry: vec![3; 8],
            nonce: SessionNonce([1; 16]),
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
            payload: vec![9; 3],
            covered_seq: 30,
        },
        LogicEvent::PendingSnapshot {
            document_id: doc,
            snapshot_id: snapshot,
            covered_seq: 31,
            payload: vec![4],
            purpose: SnapshotPurpose::Open,
        },
        LogicEvent::SnapshotDropped {
            document_id: doc,
            author: Some(user),
            reporter: None,
            refinalize: true,
        },
        LogicEvent::RemoteEphemeral {
            document_id: doc,
            from_user_id: user,
            payload: vec![2],
        },
        LogicEvent::PeerLeft {
            document_id: doc,
            user_id: user,
        },
        LogicEvent::MembersChanged {
            document_id: doc,
            members: vec![MemberInfo {
                user_id: user,
                display_name: "Ada".into(),
                email: "ada@example.org".into(),
                role: Role::Editor,
                standing: MemberStanding::Verified,
            }],
        },
        LogicEvent::DocumentReady {
            document_id: doc,
            role: Role::Viewer,
        },
        LogicEvent::ConnectionStatus {
            document_id: doc,
            connected: false,
        },
        LogicEvent::Activity {
            document_id: doc,
            counters: ambre_core::activity::ActivityCounters {
                content: 1,
                mentions: 2,
                ..Default::default()
            },
        },
        LogicEvent::Released { document_id: doc },
        LogicEvent::SyncWarning {
            document_id: doc,
            code: ambre_bus::sync_warning::SyncWarningCode::DecryptFailed,
            message: "dropped".to_owned(),
        },
        LogicEvent::SnapshotNeeded { document_id: doc },
        LogicEvent::MemberRevoked {
            document_id: doc,
            revoked_user_id: user,
        },
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
            message: "gone".to_owned(),
        },
        LogicEvent::ChannelRefused { channel_id: chan },
        LogicEvent::ChannelActivity {
            documents: vec![DocumentActivity {
                channel_id: chan,
                document_id: segment.document_id(),
                active: true,
                read: Seq(2),
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
            active: SegmentId::from_document(DocumentId::new()),
            created: false,
        },
        LogicEvent::SegmentCloseFailed {
            channel_id: chan,
            closing: segment,
            message: "refused".to_owned(),
        },
        LogicEvent::AmendmentsLinked {
            channel_id: chan,
            segment,
            amendments: DocumentId::new(),
        },
        LogicEvent::AmendmentsLinkFailed {
            channel_id: chan,
            segment,
            amendments: DocumentId::new(),
            message: "refused".to_owned(),
        },
        LogicEvent::FinalizeLeaseAnswered {
            channel_id: chan,
            segment,
            answer: LeaseAnswer::Granted {
                lease: LeaseId::new(),
                horizon: 42,
                expires_in: 30_000,
            },
        },
        LogicEvent::SegmentFinalized {
            channel_id: chan,
            segment,
        },
        LogicEvent::SegmentFinalizeFailed {
            channel_id: chan,
            segment,
            withdrawn: true,
            message: "refused".to_owned(),
        },
        LogicEvent::FoldLeaseAnswered {
            channel_id: chan,
            segment,
            answer: FoldAnswer::Granted {
                lease: LeaseId::new(),
                horizon: 42,
                amendments: DocumentId::new(),
                amendments_seq: 7,
                expires_in: 30_000,
            },
        },
        LogicEvent::SegmentFolded {
            channel_id: chan,
            segment,
        },
        LogicEvent::SegmentFoldFailed {
            channel_id: chan,
            segment,
            message: "refused".to_owned(),
        },
        LogicEvent::AmendmentsFolded {
            document_id: segment.document_id(),
            amendments: DocumentId::new(),
        },
        LogicEvent::DeltasLocked {
            document_id: segment.document_id(),
            deltas: vec![b"{}".to_vec()],
        },
        LogicEvent::ChannelJoined { channel_id: chan },
        LogicEvent::DirectMessageOpened {
            requested: chan,
            channel_id: ChannelId::new(),
        },
        LogicEvent::ChannelDeleted { channel_id: chan },
        LogicEvent::ChannelLeft { channel_id: chan },
        LogicEvent::ChannelRequestFailed {
            channel_id: chan,
            message: "refused".to_owned(),
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
                    channel_id: ChannelId::new(),
                    place: ChannelPlace::Direct,
                },
            ],
        },
        LogicEvent::MeetingParticipantJoined {
            meeting_id: MeetingId::new(),
            user_id: user,
        },
        LogicEvent::MeetingParticipantLeft {
            meeting_id: MeetingId::new(),
            user_id: user,
        },
        LogicEvent::DocumentOpenFailed {
            document_id: doc,
            message: "unwrap".to_owned(),
        },
        LogicEvent::AuthError {
            message: "expired".to_owned(),
        },
        LogicEvent::ApiError {
            id: Some(RequestId(8)),
            request: "share".to_owned(),
            code: 409,
            message: "conflict".to_owned(),
        },
        LogicEvent::SetupRequired,
        LogicEvent::UpdateRequired,
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

/// `BUS-001`: nothing on the UI to Logic path can be encoded. The four
/// messages that cross it implement no `Serialize`, so no encoder has a
/// value to take. The probe reads true for an envelope of the worker hop.
#[test]
fn the_ui_to_logic_path_carries_nothing_an_encoder_takes() {
    use ambre_bus::{AppEvent, LogicCommand, UiEvent};

    // Each probe is a constant, so a derive added tomorrow fails the build.
    const {
        assert!(!Encodable::<LogicCommand>::ENCODES);
        assert!(!Encodable::<AppCommand>::ENCODES);
        assert!(!Encodable::<UiEvent>::ENCODES);
        assert!(!Encodable::<AppEvent>::ENCODES);
        assert!(Encodable::<SyncCommand>::ENCODES);
    }
}

#[test]
fn request_id_echo_is_preserved_across_the_hop() {
    // The id allocated by the UI must be byte-identical after a Sync round-trip
    // (the whole point of the correlation token).
    let id = RequestId::next();
    let cmd = SyncCommand::Rest {
        id,
        cmd: RestCommand::InviteToDocument {
            document_id: DocumentId::new(),
            invitee_email: "x@y.z".to_string(),
            invitee: UserId::new(),
            role: Role::Editor,
            expect_exchange_pub_key: [0u8; 32],
            now: Timestamp(0),
        },
    };
    let bytes = postcard::to_allocvec(&cmd).unwrap();
    let SyncCommand::Rest { id: back, .. } = postcard::from_bytes(&bytes).unwrap() else {
        panic!("variant changed");
    };
    assert_eq!(id.0, back.0);
}

// -- Role gate: AppCommand::is_mutating (`architecture`) ---------
//
// The classification decides what a viewer may still do, and getting it wrong is silent in
// both directions. A misclassified read-only command costs a viewer its navigation, and a
// misclassified write slips past the client gate. The two ends of each family are pinned
// here rather than trusted to review.

#[test]
fn read_only_and_local_commands_are_not_mutating() {
    let cases: Vec<AppCommand> = vec![
        // Trame: local view state.
        AppCommand::Trame(TrameCommand::ScrollViewport {
            version: 1,
            row_start: 0,
            row_end: 4,
            col_start: 0,
            col_end: 4,
        }),
        AppCommand::Trame(TrameCommand::SetActiveCell {
            row: 0,
            col: 0,
            anchor_row: 0,
            anchor_col: 0,
            extra: vec![],
            kind: SelectionKind::Cells,
        }),
        AppCommand::Trame(TrameCommand::ActivateSheet {
            sheet_id: ambre_bus::SheetId("s1".to_owned()),
        }),
        // Trame: read-only queries.
        AppCommand::Trame(TrameCommand::RequestSelectionStats),
        AppCommand::Trame(TrameCommand::CopyRange),
        AppCommand::Trame(TrameCommand::ExportActiveSheetCsv),
        // Toile: local playback and view state.
        AppCommand::Toile(ToileCommand::NextSlide),
        AppCommand::Toile(ToileCommand::ExitPresent),
        AppCommand::Toile(ToileCommand::SetViewport(ToileViewport::default())),
        // Toile: pure reads a viewer must keep (`toile-comments`,
        // `toile-presentation`): both answer with a projection and write nothing.
        AppCommand::Toile(ToileCommand::ReadComments { thread: None }),
        AppCommand::Toile(ToileCommand::ReadDeck),
        // Toile: ephemeral only -- a viewer still shows a cursor (`toile-presence`).
        AppCommand::Toile(ToileCommand::Presence {
            payload: PresencePayload::default(),
            now_ms: 0,
        }),
    ];
    for cmd in cases {
        assert!(
            !cmd.is_mutating(),
            "a viewer must keep this command: {cmd:?}"
        );
    }
}

#[test]
fn writing_commands_are_mutating() {
    let cases: Vec<AppCommand> = vec![
        AppCommand::Trame(TrameCommand::SetCellText {
            row: 0,
            col: 0,
            text: "x".into(),
        }),
        AppCommand::Trame(TrameCommand::InsertRowAbove),
        AppCommand::Trame(TrameCommand::PasteText { text: "x".into() }),
        AppCommand::Trame(TrameCommand::Undo),
        AppCommand::Toile(ToileCommand::DeleteElements { ids: Vec::new() }),
        AppCommand::Toile(ToileCommand::DuplicateElements {
            ids: Vec::new(),
            delta: Default::default(),
        }),
        AppCommand::Toile(ToileCommand::Undo),
        // Reacting writes a per-user CRDT entry (`toile-reactions`): not presence.
        AppCommand::Toile(ToileCommand::React {
            target: ElementId::mint(1, 1),
            reaction: None,
        }),
    ];
    for cmd in cases {
        assert!(cmd.is_mutating(), "this command writes the CRDT: {cmd:?}");
    }
}
