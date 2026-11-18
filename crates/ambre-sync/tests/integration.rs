// ambre-sync/tests/integration.rs
// ============================================================================
// Integration tests for the SYNC P1 pipeline -- session lifecycle, document CRUD,
// multi-document open/close, and mock WebSocket transport verification.

#![expect(
    clippy::panic,
    reason = "a test asserts by panicking (ARCH-028 holds outside the tests)"
)]

use ambre_bus::{DocType, LogicEvent, Request, RestCommand, RestEvent, SyncCommand, next_request};
use ambre_core::Seq;
use ambre_core::id::{Document, Id};
use ambre_sync::mock::MockWsTransport;

fn drain_rest(
    rx: &ambre_bus::conduit::Receiver<LogicEvent>,
    expected_id: Seq<Request>,
) -> RestEvent {
    let events: Vec<LogicEvent> = rx.try_iter().collect();
    for evt in events {
        if let LogicEvent::Rest { id, evt } = evt {
            assert_eq!(id, Some(expected_id), "request id mismatch");
            return evt;
        }
    }
    panic!("no Rest event found for request {expected_id:?}");
}

/// Build a SyncState + mock transport + logic_rx for a test.
fn setup(
    with_mock: bool,
) -> (
    ambre_sync::SyncState,
    ambre_bus::conduit::Receiver<LogicEvent>,
    MockWsTransport,
) {
    let (logic_sink, logic_rx) = ambre_sync::LogicSink::channel();
    let mut state = ambre_sync::SyncState::new(
        logic_sink,
        ambre_sync::Url::<ambre_sync::RelayHost>::new(String::new()),
    );
    let mock = MockWsTransport::new();
    if with_mock {
        state.set_mock_ws(mock.client_tx.clone());
    }
    (state, logic_rx, mock)
}

fn send_rest(state: &mut ambre_sync::SyncState, id: Seq<Request>, cmd: RestCommand) {
    state.handle_command(SyncCommand::Rest { id, cmd });
}

// -----------------------------------------------------------------------------
// Session lifecycle
// -----------------------------------------------------------------------------

// Signs in by a bare name, which only a `dev-login` build holds (`ZK-045`).
#[cfg(feature = "dev-login")]
#[test]
fn dev_login_returns_session_ready() {
    let (mut state, rx, _mock) = setup(false);
    let id = next_request();

    send_rest(
        &mut state,
        id,
        RestCommand::DevLogin {
            username: ambre_core::Validated::<ambre_core::DisplayName>::parse("ada")
                .unwrap_or_else(|_| panic!("a valid name")),
        },
    );

    match drain_rest(&rx, id) {
        RestEvent::SessionReady { .. } => {}
        other => panic!("expected SessionReady, got {other:?}"),
    }
}

#[test]
fn setup_account_returns_created_with_recovery_words() {
    let (mut state, rx, _mock) = setup(false);
    let id = next_request();

    send_rest(
        &mut state,
        id,
        RestCommand::SetupAccount {
            passphrase: ambre_core::Confidential::<ambre_core::Passphrase>::new(
                "anchovy glider ruckus tadpole".to_owned(),
            ),
            email: "alice@example.com".into(),
            display_name: ambre_core::Validated::<ambre_core::DisplayName>::parse("Alice").unwrap(),
        },
    );

    match drain_rest(&rx, id) {
        RestEvent::AccountCreated { recovery_words } => {
            // `ZK-043`: 24 BIP39 words, which read back to a seed.
            let words = recovery_words.secret().expose();
            assert_eq!(words.split(' ').count(), 24, "24 words");
            assert!(ambre_crypto::keys::MasterSeed::from_recovery_words(&recovery_words).is_ok());
        }
        other => panic!("expected AccountCreated, got {other:?}"),
    }
}

