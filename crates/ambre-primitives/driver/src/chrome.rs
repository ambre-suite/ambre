//! The chrome rules, read with types.
//!
//! `crate_boundaries.py` read these on the text of a line. So a variant
//! renamed by a `use` passed unseen, and so did a window read through a
//! re-export. A modal a helper macro wrote passed too.
//! Here each name is the item rustc resolves, and each literal is read where
//! a call takes it.
//!
//! | Check | Row |
//! | :--- | :--- |
//! | an invitation built on the UI thread outside the trust decision | `CHROME-050` |
//! | the named create on the UI thread | `CHROME-037` |
//! | the open-time create that sends a name | `CHROME-037` |
//! | a content view that reads the window | `CHROME-034` |
//! | a modal outside the shared dialog | `CHROME-011` |
//! | a drag control in a file that names no `Debouncer` | `CHROME-015` |
//! | a frame created in a file that never sandboxes one | `ARCH-001` |
//! | a media query read outside the density module, the motion query aside | `CHROME-056` |
//! | a two-state attribute whose false state drops it | `CHROME-066` |
//! | a menu in a file that names no key handler of the shared menu | `CHROME-026` |
//!
//! The crypto crate a manifest declares (`CHROME-049`) is the judge's, read
//! from `cargo metadata`.

use std::collections::BTreeMap;

use rustc_hir as hir;
use rustc_hir::def::{DefKind, Res};
use rustc_hir::def_id::DefId;
use rustc_hir::intravisit::{self, Visitor};
use rustc_lint::LateContext;
use rustc_middle::ty::{self, TyCtxt};
use rustc_span::Span;

use crate::Gate;
use crate::bounds::{Bound, Reached, Reads, reached, ui_thread};
use crate::place;

/// The one file on the UI thread that builds an invitation, behind the trust
/// decision (`CHROME-050`).
const INVITE_OWNER: &str = "crates/ambre-app/src/tofu.rs";

/// The one file that renders the native modal (`CHROME-011`).
const MODAL_OWNER: &str = "crates/ambre-chrome/src/dialog.rs";

/// The content views, which lay out from the box they are given
/// (`CHROME-034`).
const CONTENT_VIEWS: &[&str] = &[
    "crates/ambre-trame/src/ui/",
    "crates/ambre-trame/src/render/",
    "crates/ambre-toile/src/ui/",
    "crates/ambre-prose/src/ui.rs",
    "crates/ambre-prose/src/view/",
];

/// The browser names that read the size of the window or of the screen
/// (`CHROME-034`).
const WINDOW_SIZES: &[&str] = &[
    "inner_width",
    "inner_height",
    "outer_width",
    "outer_height",
    "avail_width",
    "avail_height",
    "match_media",
];

/// The crates of the interface toolkit, whose element functions `view!`
/// writes for a tag.
const TOOLKIT: &[&str] = &["tachys", "leptos"];

/// The values of an `<input type>` that report on every pixel of a drag
/// (`CHROME-015`).
const DRAG_TYPES: &[&str] = &["range", "color"];

/// The one file that reads a media query for a threshold: the density
/// (`CHROME-056`).
const DENSITY: &str = "crates/ambre-canvas/src/density.rs";

/// The media query that asks for the user's motion setting, which no width
/// rides (`CHROME-056`).
const MOTION_QUERY: &str = "(prefers-reduced-motion: reduce)";

/// The attributes of the toolkit that report one of two states
/// (`CHROME-066`).
const TWO_STATES: &[&str] = &[
    "aria_pressed",
    "aria_expanded",
    "aria_checked",
    "aria_selected",
];

/// The methods of `bool` that turn `false` into `None`, which drops the
/// attribute (`CHROME-066`).
const DROPS_FALSE: &[&str] = &["then_some", "then"];

/// The key handlers of the shared menu, any of which drives a menu from the
/// keyboard (`CHROME-026`).
const MENU_KEYS: &[&str] = &["on_menu_keydown", "menu_key", "event_menu_key"];

/// The type that coalesces a burst into one command (`CHROME-015`).
const DEBOUNCER: &str = "Debouncer";

/// The fact of an open-time create that sends no name. The judge refuses a
/// workspace that records none (`CHROME-037`).
const LAZY_CREATE: &str = "lazy-create-unnamed";

/// The function of the Sync worker that creates a document on its first
/// open, the body it posts, and the field of the name (`CHROME-037`).
const LAZY_CREATOR: &str = "trigger_lazy_create";
const CREATE_BODY: &str = "CreateDocumentRequest";
const NAME_FIELD: &str = "name_enc";

