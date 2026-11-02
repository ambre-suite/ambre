// ambre-bus host tests (tier `host`, `ambre-spec-message-bus.md`).
// ============================================================================
// Pure, native (`cargo test -p ambre-bus`) coverage of the bus data plane:
//   - the two pure helpers carried on the content plane (`to_tsv`,
//     `has_decimals`),
//   - the request id issuer invariant (monotonic, one shared issuer),
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

use ambre_bus::app::toile::{PresencePayload, ToileCommand, Viewport as ToileViewport};
use ambre_bus::next_request;
use ambre_bus::{
    ActivityAnchor, ActivityKind, ActivitySummary, AppCommand, Authored, CellFormat, ChannelMeta,
    ChannelName, ChannelPlace, ClipboardCell, ClipboardPayload, DirectPeers, DocType, FoldAnswer,
    Instant, LeaseAnswer, LogicEvent, MemberChannel, MemberInfo, MemberStanding, OpenFailure,
    RestCommand, RestEvent, Role, SelectionKind, SyncCommand, TrameCommand,
};
use ambre_bus::{Col, ColCount, Decimals, DecimalsStep, Row, RowCount};
use ambre_core::id::{
    Channel, Document, Element, Id, Lease, Meeting, Message, Meta, Segment, Snapshot, Space, User,
};
use ambre_core::{Board, Generation, Px};

// -- Content plane: ClipboardPayload::to_tsv ---------------------------------

fn cell(dr: u32, dc: u32, raw: &str) -> ClipboardCell {
    ClipboardCell {
        dr: RowCount::new(dr),
        dc: ColCount::new(dc),
        raw: raw.to_string(),
        ..Default::default()
    }
}

#[test]
fn to_tsv_full_grid() {
    let payload = ClipboardPayload {
        rows: RowCount::new(2),
        cols: ColCount::new(2),
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
        rows: RowCount::new(2),
        cols: ColCount::new(2),
        cells: vec![cell(0, 0, "a"), cell(1, 1, "d")],
        ..Default::default()
    };
    assert_eq!(payload.to_tsv(), "a\t\n\td");
}

#[test]
fn to_tsv_formula_cell_keeps_raw_text() {
    let payload = ClipboardPayload {
        rows: RowCount::new(1),
        cols: ColCount::new(1),
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
        rows: RowCount::new(1),
        cols: ColCount::new(1),
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

/// A decimal count holds between zero and fifteen places, stepped or decoded.
#[test]
fn a_decimal_count_stays_within_fifteen_places() {
    assert_eq!(Decimals::new(16), None);
    assert_eq!(Decimals::MAX.stepped(DecimalsStep::MORE), Decimals::MAX);
    assert_eq!(Decimals::ZERO.stepped(DecimalsStep::FEWER), Decimals::ZERO);
    assert_eq!(Decimals::TWO.stepped(DecimalsStep::MORE).get(), 3);
    let past: Result<Decimals, _> = postcard::from_bytes(&[16]);
    assert!(past.is_err(), "a decoded count past fifteen is refused");
    let bytes = postcard::to_allocvec(&Decimals::TWO).unwrap();
    assert_eq!(bytes, [2], "the count travels as one byte");
}

/// A line-height multiple is finite and above zero, and a stored number that
/// is not reads as no setting at all.
#[test]
fn a_line_height_multiple_is_finite_and_above_zero() {
    use ambre_bus::app::prose::{LineHeight, LineHeightMode, LineSpacingMultiple};
    for refused in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert_eq!(LineSpacingMultiple::new(refused), None);
        assert_eq!(
            LineHeight::from_stored(LineHeightMode::Multiple, refused),
            None
        );
    }
    for mode in [LineHeightMode::Exact, LineHeightMode::AtLeast] {
        for refused in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(LineHeight::from_stored(mode, refused), None, "{refused}");
        }
        assert!(LineHeight::from_stored(mode, 12.0).is_some());
    }
    let read = LineHeight::from_stored(LineHeightMode::Multiple, 1.5);
    assert_eq!(
        read,
        Some(LineHeight::Multiple(LineSpacingMultiple::ONE_AND_HALF))
    );
    assert_eq!(read.map(LineHeight::stored_value), Some(1.5));
}

/// A fraction of a side lies in 0..=1, and a decoded one outside it is
/// refused.
#[test]
fn a_fraction_of_a_side_lies_between_zero_and_one() {
    use ambre_bus::app::toile::UnitFraction;
    for refused in [-0.1, 1.1, f32::NAN] {
        assert_eq!(UnitFraction::new(refused), None);
    }
    assert_eq!(
        UnitFraction::HALF.along(Px::<Board>::new(10.0), Px::<Board>::new(30.0)),
        Px::<Board>::new(20.0)
    );
    let past: Result<UnitFraction, _> =
        postcard::from_bytes(&postcard::to_allocvec(&2.0_f32).unwrap());
    assert!(past.is_err(), "a decoded fraction past one is refused");
    let bytes = postcard::to_allocvec(&UnitFraction::HALF).unwrap();
    assert_eq!(
        bytes,
        postcard::to_allocvec(&0.5_f32).unwrap(),
        "it travels as an f32"
    );
}

