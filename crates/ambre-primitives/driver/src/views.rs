//! The rules over the views and the surface, read with types.
//!
//! `crate_boundaries.py` read these on the text of a declaration or of a
//! match arm. So an alias of a type, an enum renamed by a `use` and a field a
//! macro wrote passed unseen. Here each type is the one rustc resolves, and
//! each arm names the variants its pattern resolves to.
//!
//! | Check | Rows |
//! | :--- | :--- |
//! | a parameter of a drawing primitive that is neither geometry nor paint | `CANVAS-056` |
//! | a shell signal that carries a bare primitive | `UI-THREAD-060` |
//! | a view field of a type a shell signal carries, a fact the judge joins | `UI-THREAD-059` |
//! | an event a relay arm writes a signal off and a view arm stores, facts the judge joins | `UI-THREAD-003` |
//! | an interface file that names the canvas beside a page point | `UI-THREAD-001` |
//! | a canvas element built by name outside the surface | `UI-THREAD-058` |
//! | a walk that reaches a module its crate does not list, or names screen state | `CANVAS-074` |
//! | a scroll setter that takes no step of the model | `KINETIC-001` |

use std::collections::BTreeMap;

use rustc_hir as hir;
use rustc_hir::def::{CtorOf, DefKind, Res};
use rustc_hir::def_id::{DefId, LocalDefId};
use rustc_lint::LateContext;
use rustc_middle::ty::{self, Ty, TyCtxt};
use rustc_span::Span;

use crate::Gate;
use crate::bounds::{INTERFACE, POINTER_OWNERS, Reached, reached, ui_thread};
use crate::names::SCROLL_SETTERS;
use crate::place;
use crate::types;

/// The file of the drawing trait (`CANVAS-056`).
const DRAW_TRAIT_FILE: &str = "crates/ambre-draw/src/lib.rs";

/// The types a drawing primitive takes: geometry and paint. A colour is the
/// `AlphaColor` of the `Srgb` space, and a length is a `Px` of the `Surface`
/// (`CANVAS-056`).
const PRIMITIVE_TYPES: &[&str] = &[
    "Rect",
    "Line",
    "AlphaColor",
    "Srgb",
    "Px",
    "Surface",
    "Affine",
    "BezPath",
    "Stroke",
    "Brush",
    "ParleyBrush",
    "GlyphRun",
    // The text a glyph run was shaped from: document content, as the run
    // is, and what the PDF surface writes each glyph's characters from
    // (`PDF-005`).
    "LayoutText",
    "ImageSource",
    // The paints a `Brush` holds by default: an image and a gradient.
    "ImageBrush",
    "ImageData",
    "Gradient",
    // A picture still in its file, which a print surface embeds as it is
    // and a screen surface ignores: a paint, as `ImageData` is (`CANVAS-026`).
    "EncodedImage",
];

/// The parameter names under which a bare number is geometry (`CANVAS-056`).
const NUMBER_NAMES: &[&str] = &["width"];

/// The structs holding the shell's signals: the launcher's wiring and each
/// application's relay (`UI-THREAD-059`, `UI-THREAD-060`).
const SIGNAL_HOLDERS: &[(&str, &str)] = &[
    ("crates/ambre-app/src/lib.rs", "Wiring"),
    ("crates/ambre-trame/src/chrome/relay.rs", "TrameRelay"),
    ("crates/ambre-toile/src/chrome/relay.rs", "ToileRelay"),
];

/// A view's own state: the structs of each application's view and render
/// code (`UI-THREAD-059`, `UI-THREAD-003`).
const VIEW_STATE: &[&str] = &[
    "crates/ambre-trame/src/ui/",
    "crates/ambre-trame/src/render/",
    "crates/ambre-toile/src/ui/",
    "crates/ambre-prose/src/ui.rs",
    "crates/ambre-prose/src/view/",
];