#[test]
fn a_setup_under_the_strength_floor_is_refused_before_any_key_exists() {
    // `ZK-035`: a weak passphrase seals no seed. The worker answers the
    // weakness alone: no account, no recovery words, no socket.
    for (weak, weakness) in [
        ("secret", ambre_core::PassphraseWeakness::CommonPassword),
        (
            "crane motion velvet",
            ambre_core::PassphraseWeakness::TooShort,
        ),
    ] {
        let (mut state, rx, mock) = setup(true);
        send_rest(
            &mut state,
            next_request(),
            RestCommand::SetupAccount {
                passphrase: ambre_core::Confidential::<ambre_core::Passphrase>::new(
                    weak.to_owned(),
                ),
                email: "alice@example.com".into(),
                display_name: ambre_core::Validated::<ambre_core::DisplayName>::parse("Alice")
                    .unwrap(),
            },
        );
        let events: Vec<LogicEvent> = rx.try_iter().collect();
        assert!(
            matches!(
                events.as_slice(),
                [LogicEvent::AuthError { cause: ambre_bus::AuthFailure::WeakPassphrase(w) }]
                    if *w == weakness
            ),
            "{weak}: {events:?}"
        );
        assert!(mock.server_rx.try_recv().is_err(), "{weak}: no frame left");
    }
}

/// Send `cmd` to a fresh worker. Its one answer is the refusal under the
/// strength floor, with `weakness`, and no frame leaves.
fn assert_refused_under_the_floor(cmd: RestCommand, weakness: ambre_core::PassphraseWeakness) {
    let (mut state, rx, mock) = setup(true);
    send_rest(&mut state, next_request(), cmd);
    let events: Vec<LogicEvent> = rx.try_iter().collect();
    assert!(
        matches!(
            events.as_slice(),
            [LogicEvent::AuthError { cause: ambre_bus::AuthFailure::WeakPassphrase(w) }]
                if *w == weakness
        ),
        "{weakness:?}: {events:?}"
    );
    assert!(
        mock.server_rx.try_recv().is_err(),
        "{weakness:?}: no frame left"
    );
}

/// The two weak passphrases of the floor tests, each with the weakness that
/// dominates its estimate.
const WEAK_PASSPHRASES: [(&str, ambre_core::PassphraseWeakness); 2] = [
    ("secret", ambre_core::PassphraseWeakness::CommonPassword),
    (
        "crane motion velvet",
        ambre_core::PassphraseWeakness::TooShort,
    ),
];

#[test]
fn a_passphrase_change_under_the_strength_floor_is_refused() {
    // `ZK-042`: the new passphrase of a change takes the floor of `ZK-035`.
    // The current passphrase is a strong one, so the refusal is the new one's.
    for (weak, weakness) in WEAK_PASSPHRASES {
        assert_refused_under_the_floor(
            RestCommand::ChangePassphrase {
                current_passphrase: ambre_core::Confidential::<ambre_core::Passphrase>::new(
                    "anchovy glider ruckus tadpole".to_owned(),
                ),
                new_passphrase: ambre_core::Confidential::<ambre_core::Passphrase>::new(
                    weak.to_owned(),
                ),
            },
            weakness,
        );
    }
}

#[test]
fn a_recovery_under_the_strength_floor_is_refused() {
    // `ZK-042`: the new passphrase of a recovery takes the floor of `ZK-035`.
    for (weak, weakness) in WEAK_PASSPHRASES {
        assert_refused_under_the_floor(
            RestCommand::RecoverAccount {
                recovery_words: valid_recovery_words(),
                new_passphrase: ambre_core::Confidential::<ambre_core::Passphrase>::new(
                    weak.to_owned(),
                ),
            },
            weakness,
        );
    }
}

