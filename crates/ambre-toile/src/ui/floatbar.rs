// ambre-toile/src/ui/floatbar.rs
// ============================================================================
// Command surface + placement for the content-anchored floating selection toolbar
// (`toile-styles`). The host-visible half: the pure model the canvas view publishes
// to the DOM bar each frame. That is the bar's screen anchor plus the applicability
// flags that gate its buttons. The anchor is recomputed from the
// `ViewportTransform`, so it follows the selection as the board pans / zooms.
//
// Kept in the always-compiled `ui` module (not the wasm-only `chrome`) so the
// geometry + flags are host-unit-tested, exactly like `ui::menu`, `ui::style` and
// `ui::status`. The wasm-only Leptos component that renders it
// (`chrome::SelectionToolbar`) consumes this model. Its command buttons route
// through the shared `on_command` path with `ui::menu` `CommandId`s. So the bar is
// a strict subset of the context menu and the two never diverge (`toile-menus`).
//
// The bar's transient hiding is decided by the view at publish time (`None` ==
// hidden), not here. It hides during a move/resize/rotate gesture, a pan, an open
// inline editor, or an empty selection. This module only turns a resolved, visible
// selection into an anchored [`FloatBar`].
// ============================================================================

use ambre_bus::app::toile::{
    DisplayElement, ElementId, ElementKind, ElementStyle, FontFamily, FontSizeChoice,
};
use ambre_core::{CssPoint, Rgba, ViewPoint, ViewPx};
use ambre_draw::{Point, Rect};

use super::style::MixedFlags;
use super::viewport::ViewportTransform;

/// The floating selection toolbar's published model (`toile-styles`): `at` is
/// the client CSS-pixel position of the selection's top-edge midpoint. The
/// bar is `position: fixed` there and lifted above the selection with a CSS gap
/// ("air", like Miro). The flags gate the applicable buttons, mirroring the
/// context-menu scope (`ui::menu::CtxScope`). `fill` is the selection's common
/// fill for the quick-colour swatch (`None` when unset or mixed). `mixed` marks
/// every field the bar edits that differs across the selection (`TOILE-UI-164`).
/// Published only for a resolved, visible selection (`None` == hidden).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FloatBar {
    /// The selection's top-edge midpoint on the page.
    pub at: CssPoint,
    /// At least two elements selected -> Group applies.
    pub multi: bool,
    /// At least one selected element is a group node -> Ungroup applies.
    pub has_group: bool,
    /// At least one selected element is locked -> Unlock applies.
    pub any_locked: bool,
    /// At least one selected element is unlocked -> Lock applies.
    pub any_unlocked: bool,
    /// The selection's common fill (`ElementStyle` RGBA), or `None` when unset or
    /// diverging across the selection -- the quick-colour swatch seed.
    pub fill: Option<Rgba>,
    /// The fields the bar edits that differ across the selection. A set flag
    /// means the bar shows a mixed state for that control, not a value.
    pub mixed: MixedFlags,
}

/// A single-field style edit the floating bar emits back (`toile-styles`). Unlike the
/// inspector's `restyle`, which carries a whole `ElementStyle` built on an accumulating
/// working copy, each bar control emits exactly ONE changed field. The view re-derives
/// the selection's common style LIVE at apply time and sets only that field. Successive
/// quick edits then never clobber one another (a stale working copy hazard). The view
/// folds the result into a whole-register `RestyleElements` over the live selection
/// (`TOILE-018`), attaching the ids the DOM bar cannot name.
///
/// Colours are `Option<Rgba>` (`None` == unset / no fill). The font
/// family is `None` for the picker's "Default" reset, which names no family.
#[derive(Clone, PartialEq, Debug)]
pub enum StyleEdit {
    /// Fill colour (`None` == no fill).
    Fill(Option<Rgba>),
    /// Stroke colour (`None` == no stroke). The panel alone emits it.
    Stroke(Option<Rgba>),
    /// Stroke width in world px, floored at zero. The panel alone emits it.
    StrokeWidth(f32),
    /// Opacity. The panel alone emits it.
    Opacity(ambre_core::Opacity),
    /// Text (foreground ink) colour (`None` == the kind default ink).
    TextColor(Option<Rgba>),
    /// Font family. The picker's "Default" reset is `None`, so the element
    /// names no family and draws in the default face.
    Font(Option<FontFamily>),
    /// Font size, or automatic: a note's body then tracks its card.
    Size(FontSizeChoice),
    /// Bold toggle (maps to `font.weight` 700 / 400).
    Bold(bool),
    /// Italic toggle.
    Italic(bool),
    /// Underline toggle.
    Underline(bool),
    /// Strikethrough toggle.
    Strikethrough(bool),
}

