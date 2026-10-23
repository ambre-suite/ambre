//! Smart Animate transition solver (`toile-presentation`).
//!
//! Moving between slide A and slide B, Toile interpolates the animatable
//! properties of elements that appear in **both** frames and share the same
//! stable [`ElementId`]. Those are position, size, rotation, opacity and fill,
//! interpolated with an eased `lerp`. Elements only in A fade out, only in B
//! fade in. Matching is by **stable id**, so an element the author moved /
//! resized between two frames animates smoothly "for free": the Figma Smart
//! Animate model. Geometry / heuristic matching is the rejected alternative
//! (fragile).
//!
//! This module is **pure and deterministic**. Given two [`FrameSnapshot`]s and a
//! phase `t` it returns the [`AnimatedElement`]s to paint, with no CRDT
//! mutation. It animates the *presentation* of existing elements. The two
//! snapshots are derived by the engine (`ToileState::frame_snapshot`); consuming
//! the output in the `toile-presentation` camera pass is Phase 1 (roadmap).
//!
//! Frame membership is **explicit, not spatial** (note): a snapshot's elements
//! are the frame's declared `children`, not whatever overlaps its rect.

use std::collections::HashMap;

use ambre_core::Rgba;
use kurbo::Rect;

use crate::logic::model::ElementId;

// The `SnapElement` / `FrameSnapshot` capture DTOs live in `ambre-bus`, so the
// enriched `SlideChanged` event can carry them to the render thread. That
// thread owns no board state. This solver stays here as the sole consumer.
pub use ambre_bus::app::toile::{FrameSnapshot, SnapElement};

/// The interpolated presentation of one element for a single transition frame.
/// Same shape as [`SnapElement`]; produced by [`lerp`](AnimatedElement::lerp) /
/// [`fade_in`](AnimatedElement::fade_in) / [`fade_out`](AnimatedElement::fade_out).
#[derive(Clone, Debug, PartialEq)]
pub struct AnimatedElement {
    pub id: ElementId,
    pub bounds: Rect,
    pub rotation: f64,
    pub opacity: f32,
    pub fill: Option<Rgba>,
}

impl AnimatedElement {
    /// Tween a shared-id element from its A-state `a` to its B-state `b` at phase
    /// `t`. Every property is a plain `lerp`, so `t = 0` reproduces `a`
    /// exactly and `t = 1` reproduces `b` exactly (the endpoint guarantee tested
    /// in). `t` arrives already eased (see [`ease_in_out_cubic`]).
    pub fn lerp(a: &SnapElement, b: &SnapElement, t: f64) -> Self {
        Self {
            // The id is shared by construction; keep it as the animated identity.
            id: b.id.clone(),
            bounds: lerp_rect(a.bounds, b.bounds, t),
            rotation: lerp_f64(a.rotation, b.rotation, t),
            opacity: lerp_f32(a.opacity, b.opacity, t),
            fill: lerp_fill(a.fill, b.fill, t),
        }
    }

    /// An element new in B: hold its geometry, ramp opacity from 0 up to its own
    /// target with `t`. `t = 0` -> invisible, `t = 1` -> full.
    pub fn fade_in(e: &SnapElement, t: f64) -> Self {
        Self {
            id: e.id.clone(),
            bounds: e.bounds,
            rotation: e.rotation,
            opacity: e.opacity * t as f32,
            fill: e.fill,
        }
    }

    /// An element gone in B: hold its geometry, ramp opacity down from its own
    /// value to 0 with `t`. `t = 0` -> full, `t = 1` -> invisible.
    pub fn fade_out(a: &SnapElement, t: f64) -> Self {
        Self {
            id: a.id.clone(),
            bounds: a.bounds,
            rotation: a.rotation,
            opacity: a.opacity * (1.0 - t) as f32,
            fill: a.fill,
        }
    }
}

/// Solve the Smart Animate transition from `from` to `to` at phase `t`.
///
/// Elements shared by stable id are tweened; elements only in `to` fade in;
/// elements only in `from` fade out. Deterministic and allocation-light: each
/// side is indexed once (`HashMap<&str, &SnapElement>`) rather than an O(n*m)
/// `by_id` scan, to fit the < 4 ms / 500-element transition-frame budget.
/// The output paint order is `to`'s order (tweens + fade-ins) followed by the
/// fade-outs, matching the pseudo-code map/chain of `toile-presentation`.
pub fn smart_animate(from: &FrameSnapshot, to: &FrameSnapshot, t: f64) -> Vec<AnimatedElement> {
    let from_by_id: HashMap<&str, &SnapElement> =
        from.elements.iter().map(|e| (e.id.as_str(), e)).collect();
    let to_by_id: HashMap<&str, &SnapElement> =
        to.elements.iter().map(|e| (e.id.as_str(), e)).collect();

    to.elements
        .iter()
        .map(|e| match from_by_id.get(e.id.as_str()) {
            Some(a) => AnimatedElement::lerp(a, e, t), // shared id -> tween
            None => AnimatedElement::fade_in(e, t),    // new in B
        })
        .chain(
            from.elements
                .iter()
                .filter(|a| !to_by_id.contains_key(a.id.as_str()))
                .map(|a| AnimatedElement::fade_out(a, t)), // gone in B
        )
        .collect()
}

