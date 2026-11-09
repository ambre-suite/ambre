// ambre-logic/src/view_load.rs
// ============================================================================
// The load bench of a view's queue (`ambre-spec-budgets.md section 6`, the
// view backlog row). It answers one question: can the backlog of a view reach
// its bound in a realistic use?
//
// A view holds what its queue takes plus what its backlog holds, and drains
// both once per animation frame (`BUS-038`, `BUS-039`). The bound is thus
// a count of events between two frames of that view. This bench counts the
// events one stimulus sends to the view of its document, through the real
// dispatcher and the real end of pass. The budget row divides the bound by
// these counts. The counts are deterministic, so the bench is a test and runs
// on every `cargo test`.
//
// Print the table with:
//   cargo test -p ambre-logic view_load -- --nocapture
// ============================================================================

use ambre_bus::conduit::Doorbell;
use ambre_bus::conduit::{Receiver, Sender};
use ambre_bus::{Col, DocType, LogicEvent, Row};
use ambre_core::id::{Document, Id};
use ambre_core::index::Pos;
use ambre_prose::message::ProseCommand;
use ambre_toile::message::ToileCommand;
use ambre_trame::message::TrameCommand;

use crate::app::{AppCommand, LogicCommand, UiEvent};
use crate::{DocState, LogicState, test_link};

/// One open document, the queues a pass drains, and the queue its view reads.
struct Bench {
    state: LogicState,
    document_id: Id<Document>,
    to_logic: Sender<LogicCommand>,
    from_ui: Receiver<LogicCommand>,
    to_logic_events: Sender<LogicEvent>,
    from_sync: Receiver<LogicEvent>,
    ui: Receiver<UiEvent>,
}

impl Bench {
    /// A bench over one open document of `doc_type`, its opening answers drained.
    fn open(doc_type: DocType) -> Self {
        let (to_ui, ui) = test_link();
        let (to_persist, persist) = test_link::<(Id<Document>, crate::PersistWrite)>();
        // Local persistence runs on the Main thread, and no figure here reads it.
        std::mem::forget(persist);
        let mut state = LogicState::new(to_ui, test_link().0, to_persist);
        let document_id = Id::<Document>::mint();
        state.open_with_identity(document_id, doc_type);
        // The converter is loaded before any reading: the first fetch is a
        // one-time wait, not the cost of a conversion.
        state.handle_logic_event(LogicEvent::ConverterFetched {
            module: ambre_core::Payload::new(Vec::new()),
        });
        state.answer_pass();
        ui.try_iter().for_each(drop);
        let slots = ambre_core::Bounded::<ambre_bus::conduit::ChannelSlots>::of::<4096>();
        let (to_logic, from_ui) = ambre_bus::conduit::to_worker(slots, &Doorbell::new());
        let (to_logic_events, from_sync) = ambre_bus::conduit::to_worker(slots, &Doorbell::new());
        Self {
            state,
            document_id,
            to_logic,
            from_ui,
            to_logic_events,
            from_sync,
            ui,
        }
    }

    /// Run one pass, then every pass a sliced import still owes. Answers the
    /// events that reached the document's view, and the passes run.
    fn passes(&mut self) -> (usize, usize) {
        let mut passes = 1;
        self.state.drain_pass(&self.from_ui, &self.from_sync);
        while self.state.has_pending_work() {
            passes += 1;
            self.state.drain_pass(&self.from_ui, &self.from_sync);
        }
        let events = self
            .ui
            .try_iter()
            .filter(|ev| ev.routed_document_id() == Some(self.document_id))
            .count();
        (events, passes)
    }

    /// The events one pass, and its slices, sent to the document's view.
    fn pass(&mut self) -> usize {
        self.passes().0
    }

    fn command(&self, command: AppCommand) {
        self.to_logic
            .try_send(LogicCommand::App {
                document_id: self.document_id,
                command,
            })
            .expect("the bench queue has room");
    }

    fn event(&self, event: LogicEvent) {
        self.to_logic_events
            .try_send(event)
            .expect("the bench queue has room");
    }
}

/// A peer that writes one document of `doc_type`: its own engine, under a
/// session identity of its own.
enum Author {
    Trame(Box<ambre_trame::TrameState>),
    Toile(Box<ambre_toile::ToileState>),
    Prose(Box<ambre_prose::ProseState>),
}

