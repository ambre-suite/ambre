// ambre-toile/src/ui/style.rs
// ============================================================================
// Command surface for the Toile style / property inspector (toile-ui
// SS5quinquies.B). The host-visible half: the pure model the canvas view
// publishes to the DOM inspector -- the selection's common `ElementStyle` plus a
// per-field "mixed" mask flagging attributes that differ across the selection.
//
// Kept in the always-compiled `ui` module (not the wasm-only `chrome`) so it is
// host-unit-tested (SS10), exactly like `ui::menu`, `ui::palette` and `ui::find`.
// The wasm-only Leptos component that renders it (`chrome::StyleInspector`)
// consumes this model and emits `RestyleElements` back through the view.
//
// Selection is UI-only state living in `ToileView` (toile-ui SS7 / SS11bis): the
// DOM inspector cannot read it, so the view derives this model from its selection
// x its last `DisplayViewport` and pushes it, mirroring how it publishes the
// context-menu `CtxScope` (`ui::menu`). The style register is written whole
// (SS2.E), so the inspector edits on top of `base` and emits one `RestyleElements`.
// ============================================================================

use ambre_bus::app::toile::{DisplayNode, DisplayViewport, ElementId, ElementStyle};

/// Which style attributes differ across the current selection. A set flag means
/// the inspector shows a mixed-state indicator for that field rather than a value;
/// editing the field still writes it to every selected element (whole-register
/// restyle, SS2.E). Only the fields the inspector exposes are tracked.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct MixedFlags {
    pub fill: bool,
    pub stroke: bool,
    pub stroke_w: bool,
    pub opacity: bool,
}

impl MixedFlags {
    /// True when at least one tracked field differs across the selection.
    pub fn any(self) -> bool {
        self.fill || self.stroke || self.stroke_w || self.opacity
    }
}

/// The inspector's view model for the current selection (toile-ui SS5quinquies.B).
/// `base` is a representative full style (the first selected element's), used both
/// as the displayed value where fields agree and as the base the inspector mutates
/// to build its whole-register `RestyleElements`. `mixed` marks the diverging
/// fields. Published only for a non-empty selection (`None` otherwise).
#[derive(Clone, PartialEq, Debug)]
pub struct StyleModel {
    pub base: ElementStyle,
    pub mixed: MixedFlags,
}

/// Derive the inspector model from the UI selection and the culled viewport, or
/// `None` when nothing selectable is in the selection. Pure: reads only the
/// `DisplayViewport` and the selection ids. Connectors are not addressed by the
/// element inspector, so only `DisplayElement`s contribute.
pub fn style_scope_for(selection: &[ElementId], display: &DisplayViewport) -> Option<StyleModel> {
    // Resolve the selected ids to their displayed styles, in selection order.
    let mut styles = selection.iter().filter_map(|id| {
        display.nodes.iter().find_map(|n| match n {
            DisplayNode::Element(e) if &e.id == id => Some(&e.style),
            _ => None,
        })
    });

    // The first resolved element seeds the base; an empty (or all-unresolved)
    // selection yields no model, so the inspector stays hidden.
    let base = styles.next()?.clone();

    // Fold the rest, raising a mixed flag as soon as a field diverges from base.
    let mut mixed = MixedFlags::default();
    for s in styles {
        mixed.fill |= s.fill != base.fill;
        mixed.stroke |= s.stroke != base.stroke;
        mixed.stroke_w |= s.stroke_w != base.stroke_w;
        mixed.opacity |= s.opacity != base.opacity;
    }

    Some(StyleModel { base, mixed })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::app::toile::{DisplayElement, DisplayGeom, ElementKind, ShapeKind, Viewport};
    use kurbo::{Affine, Point, Rect, Size};

    fn eid(s: &str) -> ElementId {
        ElementId(s.to_owned())
    }

    /// A displayed element carrying `style`, everything else filler.
    fn elem(id: &str, style: ElementStyle) -> DisplayNode {
        DisplayNode::Element(DisplayElement {
            id: eid(id),
            kind: ElementKind::Shape(ShapeKind::Rect),
            transform: Affine::IDENTITY,
            bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
            locked: false,
            style,
            geom: DisplayGeom::Rect,
            group: None,
        })
    }

    fn viewport(nodes: Vec<DisplayNode>) -> DisplayViewport {
        DisplayViewport {
            reactions: Default::default(),
            nodes,
            viewport: Viewport {
                center: Point::ZERO,
                zoom: 1.0,
                size: Size::new(100.0, 100.0),
            },
        }
    }

    #[test]
    fn empty_selection_has_no_model() {
        let dv = viewport(vec![elem("a", ElementStyle::default())]);
        assert_eq!(style_scope_for(&[], &dv), None);
    }

    #[test]
    fn unresolved_selection_has_no_model() {
        // A selected id not present in the culled viewport contributes nothing.
        let dv = viewport(vec![elem("a", ElementStyle::default())]);
        assert_eq!(style_scope_for(&[eid("ghost")], &dv), None);
    }

    #[test]
    fn single_element_shows_its_style_with_no_mixed() {
        let s = ElementStyle {
            fill: Some(0xff00_ff00),
            ..ElementStyle::default()
        };
        let dv = viewport(vec![elem("a", s.clone())]);
        let model = style_scope_for(&[eid("a")], &dv).expect("model");
        assert_eq!(model.base, s);
        assert_eq!(model.mixed, MixedFlags::default());
        assert!(!model.mixed.any());
    }

    #[test]
    fn differing_fill_flags_only_fill_mixed() {
        let a = ElementStyle {
            fill: Some(0xffff_0000),
            ..ElementStyle::default()
        };
        let b = ElementStyle {
            fill: Some(0xff00_00ff),
            ..ElementStyle::default()
        };
        let dv = viewport(vec![elem("a", a.clone()), elem("b", b)]);
        let model = style_scope_for(&[eid("a"), eid("b")], &dv).expect("model");
        // Base is the first element's style; only fill diverges.
        assert_eq!(model.base, a);
        assert!(model.mixed.fill);
        assert!(!model.mixed.stroke);
        assert!(!model.mixed.stroke_w);
        assert!(!model.mixed.opacity);
        assert!(model.mixed.any());
    }

    #[test]
    fn identical_elements_are_not_mixed() {
        let s = ElementStyle::default();
        let dv = viewport(vec![elem("a", s.clone()), elem("b", s.clone())]);
        let model = style_scope_for(&[eid("a"), eid("b")], &dv).expect("model");
        assert_eq!(model.mixed, MixedFlags::default());
    }

    #[test]
    fn differing_stroke_width_and_opacity_flag_both() {
        let a = ElementStyle {
            stroke_w: 1.0,
            opacity: 1.0,
            ..ElementStyle::default()
        };
        let b = ElementStyle {
            stroke_w: 3.0,
            opacity: 0.5,
            ..ElementStyle::default()
        };
        let dv = viewport(vec![elem("a", a), elem("b", b)]);
        let model = style_scope_for(&[eid("a"), eid("b")], &dv).expect("model");
        assert!(model.mixed.stroke_w);
        assert!(model.mixed.opacity);
        assert!(!model.mixed.fill);
        assert!(!model.mixed.stroke);
    }
}
