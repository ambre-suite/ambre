//! The rules over a name, a literal or a file the compile reads, each read
//! once every macro expands and every path resolves.
//!
//! `crate_boundaries.py` read these on the text of a line. So a name a macro
//! wrote, an item a `use` renamed and a file a macro of another crate
//! included passed unseen. Here a name is the identifier the code declares or
//! the item a path resolves to, and a file is one the compile loaded.
//!
//! | Check | Rows |
//! | :--- | :--- |
//! | a font file a compile embeds | `CANVAS-018` |
//! | a decay, a friction or a velocity named outside the kinetic model | `KINETIC-012` |
//! | an offset moved by a distance outside its setter | `KINETIC-001` |
//! | a frame rate a fixture benchmark names or prints | `FIXTURE-024` |
//! | an axis list reached outside the axis module | `ambre-spec-trame-schema.md` section 5 |
//! | an answer of the relay built with `json!` | `SRV-181` |
//! | a destructor of a thread that writes a document, and the panic strategy of a client build | `ARCH-015` |

use rustc_hir as hir;
use rustc_hir::def::{DefKind, Res};
use rustc_hir::def_id::{DefId, LocalDefId};
use rustc_lint::LateContext;
use rustc_middle::ty::TyCtxt;
use rustc_span::Span;

use crate::Gate;
use crate::bounds::{KINETIC_MODEL, reached};

/// The file extensions of a font (`CANVAS-018`).
const FONT_EXTENSIONS: &[&str] = &["ttf", "otf", "ttc", "woff", "woff2"];

/// The words of a second kinetic model (`KINETIC-012`).
const KINETIC_WORDS: &[&str] = &[
    "friction",
    "decay",
    "velocit",
    "halflife",
    "half_life",
    "inertia",
    "damping",
];

/// The files of each application that move a scroll offset (`KINETIC-001`).
const SCROLL_APPS: &[&str] = &[
    "crates/ambre-prose/src/",
    "crates/ambre-toile/src/",
    "crates/ambre-trame/src/",
    "crates/ambre-murmure/src/",
];

/// The offsets a relative write moves (`KINETIC-001`).
const SCROLL_OFFSETS: &[&str] = &["scroll_y", "center", "anchor_y"];

/// Each view's scroll setter: its file, its name, and the one body that
/// moves the offset by a step of the model (`KINETIC-001`).
pub const SCROLL_SETTERS: &[(&str, &str)] = &[
    ("crates/ambre-prose/src/view/mod.rs", "scroll_by"),
    ("crates/ambre-toile/src/ui/viewport.rs", "pan_by_screen"),
    ("crates/ambre-trame/src/ui/view/scroll.rs", "accumulate"),
    ("crates/ambre-murmure/src/ui/view.rs", "scroll_by"),
];

/// The Trame sources, the axis module, and the registry that declares each
/// container name once (`ambre-spec-trame-schema.md` section 5).
const TRAME_SOURCES: &str = "crates/ambre-trame/src/";
pub const AXIS_MODULE: &str = "crates/ambre-trame/src/model/axis.rs";
const AXIS_REGISTRY: &str = "crates/ambre-trame/src/logic/schema.rs";

/// The axis containers, by variant and by stored name (`ambre-spec-trame-schema.md` section 5).
const AXIS_VARIANTS: &[&str] = &["Rows", "Cols", "LineGen"];
const AXIS_NAMES: &[&str] = &["rows", "cols", "line_gen"];

/// The rule an axis refusal cites: the schema of Trame owns it, so the
/// driver names its document.
const AXIS_RULE: &str = "ambre-spec-trame-schema.md section 5";

/// The tab strip, a movable list that holds no line (`ambre-spec-trame-schema.md` section 5).
const SHEET_ORDER: &str = "SheetOrder";

/// Where the relay builds what it answers (`SRV-181`).
const ANSWER_SOURCES: &[&str] = &[
    "crates/ambre-server/src/routes/",
    "crates/ambre-server/src/error.rs",
    "crates/ambre-server/src/middleware/",
];