/// Whether an item reads the size of the window: a size of `Window` or of
/// `Screen`, the screen itself, or a media query (`CHROME-034`).
fn window_size(r: &Reached) -> bool {
    r.krate == "web_sys"
        && (WINDOW_SIZES.contains(&r.name.as_str())
            || (r.name == "screen" && r.owner.as_deref() == Some("Window"))
            || r.named_by(|n| n == "Screen"))
}

/// Whether an item builds a native modal: the `<dialog>` of the toolkit, or
/// the browser's dialog element (`CHROME-011`).
fn modal(r: &Reached) -> bool {
    (TOOLKIT.contains(&r.krate.as_str()) && matches!(r.kind, DefKind::Fn) && r.name == "dialog")
        || (r.krate == "web_sys" && r.named_by(|n| n.starts_with("HtmlDialogElement")))
}

/// The path rules of the chrome, read beside the crate boundaries.
pub(crate) const BOUNDS: &[Bound] = &[
    Bound {
        rows: "CHROME-050",
        what: "an invitation built past the trust decision",
        reads: Reads::ShippedCode,
        closed: |f| ui_thread(f) && f != INVITE_OWNER,
        reaches: |r| r.variant() && r.name == "InviteToDocument",
    },
    Bound {
        rows: "CHROME-037",
        what: "the UI thread creates a named document",
        reads: Reads::ShippedCode,
        closed: ui_thread,
        reaches: |r| r.variant() && r.name == "CreateDocument",
    },
    Bound {
        rows: "CHROME-034",
        what: "a content view reads the window",
        reads: Reads::ShippedCode,
        closed: |f| CONTENT_VIEWS.iter().any(|v| f.starts_with(v)),
        reaches: window_size,
    },
    Bound {
        rows: "CHROME-011",
        what: "a modal outside the shared dialog",
        reads: Reads::ShippedCode,
        closed: |f| ui_thread(f) && f != MODAL_OWNER,
        reaches: modal,
    },
];

/// What one file holds of the chrome rules that read a whole file.
#[derive(Default)]
pub struct File {
    /// Each drag control, where it sits (`CHROME-015`).
    drags: Vec<String>,
    /// Whether the file names the debouncer (`CHROME-015`).
    debounces: bool,
    /// Each frame the file creates, where it sits (`ARCH-001`).
    frames: Vec<String>,
    /// Whether the file sandboxes a frame (`ARCH-001`).
    sandboxes: bool,
    /// Each menu the file renders, where it sits (`CHROME-026`).
    menus: Vec<String>,
    /// Whether the file names a key handler of the shared menu (`CHROME-026`).
    keyed: bool,
}

/// What the arguments of a call hold, at any depth, closures included. The
/// value of an attribute reaches its method through what `view!` writes.
struct Held<'a, 'tcx> {
    cx: &'a LateContext<'tcx>,
    /// Each string literal.
    strings: Vec<String>,
    /// Whether a method of `bool` turns `false` into `None` (`CHROME-066`).
    drops_false: bool,
}

impl<'tcx> Visitor<'tcx> for Held<'_, 'tcx> {
    type NestedFilter = rustc_middle::hir::nested_filter::OnlyBodies;

    fn maybe_tcx(&mut self) -> TyCtxt<'tcx> {
        self.cx.tcx
    }

    fn visit_expr(&mut self, expr: &'tcx hir::Expr<'tcx>) {
        match expr.kind {
            hir::ExprKind::Lit(lit) => {
                if let rustc_ast::LitKind::Str(text, _) = lit.node {
                    self.strings.push(text.as_str().to_owned());
                }
            }
            hir::ExprKind::MethodCall(seg, receiver, _, _)
                if DROPS_FALSE.contains(&seg.ident.as_str()) =>
            {
                let typeck = self.cx.tcx.typeck(expr.hir_id.owner.def_id);
                if typeck.expr_ty_adjusted(receiver).is_bool() {
                    self.drops_false = true;
                }
            }
            _ => {}
        }
        intravisit::walk_expr(self, expr);
    }
}

/// What the arguments `args` hold.
fn held<'a, 'tcx>(cx: &'a LateContext<'tcx>, args: &'tcx [hir::Expr<'tcx>]) -> Held<'a, 'tcx> {
    let mut found = Held {
        cx,
        strings: Vec::new(),
        drops_false: false,
    };
    for arg in args {
        found.visit_expr(arg);
    }
    found
}

