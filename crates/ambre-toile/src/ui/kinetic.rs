//! The board on the shared kinetic scroll
//! (`3-Toile/features/ambre-spec-toile-viewport.md section 3`).
//!
//! The model (`ambre_draw::kinetic`) hands back how far the view moves, in
//! screen pixels. The board moves its camera by it, the content going the
//! other way, so the zoom never changes how far a flick travels.

use ambre_draw::kinetic::edge_speed;
use kurbo::{Point, Vec2};

use super::tool::Tool;
use super::viewport::ViewportTransform;

/// Whether a finger pressed with `tool` pans the board rather than acting
/// as the mouse. The hand always pans. The select tool pans on the empty
/// board, and grabs what it can grab; a drawing tool always draws.
pub fn finger_pans(tool: Tool, grabbable: bool) -> bool {
    match tool {
        Tool::Hand => true,
        Tool::Select => !grabbable,
        _ => false,
    }
}

/// Move the camera by a view step of the model.
pub fn move_view(cam: &mut ViewportTransform, step: (f32, f32)) {
    cam.pan_by_screen(-Vec2::new(f64::from(step.0), f64::from(step.1)));
}

/// The speed a gesture held at `at` pans the board at, in screen pixels per
/// millisecond, on each axis (`KINETIC-007`). Zero away from the edges.
pub fn edge_pan_speed(cam: &ViewportTransform, at: Point) -> (f32, f32) {
    (
        edge_speed(at.x as f32, cam.size.width as f32),
        edge_speed(at.y as f32, cam.size.height as f32),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `TOILE-UI-378`, `TOILE-UI-379`: a finger pans the empty board with the select tool,
    /// and always with the hand. It grabs what it lands on, and draws with
    /// a drawing tool.
    #[test]
    fn a_finger_pans_the_empty_board_and_grabs_or_draws_otherwise() {
        assert!(finger_pans(Tool::Hand, true));
        assert!(finger_pans(Tool::Select, false));
        assert!(!finger_pans(Tool::Select, true));
        assert!(!finger_pans(Tool::Pen, false));
        assert!(!finger_pans(Tool::Draw, false));
    }

    /// `TOILE-UI-380`: a view step moves the content by the same screen
    /// distance at any zoom. A gesture held at an edge pans towards it.
    #[test]
    fn a_step_moves_the_same_screen_distance_at_any_zoom() {
        for zoom in [0.25, 1.0, 4.0] {
            let mut cam = ViewportTransform::new(800.0, 600.0);
            cam.zoom = zoom;
            let world = Point::new(10.0, 10.0);
            let before = cam.world_to_screen() * world;
            move_view(&mut cam, (0.0, 30.0));
            let after = cam.world_to_screen() * world;
            assert!((before.y - after.y - 30.0).abs() < 1e-6, "{zoom}");
        }
        let cam = ViewportTransform::new(800.0, 600.0);
        assert_eq!(edge_pan_speed(&cam, Point::new(400.0, 300.0)), (0.0, 0.0));
        let (sx, sy) = edge_pan_speed(&cam, Point::new(799.0, 2.0));
        assert!(sx > 0.0 && sy < 0.0, "{sx} {sy}");
    }
}
