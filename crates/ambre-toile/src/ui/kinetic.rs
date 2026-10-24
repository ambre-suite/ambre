//! The board on the shared kinetic scroll
//! (`3-Toile/features/ambre-spec-toile-viewport.md section 3`).
//!
//! The model (`ambre_draw::kinetic`) hands back how far the view moves, in
//! screen pixels. The board moves its camera by it, the content going the
//! other way, so the zoom never changes how far a flick travels.

use ambre_core::{CssVector, ViewPx};
use ambre_draw::kinetic::{EdgeSpeed, Kinetic, ScrollStep};
use ambre_draw::wheel::WheelDeltaMode;
use kurbo::Point;

use super::tool::Tool;
use super::viewport::ViewportTransform;

/// What a finger on the board does for the length of its touch
/// (`TOILE-UI-378`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FingerRole {
    /// The finger drags the board, and a flick coasts.
    Pans,
    /// The finger acts as the mouse: it grabs, selects or draws.
    ActsAsMouse,
}

/// What lies under a finger as it presses: the empty board, or an element or
/// handle it can grab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Under {
    Empty,
    Grabbable,
}

impl Under {
    /// `Grabbable` when `grabbable` holds, `Empty` otherwise.
    pub fn grabbable_if(grabbable: bool) -> Self {
        if grabbable {
            Under::Grabbable
        } else {
            Under::Empty
        }
    }
}

/// The role of a finger pressed with `tool` over `under`. The hand always
/// pans. The select tool pans on the empty board, and grabs what it can grab;
/// a drawing tool always draws.
pub fn finger_role(tool: Tool, under: Under) -> FingerRole {
    match tool {
        Tool::Hand => FingerRole::Pans,
        Tool::Select if under == Under::Empty => FingerRole::Pans,
        _ => FingerRole::ActsAsMouse,
    }
}

/// A wheel event that pans the board: the content moves against the delta,
/// as a page scrolls, and a glide coasts once it pauses (`TOILE-UI-289`).
/// The `delta` is in pixels already; `mode` is the unit the event came in.
pub fn pan_by_wheel(
    cam: &mut ViewportTransform,
    kinetic: &mut Kinetic,
    delta: CssVector,
    mode: WheelDeltaMode,
    now: ambre_core::PageTime,
) {
    let step = kinetic.wheel(delta, mode, now);
    move_view(cam, step);
}

/// Advance a flick or a glide by one frame (`KINETIC-004`). Answers whether
/// the board moved, so the view knows to ask for what came into sight.
pub fn coast(
    cam: &mut ViewportTransform,
    kinetic: &mut Kinetic,
    now: ambre_core::PageTime,
) -> bool {
    let Some(step) = kinetic.tick(now) else {
        return false;
    };
    move_view(cam, step);
    true
}

/// Move the camera by a view step of the model.
pub fn move_view(cam: &mut ViewportTransform, step: ScrollStep) {
    cam.pan_by_screen(step);
}

