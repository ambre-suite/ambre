// ambre-canvas/src/frame.rs
// ============================================================================
// The decisions one animation frame takes, apart from the device that carries
// them out. The frame driver (`FrameHost`, in `ambre-app`) asks these, then
// encodes and presents. Kept here, on every target, so a host test reads them.
//
//   RafControl     the gate a modal closes to freeze the canvas.
//   drive_frame    one tick: the relay always, the frame only while running.
//   Tick           what a frame does once the process may be fatal.
//   Phases         which layers a repaint plan rebuilds, and the present.
//   paint_fatal    the terminal screen, on any drawing surface.
// ============================================================================

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use ambre_core::ViewPx;
use ambre_draw::{Affine, Alignment, Brush, Color, DrawSurface, Point, PositionedLayoutItem, Rect};
use parley::FontContext;

use crate::render::{ContentRepaint, OverlayRepaint};
use crate::text_cache::TextCache;
use crate::theme::{FONT_BODY, FONT_HEADING};

/// What covers the canvas: its own document, or the launcher.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cover {
    Document,
    Launcher,
}

/// Handle to gate the rAF loop, so the chrome can freeze the canvas behind a
/// fullscreen `<dialog>` or the launcher. While suspended the loop keeps
/// ticking but does no work: no dispatch, no render. The canvas holds its last
/// frame, a `backdrop-filter: blur()` over it is free, and the GPU stays idle.
///
/// Two writers share it, and neither may undo the other. The route sets
/// `away` while the launcher covers the canvas. Each open modal takes a hold
/// and releases it on close. The loop runs with no hold and not away, so a
/// modal closed at home leaves the loop suspended.
///
/// Backed by atomics so it is `Send + Sync` and can ride inside a Leptos
/// `Callback`. A literal `cancelAnimationFrame` would stop the loop, and
/// resuming it needs the `!Send` JS closure, which cannot cross that callback.
#[derive(Clone)]
pub struct RafControl {
    away: Arc<AtomicBool>,
    holds: Arc<AtomicU32>,
}

impl Default for RafControl {
    fn default() -> Self {
        Self::new()
    }
}

impl RafControl {
    /// A control that starts running: not away, no hold.
    pub fn new() -> Self {
        Self {
            away: Arc::new(AtomicBool::new(false)),
            holds: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Record whether the launcher covers the canvas.
    pub fn set_cover(&self, cover: Cover) {
        self.away.store(cover == Cover::Launcher, Ordering::Relaxed);
    }

    /// Take a hold: an open modal freezes the canvas.
    pub fn hold(&self) {
        self.holds.fetch_add(1, Ordering::Relaxed);
    }

    /// Release one hold. A release with none taken stays at zero.
    pub fn release(&self) {
        let _ = self
            .holds
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                Some(n.saturating_sub(1))
            });
    }

    /// Whether the frame runs on the next tick.
    pub fn is_running(&self) -> bool {
        !self.away.load(Ordering::Relaxed) && self.holds.load(Ordering::Relaxed) == 0
    }
}

/// Run one tick of the loop.
///
/// The `relay` (Logic to Sync, the dispatcher, the route poll) runs on every
/// tick, paused or not. The `frame` drains the view's inbound queue and draws.
/// Paused, it runs not at all (`CANVAS-058`): no dispatch, no render, and so
/// no present, which leaves the last frame on screen (`CANVAS-012`).
pub fn drive_frame(control: &RafControl, relay: impl FnOnce(), frame: impl FnOnce()) {
    relay();
    if control.is_running() {
        frame();
    }
}

/// Run one timer tick of a page that may be hidden (`CANVAS-075`).
///
/// A hidden page gets no animation frame, so the relay would stop with it:
/// the threads would hear nothing until the page shows again. A timer runs
/// it then. A shown page leaves the relay to its frames.
pub fn drive_hidden(hidden: bool, relay: impl FnOnce()) {
    if hidden {
        relay();
    }
}

/// What a frame does, given the process-wide fatal latch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tick {
    /// The ordinary frame: dispatch, plan, paint, present.
    Render,
    /// The terminal screen, painted once in place of the last good frame.
    PaintFatal,
    /// Nothing: the terminal screen is already up.
    Idle,
}

/// Whether the process is still running, or has latched a fatal error
/// (`CANVAS-014`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Health {
    Running,
    Fatal,
}

/// Whether the terminal screen of a fatal process is up yet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FatalScreen {
    #[default]
    Unpainted,
    Painted,
}

impl Tick {
    /// The frame to run. Fatal wins over everything, and the terminal screen
    /// is painted once (`CANVAS-014`). A fatal process never dispatches
    /// again: the thread feeding the view may be gone.
    pub fn of(health: Health, screen: FatalScreen) -> Self {
        match (health, screen) {
            (Health::Running, _) => Tick::Render,
            (Health::Fatal, FatalScreen::Unpainted) => Tick::PaintFatal,
            (Health::Fatal, FatalScreen::Painted) => Tick::Idle,
        }
    }
}