/// Each application's relay, which writes signals off its events
/// (`UI-THREAD-003`).
const EVENT_RELAYS: &[&str] = &[
    "crates/ambre-trame/src/chrome/relay.rs",
    "crates/ambre-toile/src/chrome/relay.rs",
];

/// The application events a relay and a view both read (`UI-THREAD-003`).
const RELAYED_EVENTS: &[&str] = &["TrameEvent", "ToileEvent"];

/// A flag that asks for a repaint holds no state the event carries
/// (`UI-THREAD-003`).
const REPAINT_FLAGS: &[&str] = &["needs_full", "data_dirty"];

/// A wrapper holds its payload's state, so a type is read through it.
const TYPE_WRAPPERS: &[&str] = &[
    "Option", "Vec", "VecDeque", "Box", "Rc", "Arc", "RefCell", "Cell", "Cow",
];

/// A signal whose payload a reviewer accepts as a primitive, and why
/// (`UI-THREAD-060`). Free text has no unit and no closed set, so a type of
/// its own would name nothing.
const BARE_PAYLOAD_ACCEPTED: &[(&str, &str)] = &[
    (
        "crates/ambre-app/src/lib.rs:Wiring.username",
        "free text: the display name the server reports",
    ),
    (
        "crates/ambre-app/src/lib.rs:Wiring.formula_ref",
        "free text: the reference label the formula bar shows",
    ),
    (
        "crates/ambre-app/src/lib.rs:Wiring.formula_text",
        "free text: the cell's raw text, as typed",
    ),
    (
        "crates/ambre-app/src/lib.rs:Wiring.filter_values",
        "free text: the column's distinct display values",
    ),
    (
        "crates/ambre-trame/src/chrome/relay.rs:TrameRelay.filter_values",
        "free text: the column's distinct display values",
    ),
    (
        "crates/ambre-app/src/lib.rs:Wiring.presenter_notes",
        "free text: the speaker notes of the slide",
    ),
    (
        "crates/ambre-toile/src/chrome/relay.rs:ToileRelay.presenter_notes",
        "free text: the speaker notes of the slide",
    ),
];

/// A view field a reviewer accepts beside a signal of its type, and why
/// (`UI-THREAD-059`).
const ONE_COPY_ACCEPTED: &[(&str, &str)] = &[
    (
        "crates/ambre-trame/src/ui/view/mod.rs:TrameView.menu_scope",
        "the host build holds no signal; the scope is written here and into the signal in one call",
    ),
    (
        "crates/ambre-trame/src/ui/view/mod.rs:FormulaBar.action",
        "the host build holds no signal; the frame copies the signal into it and clears the signal once served",
    ),
    (
        "crates/ambre-toile/src/ui/tool.rs:ToolMachine.tool",
        "the host build holds no signal; the frame reads the signal into it and publishes it back",
    ),
    (
        "crates/ambre-toile/src/ui/style.rs:StyleModel.base",
        "the selection's common style, which the inspector shows; the restyle signal carries an edit",
    ),
    (
        "crates/ambre-toile/src/ui/view.rs:ToileView.present",
        "the host build holds no signal; one PresentModeChanged event writes it here and into the signal",
    ),
    (
        "crates/ambre-toile/src/ui/viewport.rs:ViewportTransform.zoom",
        "the camera the frame draws at; the frame reads a chip pick from the zoom signal and publishes the camera back",
    ),
    (
        "crates/ambre-toile/src/ui/view.rs:ToileView.zoom_published",
        "the zoom the frame last published, which tells a chip pick in the signal from the frame's own write",
    ),
    (
        "crates/ambre-toile/src/ui/present.rs:Transition.cam_to_zoom",
        "the zoom a slide glide ends on, not the zoom the board shows",
    ),
    (
        "crates/ambre-trame/src/ui/grid/mod.rs:TrameUiState.zoom",
        "the host build holds no signal; the frame reads a chip pick from the signal and publishes its clamp back",
    ),
    (
        "crates/ambre-trame/src/render/layout.rs:GridLayout.zoom",
        "a copy the walk borrows for one paint, built from the grid state and dropped with the frame",
    ),
    (
        "crates/ambre-prose/src/view/column.rs:Viewport.zoom",
        "Prose trades no zoom signal: the zoom signal is the canvas apps' status chip",
    ),
];

