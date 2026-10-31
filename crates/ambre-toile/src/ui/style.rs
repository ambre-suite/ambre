// ambre-toile/src/ui/style.rs
// ============================================================================
// Command surface for the Toile style / property inspector
// (`toile-styles`). The host-visible half is the pure model the canvas view
// publishes to the DOM inspector. That model is the selection's common
// `ElementStyle`, plus a per-field "mixed" mask flagging attributes that differ
// across the selection.
//
// Kept in the always-compiled `ui` module (not the wasm-only `chrome`) so it is
// host-unit-tested, exactly like `ui::menu`, `ui::palette` and `ui::find`.
// The wasm-only Leptos component that renders it (`chrome::StyleInspector`)
// consumes this model and emits `RestyleElements` back through the view.
//
// Selection is UI-only state living in `ToileView` (`toile-selection`), and the
// DOM inspector cannot read it. So the view derives this model from its
// selection x its last `DisplayViewport` and pushes it, mirroring how it
// publishes the context-menu `CtxScope` (`ui::menu`). The style register is
// written whole (`TOILE-018`), so the inspector edits on top of `base` and emits
// one `RestyleElements`.
// ============================================================================

use ambre_bus::app::toile::{DisplayElement, DisplayNode, DisplayViewport, ElementStyle};
use ambre_core::id::{Connector, Element, Id};

/// Which style attributes differ across the current selection. A set flag means
/// the inspector shows a mixed-state indicator for that field rather than a value.
/// Editing the field still writes it to every selected element (whole-register
/// restyle, `TOILE-018`). Only the fields the inspector exposes are tracked.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct MixedFlags {
    pub fill: Mix,
    pub stroke: Mix,
    pub stroke_w: Mix,
    pub opacity: Mix,
    pub text_color: Mix,
    pub font_family: Mix,
    pub font_size: Mix,
    pub bold: Mix,
    pub italic: Mix,
    pub underline: Mix,
    pub strikethrough: Mix,
}

impl MixedFlags {
    /// True when at least one tracked field differs across the selection.
    pub fn any(self) -> bool {
        (self.fill
            | self.stroke
            | self.stroke_w
            | self.opacity
            | self.text_color
            | self.font_family
            | self.font_size
            | self.bold
            | self.italic
            | self.underline
            | self.strikethrough)
            .is_mixed()
    }
}

/// Whether one style field agrees across the selection, or differs.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mix {
    /// Every selected element holds the same value.
    #[default]
    Uniform,
    /// Two selected elements hold different values.
    Mixed,
}

impl Mix {
    /// `Mixed` when the field `differs` between two selected elements.
    pub fn when(differs: bool) -> Self {
        if differs { Self::Mixed } else { Self::Uniform }
    }

    /// True for `Mixed`.
    pub fn is_mixed(self) -> bool {
        self == Self::Mixed
    }
}

/// A field is mixed across a selection when it is mixed across either part.
impl std::ops::BitOr for Mix {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        if self.is_mixed() { self } else { other }
    }
}

impl std::ops::BitOrAssign for Mix {
    fn bitor_assign(&mut self, other: Self) {
        *self = *self | other;
    }
}

/// The inspector's view model for the current selection (`toile-styles`). `base`
/// is a representative full style, the first selected element's. It is both the
/// displayed value where fields agree and the base the inspector mutates to build
/// its whole-register `RestyleElements`. `mixed` marks the diverging fields.
/// Published only for a non-empty selection (`None` otherwise).
#[derive(Clone, PartialEq, Debug)]
pub struct StyleModel {
    pub base: ElementStyle,
    pub mixed: MixedFlags,
}

