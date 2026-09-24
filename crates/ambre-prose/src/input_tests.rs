//! The view driven the way a host drives it: inputs in, commands out, and the
//! Logic thread's answers fed back (`features/ambre-spec-prose-text-editing.md`,
//! `features/ambre-spec-prose-selection.md`).

use crate::events::{BlockId, BlockInit, CursorPosition, ProseCommand, ProseEvent};
use crate::input::{ProseInput, ProseKey};
use crate::logic::ProseState;
use crate::ui::{ProseUiState, SendResult};
use crate::view::tests::append;

/// One peer: its Logic side, its view, and the commands the view sent that
/// have not reached the Logic side yet.
struct Rig {
    logic: ProseState,
    ui: ProseUiState,
    outbox: Vec<ProseCommand>,
    /// Every edit the view ever sent, in order: the syncs are left out.
    sent: Vec<ProseCommand>,
}

impl Rig {
    /// A document holding one paragraph per entry, laid out and in view.
    fn new(paragraphs: &[&str]) -> Self {
        let mut logic = ProseState::with_peer(1).expect("a document is built");
        logic.load_faces(&crate::layout::fonts::shipped_faces());
        for text in paragraphs {
            append(&mut logic, BlockInit::Paragraph, text);
        }
        let mut ui = ProseUiState::default();
        ui.resize(1200.0, 900.0);
        let mut rig = Self {
            logic,
            ui,
            outbox: Vec::new(),
            sent: Vec::new(),
        };
        rig.lay();
        rig
    }

    /// The paragraphs `new` appended, after the empty one a document opens on.
    fn blocks(&self) -> Vec<BlockId> {
        self.logic.root_blocks().into_iter().skip(1).collect()
    }

    fn input(&mut self, input: ProseInput) -> bool {
        let mut sent = Vec::new();
        let consumed = self.ui.handle_input(&input, &mut |command| {
            sent.push(command);
            SendResult::Sent
        });
        self.take_sent(sent);
        consumed
    }

    fn take_sent(&mut self, sent: Vec<ProseCommand>) {
        self.outbox.extend(sent.iter().cloned());
        self.sent.extend(
            sent.into_iter()
                .filter(|command| !matches!(command, ProseCommand::Sync { .. })),
        );
    }

    fn key(&mut self, key: ProseKey, shift: bool, ctrl: bool) -> bool {
        self.input(ProseInput::Key { key, shift, ctrl })
    }

    fn type_text(&mut self, text: &str) -> bool {
        self.input(ProseInput::Text(text.to_owned()))
    }

    fn press(&mut self, (x, y): (f32, f32), shift: bool, now_ms: f64) -> bool {
        self.input(ProseInput::Press {
            x,
            y,
            shift,
            follow: false,
            now_ms,
        })
    }

    /// Hand the sent commands to the Logic side, and its display events back
    /// to the view, the layout apart.
    /// The input the view held meanwhile is replayed, as the host does.
    fn deliver(&mut self) {
        while !self.outbox.is_empty() {
            for command in std::mem::take(&mut self.outbox) {
                let outcome = self.logic.handle_view_command(command);
                for event in &outcome.display {
                    self.ui.apply_event(event);
                }
            }
            let mut sent = Vec::new();
            self.ui.replay(&mut |command| {
                sent.push(command);
                SendResult::Sent
            });
            self.take_sent(sent);
        }
    }

    /// The layout the Logic side answers after its commands.
    fn lay(&mut self) {
        if let Some(event) = self.logic.layout_event() {
            self.ui.apply_event(&event);
        }
    }

    fn caret(&self) -> CursorPosition {
        self.ui.cursor.expect("a caret")
    }

    /// Where a block's offset is drawn on the canvas, as the painter places
    /// it. The point sits inside the next glyph, half way down its line.
    fn point_of(&self, block: BlockId, offset: u32) -> (f32, f32) {
        let ui = &self.ui;
        let layout = &ui.shaped.layout;
        let index = ui
            .flow_index(&crate::events::tree_id(block))
            .expect("the block is laid out");
        let placed = &layout.blocks[index];
        let run = ui.shaped.blocks[&placed.block_id].data();
        let caret = run.caret_box(offset);
        let x = layout.margin_left_px + run.indent_px() + caret.x0 as f32 + 0.5;
        let y_doc = ui.column.page_top_y(placed.page_start)
            + layout.body_top_px()
            + placed.y_in_page
            + (caret.y0 + caret.y1) as f32 / 2.0;
        let scale = ui.viewport.scale();
        (
            ui.column.left_px(&ui.viewport) + x * scale,
            (y_doc - ui.viewport.scroll_y) * scale,
        )
    }

    /// Whether the caret is inside the window.
    fn caret_on_screen(&self) -> bool {
        let (top, bottom) = self.ui.caret_span().expect("the caret is laid out");
        let scroll = self.ui.viewport.scroll_y;
        top >= scroll - 0.01 && bottom <= scroll + self.ui.viewport.visible_height() + 0.01
    }
}

fn words(n: usize) -> String {
    vec!["word"; n].join(" ")
}

// -- Typing -----------------------------------------------------------------

/// `PROSE-UI-004`, `PROSE-UI-136`, `PROSE-UI-176`: the text goes to the draft
/// at its anchor and one insertion leaves carrying it. The document is
/// untouched until that command lands.
#[test]
fn typing_fills_the_draft_and_sends_one_insertion_per_run() {
    let mut rig = Rig::new(&["hello"]);
    let block = rig.blocks()[0];
    rig.ui.set_caret((block, 5));
    assert!(rig.type_text(" w\u{e9}"));
    assert_eq!(
        rig.sent,
        vec![ProseCommand::InsertText {
            block_id: block,
            pos: 5,
            text: " w\u{e9}".to_owned(),
        }]
    );
    assert_eq!(rig.ui.draft.pending_text, " w\u{e9}");
    assert_eq!(rig.ui.draft.insert_pos, 5);
    assert_eq!(rig.caret().offset, 8);
    // Nothing reached the document: the view wrote its own state alone.
    assert_eq!(rig.logic.text_of(&block).as_deref(), Some("hello"));
    rig.deliver();
    assert_eq!(rig.logic.text_of(&block).as_deref(), Some("hello w\u{e9}"));
}

