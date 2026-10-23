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

use ambre_bus::app::toile::{DisplayElement, ElementId, ElementKind, ElementStyle, FontFamily};
use ambre_core::{FontSize, Rgba};
use ambre_draw::{Point, Rect};

use super::style::MixedFlags;
use super::viewport::ViewportTransform;

/// The floating selection toolbar's published model (`toile-styles`): `x` / `y`
/// are the client CSS-pixel position of the selection's top-edge midpoint. The
/// bar is `position: fixed` there and lifted above the selection with a CSS gap
/// ("air", like Miro). The flags gate the applicable buttons, mirroring the
/// context-menu scope (`ui::menu::CtxScope`). `fill` is the selection's common
/// fill for the quick-colour swatch (`None` when unset or mixed). `mixed` marks
/// every field the bar edits that differs across the selection (`TOILE-UI-164`).
/// Published only for a resolved, visible selection (`None` == hidden).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FloatBar {
    /// Client CSS-pixel x of the selection's top-edge midpoint.
    pub x: f64,
    /// Client CSS-pixel y of the selection's top edge.
    pub y: f64,
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
/// family is the picker label (`None` == the "Default" reset).
#[derive(Clone, PartialEq, Debug)]
pub enum StyleEdit {
    /// Fill colour (`None` == no fill).
    Fill(Option<Rgba>),
    /// Stroke colour (`None` == no stroke). The panel alone emits it.
    Stroke(Option<Rgba>),
    /// Stroke width in world px, floored at zero. The panel alone emits it.
    StrokeWidth(f32),
    /// Opacity, clamped to `[0, 1]`. The panel alone emits it.
    Opacity(f32),
    /// Text (foreground ink) colour (`None` == the kind default ink).
    TextColor(Option<Rgba>),
    /// Font family label (`None` == the "Default" reset -> the app default face).
    Font(Option<String>),
    /// Font size in points (`0.0` == the auto sentinel: size to the card).
    Size(f32),
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
    /// host-tested: both callers are wasm-only. A `None` family maps to the default
    /// sans family, which the renderer resolves to the bundled sans face.
    pub fn apply_to(self, style: &mut ElementStyle) {
        match self {
            StyleEdit::Fill(c) => style.fill = c,
            StyleEdit::Stroke(c) => style.stroke = c,
            StyleEdit::StrokeWidth(w) => style.stroke_w = w.max(0.0),
            StyleEdit::Opacity(o) => style.opacity = o.clamp(0.0, 1.0),
            StyleEdit::TextColor(c) => style.text_color = c,
            StyleEdit::Font(f) => {
                style.font.family = f
                    .as_deref()
                    .map_or(FontFamily::Sans, FontFamily::from_label);
            }
            // A negative size is refused, never clamped to the zero sentinel:
            // the control cannot mean a negative number (`TOILE-UI-180`).
            StyleEdit::Size(s) => {
                if let Some(size) = FontSize::new(s) {
                    style.font.size = size;
                }
            }
            StyleEdit::Bold(b) => style.font.weight = if b { 700 } else { 400 },
            StyleEdit::Italic(i) => style.font.italic = i,
            StyleEdit::Underline(u) => style.font.underline = u,
            StyleEdit::Strikethrough(s) => style.font.strikethrough = s,
        }
    }
}

