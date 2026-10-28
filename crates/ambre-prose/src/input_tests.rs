//! The view driven the way a host drives it: inputs in, commands out, and the
//! Logic thread's answers fed back (`features/ambre-spec-prose-text-editing.md`,
//! `features/ambre-spec-prose-selection.md`).

use crate::layout::run::{Affinity, RunOffset};
use ambre_bus::PasteSource;
use ambre_bus::app::prose::DocOffset;
use ambre_bus::app::prose::DocSpan;
use ambre_commands::ClipboardWrite;
use ambre_core::{DocPx, ViewPx};
use ambre_draw::HitTest;

use crate::events::{BlockId, BlockInit, CursorPosition, ProseCommand, ProseEvent, TriState};
use crate::input::{ProseInput, ProseKey};
use crate::logic::ProseState;
use crate::ui::{ProseUiState, SendResult};
use crate::view::tests::{append, down, view_at};

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
        let mut logic = ProseState::with_peer(ambre_core::PeerId(1)).expect("a document is built");
        logic.load_faces(&crate::layout::fonts::shipped_faces());
        for text in paragraphs {
            append(&mut logic, BlockInit::Paragraph, text);
        }
        let mut ui = ProseUiState::default();
        ui.resize(ViewPx::new(1200.0), ViewPx::new(900.0));
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
        self.input(ProseInput::Key {
            key,
            mods: ambre_commands::Modifiers::NONE
                .ctrl_if(ambre_commands::KeyState::from_bool(ctrl))
                .shift_if(ambre_commands::KeyState::from_bool(shift)),
        })
    }

    fn type_text(&mut self, text: &str) -> bool {
        self.input(ProseInput::Text(text.to_owned()))
    }

    fn press(&mut self, (x, y): (f32, f32), shift: bool, now: ambre_core::PageTime) -> bool {
        self.input(ProseInput::Press {
            at: view_at(x, y),
            mods: ambre_commands::Modifiers::NONE
                .shift_if(ambre_commands::KeyState::from_bool(shift)),
            now,
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
        let placed = &layout.blocks[index.to_usize()];
        let run = ui.shaped.blocks[&placed.block_id].data();
        let caret = run.caret_box(RunOffset::new(offset));
        let x = layout.margin_left + run.indent() + DocPx::new(caret.x0 as f32) + DocPx::new(0.5);
        let y_doc = ui.column.page_top_y(placed.page_start)
            + layout.body_top()
            + placed.y_in_page
            + DocPx::new((caret.y0 + caret.y1) as f32 / 2.0);
        let scale = ui.viewport.scale();
        (
            (ui.column.left(&ui.viewport) + ViewPx::new(x.get() * scale)).get(),
            (y_doc - ui.viewport.scroll_y).get() * scale,
        )
    }

    /// Whether the caret is inside the window.
    fn caret_on_screen(&self) -> bool {
        let (top, bottom) = self.ui.caret_span().expect("the caret is laid out");
        let scroll = self.ui.viewport.scroll_y;
        top >= scroll - DocPx::new(0.01)
            && bottom <= scroll + self.ui.viewport.visible_height() + DocPx::new(0.01)
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
    rig.ui.set_caret((block, DocOffset::new(5)));
    assert!(rig.type_text(" w\u{e9}"));
    assert_eq!(
        rig.sent,
        vec![ProseCommand::InsertText {
            block_id: block,
            pos: DocOffset::new(5),
            text: " w\u{e9}".to_owned(),
        }]
    );
    assert_eq!(rig.ui.shown_text(block).as_deref(), Some("hello w\u{e9}"));
    assert_eq!(rig.caret().offset, DocOffset::new(8));
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
    rig.ui.set_caret((block, DocOffset::new(5)));
    rig.ui
        .handle_input(&ProseInput::Text("x".to_owned()), &mut |_| {
            SendResult::Dropped
        });
    assert!(rig.ui.draft.is_empty());
    assert_eq!(rig.caret().offset, DocOffset::new(5));
}

/// `PROSE-UI-138`: each keystroke is an entry of the draft, and a block keeps
/// its entries when the caret moves to another.
#[test]
fn each_keystroke_is_its_own_entry_of_the_draft() {
    let mut rig = Rig::new(&["hello", "world"]);
    let (first, second) = (rig.blocks()[0], rig.blocks()[1]);
    rig.ui.set_caret((first, DocOffset::ZERO));
    rig.type_text("a");
    rig.type_text("b");
    assert_eq!(rig.ui.shown_text(first).as_deref(), Some("abhello"));
    rig.ui.set_caret((second, DocOffset::new(2)));
    rig.type_text("c");
    assert_eq!(rig.ui.shown_text(second).as_deref(), Some("wocrld"));
    assert_eq!(rig.ui.shown_text(first).as_deref(), Some("abhello"));
    assert_eq!(rig.ui.draft.blocks(), vec![first, second]);
}

/// `PROSE-UI-135`: a split never enters the draft. A deletion leaves as a
/// command, and the draft holds the text it takes out.
#[test]
fn a_split_never_enters_the_draft() {
    let mut rig = Rig::new(&["hello"]);
    let block = rig.blocks()[0];
    rig.ui.set_caret((block, DocOffset::new(5)));
    rig.type_text("!");
    rig.key(ProseKey::Backspace, false, false);
    assert_eq!(rig.ui.shown_text(block).as_deref(), Some("hello"));
    let before = rig.ui.draft.clone();
    rig.key(ProseKey::Enter, false, false);
    assert_eq!(rig.ui.draft, before);
    assert!(matches!(
        rig.sent.as_slice(),
        [
            ProseCommand::InsertText { .. },
            ProseCommand::DeleteText { range, .. },
            ProseCommand::SplitBlock { pos, .. },
        ] if *range == DocSpan::new(DocOffset::new(5), DocOffset::new(6)) && *pos == DocOffset::new(5)
    ));
}

/// `PROSE-UI-206`: a deletion sent after text no layout carries yet takes
/// out what it deletes and leaves that text drawn.
#[test]
fn a_deletion_leaves_the_text_typed_before_it_drawn() {
    let mut rig = Rig::new(&["hello"]);
    let block = rig.blocks()[0];
    rig.ui.set_caret((block, DocOffset::new(5)));
    rig.type_text("abc");
    rig.ui.set_caret((block, DocOffset::new(1)));
    rig.key(ProseKey::Backspace, false, false);
    assert_eq!(rig.ui.shown_text(block).as_deref(), Some("elloabc"));
    let mut font_cx = parley::FontContext::new();
    for bytes in crate::layout::fonts::shipped_faces() {
        crate::layout::fonts::register_faces(&mut font_cx, bytes.into_bytes());
    }
    let mut layout_cx = parley::LayoutContext::new();
    rig.ui.shape_draft(&mut font_cx, &mut layout_cx);
    let drawn = rig.ui.run_of(block).map(|run| run.text().to_owned());
    assert_eq!(drawn.as_deref(), Some("elloabc"));
    assert_eq!(rig.caret().offset, DocOffset::ZERO);
    rig.deliver();
    rig.lay();
    assert_eq!(rig.logic.text_of(&block).as_deref(), Some("elloabc"));
}

/// `PROSE-124`: two deletions sent before the layout comes back read their
/// clusters on the old run. The second lands inside a cluster of the new
/// text, and the Logic side takes the whole of it.
#[test]
fn two_deletes_before_the_layout_never_split_a_grapheme() {
    let mut rig = Rig::new(&["xae\u{301}z"]);
    let block = rig.blocks()[0];
    rig.ui.set_caret((block, DocOffset::new(1)));
    rig.key(ProseKey::Delete, false, false);
    rig.key(ProseKey::Delete, false, false);
    rig.deliver();
    rig.lay();
    assert_eq!(rig.logic.text_of(&block).as_deref(), Some("xz"));
    assert_eq!(rig.caret().offset, DocOffset::new(1));
}

/// `PROSE-UI-139`: typing changes no structure, no paragraph setting and no
/// mark beside it.
#[test]
fn typing_leaves_the_structure_the_settings_and_the_marks_alone() {
    let mut rig = Rig::new(&["abcdefgh"]);
    let block = rig.blocks()[0];
    rig.logic.handle_command(ProseCommand::ApplyMark {
        block_id: block,
        range: DocSpan::from_usize_range(0..3),
        key: "b".into(),
        value: None,
    });
    rig.lay();
    let blocks = rig.logic.root_blocks();
    let style = rig.logic.paragraph_style_of(&block);
    rig.ui.set_caret((block, DocOffset::new(5)));
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
    rig.ui.set_caret((block, DocOffset::new(5)));
    rig.type_text("!");
    rig.deliver();
    // Confirmed, and still drawn: the layout in force does not carry it yet.
    assert_eq!(rig.ui.shown_text(block).as_deref(), Some("hello!"));
    rig.lay();
    assert!(rig.ui.draft.is_empty());
    assert_eq!(rig.caret().offset, DocOffset::new(6));
}

/// A second peer over the same document, for the changes the view did not
/// make.
fn a_collaborator(rig: &Rig) -> ProseState {
    let mut other = ProseState::with_peer(ambre_core::PeerId(2)).expect("a document is built");
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
    rig.ui.set_caret((block, DocOffset::new(6)));
    rig.type_text("big ");
    receive(
        &mut rig,
        &mut other,
        ProseCommand::InsertText {
            block_id: block,
            pos: DocOffset::ZERO,
            text: ">> ".to_owned(),
        },
    );
    assert_eq!(
        rig.ui.draft.text_of(block, ">> hello world"),
        ">> hello big world"
    );
    assert_eq!(rig.caret().offset, DocOffset::new(13));
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
        rig.ui.set_caret((block, DocOffset::new(6)));
        rig.type_text("big ");
        let deletion = ProseCommand::DeleteText {
            block_id: block,
            range: DocSpan::from_usize_range(0..6),
        };
        if remote_first {
            receive(&mut rig, &mut other, deletion);
        } else {
            rig.deliver();
            receive(&mut rig, &mut other, deletion);
        }
        assert_eq!(rig.caret().offset, DocOffset::new(4));
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
    rig.ui.set_caret((block, DocOffset::new(5)));
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
    rig.ui.set_caret((second, DocOffset::new(3)));
    rig.type_text("!");
    receive(
        &mut rig,
        &mut other,
        ProseCommand::InsertText {
            block_id: first,
            pos: DocOffset::ZERO,
            text: "x".to_owned(),
        },
    );
    rig.ui.apply_event(&ProseEvent::MediaArrived {
        content_hash: ambre_core::ContentHash("abc".to_owned()),
        blocks: Vec::new(),
    });
    rig.ui.apply_event(&ProseEvent::RemoteCursors {
        cursors: Vec::new(),
    });
    assert_eq!(rig.ui.shown_text(second).as_deref(), Some("two!"));
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
        rows: crate::events::grid::RowCount::new(1),
        cols: crate::events::grid::ColCount::new(1),
    });
    rig.lay();
    let table = the_table(&rig.logic);
    let cell =
        crate::events::block_id(first_block_of(&rig.logic, cell_at(&rig.logic, table, 0, 0)));
    let mut other = a_collaborator(&rig);
    rig.ui.set_caret((cell, DocOffset::ZERO));
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
    rig.ui.set_caret((second, DocOffset::new(2)));
    receive(
        &mut rig,
        &mut other,
        ProseCommand::DeleteBlock { block_id: second },
    );
    let caret = rig.caret();
    assert_eq!((caret.block_id, caret.offset), (first, DocOffset::new(3)));
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
    assert!(rig.press(point, false, ambre_core::PageTime(0.0)));
    let caret = rig.caret();
    assert_eq!(
        (caret.block_id, caret.offset, caret.anchor),
        (block, DocOffset::new(4), None)
    );
    assert_eq!(rig.ui.viewport.scroll_y, scroll);
    assert_eq!(rig.ui.runs.built(), 0);
    assert!(rig.ui.provisional_run(block).is_none());
}