/// The modules a walk may reach, per application crate, and why each holds
/// no screen state (`CANVAS-074`). A module is the one that declares the
/// item, so a re-export reaches what it names. A module not listed is
/// refused, so a new reach is a reviewed line here.
const WALK_REACH: &[(&str, &str, &str)] = &[
    (
        "ambre-prose",
        "crate::layout::run",
        "the shaped runs the page lays out",
    ),
    (
        "ambre-prose",
        "crate::view::cache",
        "the run cache, keyed by block and holding no camera",
    ),
    (
        "ambre-prose",
        "crate::view",
        "the highlight types the block paints carry",
    ),
    (
        "ambre-prose",
        "crate::message::picture",
        "the decoded or printed picture a block draws, shared and read-only",
    ),
    ("ambre-trame", "crate", "the sheet's dimensions"),
    (
        "ambre-trame",
        "crate::hash",
        "the hash table type, which holds no state of its own",
    ),
    (
        "ambre-trame",
        "crate::render",
        "the sheet's layout and cell drawing, in sheet coordinates",
    ),
    (
        "ambre-trame",
        "crate::render::layout",
        "the sheet's layout in sheet coordinates, borrowed for one paint",
    ),
    (
        "ambre-trame",
        "crate::render::cells",
        "the cell drawing, in sheet coordinates",
    ),
    (
        "ambre-trame",
        "crate::render::content",
        "the cell content a paint draws, in sheet coordinates",
    ),
    (
        "ambre-trame",
        "crate::render::grid",
        "the grid lines, in sheet coordinates",
    ),
    (
        "ambre-trame",
        "crate::render::merges",
        "the merged ranges a paint spans, in sheet coordinates",
    ),
    (
        "ambre-toile",
        "crate::logic::animate",
        "the presented element's animated state",
    ),
    (
        "ambre-toile",
        "crate::message",
        "the culled display list the Logic thread sends, in board units",
    ),
    (
        "ambre-toile",
        "crate::ui::label",
        "the label geometry, in board units",
    ),
];

/// Whether a name is the screen's state: a UI state, a viewport, an editor,
/// a caret, a clock or the display density (`CANVAS-074`).
fn screen_state(name: &str) -> bool {
    name.ends_with("UiState")
        || [
            "Viewport",
            "ViewportTransform",
            "TextEditor",
            "CursorPosition",
            "Clock",
            "device_pixel_ratio",
        ]
        .contains(&name)
}

/// What one file of an interface component names, for `UI-THREAD-001`: a
/// page point read and the canvas named by its class or id.
#[derive(Default)]
pub struct PagePoint {
    page_point: Option<String>,
    canvas_name: Option<String>,
}

/// The type under each reference and each wrapper of [`TYPE_WRAPPERS`].
fn peeled<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Ty<'tcx> {
    let mut ty = ty;
    loop {
        match ty.kind() {
            ty::Ref(_, inner, _) => ty = *inner,
            ty::Adt(adt, args)
                if TYPE_WRAPPERS.contains(&tcx.item_name(adt.did()).as_str())
                    && let Some(inner) = args.types().next() =>
            {
                ty = inner;
            }
            _ => return ty,
        }
    }
}

/// The primitive, tuple or array a payload carries once its wrappers are
/// peeled, by name; `None` for a type of its own.
pub fn bare_payload<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<String> {
    let ty = peeled(tcx, ty);
    match ty.kind() {
        ty::Tuple(items) if !items.is_empty() => Some(ty.to_string()),
        ty::Array(..) | ty::Slice(_) => Some(ty.to_string()),
        _ => types::primitive_name(tcx, ty),
    }
}