/// The client CSS-pixel anchor for the bar: the selection's top-edge midpoint.
/// It is mapped through the camera into screen pixels, which are CSS pixels
/// (`CANVAS-032`). A final shift by the canvas's client offset lands the
/// `position: fixed` bar in viewport coordinates. Pure; `offset` is `(0, 0)` in
/// host tests.
pub fn top_center_anchor(
    world_bounds: Rect,
    cam: &ViewportTransform,
    offset: (f64, f64),
) -> (f64, f64) {
    let mid_top = Point::new((world_bounds.x0 + world_bounds.x1) / 2.0, world_bounds.y0);
    let screen = cam.world_to_screen() * mid_top;
    (offset.0 + screen.x, offset.1 + screen.y)
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
    offset: (f64, f64),
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
                mixed.bold |= (s.font.weight >= 700) != (b.font.weight >= 700);
                mixed.italic |= s.font.italic != b.font.italic;
                mixed.underline |= s.font.underline != b.font.underline;
                mixed.strikethrough |= s.font.strikethrough != b.font.strikethrough;
            }
        }
    }

    // Nothing in the selection is present in the culled set: hide the bar.
    let union = union?;
    let (x, y) = top_center_anchor(union, cam, offset);
    let fill = base.and_then(|b| if mixed.fill { None } else { b.fill });
    Some(FloatBar {
        x,
        y,
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
        assert_eq!(build_floatbar(&[], &els(&dv), &cam(), (0.0, 0.0)), None);
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
            build_floatbar(&[eid("ghost")], &els(&dv), &cam(), (0.0, 0.0)),
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
        let bar = build_floatbar(&[eid("a")], &els(&dv), &cam(), (0.0, 0.0)).expect("bar");
        assert!((bar.x - 520.0).abs() < 1e-9);
        assert!((bar.y - 420.0).abs() < 1e-9);
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
        let bar = build_floatbar(&[eid("a")], &els(&dv), &cam(), (40.0, 96.0)).expect("bar");
        assert!((bar.x - 560.0).abs() < 1e-9);
        assert!((bar.y - 516.0).abs() < 1e-9);
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
        let bar =
            build_floatbar(&[eid("a"), eid("b")], &els(&dv), &cam(), (0.0, 0.0)).expect("bar");
        assert!((bar.x - 540.0).abs() < 1e-9);
        assert!((bar.y - 410.0).abs() < 1e-9);
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
        let bar =
            build_floatbar(&[eid("g"), eid("l")], &els(&dv), &cam(), (0.0, 0.0)).expect("bar");
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
        let bar =
            build_floatbar(&[eid("a"), eid("b")], &els(&dv), &cam(), (0.0, 0.0)).expect("bar");
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
        let bar =
            build_floatbar(&[eid("a"), eid("b")], &els(&dv), &cam(), (0.0, 0.0)).expect("bar");
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
                size: ambre_core::FontSize::from_whole_points(18),
                weight: 700,
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

    // Bold / size map onto the underlying representation (weight 700/400, size
    // sentinel); a `None` family resets to the default sans label.
    #[test]
    fn style_edit_field_mappings() {
        let mut s = ElementStyle {
            font: FontSpec {
                size: ambre_core::FontSize::from_whole_points(18),
                ..Default::default()
            },
            ..ElementStyle::default()
        };
        StyleEdit::Bold(false).apply_to(&mut s);
        assert_eq!(s.font.weight, 400);
        StyleEdit::Bold(true).apply_to(&mut s);
        assert_eq!(s.font.weight, 700);
        // A negative size is refused, never clamped to the auto sentinel.
        StyleEdit::Size(-3.0).apply_to(&mut s);
        assert_eq!(
            s.font.size,
            ambre_core::FontSize::from_whole_points(18),
            "a negative size is refused"
        );
        // Zero is the auto sentinel, and it is a meaning the control has.
        StyleEdit::Size(0.0).apply_to(&mut s);
        assert_eq!(
            s.font.size,
            ambre_core::FontSize::from_whole_points(0),
            "zero stays the auto sentinel"
        );
        StyleEdit::Font(None).apply_to(&mut s);
        assert_eq!(s.font.family, FontFamily::Sans);
        StyleEdit::Font(Some("Serif".to_owned())).apply_to(&mut s);
        assert_eq!(s.font.family, FontFamily::Serif);
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
            opacity: 0.5,
            corner: 7.0,
            text_color: Some(Rgba::from_bits(0x7788_99ff)),
            font: FontSpec {
                family: FontFamily::Other("Comic Sans MS".to_owned()),
                size: ambre_core::FontSize::from_whole_points(18),
                weight: 600,
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
                StyleEdit::Opacity(2.0),
                ElementStyle {
                    opacity: 1.0,
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
                family: FontFamily::Sans,
                size: ambre_core::FontSize::from_whole_points(12),
                weight: 400,
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
                family: FontFamily::Serif,
                size: ambre_core::FontSize::from_whole_points(24),
                weight: 700,
                italic: true,
                underline: true,
                strikethrough: true,
            },
            ..ElementStyle::default()
        };
        let shared_fill = a.fill;
        let dv = viewport(vec![styled_elem("a", a), styled_elem("b", b)]);
        let bar =
            build_floatbar(&[eid("a"), eid("b")], &els(&dv), &cam(), (0.0, 0.0)).expect("bar");
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