/// `PROSE-UI-004`: a dropped command is not echoed.
#[test]
fn a_dropped_insertion_leaves_no_echo() {
    let mut rig = Rig::new(&["hello"]);
    let block = rig.blocks()[0];
    rig.ui.set_caret((block, 5));
    rig.ui
        .handle_input(&ProseInput::Text("x".to_owned()), &mut |_| {
            SendResult::Dropped
        });
    assert!(rig.ui.draft.is_empty());
    assert_eq!(rig.caret().offset, 5);
}

/// `PROSE-UI-138`: keystrokes at the draft's end accumulate in it; one sent
/// elsewhere re-anchors it.
#[test]
fn keystrokes_at_the_end_of_the_draft_accumulate_in_it() {
    let mut rig = Rig::new(&["hello", "world"]);
    let (first, second) = (rig.blocks()[0], rig.blocks()[1]);
    rig.ui.set_caret((first, 0));
    rig.type_text("a");
    rig.type_text("b");
    assert_eq!(rig.ui.draft.pending_text, "ab");
    assert_eq!(rig.ui.draft.insert_pos, 0);
    rig.ui.set_caret((second, 2));
    rig.type_text("c");
    assert_eq!(rig.ui.draft.pending_text, "c");
    assert_eq!(rig.ui.draft.block_id, Some(second));
    assert_eq!(rig.ui.draft.insert_pos, 2);
}

/// `PROSE-UI-135`: what is not text never enters the draft. A split and a
/// deletion leave as commands, and the draft holds nothing of them.
#[test]
fn a_split_and_a_deletion_leave_as_commands_and_not_as_draft() {
    let mut rig = Rig::new(&["hello"]);
    let block = rig.blocks()[0];
    rig.ui.set_caret((block, 5));
    rig.type_text("!");
    rig.key(ProseKey::Backspace, false, false);
    assert!(rig.ui.draft.is_empty());
    rig.key(ProseKey::Enter, false, false);
    assert!(rig.ui.draft.is_empty());
    assert!(matches!(
        rig.sent.as_slice(),
        [
            ProseCommand::InsertText { .. },
            ProseCommand::DeleteText { range: (5, 6), .. },
            ProseCommand::SplitBlock { pos: 5, .. },
        ]
    ));
}

/// `PROSE-UI-139`: typing changes no structure, no paragraph setting and no
/// mark beside it.
#[test]
fn typing_leaves_the_structure_the_settings_and_the_marks_alone() {
    let mut rig = Rig::new(&["abcdefgh"]);
    let block = rig.blocks()[0];
    rig.logic.handle_command(ProseCommand::ApplyMark {
        block_id: block,
        range: (0, 3),
        key: "b".to_owned(),
        value: None,
    });
    rig.lay();
    let blocks = rig.logic.root_blocks();
    let style = rig.logic.paragraph_style_of(&block);
    rig.ui.set_caret((block, 5));
    rig.type_text("XY");
    rig.deliver();
    assert_eq!(rig.logic.root_blocks(), blocks);
    assert_eq!(rig.logic.paragraph_style_of(&block), style);
    let marks = format!("{:?}", rig.logic.marks_of(&block));
    // The bold run is the three letters it was, and the typed text is plain.
    assert!(marks.contains("\"abc\""), "{marks}");
    assert!(marks.contains("\"deXYfgh\""), "{marks}");
}

// -- Taking in a change -------------------------------------------------------

/// `PROSE-UI-005`: the confirmation drops what it confirms from the draft,
/// once the layout that carries it is in force.
#[test]
fn a_confirmed_run_leaves_the_draft_once_it_is_laid_out() {
    let mut rig = Rig::new(&["hello"]);
    let block = rig.blocks()[0];
    rig.ui.set_caret((block, 5));
    rig.type_text("!");
    rig.deliver();
    // Confirmed, and still drawn: the layout in force does not carry it yet.
    assert!(rig.ui.draft.pending_text.is_empty());
    assert_eq!(rig.ui.draft.shown(), Some((block, 5, "!".to_owned())));
    rig.lay();
    assert!(rig.ui.draft.is_empty());
    assert_eq!(rig.caret().offset, 6);
}

/// A second peer over the same document, for the changes the view did not
/// make.
fn a_collaborator(rig: &Rig) -> ProseState {
    let mut other = ProseState::with_peer(2).expect("a document is built");
    other
        .import_snapshot(&rig.logic.export_snapshot())
        .expect("the snapshot imports");
    other
}

/// Apply what the collaborator did to the rig's Logic side, and hand the
/// view what came of it.
fn receive(rig: &mut Rig, other: &mut ProseState, command: ProseCommand) {
    let update = other
        .handle_command(command)
        .update
        .expect("the change exported");
    for event in rig.logic.apply_remote(&update) {
        rig.ui.apply_event(&event);
    }
}

/// `PROSE-UI-028`: a collaborator's insertion before the draft pushes it on,
/// and the caret with it.
#[test]
fn a_remote_insertion_before_the_draft_shifts_it() {
    let mut rig = Rig::new(&["hello world"]);
    let block = rig.blocks()[0];
    let mut other = a_collaborator(&rig);
    rig.ui.set_caret((block, 6));
    rig.type_text("big ");
    receive(
        &mut rig,
        &mut other,
        ProseCommand::InsertText {
            block_id: block,
            pos: 0,
            text: ">> ".to_owned(),
        },
    );
    assert_eq!(rig.ui.draft.insert_pos, 9);
    assert_eq!(rig.caret().offset, 13);
    // The insertion was sent before the view saw the collaborator's, and it
    // still lands where it was typed.
    rig.deliver();
    rig.lay();
    assert_eq!(
        rig.logic.text_of(&block).as_deref(),
        Some(">> hello big world")
    );
    assert!(rig.ui.draft.is_empty());
}