/// `PROSE-UI-009`: a press in the margin still lands in the text.
#[test]
fn a_press_in_the_margin_resolves_to_the_nearest_caret() {
    let mut rig = Rig::new(&["hello world"]);
    let block = rig.blocks()[0];
    let (_, y) = rig.point_of(block, 0);
    let left = rig.ui.column.left(&rig.ui.viewport);
    assert!(rig.press((left.get() + 2.0, y), false, ambre_core::PageTime(0.0)));
    assert_eq!(rig.caret().offset, DocOffset::ZERO);
    let bottom = rig.ui.viewport.height - ViewPx::new(2.0);
    assert!(rig.press(
        (left.get() + 400.0, bottom.get()),
        false,
        ambre_core::PageTime(1000.0)
    ));
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
    let placed = rig.ui.shaped.layout.blocks[index.to_usize()].clone();
    let layout = rig.ui.shaped.layout.clone();
    let y_doc = rig.ui.column.page_top_y(placed.page_start)
        + layout.body_top()
        + placed.y_in_page
        + placed.height / 2.0;
    let y = (y_doc - rig.ui.viewport.scroll_y).get();
    let left = rig.ui.column.left(&rig.ui.viewport).get() + layout.margin_left.get();
    rig.press((left + 5.0, y), false, ambre_core::PageTime(0.0));
    assert_eq!(
        (rig.caret().block_id, rig.caret().offset),
        (before, DocOffset::new(6))
    );
    rig.press(
        (left + layout.content_width.get() - 5.0, y),
        false,
        ambre_core::PageTime(1000.0),
    );
    assert_eq!(
        (rig.caret().block_id, rig.caret().offset),
        (after, DocOffset::ZERO)
    );
}

/// `PROSE-UI-155`, `PROSE-UI-171`: a second press in place selects the word,
/// a third the paragraph.
#[test]
fn a_second_press_selects_the_word_and_a_third_the_paragraph() {
    let mut rig = Rig::new(&["hello brave world"]);
    let block = rig.blocks()[0];
    let point = rig.point_of(block, 8);
    rig.press(point, false, ambre_core::PageTime(0.0));
    rig.press(point, false, ambre_core::PageTime(100.0));
    assert_eq!(
        rig.ui.selection(),
        Some(((block, DocOffset::new(6)), (block, DocOffset::new(11))))
    );
    rig.press(point, false, ambre_core::PageTime(200.0));
    assert_eq!(
        rig.ui.selection(),
        Some(((block, DocOffset::ZERO), (block, DocOffset::new(17))))
    );
    // Too late for a fourth: a plain press again.
    rig.press(point, false, ambre_core::PageTime(2000.0));
    assert_eq!(rig.ui.selection(), None);
}

/// `PROSE-UI-171`, `PROSE-UI-167`: text drafted at a block's end waits for
/// the layout. It is inside the paragraph a third press selects, and inside
/// the whole document.
#[test]
fn a_third_press_selects_the_text_drafted_at_the_blocks_end() {
    let mut rig = Rig::new(&["hello brave world"]);
    let block = rig.blocks()[0];
    rig.ui.set_caret((block, DocOffset::new(17)));
    rig.type_text("!!");
    let point = rig.point_of(block, 8);
    for at in [0.0, 100.0, 200.0] {
        rig.press(point, false, ambre_core::PageTime(at));
    }
    assert_eq!(
        rig.ui.selection(),
        Some(((block, DocOffset::ZERO), (block, DocOffset::new(19))))
    );
    rig.ui.select_all();
    assert_eq!(
        rig.ui.selection().map(|(_, end)| end),
        Some((block, DocOffset::new(19)))
    );
}

/// `PROSE-UI-233`: a move to the end of a block lands after the text
/// drafted there, before a layout carries it.
#[test]
fn a_move_to_a_blocks_end_lands_after_its_draft() {
    // The key, the block drafted at its end, and the block the caret
    // starts at the start of.
    for (key, drafted, from) in [
        (ProseKey::End, 0, 0),
        (ProseKey::Down, 1, 1),
        (ProseKey::Left, 0, 1),
    ] {
        let mut rig = Rig::new(&["hello", "world"]);
        let blocks = rig.blocks();
        rig.ui.set_caret((blocks[drafted], DocOffset::new(5)));
        rig.type_text("xyz");
        rig.ui.set_caret((blocks[from], DocOffset::ZERO));
        rig.key(key, false, false);
        let caret = rig.caret();
        assert_eq!(
            (caret.block_id, caret.offset),
            (blocks[drafted], DocOffset::new(8)),
            "{key:?}"
        );
    }
}

/// `PROSE-UI-008`: the shared hit-test answers a document
/// offset while text drafted before the point waits for the layout.
#[test]
fn the_hit_test_answers_a_document_offset_while_drafts_wait() {
    let mut rig = Rig::new(&["hello brave world"]);
    let block = rig.blocks()[0];
    let (x, y) = rig.point_of(block, 8);
    rig.ui.set_caret((block, DocOffset::ZERO));
    rig.type_text("ab");
    let hit = rig
        .ui
        .hit_test(ambre_core::ViewPoint::new(ViewPx::new(x), ViewPx::new(y)));
    assert_eq!(hit, Some((block, DocOffset::new(10))));
}

/// `PROSE-UI-072`: a press on a document holding no block asks for one, and
/// the caret goes to it once a layout carries it.
#[test]
fn a_press_on_a_document_with_no_block_asks_for_one() {
    use std::collections::HashMap;
    use std::sync::Arc;
    let mut ui = ProseUiState::default();
    ui.resize(ViewPx::new(1200.0), ViewPx::new(900.0));
    let empty = crate::layout::run::ShapedLayout::new(
        crate::layout::run::LayoutVersion::new(1),
        Arc::new(crate::layout::pass::PageLayout {
            page_width: DocPx::new(800.0),
            page_height: DocPx::new(1000.0),
            page_count: crate::events::PageCount::ONE,
            ..crate::layout::pass::PageLayout::default()
        }),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
    );
    ui.apply_layout(empty);
    let mut sent = Vec::new();
    let press = ProseInput::Press {
        at: view_at(600.0, 100.0),
        mods: ambre_commands::Modifiers::NONE,
        now: ambre_core::PageTime(0.0),
    };
    assert!(ui.handle_input(&press, &mut |command| {
        sent.push(command);
        SendResult::Sent
    }));
    // The structural command is followed by the sync that holds the input.
    assert!(matches!(
        sent.pop(),
        Some(ProseCommand::Sync {
            seen: ambre_bus::app::prose::TextChangeSeq(0)
        })
    ));
    assert!(matches!(
        sent.as_slice(),
        [ProseCommand::InsertBlock {
            init: BlockInit::Paragraph,
            ..
        }]
    ));
    // The view invented no block: there is no caret until one is laid out.
    assert!(ui.cursor.is_none());
    let mut logic = ProseState::with_peer(ambre_core::PeerId(1)).expect("a document is built");
    logic.load_faces(&crate::layout::fonts::shipped_faces());
    let mut shaped = logic.shaped_layout();
    shaped.version = crate::layout::run::LayoutVersion::new(2);
    ui.apply_layout(shaped);
    let first = logic.root_blocks()[0];
    assert_eq!(
        ui.cursor.map(|c| (c.block_id, c.offset)),
        Some((first, DocOffset::ZERO))
    );
}

/// `PROSE-UI-156`: a layout with no page resolves no press, and the caret
/// stays where it was.
#[test]
fn a_press_on_a_layout_with_no_page_leaves_the_caret() {
    let mut ui = ProseUiState::default();
    let block = BlockId::new(ambre_core::PeerId(1), ambre_core::OpCounter(0));
    ui.set_caret((block, DocOffset::new(3)));
    let consumed = ui.handle_input(
        &ProseInput::Press {
            at: view_at(10.0, 10.0),
            mods: ambre_commands::Modifiers::NONE,
            now: ambre_core::PageTime(0.0),
        },
        &mut |_| SendResult::Sent,
    );
    assert!(!consumed);
    assert_eq!(
        ui.cursor.map(|c| (c.block_id, c.offset)),
        Some((block, DocOffset::new(3)))
    );
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
        offset: DocOffset::new(1),
        anchor: Some((second, DocOffset::new(2))),
    });
    assert_eq!(
        rig.ui.selection(),
        Some(((first, DocOffset::new(1)), (second, DocOffset::new(2))))
    );
    rig.key(ProseKey::Delete, false, false);
    assert_eq!(
        rig.sent,
        vec![ProseCommand::DeleteRange {
            anchor: (first, DocOffset::new(1)),
            focus: (second, DocOffset::new(2)),
        }]
    );
    assert_eq!(rig.caret().offset, DocOffset::new(1));
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
        offset: DocOffset::new(11),
        anchor: Some((block, DocOffset::new(6))),
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
    rig.ui.set_caret((block, DocOffset::new(2)));
    rig.key(ProseKey::Right, true, false);
    rig.key(ProseKey::Right, true, false);
    rig.key(ProseKey::Right, true, true);
    let caret = rig.caret();
    assert_eq!(caret.anchor, Some((block, DocOffset::new(2))));
    assert_eq!(caret.offset, DocOffset::new(5));
    rig.key(ProseKey::Left, false, false);
    assert_eq!(rig.caret().anchor, None);
    assert_eq!(rig.caret().offset, DocOffset::new(4));
}

/// The arrows cross the edge of a block into the next one.
#[test]
fn the_arrows_cross_into_the_next_block() {
    let mut rig = Rig::new(&["ab", "cd"]);
    let (first, second) = (rig.blocks()[0], rig.blocks()[1]);
    rig.ui.set_caret((first, DocOffset::new(2)));
    rig.key(ProseKey::Right, false, false);
    assert_eq!(
        (rig.caret().block_id, rig.caret().offset),
        (second, DocOffset::ZERO)
    );
    rig.key(ProseKey::Left, false, false);
    assert_eq!(
        (rig.caret().block_id, rig.caret().offset),
        (first, DocOffset::new(2))
    );
}