/// The threads that write a document: the Logic thread, each engine's
/// command handlers, and the Sync worker (`ARCH-015`).
fn exit_writer(f: &str) -> bool {
    f.starts_with("crates/ambre-logic/src/")
        || f.starts_with("crates/ambre-sync/src/")
        || f.strip_prefix("crates/")
            .and_then(|rest| rest.split_once('/'))
            .is_some_and(|(_, rest)| rest.starts_with("src/logic/"))
}

/// Whether `name` names a part of a second kinetic model.
pub fn kinetic_word(name: &str) -> bool {
    let lower = name.to_lowercase();
    KINETIC_WORDS.iter().any(|w| lower.contains(w))
}

/// Whether `text` names a frame rate: `fps` as a word, a frame rate or
/// frames per second, in any case and with any separator (`FIXTURE-024`).
pub fn frame_rate(text: &str) -> bool {
    let lower = text.to_lowercase();
    let squashed: String = lower.chars().filter(|c| *c != '_' && *c != ' ').collect();
    let word = lower.match_indices("fps").any(|(at, _)| {
        let before = lower[..at].chars().next_back();
        let after = lower[at + 3..].chars().next();
        !before.is_some_and(|c| c.is_ascii_lowercase())
            && !after.is_some_and(|c| c.is_ascii_lowercase())
    });
    word || squashed.contains("framerate")
        || squashed.contains("framepersecond")
        || squashed.contains("framespersecond")
}

/// Whether `text` is the stored name of an axis container, or a key built
/// on one (`ambre-spec-trame-schema.md` section 5).
fn axis_name(text: &str) -> bool {
    AXIS_NAMES
        .iter()
        .any(|n| text == *n || text.starts_with(&format!("{n}_")))
}

/// What a benchmark unit holds, read at its end (`FIXTURE-024`).
#[derive(Default)]
pub struct Bench {
    /// Whether the benchmark reaches a fixture.
    pub fixture: bool,
    /// Each frame rate it names, with its line.
    pub rates: Vec<(String, String)>,
}

