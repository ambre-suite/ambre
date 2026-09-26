// ambre-toile/src/ui/comments.rs
// ============================================================================
// The comment pins and the panel's pure model (toile-comments). A pin is a
// screen-space marker over the board: one badge per thread whose anchor
// resolves in the current camera. The badge carries the message count and the
// resolved state the derived viewport hands it, never a body (`TOILE-205`).
//
// * A pin keeps one screen size through the whole zoom range (`TOILE-UI-022`).
// * A pin paints after every content and overlay pass, so nothing on the board
//   hides it (`TOILE-UI-306`).
// * The pick answers before the element hit-test, so a pin over a note takes
//   the click and the note under it stays unselected (`TOILE-UI-308`).
//
// Pure over `DrawSurface` and over plain geometry. The painter is therefore
// host-testable against `RecordingSurface` with no GPU, and the pick and the
// badge are ordinary unit tests. The wasm-only Leptos panel (`chrome::comments`)
// consumes the model types declared here.
// ============================================================================

use ambre_bus::app::toile::{CommentAnchor, CommentPin, DisplayViewport, ThreadId};
use ambre_canvas::{Palette, TextCache};
use ambre_draw::{
    Affine, Alignment, Brush, Color, DrawSurface, FontContext, Point, PositionedLayoutItem, Rect,
    Stroke,
};
use kurbo::{Circle, Shape};

/// Pin radius in logical pixels. Constant in screen space: the marker keeps one
/// size whatever the zoom (`TOILE-UI-022`).
const PIN_RADIUS: f64 = 11.0;
/// Badge font size, in logical pixels.
const PIN_FONT_PX: f32 = 11.0;
/// The pin's tail: the marker sits this far above and right of its anchor point,
/// so it flags the anchor instead of covering it.
const PIN_OFFSET: f64 = 10.0;

/// What the badge of a pin reads (`TOILE-UI-024`): the message count, or a check
/// mark once a reader resolves the thread. Readable from the board alone,
/// without opening the thread or the panel.
///
/// A count over 9 reads `9+`: the marker is a constant 22 logical pixels across,
/// and three digits do not fit in it.
pub fn pin_badge(pin: &CommentPin) -> String {
    if pin.resolved {
        return "v".to_string();
    }
    match pin.messages {
        0..=9 => pin.messages.to_string(),
        _ => "9+".to_string(),
    }
}

/// The pin's screen-space circle for `at` (world), under camera transform `w2s`.
/// The radius is in screen pixels, so it does not scale with the zoom
/// (`TOILE-UI-022`).
pub fn pin_circle(at: Point, w2s: &Affine) -> Circle {
    let p = *w2s * at;
    Circle::new(Point::new(p.x + PIN_OFFSET, p.y - PIN_OFFSET), PIN_RADIUS)
}

/// The thread whose pin covers `screen`, topmost last-painted first, or `None`
/// when the click missed every pin.
///
/// The caller runs this **before** the element hit-test. A pin over a note
/// therefore takes the click, and the note under it stays unselected
/// (`TOILE-UI-082`, `TOILE-UI-308`).
pub fn hit_pin(pins: &[CommentPin], w2s: &Affine, screen: Point) -> Option<ThreadId> {
    pins.iter()
        .rev()
        .find(|pin| {
            let c = pin_circle(pin.at, w2s);
            (screen - c.center).hypot() <= c.radius
        })
        .map(|pin| pin.thread.clone())
}

/// What a left press on the board resolves to before any gesture starts
/// (`TOILE-UI-082`, `TOILE-UI-308`). A pin answers first, so a press on one
/// opens its thread and never reaches the element under it. Everything else is
/// the board's own press.
#[derive(Clone, Debug, PartialEq)]
pub enum BoardPress {
    /// The press landed on this thread's pin.
    Thread(ThreadId),
    /// The press is the board's: a gesture, a selection, a placement.
    Board,
}