/// `PROSE-UI-034`: a run of vertical moves keeps its column across a short
/// line, and returns to it.
#[test]
fn a_run_of_vertical_moves_keeps_its_column() {
    let long = words(6);
    let mut rig = Rig::new(&[&long, "ab", &long]);
    let blocks = rig.blocks();
    rig.ui.set_caret((blocks[0], DocOffset::new(20)));
    rig.key(ProseKey::Down, false, false);
    assert_eq!(
        (rig.caret().block_id, rig.caret().offset),
        (blocks[1], DocOffset::new(2))
    );
    rig.key(ProseKey::Down, false, false);
    assert_eq!(
        (rig.caret().block_id, rig.caret().offset),
        (blocks[2], DocOffset::new(20))
    );
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
    let run_len = rig.ui.run_of(block).expect("a run").len().as_end();
    let second_line = {
        let run = rig.ui.run_of(block).expect("a run");
        assert!(run.line_count().get() > 2);
        run.at_line(crate::layout::run::RunLine::new(1), DocPx::new(5.0))
            .0
    };
    rig.ui
        .set_caret((block, DocOffset::new(second_line.get() + 2)));
    rig.key(ProseKey::End, false, false);
    let end = RunOffset::new(rig.caret().offset.get());
    assert!(end < run_len);
    assert_eq!(
        rig.ui.caret_affinity,
        Affinity::Upstream,
        "the caret stays on the line it ends"
    );
    let run = rig.ui.run_of(block).expect("a run");
    assert_eq!(
        run.line_of(end, Affinity::Upstream),
        crate::layout::run::RunLine::new(1)
    );
    rig.key(ProseKey::Home, false, false);
    let start = RunOffset::new(rig.caret().offset.get());
    assert!(start > RunOffset::ZERO && start <= second_line);
    let run = rig.ui.run_of(block).expect("a run");
    assert_eq!(
        run.line_of(start, Affinity::Downstream),
        crate::layout::run::RunLine::new(1)
    );
}

/// `PROSE-UI-035`, `PROSE-UI-158`: every move leaves the caret on screen,
/// and one that left it scrolls the least that brings it back.
#[test]
fn every_move_leaves_the_caret_on_screen() {
    let long = words(60);
    let paragraphs: Vec<&str> = vec![long.as_str(); 60];
    let mut rig = Rig::new(&paragraphs);
    let first = rig.blocks()[0];
    rig.ui.set_caret((first, DocOffset::ZERO));
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
    assert!((bottom - (rig.ui.viewport.scroll_y + visible)).abs() < DocPx::new(0.01));
}

// -- Dragging -----------------------------------------------------------------

/// `PROSE-UI-037`, `PROSE-UI-161`, `PROSE-UI-162`: the press captures the
/// anchor, the drag moves the other end under the pointer, and the window
/// does not scroll.
#[test]
fn a_drag_grows_from_the_anchor_its_press_captured() {
    let mut rig = Rig::new(&["hello world", "second line"]);
    let (first, second) = (rig.blocks()[0], rig.blocks()[1]);
    rig.press(rig.point_of(first, 3), false, ambre_core::PageTime(0.0));
    let scroll = rig.ui.viewport.scroll_y;
    let (x, y) = rig.point_of(second, 4);
    rig.input(ProseInput::Drag { at: view_at(x, y) });
    assert_eq!(
        rig.ui.selection(),
        Some(((first, DocOffset::new(3)), (second, DocOffset::new(4))))
    );
    // Upward, past the anchor: the anchor holds, the ends swap order.
    let (x, y) = rig.point_of(first, 1);
    rig.input(ProseInput::Drag { at: view_at(x, y) });
    assert_eq!(rig.caret().anchor, Some((first, DocOffset::new(3))));
    assert_eq!(
        rig.ui.selection(),
        Some(((first, DocOffset::new(1)), (first, DocOffset::new(3))))
    );
    assert_eq!(rig.ui.viewport.scroll_y, scroll);
    rig.input(ProseInput::Release);
    let (x, y) = rig.point_of(second, 2);
    assert!(!rig.input(ProseInput::Drag { at: view_at(x, y) }));
}

/// `PROSE-UI-152`: a collaborator's change does not move the anchor of a
/// drag in flight.
#[test]
fn a_remote_change_does_not_move_the_anchor_of_a_drag() {
    let mut rig = Rig::new(&["hello world"]);
    let block = rig.blocks()[0];
    let mut other = a_collaborator(&rig);
    rig.press(rig.point_of(block, 6), false, ambre_core::PageTime(0.0));
    let (x, y) = rig.point_of(block, 9);
    rig.input(ProseInput::Drag { at: view_at(x, y) });
    receive(
        &mut rig,
        &mut other,
        ProseCommand::InsertText {
            block_id: block,
            pos: DocOffset::ZERO,
            text: "abc".to_owned(),
        },
    );
    assert_eq!(rig.caret().anchor, Some((block, DocOffset::new(6))));
}

// -- Escape and select all ----------------------------------------------------

/// `PROSE-UI-164`, `PROSE-UI-165`, `PROSE-UI-166`: Escape collapses the
/// selection to the caret, cancels the drag, and moves neither the caret nor
/// the window.
#[test]
fn escape_collapses_and_cancels_the_drag() {
    let mut rig = Rig::new(&["hello world"]);
    let block = rig.blocks()[0];
    rig.press(rig.point_of(block, 2), false, ambre_core::PageTime(0.0));
    let (x, y) = rig.point_of(block, 7);
    rig.input(ProseInput::Drag { at: view_at(x, y) });
    let scroll = rig.ui.viewport.scroll_y;
    rig.key(ProseKey::Escape, false, false);
    let caret = rig.caret();
    assert_eq!((caret.offset, caret.anchor), (DocOffset::new(7), None));
    assert_eq!(rig.ui.viewport.scroll_y, scroll);
    let (x, y) = rig.point_of(block, 9);
    assert!(!rig.input(ProseInput::Drag { at: view_at(x, y) }));
    assert!(!rig.input(ProseInput::Release));
    assert_eq!(rig.caret().offset, DocOffset::new(7));
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
    rig.ui.set_caret((blocks[3], DocOffset::ZERO));
    rig.ui.scroll_by(down(700.0));
    let scroll = rig.ui.viewport.scroll_y;
    rig.key(ProseKey::SelectAll, false, true);
    let len = rig.ui.run_of(last).expect("a run").len();
    let len = DocOffset::new(len.get());
    assert_eq!(
        rig.ui.selection(),
        Some(((first, DocOffset::ZERO), (last, len)))
    );
    assert_eq!(rig.ui.viewport.scroll_y, scroll);
}

// -- Hit-testing --------------------------------------------------------------

/// `PROSE-UI-008`, `PROSE-UI-031`, `UI-THREAD-001`: a point drawn at an offset
/// resolves back to it through the shared hit-test, at any zoom and scroll.
/// The rig never paints a block.
#[test]
fn a_point_resolves_to_the_offset_drawn_there_under_zoom_and_scroll() {
    let long = words(50);
    let paragraphs: Vec<&str> = vec![long.as_str(); 20];
    let mut rig = Rig::new(&paragraphs);
    let block = rig.blocks()[12];
    for (zoom, scroll) in [(1.0, 0.0), (1.5, 900.0), (0.75, 2400.0)] {
        rig.ui.viewport.zoom = ambre_core::ZoomFactor::positive(f64::from(zoom));
        rig.ui.viewport.scroll_y = DocPx::new(0.0);
        rig.ui.scroll_by(down(scroll * zoom));
        for offset in [0, 9, 42, 120] {
            let (x, y) = rig.point_of(block, offset);
            let hit = rig
                .ui
                .hit_test(ambre_core::ViewPoint::new(ViewPx::new(x), ViewPx::new(y)));
            assert_eq!(hit, Some((block, DocOffset::new(offset))), "zoom {zoom}");
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
    rig.ui.set_caret((second, DocOffset::new(3)));
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
        range: DocSpan::from_usize_range(6..10),
        key: "a".into(),
        value: Some(crate::mark::test_value(TARGET)),
    });
    rig.deliver();
    rig.lay();
    (rig, block)
}

fn press_follow(rig: &mut Rig, (x, y): (f32, f32), follow: bool) -> bool {
    rig.input(ProseInput::Press {
        at: view_at(x, y),
        mods: ambre_commands::Modifiers::NONE.ctrl_if(ambre_commands::KeyState::from_bool(follow)),
        now: ambre_core::PageTime(0.0),
    })
}

/// `PROSE-UI-153`: the link answers over its marked text, and the text beside
/// it on the same line does not.
#[test]
fn a_link_answers_over_its_marked_text_alone() {
    let (rig, block) = linked();
    assert_eq!(rig.ui.links.len(), 1);
    let (x, y) = rig.point_of(block, 7);
    assert_eq!(
        rig.ui.link_at(view_at(x, y)).map(|l| l.url.as_str()),
        Some(TARGET)
    );
    let (x, y) = rig.point_of(block, 2);
    assert_eq!(rig.ui.link_at(view_at(x, y)), None);
    let (x, y) = rig.point_of(block, 12);
    assert_eq!(rig.ui.link_at(view_at(x, y)), None);
}

/// `PROSE-UI-070`, `PROSE-UI-169`: a plain press inside a link places the
/// caret there and asks the host for nothing.
#[test]
fn a_plain_press_inside_a_link_places_the_caret_and_opens_nothing() {
    let (mut rig, block) = linked();
    let at = rig.point_of(block, 8);
    assert!(press_follow(&mut rig, at, false));
    assert_eq!(
        (rig.caret().block_id, rig.caret().offset),
        (block, DocOffset::new(8))
    );
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
        offset: DocOffset::new(2),
        anchor: Some((block, DocOffset::ZERO)),
    });
    let before = rig.ui.cursor;
    let sent = rig.sent.len();
    let at = rig.point_of(block, 8);
    assert!(press_follow(&mut rig, at, true));
    assert_eq!(rig.ui.followed, Some(TARGET.into()));
    assert_eq!(rig.ui.cursor, before);
    assert_eq!(rig.sent.len(), sent, "nothing is written");
    // Off a link, the modifier is an ordinary press.
    let at = rig.point_of(block, 15);
    rig.ui.followed = None;
    assert!(press_follow(&mut rig, at, true));
    assert_eq!(rig.ui.followed, None);
    assert_eq!(rig.caret().offset, DocOffset::new(15));
}