/// Derive the inspector model from the UI selection and the culled viewport, or
/// `None` when nothing selectable is in the selection. Pure: reads only the
/// `DisplayViewport` and the selection ids. Connectors are not addressed by the
/// element inspector, so only `DisplayElement`s contribute.
/// The inspector's model for a selected arrow: its own style, nothing mixed.
/// An arrow is always a selection of one, so no field can diverge
/// (`toile-connectors`).
pub fn arrow_style_scope(id: &Id<Connector>, display: &DisplayViewport) -> Option<StyleModel> {
    let base = display.nodes.iter().find_map(|n| match n {
        DisplayNode::Connector(c) if &c.id == id => Some(c.style.clone()),
        _ => None,
    })?;
    Some(StyleModel {
        base,
        mixed: MixedFlags::default(),
    })
}

pub fn style_scope_for(
    selection: &[Id<Element>],
    selected: &[DisplayElement],
) -> Option<StyleModel> {
    // Resolve the selected ids to their styles, in selection order. `selected`
    // is the whole selection's data from Logic, off-screen included
    // (`TOILE-UI-070`).
    let mut styles = selection
        .iter()
        .filter_map(|id| selected.iter().find(|e| &e.id == id).map(|e| &e.style));

    // The first resolved element seeds the base; an empty (or all-unresolved)
    // selection yields no model, so the inspector stays hidden.
    let base = styles.next()?.clone();

    // Fold the rest, raising a mixed flag as soon as a field diverges from base.
    let mut mixed = MixedFlags::default();
    for s in styles {
        mixed.fill |= Mix::when(s.fill != base.fill);
        mixed.stroke |= Mix::when(s.stroke != base.stroke);
        mixed.stroke_w |= Mix::when(s.stroke_w != base.stroke_w);
        mixed.opacity |= Mix::when(s.opacity != base.opacity);
        mixed.text_color |= Mix::when(s.text_color != base.text_color);
        mixed.font_family |= Mix::when(s.font.family != base.font.family);
        mixed.font_size |= Mix::when(s.font.size != base.font.size);
        mixed.bold |= Mix::when(s.font.weight.is_bold() != base.font.weight.is_bold());
        mixed.italic |= Mix::when(s.font.italic != base.font.italic);
        mixed.underline |= Mix::when(s.font.underline != base.font.underline);
        mixed.strikethrough |= Mix::when(s.font.strikethrough != base.font.strikethrough);
    }

    Some(StyleModel { base, mixed })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::app::toile::{DisplayElement, DisplayGeom, ElementKind, ShapeKind, Viewport};
    use ambre_core::Rgba;
    use kurbo::{Affine, Point, Rect, Size};

    fn eid(s: &str) -> Id<Element> {
        Id::<Element>::from_text(s.to_owned())
    }

    /// A displayed element carrying `style`, everything else filler.
    fn elem(id: &str, style: ElementStyle) -> DisplayNode {
        DisplayNode::Element(DisplayElement {
            id: eid(id),
            kind: ElementKind::Shape(ShapeKind::Rect),
            transform: Affine::IDENTITY,
            bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
            locked: ambre_bus::app::toile::LockState::Unlocked,
            style,
            geom: DisplayGeom::Rect,
            link: None,
            group: None,
        })
    }

    fn viewport(nodes: Vec<DisplayNode>) -> DisplayViewport {
        DisplayViewport {
            version: ambre_bus::ViewportVersion(0),
            grid_unit: ambre_core::BoardLength::new(20.0),
            pins: Vec::new(),
            reactions: Default::default(),
            nodes,
            viewport: Viewport {
                center: Point::ZERO,
                zoom: ambre_core::ZoomFactor::positive(1.0),
                size: Size::new(100.0, 100.0),
            },
        }
    }

    /// The selection's whole display data, as Logic sends it.
    fn els(dv: &DisplayViewport) -> Vec<DisplayElement> {
        dv.nodes
            .iter()
            .filter_map(|n| match n {
                DisplayNode::Element(e) => Some(e.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn empty_selection_has_no_model() {
        let dv = viewport(vec![elem("a", ElementStyle::default())]);
        assert_eq!(style_scope_for(&[], &els(&dv)), None);
    }

    #[test]
    fn unresolved_selection_has_no_model() {
        // A selected id not present in the culled viewport contributes nothing.
        let dv = viewport(vec![elem("a", ElementStyle::default())]);
        assert_eq!(style_scope_for(&[eid("ghost")], &els(&dv)), None);
    }

    #[test]
    fn single_element_shows_its_style_with_no_mixed() {
        let s = ElementStyle {
            fill: Some(Rgba::from_bits(0x00ff_00ff)),
            ..ElementStyle::default()
        };
        let dv = viewport(vec![elem("a", s.clone())]);
        let model = style_scope_for(&[eid("a")], &els(&dv)).expect("model");
        assert_eq!(model.base, s);
        assert_eq!(model.mixed, MixedFlags::default());
        assert!(!model.mixed.any());
    }

    /// The culled viewport holds only `a`; `b` is off-screen but in the
    /// selection data Logic sent. Its differing fill must still flag mixed
    /// (`TOILE-UI-070`).
    #[test]
    fn off_screen_selection_contributes_its_style() {
        let a = ElementStyle {
            fill: Some(Rgba::from_bits(0xff00_00ff)),
            ..ElementStyle::default()
        };
        let b = ElementStyle {
            fill: Some(Rgba::from_bits(0x0000_ffff)),
            ..ElementStyle::default()
        };
        let selected: Vec<DisplayElement> = [elem("a", a.clone()), elem("b", b.clone())]
            .into_iter()
            .map(|n| match n {
                DisplayNode::Element(e) => e,
                _ => panic!("the fixture builds elements only"),
            })
            .collect();
        let model = style_scope_for(&[eid("a"), eid("b")], &selected).expect("model");
        assert!(model.mixed.fill.is_mixed(), "off-screen b's fill differs");
    }

    #[test]
    fn differing_fill_flags_only_fill_mixed() {
        let a = ElementStyle {
            fill: Some(Rgba::from_bits(0xff00_00ff)),
            ..ElementStyle::default()
        };
        let b = ElementStyle {
            fill: Some(Rgba::from_bits(0x0000_ffff)),
            ..ElementStyle::default()
        };
        let dv = viewport(vec![elem("a", a.clone()), elem("b", b)]);
        let model = style_scope_for(&[eid("a"), eid("b")], &els(&dv)).expect("model");
        // Base is the first element's style; only fill diverges.
        assert_eq!(model.base, a);
        assert!(model.mixed.fill.is_mixed());
        assert!(!model.mixed.stroke.is_mixed());
        assert!(!model.mixed.stroke_w.is_mixed());
        assert!(!model.mixed.opacity.is_mixed());
        assert!(model.mixed.any());
    }

    #[test]
    fn identical_elements_are_not_mixed() {
        let s = ElementStyle::default();
        let dv = viewport(vec![elem("a", s.clone()), elem("b", s.clone())]);
        let model = style_scope_for(&[eid("a"), eid("b")], &els(&dv)).expect("model");
        assert_eq!(model.mixed, MixedFlags::default());
    }

    #[test]
    fn differing_stroke_width_and_opacity_flag_both() {
        let a = ElementStyle {
            stroke_w: ambre_core::StoredBoardLength::new(1.0),
            opacity: ambre_core::Opacity::new(1.0).unwrap(),
            ..ElementStyle::default()
        };
        let b = ElementStyle {
            stroke_w: ambre_core::StoredBoardLength::new(3.0),
            opacity: ambre_core::Opacity::new(0.5).unwrap(),
            ..ElementStyle::default()
        };
        let dv = viewport(vec![elem("a", a), elem("b", b)]);
        let model = style_scope_for(&[eid("a"), eid("b")], &els(&dv)).expect("model");
        assert!(model.mixed.stroke_w.is_mixed());
        assert!(model.mixed.opacity.is_mixed());
        assert!(!model.mixed.fill.is_mixed());
        assert!(!model.mixed.stroke.is_mixed());
    }

    /// `TOILE-UI-397`: an element that names no family shows the picker's
    /// "Default" entry, and so does one the picker reset to it.
    #[test]
    fn an_element_with_no_family_shows_the_default_entry() {
        let dv = viewport(vec![elem("a", ElementStyle::default())]);
        let model = style_scope_for(&[eid("a")], &els(&dv)).expect("model");
        assert_eq!(model.base.font.picker_label(), Some("Default"));
        let mut reset = ElementStyle::default();
        crate::ui::floatbar::StyleEdit::Font(None).apply_to(&mut reset);
        assert_eq!(reset.font.picker_label(), Some("Default"));
    }

    /// `TOILE-UI-397`: an element the picker set to Sans names a family, so
    /// it shows "Sans" and never the "Default" entry.
    #[test]
    fn an_element_set_to_sans_shows_sans() {
        let mut sans = ElementStyle::default();
        let picked = ambre_bus::app::toile::FontFamily::from_label("Sans");
        crate::ui::floatbar::StyleEdit::Font(Some(picked)).apply_to(&mut sans);
        let dv = viewport(vec![elem("a", sans)]);
        let model = style_scope_for(&[eid("a")], &els(&dv)).expect("model");
        assert_eq!(model.base.font.picker_label(), Some("Sans"));
    }

    /// `TOILE-UI-161`: an element on the default family and one the picker
    /// reset to it agree, so the family does not read as mixed.
    #[test]
    fn two_elements_on_the_default_family_are_not_mixed() {
        let fresh = ElementStyle::default();
        let mut reset = ElementStyle::default();
        // The picker's "Default" reset names no family, as a fresh element does.
        crate::ui::floatbar::StyleEdit::Font(None).apply_to(&mut reset);
        let dv = viewport(vec![elem("a", fresh), elem("b", reset)]);
        let model = style_scope_for(&[eid("a"), eid("b")], &els(&dv)).expect("model");
        assert!(!model.mixed.font_family.is_mixed());
    }

    #[test]
    fn differing_text_decorations_flag_underline_and_strikethrough() {
        let font_with = |underline, strikethrough| ambre_bus::app::toile::FontSpec {
            underline,
            strikethrough,
            ..Default::default()
        };
        let a = ElementStyle {
            font: font_with(
                ambre_core::Underline::Plain,
                ambre_core::Strikethrough::Plain,
            ),
            ..ElementStyle::default()
        };
        let b = ElementStyle {
            font: font_with(
                ambre_core::Underline::Underlined,
                ambre_core::Strikethrough::Struck,
            ),
            ..ElementStyle::default()
        };
        let dv = viewport(vec![elem("a", a), elem("b", b)]);
        let model = style_scope_for(&[eid("a"), eid("b")], &els(&dv)).expect("model");
        assert!(model.mixed.underline.is_mixed());
        assert!(model.mixed.strikethrough.is_mixed());
        // The base is the first element; its decorations are off.
        assert_eq!(model.base.font.underline, ambre_core::Underline::Plain);
        assert_eq!(
            model.base.font.strikethrough,
            ambre_core::Strikethrough::Plain
        );
        // Weight / italic agree, so they are not flagged.
        assert!(!model.mixed.bold.is_mixed());
        assert!(!model.mixed.italic.is_mixed());
    }

    /// A field is mixed across a selection once any pair differs, and a later
    /// agreeing pair never clears it.
    #[test]
    fn a_mixed_field_stays_mixed() {
        assert_eq!(Mix::Uniform | Mix::Uniform, Mix::Uniform);
        let mut field = Mix::Uniform;
        field |= Mix::Mixed;
        field |= Mix::Uniform;
        assert_eq!(field, Mix::Mixed);
    }
}
