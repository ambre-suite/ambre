// ambre-sync/tests/integration.rs
// ============================================================================
// Integration tests for the SYNC P1 pipeline — session lifecycle, document CRUD,
// multi-document open/close, and mock WebSocket transport verification.

use ambre_bus::{DocType, LogicEvent, RequestId, RestCommand, RestEvent, SyncCommand};
use ambre_sync::mock::MockWsTransport;

fn drain_rest(rx: &flume::Receiver<LogicEvent>, expected_id: RequestId) -> RestEvent {
    let events: Vec<LogicEvent> = rx.try_iter().collect();
    for evt in events {
        if let LogicEvent::Rest { id, evt } = evt {
            assert_eq!(id, expected_id, "RequestId mismatch");
            return evt;
        }
    }
    panic!("no Rest event found for RequestId {expected_id:?}");
}

/// Build a SyncState + mock transport + logic_rx for a test.
fn setup(
    with_mock: bool,
) -> (
    ambre_sync::SyncState,
    flume::Receiver<LogicEvent>,
    MockWsTransport,
) {
    let (logic_sink, logic_rx) = ambre_sync::LogicSink::channel();
    let mut state = ambre_sync::SyncState::new(logic_sink, String::new(), None);
    let mock = MockWsTransport::new();
    if with_mock {
        state.set_mock_ws(mock.client_tx.clone());
    }
    (state, logic_rx, mock)
}

fn send_rest(state: &mut ambre_sync::SyncState, id: RequestId, cmd: RestCommand) {
    state.handle_command(SyncCommand::Rest { id, cmd });
}