/// `PROSE-UI-182`: text sent before a collaborator's deletion reached the view
/// lands where it was typed, and so does the next keystroke, both orders alike.
#[test]
fn typing_in_flight_and_a_remote_deletion_land_on_one_text() {
    for remote_first in [true, false] {
        let mut rig = Rig::new(&["hello world"]);
        let block = rig.blocks()[0];
        let mut other = a_collaborator(&rig);
        rig.ui.set_caret((block, 6));
        rig.type_text("big ");
        let deletion = ProseCommand::DeleteText {
            block_id: block,
            range: (0, 6),
        };
        if remote_first {
            receive(&mut rig, &mut other, deletion);
        } else {
            rig.deliver();
            receive(&mut rig, &mut other, deletion);
        }
        assert_eq!(rig.caret().offset, 4);
        rig.type_text("!");
        rig.deliver();
        rig.lay();
        assert_eq!(rig.logic.text_of(&block).as_deref(), Some("big !world"));
        assert!(rig.ui.inflight.is_empty());
    }
}

/// `PROSE-UI-183`: typing right after Enter waits for the split, then lands in
/// the new paragraph rather than in the one that parted.
#[test]
fn typing_right_after_enter_lands_in_the_new_paragraph() {
    let mut rig = Rig::new(&["hello"]);
    let block = rig.blocks()[0];
    rig.ui.set_caret((block, 5));
    rig.key(ProseKey::Enter, false, false);
    assert!(rig.type_text("x"));
    assert_eq!(rig.ui.queued.len(), 1);
    rig.deliver();
    rig.lay();
    let blocks = rig.blocks();
    assert_eq!(rig.logic.text_of(&blocks[0]).as_deref(), Some("hello"));
    assert_eq!(rig.logic.text_of(&blocks[1]).as_deref(), Some("x"));
    assert!(rig.ui.queued.is_empty());
}

/// `PROSE-UI-143`, `PROSE-UI-027`: a change elsewhere and an event carrying
/// no change keep the draft; deleting its block drops it.
#[test]
fn the_draft_is_dropped_only_with_its_block() {
    let mut rig = Rig::new(&["one", "two"]);
    let (first, second) = (rig.blocks()[0], rig.blocks()[1]);
    let mut other = a_collaborator(&rig);
    rig.ui.set_caret((second, 3));
    rig.type_text("!");
    receive(
        &mut rig,
        &mut other,
        ProseCommand::InsertText {
            block_id: first,
            pos: 0,
            text: "x".to_owned(),
        },
    );
    rig.ui.apply_event(&ProseEvent::MediaArrived {
        content_hash: "abc".to_owned(),
        blocks: Vec::new(),
    });
    rig.ui.apply_event(&ProseEvent::RemoteCursors {
        cursors: Vec::new(),
    });
    assert_eq!(rig.ui.draft.pending_text, "!");
    receive(
        &mut rig,
        &mut other,
        ProseCommand::DeleteBlock { block_id: second },
    );
    assert!(rig.ui.draft.is_empty());
}

/// `PROSE-UI-143`, `PROSE-UI-178`: deleting a table takes its cells with it,
/// so a draft in a cell is dropped and the caret leaves it.
#[test]
fn deleting_a_table_drops_the_draft_in_its_cell() {
    use crate::logic::tables::tests::{cell_at, first_block_of, the_table};
    let mut rig = Rig::new(&["one"]);
    let first = rig.blocks()[0];
    rig.logic.handle_command(ProseCommand::InsertTable {
        parent_id: None,
        after_id: Some(first),
        rows: 1,
        cols: 1,
    });
    rig.lay();
    let table = the_table(&rig.logic);
    let cell =
        crate::events::block_id(first_block_of(&rig.logic, cell_at(&rig.logic, table, 0, 0)));
    let mut other = a_collaborator(&rig);
    rig.ui.set_caret((cell, 0));
    rig.type_text("!");
    rig.deliver();
    receive(
        &mut rig,
        &mut other,
        ProseCommand::DeleteBlock {
            block_id: crate::events::block_id(table),
        },
    );
    assert!(rig.ui.draft.is_empty());
    assert_ne!(rig.caret().block_id, cell);
}

/// `PROSE-UI-178`: the caret in a deleted block collapses to the end of the
/// text before it.
#[test]
fn the_caret_of_a_deleted_block_collapses_beside_it() {
    let mut rig = Rig::new(&["one", "two"]);
    let (first, second) = (rig.blocks()[0], rig.blocks()[1]);
    let mut other = a_collaborator(&rig);
    rig.ui.set_caret((second, 2));
    receive(
        &mut rig,
        &mut other,
        ProseCommand::DeleteBlock { block_id: second },
    );
    let caret = rig.caret();
    assert_eq!((caret.block_id, caret.offset), (first, 3));
}

// -- Pressing -----------------------------------------------------------------

/// `PROSE-UI-154`, `PROSE-UI-157`: a press places the caret, collapses the
/// selection, shapes nothing and scrolls nothing.
#[test]
fn a_press_places_the_caret_and_collapses_the_selection() {
    let mut rig = Rig::new(&["hello world"]);
    let block = rig.blocks()[0];
    rig.ui.select_all();
    let scroll = rig.ui.viewport.scroll_y;
    let point = rig.point_of(block, 4);
    assert!(rig.press(point, false, 0.0));
    let caret = rig.caret();
    assert_eq!(
        (caret.block_id, caret.offset, caret.anchor),
        (block, 4, None)
    );
    assert_eq!(rig.ui.viewport.scroll_y, scroll);
    assert_eq!(rig.ui.runs.built(), 0);
    assert!(rig.ui.draft_shape.is_none());
}