/// The standard ease-in-out cubic curve the camera pass applies before feeding
/// `t` to [`smart_animate`] ("eased lerp"). Kept here so Phase 1 reuses the
/// same easing the tests pin. Input is clamped to `0.0 ..= 1.0`; the curve is
/// symmetric (`ease(0.5) == 0.5`) and monotonic.
pub fn ease_in_out_cubic(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        let f = -2.0 * t + 2.0;
        1.0 - f * f * f / 2.0
    }
}

// -- interpolation primitives -------------------------------------------------

fn lerp_f64(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn lerp_f32(a: f32, b: f32, t: f64) -> f32 {
    a + (b - a) * t as f32
}

fn lerp_rect(a: Rect, b: Rect, t: f64) -> Rect {
    Rect::new(
        lerp_f64(a.x0, b.x0, t),
        lerp_f64(a.y0, b.y0, t),
        lerp_f64(a.x1, b.x1, t),
        lerp_f64(a.y1, b.y1, t),
    )
}

/// Interpolate two RGBA fills. Both present -> per-channel lerp. Otherwise there
/// is no common color to blend, so crossfade to the `t`-dominant side (`< 0.5`
/// keeps `a`, `>= 0.5` takes `b`). This keeps the tween deterministic and never
/// invents a color out of `None`.
fn lerp_fill(a: Option<Rgba>, b: Option<Rgba>, t: f64) -> Option<Rgba> {
    match (a, b) {
        (Some(ca), Some(cb)) => Some(lerp_rgba(ca, cb, t)),
        _ => {
            if t < 0.5 {
                a
            } else {
                b
            }
        }
    }
}

/// Per-channel lerp of two RGBA colors.
fn lerp_rgba(a: Rgba, b: Rgba, t: f64) -> Rgba {
    let (a, b) = (a.bits(), b.bits());
    let ch = |shift: u32| {
        let ca = ((a >> shift) & 0xFF) as f64;
        let cb = ((b >> shift) & 0xFF) as f64;
        (lerp_f64(ca, cb, t).round().clamp(0.0, 255.0) as u32) << shift
    };
    Rgba::from_bits(ch(24) | ch(16) | ch(8) | ch(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(
        id: &str,
        bounds: Rect,
        rotation: f64,
        opacity: f32,
        fill: Option<Rgba>,
    ) -> SnapElement {
        SnapElement {
            id: ElementId(id.to_owned()),
            bounds,
            rotation,
            opacity,
            fill,
        }
    }

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect::new(x, y, x + w, y + h)
    }

    #[test]
    fn shared_id_tweens_at_midpoint() {
        let a = snap(
            "e1",
            rect(0.0, 0.0, 10.0, 10.0),
            0.0,
            1.0,
            Some(Rgba::from_bits(0x0000_00FF)),
        );
        let b = snap(
            "e1",
            rect(20.0, 40.0, 30.0, 50.0),
            2.0,
            0.0,
            Some(Rgba::from_bits(0x0000_FFFF)),
        );
        let from = FrameSnapshot {
            elements: vec![a.clone()],
        };
        let to = FrameSnapshot {
            elements: vec![b.clone()],
        };
        let out = smart_animate(&from, &to, 0.5);
        assert_eq!(out.len(), 1);
        let m = &out[0];
        assert_eq!(m.id, ElementId("e1".to_owned()));
        // Geometry / rotation / opacity are the exact midpoints.
        assert_eq!(m.bounds, rect(10.0, 20.0, 20.0, 30.0));
        assert_eq!(m.rotation, 1.0);
        assert_eq!(m.opacity, 0.5);
        // Blue channel midpoint of 0x00 -> 0xFF is 0x80 (128), alpha stays 0xFF.
        assert_eq!(m.fill, Some(Rgba::from_bits(0x0000_80FF)));
    }

    #[test]
    fn new_element_fades_in() {
        let b = snap(
            "new",
            rect(0.0, 0.0, 5.0, 5.0),
            0.0,
            0.8,
            Some(Rgba::from_bits(0xAABB_CCFF)),
        );
        let from = FrameSnapshot::default();
        let to = FrameSnapshot {
            elements: vec![b.clone()],
        };
        let out = smart_animate(&from, &to, 0.25);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, b.id);
        assert_eq!(out[0].bounds, b.bounds); // geometry held at target
        assert!((out[0].opacity - 0.8 * 0.25).abs() < 1e-6);
    }

    #[test]
    fn gone_element_fades_out() {
        let a = snap(
            "old",
            rect(1.0, 2.0, 3.0, 4.0),
            0.5,
            0.6,
            Some(Rgba::from_bits(0x1122_33FF)),
        );
        let from = FrameSnapshot {
            elements: vec![a.clone()],
        };
        let to = FrameSnapshot::default();
        let out = smart_animate(&from, &to, 0.25);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, a.id);
        assert_eq!(out[0].bounds, a.bounds); // geometry held at source
        assert!((out[0].opacity - 0.6 * 0.75).abs() < 1e-6);
    }

    #[test]
    fn endpoints_reproduce_source_and_target() {
        let a = snap(
            "e",
            rect(0.0, 0.0, 10.0, 10.0),
            0.0,
            1.0,
            Some(Rgba::from_bits(0x0000_00FF)),
        );
        let b = snap(
            "e",
            rect(100.0, 100.0, 20.0, 20.0),
            3.0,
            0.2,
            Some(Rgba::from_bits(0xFFFF_FFFF)),
        );
        // t = 0 reproduces A, t = 1 reproduces B.
        let at0 = AnimatedElement::lerp(&a, &b, 0.0);
        assert_eq!(at0.bounds, a.bounds);
        assert_eq!(at0.rotation, a.rotation);
        assert_eq!(at0.opacity, a.opacity);
        assert_eq!(at0.fill, a.fill);
        let at1 = AnimatedElement::lerp(&a, &b, 1.0);
        assert_eq!(at1.bounds, b.bounds);
        assert_eq!(at1.rotation, b.rotation);
        // f32 lerp `a + (b - a) * 1.0` reproduces b within rounding.
        assert!((at1.opacity - b.opacity).abs() < 1e-6);
        assert_eq!(at1.fill, b.fill);
        // Fade endpoints.
        assert_eq!(AnimatedElement::fade_in(&b, 0.0).opacity, 0.0);
        assert_eq!(AnimatedElement::fade_in(&b, 1.0).opacity, b.opacity);
        assert_eq!(AnimatedElement::fade_out(&a, 0.0).opacity, a.opacity);
        assert_eq!(AnimatedElement::fade_out(&a, 1.0).opacity, 0.0);
    }

    #[test]
    fn mixed_membership_classifies_each_id() {
        let a = snap("a", rect(0.0, 0.0, 1.0, 1.0), 0.0, 1.0, None);
        let b = snap("b", rect(0.0, 0.0, 1.0, 1.0), 0.0, 1.0, None);
        let a2 = snap("a", rect(5.0, 5.0, 1.0, 1.0), 0.0, 1.0, None);
        let c = snap("c", rect(0.0, 0.0, 1.0, 1.0), 0.0, 1.0, None);
        // from = {a, b}, to = {a, c}
        let from = FrameSnapshot {
            elements: vec![a, b],
        };
        let to = FrameSnapshot {
            elements: vec![a2, c],
        };
        let out = smart_animate(&from, &to, 0.5);
        // a tweened, c faded in, b faded out -> three animated elements.
        assert_eq!(out.len(), 3);
        // Order: to's order (a, c) then the fade-outs (b).
        assert_eq!(out[0].id, ElementId("a".to_owned()));
        assert_eq!(out[0].bounds, rect(2.5, 2.5, 1.0, 1.0)); // tween midpoint
        assert_eq!(out[1].id, ElementId("c".to_owned()));
        assert_eq!(out[1].opacity, 0.5); // fade in at t=0.5
        assert_eq!(out[2].id, ElementId("b".to_owned()));
        assert_eq!(out[2].opacity, 0.5); // fade out at t=0.5
    }

    #[test]
    fn empty_frames() {
        let e = snap("x", rect(0.0, 0.0, 1.0, 1.0), 0.0, 1.0, None);
        // both empty -> nothing.
        assert!(
            smart_animate(&FrameSnapshot::default(), &FrameSnapshot::default(), 0.5).is_empty()
        );
        // from empty -> all fade in.
        let to = FrameSnapshot {
            elements: vec![e.clone()],
        };
        let out = smart_animate(&FrameSnapshot::default(), &to, 0.5);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].opacity, 0.5);
        // to empty -> all fade out.
        let from = FrameSnapshot { elements: vec![e] };
        let out = smart_animate(&from, &FrameSnapshot::default(), 0.5);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].opacity, 0.5);
    }

    #[test]
    fn by_id_finds_declared_child() {
        let e = snap("child", rect(0.0, 0.0, 1.0, 1.0), 0.0, 1.0, None);
        let f = FrameSnapshot {
            elements: vec![e.clone()],
        };
        assert_eq!(f.by_id(&ElementId("child".to_owned())), Some(&e));
        assert_eq!(f.by_id(&ElementId("absent".to_owned())), None);
    }

    #[test]
    fn ease_in_out_cubic_curve() {
        assert_eq!(ease_in_out_cubic(0.0), 0.0);
        assert_eq!(ease_in_out_cubic(1.0), 1.0);
        assert!((ease_in_out_cubic(0.5) - 0.5).abs() < 1e-9);
        // Monotonic non-decreasing over the unit interval.
        let mut prev = -1.0;
        for i in 0..=100 {
            let v = ease_in_out_cubic(i as f64 / 100.0);
            assert!(v >= prev, "ease must be monotonic");
            prev = v;
        }
        // Out-of-range clamps.
        assert_eq!(ease_in_out_cubic(-1.0), 0.0);
        assert_eq!(ease_in_out_cubic(2.0), 1.0);
    }
}