/// Resolve a press at `screen`. Only a left press with the Select tool picks a
/// pin. A creation tool still places where it clicks, and the right button
/// opens the menu. Pure, so a host test covers the order the view applies,
/// rather than the pointer path burying it.
pub fn press_target(
    select_tool: bool,
    button: i16,
    pins: &[CommentPin],
    w2s: &Affine,
    screen: Point,
) -> BoardPress {
    if !select_tool || button != 0 {
        return BoardPress::Board;
    }
    match hit_pin(pins, w2s, screen) {
        Some(thread) => BoardPress::Thread(thread),
        None => BoardPress::Board,
    }
}

/// Paint every pin of the frame. Called last in the overlay pass, so no board
/// content and no other overlay covers a pin (`TOILE-UI-306`). Pure over
/// `DrawSurface`: host-testable.
pub fn paint_pins<S: DrawSurface>(
    surface: &mut S,
    vp: &DisplayViewport,
    w2s: &Affine,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
    palette: &Palette,
) {
    for pin in &vp.pins {
        paint_pin(surface, pin, w2s, font_cx, text_cache, palette);
    }
}

/// One pin: a filled disc with a hairline border, and its badge centred in it.
fn paint_pin<S: DrawSurface>(
    surface: &mut S,
    pin: &CommentPin,
    w2s: &Affine,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
    palette: &Palette,
) {
    let circle = pin_circle(pin.at, w2s);
    let path = circle.to_path(0.1);
    // A resolved thread reads as background, an open one as an accent. The
    // board therefore says which is which at a glance (`TOILE-UI-024`).
    let fill = if pin.resolved {
        palette.header_bg
    } else {
        palette.selection
    };
    surface.fill_path(&path, &Brush::Solid(fill));
    surface.stroke_path(
        &path,
        &Brush::Solid(palette.header_border),
        &Stroke::new(1.0),
    );

    let label = pin_badge(pin);
    let layout = text_cache.get_or_build(
        font_cx,
        &label,
        PIN_FONT_PX,
        1000.0,
        Alignment::Start,
        false,
        false,
    );
    let (w, h) = (layout.width() as f64, layout.height() as f64);
    let origin = Point::new(circle.center.x - w / 2.0, circle.center.y - h / 2.0);
    let brush = Brush::Solid(badge_color(palette, pin.resolved));
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            let tx = origin.x + run.offset() as f64;
            let ty = origin.y + run.baseline() as f64;
            surface.draw_glyphs(&run, &brush, Affine::translate((tx, ty)));
        }
    }
}

/// The badge colour over each of the two pin fills.
fn badge_color(palette: &Palette, resolved: bool) -> Color {
    if resolved {
        palette.cell_text
    } else {
        // The open fill is the accent, so the badge takes the content ground
        // back: the two are the theme's own contrasting pair.
        palette.content_bg
    }
}

/// A thread the panel offers, filtered by the panel's open / resolved switch
/// (`toile-comments` section 4). Pure so the filter is host-tested rather than
/// buried in the wasm-only component.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ThreadFilter {
    /// Every thread of the board.
    #[default]
    All,
    /// The threads still open.
    Open,
    /// The threads already resolved.
    Resolved,
}

impl ThreadFilter {
    /// Whether a thread with this `resolved` flag belongs in the filtered list.
    pub fn keeps(self, resolved: bool) -> bool {
        match self {
            ThreadFilter::All => true,
            ThreadFilter::Open => !resolved,
            ThreadFilter::Resolved => resolved,
        }
    }
}

/// A composer the comment tool just opened at a click (`TOILE-UI-307`). It
/// carries where the thread will hang, and where on screen the field must
/// appear, so the user types where they clicked. Local view state: the thread
/// exists only once the user posts the first message.
#[derive(Clone, Debug, PartialEq)]
pub struct CommentDraft {
    pub anchor: CommentAnchor,
    /// The composer's top-left in CSS pixels, for the DOM field.
    pub at: Point,
}