#[test]
fn a_change_or_a_recovery_above_the_strength_floor_draws_no_refusal() {
    // `ZK-042`: the floor refuses the weak passphrase alone. A strong one
    // passes it, so no `WeakPassphrase` answers either command.
    let strong = || {
        ambre_core::Confidential::<ambre_core::Passphrase>::new(
            "anchovy glider ruckus tadpole".to_owned(),
        )
    };
    for cmd in [
        RestCommand::ChangePassphrase {
            current_passphrase: strong(),
            new_passphrase: strong(),
        },
        RestCommand::RecoverAccount {
            recovery_words: valid_recovery_words(),
            new_passphrase: strong(),
        },
    ] {
        let (mut state, rx, _mock) = setup(true);
        send_rest(&mut state, next_request(), cmd);
        let events: Vec<LogicEvent> = rx.try_iter().collect();
        assert!(
            !events.iter().any(|e| matches!(
                e,
                LogicEvent::AuthError {
                    cause: ambre_bus::AuthFailure::WeakPassphrase(_)
                }
            )),
            "{events:?}"
        );
    }
}

/// The recovery words of the all-zero seed, the first BIP39 vector.
fn valid_recovery_words() -> ambre_core::Confidential<ambre_core::RecoveryWords> {
    recovery_words(&format!("{}art", "abandon ".repeat(23)))
}

fn recovery_words(text: &str) -> ambre_core::Confidential<ambre_core::RecoveryWords> {
    ambre_core::Confidential::<ambre_core::RecoveryWords>::new(text.to_owned())
}

#[test]
fn a_recovery_with_a_mistyped_word_or_a_bad_checksum_is_refused() {
    // `ZK-043`: the words are read whole before any key is derived. The
    // refusal is the one event the worker sends, and no frame leaves.
    for (text, refusal) in [
        (
            format!("{}art", "abandon ".repeat(22) + "abandn "),
            ambre_core::RecoveryWordsRefusal::UnknownWord,
        ),
        (
            "abandon ".repeat(24).trim_end().to_owned(),
            ambre_core::RecoveryWordsRefusal::BadChecksum,
        ),
        (
            "00".repeat(32),
            ambre_core::RecoveryWordsRefusal::WrongWordCount,
        ),
    ] {
        let (mut state, rx, mock) = setup(true);
        send_rest(
            &mut state,
            next_request(),
            RestCommand::RecoverAccount {
                recovery_words: recovery_words(&text),
                new_passphrase: ambre_core::Confidential::<ambre_core::Passphrase>::new(
                    "anchovy glider ruckus tadpole".to_owned(),
                ),
            },
        );
        let events: Vec<LogicEvent> = rx.try_iter().collect();
        assert!(
            matches!(
                events.as_slice(),
                [LogicEvent::AuthError { cause: ambre_bus::AuthFailure::BadRecoveryWords(r) }]
                    if *r == refusal
            ),
            "{refusal:?}: {events:?}"
        );
        assert!(
            mock.server_rx.try_recv().is_err(),
            "{refusal:?}: no frame left"
        );
    }
}

#[test]
fn a_recovery_with_valid_words_goes_on_to_the_relay() {
    // `ZK-043`: the words of a seed read back, so no refusal of the words
    // answers them. The re-seal needs the relay, which an offline worker
    // lacks, so that is the one answer, and no frame leaves (`SYNC-248`).
    let (mut state, rx, mock) = setup(true);
    send_rest(
        &mut state,
        next_request(),
        RestCommand::RecoverAccount {
            recovery_words: valid_recovery_words(),
            new_passphrase: ambre_core::Confidential::<ambre_core::Passphrase>::new(
                "anchovy glider ruckus tadpole".to_owned(),
            ),
        },
    );
    let events: Vec<LogicEvent> = rx.try_iter().collect();
    assert!(
        matches!(
            events.as_slice(),
            [LogicEvent::AuthError {
                cause: ambre_bus::AuthFailure::Api(ambre_bus::ApiFailure::Local(
                    ambre_core::LocalFailure::NoRelay
                ))
            }]
        ),
        "{events:?}"
    );
    assert!(mock.server_rx.try_recv().is_err(), "no frame left");
}