/// `PROSE-UI-009`: a press in the margin still lands in the text.
#[test]
fn a_press_in_the_margin_resolves_to_the_nearest_caret() {
    let mut rig = Rig::new(&["hello world"]);
    let block = rig.blocks()[0];
    let (_, y) = rig.point_of(block, 0);
    let left = rig.ui.column.left_px(&rig.ui.viewport);
    assert!(rig.press((left + 2.0, y), false, 0.0));
    assert_eq!(rig.caret().offset, 0);
    let bottom = rig.ui.viewport.height_px - 2.0;
    assert!(rig.press((left + 400.0, bottom), false, 1000.0));
    assert!(rig.ui.cursor.is_some());
}

/// `PROSE-UI-032`: a block holding no text resolves to the edge on the
/// pointer's side.
#[test]
fn a_block_without_text_resolves_to_the_edge_on_the_pointers_side() {
    let mut rig = Rig::new(&["before"]);
    append(&mut rig.logic, BlockInit::HRule, "");
    append(&mut rig.logic, BlockInit::Paragraph, "after");
    rig.lay();
    let blocks = rig.blocks();
    let (before, rule, after) = (blocks[0], blocks[1], blocks[2]);
    assert!(rig.ui.run_of(rule).is_none(), "a rule carries no run");
    let index = rig
        .ui
        .flow_index(&crate::events::tree_id(rule))
        .expect("laid out");
    let placed = rig.ui.shaped.layout.blocks[index].clone();
    let layout = rig.ui.shaped.layout.clone();
    let y_doc = rig.ui.column.page_top_y(placed.page_start)
        + layout.body_top_px()
        + placed.y_in_page
        + placed.height_px / 2.0;
    let y = y_doc - rig.ui.viewport.scroll_y;
    let left = rig.ui.column.left_px(&rig.ui.viewport) + layout.margin_left_px;
    rig.press((left + 5.0, y), false, 0.0);
    assert_eq!((rig.caret().block_id, rig.caret().offset), (before, 6));
    rig.press((left + layout.content_width_px - 5.0, y), false, 1000.0);
    assert_eq!((rig.caret().block_id, rig.caret().offset), (after, 0));
}

/// `PROSE-UI-155`, `PROSE-UI-171`: a second press in place selects the word,
/// a third the paragraph.
#[test]
fn a_second_press_selects_the_word_and_a_third_the_paragraph() {
    let mut rig = Rig::new(&["hello brave world"]);
    let block = rig.blocks()[0];
    let point = rig.point_of(block, 8);
    rig.press(point, false, 0.0);
    rig.press(point, false, 100.0);
    assert_eq!(rig.ui.selection(), Some(((block, 6), (block, 11))));
    rig.press(point, false, 200.0);
    assert_eq!(rig.ui.selection(), Some(((block, 0), (block, 17))));
    // Too late for a fourth: a plain press again.
    rig.press(point, false, 2000.0);
    assert_eq!(rig.ui.selection(), None);
}

/// `PROSE-UI-072`: a press on a document holding no block asks for one, and
/// the caret goes to it once a layout carries it.
#[test]
fn a_press_on_a_document_with_no_block_asks_for_one() {
    use std::sync::Arc;
    let mut ui = ProseUiState::default();
    ui.resize(1200.0, 900.0);
    let empty = crate::layout::run::ShapedLayout {
        version: 1,
        layout: Arc::new(crate::layout::pass::PageLayout {
            page_width_px: 800.0,
            page_height_px: 1000.0,
            page_count: 1,
            ..crate::layout::pass::PageLayout::default()
        }),
        ..crate::layout::run::ShapedLayout::default()
    };
    ui.apply_layout(empty);
    let mut sent = Vec::new();
    let press = ProseInput::Press {
        x: 600.0,
        y: 100.0,
        shift: false,
        follow: false,
        now_ms: 0.0,
    };
    assert!(ui.handle_input(&press, &mut |command| {
        sent.push(command);
        SendResult::Sent
    }));
    // The structural command is followed by the sync that holds the input.
    assert!(matches!(sent.pop(), Some(ProseCommand::Sync { seen: 0 })));
    assert!(matches!(
        sent.as_slice(),
        [ProseCommand::InsertBlock {
            init: BlockInit::Paragraph,
            ..
        }]
    ));
    // The view invented no block: there is no caret until one is laid out.
    assert!(ui.cursor.is_none());
    let mut logic = ProseState::with_peer(1).expect("a document is built");
    logic.load_faces(&crate::layout::fonts::shipped_faces());
    let mut shaped = logic.shaped_layout();
    shaped.version = 2;
    ui.apply_layout(shaped);
    let first = logic.root_blocks()[0];
    assert_eq!(ui.cursor.map(|c| (c.block_id, c.offset)), Some((first, 0)));
}

/// `PROSE-UI-156`: a layout with no page resolves no press, and the caret
/// stays where it was.
#[test]
fn a_press_on_a_layout_with_no_page_leaves_the_caret() {
    let mut ui = ProseUiState::default();
    let block = BlockId::new(1, 0);
    ui.set_caret((block, 3));
    let consumed = ui.handle_input(
        &ProseInput::Press {
            x: 10.0,
            y: 10.0,
            shift: false,
            follow: false,
            now_ms: 0.0,
        },
        &mut |_| SendResult::Sent,
    );
    assert!(!consumed);
    assert_eq!(ui.cursor.map(|c| (c.block_id, c.offset)), Some((block, 3)));
}

// -- Selections ---------------------------------------------------------------