/// The speed a gesture held at `at` pans the board at, in screen pixels per
/// millisecond, on each axis (`KINETIC-007`). Zero away from the edges.
pub fn edge_pan_speed(cam: &ViewportTransform, at: Point) -> (EdgeSpeed, EdgeSpeed) {
    // A screen point of the board is a view pixel, in the `f64` kurbo takes.
    #[allow(clippy::cast_possible_truncation)]
    let screen_px = |at: f64| ViewPx::new(at as f32);
    (
        EdgeSpeed::at(screen_px(at.x), screen_px(cam.size.width)),
        EdgeSpeed::at(screen_px(at.y), screen_px(cam.size.height)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_core::CssPx;

    /// `TOILE-UI-378`, `TOILE-UI-379`: a finger pans the empty board with the select tool,
    /// and always with the hand. It grabs what it lands on, and draws with
    /// a drawing tool.
    #[test]
    fn a_finger_pans_the_empty_board_and_grabs_or_draws_otherwise() {
        assert_eq!(finger_role(Tool::Hand, Under::Grabbable), FingerRole::Pans);
        assert_eq!(finger_role(Tool::Select, Under::Empty), FingerRole::Pans);
        assert_eq!(
            finger_role(Tool::Select, Under::Grabbable),
            FingerRole::ActsAsMouse
        );
        assert_eq!(
            finger_role(Tool::Pen, Under::Empty),
            FingerRole::ActsAsMouse
        );
        assert_eq!(
            finger_role(Tool::Draw, Under::Empty),
            FingerRole::ActsAsMouse
        );
    }

    /// `TOILE-UI-289`: a wheel turned down moves the content up, and one
    /// turned right moves it left, as a page scrolls.
    #[test]
    fn the_wheel_moves_the_content_as_a_page_scrolls() {
        let mut cam = ViewportTransform::new(ambre_draw::Size::new(800.0, 600.0));
        cam.zoom = ambre_core::ZoomFactor::positive(2.0);
        let mut kinetic = Kinetic::default();
        let world = Point::new(10.0, 10.0);
        let before = cam.world_to_screen() * world;
        pan_by_wheel(
            &mut cam,
            &mut kinetic,
            CssVector::new(CssPx::new(6.0), CssPx::new(12.0)),
            WheelDeltaMode::Pixel,
            ambre_bus::PageTime(0.0),
        );
        let after = cam.world_to_screen() * world;
        assert!(
            (after.x - before.x + 6.0).abs() < 1e-6,
            "{before:?} {after:?}"
        );
        assert!(
            (after.y - before.y + 12.0).abs() < 1e-6,
            "{before:?} {after:?}"
        );
    }

    /// `KINETIC-002`, `KINETIC-009`: a trackpad glide that pauses keeps the
    /// board moving at the glide's speed, and a notch of the same size never
    /// does.
    #[test]
    fn a_paused_glide_keeps_the_board_moving_and_a_notch_never_does() {
        let world = Point::new(10.0, 10.0);
        let mut cam = ViewportTransform::new(ambre_draw::Size::new(800.0, 600.0));
        let mut kinetic = Kinetic::default();
        for step in 0..5 {
            pan_by_wheel(
                &mut cam,
                &mut kinetic,
                CssVector::new(CssPx::new(0.0), CssPx::new(16.0)),
                WheelDeltaMode::Pixel,
                ambre_bus::PageTime(16.0 * f64::from(step)),
            );
        }
        let glided = (cam.world_to_screen() * world).y;
        coast(&mut cam, &mut kinetic, ambre_bus::PageTime(64.0));
        assert_eq!(kinetic.speed(), None, "no motion while the glide lasts");
        // The frame past the pause launches the motion, and the next one moves.
        coast(&mut cam, &mut kinetic, ambre_bus::PageTime(64.0 + 70.0));
        let vy = kinetic
            .speed()
            .expect("the paused glide carries on")
            .dy
            .get();
        assert!((vy - 1.0).abs() < 0.05, "the glide's speed, {vy}");
        assert!(coast(
            &mut cam,
            &mut kinetic,
            ambre_bus::PageTime(64.0 + 86.0)
        ));
        assert!(
            (cam.world_to_screen() * world).y < glided,
            "the content keeps going up"
        );

        let mut cam = ViewportTransform::new(ambre_draw::Size::new(800.0, 600.0));
        let mut kinetic = Kinetic::default();
        for step in 0..5 {
            pan_by_wheel(
                &mut cam,
                &mut kinetic,
                CssVector::new(CssPx::new(0.0), CssPx::new(16.0)),
                WheelDeltaMode::Line,
                ambre_bus::PageTime(16.0 * f64::from(step)),
            );
        }
        let stepped = (cam.world_to_screen() * world).y;
        assert!(
            !coast(&mut cam, &mut kinetic, ambre_bus::PageTime(64.0 + 70.0)),
            "a notch never coasts"
        );
        assert_eq!((cam.world_to_screen() * world).y, stepped);
        assert_eq!(kinetic.speed(), None);
    }

    /// `TOILE-UI-380`: a view step moves the content by the same screen
    /// distance at any zoom. A gesture held at an edge pans towards it.
    #[test]
    fn a_step_moves_the_same_screen_distance_at_any_zoom() {
        // A notch is a step one for one: 30 px down.
        let step = Kinetic::default().wheel(
            CssVector::new(CssPx::ZERO, CssPx::new(30.0)),
            WheelDeltaMode::Line,
            ambre_bus::PageTime(0.0),
        );
        for zoom in [0.25, 1.0, 4.0] {
            let mut cam = ViewportTransform::new(ambre_draw::Size::new(800.0, 600.0));
            cam.zoom = ambre_core::ZoomFactor::positive(zoom);
            let world = Point::new(10.0, 10.0);
            let before = cam.world_to_screen() * world;
            move_view(&mut cam, step);
            let after = cam.world_to_screen() * world;
            assert!((before.y - after.y - 30.0).abs() < 1e-6, "{zoom}");
        }
        let cam = ViewportTransform::new(ambre_draw::Size::new(800.0, 600.0));
        let still = (EdgeSpeed::NONE, EdgeSpeed::NONE);
        assert_eq!(edge_pan_speed(&cam, Point::new(400.0, 300.0)), still);
        let (sx, sy) = edge_pan_speed(&cam, Point::new(799.0, 2.0));
        assert!(
            sx.get().get() > 0.0 && sy.get().get() < 0.0,
            "{sx:?} {sy:?}"
        );
    }
}
