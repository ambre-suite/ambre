//! The send window through the Logic thread (`SYNC-192` to `SYNC-196`). The
//! tests read what a pass hands the Sync worker, on a simulated clock and on
//! the thread's own.

use super::*;
use crate::app::AppCommand;
use ambre_bus::{DocType, RestCommand};
use ambre_core::index::Pos;
use ambre_prose::message::{BlockId, DocChar, ProseCommand};
use ambre_trame::message::TrameCommand;
use ambre_trame::{Col, Row};

/// A state with one open Prose document, its opening sends drained.
struct Rig {
    state: LogicState,
    sync: Receiver<SyncCommand>,
    persist: Receiver<(Id<Document>, PersistWrite)>,
    _ui: Receiver<UiEvent>,
    document_id: Id<Document>,
    block: BlockId,
}

fn rig() -> Rig {
    let (to_ui, ui) = test_link();
    let (to_sync, sync) = test_link();
    let (to_persist, persist) = test_link();
    let mut state = LogicState::new(to_ui, to_sync, to_persist);
    state.clock = SendClock::At(at(0.0));
    let document_id = Id::<Document>::mint();
    state.open_with_identity(document_id, DocType::Prose);
    state.answer_pass();
    let block = match &state.docs[&document_id].state {
        DocState::Prose(prose) => prose.root_blocks()[0],
        other => panic!("the registry opened {other:?}"),
    };
    let mut rig = Rig {
        state,
        sync,
        persist,
        _ui: ui,
        document_id,
        block,
    };
    // A first key: the first save is a snapshot, and the window it opens
    // ends before the test begins.
    rig.pass_at(1_000.0, 1);
    rig.sync.try_iter().for_each(drop);
    rig.persist.try_iter().for_each(drop);
    rig
}

fn at(ms: f64) -> Instant<PageLoad> {
    Instant::<PageLoad>::new(ms)
}

fn keystroke(document_id: Id<Document>, block: BlockId, pos: u32) -> LogicCommand {
    LogicCommand::App {
        document_id,
        command: AppCommand::Prose(ProseCommand::InsertText {
            block_id: block,
            pos: Pos::<DocChar>::new(pos),
            text: "a".to_owned(),
        }),
    }
}

impl Rig {
    /// One pass at `ms` that applies `keys` keystrokes, then answers.
    fn pass_at(&mut self, ms: f64, keys: u32) {
        self.state.clock = SendClock::At(at(ms));
        let typed = self.typed();
        for pos in typed..typed + keys {
            let key = keystroke(self.document_id, self.block, pos);
            self.state.handle_logic_command(key);
        }
        self.state.answer_pass();
    }

    /// The characters the document holds.
    fn typed(&self) -> u32 {
        match &self.state.docs.get(&self.document_id).map(|d| &d.state) {
            Some(DocState::Prose(prose)) => prose
                .resolved_runs(&self.block)
                .iter()
                .map(|(run, _)| run.chars().count())
                .sum::<usize>()
                .try_into()
                .expect("a short text"),
            _ => 0,
        }
    }

    /// The deltas handed to the Sync worker since the last call.
    fn sent(&self) -> Vec<ambre_core::Payload<ambre_core::DeltaBytes>> {
        self.sync
            .try_iter()
            .filter_map(|cmd| match cmd {
                SyncCommand::SendUpdate { payload, .. } => Some(payload),
                _ => None,
            })
            .collect()
    }
}

/// The run of operations one delta, or several end to end, carry.
fn run_of(deltas: &[ambre_core::Payload<ambre_core::DeltaBytes>]) -> (i32, i32) {
    let metas: Vec<_> = deltas
        .iter()
        .map(|d| loro::LoroDoc::decode_import_blob_meta(d.as_bytes(), false).expect("a delta"))
        .collect();
    let start = metas
        .first()
        .and_then(|m| m.partial_start_vv.iter().next().map(|(_, c)| *c))
        .unwrap_or(0);
    let end = metas
        .last()
        .and_then(|m| m.partial_end_vv.iter().next().map(|(_, c)| *c))
        .expect("an end");
    (start, end)
}

/// `SYNC-194`: three keys of one pass leave as one delta, which carries the
/// operations of the three deltas saved for them.
#[test]
fn the_keys_of_one_pass_leave_as_one_delta() {
    let mut rig = rig();
    rig.pass_at(2_000.0, 3);
    let sent = rig.sent();
    assert_eq!(sent.len(), 1, "one delta leaves for the three keys");
    let saved: Vec<_> = rig
        .persist
        .try_iter()
        .filter_map(|(_, w)| match w {
            PersistWrite::Delta(delta) => Some(delta),
            _ => None,
        })
        .collect();
    assert_eq!(saved.len(), 3, "each key is saved as it came");
    assert_eq!(run_of(&sent), run_of(&saved), "the delta carries every key");
}

/// `SYNC-192`: inside a window no delta leaves. The keys of the passes the
/// window spans leave together at its end, which opens the next window.
#[test]
fn a_burst_of_passes_sends_once_per_window() {
    let mut rig = rig();
    rig.pass_at(2_000.0, 1);
    assert_eq!(rig.sent().len(), 1, "the first key leaves at once");
    rig.pass_at(2_005.0, 1);
    rig.pass_at(2_010.0, 1);
    rig.pass_at(2_024.9, 0);
    assert!(rig.sent().is_empty(), "nothing leaves inside the window");
    rig.pass_at(2_025.0, 0);
    let sent = rig.sent();
    assert_eq!(sent.len(), 1, "the two held keys leave as one delta");
    rig.pass_at(2_030.0, 1);
    assert!(rig.sent().is_empty(), "the release opened the next window");
    rig.pass_at(2_050.0, 0);
    assert_eq!(rig.sent().len(), 1);
}