impl Gate {
    /// The file `span` sits in, from the workspace root.
    fn chrome_file(&mut self, tcx: TyCtxt<'_>, span: Span) -> (String, String) {
        let at = place::position(tcx, &self.unit, span);
        (at.shown.clone(), at.at())
    }

    /// Notes a file that names the debouncer (`CHROME-015`).
    pub fn chrome_reach(&mut self, tcx: TyCtxt<'_>, def: DefId, span: Span) {
        if self.test {
            return;
        }
        let r = reached(tcx, def);
        let debouncer = r.named_by(|n| n == DEBOUNCER);
        let keys = matches!(r.kind, DefKind::Fn) && MENU_KEYS.contains(&r.name.as_str());
        if r.krate == "ambre_chrome" && (debouncer || keys) {
            let (file, _) = self.chrome_file(tcx, span);
            let entry = self.chrome.entry(file).or_default();
            entry.debounces |= debouncer;
            entry.keyed |= keys;
        }
    }

    /// The chrome rules of one expression. A call can build a drag control,
    /// a frame, a sandbox, a modal or a media query. A struct can be the body
    /// of the open-time create.
    pub fn chrome_expr<'tcx>(&mut self, cx: &LateContext<'tcx>, expr: &'tcx hir::Expr<'tcx>) {
        match expr.kind {
            hir::ExprKind::MethodCall(seg, _, args, _) => {
                let callee = cx.typeck_results().type_dependent_def_id(expr.hir_id);
                self.chrome_call(cx, callee, seg.ident.as_str(), args, seg.ident.span);
            }
            hir::ExprKind::Call(func, args) => {
                if let hir::ExprKind::Path(ref qpath) = func.kind
                    && let Res::Def(_, callee) = cx.typeck_results().qpath_res(qpath, func.hir_id)
                {
                    let name = cx.tcx.item_name(callee).to_string();
                    self.chrome_call(cx, Some(callee), &name, args, func.span);
                }
            }
            hir::ExprKind::Struct(_, fields, tail) => self.lazy_create(cx, expr, fields, tail),
            _ => {}
        }
    }

    /// Notes or refuses one call by the name it resolves to and the literals
    /// it takes.
    fn chrome_call<'tcx>(
        &mut self,
        cx: &LateContext<'tcx>,
        callee: Option<DefId>,
        name: &str,
        args: &'tcx [hir::Expr<'tcx>],
        span: Span,
    ) {
        let tcx = cx.tcx;
        let Some(callee) = callee else {
            return;
        };
        let r = reached(tcx, callee);
        if !TOOLKIT.contains(&r.krate.as_str()) && r.krate != "web_sys" {
            return;
        }
        let held = held(cx, args);
        let lits = held.strings;
        let first = lits.first().map(String::as_str);
        // A threshold is written once, test code included: a test that
        // queries a width holds a second copy of it.
        if r.krate == "web_sys"
            && name == "match_media"
            && first != Some(MOTION_QUERY)
            && self.in_file(tcx, span, |f| f.starts_with("crates/") && f != DENSITY)
        {
            self.wired(
                tcx,
                span,
                "CHROME-056",
                "a width threshold read outside the stylesheet: `match_media`",
            );
        }
        if self.test {
            return;
        }
        let set_attribute = ["attr", "set_attribute"].contains(&name);
        // `view!` writes a static subtree as its HTML, one string an inert
        // element takes. The string is read where that call takes it.
        let inert: String = if TOOLKIT.contains(&r.krate.as_str())
            && r.owner.as_deref() == Some("InertElement")
            && name == "new"
        {
            lits.concat()
        } else {
            String::new()
        };
        let in_html = |part: &str| inert.contains(part);
        let drag = DRAG_TYPES.iter().any(|t| in_html(&format!("type=\"{t}\"")))
            || (["type", "set_type"].contains(&name)
                && lits.iter().any(|l| DRAG_TYPES.contains(&l.as_str())))
            || (set_attribute
                && first == Some("type")
                && lits
                    .get(1)
                    .is_some_and(|l| DRAG_TYPES.contains(&l.as_str())));
        let created = (name == "create_element").then_some(first).flatten();
        let frame = created == Some("iframe")
            || in_html("<iframe")
            || (TOOLKIT.contains(&r.krate.as_str())
                && matches!(r.kind, DefKind::Fn)
                && name == "iframe");
        let sandbox =
            name == "sandbox" || (set_attribute && first == Some("sandbox")) || in_html(" sandbox");
        let menu = in_html("role=\"menu\"")
            || (name == "role" && first == Some("menu"))
            || (set_attribute
                && first == Some("role")
                && lits.get(1).map(String::as_str) == Some("menu"));
        if TOOLKIT.contains(&r.krate.as_str())
            && TWO_STATES.contains(&name)
            && held.drops_false
            && self.in_file(tcx, span, ui_thread)
        {
            self.wired(
                tcx,
                span,
                "CHROME-066",
                &format!("a two-state attribute that drops its false state: `{name}`"),
            );
        }
        if (created == Some("dialog") || in_html("<dialog"))
            && self.in_file(tcx, span, |f| ui_thread(f) && f != MODAL_OWNER)
        {
            self.wired(
                tcx,
                span,
                "CHROME-011",
                "a modal outside the shared dialog: `<dialog>`",
            );
        }
        if !(drag || frame || sandbox || menu) {
            return;
        }
        let (file, at) = self.chrome_file(tcx, span);
        let entry = self.chrome.entry(file).or_default();
        if drag {
            entry.drags.push(at.clone());
        }
        if frame {
            entry.frames.push(at.clone());
        }
        if sandbox {
            entry.sandboxes = true;
        }
        if menu {
            entry.menus.push(at);
        }
    }

    /// `CHROME-037`: the open-time create posts `name_enc: None`. The driver
    /// reads the body `trigger_lazy_create` builds, by the type rustc gives
    /// it, and records the fact the judge requires.
    fn lazy_create<'tcx>(
        &mut self,
        cx: &LateContext<'tcx>,
        expr: &'tcx hir::Expr<'tcx>,
        fields: &'tcx [hir::ExprField<'tcx>],
        tail: hir::StructTailExpr<'tcx>,
    ) {
        let tcx = cx.tcx;
        if self.test || self.unit.krate != "ambre-sync" {
            return;
        }
        let ty::Adt(adt, _) = cx.typeck_results().expr_ty(expr).kind() else {
            return;
        };
        if tcx.item_name(adt.did()).as_str() != CREATE_BODY {
            return;
        }
        let owner = tcx.typeck_root_def_id(expr.hir_id.owner.to_def_id());
        if tcx
            .opt_item_name(owner)
            .is_none_or(|n| n.as_str() != LAZY_CREATOR)
        {
            return;
        }
        let unnamed = fields
            .iter()
            .find(|f| f.ident.as_str() == NAME_FIELD)
            .is_some_and(|f| none(cx, f.expr));
        let at = place::position(tcx, &self.unit, expr.span).at();
        if unnamed && matches!(tail, hir::StructTailExpr::None) {
            self.record(&["fact", LAZY_CREATE, "None", &at]);
        } else {
            self.wired(
                tcx,
                expr.span,
                "CHROME-037",
                "the open-time create does not send `name_enc: None`",
            );
        }
    }

    /// Refuses what a file holds without its guard (`CHROME-015`,
    /// `CHROME-026`, `ARCH-001`). A drag control needs the debouncer, a menu
    /// a key handler, and a frame a sandbox.
    pub fn chrome_post(&mut self) {
        let files = std::mem::take(&mut self.chrome);
        for (file, held) in files {
            if ui_thread(&file) && !held.debounces {
                for at in held.drags {
                    self.refuse(
                        "boundary",
                        format!("{at}: a drag control that is not debounced (CHROME-015)"),
                    );
                }
            }
            if ui_thread(&file) && !held.keyed {
                for at in held.menus {
                    self.refuse(
                        "boundary",
                        format!("{at}: a menu with no keyboard: it names no key handler of the shared menu (CHROME-026)"),
                    );
                }
            }
            if file.starts_with("crates/")
                && !file.starts_with("crates/ambre-server/")
                && !held.sandboxes
            {
                for at in held.frames {
                    self.refuse(
                        "boundary",
                        format!("{at}: creates a frame and never sandboxes it (ARCH-001)"),
                    );
                }
            }
        }
    }
}

/// Whether `expr` is the `None` of `Option`.
fn none(cx: &LateContext<'_>, expr: &hir::Expr<'_>) -> bool {
    let hir::ExprKind::Path(ref qpath) = expr.kind else {
        return false;
    };
    match cx.typeck_results().qpath_res(qpath, expr.hir_id) {
        Res::Def(DefKind::Ctor(..), ctor) => {
            let r = reached(cx.tcx, ctor);
            r.krate == "core" && r.name == "None" && r.owner.as_deref() == Some("Option")
        }
        _ => false,
    }
}

/// The files of a unit, as the chrome rules read them.
pub type Files = BTreeMap<String, File>;