/// `PROSE-UI-039`, `PROSE-UI-151`: a selection built backwards holds, and
/// every reader gets its ends in order, the deletion included.
#[test]
fn a_backwards_selection_is_read_in_order() {
    let mut rig = Rig::new(&["one", "two"]);
    let (first, second) = (rig.blocks()[0], rig.blocks()[1]);
    rig.ui.cursor = Some(CursorPosition {
        block_id: first,
        offset: 1,
        anchor: Some((second, 2)),
    });
    assert_eq!(rig.ui.selection(), Some(((first, 1), (second, 2))));
    rig.key(ProseKey::Delete, false, false);
    assert_eq!(
        rig.sent,
        vec![ProseCommand::DeleteRange {
            anchor: (first, 1),
            focus: (second, 2),
        }]
    );
    assert_eq!(rig.caret().offset, 1);
    assert_eq!(rig.caret().block_id, first);
}

/// `PROSE-UI-030`, `PROSE-UI-137`, `PROSE-UI-140`, `PROSE-UI-177`: typing
/// over a selection takes it first, and the insertion lands where it started.
#[test]
fn typing_over_a_selection_deletes_it_first() {
    let mut rig = Rig::new(&["hello world"]);
    let block = rig.blocks()[0];
    rig.ui.cursor = Some(CursorPosition {
        block_id: block,
        offset: 11,
        anchor: Some((block, 6)),
    });
    rig.type_text("there");
    rig.deliver();
    assert_eq!(rig.logic.text_of(&block).as_deref(), Some("hello there"));
}

// -- Walking the caret --------------------------------------------------------

/// `PROSE-UI-036`, `PROSE-UI-160`: a move with Shift keeps the anchor, one
/// without collapses the selection.
#[test]
fn shift_extends_from_a_fixed_anchor_and_a_plain_move_collapses() {
    let mut rig = Rig::new(&["hello world"]);
    let block = rig.blocks()[0];
    rig.ui.set_caret((block, 2));
    rig.key(ProseKey::Right, true, false);
    rig.key(ProseKey::Right, true, false);
    rig.key(ProseKey::Right, true, true);
    let caret = rig.caret();
    assert_eq!(caret.anchor, Some((block, 2)));
    assert_eq!(caret.offset, 5);
    rig.key(ProseKey::Left, false, false);
    assert_eq!(rig.caret().anchor, None);
    assert_eq!(rig.caret().offset, 4);
}

/// The arrows cross the edge of a block into the next one.
#[test]
fn the_arrows_cross_into_the_next_block() {
    let mut rig = Rig::new(&["ab", "cd"]);
    let (first, second) = (rig.blocks()[0], rig.blocks()[1]);
    rig.ui.set_caret((first, 2));
    rig.key(ProseKey::Right, false, false);
    assert_eq!((rig.caret().block_id, rig.caret().offset), (second, 0));
    rig.key(ProseKey::Left, false, false);
    assert_eq!((rig.caret().block_id, rig.caret().offset), (first, 2));
}

/// `PROSE-UI-034`: a run of vertical moves keeps its column across a short
/// line, and returns to it.
#[test]
fn a_run_of_vertical_moves_keeps_its_column() {
    let long = words(6);
    let mut rig = Rig::new(&[&long, "ab", &long]);
    let blocks = rig.blocks();
    rig.ui.set_caret((blocks[0], 20));
    rig.key(ProseKey::Down, false, false);
    assert_eq!((rig.caret().block_id, rig.caret().offset), (blocks[1], 2));
    rig.key(ProseKey::Down, false, false);
    assert_eq!((rig.caret().block_id, rig.caret().offset), (blocks[2], 20));
    // A horizontal move ends the run.
    rig.key(ProseKey::Left, false, false);
    assert_eq!(rig.ui.x_goal, None);
}

/// `PROSE-UI-159`: Home and End go to the edges of the wrapped line under
/// the caret, never of the paragraph.
#[test]
fn home_and_end_honour_the_wrapped_line() {
    let long = words(400);
    let mut rig = Rig::new(&[&long]);
    let block = rig.blocks()[0];
    let run_len = rig.ui.run_of(block).expect("a run").len();
    let second_line = {
        let run = rig.ui.run_of(block).expect("a run");
        assert!(run.line_count() > 2);
        run.at_line(1, 5.0).0
    };
    rig.ui.set_caret((block, second_line + 2));
    rig.key(ProseKey::End, false, false);
    let end = rig.caret().offset;
    assert!(end < run_len);
    assert!(rig.ui.caret_upstream, "the caret stays on the line it ends");
    let run = rig.ui.run_of(block).expect("a run");
    assert_eq!(run.line_of(end, true), 1);
    rig.key(ProseKey::Home, false, false);
    let start = rig.caret().offset;
    assert!(start > 0 && start <= second_line);
    let run = rig.ui.run_of(block).expect("a run");
    assert_eq!(run.line_of(start, false), 1);
}

/// `PROSE-UI-035`, `PROSE-UI-158`: every move leaves the caret on screen,
/// and one that left it scrolls the least that brings it back.
#[test]
fn every_move_leaves_the_caret_on_screen() {
    let long = words(60);
    let paragraphs: Vec<&str> = vec![long.as_str(); 60];
    let mut rig = Rig::new(&paragraphs);
    let first = rig.blocks()[0];
    rig.ui.set_caret((first, 0));
    let moves = [
        (ProseKey::End, true),
        (ProseKey::Down, false),
        (ProseKey::PageDown, false),
        (ProseKey::End, true),
        (ProseKey::Up, false),
        (ProseKey::Home, true),
        (ProseKey::Right, false),
        (ProseKey::PageUp, false),
        (ProseKey::Left, false),
    ];
    for (key, ctrl) in moves {
        rig.key(key, false, ctrl);
        assert!(rig.caret_on_screen(), "{key:?} with ctrl {ctrl}");
    }
    // At the bottom of the window, one line down scrolls by that line alone.
    rig.key(ProseKey::Home, false, true);
    let visible = rig.ui.viewport.visible_height();
    let start = rig.ui.viewport.scroll_y;
    while rig.ui.viewport.scroll_y <= start {
        rig.key(ProseKey::Down, false, false);
    }
    let (_, bottom) = rig.ui.caret_span().expect("laid out");
    assert!((bottom - (rig.ui.viewport.scroll_y + visible)).abs() < 0.01);
}