/// `SYNC-196`: a key the window holds is saved in its own pass.
#[test]
fn a_held_delta_is_saved_in_its_own_pass() {
    let mut rig = rig();
    rig.pass_at(2_000.0, 1);
    rig.sent();
    rig.persist.try_iter().for_each(drop);
    rig.pass_at(2_005.0, 1);
    assert!(rig.sent().is_empty(), "the window holds the key");
    let saved = rig
        .persist
        .try_iter()
        .filter(|(_, w)| matches!(w, PersistWrite::Delta(_)))
        .count();
    assert_eq!(saved, 1, "the held key is saved all the same");
}

/// `SYNC-195`: a close inside the window sends the held key, then closes.
#[test]
fn a_close_sends_the_held_delta_first() {
    let mut rig = rig();
    rig.pass_at(2_000.0, 1);
    rig.sent();
    rig.pass_at(2_005.0, 1);
    let document_id = rig.document_id;
    rig.state.clock = SendClock::At(at(2_006.0));
    rig.state
        .handle_logic_command(LogicCommand::CloseDocument { document_id });
    let sent: Vec<_> = rig.sync.try_iter().collect();
    let update = sent
        .iter()
        .position(|c| matches!(c, SyncCommand::SendUpdate { .. }));
    let close = sent.iter().position(|c| {
        matches!(
            c,
            SyncCommand::ReleaseDocument { .. } | SyncCommand::CloseDocument { .. }
        )
    });
    assert!(
        matches!((update, close), (Some(u), Some(c)) if u < c),
        "the held key leaves before the close: {sent:?}"
    );
}

/// `SYNC-195`: a sign-out inside the window sends the held key, then the
/// sign-out.
#[test]
fn a_sign_out_sends_the_held_delta_first() {
    let mut rig = rig();
    rig.pass_at(2_000.0, 1);
    rig.sent();
    rig.pass_at(2_005.0, 1);
    rig.state.handle_logic_command(LogicCommand::Rest {
        id: ambre_bus::next_request(),
        cmd: RestCommand::Logout,
    });
    let sent: Vec<_> = rig.sync.try_iter().collect();
    assert!(
        matches!(
            sent.as_slice(),
            [
                SyncCommand::SendUpdate { .. },
                SyncCommand::Rest {
                    cmd: RestCommand::Logout,
                    ..
                }
            ]
        ),
        "the held key leaves before the sign-out: {sent:?}"
    );
}

/// `SYNC-193`: on the thread's own clock, an edit held by the window leaves
/// once the window ends, with no message to wake the thread. Trame, since its
/// commands name a cell rather than a block the view reads from the layout.
#[test]
fn a_held_delta_leaves_with_no_message() {
    let bell = Doorbell::new();
    let slots = ambre_core::Bounded::<ambre_bus::conduit::ChannelSlots>::of::<64>();
    let (to_logic, from_ui) = ambre_bus::conduit::to_worker::<LogicCommand>(slots, &bell);
    let (to_logic_events, from_sync) = ambre_bus::conduit::to_worker::<LogicEvent>(slots, &bell);
    let (to_ui, _ui) = test_link();
    let (to_sync, sync) = test_link();
    let (to_persist, _persist) = test_link();
    let (converter, _ends) = converter_thread::converter_channels(&bell);
    // The first spawn hands the link; a spawn after it gives none.
    let first = std::cell::Cell::new(Some(converter));
    let converter = converter_thread::ConverterThreads::boot(
        converter_thread::ConverterSpawner::new(move || first.take()),
    );
    let thread_bell = bell.clone();
    let thread = std::thread::spawn(move || {
        run(
            thread_bell,
            from_ui,
            to_ui,
            to_sync,
            from_sync,
            to_persist,
            converter,
        );
    });

    let document_id = Id::<Document>::mint();
    to_logic
        .try_send(LogicCommand::OpenDocument {
            document_id,
            doc_type: DocType::Trame,
        })
        .expect("queued");
    let peer = test_peer();
    to_logic_events
        .try_send(LogicEvent::SessionIdentity {
            document_id,
            peer,
            entry: test_entry(peer),
            nonce: FixedBytes::from_array([0; 16]),
        })
        .expect("queued");
    let cell = |row: u32| LogicCommand::App {
        document_id,
        command: AppCommand::Trame(TrameCommand::SetCellText {
            row: Pos::<Row>::new(row),
            col: Pos::<Col>::new(0),
            text: "kept".to_owned(),
        }),
    };

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut updates = Vec::new();
    let wait_for = |count: usize, updates: &mut Vec<_>| {
        while updates.len() < count && std::time::Instant::now() < deadline {
            match sync.try_recv() {
                Ok(SyncCommand::SendUpdate { payload, .. }) => updates.push(payload),
                Ok(_) => {}
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(1)),
            }
        }
    };
    to_logic.try_send(cell(0)).expect("queued");
    wait_for(1, &mut updates);
    // The first edit opened a window: the second lands inside it, and no
    // message follows it.
    to_logic.try_send(cell(1)).expect("queued");
    let sent = std::time::Instant::now();
    wait_for(2, &mut updates);
    let waited = sent.elapsed();

    drop(to_logic);
    drop(to_logic_events);
    let _ = thread.join();
    assert_eq!(updates.len(), 2, "the held edit left with no message");
    assert!(
        waited < std::time::Duration::from_secs(2),
        "the held edit waited {waited:?}"
    );
}