impl StyleEdit {
    /// Overwrite exactly the one field this edit carries, leaving every other
    /// attribute of `style` untouched. The view calls this on the selection's
    /// freshly-derived common style so back-to-back bar edits never clobber one
    /// another. The inspector applies it to its working copy for the same reason
    /// (`TOILE-UI-172`). Kept in the always-compiled `ui` module so it is
    /// host-tested: both callers are wasm-only.
    pub fn apply_to(self, style: &mut ElementStyle) {
        match self {
            StyleEdit::Fill(c) => style.fill = c,
            StyleEdit::Stroke(c) => style.stroke = c,
            StyleEdit::StrokeWidth(w) => style.stroke_w = w.max(0.0),
            StyleEdit::Opacity(o) => style.opacity = o,
            StyleEdit::TextColor(c) => style.text_color = c,
            StyleEdit::Font(f) => style.font.family = f,
            // The choice holds no negative size, so none reaches the style
            // (`TOILE-UI-180`).
            StyleEdit::Size(s) => style.font.size = s,
            StyleEdit::Bold(b) => style.font.weight = ambre_core::FontWeight::of_bold(b),
            StyleEdit::Italic(i) => style.font.italic = i,
            StyleEdit::Underline(u) => style.font.underline = u,
            StyleEdit::Strikethrough(s) => style.font.strikethrough = s,
        }
    }
}

/// The client CSS-pixel anchor for the bar: the selection's top-edge midpoint.
/// It is mapped through the camera into screen pixels, which are CSS pixels
/// (`CANVAS-032`). A final shift by the canvas's client offset lands the
/// `position: fixed` bar in viewport coordinates. Pure; `canvas_origin` is the
/// page's corner in host tests.
pub fn top_center_anchor(
    world_bounds: Rect,
    cam: &ViewportTransform,
    canvas_origin: CssPoint,
) -> CssPoint {
    let mid_top = Point::new((world_bounds.x0 + world_bounds.x1) / 2.0, world_bounds.y0);
    let screen = cam.world_to_screen() * mid_top;
    ViewPoint::new(ViewPx::new(screen.x as f32), ViewPx::new(screen.y as f32))
        .on_page(canvas_origin)
}

/// Build the floating-bar model from the UI selection and the culled viewport, or
/// `None` when nothing selectable resolves (the bar hides). Pure: reads only the
/// `DisplayViewport`, the selection ids and the camera. The union of the selected
/// elements' bounds anchors the bar. The per-element kind / lock state set the
/// button flags, and the common fill seeds the quick-colour swatch. Connectors are
/// not addressed by the bar, so only `DisplayElement`s contribute.
pub fn build_floatbar(
    selection: &[ElementId],
    selected: &[DisplayElement],
    cam: &ViewportTransform,
    canvas_origin: CssPoint,
) -> Option<FloatBar> {
    if selection.is_empty() {
        return None;
    }

    let mut union: Option<Rect> = None;
    let mut count = 0usize;
    let mut has_group = false;
    let mut any_locked = false;
    let mut any_unlocked = false;
    // The bar's own fold: a base style plus one difference flag per field it
    // edits (`TOILE-UI-164`).
    let mut base: Option<&ElementStyle> = None;
    let mut mixed = MixedFlags::default();

    for id in selection {
        let Some(e) = find_element(selected, id) else {
            continue;
        };
        count += 1;
        if e.kind == ElementKind::Group {
            has_group = true;
        }
        if e.locked {
            any_locked = true;
        } else {
            any_unlocked = true;
        }
        union = Some(match union {
            Some(u) => u.union(e.bounds),
            None => e.bounds,
        });
        let s = &e.style;
        match base {
            None => base = Some(s),
            Some(b) => {
                mixed.fill |= s.fill != b.fill;
                mixed.text_color |= s.text_color != b.text_color;
                mixed.font_family |= s.font.family != b.font.family;
                mixed.font_size |= s.font.size != b.font.size;
                mixed.bold |= (s.font.weight.is_bold()) != (b.font.weight.is_bold());
                mixed.italic |= s.font.italic != b.font.italic;
                mixed.underline |= s.font.underline != b.font.underline;
                mixed.strikethrough |= s.font.strikethrough != b.font.strikethrough;
            }
        }
    }

    // Nothing in the selection is present in the culled set: hide the bar.
    let union = union?;
    let at = top_center_anchor(union, cam, canvas_origin);
    let fill = base.and_then(|b| if mixed.fill { None } else { b.fill });
    Some(FloatBar {
        at,
        multi: count >= 2,
        has_group,
        any_locked,
        any_unlocked,
        fill,
        mixed,
    })
}