impl Gate {
    /// Refuses each font file the compile loaded: an `include_bytes!` read
    /// it, whoever wrote the call (`CANVAS-018`).
    pub fn embedded_fonts(&mut self, tcx: TyCtxt<'_>) {
        for file in tcx.sess.source_map().files().iter() {
            // A font an upstream crate embeds is that crate's refusal.
            if file.cnum != rustc_span::def_id::LOCAL_CRATE {
                continue;
            }
            let rustc_span::FileName::Real(ref real) = file.name else {
                continue;
            };
            let Some(path) = real.local_path() else {
                continue;
            };
            let font = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| FONT_EXTENSIONS.contains(&e.to_lowercase().as_str()));
            if font {
                let shown = crate::place::lexical(&self.unit.cwd.join(path));
                let shown = shown
                    .strip_prefix(&self.unit.cwd)
                    .unwrap_or(&shown)
                    .to_string_lossy()
                    .into_owned();
                self.refuse(
                    "boundary",
                    format!(
                        "{shown}: a font embedded in the binary of {} (CANVAS-018)",
                        self.unit.krate
                    ),
                );
            }
        }
    }

    /// The checks on one name the code writes at `span`: an identifier it
    /// declares, or the segment of a path. `def` is the item the name
    /// resolves to, if any.
    pub fn name_rules(&mut self, tcx: TyCtxt<'_>, name: &str, def: Option<DefId>, span: Span) {
        // A constructor takes the name of its struct or its variant.
        let named = def.map(|d| match tcx.def_kind(d) {
            DefKind::Ctor(..) => tcx.parent(d),
            _ => d,
        });
        let resolved = named
            .and_then(|d| tcx.opt_item_name(d))
            .map(|n| n.to_string());
        if let Some(bench) = self.bench.as_mut() {
            if def.is_some_and(|d| {
                crate::place::stable_path(tcx, d)
                    .to_lowercase()
                    .contains("fixture")
            }) {
                bench.fixture = true;
            }
            for named in std::iter::once(name).chain(resolved.as_deref()) {
                if frame_rate(named) {
                    let at = crate::place::position(tcx, &self.unit, span).at();
                    bench.rates.push((at, named.to_owned()));
                }
            }
            return;
        }
        if self.test {
            return;
        }
        let words: Vec<&str> = std::iter::once(name)
            .chain(resolved.as_deref())
            .filter(|n| kinetic_word(n))
            .collect();
        if let Some(word) = words.first()
            && self.in_file(tcx, span, |f| {
                f.starts_with("crates/") && f != KINETIC_MODEL
            })
        {
            let what = format!("a kinetic model outside `kinetic.rs`: `{word}`");
            self.wired(tcx, span, "KINETIC-012", &what);
        }
    }

    /// The checks on one literal string, `expr` (`FIXTURE-024`,
    /// `ambre-spec-trame-schema.md` section 5). A literal a derive wrote is the name of a field, and
    /// no literal of the source.
    pub fn literal_rules(&mut self, cx: &LateContext<'_>, text: &str, expr: &hir::Expr<'_>) {
        let tcx = cx.tcx;
        let span = expr.span;
        if derived(span) {
            return;
        }
        if let Some(bench) = self.bench.as_mut() {
            if frame_rate(text) {
                let at = crate::place::position(tcx, &self.unit, span).at();
                bench.rates.push((at, text.to_owned()));
            }
            return;
        }
        if self.test || self.unit.krate != "ambre-trame" || !axis_name(text) {
            return;
        }
        // A container name reaches a list when the CRDT engine takes it, or
        // when a constant holds it for a later call. A display word such as
        // "2 rows" reaches none.
        let reaches = match tcx.parent_hir_node(expr.hir_id) {
            hir::Node::Expr(parent) => match parent.kind {
                hir::ExprKind::MethodCall(..) => cx
                    .typeck_results()
                    .type_dependent_def_id(parent.hir_id)
                    .is_some_and(|d| tcx.crate_name(d.krate).as_str().starts_with("loro")),
                hir::ExprKind::Call(func, _) => match func.kind {
                    hir::ExprKind::Path(ref qpath) => {
                        match cx.typeck_results().qpath_res(qpath, func.hir_id) {
                            Res::Def(_, d) => tcx.crate_name(d.krate).as_str().starts_with("loro"),
                            _ => false,
                        }
                    }
                    _ => false,
                },
                _ => false,
            },
            hir::Node::Item(item) => matches!(
                item.kind,
                hir::ItemKind::Const(..) | hir::ItemKind::Static(..)
            ),
            hir::Node::ImplItem(item) => matches!(item.kind, hir::ImplItemKind::Const(..)),
            _ => false,
        };
        let registry = self.in_file(tcx, span, |f| f == AXIS_REGISTRY) && macro_owned(cx);
        if reaches && !registry && self.in_file(tcx, span, trame_outside_axis) {
            let what = format!("an axis list reached outside `model/axis.rs`: \"{text}\"");
            self.wired(tcx, span, AXIS_RULE, &what);
        }
    }

    /// The checks on one resolved item the code reaches at `span`
    /// (`ambre-spec-trame-schema.md` section 5). `arg` is the first argument of a call, when the item is
    /// a method called on one.
    pub fn reach_rules<'tcx>(
        &mut self,
        cx: &LateContext<'tcx>,
        def: DefId,
        span: Span,
        arg: Option<&'tcx hir::Expr<'tcx>>,
    ) {
        let tcx = cx.tcx;
        if self.test || self.unit.krate != "ambre-trame" {
            return;
        }
        let r = reached(tcx, def);
        let variant = r.owner.as_deref() == Some("SheetRoot")
            && AXIS_VARIANTS.contains(&r.name.as_str())
            && matches!(r.kind, DefKind::Variant | DefKind::Ctor(..));
        let list = r.krate.starts_with("loro") && r.name == "get_movable_list";
        if !variant && !list {
            return;
        }
        // The tab strip is a movable list too, and holds no line.
        if list && arg.is_some_and(|a| names_variant(cx, a, SHEET_ORDER)) {
            return;
        }
        let registry = self.in_file(tcx, span, |f| f == AXIS_REGISTRY)
            && (span.from_expansion() || macro_owned(cx));
        if !registry && self.in_file(tcx, span, trame_outside_axis) {
            let what = format!("an axis list reached outside `model/axis.rs`: `{}`", r.name);
            self.wired(tcx, span, AXIS_RULE, &what);
        }
    }

    /// The checks on one expression of a body (`KINETIC-001`, `SRV-181`).
    pub fn expr_rules<'tcx>(&mut self, cx: &LateContext<'tcx>, expr: &'tcx hir::Expr<'tcx>) {
        if self.test {
            return;
        }
        let tcx = cx.tcx;
        if self.unit.krate == "ambre-server" && expr.span.from_expansion() {
            self.ad_hoc_body(tcx, expr.span);
        }
        let moved = match expr.kind {
            hir::ExprKind::AssignOp(op, lhs, _)
                if matches!(
                    op.node,
                    hir::AssignOpKind::AddAssign | hir::AssignOpKind::SubAssign
                ) =>
            {
                Some(lhs)
            }
            hir::ExprKind::Binary(op, lhs, _)
                if matches!(op.node, hir::BinOpKind::Add | hir::BinOpKind::Sub) =>
            {
                Some(lhs)
            }
            _ => None,
        };
        let Some(offset) = moved.and_then(offset_name) else {
            return;
        };
        if !self.in_file(tcx, expr.span, |f| {
            SCROLL_APPS.iter().any(|a| f.starts_with(a)) && f != crate::bounds::GRID_SCROLL
        }) {
            return;
        }
        if let Some(owner) = cx.enclosing_body.map(|b| tcx.hir_body_owner_def_id(b))
            && self.setter(tcx, owner)
        {
            return;
        }
        let what = format!("an offset moved by a distance outside its setter: `{offset}`");
        self.wired(tcx, expr.span, "KINETIC-001", &what);
    }

    /// Whether `def`, or the function around it, is the scroll setter of
    /// its file.
    fn setter(&mut self, tcx: TyCtxt<'_>, def: LocalDefId) -> bool {
        let root = tcx.typeck_root_def_id(def.to_def_id());
        let Some(name) = tcx.opt_item_name(root) else {
            return false;
        };
        let span = tcx.def_span(root);
        SCROLL_SETTERS
            .iter()
            .filter(|(_, setter)| name.as_str() == *setter)
            .any(|(file, _)| self.in_file(tcx, span, |f| f == *file))
    }

    /// Refuses a `json!` body of the relay, whatever name the macro is
    /// called by (`SRV-181`).
    fn ad_hoc_body(&mut self, tcx: TyCtxt<'_>, span: Span) {
        let mut at = span;
        while at.from_expansion() {
            let data = at.ctxt().outer_expn_data();
            let json = data.macro_def_id.is_some_and(|m| {
                tcx.crate_name(m.krate).as_str() == "serde_json"
                    && tcx.opt_item_name(m).is_some_and(|n| n.as_str() == "json")
            });
            if json {
                if self.in_file(tcx, data.call_site, |f| {
                    ANSWER_SOURCES.iter().any(|s| f.starts_with(s))
                }) {
                    self.wired(
                        tcx,
                        data.call_site,
                        "SRV-181",
                        "an answer built with `json!`; give it a type",
                    );
                }
                return;
            }
            at = data.call_site;
        }
    }

    /// Refuses a destructor in a thread that writes a document: it would run
    /// on the thread's way out (`ARCH-015`).
    pub fn destructor(&mut self, tcx: TyCtxt<'_>, item: &hir::Item<'_>, def: LocalDefId) {
        if self.test {
            return;
        }
        let hir::ItemKind::Impl(imp) = item.kind else {
            return;
        };
        let Some(trait_ref) = imp.of_trait else {
            return;
        };
        let drop = tcx.lang_items().drop_trait();
        if trait_ref.trait_ref.trait_def_id() != drop || drop.is_none() {
            return;
        }
        let span = tcx.def_span(def);
        if self.in_file(tcx, span, exit_writer) {
            self.wired(
                tcx,
                span,
                "ARCH-015",
                "a thread that writes a document runs a destructor on its way out",
            );
        }
    }

    /// Records the panic strategy of a client build on the wasm target: the
    /// judge refuses one that unwinds, and a run that read none (`ARCH-015`).
    pub fn panic_strategy(&mut self, tcx: TyCtxt<'_>) {
        if self.test || !tcx.sess.opts.target_triple.tuple().starts_with("wasm32") {
            return;
        }
        let strategy = match tcx.sess.panic_strategy() {
            rustc_target::spec::PanicStrategy::Abort => "abort",
            _ => "unwind",
        };
        let krate = self.unit.krate.clone();
        self.record(&["fact", "wasm-panic", strategy, &krate]);
    }

    /// Refuses each frame rate a fixture benchmark names (`FIXTURE-024`).
    pub fn bench_rates(&mut self) {
        let Some(bench) = self.bench.take() else {
            return;
        };
        if !bench.fixture {
            return;
        }
        for (at, name) in bench.rates {
            if self.bounded.insert(("FIXTURE-024", at.clone())) {
                self.refuse(
                    "boundary",
                    format!(
                        "{at}: a fixture benchmark reports a frame rate: `{name}` (FIXTURE-024)"
                    ),
                );
            }
        }
    }
}