impl Author {
    fn new(doc_type: DocType) -> Self {
        let peer = crate::test_peer();
        let entry = crate::test_entry(peer);
        match doc_type {
            DocType::Trame => {
                let mut state = ambre_trame::TrameState::new();
                assert!(state.adopt_session(peer, entry));
                Self::Trame(Box::new(state))
            }
            DocType::Toile => {
                let mut state = ambre_toile::ToileState::new();
                assert!(state.adopt_session(peer, entry));
                Self::Toile(Box::new(state))
            }
            DocType::Prose => {
                let mut state = ambre_prose::ProseState::new().expect("a Prose engine");
                assert!(state.adopt_session(peer, entry));
                Self::Prose(Box::new(state))
            }
            other => panic!("no engine for {other:?}"),
        }
    }

    /// The delta of the `i`-th small edit: a typed cell, a new sticky note, a
    /// typed word.
    fn edit(&mut self, i: u32) -> ambre_core::Payload<ambre_core::DeltaBytes> {
        match self {
            Self::Trame(state) => {
                state
                    .handle_command(TrameCommand::SetCellText {
                        row: Pos::<Row>::new(i),
                        col: Pos::<Col>::new(0),
                        text: format!("{i}"),
                    })
                    .update
            }
            Self::Toile(state) => {
                state
                    .handle_command(ToileCommand::CreateElement {
                        kind: ambre_toile::logic::model::ElementKind::Shape(
                            ambre_toile::logic::model::ShapeKind::Rect,
                        ),
                        at: kurbo::Point::new(f64::from(i) * 10.0, 0.0),
                        size: kurbo::Size::new(100.0, 60.0),
                        content: ambre_toile::message::InitContent::None,
                        fill: None,
                        correlate: None,
                    })
                    .update
            }
            Self::Prose(state) => {
                let block_id = state.root_blocks()[0];
                state
                    .handle_command(ProseCommand::InsertText {
                        block_id,
                        pos: Pos::ZERO,
                        text: "word ".to_owned(),
                    })
                    .update
            }
        }
        .expect("an edit writes a delta")
    }
}

/// How many events one stimulus sends to the view of its document.
struct Reading {
    stimulus: &'static str,
    events: usize,
    per: usize,
}

impl Reading {
    /// Events per stimulus, rounded up: the figure the bound is divided by.
    fn each(&self) -> usize {
        self.events.div_ceil(self.per)
    }
}

const SMALL_EDITS: u32 = 200;

/// A collaborator types: one remote delta per pass, as a delta that arrives
/// alone is handled alone.
fn remote_edits(doc_type: DocType, stimulus: &'static str) -> Reading {
    let mut bench = Bench::open(doc_type);
    let mut author = Author::new(doc_type);
    let mut events = 0;
    for i in 0..SMALL_EDITS {
        bench.event(LogicEvent::RemoteUpdate {
            document_id: bench.document_id,
            delta: crate::remote_delta(u64::from(i) + 1, author.edit(i)),
        });
        events += bench.pass();
    }
    Reading {
        stimulus,
        events,
        per: SMALL_EDITS as usize,
    }
}

/// A burst: every remote delta of `SMALL_EDITS` queued before one pass.
fn remote_burst() -> Reading {
    let mut bench = Bench::open(DocType::Trame);
    let mut author = Author::new(DocType::Trame);
    for i in 0..SMALL_EDITS {
        bench.event(LogicEvent::RemoteUpdate {
            document_id: bench.document_id,
            delta: crate::remote_delta(u64::from(i) + 1, author.edit(i)),
        });
    }
    Reading {
        stimulus: "Trame, burst of remote deltas in one pass",
        events: bench.pass(),
        per: SMALL_EDITS as usize,
    }
}

/// A history replay on open: one batch of `SMALL_EDITS` deltas.
fn replay() -> Reading {
    let mut bench = Bench::open(DocType::Trame);
    let mut author = Author::new(DocType::Trame);
    let deltas = (0..SMALL_EDITS)
        .map(|i| crate::remote_delta(u64::from(i) + 1, author.edit(i)))
        .collect();
    bench.event(LogicEvent::RemoteUpdateBatch {
        document_id: bench.document_id,
        deltas,
    });
    Reading {
        stimulus: "Trame, replay batch",
        events: bench.pass(),
        per: 1,
    }
}