// ─────────────────────────────────────────────────────────────────────────────
// Session lifecycle
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn unlock_returns_session_ready() {
    let (mut state, rx, _mock) = setup(false);
    let id = RequestId::next();

    send_rest(
        &mut state,
        id,
        RestCommand::Unlock {
            passphrase: b"secret".to_vec(),
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
    let id = RequestId::next();

    send_rest(
        &mut state,
        id,
        RestCommand::SetupAccount {
            passphrase: b"secret".to_vec(),
        },
    );

    match drain_rest(&rx, id) {
        RestEvent::AccountCreated { recovery_words } => {
            assert!(
                !recovery_words.is_empty(),
                "recovery words must not be empty"
            );
            assert_eq!(recovery_words.len(), 64, "32-byte hex-encoded string");
        }
        other => panic!("expected AccountCreated, got {other:?}"),
    }
}

#[test]
fn logout_returns_logged_out_and_clears_session() {
    let (mut state, rx, _mock) = setup(false);

    // Unlock first to populate session keys.
    send_rest(
        &mut state,
        RequestId::next(),
        RestCommand::Unlock {
            passphrase: b"secret".to_vec(),
        },
    );
    rx.try_iter().count(); // drain SessionReady

    let id = RequestId::next();
    send_rest(&mut state, id, RestCommand::Logout);

    match drain_rest(&rx, id) {
        RestEvent::LoggedOut => {}
        other => panic!("expected LoggedOut, got {other:?}"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Document CRUD
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn create_document_returns_created() {
    let (mut state, rx, _mock) = setup(false);
    let id = RequestId::next();

    send_rest(
        &mut state,
        id,
        RestCommand::CreateDocument {
            doc_type: DocType::Trame,
            name: "My Sheet".into(),
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

    let create_id = RequestId::next();
    send_rest(
        &mut state,
        create_id,
        RestCommand::CreateDocument {
            doc_type: DocType::Trame,
            name: "My Sheet".into(),
        },
    );
    let created_doc_id = match drain_rest(&rx, create_id) {
        RestEvent::DocumentCreated { document_id, .. } => document_id,
        other => panic!("expected DocumentCreated, got {other:?}"),
    };

    let list_id = RequestId::next();
    send_rest(&mut state, list_id, RestCommand::ListDocuments);

    match drain_rest(&rx, list_id) {
        RestEvent::DocumentList { documents } => {
            assert_eq!(documents.len(), 1);
            assert_eq!(documents[0].document_id, created_doc_id);
            assert_eq!(documents[0].name.as_deref(), Some("My Sheet"));
        }
        other => panic!("expected DocumentList, got {other:?}"),
    }
}

#[test]
fn delete_document_removes_from_list() {
    let (mut state, rx, _mock) = setup(false);

    let create_id = RequestId::next();
    send_rest(
        &mut state,
        create_id,
        RestCommand::CreateDocument {
            doc_type: DocType::Trame,
            name: "Temp".into(),
        },
    );
    let doc_id = match drain_rest(&rx, create_id) {
        RestEvent::DocumentCreated { document_id, .. } => document_id,
        other => panic!("expected DocumentCreated, got {other:?}"),
    };

    let del_id = RequestId::next();
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
    let list_id = RequestId::next();
    send_rest(&mut state, list_id, RestCommand::ListDocuments);
    match drain_rest(&rx, list_id) {
        RestEvent::DocumentList { documents } => assert!(documents.is_empty()),
        other => panic!("expected DocumentList, got {other:?}"),
    }
}

#[test]
fn rename_document_updates_name() {
    let (mut state, rx, _mock) = setup(false);

    let create_id = RequestId::next();
    send_rest(
        &mut state,
        create_id,
        RestCommand::CreateDocument {
            doc_type: DocType::Trame,
            name: "Old".into(),
        },
    );
    let doc_id = match drain_rest(&rx, create_id) {
        RestEvent::DocumentCreated { document_id, .. } => document_id,
        other => panic!("expected DocumentCreated, got {other:?}"),
    };

    let rename_id = RequestId::next();
    send_rest(
        &mut state,
        rename_id,
        RestCommand::RenameDocument {
            document_id: doc_id,
            name: "New".into(),
        },
    );

    match drain_rest(&rx, rename_id) {
        RestEvent::DocumentRenamed { document_id } => {
            assert_eq!(document_id, doc_id);
        }
        other => panic!("expected DocumentRenamed, got {other:?}"),
    }

    // Confirm name changed.
    let list_id = RequestId::next();
    send_rest(&mut state, list_id, RestCommand::ListDocuments);
    match drain_rest(&rx, list_id) {
        RestEvent::DocumentList { documents } => {
            assert_eq!(documents.len(), 1);
            assert_eq!(documents[0].name.as_deref(), Some("New"));
        }
        other => panic!("expected DocumentList, got {other:?}"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Multi-document lifecycle (open / close)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn open_document_subscribes_via_mock_ws() {
    let (mut state, rx, mock) = setup(true);

    // Unlock first (required for ws_connected).
    send_rest(
        &mut state,
        RequestId::next(),
        RestCommand::Unlock {
            passphrase: b"secret".to_vec(),
        },
    );
    rx.try_iter().count(); // drain SessionReady

    let doc_id = ambre_core::DocumentId::new();
    state.handle_command(SyncCommand::OpenDocument {
        document_id: doc_id,
        doc_type: DocType::Trame,
    });

    // No Rest event is expected (OpenDocument is fire-and-forget).
    assert!(rx.try_iter().count() == 0);

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

#[test]
fn close_document_unsubscribes_via_mock_ws() {
    let (mut state, rx, mock) = setup(true);

    // Unlock first.
    send_rest(
        &mut state,
        RequestId::next(),
        RestCommand::Unlock {
            passphrase: b"secret".to_vec(),
        },
    );
    rx.try_iter().count();

    let doc_id = ambre_core::DocumentId::new();
    state.handle_command(SyncCommand::OpenDocument {
        document_id: doc_id,
        doc_type: DocType::Trame,
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

#[test]
fn send_update_after_open_document_encrypts_and_sends() {
    let (mut state, rx, mock) = setup(true);

    // Unlock first.
    send_rest(
        &mut state,
        RequestId::next(),
        RestCommand::Unlock {
            passphrase: b"secret".to_vec(),
        },
    );
    rx.try_iter().count();

    let doc_id = ambre_core::DocumentId::new();
    state.handle_command(SyncCommand::OpenDocument {
        document_id: doc_id,
        doc_type: DocType::Trame,
    });
    mock.drain_client_msgs(); // drain Subscribe

    // A freshly opened doc is local-only; the direct-send path requires it to
    // exist server-side (otherwise the first update is buffered behind a lazy
    // create). Establish that precondition explicitly.
    state.mark_document_server_created(doc_id);

    state.handle_command(SyncCommand::SendUpdate {
        document_id: doc_id,
        payload: b"hello world".to_vec(),
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

// ─────────────────────────────────────────────────────────────────────────────
// Full session flow (end-to-end smoke test)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn full_session_flow_unlock_create_open_close_logout() {
    let (mut state, rx, mock) = setup(true);

    // 1. Unlock
    let id1 = RequestId::next();
    send_rest(
        &mut state,
        id1,
        RestCommand::Unlock {
            passphrase: b"pass".to_vec(),
        },
    );
    match drain_rest(&rx, id1) {
        RestEvent::SessionReady { .. } => {}
        other => panic!("expected SessionReady, got {other:?}"),
    }

    // 2. Create a document
    let id2 = RequestId::next();
    send_rest(
        &mut state,
        id2,
        RestCommand::CreateDocument {
            doc_type: DocType::Trame,
            name: "Budget".into(),
        },
    );
    let doc_id = match drain_rest(&rx, id2) {
        RestEvent::DocumentCreated { document_id, .. } => document_id,
        other => panic!("expected DocumentCreated, got {other:?}"),
    };

    // 3. List documents
    let id3 = RequestId::next();
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
    });
    let open_msgs = mock.drain_client_msgs();
    assert!(
        open_msgs
            .iter()
            .any(|m| matches!(m, ambre_protocol::ws::WsClientMsg::Subscribe { .. })),
        "OpenDocument should send Subscribe"
    );

    // 5. Send an update. The doc is local-only in this synchronous test, so mark
    // it server-created first to exercise the direct-send path (lazy creation is
    // covered separately and needs an async runtime).
    state.mark_document_server_created(doc_id);
    state.handle_command(SyncCommand::SendUpdate {
        document_id: doc_id,
        payload: b"data".to_vec(),
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
    let id4 = RequestId::next();
    send_rest(&mut state, id4, RestCommand::Logout);
    match drain_rest(&rx, id4) {
        RestEvent::LoggedOut => {}
        other => panic!("expected LoggedOut, got {other:?}"),
    }

    // After logout, documents should be empty.
    assert_eq!(state.documents_len(), 0);
}