/// `PROSE-UI-186`: a peer's `javascript:` target never reaches the host, and
/// the press that met it moves nothing either.
#[test]
fn a_target_that_could_run_code_is_held_back() {
    let mut rig = Rig::new(&["go to link and back"]);
    let block = rig.blocks()[0];
    rig.outbox.push(ProseCommand::ApplyMark {
        block_id: block,
        range: DocSpan::from_usize_range(6..10),
        key: "a".into(),
        value: Some(crate::mark::test_value("javascript:alert(1)")),
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
    rig.outbox.push(ProseCommand::SetHeaderEnabled {
        shown: crate::events::BandState::On,
    });
    rig.outbox.push(ProseCommand::SetFooterEnabled {
        shown: crate::events::BandState::On,
    });
    rig.deliver();
    let bands = rig.logic.bands();
    let header = bands.header.expect("the header block");
    let footer = bands.footer.expect("the footer block");
    for (block, text) in [(header, "the running head"), (footer, "the running foot")] {
        rig.outbox.push(ProseCommand::InsertText {
            block_id: block,
            pos: DocOffset::ZERO,
            text: text.to_owned(),
        });
    }
    rig.deliver();
    rig.lay();
    (rig, header, footer)
}

/// Where a band's offset is drawn on `page`, as the painter places it.
fn band_point(rig: &Rig, block: BlockId, offset: u32, page: u32) -> (f32, f32) {
    let page = crate::events::PageIndex::new(page);
    let ui = &rig.ui;
    let layout = &ui.shaped.layout;
    let slot = ui
        .band_slots(page)
        .into_iter()
        .find(|slot| slot.block == crate::events::tree_id(block))
        .expect("the band is drawn");
    let run = ui.shaped.blocks[&slot.block].data();
    let caret = run.caret_box(RunOffset::new(offset));
    let x = layout.margin_left + run.indent() + DocPx::new(caret.x0 as f32) + DocPx::new(0.5);
    let y_doc =
        ui.column.page_top_y(page) + slot.top + DocPx::new((caret.y0 + caret.y1) as f32 / 2.0);
    let scale = ui.viewport.scale();
    (
        (ui.column.left(&ui.viewport) + ViewPx::new(x.get() * scale)).get(),
        (y_doc - ui.viewport.scroll_y).get() * scale,
    )
}

/// `PROSE-UI-033`: a press in the header or the footer puts the caret in its
/// text, on the page it was placed on. Typing then goes to that block.
#[test]
fn a_press_in_a_header_or_a_footer_lands_in_its_text() {
    let (mut rig, header, footer) = banded();
    for block in [header, footer] {
        let at = band_point(&rig, block, 4, 0);
        assert!(rig.press(at, false, ambre_core::PageTime(0.0)));
        assert_eq!(
            (rig.caret().block_id, rig.caret().offset),
            (block, DocOffset::new(4))
        );
        assert_eq!(rig.ui.band_page, Some(crate::events::PageIndex::ZERO));
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
            ProseCommand::InsertText { block_id, pos, text } if *block_id == footer && *pos == DocOffset::new(4) && text == "x"
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
    assert!(rig.press(at, false, ambre_core::PageTime(0.0)));
    let (x, y) = rig.point_of(body, 5);
    rig.input(ProseInput::Drag { at: view_at(x, y) });
    assert_eq!(rig.caret().block_id, header);
    assert_eq!(rig.caret().anchor, None);
    // Within the band, a drag selects.
    let (x, y) = band_point(&rig, header, 8, 0);
    rig.input(ProseInput::Drag { at: view_at(x, y) });
    assert_eq!(rig.caret().anchor, Some((header, DocOffset::new(2))));
    assert_eq!(
        rig.ui.selection(),
        Some(((header, DocOffset::new(2)), (header, DocOffset::new(8))))
    );
    // A shift press in the body is a plain press there.
    rig.input(ProseInput::Release);
    rig.ui.last_press = None;
    assert!(rig.press(rig.point_of(body, 5), true, ambre_core::PageTime(0.0)));
    assert_eq!(
        (rig.caret().block_id, rig.caret().offset, rig.caret().anchor),
        (body, DocOffset::new(5), None)
    );
}

/// A document whose one page calls one note, and the note's block.
fn noted() -> (Rig, BlockId, BlockId) {
    let mut rig = Rig::new(&["the body of the page"]);
    let body = rig.blocks()[0];
    rig.outbox.push(ProseCommand::InsertFootnote {
        block_id: body,
        pos: DocOffset::new(4),
        note_text: "the text of the note".to_owned(),
    });
    rig.deliver();
    rig.lay();
    let note = rig
        .ui
        .note_slots(crate::events::PageIndex::new(0))
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
        .note_slots(crate::events::PageIndex::new(0))
        .into_iter()
        .find(|slot| slot.block == crate::events::tree_id(block))
        .expect("the note is drawn");
    let run = ui.shaped.blocks[&slot.block].data();
    let caret = run.caret_box(RunOffset::new(offset));
    let x = layout.margin_left + run.indent() + DocPx::new(caret.x0 as f32) + DocPx::new(0.5);
    let y_doc = ui.column.page_top_y(crate::events::PageIndex::new(0))
        + slot.top
        + DocPx::new((caret.y0 + caret.y1) as f32 / 2.0);
    let scale = ui.viewport.scale();
    (
        (ui.column.left(&ui.viewport) + ViewPx::new(x.get() * scale)).get(),
        (y_doc - ui.viewport.scroll_y).get() * scale,
    )
}

/// `PROSE-UI-187`: a press in a note puts the caret in its text, past the
/// number drawn beside it, and typing goes to the note's block.
#[test]
fn a_press_in_a_note_lands_in_its_text() {
    let (mut rig, _, note) = noted();
    let at = note_point(&rig, note, 4);
    assert!(rig.press(at, false, ambre_core::PageTime(0.0)));
    assert_eq!(
        (rig.caret().block_id, rig.caret().offset),
        (note, DocOffset::new(4))
    );
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
            ProseCommand::InsertText { block_id, pos, text } if *block_id == note && *pos == DocOffset::new(4) && text == "x"
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
    assert!(rig.press(at, false, ambre_core::PageTime(0.0)));
    let (x, y) = rig.point_of(body, 5);
    rig.input(ProseInput::Drag { at: view_at(x, y) });
    assert_eq!(rig.caret().block_id, note);
    assert_eq!(rig.caret().anchor, None);
    let (x, y) = note_point(&rig, note, 8);
    rig.input(ProseInput::Drag { at: view_at(x, y) });
    assert_eq!(
        rig.ui.selection(),
        Some(((note, DocOffset::new(2)), (note, DocOffset::new(8))))
    );
}

// -- The format bar -----------------------------------------------------------

impl Rig {
    /// Select `start..end` of one block, as a drag would leave it.
    fn select(&mut self, block: BlockId, start: u32, end: u32) {
        self.ui.cursor = Some(CursorPosition {
            block_id: block,
            offset: DocOffset::new(end),
            anchor: Some((block, DocOffset::new(start))),
        });
    }

    /// Ask the marks the bar needs, and take the answer, as the host does
    /// on each frame.
    fn read_marks(&mut self) {
        if let Some(command) = self.ui.marks_query() {
            self.ui.marks_sent();
            self.outbox.push(command);
            self.deliver();
        }
    }

    /// Send what a control asks for, then take the new layout.
    fn format(&mut self, id: ambre_commands::CommandId) -> Option<ProseCommand> {
        let command = self.ui.format(id);
        self.take_sent(command.clone().into_iter().collect());
        self.deliver();
        self.lay();
        command
    }

    /// Whether every run of a block carries a mark key.
    fn all_marked(&self, block: BlockId, key: &str) -> bool {
        let runs = self.logic.resolved_runs(&block);
        !runs.is_empty()
            && runs.iter().all(|(_, style)| match key {
                "b" => style.bold,
                "i" => style.italic,
                _ => false,
            })
    }
}

fn marked(rig: &mut Rig, block: BlockId, range: (u32, u32), key: &str) {
    let outcome = rig.logic.handle_command(ProseCommand::ApplyMark {
        block_id: block,
        range: DocSpan::new(DocOffset::new(range.0), DocOffset::new(range.1)),
        key: key.into(),
        value: None,
    });
    assert!(outcome.update.is_some(), "the mark is written");
    rig.lay();
}

/// `PROSE-UI-053`, `PROSE-UI-100`: the bar shows over a non-empty selection
/// alone, and goes when the selection is emptied or cancelled.
#[test]
fn the_bar_shows_over_a_selection_and_goes_with_it() {
    let mut rig = Rig::new(&["hello world"]);
    let block = rig.blocks()[0];
    rig.ui.set_caret((block, DocOffset::new(3)));
    assert!(!rig.ui.toolbar().visible());
    assert_eq!(rig.ui.marks_query(), None);
    rig.key(ProseKey::Right, true, false);
    assert!(rig.ui.toolbar().visible());
    rig.key(ProseKey::Escape, false, false);
    assert!(!rig.ui.toolbar().visible());
    rig.select(block, 0, 5);
    assert!(rig.ui.toolbar().visible());
    rig.key(ProseKey::Right, false, false);
    assert!(!rig.ui.toolbar().visible());
}

/// `PROSE-UI-099`: raising the bar writes nothing, and the selection it was
/// raised on stays whole.
#[test]
fn raising_the_bar_writes_nothing_and_keeps_the_selection() {
    let mut rig = Rig::new(&["hello world"]);
    let block = rig.blocks()[0];
    rig.select(block, 2, 7);
    let before = rig.ui.cursor;
    let query = rig.ui.marks_query().expect("the bar needs its marks");
    let outcome = rig.logic.handle_view_command(query);
    assert!(outcome.update.is_none());
    assert!(
        outcome
            .display
            .iter()
            .all(|event| matches!(event, ProseEvent::SelectionMarks { .. }))
    );
    assert_eq!(rig.ui.cursor, before);
    assert_eq!(rig.logic.text_of(&block).as_deref(), Some("hello world"));
}

/// `PROSE-UI-054`, `PROSE-UI-096`: each control reports the whole
/// selection, and a key carried by part of it reads mixed whichever end
/// carries it.
#[test]
fn a_selection_bold_in_part_reads_mixed() {
    let mut rig = Rig::new(&["hello world", "second"]);
    let (first, second) = (rig.blocks()[0], rig.blocks()[1]);
    marked(&mut rig, first, (0, 5), "b");
    rig.select(first, 0, 11);
    rig.read_marks();
    let marks = rig.ui.toolbar().active.expect("the marks arrived");
    assert_eq!(marks.bold, TriState::Mixed);
    assert_eq!(marks.italic, TriState::Off);
    rig.select(first, 6, 11);
    rig.read_marks();
    assert_eq!(rig.ui.toolbar().active.map(|m| m.bold), Some(TriState::Off));
    rig.select(first, 0, 5);
    rig.read_marks();
    assert_eq!(rig.ui.toolbar().active.map(|m| m.bold), Some(TriState::On));
    // Across two paragraphs: the bold start alone does not make it bold.
    rig.ui.cursor = Some(CursorPosition {
        block_id: second,
        offset: DocOffset::new(3),
        anchor: Some((first, DocOffset::ZERO)),
    });
    rig.read_marks();
    assert_eq!(
        rig.ui.toolbar().active.map(|m| m.bold),
        Some(TriState::Mixed)
    );
}

/// `PROSE-UI-057`, `PROSE-UI-097`: the controls show what the Logic thread
/// read for this very selection, and nothing carries over to the next one.
#[test]
fn the_marks_shown_are_the_ones_read_for_this_selection() {
    let mut rig = Rig::new(&["hello world"]);
    let block = rig.blocks()[0];
    marked(&mut rig, block, (0, 5), "b");
    rig.select(block, 0, 5);
    rig.read_marks();
    assert_eq!(rig.ui.toolbar().active.map(|m| m.bold), Some(TriState::On));
    // Another selection: nothing is shown until its own answer arrives.
    rig.select(block, 6, 11);
    assert_eq!(rig.ui.toolbar().active, None);
    assert!(rig.ui.marks_query().is_some());
    // The same selection again, after it was cancelled: asked afresh.
    rig.select(block, 0, 5);
    rig.key(ProseKey::Escape, false, false);
    assert_eq!(rig.ui.marks_query(), None);
    rig.select(block, 0, 5);
    assert_eq!(rig.ui.toolbar().active, None);
    assert!(rig.ui.marks_query().is_some());
}

/// `PROSE-UI-102`, `PROSE-UI-103`, `PROSE-UI-098`: a mixed control applies
/// its mark to the whole selection, and an on control removes it. One
/// command the plane declares leaves, and one undo takes it back.
#[test]
fn a_mixed_control_applies_its_mark_to_the_whole_selection() {
    let mut rig = Rig::new(&["hello world", "second"]);
    let (first, second) = (rig.blocks()[0], rig.blocks()[1]);
    marked(&mut rig, first, (0, 5), "b");
    rig.ui.cursor = Some(CursorPosition {
        block_id: second,
        offset: DocOffset::new(6),
        anchor: Some((first, DocOffset::ZERO)),
    });
    rig.read_marks();
    let sent = rig.format(crate::commands::BOLD);
    assert_eq!(
        sent,
        Some(ProseCommand::FormatSelection {
            anchor: (first, DocOffset::ZERO),
            focus: (second, DocOffset::new(6)),
            key: "b".into(),
            action: ambre_bus::app::prose::MarkAction::Apply,
        })
    );
    assert!(rig.all_marked(first, "b") && rig.all_marked(second, "b"));
    rig.read_marks();
    assert_eq!(rig.ui.toolbar().active.map(|m| m.bold), Some(TriState::On));
    let sent = rig.format(crate::commands::BOLD);
    assert!(matches!(
        sent,
        Some(ProseCommand::FormatSelection {
            action: ambre_bus::app::prose::MarkAction::Remove,
            ..
        })
    ));
    rig.read_marks();
    assert_eq!(rig.ui.toolbar().active.map(|m| m.bold), Some(TriState::Off));
    // The removal spanned two blocks, and one undo restores both.
    rig.logic.handle_command(ProseCommand::Undo);
    assert!(rig.all_marked(first, "b") && rig.all_marked(second, "b"));
}

/// `PROSE-UI-104`: a control changes neither the text, nor the selection,
/// nor a mark it does not name.
#[test]
fn a_control_leaves_the_text_the_selection_and_other_marks() {
    let mut rig = Rig::new(&["hello world"]);
    let block = rig.blocks()[0];
    marked(&mut rig, block, (0, 11), "i");
    rig.select(block, 2, 9);
    rig.read_marks();
    let before = rig.ui.cursor;
    rig.format(crate::commands::BOLD);
    assert_eq!(rig.logic.text_of(&block).as_deref(), Some("hello world"));
    assert_eq!(rig.ui.cursor, before);
    assert!(rig.all_marked(block, "i"));
}

/// `PROSE-UI-101`: dismissing the bar sends nothing, and a mark applied
/// before stays.
#[test]
fn dismissing_the_bar_writes_nothing() {
    let mut rig = Rig::new(&["hello world"]);
    let block = rig.blocks()[0];
    rig.select(block, 0, 11);
    rig.read_marks();
    rig.format(crate::commands::BOLD);
    let sent = rig.sent.len();
    rig.key(ProseKey::Escape, false, false);
    assert!(!rig.ui.toolbar().visible());
    assert_eq!(rig.sent.len(), sent);
    assert!(rig.all_marked(block, "b"));
}

/// `PROSE-UI-055`: the bar stays over its text while the page scrolls and
/// while it zooms.
#[test]
fn the_bar_follows_its_text_through_a_scroll_and_a_zoom() {
    let paragraphs: Vec<String> = (0..40).map(|_| words(30)).collect();
    let texts: Vec<&str> = paragraphs.iter().map(String::as_str).collect();
    let mut rig = Rig::new(&texts);
    let block = rig.blocks()[3];
    rig.select(block, 5, 9);
    let text_at = |rig: &Rig| rig.point_of(block, 5);
    let anchor = |rig: &Rig| {
        let at = rig.ui.toolbar().anchor.expect("the bar shows");
        (at.x.get(), at.y.get())
    };
    let (x0, y0) = anchor(&rig);
    let (_, line) = text_at(&rig);
    rig.ui.scroll_by(down(120.0));
    let (x1, y1) = anchor(&rig);
    let (_, moved) = text_at(&rig);
    assert!((x1 - x0).abs() < 0.01);
    assert!(((y0 - y1) - (line - moved)).abs() < 0.01);
    assert!(y1 < y0);
    rig.ui.viewport.zoom = ambre_core::ZoomFactor::positive(2.0);
    let (_, zoomed) = anchor(&rig);
    let (_, line) = text_at(&rig);
    // Above the text it formats, by less than one line at this zoom.
    assert!(zoomed <= line && line - zoomed < 40.0);
}

/// `PROSE-UI-185`: a mark sent before the view saw a collaborator's
/// insertion lands on the text it was sent for.
#[test]
fn a_mark_sent_before_a_remote_insertion_lands_on_its_text() {
    let mut rig = Rig::new(&["hello world"]);
    let block = rig.blocks()[0];
    // A collaborator's insertion the Logic side took and the view has not
    // seen yet: its events stay undelivered.
    let mut other = a_collaborator(&rig);
    let update = other
        .handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: DocOffset::ZERO,
            text: ">> ".to_owned(),
        })
        .update
        .expect("the change exported");
    let _unseen = rig.logic.apply_remote(&update);
    // The view marks "world" as it still sees it, at 6..11.
    rig.select(block, 6, 11);
    let _ = rig.format(crate::commands::BOLD);
    let runs = rig.logic.resolved_runs(&block);
    let bold: String = runs
        .iter()
        .filter(|(_, style)| style.bold)
        .map(|(text, _)| text.as_str())
        .collect();
    assert_eq!(bold, "world");
}

/// Put the caret at a place, with no selection.
fn caret_at(rig: &mut Rig, block: BlockId, offset: u32) {
    rig.ui.cursor = Some(CursorPosition {
        block_id: block,
        offset: DocOffset::new(offset),
        anchor: None,
    });
}

/// `PROSE-UI-058`: a slash opens the menu at the start of a block, on an
/// empty one or before its first character, and nowhere else.
#[test]
fn a_slash_opens_the_menu_only_at_the_start_of_a_block() {
    let mut rig = Rig::new(&["hello", ""]);
    let (text, empty) = (rig.blocks()[0], rig.blocks()[1]);
    caret_at(&mut rig, text, 3);
    rig.type_text("/");
    assert!(
        !rig.ui.slash_menu().visible(),
        "a slash mid-word is a slash"
    );
    caret_at(&mut rig, empty, 0);
    rig.type_text("/");
    assert!(rig.ui.slash_menu().visible());
    rig.key(ProseKey::Escape, false, false);
    caret_at(&mut rig, text, 0);
    rig.type_text("/");
    assert!(rig.ui.slash_menu().visible());
}

/// `PROSE-UI-091`, `PROSE-UI-092`, `PROSE-UI-059`: opening records the
/// block and the slash's offset, and the slash and the query leave as
/// ordinary insertions, with nothing else.
#[test]
fn the_slash_and_the_query_reach_the_document_as_text() {
    let mut rig = Rig::new(&[""]);
    let block = rig.blocks()[0];
    caret_at(&mut rig, block, 0);
    rig.type_text("/");
    let origin = rig.ui.slash.as_ref().map(|menu| menu.origin);
    assert_eq!(origin, Some((block, DocOffset::ZERO)));
    rig.type_text("tab");
    rig.deliver();
    assert!(
        rig.sent
            .iter()
            .all(|c| matches!(c, ProseCommand::InsertText { .. }))
    );
    assert_eq!(rig.logic.text_of(&block).as_deref(), Some("/tab"));
    assert_eq!(rig.ui.slash_menu().query, "tab");
}

/// `PROSE-UI-060`, `PROSE-UI-093`, `PROSE-UI-174`, `PROSE-UI-175`: Enter
/// takes the slash and the query out of the origin block. It sends the
/// item's command once, addressed to that block, and closes the menu.
#[test]
fn enter_erases_the_query_then_sends_one_command() {
    let mut rig = Rig::new(&[""]);
    let block = rig.blocks()[0];
    caret_at(&mut rig, block, 0);
    rig.type_text("/");
    rig.type_text("tab");
    rig.deliver();
    let before = rig.sent.len();
    rig.key(ProseKey::Enter, false, false);
    assert!(!rig.ui.slash_menu().visible());
    let sent: Vec<_> = rig.sent.split_off(before);
    assert_eq!(
        sent,
        vec![
            ProseCommand::DeleteText {
                block_id: block,
                range: DocSpan::from_usize_range(0..4),
            },
            ProseCommand::InsertTable {
                parent_id: None,
                after_id: Some(block),
                rows: crate::events::grid::RowCount::new(3),
                cols: crate::events::grid::ColCount::new(3),
            },
        ]
    );
    rig.deliver();
    assert_eq!(rig.logic.text_of(&block).as_deref(), Some(""));
    let roots = rig.logic.root_blocks();
    let next = roots.iter().position(|b| *b == block).map(|i| roots[i + 1]);
    assert_eq!(
        next.and_then(|b| rig.logic.block_type_of(&b)),
        Some(crate::events::BlockType::Table)
    );
}

/// `PROSE-UI-061`, `PROSE-UI-095`: four gestures close the menu: Escape, a
/// space, a press outside and the Backspace over the slash. None of them
/// writes beyond what the key itself does.
#[test]
fn closing_the_menu_leaves_the_text_as_typed() {
    let mut rig = Rig::new(&["", "below"]);
    let (block, below) = (rig.blocks()[0], rig.blocks()[1]);
    let open = |rig: &mut Rig| {
        caret_at(rig, block, 0);
        rig.type_text("/");
        rig.deliver();
        rig.lay();
        assert!(rig.ui.slash_menu().visible());
        rig.sent.len()
    };
    let sent = open(&mut rig);
    rig.key(ProseKey::Escape, false, false);
    assert!(!rig.ui.slash_menu().visible());
    assert_eq!(rig.sent.len(), sent, "Escape sends nothing");
    rig.deliver();
    assert_eq!(rig.logic.text_of(&block).as_deref(), Some("/"));

    // The slash is still there: a second one would sit after it.
    rig.key(ProseKey::Backspace, false, false);
    rig.deliver();
    let sent = open(&mut rig);
    let point = rig.point_of(below, 2);
    rig.press(point, false, ambre_core::PageTime(0.0));
    assert!(!rig.ui.slash_menu().visible());
    assert_eq!(rig.sent.len(), sent, "a press outside sends nothing");

    caret_at(&mut rig, block, 1);
    rig.key(ProseKey::Backspace, false, false);
    rig.deliver();
    open(&mut rig);
    rig.type_text(" ");
    assert!(!rig.ui.slash_menu().visible());
    rig.deliver();
    assert_eq!(rig.logic.text_of(&block).as_deref(), Some("/ "));

    rig.key(ProseKey::Backspace, false, false);
    rig.key(ProseKey::Backspace, false, false);
    rig.deliver();
    open(&mut rig);
    rig.key(ProseKey::Backspace, false, false);
    assert!(!rig.ui.slash_menu().visible());
    rig.deliver();
    assert_eq!(rig.logic.text_of(&block).as_deref(), Some(""));
}

/// `PROSE-UI-089`, `PROSE-UI-087`: the arrows move the menu's selection and
/// send nothing, and the document holds the typed text alone.
#[test]
fn moving_through_the_menu_sends_nothing() {
    let mut rig = Rig::new(&[""]);
    let block = rig.blocks()[0];
    caret_at(&mut rig, block, 0);
    rig.type_text("/");
    rig.type_text("h");
    rig.deliver();
    let sent = rig.sent.len();
    let before = rig.logic.export_snapshot();
    rig.key(ProseKey::Down, false, false);
    rig.key(ProseKey::Down, false, false);
    rig.key(ProseKey::Up, false, false);
    assert_eq!(
        rig.ui.slash_menu().selected,
        crate::view::slash::SlashChoice::new(1)
    );
    assert_eq!(rig.sent.len(), sent);
    rig.deliver();
    assert_eq!(rig.logic.export_snapshot(), before);
    assert_eq!(rig.logic.text_of(&block).as_deref(), Some("/h"));
}

/// `PROSE-UI-090`: a peer sees the typed text and nothing of the menu.
#[test]
fn a_peer_sees_the_slash_and_no_menu() {
    let mut rig = Rig::new(&[""]);
    let block = rig.blocks()[0];
    let mut peer = a_collaborator(&rig);
    caret_at(&mut rig, block, 0);
    rig.type_text("/");
    rig.type_text("ta");
    rig.key(ProseKey::Down, false, false);
    rig.deliver();
    assert!(rig.ui.slash_menu().visible());
    peer.import_snapshot(&rig.logic.export_snapshot())
        .expect("the snapshot imports");
    assert_eq!(peer.text_of(&block).as_deref(), Some("/ta"));
    assert_eq!(peer.root_blocks(), rig.logic.root_blocks());
}

/// `PROSE-UI-094`: an item leaves every other block, and the text after the
/// query, as they were.
#[test]
fn an_item_touches_the_origin_block_alone() {
    let mut rig = Rig::new(&["hello", "other"]);
    let (block, other) = (rig.blocks()[0], rig.blocks()[1]);
    caret_at(&mut rig, block, 0);
    rig.type_text("/");
    rig.type_text("sep");
    rig.deliver();
    rig.key(ProseKey::Enter, false, false);
    rig.deliver();
    assert_eq!(rig.logic.text_of(&block).as_deref(), Some("hello"));
    assert_eq!(rig.logic.text_of(&other).as_deref(), Some("other"));
}

/// `PROSE-UI-062`, `PROSE-UI-088`: every item resolves to a command the
/// plane declares, and the Logic thread carries each one out.
#[test]
fn every_item_is_a_command_the_plane_carries_out() {
    for index in 0..crate::view::slash::ITEMS.len() {
        let mut rig = Rig::new(&[""]);
        let block = rig.blocks()[0];
        let roots = rig.logic.root_blocks().len();
        caret_at(&mut rig, block, 0);
        rig.type_text("/");
        rig.deliver();
        rig.input(ProseInput::SlashPick(
            crate::view::slash::SlashChoice::from_usize(index),
        ));
        rig.deliver();
        let turned = rig.logic.block_type_of(&block) == Some(crate::events::BlockType::Heading);
        assert!(
            turned || rig.logic.root_blocks().len() > roots,
            "item {index} changed nothing"
        );
    }
}

/// `PROSE-UI-193`: a collaborator's insertion before the slash moves the
/// origin, and Enter takes the slash and the query, and nothing else.
#[test]
fn a_remote_insertion_before_the_slash_moves_the_origin() {
    let mut rig = Rig::new(&["tail"]);
    let block = rig.blocks()[0];
    caret_at(&mut rig, block, 0);
    rig.type_text("/");
    rig.type_text("sep");
    rig.deliver();
    let mut other = a_collaborator(&rig);
    receive(
        &mut rig,
        &mut other,
        ProseCommand::InsertText {
            block_id: block,
            pos: DocOffset::ZERO,
            text: "ab".to_owned(),
        },
    );
    assert_eq!(
        rig.ui.slash.as_ref().map(|m| m.origin),
        Some((block, DocOffset::new(2)))
    );
    rig.key(ProseKey::Enter, false, false);
    rig.deliver();
    assert_eq!(rig.logic.text_of(&block).as_deref(), Some("abtail"));
}

/// `PROSE-UI-194`: the query keeps the items whose label holds it, case
/// folded, in the menu's order.
#[test]
fn the_query_narrows_the_items_in_order() {
    let labels = |q: &str| -> Vec<&str> {
        crate::view::slash::filter(q)
            .iter()
            .map(|item| item.label)
            .collect()
    };
    assert_eq!(labels("HEAD"), vec!["Heading 1", "Heading 2", "Heading 3"]);
    assert_eq!(
        labels("list"),
        vec!["Bullet list", "Numbered list", "Task list"]
    );
    assert_eq!(labels("tab"), vec!["Table", "Table of contents"]);
    assert!(labels("zzz").is_empty());
}

/// `PROSE-UI-142`: the composition in progress is drawn at the caret and
/// sends nothing, and the caret stays where the document has it.
#[test]
fn a_composition_in_progress_sends_nothing() {
    let mut rig = Rig::new(&["hello"]);
    let block = rig.blocks()[0];
    rig.ui.set_caret((block, DocOffset::new(5)));
    rig.input(ProseInput::Compose("n".to_owned()));
    rig.input(ProseInput::Compose("ni".to_owned()));
    assert!(rig.sent.is_empty());
    assert_eq!(rig.ui.composing, "ni");
    assert_eq!(rig.caret().offset, DocOffset::new(5));
    assert!(rig.ui.draft.is_empty());
}

/// `PROSE-UI-029`: the composition ends in one insertion of the committed
/// text, and the preview goes.
#[test]
fn a_composition_ends_in_one_insertion() {
    let mut rig = Rig::new(&["hello"]);
    let block = rig.blocks()[0];
    rig.ui.set_caret((block, DocOffset::new(5)));
    rig.input(ProseInput::Compose("ni".to_owned()));
    rig.input(ProseInput::Compose("\u{4f60}".to_owned()));
    rig.input(ProseInput::ComposeEnd("\u{4f60}".to_owned()));
    assert_eq!(
        rig.sent,
        vec![ProseCommand::InsertText {
            block_id: block,
            pos: DocOffset::new(5),
            text: "\u{4f60}".to_owned(),
        }]
    );
    assert!(rig.ui.composing.is_empty());
    assert_eq!(rig.ui.shown_text(block).as_deref(), Some("hello\u{4f60}"));
    rig.deliver();
    rig.lay();
    assert_eq!(rig.logic.text_of(&block).as_deref(), Some("hello\u{4f60}"));
}

/// `PROSE-UI-029`: a collaborator's insertion during the composition moves
/// the caret, and the committed text lands where the author was.
#[test]
fn a_remote_insertion_during_a_composition_moves_its_landing() {
    let mut rig = Rig::new(&["hello"]);
    let block = rig.blocks()[0];
    rig.ui.set_caret((block, DocOffset::new(5)));
    rig.input(ProseInput::Compose("x".to_owned()));
    let mut other = a_collaborator(&rig);
    receive(
        &mut rig,
        &mut other,
        ProseCommand::InsertText {
            block_id: block,
            pos: DocOffset::ZERO,
            text: ">> ".to_owned(),
        },
    );
    rig.input(ProseInput::ComposeEnd("xy".to_owned()));
    rig.deliver();
    rig.lay();
    assert_eq!(rig.logic.text_of(&block).as_deref(), Some(">> helloxy"));
}

/// `PROSE-UI-197`: a composition opened over a selection deletes it before
/// the preview shows, and the committed text takes its place.
#[test]
fn a_composition_over_a_selection_deletes_it_first() {
    let mut rig = Rig::new(&["hello world"]);
    let block = rig.blocks()[0];
    rig.ui.cursor = Some(CursorPosition {
        block_id: block,
        offset: DocOffset::new(11),
        anchor: Some((block, DocOffset::new(6))),
    });
    rig.input(ProseInput::Compose(String::new()));
    assert!(rig.sent.is_empty(), "an empty preview takes nothing");
    rig.input(ProseInput::Compose("n".to_owned()));
    assert_eq!(
        rig.sent,
        vec![ProseCommand::DeleteText {
            block_id: block,
            range: DocSpan::from_usize_range(6..11),
        }]
    );
    assert!(rig.ui.selection().is_none());
    assert_eq!(rig.caret().offset, DocOffset::new(6));
    assert_eq!(rig.ui.composing, "n");
    rig.input(ProseInput::ComposeEnd("\u{4f60}".to_owned()));
    rig.deliver();
    rig.lay();
    assert_eq!(rig.logic.text_of(&block).as_deref(), Some("hello \u{4f60}"));
}

/// `PROSE-UI-198`: past the bound a held key goes, and held text stays,
/// joined to the text before it. A preview replaces the one before it.
#[test]
fn held_text_is_never_dropped() {
    let mut rig = Rig::new(&["hello"]);
    let block = rig.blocks()[0];
    rig.ui.set_caret((block, DocOffset::new(5)));
    rig.key(ProseKey::Enter, false, false);
    for _ in 0..300 {
        rig.key(ProseKey::Right, false, false);
    }
    assert_eq!(rig.ui.queued.len(), 256);
    rig.type_text("a");
    rig.key(ProseKey::Backspace, false, false);
    rig.type_text("b");
    rig.input(ProseInput::Compose("n".to_owned()));
    rig.input(ProseInput::Compose("ni".to_owned()));
    rig.input(ProseInput::ComposeEnd("\u{4f60}".to_owned()));
    rig.type_text("!");
    assert_eq!(rig.ui.queued.len(), 258);
    rig.deliver();
    rig.lay();
    let blocks = rig.blocks();
    assert_eq!(
        rig.logic.text_of(&blocks[1]).as_deref(),
        Some("ab\u{4f60}!")
    );
}

/// `PROSE-UI-044`, `PROSE-UI-111`: a cut asks for the copy before it deletes,
/// and deletes the selection and nothing else.
#[test]
fn a_cut_copies_before_it_deletes_the_selection() {
    let mut rig = Rig::new(&["keep", "hello world"]);
    let [keep, block] = rig.blocks()[..] else {
        panic!("two paragraphs");
    };
    rig.ui.cursor = Some(CursorPosition {
        block_id: block,
        offset: DocOffset::new(5),
        anchor: Some((block, DocOffset::ZERO)),
    });
    assert!(rig.input(ProseInput::Copy(ClipboardWrite::Cut)));
    assert!(matches!(rig.sent[0], ProseCommand::CopyRange { .. }));
    assert!(
        rig.sent.len() > 1,
        "the selection is deleted after the copy"
    );
    rig.deliver();
    assert_eq!(rig.logic.text_of(&block).as_deref(), Some(" world"));
    assert_eq!(rig.logic.text_of(&keep).as_deref(), Some("keep"));
    assert!(
        rig.ui
            .poll_clipboard()
            .is_some_and(|clip| clip.text == "hello")
    );
}

/// `PROSE-UI-110`: a cut whose copy found no room deletes nothing.
#[test]
fn a_refused_copy_deletes_nothing() {
    let mut rig = Rig::new(&["hello world"]);
    rig.ui.select_all();
    let mut sent = Vec::new();
    rig.ui
        .handle_input(&ProseInput::Copy(ClipboardWrite::Cut), &mut |command| {
            sent.push(command);
            SendResult::Dropped
        });
    assert_eq!(sent.len(), 1);
    assert!(matches!(sent[0], ProseCommand::CopyRange { .. }));
}

/// `UI-THREAD-044`: a copy sends the copy request alone and leaves the
/// document as it was.
#[test]
fn a_copy_leaves_the_document_unchanged() {
    let mut rig = Rig::new(&["hello world"]);
    let block = rig.blocks()[0];
    rig.ui.select_all();
    assert!(rig.input(ProseInput::Copy(ClipboardWrite::Copy)));
    assert!(matches!(
        rig.sent.as_slice(),
        [ProseCommand::CopyRange { .. }]
    ));
    rig.deliver();
    assert_eq!(rig.logic.text_of(&block).as_deref(), Some("hello world"));
}

/// `CHROME-062`: no printable character is read as a key, so each one is left
/// to the text path.
#[test]
fn a_printable_character_is_not_a_key() {
    use ambre_commands::Key;
    for key in [
        Key::char('a'),
        Key::char('Z'),
        Key::char('é'),
        Key::Space,
        Key::char('1'),
        Key::char('='),
    ] {
        assert_eq!(
            ProseKey::from_key(key, ambre_commands::Modifiers::NONE),
            None,
            "{key:?}"
        );
    }
}

/// `PROSE-UI-043`: the kept payload serves only on this tab's token, and
/// the same text without it is another copy.
#[test]
fn the_kept_payload_serves_only_its_own_copy() {
    let mut rig = Rig::new(&["hello"]);
    let block = rig.blocks()[0];
    rig.ui.cursor = Some(CursorPosition {
        block_id: block,
        offset: DocOffset::new(5),
        anchor: Some((block, DocOffset::ZERO)),
    });
    rig.input(ProseInput::Copy(ClipboardWrite::Copy));
    rig.deliver();
    let own = |text: &str| ambre_bus::ClipboardBlob {
        text: text.to_owned(),
        html: None,
        ambre: None,
        form: ambre_core::PasteForm::Rich,
    };
    let ProseInput::Paste { rich, .. } = rig.ui.paste_input(own("other"), PasteSource::Own) else {
        panic!("a paste");
    };
    assert!(rich.is_some(), "the token keeps the payload");
    let ProseInput::Paste { rich, text, .. } =
        rig.ui.paste_input(own("hello"), PasteSource::Foreign)
    else {
        panic!("a paste");
    };
    assert!(
        rich.is_none(),
        "the same text without the token is another copy"
    );
    assert_eq!(text, "hello");
    let ProseInput::Paste { rich, text, .. } =
        rig.ui.paste_input(own("other"), PasteSource::Foreign)
    else {
        panic!("a paste");
    };
    assert!(rich.is_none());
    assert_eq!(text, "other");
}

/// `PROSE-UI-013`: a paste leaves as one command, and the view writes no
/// model of it.
#[test]
fn a_paste_leaves_as_a_command() {
    let mut rig = Rig::new(&["ab"]);
    let block = rig.blocks()[0];
    rig.ui.cursor = Some(CursorPosition {
        block_id: block,
        offset: DocOffset::new(1),
        anchor: None,
    });
    rig.input(ProseInput::Paste {
        text: "x\r\ny".to_owned(),
        html: None,
        rich: None,
    });
    assert!(matches!(
        &rig.sent[..],
        [ProseCommand::Paste { text, .. }] if text == "x\ny"
    ));
    rig.deliver();
    assert_eq!(rig.logic.text_of(&block).as_deref(), Some("ax"));
}

/// `PROSE-UI-219`: a one-line paste with an HTML half still leaves as a
/// paste, so its marks reach Logic. The view hands the half on untouched.
#[test]
fn an_html_half_keeps_a_short_paste_off_the_typing_path() {
    let mut rig = Rig::new(&["ab"]);
    let block = rig.blocks()[0];
    rig.ui.cursor = Some(CursorPosition {
        block_id: block,
        offset: DocOffset::new(1),
        anchor: None,
    });
    let blob = ambre_bus::ClipboardBlob {
        text: "x".to_owned(),
        html: Some("<b>x</b>".to_owned()),
        ambre: None,
        form: ambre_core::PasteForm::Rich,
    };
    let input = rig.ui.paste_input(blob, PasteSource::Foreign);
    assert!(matches!(
        &input,
        ProseInput::Paste { html: Some(html), .. } if html == "<b>x</b>"
    ));
    rig.input(input);
    assert!(matches!(
        &rig.sent[..],
        [ProseCommand::Paste { html: Some(html), .. }] if html == "<b>x</b>"
    ));
    rig.deliver();
    assert_eq!(rig.logic.text_of(&block).as_deref(), Some("axb"));
}

/// `PROSE-UI-227`: a plain paste leaves the text alone. The HTML the host
/// read and the payload this tab kept, which the same text would otherwise
/// serve, both stay out of the command.
#[test]
fn a_plain_paste_carries_the_text_alone() {
    let mut rig = Rig::new(&["hello"]);
    let block = rig.blocks()[0];
    rig.ui.cursor = Some(CursorPosition {
        block_id: block,
        offset: DocOffset::new(5),
        anchor: Some((block, DocOffset::ZERO)),
    });
    rig.input(ProseInput::Copy(ClipboardWrite::Copy));
    rig.deliver();
    let read = |form: ambre_core::PasteForm| ambre_bus::ClipboardBlob {
        text: "hello".to_owned(),
        html: Some("<b>hello</b>".to_owned()),
        ambre: None,
        form,
    };
    let ProseInput::Paste { rich, .. } = rig
        .ui
        .paste_input(read(ambre_core::PasteForm::Rich), PasteSource::Own)
    else {
        panic!("a paste");
    };
    assert!(rich.is_some(), "the ordinary paste serves the kept payload");
    let input = rig
        .ui
        .paste_input(read(ambre_core::PasteForm::Plain), PasteSource::Own);
    assert!(matches!(
        &input,
        ProseInput::Paste { text, html: None, rich: None } if text == "hello"
    ));
    rig.sent.clear();
    rig.ui.cursor = Some(CursorPosition {
        block_id: block,
        offset: DocOffset::new(5),
        anchor: None,
    });
    rig.input(input);
    rig.deliver();
    assert_eq!(rig.logic.text_of(&block).as_deref(), Some("hellohello"));
    assert!(
        rig.logic
            .marks_of(&block)
            .is_some_and(|marks| !format!("{marks:?}").contains("\"b\""))
    );
}

/// `PROSE-UI-038`: a drag held past the bottom edge scrolls on each tick,
/// and the selection follows the text being revealed.
#[test]
fn a_drag_past_the_edge_scrolls_and_the_selection_follows() {
    let lines: Vec<String> = (0..80).map(|i| format!("line {i}")).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let mut rig = Rig::new(&refs);
    let first = rig.blocks()[0];
    rig.press(rig.point_of(first, 2), false, ambre_core::PageTime(0.0));
    let (x, _) = rig.point_of(first, 2);
    let bottom = rig.ui.viewport.height + ViewPx::new(40.0);
    rig.input(ProseInput::Drag {
        at: view_at(x, bottom.get()),
    });
    let before = rig.ui.selection().expect("a selection").1;
    assert!(
        !rig.ui.tick_autoscroll(ambre_core::PageTime(0.0)),
        "the first tick only starts the clock"
    );
    let mut now = 0.0;
    for _ in 0..20 {
        now += 16.0;
        assert!(rig.ui.tick_autoscroll(ambre_core::PageTime(now)));
        rig.lay();
    }
    assert!(rig.ui.viewport.scroll_y > DocPx::new(0.0));
    let after = rig.ui.selection().expect("a selection").1;
    assert_ne!(after, before, "the focus follows the revealed text");
    assert_eq!(rig.caret().anchor, Some((first, DocOffset::new(2))));
}

/// `PROSE-UI-163`: once released, no tick scrolls, and nothing is armed.
#[test]
fn a_released_drag_leaves_nothing_armed() {
    let lines: Vec<String> = (0..80).map(|i| format!("line {i}")).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let mut rig = Rig::new(&refs);
    let first = rig.blocks()[0];
    rig.press(rig.point_of(first, 2), false, ambre_core::PageTime(0.0));
    let (x, _) = rig.point_of(first, 2);
    let bottom = rig.ui.viewport.height + ViewPx::new(40.0);
    rig.input(ProseInput::Drag {
        at: view_at(x, bottom.get()),
    });
    rig.ui.tick_autoscroll(ambre_core::PageTime(0.0));
    rig.input(ProseInput::Release);
    let scroll = rig.ui.viewport.scroll_y;
    assert!(!rig.ui.tick_autoscroll(ambre_core::PageTime(16.0)));
    assert!(!rig.ui.tick_autoscroll(ambre_core::PageTime(32.0)));
    assert_eq!(rig.ui.viewport.scroll_y, scroll);
    // Escape disarms a drag in flight the same way.
    rig.press(rig.point_of(first, 2), false, ambre_core::PageTime(1000.0));
    rig.input(ProseInput::Drag {
        at: view_at(x, bottom.get()),
    });
    rig.ui.tick_autoscroll(ambre_core::PageTime(40.0));
    rig.key(ProseKey::Escape, false, false);
    assert!(!rig.ui.tick_autoscroll(ambre_core::PageTime(56.0)));
    // A drag away from the edges arms nothing.
    rig.press(rig.point_of(first, 2), false, ambre_core::PageTime(2000.0));
    let (x, y) = rig.point_of(first, 4);
    rig.input(ProseInput::Drag { at: view_at(x, y) });
    rig.ui.tick_autoscroll(ambre_core::PageTime(80.0));
    assert!(!rig.ui.tick_autoscroll(ambre_core::PageTime(96.0)));
}

/// `PROSE-UI-204`: dropped text goes in at the drop point as one insertion,
/// and the draft shows it before the Logic thread answers.
#[test]
fn dropped_text_is_one_insertion_at_the_drop_point() {
    let mut rig = Rig::new(&["hello world"]);
    let block = rig.blocks()[0];
    rig.press(rig.point_of(block, 0), false, ambre_core::PageTime(0.0));
    let (x, y) = rig.point_of(block, 6);
    assert!(rig.input(ProseInput::Drop {
        at: view_at(x, y),
        text: "big ".to_owned()
    }));
    assert_eq!(
        rig.sent,
        vec![ProseCommand::InsertText {
            block_id: block,
            pos: DocOffset::new(6),
            text: "big ".to_owned()
        }]
    );
    assert_eq!(rig.caret().offset, DocOffset::new(10));
    assert_eq!(rig.caret().anchor, None);
    assert_eq!(rig.ui.shown_text(block).as_deref(), Some("hello big world"));
    rig.deliver();
    assert_eq!(
        rig.logic.text_of(&block).as_deref(),
        Some("hello big world")
    );
}

/// A document long enough to scroll through, laid out and in view.
fn long_rig() -> Rig {
    let lines: Vec<String> = (0..200).map(|i| format!("line {i}")).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    Rig::new(&refs)
}

/// Flick a finger up the canvas at 2 px/ms, and lift it.
fn flick_up(rig: &mut Rig) {
    rig.ui
        .touch_start(view_at(300.0, 600.0), ambre_core::PageTime(0.0));
    for step in 1..=5 {
        rig.ui.touch_move(
            view_at(300.0, 600.0 - 32.0 * step as f32),
            ambre_core::PageTime(16.0 * f64::from(step)),
        );
    }
    assert_eq!(
        rig.ui.touch_end(ambre_core::PageTime(80.0)),
        None,
        "a pan is no tap"
    );
}

/// `PROSE-UI-217`: a flick keeps the column moving, each frame slower, and
/// stops at an end of the column.
#[test]
fn a_flick_slows_down_and_stops_at_the_end_of_the_column() {
    let mut rig = long_rig();
    flick_up(&mut rig);
    let speed = rig.ui.fling_speed().expect("a flick is in motion").get();
    assert!((speed - 2.0).abs() < 0.01, "the release speed, {speed}");
    let mut now = 80.0;
    rig.ui.tick_fling(ambre_core::PageTime(now));
    let (mut last_step, mut scroll) = (DocPx::new(f32::INFINITY), rig.ui.viewport.scroll_y);
    while rig.ui.fling_speed().is_some() {
        now += 16.0;
        rig.ui.tick_fling(ambre_core::PageTime(now));
        rig.lay();
        let step = rig.ui.viewport.scroll_y - scroll;
        assert!(
            step >= DocPx::ZERO && step < last_step,
            "each frame slower than the last"
        );
        (last_step, scroll) = (step, rig.ui.viewport.scroll_y);
        assert!(now < 10_000.0, "the motion stops");
    }
    assert!(!rig.ui.tick_fling(ambre_core::PageTime(now + 16.0)));
    // Near the end of the column, the motion stops there.
    let end = rig.ui.column.max_scroll(&rig.ui.viewport);
    rig.ui.scroll_by(down(
        ((end - rig.ui.viewport.scroll_y - DocPx::new(5.0)) * rig.ui.viewport.scale()).get(),
    ));
    flick_up(&mut rig);
    for frame in 1..=20 {
        rig.ui
            .tick_fling(ambre_core::PageTime(80.0 + 16.0 * f64::from(frame)));
    }
    assert_eq!(rig.ui.viewport.scroll_y, end);
    assert_eq!(rig.ui.fling_speed(), None);
}

/// `PROSE-UI-218`: a press, a key or a new finger stops the motion, and the
/// motion writes nothing.
#[test]
fn a_press_stops_a_flick_and_the_motion_writes_nothing() {
    let mut rig = long_rig();
    let block = rig.blocks()[0];
    flick_up(&mut rig);
    rig.ui.tick_fling(ambre_core::PageTime(80.0));
    rig.ui.tick_fling(ambre_core::PageTime(96.0));
    rig.press(rig.point_of(block, 1), false, ambre_core::PageTime(100.0));
    assert_eq!(rig.ui.fling_speed(), None);
    flick_up(&mut rig);
    rig.key(ProseKey::Right, false, false);
    assert_eq!(rig.ui.fling_speed(), None);
    flick_up(&mut rig);
    rig.ui
        .touch_start(view_at(10.0, 10.0), ambre_core::PageTime(200.0));
    assert_eq!(rig.ui.fling_speed(), None);
    // Panning and flicking send nothing: the sent edits are the press's.
    assert!(rig.sent.is_empty());
    assert!(
        rig.outbox
            .iter()
            .all(|c| matches!(c, ProseCommand::Sync { .. }))
    );
}

/// A pixel delta of `dy`, as a trackpad glide sends it.
fn glide(dy: f64) -> ambre_draw::WheelDelta {
    ambre_draw::WheelDelta::pixels(ambre_draw::WheelVector::new(
        ambre_draw::WheelAmount::new(0.0),
        ambre_draw::WheelAmount::new(dy),
    ))
}

/// A delta of `dy` lines, as a notched wheel sends it.
fn notch(dy: f64) -> ambre_draw::WheelDelta {
    ambre_draw::WheelDelta::new(
        ambre_draw::WheelVector::new(
            ambre_draw::WheelAmount::new(0.0),
            ambre_draw::WheelAmount::new(dy),
        ),
        ambre_draw::WheelDeltaMode::Line,
    )
}

/// `KINETIC-015`: a line delta scrolls the column by the pixels a line is
/// worth, and a page delta by the viewport's height.
#[test]
fn a_line_or_a_page_scrolls_the_column_by_its_pixels() {
    let mut by_line = long_rig();
    by_line.ui.wheel(notch(2.0), ambre_core::PageTime(0.0));
    let mut by_px = long_rig();
    by_px.ui.wheel(
        glide(2.0 * f64::from(ambre_draw::WHEEL_LINE.get())),
        ambre_core::PageTime(0.0),
    );
    assert!(
        by_px.ui.viewport.scroll_y > DocPx::new(0.0),
        "the column moved"
    );
    assert_eq!(by_line.ui.viewport.scroll_y, by_px.ui.viewport.scroll_y);

    let mut by_page = long_rig();
    let page = by_page.ui.viewport.height;
    let page_delta = ambre_draw::WheelDelta::new(
        ambre_draw::WheelVector::new(
            ambre_draw::WheelAmount::new(0.0),
            ambre_draw::WheelAmount::new(1.0),
        ),
        ambre_draw::WheelDeltaMode::Page,
    );
    by_page.ui.wheel(page_delta, ambre_core::PageTime(0.0));
    assert_eq!(by_page.ui.viewport.scroll_y.get(), page.get());
}

/// `KINETIC-002`, `KINETIC-009`: a trackpad glide that pauses keeps the column
/// moving at the glide's speed, and a notch of the same size never does.
#[test]
fn a_paused_glide_keeps_the_column_moving_and_a_notch_never_does() {
    let mut rig = long_rig();
    for step in 0..5 {
        rig.ui
            .wheel(glide(16.0), ambre_core::PageTime(16.0 * f64::from(step)));
    }
    let glided = rig.ui.viewport.scroll_y;
    rig.ui.tick_fling(ambre_core::PageTime(64.0));
    assert_eq!(
        rig.ui.fling_speed(),
        None,
        "no motion while the glide lasts"
    );
    // The frame past the pause launches the motion, and the next one moves.
    rig.ui.tick_fling(ambre_core::PageTime(64.0 + 70.0));
    let speed = rig
        .ui
        .fling_speed()
        .expect("the paused glide carries on")
        .get();
    assert!((speed - 1.0).abs() < 0.05, "the glide's speed, {speed}");
    assert!(rig.ui.tick_fling(ambre_core::PageTime(64.0 + 86.0)));
    assert!(rig.ui.viewport.scroll_y > glided);

    let mut rig = long_rig();
    for step in 0..5 {
        rig.ui
            .wheel(notch(0.4), ambre_core::PageTime(16.0 * f64::from(step)));
    }
    let stepped = rig.ui.viewport.scroll_y;
    assert!(
        !rig.ui.tick_fling(ambre_core::PageTime(64.0 + 70.0)),
        "a notch never coasts"
    );
    assert_eq!(rig.ui.viewport.scroll_y, stepped);
    assert_eq!(rig.ui.fling_speed(), None);
}

/// A finger that stays inside the slop is a tap, and its point comes back.
#[test]
fn a_finger_that_barely_moves_is_a_tap() {
    let mut rig = long_rig();
    rig.ui
        .touch_start(view_at(300.0, 400.0), ambre_core::PageTime(0.0));
    assert!(
        !rig.ui
            .touch_move(view_at(303.0, 402.0), ambre_core::PageTime(10.0))
    );
    assert_eq!(
        rig.ui.touch_end(ambre_core::PageTime(20.0)),
        Some(view_at(300.0, 400.0))
    );
    assert_eq!(rig.ui.viewport.scroll_y, DocPx::new(0.0));
    assert_eq!(rig.ui.fling_speed(), None);
}

// -- The input pipe -----------------------------------------------------------

/// Paint the view once, and answer the caret bar it drew, in canvas pixels.
fn paint_caret(rig: &mut Rig) -> Option<ambre_draw::Rect> {
    let ink = ambre_draw::Color::from_rgba8(20, 20, 20, 255);
    let palette = crate::view::paint::PagePalette {
        sheet: ink,
        sheet_border: ink,
        shadow: ink,
        text: ink,
        accent: ink,
        warning: ink,
        success: ink,
        error: ink,
    };
    let mut surface = ambre_draw::testing::RecordingSurface::new();
    rig.ui.paint(&mut surface, &palette);
    rig.ui.caret_drawn
}

/// The caret bar a paint draws, where the rig has one.
fn caret_drawn(rig: &mut Rig) -> ambre_draw::Rect {
    paint_caret(rig).expect("the paint drew a caret")
}

/// `CHROME-007`: the paint reports the caret where it drew it, at any zoom and
/// scroll. The report follows the caret as the text grows before it.
#[test]
fn the_pipe_spot_is_the_caret_the_paint_drew() {
    let long = words(50);
    let paragraphs: Vec<&str> = vec![long.as_str(); 20];
    let mut rig = Rig::new(&paragraphs);
    let block = rig.blocks()[12];
    for (zoom, scroll) in [(1.0, 0.0), (1.5, 900.0), (0.75, 2400.0)] {
        rig.ui.viewport.zoom = ambre_core::ZoomFactor::positive(f64::from(zoom));
        rig.ui.viewport.scroll_y = DocPx::new(0.0);
        rig.ui.scroll_by(down(scroll * zoom));
        for offset in [0, 9, 42] {
            caret_at(&mut rig, block, offset);
            let (x, y) = rig.point_of(block, offset);
            let bar = caret_drawn(&mut rig);
            let half = 0.5 * f64::from(rig.ui.viewport.scale());
            assert!((bar.x0 - (f64::from(x) - half)).abs() < 0.01, "zoom {zoom}");
            assert!(
                bar.y0 < f64::from(y) && f64::from(y) < bar.y1,
                "zoom {zoom}"
            );
        }
    }
    let before = caret_drawn(&mut rig).x0;
    rig.type_text("abc");
    rig.deliver();
    rig.lay();
    assert!(caret_drawn(&mut rig).x0 > before, "the caret moved right");

    // No caret, no spot: the host leaves the pipe where it last was.
    rig.ui.cursor = None;
    assert_eq!(paint_caret(&mut rig), None);
}

// -- History ----------------------------------------------------------------

/// Press one chord the way the host hands it over, and return what it sent.
fn press_chord(rig: &mut Rig, chord: ambre_commands::KeyBinding) -> Vec<ProseCommand> {
    let key = ProseKey::from_key(chord.key, chord.mods).expect("the view decodes the chord");
    let before = rig.sent.len();
    assert!(rig.key(key, chord.mods.shift(), chord.mods.primary()));
    rig.sent[before..].to_vec()
}

/// `UNDO-010`, `UNDO-011`: the view decodes each chord the table declares
/// for undo and redo, and sends the one history command. The text reverts,
/// then comes back, once the Logic thread answers.
#[test]
fn the_history_chords_undo_and_redo_the_typing() {
    let chords = |id| {
        ambre_commands::find(crate::commands::COMMANDS, id)
            .expect("declared")
            .keybindings
    };
    let mut rig = Rig::new(&["hello"]);
    let block = rig.blocks()[0];
    rig.ui.set_caret((block, DocOffset::new(5)));
    assert!(rig.type_text(" world"));
    rig.deliver();
    let text = |rig: &Rig| rig.logic.text_of(&block);
    assert_eq!(text(&rig).as_deref(), Some("hello world"));

    for chord in chords(crate::commands::UNDO) {
        assert_eq!(press_chord(&mut rig, *chord), [ProseCommand::Undo]);
        rig.deliver();
        assert_eq!(text(&rig).as_deref(), Some("hello"));
        rig.key(ProseKey::Redo, false, true);
        rig.deliver();
        assert_eq!(text(&rig).as_deref(), Some("hello world"));
    }
    for chord in chords(crate::commands::REDO) {
        rig.key(ProseKey::Undo, false, true);
        rig.deliver();
        assert_eq!(text(&rig).as_deref(), Some("hello"));
        assert_eq!(press_chord(&mut rig, *chord), [ProseCommand::Redo]);
        rig.deliver();
        assert_eq!(text(&rig).as_deref(), Some("hello world"));
    }
}