// -- Dragging -----------------------------------------------------------------

/// `PROSE-UI-037`, `PROSE-UI-161`, `PROSE-UI-162`: the press captures the
/// anchor, the drag moves the other end under the pointer, and the window
/// does not scroll.
#[test]
fn a_drag_grows_from_the_anchor_its_press_captured() {
    let mut rig = Rig::new(&["hello world", "second line"]);
    let (first, second) = (rig.blocks()[0], rig.blocks()[1]);
    rig.press(rig.point_of(first, 3), false, 0.0);
    let scroll = rig.ui.viewport.scroll_y;
    let (x, y) = rig.point_of(second, 4);
    rig.input(ProseInput::Drag { x, y });
    assert_eq!(rig.ui.selection(), Some(((first, 3), (second, 4))));
    // Upward, past the anchor: the anchor holds, the ends swap order.
    let (x, y) = rig.point_of(first, 1);
    rig.input(ProseInput::Drag { x, y });
    assert_eq!(rig.caret().anchor, Some((first, 3)));
    assert_eq!(rig.ui.selection(), Some(((first, 1), (first, 3))));
    assert_eq!(rig.ui.viewport.scroll_y, scroll);
    rig.input(ProseInput::Release);
    let (x, y) = rig.point_of(second, 2);
    assert!(!rig.input(ProseInput::Drag { x, y }));
}

/// `PROSE-UI-152`: a collaborator's change does not move the anchor of a
/// drag in flight.
#[test]
fn a_remote_change_does_not_move_the_anchor_of_a_drag() {
    let mut rig = Rig::new(&["hello world"]);
    let block = rig.blocks()[0];
    let mut other = a_collaborator(&rig);
    rig.press(rig.point_of(block, 6), false, 0.0);
    let (x, y) = rig.point_of(block, 9);
    rig.input(ProseInput::Drag { x, y });
    receive(
        &mut rig,
        &mut other,
        ProseCommand::InsertText {
            block_id: block,
            pos: 0,
            text: "abc".to_owned(),
        },
    );
    assert_eq!(rig.caret().anchor, Some((block, 6)));
}

// -- Escape and select all ----------------------------------------------------

/// `PROSE-UI-164`, `PROSE-UI-165`, `PROSE-UI-166`: Escape collapses the
/// selection to the caret, cancels the drag, and moves neither the caret nor
/// the window.
#[test]
fn escape_collapses_and_cancels_the_drag() {
    let mut rig = Rig::new(&["hello world"]);
    let block = rig.blocks()[0];
    rig.press(rig.point_of(block, 2), false, 0.0);
    let (x, y) = rig.point_of(block, 7);
    rig.input(ProseInput::Drag { x, y });
    let scroll = rig.ui.viewport.scroll_y;
    rig.key(ProseKey::Escape, false, false);
    let caret = rig.caret();
    assert_eq!((caret.offset, caret.anchor), (7, None));
    assert_eq!(rig.ui.viewport.scroll_y, scroll);
    let (x, y) = rig.point_of(block, 9);
    assert!(!rig.input(ProseInput::Drag { x, y }));
    assert!(!rig.input(ProseInput::Release));
    assert_eq!(rig.caret().offset, 7);
}

/// `PROSE-UI-167`, `PROSE-UI-168`: select all runs from the start of the
/// first block to the end of the last, and the reader stays on their page.
#[test]
fn select_all_spans_the_document_and_keeps_the_window() {
    let long = words(80);
    let paragraphs: Vec<&str> = vec![long.as_str(); 30];
    let mut rig = Rig::new(&paragraphs);
    let blocks = rig.logic.root_blocks();
    let (first, last) = (blocks[0], *blocks.last().expect("a block"));
    rig.ui.set_caret((blocks[3], 0));
    rig.ui.scroll_by(700.0);
    let scroll = rig.ui.viewport.scroll_y;
    rig.key(ProseKey::SelectAll, false, true);
    let len = rig.ui.run_of(last).expect("a run").len();
    assert_eq!(rig.ui.selection(), Some(((first, 0), (last, len))));
    assert_eq!(rig.ui.viewport.scroll_y, scroll);
}

// -- Hit-testing --------------------------------------------------------------

/// `PROSE-UI-008`, `PROSE-UI-031`: a point drawn at an offset resolves back
/// to it, at any zoom and any scroll, and the rig never paints a block.
#[test]
fn a_point_resolves_to_the_offset_drawn_there_under_zoom_and_scroll() {
    let long = words(50);
    let paragraphs: Vec<&str> = vec![long.as_str(); 20];
    let mut rig = Rig::new(&paragraphs);
    let block = rig.blocks()[12];
    for (zoom, scroll) in [(1.0, 0.0), (1.5, 900.0), (0.75, 2400.0)] {
        rig.ui.viewport.zoom = zoom;
        rig.ui.viewport.scroll_y = 0.0;
        rig.ui.scroll_by(scroll * zoom);
        for offset in [0, 9, 42, 120] {
            let (x, y) = rig.point_of(block, offset);
            assert_eq!(rig.ui.hit_test(x, y), Some((block, offset)), "zoom {zoom}");
        }
    }
}