/// Find a selected element by id (linear over the selection's data from Logic).
fn find_element<'a>(elements: &'a [DisplayElement], id: &ElementId) -> Option<&'a DisplayElement> {
    elements.iter().find(|e| &e.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::app::toile::{
        DisplayElement, DisplayGeom, DisplayNode, DisplayViewport, ElementStyle, FontSpec,
        ShapeKind, Viewport,
    };
    use ambre_draw::Size;
    use kurbo::Affine;

    fn page(x: f32, y: f32) -> CssPoint {
        CssPoint::new(ambre_core::CssPx::new(x), ambre_core::CssPx::new(y))
    }

    fn eid(s: &str) -> ElementId {
        ElementId(s.to_owned())
    }

    fn elem(
        id: &str,
        kind: ElementKind,
        bounds: Rect,
        locked: bool,
        fill: Option<Rgba>,
    ) -> DisplayNode {
        DisplayNode::Element(DisplayElement {
            id: eid(id),
            kind,
            transform: Affine::IDENTITY,
            bounds,
            locked,
            style: ElementStyle {
                fill,
                ..ElementStyle::default()
            },
            geom: match kind {
                ElementKind::Group => DisplayGeom::Group,
                _ => DisplayGeom::Rect,
            },
            link: None,
            group: None,
        })
    }

    /// An element carrying a whole `style`, for the mixed-field fold.
    fn styled_elem(id: &str, style: ElementStyle) -> DisplayNode {
        DisplayNode::Element(DisplayElement {
            id: eid(id),
            kind: ElementKind::Sticky,
            transform: Affine::IDENTITY,
            bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
            locked: false,
            style,
            geom: DisplayGeom::Rect,
            link: None,
            group: None,
        })
    }

    fn viewport(nodes: Vec<DisplayNode>) -> DisplayViewport {
        DisplayViewport {
            version: ambre_bus::ViewportVersion(0),
            grid_unit: 20.0,
            pins: Vec::new(),
            reactions: Default::default(),
            nodes,
            viewport: Viewport::default(),
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

    /// An identity camera (zoom 1, centered on origin) with a known size, so
    /// world->screen is a pure translate by half the size.
    fn cam() -> ViewportTransform {
        ViewportTransform {
            center: Point::ZERO,
            zoom: ambre_core::ZoomFactor::positive(1.0),
            size: Size::new(1000.0, 800.0),
        }
    }

    #[test]
    fn empty_selection_hides_bar() {
        let dv = viewport(vec![elem(
            "a",
            ElementKind::Sticky,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            false,
            None,
        )]);
        assert_eq!(
            build_floatbar(&[], &els(&dv), &cam(), CssPoint::default()),
            None
        );
    }

    #[test]
    fn unresolved_selection_hides_bar() {
        let dv = viewport(vec![elem(
            "a",
            ElementKind::Sticky,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            false,
            None,
        )]);
        assert_eq!(
            build_floatbar(&[eid("ghost")], &els(&dv), &cam(), CssPoint::default()),
            None
        );
    }

    #[test]
    fn anchor_is_top_edge_midpoint_in_screen_space() {
        // A 20-wide rect from (10,20)->(30,60): top-edge midpoint is (20, 20) in
        // world. With zoom 1 centered on origin, screen = world + size/2 =
        // (20+500, 20+400) = (520, 420). No offset.
        let dv = viewport(vec![elem(
            "a",
            ElementKind::Sticky,
            Rect::new(10.0, 20.0, 30.0, 60.0),
            false,
            Some(Rgba::from_bits(0x00ff_00ff)),
        )]);
        let bar = build_floatbar(&[eid("a")], &els(&dv), &cam(), CssPoint::default()).expect("bar");
        assert_eq!(bar.at, page(520.0, 420.0));
        assert!(!bar.multi);
        assert_eq!(bar.fill, Some(Rgba::from_bits(0x00ff_00ff)));
    }

    #[test]
    fn the_canvas_offset_applies_to_the_anchor() {
        // Same rect: the canvas client offset shifts it into viewport coords.
        // Screen = (520, 420); + offset (40, 96) = (560, 516).
        let dv = viewport(vec![elem(
            "a",
            ElementKind::Sticky,
            Rect::new(10.0, 20.0, 30.0, 60.0),
            false,
            None,
        )]);
        let bar = build_floatbar(&[eid("a")], &els(&dv), &cam(), page(40.0, 96.0)).expect("bar");
        assert_eq!(bar.at, page(560.0, 516.0));
    }

    #[test]
    fn union_bounds_anchor_a_multi_selection() {
        // Two rects; union top edge is the smaller y0 (10), midpoint x is the
        // union center x = (0 + 80)/2 = 40. Screen = (40+500, 10+400).
        let dv = viewport(vec![
            elem(
                "a",
                ElementKind::Sticky,
                Rect::new(0.0, 10.0, 20.0, 30.0),
                false,
                None,
            ),
            elem(
                "b",
                ElementKind::Sticky,
                Rect::new(60.0, 40.0, 80.0, 70.0),
                false,
                None,
            ),
        ]);
        let bar = build_floatbar(
            &[eid("a"), eid("b")],
            &els(&dv),
            &cam(),
            CssPoint::default(),
        )
        .expect("bar");
        assert_eq!(bar.at, page(540.0, 410.0));
        assert!(bar.multi);
    }

    #[test]
    fn flags_track_group_and_lock_state() {
        let dv = viewport(vec![
            elem(
                "g",
                ElementKind::Group,
                Rect::new(0.0, 0.0, 10.0, 10.0),
                false,
                None,
            ),
            elem(
                "l",
                ElementKind::Sticky,
                Rect::new(0.0, 0.0, 5.0, 5.0),
                true,
                None,
            ),
        ]);
        let bar = build_floatbar(
            &[eid("g"), eid("l")],
            &els(&dv),
            &cam(),
            CssPoint::default(),
        )
        .expect("bar");
        assert!(bar.has_group);
        assert!(bar.any_locked);
        assert!(bar.any_unlocked);
        assert!(bar.multi);
    }

    #[test]
    fn common_fill_is_none_when_mixed() {
        let dv = viewport(vec![
            elem(
                "a",
                ElementKind::Sticky,
                Rect::new(0.0, 0.0, 5.0, 5.0),
                false,
                Some(Rgba::from_bits(0xff00_00ff)),
            ),
            elem(
                "b",
                ElementKind::Sticky,
                Rect::new(6.0, 0.0, 10.0, 5.0),
                false,
                Some(Rgba::from_bits(0x0000_ffff)),
            ),
        ]);
        let bar = build_floatbar(
            &[eid("a"), eid("b")],
            &els(&dv),
            &cam(),
            CssPoint::default(),
        )
        .expect("bar");
        assert_eq!(bar.fill, None);
    }

    #[test]
    fn common_fill_survives_when_shared() {
        let dv = viewport(vec![
            elem(
                "a",
                ElementKind::Shape(ShapeKind::Rect),
                Rect::new(0.0, 0.0, 5.0, 5.0),
                false,
                Some(Rgba::from_bits(0x1122_33ff)),
            ),
            elem(
                "b",
                ElementKind::Shape(ShapeKind::Rect),
                Rect::new(6.0, 0.0, 10.0, 5.0),
                false,
                Some(Rgba::from_bits(0x1122_33ff)),
            ),
        ]);
        let bar = build_floatbar(
            &[eid("a"), eid("b")],
            &els(&dv),
            &cam(),
            CssPoint::default(),
        )
        .expect("bar");
        assert_eq!(bar.fill, Some(Rgba::from_bits(0x1122_33ff)));
    }

    // A `StyleEdit` overwrites exactly its own field and nothing else. That is the
    // no-clobber property the floating bar relies on: each control emits one field,
    // applied on the live common style.
    #[test]
    fn style_edit_touches_only_its_field() {
        let base = ElementStyle {
            fill: Some(Rgba::from_bits(0x1122_33ff)),
            text_color: Some(Rgba::from_bits(0x4455_66ff)),
            font: ambre_bus::app::toile::FontSpec {
                size: FontSizeChoice::from_whole_points(Some(18)),
                weight: ambre_core::FontWeight::BOLD,
                italic: true,
                underline: false,
                strikethrough: true,
                ..Default::default()
            },
            ..ElementStyle::default()
        };

        // Toggling underline leaves every other attribute (incl. the other three
        // decorations, the fill and the text colour) exactly as it was.
        let mut s = base.clone();
        StyleEdit::Underline(true).apply_to(&mut s);
        assert!(s.font.underline);
        assert_eq!(s.fill, base.fill);
        assert_eq!(s.text_color, base.text_color);
        assert_eq!(s.font.weight, base.font.weight);
        assert_eq!(s.font.italic, base.font.italic);
        assert_eq!(s.font.strikethrough, base.font.strikethrough);
        assert_eq!(s.font.size, base.font.size);

        // Fill edit preserves the whole text style.
        let mut s = base.clone();
        StyleEdit::Fill(None).apply_to(&mut s);
        assert_eq!(s.fill, None);
        assert_eq!(s.font, base.font);
        assert_eq!(s.text_color, base.text_color);
    }

    // Bold / size map onto the underlying representation (weight 700/400, an
    // automatic size); the family lands as the picker named it.
    #[test]
    fn style_edit_field_mappings() {
        let eighteen = FontSizeChoice::from_whole_points(Some(18));
        let mut s = ElementStyle {
            font: FontSpec {
                size: eighteen,
                ..Default::default()
            },
            ..ElementStyle::default()
        };
        StyleEdit::Bold(false).apply_to(&mut s);
        assert_eq!(s.font.weight, ambre_core::FontWeight::REGULAR);
        StyleEdit::Bold(true).apply_to(&mut s);
        assert_eq!(s.font.weight, ambre_core::FontWeight::BOLD);
        // A negative size is refused, never clamped to automatic: no choice
        // holds one, and a decoded one does not decode.
        assert!(
            FontSizeChoice::try_from(-3.0_f32).is_err(),
            "a negative size is refused"
        );
        assert_eq!(s.font.size, eighteen);
        // Automatic is a meaning the control has.
        StyleEdit::Size(FontSizeChoice::AUTO).apply_to(&mut s);
        assert_eq!(s.font.size, FontSizeChoice::AUTO, "Auto lands as automatic");
        assert_eq!(FontSizeChoice::try_from(0.0_f32), Ok(FontSizeChoice::AUTO));
        StyleEdit::Font(Some(FontFamily::Serif)).apply_to(&mut s);
        assert_eq!(s.font.family, Some(FontFamily::Serif));
        StyleEdit::Font(None).apply_to(&mut s);
        assert_eq!(s.font.family, None, "the reset names no family");
        assert_eq!(s.font.face(), &FontFamily::Sans, "and draws the default face");
        StyleEdit::Font(Some(FontFamily::Sans)).apply_to(&mut s);
        assert_eq!(s.font.family, Some(FontFamily::Sans));
    }

    /// `TOILE-UI-172`: a panel edit writes its own field and keeps every other
    /// one as the selection had it. That includes a family the picker cannot
    /// name and a weight that is neither bold nor regular.
    #[test]
    fn a_panel_edit_keeps_every_field_it_did_not_edit() {
        let base = ElementStyle {
            fill: Some(Rgba::from_bits(0x1122_33ff)),
            stroke: Some(Rgba::from_bits(0x4455_66ff)),
            stroke_w: 3.0,
            opacity: ambre_core::Opacity::new(0.5).unwrap(),
            corner: 7.0,
            text_color: Some(Rgba::from_bits(0x7788_99ff)),
            font: FontSpec {
                family: Some(FontFamily::Other("Comic Sans MS".to_owned())),
                size: FontSizeChoice::from_whole_points(Some(18)),
                weight: ambre_core::FontWeight::new(600).unwrap(),
                italic: true,
                ..Default::default()
            },
            ..ElementStyle::default()
        };
        let edits = [
            (
                StyleEdit::Fill(None),
                ElementStyle {
                    fill: None,
                    ..base.clone()
                },
            ),
            (
                StyleEdit::Stroke(Some(Rgba::from_bits(0x0000_0100))),
                ElementStyle {
                    stroke: Some(Rgba::from_bits(0x0000_0100)),
                    ..base.clone()
                },
            ),
            (
                StyleEdit::StrokeWidth(-2.0),
                ElementStyle {
                    stroke_w: 0.0,
                    ..base.clone()
                },
            ),
            (
                StyleEdit::Opacity(ambre_core::Opacity::clamped(2.0)),
                ElementStyle {
                    opacity: ambre_core::Opacity::OPAQUE,
                    ..base.clone()
                },
            ),
            (
                StyleEdit::TextColor(None),
                ElementStyle {
                    text_color: None,
                    ..base.clone()
                },
            ),
        ];
        for (edit, expected) in edits {
            let mut s = base.clone();
            edit.clone().apply_to(&mut s);
            assert_eq!(s, expected, "{edit:?} touched another field");
        }
    }

    /// `TOILE-UI-164`: the bar folds every field it edits. A field that
    /// differs across the selection reads as mixed, not as the first
    /// member's value.
    #[test]
    fn bar_reports_every_edited_field_mixed() {
        let a = ElementStyle {
            fill: Some(Rgba::from_bits(0x1122_33ff)),
            text_color: Some(Rgba::from_bits(0x0000_00ff)),
            font: FontSpec {
                family: Some(FontFamily::Sans),
                size: FontSizeChoice::from_whole_points(Some(12)),
                weight: ambre_core::FontWeight::REGULAR,
                italic: false,
                underline: false,
                strikethrough: false,
            },
            ..ElementStyle::default()
        };
        let b = ElementStyle {
            fill: Some(Rgba::from_bits(0x1122_33ff)),
            text_color: Some(Rgba::from_bits(0xffff_ffff)),
            font: FontSpec {
                family: Some(FontFamily::Serif),
                size: FontSizeChoice::from_whole_points(Some(24)),
                weight: ambre_core::FontWeight::BOLD,
                italic: true,
                underline: true,
                strikethrough: true,
            },
            ..ElementStyle::default()
        };
        let shared_fill = a.fill;
        let dv = viewport(vec![styled_elem("a", a), styled_elem("b", b)]);
        let bar = build_floatbar(
            &[eid("a"), eid("b")],
            &els(&dv),
            &cam(),
            CssPoint::default(),
        )
        .expect("bar");
        assert!(bar.mixed.text_color);
        assert!(bar.mixed.font_family);
        assert!(bar.mixed.font_size);
        assert!(bar.mixed.bold);
        assert!(bar.mixed.italic);
        assert!(bar.mixed.underline);
        assert!(bar.mixed.strikethrough);
        // The shared fill is not mixed, and it reaches the swatch.
        assert!(!bar.mixed.fill);
        assert_eq!(bar.fill, shared_fill);
    }
}
