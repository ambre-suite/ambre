//! Operations whose budget is held by a count of instructions, not by a time.
//!
//! A time measured on a shared CI runner moves by tens of percent between two
//! runs, so a threshold on it fails on noise. An instruction count under
//! `callgrind` is stable, so `scripts/instructions.py` fails the build when an
//! operation outgrows its ceiling (`ambre-spec-testing.md section 2`).
//!
//! Each operation runs once, inside a function that is never inlined and whose
//! name is unique. The script names the operation on the command line and tells
//! `callgrind` to count only inside that function, so the setup costs nothing.
//! Run with no operation named, as `cargo bench` does, every operation runs
//! once: a smoke test.

#![expect(
    clippy::expect_used,
    reason = "a bench aborts on a fixture it cannot run"
)]
#![cfg_attr(
    feature = "pdf",
    expect(
        clippy::panic,
        reason = "the deck fixture aborts on a board it cannot build"
    )
)]

// The deck of the PDF export, which only the `pdf` feature records.
#[cfg(feature = "pdf")]
mod common;

use ambre_core::hash::HashMap;
use ambre_core::{Board, Px};
use std::hint::black_box;

use ambre_canvas::{TextCache, Theme};
use ambre_core::id::{Element, Id};
use ambre_draw::FontContext;
use ambre_draw::testing::RecordingSurface;
use ambre_toile::logic::command::DisplayViewport;
use ambre_toile::message::{DisplayElement, DisplayGeom, DisplayNode, ElementStyle, ShapeKind};
use ambre_toile::ui::render::{Scene, paint_content};
use ambre_toile::ui::viewport::ViewportTransform;
use ambre_toile::ui::walk::BoardWalk;
use ambre_toile::{ElementKind, InitContent, ToileCommand, ToileEvent, ToileOutcome, ToileState};
use kurbo::{Affine, Point, Rect, Size};

/// Characters in the body the "Toile body commit" row names: 4 KB of ASCII.
const BODY_LEN: usize = 4096;

/// A body of `BODY_LEN` characters, words of five letters and a space.
fn body() -> String {
    (0..BODY_LEN)
        .map(|i| {
            if i % 6 == 5 {
                ' '
            } else {
                char::from(b'a' + (i % 26) as u8)
            }
        })
        .collect()
}

/// `text` with the characters `range` replaced by `with`.
fn replace(text: &str, range: std::ops::Range<usize>, with: &str) -> String {
    let mut out: Vec<char> = text.chars().collect();
    out.splice(range, with.chars());
    out.into_iter().collect()
}

/// `text` with every `stride`-th character of `range` replaced by `with`.
fn scatter(text: &str, range: std::ops::Range<usize>, stride: usize, with: char) -> String {
    text.chars()
        .enumerate()
        .map(|(i, c)| {
            if range.contains(&i) && i % stride == 0 {
                with
            } else {
                c
            }
        })
        .collect()
}

/// A board holding one sticky whose body is `base`, then a peer's edit of it.
/// The peer is pinned: `ToileState::new` draws one at random, and the ids it
/// mints would move the count from run to run (`ambre-spec-testing.md
/// section 2`).
fn board(base: &str, stored: &str) -> (ToileState, Id<Element>) {
    let mut st = ToileState::with_peer(ambre_core::PeerId::new(1));
    let out = st.handle_command(ToileCommand::CreateElement {
        kind: ElementKind::Sticky,
        at: Point::new(0.0, 0.0),
        size: Size::new(400.0, 400.0),
        content: InitContent::Text(base.to_owned()),
        fill: None,
        correlate: None,
    });
    let id = out
        .display
        .iter()
        .find_map(|e| match e {
            ToileEvent::ElementCreated { id, .. } => Some(id.clone()),
            _ => None,
        })
        .expect("a create names its element");
    st.handle_command(ToileCommand::SetText {
        id: id.clone(),
        base: base.to_owned(),
        text: stored.to_owned(),
    });
    (st, id)
}

/// Close an editor on `typed`, opened on `base`, over a board a peer moved to
/// `stored` meanwhile.
fn close(base: String, stored: &str, typed: String, measured: fn(&mut ToileState, ToileCommand)) {
    let (mut st, id) = board(&base, stored);
    measured(
        &mut st,
        black_box(ToileCommand::SetText {
            id,
            base,
            text: typed,
        }),
    );
}

/// One `SetText`, derivation and delta included. Counted alone by `callgrind`.
#[inline(never)]
fn ambre_budget_toile_body_commit(st: &mut ToileState, command: ToileCommand) {
    let out: ToileOutcome = st.handle_command(command);
    assert!(out.update.is_some(), "the close writes the merged body");
}