/// What a frame does with each layer, from its repaint plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Phases {
    /// Rebuild the content layer's CPU batch, and so upload it.
    pub build_content: bool,
    /// Rebuild the overlay layer's CPU batch, and so upload it.
    pub build_overlay: bool,
    /// Encode the composite and present it.
    pub present: bool,
}

impl Phases {
    /// The phases a plan asks for. A layer planned `Reuse` is neither built
    /// nor uploaded (`CANVAS-003`). The present runs on every frame: the
    /// swapchain texture is fresh and undefined each time (`CANVAS-010`).
    pub fn of<P>(content: ContentRepaint<P>, overlay: OverlayRepaint) -> Self {
        Self {
            build_content: !matches!(content, ContentRepaint::Reuse),
            build_overlay: matches!(overlay, OverlayRepaint::Redraw),
            present: true,
        }
    }

    /// Whether any layer is rebuilt this frame.
    pub fn any_build(self) -> bool {
        self.build_content || self.build_overlay
    }
}

/// The fatal screen's backdrop, which its frame also clears to.
pub const FATAL_BACKDROP: Color = Color::from_rgba8(0x1a, 0x1a, 0x2e, 255);

/// Paint the unrecoverable-error terminal screen over a `width` x `height`
/// logical surface: a dark backdrop, a centered title and `message` below.
///
/// The colours are fixed, never the theme's: the fatal path reads no state
/// that a dead thread may have left half-written. It is the canvas twin of the
/// `window.ambreFatal` DOM overlay.
pub fn paint_fatal(
    surface: &mut impl DrawSurface,
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    (width, height): (f32, f32),
    message: &str,
) {
    const TITLE: Color = Color::from_rgba8(0xe0, 0x5a, 0x5a, 255);
    const BODY: Color = Color::from_rgba8(0x9a, 0xa0, 0xa6, 255);
    const TITLE_TEXT: &str = "Ambre encountered a fatal error and must reload.";

    surface.fill_rect(
        Rect::new(0.0, 0.0, f64::from(width), f64::from(height)),
        FATAL_BACKDROP,
    );
    // A centered column at most 640 logical pixels wide, the stack centered.
    let column = ViewPx::new((width * 0.8).min(640.0));
    let x = (f64::from(width) - f64::from(column.get())) / 2.0;
    let heading = FONT_HEADING;
    let body = FONT_BODY;
    let title_h = text_height(font_cx, cache, TITLE_TEXT, heading, column);
    let body_h = text_height(font_cx, cache, message, body, column);
    let gap = 16.0;
    let title_y = (f64::from(height) - (title_h + gap + body_h)) / 2.0;
    let lines = [
        (TITLE_TEXT, heading, title_y, TITLE),
        (message, body, title_y + title_h + gap, BODY),
    ];
    for (text, size, y, color) in lines {
        draw_centered(
            surface,
            font_cx,
            cache,
            text,
            size,
            column,
            Point::new(x, y),
            color,
        );
    }
}

/// The height `text` lays out to over a `width` column.
fn text_height(
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    text: &str,
    size: ViewPx,
    width: ViewPx,
) -> f64 {
    let layout = cache.get_or_build(
        font_cx,
        text,
        size,
        width,
        Alignment::Center,
        crate::Weight::Regular,
        crate::Slant::Upright,
    );
    f64::from(layout.height())
}