/// `PROSE-UI-184`: an insertion the document refuses, because its block was
/// deleted meanwhile, leaves the view on the text the document holds.
#[test]
fn a_refused_insertion_leaves_the_view_on_the_documents_text() {
    let mut rig = Rig::new(&["one", "two"]);
    let (first, second) = (rig.blocks()[0], rig.blocks()[1]);
    let mut other = a_collaborator(&rig);
    rig.ui.set_caret((second, 3));
    rig.type_text("!");
    receive(
        &mut rig,
        &mut other,
        ProseCommand::DeleteBlock { block_id: second },
    );
    rig.deliver();
    rig.lay();
    assert!(rig.ui.inflight.is_empty());
    assert!(rig.ui.draft.is_empty());
    assert_eq!(rig.caret().block_id, first);
    rig.type_text("s");
    rig.deliver();
    assert_eq!(rig.logic.text_of(&first).as_deref(), Some("ones"));
}

// -- Links ------------------------------------------------------------------

const TARGET: &str = "https://example.org/";

/// A paragraph whose word `link` (offsets 6 to 10) is a link, its list in the
/// view.
fn linked() -> (Rig, BlockId) {
    let mut rig = Rig::new(&["go to link and back"]);
    let block = rig.blocks()[0];
    rig.outbox.push(ProseCommand::ApplyMark {
        block_id: block,
        range: (6, 10),
        key: "a".to_owned(),
        value: Some(TARGET.to_owned()),
    });
    rig.deliver();
    rig.lay();
    (rig, block)
}

fn press_follow(rig: &mut Rig, (x, y): (f32, f32), follow: bool) -> bool {
    rig.input(ProseInput::Press {
        x,
        y,
        shift: false,
        follow,
        now_ms: 0.0,
    })
}

/// `PROSE-UI-153`: the link answers over its marked text, and the text beside
/// it on the same line does not.
#[test]
fn a_link_answers_over_its_marked_text_alone() {
    let (rig, block) = linked();
    assert_eq!(rig.ui.links.len(), 1);
    let (x, y) = rig.point_of(block, 7);
    assert_eq!(rig.ui.link_at(x, y).map(|l| l.url.as_str()), Some(TARGET));
    let (x, y) = rig.point_of(block, 2);
    assert_eq!(rig.ui.link_at(x, y), None);
    let (x, y) = rig.point_of(block, 12);
    assert_eq!(rig.ui.link_at(x, y), None);
}

/// `PROSE-UI-070`, `PROSE-UI-169`: a plain press inside a link places the
/// caret there and asks the host for nothing.
#[test]
fn a_plain_press_inside_a_link_places_the_caret_and_opens_nothing() {
    let (mut rig, block) = linked();
    let at = rig.point_of(block, 8);
    assert!(press_follow(&mut rig, at, false));
    assert_eq!((rig.caret().block_id, rig.caret().offset), (block, 8));
    assert_eq!(rig.ui.followed, None);
    assert!(
        rig.sent
            .iter()
            .all(|c| !matches!(c, ProseCommand::ApplyMark { .. }))
    );
}

/// `PROSE-UI-071`, `PROSE-UI-170`: a press with the modifier hands the target
/// to the host, and the caret and the selection stay where they were.
#[test]
fn a_press_with_the_modifier_hands_the_target_over_and_moves_nothing() {
    let (mut rig, block) = linked();
    rig.ui.cursor = Some(CursorPosition {
        block_id: block,
        offset: 2,
        anchor: Some((block, 0)),
    });
    let before = rig.ui.cursor;
    let sent = rig.sent.len();
    let at = rig.point_of(block, 8);
    assert!(press_follow(&mut rig, at, true));
    assert_eq!(rig.ui.followed.as_deref(), Some(TARGET));
    assert_eq!(rig.ui.cursor, before);
    assert_eq!(rig.sent.len(), sent, "nothing is written");
    // Off a link, the modifier is an ordinary press.
    let at = rig.point_of(block, 15);
    rig.ui.followed = None;
    assert!(press_follow(&mut rig, at, true));
    assert_eq!(rig.ui.followed, None);
    assert_eq!(rig.caret().offset, 15);
}

/// `PROSE-UI-186`: a peer's `javascript:` target never reaches the host, and
/// the press that met it moves nothing either.
#[test]
fn a_target_that_could_run_code_is_held_back() {
    let mut rig = Rig::new(&["go to link and back"]);
    let block = rig.blocks()[0];
    rig.outbox.push(ProseCommand::ApplyMark {
        block_id: block,
        range: (6, 10),
        key: "a".to_owned(),
        value: Some("javascript:alert(1)".to_owned()),
    });
    rig.deliver();
    rig.lay();
    let before = rig.ui.cursor;
    let at = rig.point_of(block, 8);
    assert!(press_follow(&mut rig, at, true));
    assert_eq!(rig.ui.followed, None);
    assert_eq!(rig.ui.cursor, before);
}

/// A document whose one page carries a header and a footer, each with text.
fn banded() -> (Rig, BlockId, BlockId) {
    let mut rig = Rig::new(&["the body of the page"]);
    rig.outbox
        .push(ProseCommand::SetHeaderEnabled { enabled: true });
    rig.outbox
        .push(ProseCommand::SetFooterEnabled { enabled: true });
    rig.deliver();
    let bands = rig.logic.bands();
    let header = bands.header.expect("the header block");
    let footer = bands.footer.expect("the footer block");
    for (block, text) in [(header, "the running head"), (footer, "the running foot")] {
        rig.outbox.push(ProseCommand::InsertText {
            block_id: block,
            pos: 0,
            text: text.to_owned(),
        });
    }
    rig.deliver();
    rig.lay();
    (rig, header, footer)
}

/// Where a band's offset is drawn on `page`, as the painter places it.
fn band_point(rig: &Rig, block: BlockId, offset: u32, page: u32) -> (f32, f32) {
    let ui = &rig.ui;
    let layout = &ui.shaped.layout;
    let slot = ui
        .band_slots(page)
        .into_iter()
        .find(|slot| slot.block == crate::events::tree_id(block))
        .expect("the band is drawn");
    let run = ui.shaped.blocks[&slot.block].data();
    let caret = run.caret_box(offset);
    let x = layout.margin_left_px + run.indent_px() + caret.x0 as f32 + 0.5;
    let y_doc = ui.column.page_top_y(page) + slot.top + (caret.y0 + caret.y1) as f32 / 2.0;
    let scale = ui.viewport.scale();
    (
        ui.column.left_px(&ui.viewport) + x * scale,
        (y_doc - ui.viewport.scroll_y) * scale,
    )
}