/// Whether a derive wrote `span`, through any macro call between it and the
/// source.
fn derived(span: Span) -> bool {
    let mut at = span;
    while at.from_expansion() {
        let data = at.ctxt().outer_expn_data();
        if matches!(
            data.kind,
            rustc_span::ExpnKind::Macro(rustc_span::MacroKind::Derive, _)
        ) {
            return true;
        }
        at = data.call_site;
    }
    false
}

/// Whether a file of Trame sits outside the axis module.
fn trame_outside_axis(f: &str) -> bool {
    f.starts_with(TRAME_SOURCES) && f != AXIS_MODULE
}

/// Whether a workspace macro wrote the item whose body holds the expression
/// the context reads: the registry's own `as_str`.
fn macro_owned(cx: &LateContext<'_>) -> bool {
    cx.enclosing_body.is_some_and(|body| {
        let owner = cx.tcx.hir_body_owner_def_id(body);
        cx.tcx.def_span(owner).from_expansion()
    })
}

/// The offset an expression reads, when it is a field or a local of one of
/// the names of [`SCROLL_OFFSETS`].
fn offset_name(expr: &hir::Expr<'_>) -> Option<String> {
    let name = match expr.kind {
        hir::ExprKind::Field(_, field) => field.name.to_string(),
        hir::ExprKind::Path(hir::QPath::Resolved(None, path))
            if matches!(path.res, Res::Local(_)) =>
        {
            path.segments.last()?.ident.name.to_string()
        }
        _ => return None,
    };
    SCROLL_OFFSETS.contains(&name.as_str()).then_some(name)
}