// Signs in by a bare name, which only a `dev-login` build holds (`ZK-045`).
#[cfg(feature = "dev-login")]
#[test]
fn logout_returns_logged_out_and_clears_session() {
    let (mut state, rx, _mock) = setup(false);

    // Sign in first to populate session keys.
    send_rest(
        &mut state,
        next_request(),
        RestCommand::DevLogin {
            username: ambre_core::Validated::<ambre_core::DisplayName>::parse("ada")
                .unwrap_or_else(|_| panic!("a valid name")),
        },
    );
    rx.try_iter().count(); // drain SessionReady

    let id = next_request();
    send_rest(&mut state, id, RestCommand::Logout);

    match drain_rest(&rx, id) {
        RestEvent::LoggedOut => {}
        other => panic!("expected LoggedOut, got {other:?}"),
    }
}

// -----------------------------------------------------------------------------
// Document CRUD
// -----------------------------------------------------------------------------

#[test]
fn create_document_returns_created() {
    let (mut state, rx, _mock) = setup(false);
    let id = next_request();

    send_rest(
        &mut state,
        id,
        RestCommand::CreateDocument {
            doc_type: DocType::Trame,
            name: ambre_core::Validated::<ambre_core::Name>::parse("My Sheet").unwrap(),
        },
    );

    match drain_rest(&rx, id) {
        RestEvent::DocumentCreated {
            document_id: _,
            doc_type,
        } => {
            assert_eq!(doc_type, DocType::Trame);
        }
        other => panic!("expected DocumentCreated, got {other:?}"),
    }
}

#[test]
fn list_documents_includes_created() {
    let (mut state, rx, _mock) = setup(false);

    let create_id = next_request();
    send_rest(
        &mut state,
        create_id,
        RestCommand::CreateDocument {
            doc_type: DocType::Trame,
            name: ambre_core::Validated::<ambre_core::Name>::parse("My Sheet").unwrap(),
        },
    );
    let created_doc_id = match drain_rest(&rx, create_id) {
        RestEvent::DocumentCreated { document_id, .. } => document_id,
        other => panic!("expected DocumentCreated, got {other:?}"),
    };

    let list_id = next_request();
    send_rest(&mut state, list_id, RestCommand::ListDocuments);

    match drain_rest(&rx, list_id) {
        RestEvent::DocumentList { documents } => {
            assert_eq!(documents.len(), 1);
            assert_eq!(documents[0].document_id, created_doc_id);
            assert_eq!(
                documents[0]
                    .name
                    .as_ref()
                    .map(ambre_core::Validated::as_str),
                Some("My Sheet")
            );
        }
        other => panic!("expected DocumentList, got {other:?}"),
    }
}

#[test]
fn delete_document_removes_from_list() {
    let (mut state, rx, _mock) = setup(false);

    let create_id = next_request();
    send_rest(
        &mut state,
        create_id,
        RestCommand::CreateDocument {
            doc_type: DocType::Trame,
            name: ambre_core::Validated::<ambre_core::Name>::parse("Temp").unwrap(),
        },
    );
    let doc_id = match drain_rest(&rx, create_id) {
        RestEvent::DocumentCreated { document_id, .. } => document_id,
        other => panic!("expected DocumentCreated, got {other:?}"),
    };

    let del_id = next_request();
    send_rest(
        &mut state,
        del_id,
        RestCommand::DeleteDocument {
            document_id: doc_id,
        },
    );

    match drain_rest(&rx, del_id) {
        RestEvent::DocumentDeleted { document_id } => {
            assert_eq!(document_id, doc_id);
        }
        other => panic!("expected DocumentDeleted, got {other:?}"),
    }

    // Confirm it's gone.
    let list_id = next_request();
    send_rest(&mut state, list_id, RestCommand::ListDocuments);
    match drain_rest(&rx, list_id) {
        RestEvent::DocumentList { documents } => assert!(documents.is_empty()),
        other => panic!("expected DocumentList, got {other:?}"),
    }
}