/// The same close, counted under another name for the bound of the cap.
#[inline(never)]
fn ambre_bound_toile_body_commit(st: &mut ToileState, command: ToileCommand) {
    let out: ToileOutcome = st.handle_command(command);
    assert!(out.update.is_some(), "the close writes the merged body");
}

/// The close the budget row names. While the editor was open, a peer retyped
/// a word and added a sentence in the first quarter. The author fixed a word
/// in the second half and added a sentence at the end.
fn body_commit() {
    let base = body();
    let stored = replace(&base, 600..605, "PEERS");
    let stored = replace(&stored, 900..900, "A sentence a peer added. ");
    let typed = replace(&base, 2400..2405, "FIXED");
    let typed = format!("{typed} And a sentence the author typed before leaving.");
    close(base, &stored, typed, ambre_budget_toile_body_commit);
}

/// The worst close the cap lets through: every 13th character of each half
/// changes, one half per side. Each change is two edit steps, so each diff
/// runs to its 256-step cap before it gives up (`TOILE-UI-381`).
fn body_commit_at_cap() {
    let base = body();
    let stored = scatter(&base, 0..BODY_LEN / 2, 13, '#');
    let typed = scatter(&base, BODY_LEN / 2..BODY_LEN, 13, '@');
    close(base, &stored, typed, ambre_bound_toile_body_commit);
}

/// Elements the "Toile board paint" row names, all of them visible.
const BOARD_ELEMENTS: usize = 2000;

/// One visible element of the busy board: every fourth a sticky holding a
/// word, the rest shapes of the three kinds.
fn busy_node(i: usize) -> DisplayNode {
    let (x, y) = ((i % 50) as f64 * 24.0, (i / 50) as f64 * 24.0);
    let (kind, geom) = match i % 4 {
        0 => (
            ElementKind::Sticky,
            DisplayGeom::Sticky(format!("note {i}")),
        ),
        1 => (ElementKind::Shape(ShapeKind::Ellipse), DisplayGeom::Ellipse),
        2 => (
            ElementKind::Shape(ShapeKind::Triangle),
            DisplayGeom::Triangle,
        ),
        _ => (ElementKind::Shape(ShapeKind::Rect), DisplayGeom::Rect),
    };
    DisplayNode::Element(DisplayElement {
        id: Id::<Element>::from_text(format!("e{i}")),
        kind,
        transform: Affine::IDENTITY,
        bounds: Rect::new(x, y, x + 20.0, y + 20.0),
        locked: ambre_toile::message::LockState::Unlocked,
        style: ElementStyle::default(),
        geom,
        link: None,
        group: None,
    })
}

/// The whole content walk of one frame. Counted alone by `callgrind`.
#[inline(never)]
fn ambre_budget_toile_board_paint(
    viewport: &DisplayViewport,
    camera: &ViewportTransform,
    font_cx: &mut FontContext,
    cache: &mut TextCache,
) -> usize {
    walk(viewport, camera, font_cx, cache)
}

/// The content walk onto a recording surface, returning the ops it drew.
fn walk(
    viewport: &DisplayViewport,
    camera: &ViewportTransform,
    font_cx: &mut FontContext,
    cache: &mut TextCache,
) -> usize {
    let mut surface = RecordingSurface::new();
    paint_content(
        &mut surface,
        camera,
        Scene {
            grid: ambre_toile::ui::grid::GridShown::Shown,
            viewport,
            palette: Theme::Light.palette(),
            images: &HashMap::default(),
            anim: &HashMap::default(),
        },
        BoardWalk {
            font_cx,
            text_cache: cache,
        },
        None,
        ambre_toile::message::PresentMode::Editing,
    );
    surface.ops().len()
}

/// A frame of the busy board the row names, the text cache warm from the frame
/// before, as it is while a board is panned.
fn board_paint() {
    let viewport = DisplayViewport {
        version: ambre_core::Seq::ZERO,
        grid_unit: Px::<Board>::new(20.0),
        snap: ambre_toile::message::GridSnap::Free,
        ground: ambre_toile::message::BoardGround::Theme,
        reactions: Default::default(),
        pins: Vec::new(),
        viewport: Default::default(),
        nodes: (0..BOARD_ELEMENTS).map(busy_node).collect(),
    };
    let camera = ViewportTransform::new(ambre_draw::Size::new(1280.0, 1000.0));
    let mut font_cx = ambre_draw::sealed_font_context();
    let face = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../ambre-app/fonts/NotoSans-Regular.ttf"
    );
    let bytes = std::fs::read(face).expect("the workspace ships its faces");
    ambre_draw::register_face(
        &mut font_cx,
        &ambre_draw::FontFace::new(ambre_core::Payload::new(bytes)),
    );
    let mut cache = TextCache::new();
    // The frame before, outside the counted function: it fills the cache.
    walk(&viewport, &camera, &mut font_cx, &mut cache);
    let ops =
        ambre_budget_toile_board_paint(black_box(&viewport), &camera, &mut font_cx, &mut cache);
    assert!(ops >= BOARD_ELEMENTS, "every element was drawn");
}