/// What the panel and the composer ask the canvas view to do
/// (`TOILE-UI-023`). The DOM holds no board state, so the view drains every
/// intent: it owns the camera and the command queue.
#[derive(Clone, Debug, PartialEq)]
pub enum CommentIntent {
    /// Post the draft composer's first message, opening the thread.
    Post(String),
    /// Abandon the draft composer without opening a thread.
    Discard,
    /// Append a message to an open thread.
    Reply { thread: ThreadId, body: String },
    /// Resolve a thread, or reopen it.
    Resolve { thread: ThreadId, resolved: bool },
    /// Open a thread from the board-wide list, bringing the camera to its anchor
    /// (`TOILE-UI-309`).
    Focus {
        thread: ThreadId,
        anchor: CommentAnchor,
    },
}

/// The signals the canvas view and the DOM panel share (`toile-comments`
/// section 4). The panel holds no board state of its own (`TOILE-UI-023`). The
/// view writes what it read back from Logic, and reads the intents the panel
/// emits, exactly like the style inspector's model / restyle pair.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy)]
pub struct CommentSignals {
    /// Whether the panel is open (the View menu and a pin click set it).
    pub panel: leptos::prelude::RwSignal<bool>,
    /// The board-wide thread list, re-read on every change.
    pub list: leptos::prelude::RwSignal<Vec<ambre_bus::app::toile::CommentSummary>>,
    /// The thread the panel currently shows whole, bodies included.
    pub open: leptos::prelude::RwSignal<Option<ambre_bus::app::toile::CommentThread>>,
    /// The composer the comment tool just dropped (`TOILE-UI-307`).
    pub draft: leptos::prelude::RwSignal<Option<CommentDraft>>,
    /// The panel's next intent, drained by the view each frame.
    pub intent: leptos::prelude::RwSignal<Option<CommentIntent>>,
}