/// The state a type holds, as every crate names it (`UI-THREAD-059`). The
/// wrappers are peeled, and the path from its crate keeps the arguments of
/// a family. `None` for a primitive, a tuple, an array or an identifier: a
/// view keeps a hundred of each.
fn state_type<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<String> {
    let ty = peeled(tcx, ty);
    let ty::Adt(adt, args) = ty.kind() else {
        return None;
    };
    let name = tcx.item_name(adt.did());
    if types::primitive_name(tcx, ty).is_some() || name.as_str().ends_with("Id") {
        return None;
    }
    let arguments: Vec<String> = args
        .types()
        .map(|a| match a.kind() {
            ty::Adt(inner, _) => place::stable_path(tcx, inner.did()),
            _ => a.to_string(),
        })
        .collect();
    let path = place::stable_path(tcx, adt.did());
    Some(if arguments.is_empty() {
        path
    } else {
        format!("{path}<{}>", arguments.join(","))
    })
}

/// The payload of a signal field, when its type is a signal of the
/// interface toolkit.
fn signal_payload<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<Ty<'tcx>> {
    let ty::Adt(adt, args) = ty.kind() else {
        return None;
    };
    let name = tcx.item_name(adt.did());
    let krate = tcx.crate_name(adt.did().krate);
    let toolkit = ["leptos", "reactive_graph", "reactive_stores"].contains(&krate.as_str());
    (toolkit && name.as_str().ends_with("Signal"))
        .then(|| args.types().next())
        .flatten()
}

/// Whether a type is a signal of the interface toolkit, wrapped or not.
fn holds_signal<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> bool {
    ty.walk()
        .filter_map(|a| a.as_type())
        .any(|t| signal_payload(tcx, t).is_some())
}

/// The variant a pattern path resolves to, as `TrameEvent::SheetList`, when
/// its enum is one of [`RELAYED_EVENTS`].
fn event_variant(tcx: TyCtxt<'_>, res: Res) -> Option<String> {
    let Res::Def(kind, def) = res else {
        return None;
    };
    let variant = match kind {
        DefKind::Variant => def,
        DefKind::Ctor(CtorOf::Variant, _) => tcx.opt_parent(def)?,
        _ => return None,
    };
    let enum_def = tcx.opt_parent(variant)?;
    let enum_name = tcx.item_name(enum_def);
    RELAYED_EVENTS
        .contains(&enum_name.as_str())
        .then(|| format!("{enum_name}::{}", tcx.item_name(variant)))
}

/// The field chain an assignment writes, from `self`: `cache.dirty`.
fn self_chain(expr: &hir::Expr<'_>) -> Option<String> {
    let mut chain = Vec::new();
    let mut at = expr;
    loop {
        match at.kind {
            hir::ExprKind::Field(base, field) => {
                chain.push(field.name.to_string());
                at = base;
            }
            hir::ExprKind::Path(hir::QPath::Resolved(None, path))
                if path.segments.len() == 1
                    && path
                        .segments
                        .first()
                        .is_some_and(|s| s.ident.name == rustc_span::symbol::kw::SelfLower) =>
            {
                break;
            }
            _ => return None,
        }
    }
    (!chain.is_empty()).then(|| {
        chain.reverse();
        chain.join(".")
    })
}

/// What one arm body does: each signal it sets through `self`, and each
/// field chain of `self` it stores.
#[derive(Default)]
struct ArmBody {
    signals: Vec<String>,
    stores: Vec<(Span, String)>,
}

impl<'tcx> hir::intravisit::Visitor<'tcx> for ArmBody {
    fn visit_expr(&mut self, e: &'tcx hir::Expr<'tcx>) {
        match e.kind {
            hir::ExprKind::MethodCall(seg, receiver, _, _)
                if ["set", "update"].contains(&seg.ident.as_str()) =>
            {
                if let hir::ExprKind::Field(base, field) = receiver.kind
                    && self_chain(base).is_none()
                    && matches!(base.kind, hir::ExprKind::Path(_))
                    && self_chain(receiver).is_some()
                {
                    self.signals.push(field.name.to_string());
                }
            }
            hir::ExprKind::Assign(lhs, _, _) | hir::ExprKind::AssignOp(_, lhs, _) => {
                if let Some(chain) = self_chain(lhs)
                    && !REPAINT_FLAGS.contains(&chain.as_str())
                {
                    self.stores.push((lhs.span, chain));
                }
            }
            _ => {}
        }
        hir::intravisit::walk_expr(self, e);
    }
}