/// A board of `BOARD_ELEMENTS` stickies, each placed by its own command, and
/// one of them moved: the edit an undo takes back.
/// The peer is pinned, as in `board`.
fn moved_board() -> ToileState {
    let mut st = ToileState::with_peer(ambre_core::PeerId::new(1));
    let mut last = None;
    for i in 0..BOARD_ELEMENTS {
        let out = st.handle_command(ToileCommand::CreateElement {
            kind: ElementKind::Sticky,
            at: Point::new((i % 50) as f64 * 220.0, (i / 50) as f64 * 220.0),
            size: Size::new(200.0, 200.0),
            content: InitContent::Text(format!("note {i}")),
            fill: None,
            correlate: None,
        });
        last = out.display.iter().find_map(|e| match e {
            ToileEvent::ElementCreated { id, .. } => Some(id.clone()),
            _ => None,
        });
    }
    let id = last.expect("a create names its element");
    st.handle_command(ToileCommand::MoveElements {
        ids: vec![id],
        delta: kurbo::Vec2::new(40.0, 0.0),
    });
    st
}

/// One `Undo` of a move, from the command to its outcome: the replay, the
/// commit, the refresh of what it wrote and the delta. Counted alone.
#[inline(never)]
fn ambre_budget_toile_move_undo(st: &mut ToileState) {
    let out = st.handle_command(black_box(ToileCommand::Undo));
    assert!(out.update.is_some(), "the undo writes the old place back");
}

/// One `Redo` of that move, counted the same way.
#[inline(never)]
fn ambre_budget_toile_move_redo(st: &mut ToileState) {
    let out = st.handle_command(black_box(ToileCommand::Redo));
    assert!(out.update.is_some(), "the redo writes the new place again");
}

/// The undo of a move on a board of `BOARD_ELEMENTS` elements, the
/// "Toile transform command" row of `ambre-spec-budgets.md section 1`.
fn move_undo() {
    let mut st = moved_board();
    ambre_budget_toile_move_undo(&mut st);
}

/// The redo of that move, after its undo outside the count.
fn move_redo() {
    let mut st = moved_board();
    let out = st.handle_command(ToileCommand::Undo);
    assert!(out.update.is_some(), "the undo writes the old place back");
    ambre_budget_toile_move_redo(&mut st);
}

/// One page of a running deck export, recorded through the screen walk: what
/// an edit of the board waits behind (`TOILE-IO-099`). Each call is counted
/// apart, up to the dump at the entry of [`ambre_pass_end_toile_deck_page`].
#[cfg(feature = "pdf")]
#[inline(never)]
fn ambre_budget_toile_deck_page(
    recording: &mut ambre_toile::logic::interop::DeckRecording,
    st: &ToileState,
    fonts: &mut FontContext,
) {
    recording.record_next(st, fonts);
    black_box(recording.is_recorded());
}

/// The end of one counted page, and how far the export got after it. Never
/// inlined: the dump at its entry closes the count of the page.
#[cfg(feature = "pdf")]
#[inline(never)]
fn ambre_pass_end_toile_deck_page(
    recording: &ambre_toile::logic::interop::DeckRecording,
) -> ambre_bus::interop::ConversionProgress {
    black_box(recording).progress()
}

/// Every page of the export of the 50-slide deck, on the shipped faces, each
/// counted apart: the "Toile PDF export, one slice" row of
/// `ambre-spec-budgets.md section 1`. The first page shapes its runs cold.
#[cfg(feature = "pdf")]
fn deck_page() {
    let st = common::deck(common::SLIDES);
    let mut fonts = common::shipped_fonts();
    let mut recording = st
        .record_deck(Default::default())
        .expect("the deck has pages");
    while !recording.is_recorded() {
        ambre_budget_toile_deck_page(&mut recording, &st, &mut fonts);
        ambre_pass_end_toile_deck_page(&recording);
    }
    let document = recording.into_document();
    assert_eq!(document.pages.len(), common::SLIDES, "one page per slide");
    assert!(!document.faces.is_empty(), "the pages shape their text");
}

/// The whole pass of the request of a deck export: what an edit typed at the
/// request waits behind. It reads the slides, the pictures and the version
/// of the board. It asks for no stored file, since the deck draws no
/// picture. Then it frames, derives and records the first page
/// (`ambre-spec-toile-pdf-export.md section 3`, step 1). The font context is
/// seeded outside the count.
#[cfg(feature = "pdf")]
#[inline(never)]
fn ambre_budget_toile_deck_request(
    st: &ToileState,
    fonts: &mut FontContext,
) -> ambre_toile::logic::interop::DeckRecording {
    let mut recording = st
        .record_deck(Default::default())
        .expect("the deck has pages");
    assert!(
        recording.fetch_full().is_empty(),
        "the deck draws no picture"
    );
    recording.record_next(st, fonts);
    black_box(recording)
}