/// The screen rectangle a pin occupies, for the tests and for any caller that
/// needs the marker's box rather than its circle.
pub fn pin_rect(at: Point, w2s: &Affine) -> Rect {
    let c = pin_circle(at, w2s);
    Rect::new(
        c.center.x - c.radius,
        c.center.y - c.radius,
        c.center.x + c.radius,
        c.center.y + c.radius,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::app::toile::{ElementId, Viewport};
    use ambre_canvas::theme::LIGHT;
    use ambre_draw::testing::{DrawOp, RecordingSurface};

    fn pin(id: &str, at: Point, messages: u32, resolved: bool) -> CommentPin {
        CommentPin {
            thread: ElementId(id.to_owned()),
            at,
            messages,
            resolved,
        }
    }

    fn viewport(pins: Vec<CommentPin>) -> DisplayViewport {
        DisplayViewport {
            viewport: Viewport::default(),
            nodes: Vec::new(),
            reactions: Default::default(),
            pins,
        }
    }

    /// A pin is the same size at every zoom: the camera moves where it sits, not
    /// how big it is (`TOILE-UI-022`).
    #[test]
    fn a_pin_keeps_one_screen_size_through_the_zoom_range() {
        let at = Point::new(40.0, 40.0);
        let mut widths = Vec::new();
        for zoom in [0.1, 1.0, 4.0, 10.0] {
            let w2s = Affine::scale(zoom);
            widths.push(pin_rect(at, &w2s).width());
        }
        for w in &widths {
            assert_eq!(*w, widths[0], "pin width must not follow the zoom");
        }
        assert_eq!(widths[0], 2.0 * PIN_RADIUS);
    }

    /// The pick answers the pin under the click, and nothing when the click
    /// misses it: the caller then falls through to the elements
    /// (`TOILE-UI-308`).
    #[test]
    fn a_click_on_a_pin_names_its_thread_and_a_miss_names_none() {
        let w2s = Affine::IDENTITY;
        let pins = vec![pin("t1", Point::new(100.0, 100.0), 2, false)];
        let centre = pin_circle(pins[0].at, &w2s).center;
        assert_eq!(
            hit_pin(&pins, &w2s, centre),
            Some(ElementId("t1".to_owned()))
        );
        let outside = Point::new(centre.x + PIN_RADIUS + 2.0, centre.y);
        assert_eq!(hit_pin(&pins, &w2s, outside), None);
    }

    /// Two overlapping pins: the click reaches the one painted last, so the pick
    /// matches what the user sees on top.
    #[test]
    fn overlapping_pins_pick_the_topmost() {
        let w2s = Affine::IDENTITY;
        let at = Point::new(10.0, 10.0);
        let pins = vec![pin("under", at, 1, false), pin("over", at, 1, false)];
        assert_eq!(
            hit_pin(&pins, &w2s, pin_circle(at, &w2s).center),
            Some(ElementId("over".to_owned()))
        );
    }

    /// The badge reads the thread's state from the board alone. It shows the
    /// count while the thread is open, a mark once it closes, and never three
    /// digits (`TOILE-UI-024`).
    #[test]
    fn the_badge_reads_the_count_and_the_resolved_state() {
        assert_eq!(pin_badge(&pin("t", Point::ZERO, 3, false)), "3");
        assert_eq!(pin_badge(&pin("t", Point::ZERO, 42, false)), "9+");
        assert_eq!(pin_badge(&pin("t", Point::ZERO, 42, true)), "v");
    }

    /// Every pin of the frame paints a disc and its badge glyphs.
    #[test]
    fn every_pin_paints_a_marker_and_a_badge() {
        let mut font_cx = FontContext::new();
        let mut text_cache = TextCache::new(1.0);
        let mut surface = RecordingSurface::new();
        let vp = viewport(vec![
            pin("t1", Point::new(0.0, 0.0), 1, false),
            pin("t2", Point::new(60.0, 0.0), 2, true),
        ]);
        paint_pins(
            &mut surface,
            &vp,
            &Affine::IDENTITY,
            &mut font_cx,
            &mut text_cache,
            &LIGHT,
        );
        let fills = surface
            .ops()
            .iter()
            .filter(|op| matches!(op, DrawOp::FillPath { .. }))
            .count();
        assert_eq!(fills, 2, "one disc per pin");
        assert!(
            surface
                .ops()
                .iter()
                .any(|op| matches!(op, DrawOp::Glyphs { .. })),
            "the badge paints glyphs"
        );
    }

    /// A press on a pin resolves to its thread, and never reaches the element
    /// under the pin. Only the board's own press selects (`TOILE-UI-308`).
    #[test]
    fn a_pin_takes_the_press_before_the_board_does() {
        let w2s = Affine::IDENTITY;
        let at = Point::new(50.0, 50.0);
        let pins = vec![pin("t1", at, 1, false)];
        let on_pin = pin_circle(at, &w2s).center;
        assert_eq!(
            press_target(true, 0, &pins, &w2s, on_pin),
            BoardPress::Thread(ElementId("t1".to_owned()))
        );
        // Off the pin, with another button, or with a creation tool armed: the
        // press is the board's.
        assert_eq!(
            press_target(true, 0, &pins, &w2s, Point::new(400.0, 400.0)),
            BoardPress::Board
        );
        assert_eq!(
            press_target(true, 2, &pins, &w2s, on_pin),
            BoardPress::Board
        );
        assert_eq!(
            press_target(false, 0, &pins, &w2s, on_pin),
            BoardPress::Board
        );
    }

    /// The panel's filter is the only thing that hides a thread from the list,
    /// and it hides nothing by default.
    #[test]
    fn the_filter_keeps_what_it_names() {
        assert!(ThreadFilter::All.keeps(true) && ThreadFilter::All.keeps(false));
        assert!(ThreadFilter::Open.keeps(false) && !ThreadFilter::Open.keeps(true));
        assert!(ThreadFilter::Resolved.keeps(true) && !ThreadFilter::Resolved.keeps(false));
    }
}