impl Gate {
    /// Refuses a parameter of a drawing primitive that is neither geometry
    /// nor paint, read with its type once each alias resolves
    /// (`CANVAS-056`).
    pub fn draw_primitives(&mut self, tcx: TyCtxt<'_>, item: &hir::Item<'_>, def: LocalDefId) {
        if self.test || self.unit.krate != "ambre-draw" {
            return;
        }
        let hir::ItemKind::Trait { .. } = item.kind else {
            return;
        };
        if tcx.item_name(def.to_def_id()).as_str() != "DrawSurface"
            || !self.in_file(tcx, item.span, |f| f == DRAW_TRAIT_FILE)
        {
            return;
        }
        let mut held = 0;
        for assoc in tcx.associated_items(def).in_definition_order() {
            if !matches!(assoc.kind, ty::AssocKind::Fn { .. }) {
                continue;
            }
            held += 1;
            let method = tcx.item_name(assoc.def_id);
            let sig = tcx
                .fn_sig(assoc.def_id)
                .instantiate_identity()
                .skip_normalization()
                .skip_binder();
            let names = tcx.fn_arg_idents(assoc.def_id);
            let has_self = assoc.is_method();
            for (index, input) in sig.inputs().iter().enumerate() {
                if has_self && index == 0 {
                    continue;
                }
                let label = names
                    .get(index)
                    .copied()
                    .flatten()
                    .map_or_else(|| "_".to_owned(), |i| i.to_string());
                let foreign =
                    input
                        .walk()
                        .filter_map(|a| a.as_type())
                        .find_map(|t| match t.kind() {
                            ty::Adt(adt, _) => {
                                let name = tcx.item_name(adt.did());
                                (!PRIMITIVE_TYPES.contains(&name.as_str()))
                                    .then(|| name.to_string())
                            }
                            ty::Foreign(_) | ty::Param(_) | ty::Alias(..) | ty::Dynamic(..) => {
                                Some(t.to_string())
                            }
                            ty::Float(_) if !NUMBER_NAMES.contains(&label.as_str()) => {
                                Some(format!("a bare number `{t}`"))
                            }
                            ty::Bool | ty::Char | ty::Int(_) | ty::Uint(_) | ty::Str => {
                                Some(t.to_string())
                            }
                            _ => None,
                        });
                if let Some(found) = foreign {
                    let span = tcx.def_span(assoc.def_id);
                    let what = format!(
                        "`{method}` takes `{label}: {input}`, which is neither geometry nor \
                         paint: `{found}`"
                    );
                    self.wired(tcx, span, "CANVAS-056", &what);
                }
            }
        }
        if held == 0 {
            self.wired(
                tcx,
                item.span,
                "CANVAS-056",
                "the `DrawSurface` trait declares no primitive, so the boundary holds nothing",
            );
        }
    }