/// A local paste of 10 000 cells, 100 rows of 100 columns.
fn trame_paste() -> Reading {
    let mut bench = Bench::open(DocType::Trame);
    let row = vec!["7"; 100].join("\t");
    let text = vec![row; 100].join("\n");
    bench.command(AppCommand::Trame(TrameCommand::PasteText { text }));
    Reading {
        stimulus: "Trame, paste of 10 000 cells",
        events: bench.pass(),
        per: 1,
    }
}

/// A local CSV import of 1 000 lines. The count does not grow with the file,
/// and a debug build takes about two minutes for 10 000 lines.
fn trame_import() -> Reading {
    let mut bench = Bench::open(DocType::Trame);
    let csv: String = (0..1_000).map(|i| format!("{i},a,b\n")).collect();
    bench.command(AppCommand::Trame(TrameCommand::import_csv(
        ambre_trame::message::CsvImport {
            data: ambre_core::Payload::<ambre_core::FileBytes>::new(csv.into_bytes()),
            delimiter: None,
            header: ambre_trame::message::HeaderRow::Absent,
            target_sheet_id: None,
            start_row: Pos::<Row>::new(0),
            start_col: Pos::<Col>::new(0),
        },
    )));
    Reading {
        stimulus: "Trame, CSV import of 1 000 lines",
        events: bench.pass(),
        per: 1,
    }
}

/// A local SVG import of 5 000 nodes, counted per pass. The import runs one
/// slice a pass (`ARCH-030`).
fn toile_import() -> Reading {
    let mut bench = Bench::open(DocType::Toile);
    let mut svg = String::from(r#"<svg xmlns="http://www.w3.org/2000/svg">"#);
    for i in 0..5_000 {
        svg.push_str(&format!(r#"<rect x="{i}" y="0" width="5" height="5"/>"#));
    }
    svg.push_str("</svg>");
    bench.command(AppCommand::Toile(ToileCommand::import(
        ambre_toile::message::ToileImportFormat::Svg,
        ambre_core::Payload::<ambre_core::FileBytes>::new(svg.into_bytes()),
        kurbo::Point::ZERO,
    )));
    let (events, passes) = bench.passes();
    Reading {
        stimulus: "Toile, SVG import of 5 000 nodes, per slice",
        events,
        per: passes,
    }
}

/// The figure the budget row reads: what a view hears per stimulus.
///
/// Two facts decide whether the bound is reachable. Every remote delta sends
/// the view at least one event, even in a burst. So a view that reads nothing
/// overflows after at most as many deltas as its bound. A bulk stimulus sends
/// a handful per pass, so a view that reads each frame never overflows from one.
#[test]
fn every_remote_delta_reaches_the_view_and_a_bulk_edit_sends_a_handful() {
    let small = [
        remote_edits(DocType::Trame, "Trame, one remote cell write per pass"),
        remote_edits(DocType::Toile, "Toile, one remote element per pass"),
        remote_edits(DocType::Prose, "Prose, one remote word per pass"),
        remote_burst(),
    ];
    let bulk = [replay(), trame_paste(), trame_import(), toile_import()];
    println!("| Stimulus | Events to the view | Stimuli or passes |");
    for reading in small.iter().chain(bulk.iter()) {
        println!(
            "| {} | {} | {} |",
            reading.stimulus, reading.events, reading.per
        );
    }
    for reading in &small {
        assert!(
            reading.each() >= 1,
            "{}: a remote delta the view never hears",
            reading.stimulus
        );
    }
    for reading in &bulk {
        assert!(
            reading.each() <= 16,
            "{}: {} events in {} passes",
            reading.stimulus,
            reading.events,
            reading.per
        );
    }
}

/// The bench opens a document of each type it reads, and each one is live.
#[test]
fn the_bench_reads_a_live_document() {
    for doc_type in [DocType::Trame, DocType::Toile, DocType::Prose] {
        let bench = Bench::open(doc_type);
        let live = bench.state.docs.get(&bench.document_id).map(|d| &d.state);
        assert!(
            matches!(
                (doc_type, live),
                (DocType::Trame, Some(DocState::Trame(_)))
                    | (DocType::Toile, Some(DocState::Toile(_)))
                    | (DocType::Prose, Some(DocState::Prose(_)))
            ),
            "{doc_type:?} did not open"
        );
    }
}