// -- Service plane: request id issuer -----------------------------------------

#[test]
fn request_id_is_strictly_monotonic() {
    let a = next_request();
    let b = next_request();
    let c = next_request();
    assert!(Generation::ZERO < a, "no request carries the zero id");
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
        id: Generation::nth(42),
        cmd: RestCommand::CreateDocument {
            doc_type: DocType::Trame,
            name: ambre_core::Name::new("Budget").unwrap(),
        },
    });
    roundtrip_sync(&SyncCommand::OpenDocument {
        document_id: doc,
        doc_type: DocType::Trame,
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
    });
    roundtrip_sync(&SyncCommand::Shutdown);
}

#[test]
fn logic_event_roundtrips_through_postcard() {
    let doc = Id::<Document>::mint();
    let user = Id::<User>::mint();
    roundtrip_event(&LogicEvent::Rest {
        id: Some(Generation::nth(7)),
        evt: RestEvent::DocumentCreated {
            document_id: doc,
            doc_type: DocType::Trame,
        },
    });
    roundtrip_event(&LogicEvent::RemoteUpdate {
        document_id: doc,
        delta: ambre_bus::SignedDelta {
            seq: ambre_bus::Seq::new(8),
            payload: ambre_core::Payload::<ambre_core::DeltaBytes>::new(vec![3, 1, 4, 1, 5, 9]),
            nonce: ambre_core::FixedBytes::from_array([2; 16]),
            peer: ambre_core::PeerId::new(7),
            user: Some(Id::<User>::from_bytes([1; 16])),
            summary: Some(ambre_bus::ActivitySummary {
                kind: ambre_bus::ActivityKind::Content,
                mentions: vec![Id::<User>::from_bytes([3; 16])],
                mentions_all: ambre_bus::MentionScope::Named,
                anchor: None,
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
        LogicEvent::ThreadActivity { .. } => "ThreadActivity",
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
        LogicEvent::ApiError { .. } => "ApiError",
        LogicEvent::SetupRequired => "SetupRequired",
        LogicEvent::UpdateRequired => "UpdateRequired",
    }
}

/// The number of arms of `sync_variant` and of `event_variant`.
const SYNC_VARIANTS: usize = 39;
const EVENT_VARIANTS: usize = 55;

/// `BUS-003`: every variant of both envelopes survives the hop, field for
/// field. Each arm of the two name functions needs a sample here, and the
/// samples cover each name exactly once.
#[test]
fn every_envelope_variant_roundtrips_through_postcard() {
    use ambre_bus::{
        ChannelDocument, ChannelDocumentKind, DirectMember, DocumentActivity, SealedChannelKey,
        Seq, SignedDelta, SnapshotPurpose, SnapshotVerdict,
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
        seq: ambre_bus::Seq::new(3),
        payload: ambre_core::Payload::<ambre_core::DeltaBytes>::new(vec![1, 2, 255]),
        nonce: FixedBytes::from_array([4; 16]),
        peer: ambre_core::PeerId::new(9),
        user: Some(user),
        summary: None,
    };
    let commands = [
        SyncCommand::Rest {
            id: Generation::nth(5),
            cmd: RestCommand::CreateDocument {
                doc_type: DocType::Toile,
                name: ambre_core::Name::new("Plan").unwrap(),
            },
        },
        SyncCommand::OpenDocument {
            document_id: doc,
            doc_type: DocType::Prose,
        },
        SyncCommand::CloseDocument { document_id: doc },
        SyncCommand::ReleaseDocument {
            document_id: doc,
            applied_seq: ambre_bus::Seq::new(12),
        },
        SyncCommand::SendUpdate {
            document_id: doc,
            payload: ambre_core::Payload::<ambre_core::DeltaBytes>::new(vec![0, 7]),
            summary: Some(ActivitySummary {
                kind: ActivityKind::Content,
                mentions: vec![user],
                mentions_all: ambre_bus::MentionScope::Named,
                anchor: ActivityAnchor::new(vec![3]),
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
            covered_seq: ambre_bus::Seq::new(40),
        },
        SyncCommand::SnapshotVerdict {
            document_id: doc,
            snapshot_id: snapshot,
            purpose: SnapshotPurpose::Confirm,
            verdict: SnapshotVerdict::Match {
                extras: vec![(
                    ambre_core::PeerId::new(41),
                    ambre_core::PeerEntryBytes::new(vec![2, 3]),
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
            name: Some(ambre_core::Name::new("board").unwrap()),
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
            horizon: ambre_bus::Seq::new(42),
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
            horizon: ambre_bus::Seq::new(42),
            amendments_seq: ambre_bus::Seq::new(7),
            payload: ambre_core::Payload::<ambre_core::SnapshotBytes>::new(vec![5; 3]),
        },
        SyncCommand::WatchSegment { segment, meta },
        SyncCommand::WatchThread {
            thread: doc,
            meta,
            after: Seq::new(3),
        },
        SyncCommand::WatchChannel {
            channel_id: chan,
            meta,
            meta_last: Some(Seq::new(2)),
            documents: vec![(doc, Some(Seq::new(3))), (doc, None)],
        },
        SyncCommand::DeleteChannel { channel_id: chan },
        SyncCommand::LeaveChannel { channel_id: chan },
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
            cursors: vec![(segment, Seq::new(4))],
        },
        SyncCommand::MarkThreadRead { thread: doc },
        SyncCommand::MarkAllRead,
        SyncCommand::FetchActivity,
        SyncCommand::Shutdown,
    ];
    let events = [
        LogicEvent::Rest {
            id: Some(Generation::nth(6)),
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
            covered_seq: ambre_bus::Seq::new(30),
        },
        LogicEvent::PendingSnapshot {
            document_id: doc,
            snapshot_id: snapshot,
            covered_seq: ambre_bus::Seq::new(31),
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
                display_name: ambre_core::DisplayName::new("Ada").unwrap(),
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
            connected: ambre_core::LinkState::Disconnected,
        },
        LogicEvent::Activity {
            document_id: doc,
            counters: ambre_core::activity::ActivityCounters {
                content: ambre_core::activity::ContentUpdates::new(1),
                mentions: ambre_core::activity::MentionUpdates::new(2),
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
            message: "gone".to_owned(),
        },
        LogicEvent::ChannelRefused { channel_id: chan },
        LogicEvent::ChannelActivity {
            documents: vec![DocumentActivity {
                channel_id: chan,
                document_id: segment.document_id(),
                kind: ChannelDocumentKind::Segment,
                active: ambre_bus::Activity::Active,
                read: Seq::new(2),
                last: Some(Seq::new(5)),
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
            successor: ambre_bus::app::murmure::Successor::Fetched,
        },
        LogicEvent::SegmentCloseFailed {
            channel_id: chan,
            closing: segment,
            message: "refused".to_owned(),
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
            message: "refused".to_owned(),
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
            message: "refused".to_owned(),
        },
        LogicEvent::FinalizeLeaseAnswered {
            channel_id: chan,
            segment,
            answer: LeaseAnswer::Granted {
                lease: Id::<Lease>::mint(),
                horizon: ambre_bus::Seq::new(42),
                expires_in: ambre_core::TimeSpan::<ambre_core::Ms>::new(30_000),
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
            message: "refused".to_owned(),
        },
        LogicEvent::FoldLeaseAnswered {
            channel_id: chan,
            segment,
            answer: FoldAnswer::Granted {
                lease: Id::<Lease>::mint(),
                horizon: ambre_bus::Seq::new(42),
                amendments: Id::<Document>::mint(),
                amendments_seq: ambre_bus::Seq::new(7),
                expires_in: ambre_core::TimeSpan::<ambre_core::Ms>::new(30_000),
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
                    channel_id: direct,
                    place: ChannelPlace::Direct,
                },
            ],
            spaces: vec![space],
            names: vec![ChannelName {
                channel_id: chan,
                name: ambre_core::Name::new("board").unwrap(),
            }],
            peers: vec![DirectPeers {
                channel_id: direct,
                names: vec![ambre_core::DisplayName::new("Bob").unwrap()],
            }],
            metas: vec![ChannelMeta {
                channel_id: chan,
                meta: Id::<Meta>::from_document(doc),
                last: Some(Seq::new(5)),
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
            message: "unwrap".to_owned(),
        },
        LogicEvent::AuthError {
            message: "expired".to_owned(),
        },
        LogicEvent::ApiError {
            id: Some(Generation::nth(8)),
            request: ambre_bus::ApiRequest::InviteToDocument,
            failure: ambre_bus::ApiFailure::refused(ambre_bus::RefusalCode::MembersChanged),
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
            version: ambre_core::Generation::nth(1),
            row_start: Row::new(0),
            row_end: Row::new(4),
            col_start: Col::new(0),
            col_end: Col::new(4),
        }),
        AppCommand::Trame(TrameCommand::SetActiveCell {
            row: Row::new(0),
            col: Col::new(0),
            anchor_row: Row::new(0),
            anchor_col: Col::new(0),
            extra: vec![],
            kind: SelectionKind::Cells,
        }),
        AppCommand::Trame(TrameCommand::ActivateSheet {
            sheet_id: ambre_bus::SheetId::new("s1".to_owned()),
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
            now: ambre_core::Instant::<ambre_core::PageLoad>::new(0.0),
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
            row: Row::new(0),
            col: Col::new(0),
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
            target: Id::<Element>::mint_from(
                ambre_core::PeerId::new(1),
                ambre_bus::app::toile::IdCounter::new(1),
            ),
            reaction: None,
        }),
    ];
    for cmd in cases {
        assert!(cmd.is_mutating(), "this command writes the CRDT: {cmd:?}");
    }
}

/// A registry entry whose every byte is `fill`; no test checks its signature.
fn an_entry(fill: u8) -> ambre_core::PeerEntry {
    ambre_core::PeerEntry {
        user: Id::<User>::from_bytes([fill; 16]),
        nonce: ambre_core::FixedBytes::from_array([fill; 16]),
        signature: ambre_core::FixedBytes::from_array([fill; 64]),
    }
}