    /// Reads the signals of a shell holder and the state fields of a view
    /// (`UI-THREAD-059`, `UI-THREAD-060`).
    pub fn view_fields(&mut self, tcx: TyCtxt<'_>, item: &hir::Item<'_>, def: LocalDefId) {
        if self.test {
            return;
        }
        let hir::ItemKind::Struct(_, _, hir::VariantData::Struct { .. }) = item.kind else {
            return;
        };
        let name = tcx.item_name(def.to_def_id()).to_string();
        let holder = SIGNAL_HOLDERS
            .iter()
            .find(|(file, holder)| *holder == name && self.in_file(tcx, item.span, |f| f == *file))
            .map(|(file, _)| *file);
        let view = self.in_file(tcx, item.span, |f| {
            VIEW_STATE.iter().any(|v| f.starts_with(v))
        });
        if holder.is_none() && !view {
            return;
        }
        let adt = tcx.adt_def(def);
        let mut signals = 0;
        for field in adt.all_fields() {
            let ty = tcx
                .type_of(field.did)
                .instantiate_identity()
                .skip_normalization();
            let span = tcx.def_span(field.did);
            let key = |file: &str| format!("{file}:{name}.{}", field.name);
            if let Some(file) = holder
                && let Some(payload) = signal_payload(tcx, ty)
            {
                signals += 1;
                let key = key(file);
                if let Some(bare) = bare_payload(tcx, payload)
                    && !BARE_PAYLOAD_ACCEPTED.iter().any(|(k, _)| *k == key)
                {
                    let what = format!(
                        "the `{}` signal carries a bare `{bare}`, not a type of its own",
                        field.name
                    );
                    self.wired(tcx, span, "UI-THREAD-060", &what);
                }
                if let Some(state) = state_type(tcx, payload) {
                    let at = place::position(tcx, &self.unit, span).at();
                    self.record(&[
                        "fact",
                        "signal-state",
                        &format!("{} {state}", field.name),
                        &at,
                    ]);
                }
            }
            if view && !holds_signal(tcx, ty) {
                let at = place::position(tcx, &self.unit, span);
                let accepted = ONE_COPY_ACCEPTED.iter().any(|(k, _)| *k == key(&at.shown));
                if !accepted && let Some(state) = state_type(tcx, ty) {
                    let value = format!("{name}.{} {state}", field.name);
                    self.record(&["fact", "view-state", &value, &at.at()]);
                }
            }
        }
        if holder.is_some() && signals == 0 {
            let what = format!("`{name}` holds no signal, so no interface state is read");
            self.wired(tcx, item.span, "UI-THREAD-059", &what);
        }
    }