/// Draw `text` as centered glyph runs over a `width` column at `origin`.
#[allow(clippy::too_many_arguments)]
fn draw_centered(
    surface: &mut impl DrawSurface,
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    text: &str,
    size: ViewPx,
    width: ViewPx,
    origin: Point,
    color: Color,
) {
    if text.is_empty() {
        return;
    }
    let layout = cache.get_or_build(
        font_cx,
        text,
        size,
        width,
        Alignment::Center,
        crate::Weight::Regular,
        crate::Slant::Upright,
    );
    let brush = Brush::Solid(color);
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            let at = (
                origin.x + f64::from(run.offset()),
                origin.y + f64::from(run.baseline()),
            );
            surface.draw_glyphs(&run, &brush, Affine::translate(at));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_draw::testing::{DrawOp, RecordingSurface};
    use std::cell::Cell;

    /// A paused loop runs the relay and neither dispatches nor renders
    /// (`CANVAS-058`). It presents nothing, so the surface keeps the last frame
    /// (`CANVAS-012`).
    ///
    /// Skipping the render alone would drain the inbound queue into a surface
    /// nobody repaints.
    #[test]
    fn a_paused_loop_relays_and_does_no_frame_work() {
        let control = RafControl::new();
        let (relayed, framed) = (Cell::new(0), Cell::new(0));
        let tick = |c: &RafControl| {
            drive_frame(
                c,
                || relayed.set(relayed.get() + 1),
                || framed.set(framed.get() + 1),
            );
        };
        tick(&control);
        control.hold();
        tick(&control);
        tick(&control);
        assert_eq!((relayed.get(), framed.get()), (3, 1), "paused: relay only");
        control.release();
        tick(&control);
        assert_eq!(framed.get(), 2, "a release takes effect on the next tick");
    }

    /// Each tick drains the Logic to Sync queue once, before the draw
    /// (`ARCH-007`). The relay is the host's drain, so its order is the
    /// drain's order.
    #[test]
    fn each_tick_relays_once_before_it_draws() {
        let control = RafControl::new();
        let order = std::cell::RefCell::new(Vec::new());
        for _ in 0..3 {
            drive_frame(
                &control,
                || order.borrow_mut().push("relay"),
                || order.borrow_mut().push("draw"),
            );
        }
        assert_eq!(
            order.into_inner(),
            ["relay", "draw", "relay", "draw", "relay", "draw"]
        );
    }

    /// A hidden page relays on its timer, and a shown one does not, so the
    /// relay never runs twice in one frame (`CANVAS-075`).
    #[test]
    fn a_hidden_page_relays_on_its_timer_and_a_shown_one_does_not() {
        let relayed = Cell::new(0);
        for hidden in [true, false, true] {
            drive_hidden(hidden, || relayed.set(relayed.get() + 1));
        }
        assert_eq!(relayed.get(), 2);
    }

    /// An open modal suspends the loop and its close resumes it (`CHROME-089`).
    ///
    /// Closing it at home leaves the loop suspended, since the launcher still
    /// covers the canvas (`CHROME-042`). With one shared flag, the close would
    /// resume a loop the route had paused.
    #[test]
    fn a_closed_modal_resumes_only_what_it_suspended() {
        let control = RafControl::new();
        control.hold();
        assert!(!control.is_running(), "a modal suspends the loop");
        control.release();
        assert!(control.is_running(), "its close resumes the loop");

        control.set_cover(Cover::Launcher);
        assert!(!control.is_running(), "the launcher suspends the loop");
        control.hold();
        control.release();
        assert!(!control.is_running(), "a modal closed at home");
        control.set_cover(Cover::Document);
        assert!(control.is_running(), "back in a document");

        control.hold();
        control.hold();
        control.release();
        assert!(!control.is_running(), "one of two modals still open");
        control.release();
        control.release();
        assert!(control.is_running(), "an extra release stays at zero");
    }

    /// Once fatal, the frame paints the terminal screen once, then idles
    /// (`CANVAS-014`).
    #[test]
    fn a_fatal_process_paints_its_screen_once_then_idles() {
        assert_eq!(
            Tick::of(Health::Running, FatalScreen::Unpainted),
            Tick::Render
        );
        assert_eq!(
            Tick::of(Health::Fatal, FatalScreen::Unpainted),
            Tick::PaintFatal
        );
        assert_eq!(Tick::of(Health::Fatal, FatalScreen::Painted), Tick::Idle);
    }

    /// The terminal screen covers the whole surface and draws its title and
    /// the message over it (`CANVAS-014`).
    ///
    /// Holding the last good frame instead is the failure that looks like
    /// nothing is wrong.
    #[test]
    fn the_fatal_screen_covers_the_surface_and_names_the_error() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../ambre-app/fonts/");
        let face = std::fs::read(format!("{dir}NotoSans-Regular.ttf")).unwrap_or_default();
        let mut font_cx = crate::fonts::font_context_from_bytes([face]);
        let mut cache = TextCache::new(1.0);
        let mut surface = RecordingSurface::new();
        paint_fatal(
            &mut surface,
            &mut font_cx,
            &mut cache,
            (800.0, 600.0),
            "Logic panicked",
        );

        let Some(DrawOp::FillRect { rect, .. }) = surface.ops().first() else {
            panic!("the backdrop comes first");
        };
        assert_eq!(*rect, Rect::new(0.0, 0.0, 800.0, 600.0));
        let runs = surface
            .count(|op| matches!(op, DrawOp::Glyphs { glyph_count, .. } if *glyph_count > 0));
        assert!(runs >= 2, "the title and the message are drawn");
    }

    /// A layer planned `Reuse` is not built, and so not uploaded
    /// (`CANVAS-003`); the present runs whatever the plan (`CANVAS-010`).
    #[test]
    fn a_reused_layer_is_skipped_and_the_present_always_runs() {
        let contents = [
            ContentRepaint::Full,
            ContentRepaint::Scroll {
                dx: ambre_core::ViewPx::ZERO,
                dy: ambre_core::ViewPx::new(4.0),
            },
            ContentRepaint::Reuse,
        ];
        for content in contents {
            for overlay in [OverlayRepaint::Redraw, OverlayRepaint::Reuse] {
                let phases = Phases::of(content, overlay);
                assert!(
                    phases.present,
                    "{content:?} / {overlay:?} skipped the present"
                );
                let reused = matches!(content, ContentRepaint::Reuse);
                assert_eq!(phases.build_content, !reused);
                assert_eq!(
                    phases.build_overlay,
                    matches!(overlay, OverlayRepaint::Redraw)
                );
            }
        }
        let idle = Phases::of(
            ContentRepaint::<ambre_core::ViewPx>::Reuse,
            OverlayRepaint::Reuse,
        );
        assert!(!idle.any_build(), "an idle frame builds no batch");
    }
}