#[test]
fn rename_document_updates_name() {
    let (mut state, rx, _mock) = setup(false);

    let create_id = next_request();
    send_rest(
        &mut state,
        create_id,
        RestCommand::CreateDocument {
            doc_type: DocType::Trame,
            name: ambre_core::Validated::<ambre_core::Name>::parse("Old").unwrap(),
        },
    );
    let doc_id = match drain_rest(&rx, create_id) {
        RestEvent::DocumentCreated { document_id, .. } => document_id,
        other => panic!("expected DocumentCreated, got {other:?}"),
    };

    let rename_id = next_request();
    send_rest(
        &mut state,
        rename_id,
        RestCommand::RenameDocument {
            document_id: doc_id,
            name: ambre_core::Validated::<ambre_core::Name>::parse("New").unwrap(),
        },
    );

    match drain_rest(&rx, rename_id) {
        RestEvent::DocumentRenamed { document_id } => {
            assert_eq!(document_id, doc_id);
        }
        other => panic!("expected DocumentRenamed, got {other:?}"),
    }

    // Confirm name changed.
    let list_id = next_request();
    send_rest(&mut state, list_id, RestCommand::ListDocuments);
    match drain_rest(&rx, list_id) {
        RestEvent::DocumentList { documents } => {
            assert_eq!(documents.len(), 1);
            assert_eq!(
                documents[0]
                    .name
                    .as_ref()
                    .map(ambre_core::Validated::as_str),
                Some("New")
            );
        }
        other => panic!("expected DocumentList, got {other:?}"),
    }
}

// -----------------------------------------------------------------------------
// Multi-document lifecycle (open / close)
// -----------------------------------------------------------------------------

// Signs in by a bare name, which only a `dev-login` build holds (`ZK-045`).
#[cfg(feature = "dev-login")]
#[test]
fn open_document_subscribes_via_mock_ws() {
    let (mut state, rx, mock) = setup(true);

    // Sign in first (required for ws_connected).
    send_rest(
        &mut state,
        next_request(),
        RestCommand::DevLogin {
            username: ambre_core::Validated::<ambre_core::DisplayName>::parse("ada")
                .unwrap_or_else(|_| panic!("a valid name")),
        },
    );
    rx.try_iter().count(); // drain SessionReady

    let doc_id = Id::<Document>::mint();
    state.handle_command(SyncCommand::OpenDocument {
        document_id: doc_id,
        doc_type: DocType::Trame,
        meta: None,
    });

    // No Rest event is expected (OpenDocument is fire-and-forget). The one
    // event Logic receives is the session identity of the opened document.
    let events: Vec<LogicEvent> = rx.try_iter().collect();
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events.first(),
        Some(LogicEvent::SessionIdentity { document_id, .. }) if *document_id == doc_id
    ));

    // The mock WS must have received a Subscribe message.
    let msgs = mock.drain_client_msgs();
    assert_eq!(msgs.len(), 1);
    match &msgs[0] {
        ambre_protocol::ws::WsClientMsg::Subscribe { document_id } => {
            assert_eq!(*document_id, doc_id);
        }
        other => panic!("expected Subscribe, got {other:?}"),
    }
}