/// The same pass for the larger deck, in a function of its own. Each
/// operation of `scripts/instructions.py` counts in a symbol no other
/// operation matches. The call keeps the body apart from the one above, so
/// the linker merges neither symbol into the other.
#[cfg(feature = "pdf")]
#[inline(never)]
fn ambre_budget_toile_large_deck_request(
    st: &ToileState,
    fonts: &mut FontContext,
) -> ambre_toile::logic::interop::DeckRecording {
    black_box(ambre_budget_toile_deck_request(st, fonts))
}

/// The request of the export of a deck of `slides` slides, on the shipped
/// faces: the "Toile PDF export, one slice" row of
/// `ambre-spec-budgets.md section 1`. `request` is the counted function.
#[cfg(feature = "pdf")]
fn deck_request_of(
    slides: usize,
    request: fn(&ToileState, &mut FontContext) -> ambre_toile::logic::interop::DeckRecording,
) {
    let st = common::deck(slides);
    let mut fonts = common::shipped_fonts();
    let recording = request(&st, &mut fonts);
    assert!(
        !recording.is_recorded(),
        "the request records the first page, not the last"
    );
}

/// The request of the export of the 50-slide deck.
#[cfg(feature = "pdf")]
fn deck_request() {
    deck_request_of(common::SLIDES, ambre_budget_toile_deck_request);
}

/// The slides of the deck that shows the request flat: four times
/// [`common::SLIDES`].
#[cfg(feature = "pdf")]
const LARGE_SLIDES: usize = 200;

/// The request of a deck four times larger. Its count stays near the
/// count of the 50-slide deck: the request reads no element that the first
/// page leaves out (`TOILE-IO-099`).
#[cfg(feature = "pdf")]
fn deck_request_large() {
    deck_request_of(LARGE_SLIDES, ambre_budget_toile_large_deck_request);
}

/// Without the export compiled in there is no page to count: the script then
/// reads a count of zero, which fails like an overrun.
#[cfg(not(feature = "pdf"))]
fn deck_page() {}

/// The step after an edit landed mid-export, counted whole. It copies the
/// board at the version of the request (`TOILE-IO-133`). Then it derives and
/// records its page from the copy.
#[cfg(feature = "pdf")]
#[inline(never)]
fn ambre_budget_toile_deck_freeze(
    recording: &mut ambre_toile::logic::interop::DeckRecording,
    st: &ToileState,
    fonts: &mut FontContext,
) {
    recording.record_next(st, fonts);
    // Not `is_recorded`: a body equal to the page function's would merge
    // the two symbols into one, and this one would count nothing.
    black_box(recording.progress());
}

/// The export of the 50-slide deck, with a shape dropped after its first
/// page. The step of the second page builds the copy of the board.
#[cfg(feature = "pdf")]
fn deck_freeze() {
    let mut st = common::deck(common::SLIDES);
    let mut fonts = common::shipped_fonts();
    let mut recording = ambre_budget_toile_deck_request(&st, &mut fonts);
    let out = st.handle_command(ToileCommand::CreateElement {
        kind: ElementKind::Shape(ShapeKind::Rect),
        at: Point::new(1_900.0, 100.0),
        size: Size::new(50.0, 50.0),
        content: InitContent::None,
        fill: None,
        correlate: None,
    });
    assert!(out.update.is_some(), "the drop writes the board");
    ambre_budget_toile_deck_freeze(&mut recording, &st, &mut fonts);
}

/// Without the export compiled in there is no request to count: zero again.
#[cfg(not(feature = "pdf"))]
fn deck_request() {}

/// Nor a request of the larger deck.
#[cfg(not(feature = "pdf"))]
fn deck_request_large() {}

/// Nor a step to count after an edit.
#[cfg(not(feature = "pdf"))]
fn deck_freeze() {}

/// Each operation, by the name `scripts/instructions.py` passes.
const OPERATIONS: [(&str, fn()); 9] = [
    ("body_commit", body_commit),
    ("body_commit_at_cap", body_commit_at_cap),
    ("board_paint", board_paint),
    ("move_undo", move_undo),
    ("move_redo", move_redo),
    ("deck_page", deck_page),
    ("deck_request", deck_request),
    ("deck_request_large", deck_request_large),
    ("deck_freeze", deck_freeze),
];

fn main() {
    // `cargo bench` passes flags such as `--bench`; only a bare word names an
    // operation.
    let named: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .collect();
    for (name, run) in OPERATIONS {
        if named.is_empty() || named.iter().any(|n| n == name) {
            run();
        }
    }
}