    /// Reads each arm of a match on an application event: the signals a
    /// relay arm writes, and the fields a view arm stores. The judge joins
    /// the two (`UI-THREAD-003`).
    pub fn event_arms<'tcx>(&mut self, cx: &LateContext<'tcx>, expr: &'tcx hir::Expr<'tcx>) {
        if self.test {
            return;
        }
        let hir::ExprKind::Match(_, arms, _) = expr.kind else {
            return;
        };
        let tcx = cx.tcx;
        let relay = self.in_file(tcx, expr.span, |f| EVENT_RELAYS.contains(&f));
        let view = self.in_file(tcx, expr.span, |f| {
            VIEW_STATE.iter().any(|v| f.starts_with(v))
        });
        if !relay && !view {
            return;
        }
        let typeck = cx.typeck_results();
        for arm in arms {
            let mut variants = Vec::new();
            arm.pat.walk_always(|p| {
                let res = match p.kind {
                    hir::PatKind::Struct(ref qpath, ..)
                    | hir::PatKind::TupleStruct(ref qpath, ..) => {
                        Some(typeck.qpath_res(qpath, p.hir_id))
                    }
                    hir::PatKind::Expr(hir::PatExpr {
                        kind: hir::PatExprKind::Path(qpath),
                        hir_id,
                        ..
                    }) => Some(typeck.qpath_res(qpath, *hir_id)),
                    _ => None,
                };
                if let Some(variant) = res.and_then(|r| event_variant(tcx, r)) {
                    variants.push(variant);
                }
            });
            if variants.is_empty() {
                continue;
            }
            let mut body = ArmBody::default();
            if let Some(guard) = arm.guard {
                hir::intravisit::Visitor::visit_expr(&mut body, guard);
            }
            hir::intravisit::Visitor::visit_expr(&mut body, arm.body);
            let at = place::position(tcx, &self.unit, arm.pat.span).at();
            for variant in &variants {
                if relay {
                    for signal in &body.signals {
                        self.record(&["fact", "relay-writes", &format!("{variant} {signal}"), &at]);
                    }
                }
                if view && let Some((span, chain)) = body.stores.first() {
                    let store = place::position(tcx, &self.unit, *span).at();
                    self.record(&["fact", "view-stores", &format!("{variant} {chain}"), &store]);
                }
            }
        }
    }

    /// Notes a page point read in an interface file, or the canvas named
    /// there by its class or id (`UI-THREAD-001`).
    pub fn page_point(
        &mut self,
        tcx: TyCtxt<'_>,
        def: Option<DefId>,
        text: Option<&str>,
        span: Span,
    ) {
        if self.test {
            return;
        }
        let point = def.is_some_and(|d| {
            let r = reached(tcx, d);
            r.krate == "web_sys"
                && ["client", "page", "screen"]
                    .iter()
                    .any(|p| r.name == format!("{p}_x") || r.name == format!("{p}_y"))
        });
        let canvas = text.is_some_and(|t| t == "ambre-canvas" || t == ".ambre-canvas");
        if !point && !canvas {
            return;
        }
        let interface = |f: &str| {
            ui_thread(f)
                && INTERFACE.iter().any(|i| f.starts_with(i))
                && !POINTER_OWNERS.iter().any(|o| f.starts_with(o))
        };
        if !self.in_file(tcx, span, interface) {
            return;
        }
        let at = place::position(tcx, &self.unit, span);
        let entry = self.page_points.entry(at.shown.clone()).or_default();
        if point {
            entry.page_point.get_or_insert_with(|| at.at());
        }
        if canvas {
            entry.canvas_name.get_or_insert_with(|| at.at());
        }
    }

    /// Refuses each interface file that turns a page point into a canvas
    /// point: it names the canvas beside a page-relative read
    /// (`UI-THREAD-001`).
    pub fn page_points_post(&mut self) {
        let files: BTreeMap<String, PagePoint> = std::mem::take(&mut self.page_points);
        for (file, found) in files {
            if let (Some(point), Some(canvas)) = (found.page_point, found.canvas_name) {
                self.refuse(
                    "boundary",
                    format!(
                        "{file}: an interface component turns a page point into a canvas \
                         point: a page point at {point} beside the canvas named at {canvas} \
                         (UI-THREAD-001)"
                    ),
                );
            }
        }
    }

    /// Refuses `create_element("canvas")` outside the surface, its host and
    /// the shell cell (`UI-THREAD-058`).
    pub fn canvas_element(
        &mut self,
        tcx: TyCtxt<'_>,
        name: &str,
        args: &[hir::Expr<'_>],
        span: Span,
    ) {
        if self.test || name != "create_element" {
            return;
        }
        let canvas = args.first().is_some_and(|a| {
            matches!(a.kind, hir::ExprKind::Lit(lit)
                if matches!(lit.node, rustc_ast::LitKind::Str(text, _) if text.as_str() == "canvas"))
        });
        let closed = |f: &str| {
            ui_thread(f)
                && ![
                    "crates/ambre-canvas/src/",
                    "crates/ambre-app/src/host/",
                    "crates/ambre-chrome/src/app_shell.rs",
                ]
                .iter()
                .any(|o| f.starts_with(o))
        };
        if canvas && self.in_file(tcx, span, closed) {
            self.wired(
                tcx,
                span,
                "UI-THREAD-058",
                "a page element draws: `create_element(\"canvas\")`",
            );
        }
    }

    /// Notes each file that implements `DocumentWalk`, before any path is
    /// read (`CANVAS-074`).
    pub fn find_walks(&mut self, tcx: TyCtxt<'_>, def: LocalDefId) {
        let Some(trait_ref) = tcx.impl_opt_trait_ref(def) else {
            return;
        };
        let trait_def = trait_ref.skip_binder().def_id;
        if tcx.item_name(trait_def).as_str() != "DocumentWalk"
            || tcx.crate_name(trait_def.krate).as_str() != "ambre_draw"
        {
            return;
        }
        let at = place::position(tcx, &self.unit, tcx.def_span(def));
        self.walk_files
            .insert(at.shown, place::module_path(tcx, def));
    }

    /// Refuses a walk that reaches a module its crate does not list, or that
    /// names the screen's state (`CANVAS-074`).
    pub fn walk_reach(&mut self, tcx: TyCtxt<'_>, def: DefId, span: Span) {
        if self.test || self.walk_files.is_empty() {
            return;
        }
        let at = place::position(tcx, &self.unit, span);
        let Some(here) = self.walk_files.get(&at.shown).cloned() else {
            return;
        };
        let r: Reached = reached(tcx, def);
        if r.named_by(screen_state) {
            let what = format!("the walk names screen state: `{}`", tcx.def_path_str(def));
            self.wired(tcx, span, "CANVAS-074", &what);
        }
        let Some(local) = def.as_local() else {
            return;
        };
        if matches!(
            tcx.def_kind(def),
            DefKind::Mod | DefKind::TyParam | DefKind::ConstParam | DefKind::LifetimeParam
        ) {
            return;
        }
        let module = place::module_path(tcx, local);
        let own = module == here || module.starts_with(&format!("{here}::"));
        let listed = WALK_REACH
            .iter()
            .any(|(krate, m, _)| *krate == self.unit.krate && *m == module);
        if !own && !listed {
            let what = format!("the walk reaches `{module}`, which `WALK_REACH` does not list");
            self.wired(tcx, span, "CANVAS-074", &what);
        }
    }

    /// Notes a scroll setter that takes a step of the model
    /// (`KINETIC-001`).
    pub fn scroll_setter(&mut self, tcx: TyCtxt<'_>, def: LocalDefId) {
        if self.test {
            return;
        }
        let name = tcx.item_name(def.to_def_id());
        let span = tcx.def_span(def);
        let Some(&(file, setter)) = SCROLL_SETTERS.iter().find(|(file, setter)| {
            name.as_str() == *setter && self.in_file(tcx, span, |f| f == *file)
        }) else {
            return;
        };
        let sig = tcx
            .fn_sig(def)
            .instantiate_identity()
            .skip_normalization()
            .skip_binder();
        let steps = sig.inputs().iter().any(|t| {
            matches!(peeled(tcx, *t).kind(), ty::Adt(adt, _)
                if tcx.item_name(adt.did()).as_str() == "ScrollStep"
                    && tcx.crate_name(adt.did().krate).as_str() == "ambre_draw")
        });
        if steps {
            self.setters.insert(format!("{file}:{setter}"));
        }
    }

    /// Refuses each scroll setter of a file this compile read that takes no
    /// step of the model, or that is gone from the file (`KINETIC-001`).
    /// The judge refuses a setter file no compile reads, so a moved file
    /// fails closed.
    pub fn scroll_setters_post(&mut self, tcx: TyCtxt<'_>) {
        if self.test {
            return;
        }
        let read: Vec<String> = tcx
            .sess
            .source_map()
            .files()
            .iter()
            // A file of an upstream crate sits in the map too, once a span
            // of it is read: it is that crate's.
            .filter(|f| f.cnum == rustc_span::def_id::LOCAL_CRATE)
            .filter_map(|f| match f.name {
                rustc_span::FileName::Real(ref real) => real.local_path().map(|p| {
                    let path = place::lexical(&self.unit.cwd.join(p));
                    path.strip_prefix(&self.unit.cwd)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .into_owned()
                }),
                _ => None,
            })
            .collect();
        for (file, setter) in SCROLL_SETTERS {
            if read.iter().any(|r| r == file) && !self.setters.contains(&format!("{file}:{setter}"))
            {
                self.refuse(
                    "boundary",
                    format!(
                        "{file}: the scroll setter `{setter}` takes no model step, so a bare \
                         distance reaches the view (KINETIC-001)"
                    ),
                );
            }
        }
    }
}