// Signs in by a bare name, which only a `dev-login` build holds (`ZK-045`).
#[cfg(feature = "dev-login")]
#[test]
fn close_document_unsubscribes_via_mock_ws() {
    let (mut state, rx, mock) = setup(true);

    // Sign in first.
    send_rest(
        &mut state,
        next_request(),
        RestCommand::DevLogin {
            username: ambre_core::Validated::<ambre_core::DisplayName>::parse("ada")
                .unwrap_or_else(|_| panic!("a valid name")),
        },
    );
    rx.try_iter().count();

    let doc_id = Id::<Document>::mint();
    state.handle_command(SyncCommand::OpenDocument {
        document_id: doc_id,
        doc_type: DocType::Trame,
        meta: None,
    });
    mock.drain_client_msgs(); // drain Subscribe

    state.handle_command(SyncCommand::CloseDocument {
        document_id: doc_id,
    });

    let msgs = mock.drain_client_msgs();
    assert_eq!(msgs.len(), 1);
    match &msgs[0] {
        ambre_protocol::ws::WsClientMsg::Unsubscribe { document_id } => {
            assert_eq!(*document_id, doc_id);
        }
        other => panic!("expected Unsubscribe, got {other:?}"),
    }
}

// Signs in by a bare name, which only a `dev-login` build holds (`ZK-045`).
#[cfg(feature = "dev-login")]
#[test]
fn send_update_after_open_document_encrypts_and_sends() {
    let (mut state, rx, mock) = setup(true);

    // Sign in first.
    send_rest(
        &mut state,
        next_request(),
        RestCommand::DevLogin {
            username: ambre_core::Validated::<ambre_core::DisplayName>::parse("ada")
                .unwrap_or_else(|_| panic!("a valid name")),
        },
    );
    rx.try_iter().count();

    let doc_id = Id::<Document>::mint();
    state.handle_command(SyncCommand::OpenDocument {
        document_id: doc_id,
        doc_type: DocType::Trame,
        meta: None,
    });
    mock.drain_client_msgs(); // drain Subscribe

    // A freshly opened doc is local-only; the direct-send path requires it to
    // exist server-side (otherwise the first update is buffered behind a lazy
    // create). Establish that precondition explicitly.
    state.mark_document_server_created(doc_id);

    state.handle_command(SyncCommand::SendUpdate {
        document_id: doc_id,
        payload: b"hello world".to_vec().into(),
        summary: None,
    });

    let msgs = mock.drain_client_msgs();
    assert_eq!(msgs.len(), 1, "one Update message sent via WS");
    match &msgs[0] {
        ambre_protocol::ws::WsClientMsg::Update(update) => {
            assert_eq!(update.public_data.document_id, doc_id);
        }
        other => panic!("expected Update, got {other:?}"),
    }
}

// -----------------------------------------------------------------------------
// Full session flow (end-to-end smoke test)
// -----------------------------------------------------------------------------