/// Whether `expr` names the variant `variant` of an enum, anywhere inside.
fn names_variant<'tcx>(cx: &LateContext<'tcx>, expr: &'tcx hir::Expr<'tcx>, variant: &str) -> bool {
    struct Seek<'a, 'tcx> {
        cx: &'a LateContext<'tcx>,
        variant: &'a str,
        found: bool,
    }
    impl<'tcx> hir::intravisit::Visitor<'tcx> for Seek<'_, 'tcx> {
        fn visit_expr(&mut self, e: &'tcx hir::Expr<'tcx>) {
            if let hir::ExprKind::Path(ref qpath) = e.kind
                && let Res::Def(DefKind::Variant | DefKind::Ctor(..), def) =
                    self.cx.typeck_results().qpath_res(qpath, e.hir_id)
                && let Some(name) = self.cx.tcx.opt_item_name(def)
                && let Some(parent) = self.cx.tcx.opt_parent(def)
            {
                let own = name.as_str() == self.variant;
                let ctor = self
                    .cx
                    .tcx
                    .opt_item_name(parent)
                    .is_some_and(|n| n.as_str() == self.variant);
                self.found |= own || ctor;
            }
            hir::intravisit::walk_expr(self, e);
        }
    }
    let mut seek = Seek {
        cx,
        variant,
        found: false,
    };
    hir::intravisit::Visitor::visit_expr(&mut seek, expr);
    seek.found
}