/// `PROSE-UI-033`: a press in the header or the footer puts the caret in its
/// text, on the page it was placed on. Typing then goes to that block.
#[test]
fn a_press_in_a_header_or_a_footer_lands_in_its_text() {
    let (mut rig, header, footer) = banded();
    for block in [header, footer] {
        let at = band_point(&rig, block, 4, 0);
        assert!(rig.press(at, false, 0.0));
        assert_eq!((rig.caret().block_id, rig.caret().offset), (block, 4));
        assert_eq!(rig.ui.band_page, Some(0));
        assert!(
            rig.ui.caret_span().is_some(),
            "the caret is placed on its page"
        );
        rig.ui.last_press = None;
    }
    let sent = rig.sent.len();
    assert!(rig.type_text("x"));
    rig.deliver();
    assert!(
        rig.sent[sent..].iter().any(|command| matches!(
            command,
            ProseCommand::InsertText { block_id, pos: 4, text } if *block_id == footer && text == "x"
        )),
        "the text goes to the footer"
    );
}

/// `PROSE-UI-033`: a band holds its own selection. A drag from it into the
/// body, or a shift press across its edge, selects nothing across.
#[test]
fn a_selection_does_not_cross_the_edge_of_a_band() {
    let (mut rig, header, _) = banded();
    let body = rig.blocks()[0];
    let at = band_point(&rig, header, 2, 0);
    assert!(rig.press(at, false, 0.0));
    let (x, y) = rig.point_of(body, 5);
    rig.input(ProseInput::Drag { x, y });
    assert_eq!(rig.caret().block_id, header);
    assert_eq!(rig.caret().anchor, None);
    // Within the band, a drag selects.
    let (x, y) = band_point(&rig, header, 8, 0);
    rig.input(ProseInput::Drag { x, y });
    assert_eq!(rig.caret().anchor, Some((header, 2)));
    assert_eq!(rig.ui.selection(), Some(((header, 2), (header, 8))));
    // A shift press in the body is a plain press there.
    rig.input(ProseInput::Release);
    rig.ui.last_press = None;
    assert!(rig.press(rig.point_of(body, 5), true, 0.0));
    assert_eq!(
        (rig.caret().block_id, rig.caret().offset, rig.caret().anchor),
        (body, 5, None)
    );
}

/// A document whose one page calls one note, and the note's block.
fn noted() -> (Rig, BlockId, BlockId) {
    let mut rig = Rig::new(&["the body of the page"]);
    let body = rig.blocks()[0];
    rig.outbox.push(ProseCommand::InsertFootnote {
        block_id: body,
        pos: 4,
        note_text: "the text of the note".to_owned(),
    });
    rig.deliver();
    rig.lay();
    let note = rig
        .ui
        .note_slots(0)
        .first()
        .map(|slot| crate::events::block_id(slot.block))
        .expect("the note is drawn on the first page");
    (rig, body, note)
}

/// Where a note's offset is drawn, as the painter places it.
fn note_point(rig: &Rig, block: BlockId, offset: u32) -> (f32, f32) {
    let ui = &rig.ui;
    let layout = &ui.shaped.layout;
    let slot = ui
        .note_slots(0)
        .into_iter()
        .find(|slot| slot.block == crate::events::tree_id(block))
        .expect("the note is drawn");
    let run = ui.shaped.blocks[&slot.block].data();
    let caret = run.caret_box(offset);
    let x = layout.margin_left_px + run.indent_px() + caret.x0 as f32 + 0.5;
    let y_doc = ui.column.page_top_y(0) + slot.top + (caret.y0 + caret.y1) as f32 / 2.0;
    let scale = ui.viewport.scale();
    (
        ui.column.left_px(&ui.viewport) + x * scale,
        (y_doc - ui.viewport.scroll_y) * scale,
    )
}

/// `PROSE-UI-187`: a press in a note puts the caret in its text, past the
/// number drawn beside it, and typing goes to the note's block.
#[test]
fn a_press_in_a_note_lands_in_its_text() {
    let (mut rig, _, note) = noted();
    let at = note_point(&rig, note, 4);
    assert!(rig.press(at, false, 0.0));
    assert_eq!((rig.caret().block_id, rig.caret().offset), (note, 4));
    assert!(
        rig.ui.caret_span().is_some(),
        "the caret is placed in the note"
    );
    let sent = rig.sent.len();
    assert!(rig.type_text("x"));
    rig.deliver();
    assert!(
        rig.sent[sent..].iter().any(|command| matches!(
            command,
            ProseCommand::InsertText { block_id, pos: 4, text } if *block_id == note && text == "x"
        )),
        "the text goes to the note"
    );
    assert_eq!(
        rig.logic.text_of(&note).as_deref(),
        Some("the xtext of the note")
    );
}

/// `PROSE-UI-187`: a note holds its own selection, as a band does.
#[test]
fn a_selection_does_not_cross_the_edge_of_a_note() {
    let (mut rig, body, note) = noted();
    let at = note_point(&rig, note, 2);
    assert!(rig.press(at, false, 0.0));
    let (x, y) = rig.point_of(body, 5);
    rig.input(ProseInput::Drag { x, y });
    assert_eq!(rig.caret().block_id, note);
    assert_eq!(rig.caret().anchor, None);
    let (x, y) = note_point(&rig, note, 8);
    rig.input(ProseInput::Drag { x, y });
    assert_eq!(rig.ui.selection(), Some(((note, 2), (note, 8))));
}