// Signs in by a bare name, which only a `dev-login` build holds (`ZK-045`).
#[cfg(feature = "dev-login")]
#[test]
fn full_session_flow_sign_in_create_open_close_logout() {
    let (mut state, rx, mock) = setup(true);

    // 1. Sign in: an offline unlock opens no key (`SYNC-270`).
    let id1 = next_request();
    send_rest(
        &mut state,
        id1,
        RestCommand::DevLogin {
            username: ambre_core::Validated::<ambre_core::DisplayName>::parse("ada")
                .unwrap_or_else(|_| panic!("a valid name")),
        },
    );
    match drain_rest(&rx, id1) {
        RestEvent::SessionReady { .. } => {}
        other => panic!("expected SessionReady, got {other:?}"),
    }

    // 2. Create a document
    let id2 = next_request();
    send_rest(
        &mut state,
        id2,
        RestCommand::CreateDocument {
            doc_type: DocType::Trame,
            name: ambre_core::Validated::<ambre_core::Name>::parse("Budget").unwrap(),
        },
    );
    let doc_id = match drain_rest(&rx, id2) {
        RestEvent::DocumentCreated { document_id, .. } => document_id,
        other => panic!("expected DocumentCreated, got {other:?}"),
    };

    // 3. List documents
    let id3 = next_request();
    send_rest(&mut state, id3, RestCommand::ListDocuments);
    match drain_rest(&rx, id3) {
        RestEvent::DocumentList { documents } => {
            assert_eq!(documents.len(), 1);
            assert_eq!(documents[0].document_id, doc_id);
        }
        other => panic!("expected DocumentList, got {other:?}"),
    };

    // Drain any Subscribe from create/open combo.
    mock.drain_client_msgs();

    // 4. Open the document
    state.handle_command(SyncCommand::OpenDocument {
        document_id: doc_id,
        doc_type: DocType::Trame,
        meta: None,
    });
    let open_msgs = mock.drain_client_msgs();
    assert!(
        open_msgs
            .iter()
            .any(|m| matches!(m, ambre_protocol::ws::WsClientMsg::Subscribe { .. })),
        "OpenDocument should send Subscribe"
    );

    // 5. Send an update. The doc is local-only in this synchronous test, so mark
    // it server-created first to exercise the direct-send path. Lazy creation is
    // covered separately and needs an async runtime.
    state.mark_document_server_created(doc_id);
    state.handle_command(SyncCommand::SendUpdate {
        document_id: doc_id,
        payload: b"data".to_vec().into(),
        summary: None,
    });
    let update_msgs = mock.drain_client_msgs();
    assert_eq!(update_msgs.len(), 1, "SendUpdate should send one Update");

    // 6. Close the document
    state.handle_command(SyncCommand::CloseDocument {
        document_id: doc_id,
    });
    let close_msgs = mock.drain_client_msgs();
    assert!(
        close_msgs
            .iter()
            .any(|m| matches!(m, ambre_protocol::ws::WsClientMsg::Unsubscribe { .. })),
        "CloseDocument should send Unsubscribe"
    );

    // 7. Logout
    let id4 = next_request();
    send_rest(&mut state, id4, RestCommand::Logout);
    match drain_rest(&rx, id4) {
        RestEvent::LoggedOut => {}
        other => panic!("expected LoggedOut, got {other:?}"),
    }

    // After logout, documents should be empty.
    assert_eq!(state.documents_len().get(), 0);
}

// -----------------------------------------------------------------------------
// Passphrase-sealed files (`DOCFILE P2`)
//
// These run the real Argon2id work factor, which is the point. The seam under
// test is the service plane, and a test key derivation would not exercise it.
// Each one costs a few hundred milliseconds, so there are as few as the rules
// need.
// -----------------------------------------------------------------------------

#[test]
fn a_sealed_file_round_trips_through_the_service_plane() {
    let (mut state, rx, _mock) = setup(false);
    let doc_id = Id::<Document>::mint();
    let plaintext = ambre_client_core::write_ambre_plaintext(
        DocType::Prose,
        doc_id,
        &ambre_core::Payload::new(b"body".to_vec()),
    );

    let seal_id = next_request();
    send_rest(
        &mut state,
        seal_id,
        RestCommand::SealDocFile {
            plaintext: plaintext.as_bytes().to_vec().into(),
            passphrase: ambre_core::Confidential::<ambre_core::Passphrase>::new(
                "correct horse".to_owned(),
            ),
        },
    );
    let sealed = match drain_rest(&rx, seal_id) {
        RestEvent::DocFileSealed { data } => data.into_bytes(),
        other => panic!("expected DocFileSealed, got {other:?}"),
    };

    // The sealed bytes are a container of the other lineage, and they do not
    // carry the plaintext header the plaintext reader would otherwise find.
    assert!(matches!(
        ambre_client_core::classify(&ambre_core::Payload::<ambre_core::PickedContainer>::new(
            sealed.clone()
        )),
        Ok(ambre_client_core::ContainerKind::Sealed(version))
            if version == ambre_client_core::AMBRE_SEALED_VERSION
    ));
    // Read as a plaintext container, the same bytes are refused.
    let misread = ambre_core::Payload::<ambre_core::PlaintextContainer>::new(sealed.clone());
    assert!(ambre_client_core::read_ambre_plaintext(&misread).is_err());

    let open_id = next_request();
    send_rest(
        &mut state,
        open_id,
        RestCommand::OpenDocFile {
            sealed: ambre_core::Payload::new(sealed),
            passphrase: ambre_core::Confidential::<ambre_core::Passphrase>::new(
                "correct horse".to_owned(),
            ),
        },
    );
    match drain_rest(&rx, open_id) {
        // What comes back is the whole plaintext container, byte for byte, so
        // the caller reads it exactly as it reads a file off the disk.
        RestEvent::DocFileOpened { plaintext: opened } => {
            let opened = opened.into_plaintext();
            assert_eq!(opened, plaintext.as_bytes());
            let opened = ambre_core::Payload::<ambre_core::PlaintextContainer>::new(opened);
            let header =
                ambre_client_core::read_ambre_plaintext(&opened).expect("plaintext header");
            assert_eq!(header.doc_type, DocType::Prose);
            assert_eq!(header.document_id, doc_id);
        }
        other => panic!("expected DocFileOpened, got {other:?}"),
    }
}

#[test]
fn a_wrong_passphrase_and_a_tampered_file_fail_the_same_way() {
    let (mut state, rx, _mock) = setup(false);
    let plaintext = ambre_client_core::write_ambre_plaintext(
        DocType::Prose,
        Id::<Document>::mint(),
        &ambre_core::Payload::new(b"body".to_vec()),
    );

    let seal_id = next_request();
    send_rest(
        &mut state,
        seal_id,
        RestCommand::SealDocFile {
            plaintext: plaintext.into_bytes().into(),
            passphrase: ambre_core::Confidential::<ambre_core::Passphrase>::new(
                "correct horse".to_owned(),
            ),
        },
    );
    let sealed = match drain_rest(&rx, seal_id) {
        RestEvent::DocFileSealed { data } => data.into_bytes(),
        other => panic!("expected DocFileSealed, got {other:?}"),
    };

    for (case, bytes, passphrase) in [
        ("wrong passphrase", sealed.clone(), "correct horsf"),
        (
            "tampered ciphertext",
            {
                let mut t = sealed.clone();
                let last = t.len() - 1;
                t[last] ^= 0x01;
                t
            },
            "correct horse",
        ),
        (
            "tampered salt",
            {
                let mut t = sealed.clone();
                t[5] ^= 0x01;
                t
            },
            "correct horse",
        ),
    ] {
        let id = next_request();
        send_rest(
            &mut state,
            id,
            RestCommand::OpenDocFile {
                sealed: ambre_core::Payload::new(bytes),
                passphrase: ambre_core::Confidential::<ambre_core::Passphrase>::new(
                    passphrase.to_owned(),
                ),
            },
        );
        match drain_rest(&rx, id) {
            RestEvent::DocFileOpenFailed => {}
            other => panic!("{case}: expected DocFileOpenFailed, got {other:?}"),
        }
    }
}

#[test]
fn opening_a_plaintext_file_fails_without_deriving_a_key() {
    let (mut state, rx, _mock) = setup(false);
    let plaintext = ambre_client_core::write_ambre_plaintext(
        DocType::Prose,
        Id::<Document>::mint(),
        &ambre_core::Payload::new(b"body".to_vec()),
    );

    // The header says plaintext, so the answer comes from the parse. Nothing
    // pays the work factor: an unsealed file is not a guessing attempt.
    let id = next_request();
    send_rest(
        &mut state,
        id,
        RestCommand::OpenDocFile {
            sealed: ambre_core::Payload::new(plaintext.into_bytes()),
            passphrase: ambre_core::Confidential::<ambre_core::Passphrase>::new(
                "anything".to_owned(),
            ),
        },
    );
    match drain_rest(&rx, id) {
        RestEvent::DocFileOpenFailed => {}
        other => panic!("expected DocFileOpenFailed, got {other:?}"),
    }
}
