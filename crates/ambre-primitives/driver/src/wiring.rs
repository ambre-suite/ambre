//! The boundaries that read a type, a body or a fact of the whole unit
//! (`ambre-spec-architecture.md` section 2).
//!
//! `bounds.rs` refuses a name by what it resolves to. The checks here need
//! more than the name. They read the type of an expression, the closure a
//! call is handed or the instance of a generic. Some read what every file
//! of a unit holds. Each
//! reads what rustc resolved, so an alias, a re-export, a glob import, an
//! inferred type and the code a macro writes hide nothing.
//!
//! | Check | Rows |
//! | :--- | :--- |
//! | a generator that reaches a CRDT container | `FIXTURE-004` |
//! | a request id the worker allocates | `BUS-004` |
//! | the raw sender of the Logic queue used past the launcher's outbox | `BUS-035`, `BUS-036` |
//! | a passphrase in a field or a key of a request of the worker | `ZK-012` |
//! | a payload, a blob or its bytes in a log line or a span of the relay | `SRV-170` |
//! | a file of the relay that reads a payload's bytes and logs | `SRV-001` |
//! | the frame loop, its relay and the Logic to Sync drain | `ARCH-007` |
//! | the start of the Logic runtime | `ARCH-010` |
//! | a panic hook or a worker error that does not latch the fatal screen | `ARCH-014` |
//!
//! A fact of the whole workspace -- one start, one hook, a relay that
//! drains -- is a `fact` record. The judge counts the facts of every unit,
//! since the host build and the wasm build each compile a part of the
//! launcher.

use std::collections::BTreeMap;

use rustc_hir as hir;
use rustc_hir::def::{DefKind, Res};
use rustc_hir::def_id::{DefId, LocalDefId};
use rustc_lint::LateContext;
use rustc_middle::ty::{self, Ty, TyCtxt, TypeckResults};
use rustc_span::Span;

use crate::Gate;
use crate::bounds::{SHELL_OUTBOX, reached};
use crate::checks::shown_paths;
use crate::place;

/// The fixture modules whose generators drive commands (`FIXTURE-004`).
const FIXTURE_MODULES: &[&str] = &[
    "crates/ambre-trame/src/logic/fixtures.rs",
    "crates/ambre-toile/src/logic/state/fixtures.rs",
];

/// The document of the CRDT engine and its containers (`FIXTURE-004`).
const CONTAINERS: &[&str] = &[
    "LoroDoc",
    "LoroMap",
    "LoroList",
    "LoroMovableList",
    "LoroText",
    "LoroTree",
    "LoroCounter",
];

/// The trait whose impl builds a fixture (`FIXTURE-004`).
const FIXTURE_TRAIT: &str = "FixtureGen";

/// The types of `ambre-core` that allocate a sequence number (`BUS-004`).
const ALLOCATORS: &[&str] = &["Seq", "Issuer", "SharedIssuer"];

/// The macros whose expansion writes a log line or a span (`SRV-170`).
const LOG_MACROS: &[&str] = &[
    "info",
    "warn",
    "error",
    "debug",
    "trace",
    "event",
    "span",
    "info_span",
    "warn_span",
    "error_span",
    "debug_span",
    "trace_span",
    "instrument",
    "log",
    "println",
    "eprintln",
    "print",
    "eprint",
];

/// The words that name a payload, a blob or its bytes in a type (`SRV-170`).
const PAYLOAD_WORDS: &[&str] = &[
    "payload",
    "blob",
    "plaintext",
    "ciphertext",
    "cipher",
    "sealed",
    "body",
];

/// The wrappers a value is read through, peeled before its type is judged.
const WRAPPERS: &[&str] = &["Option", "Box", "Rc", "Arc", "Cow"];

/// How deep a walk follows a binding to the expression that made it.
const DEPTH: usize = 4;

/// What one file of the relay holds, for `SRV-001`.
#[derive(Default)]
pub struct RelayFile {
    /// The first read of a payload's bytes: the line and what it names.
    reads: Option<(String, String)>,
    /// Whether a log line or a span sits in the file.
    logs: bool,
}

/// A frame loop that runs its relay from a parameter of the function around
/// it (`ARCH-007`).
pub struct FrameLoop {
    /// The function that holds the loop.
    function: DefId,
    /// The parameter of that function the relay calls.
    parameter: usize,
    /// Where the loop calls `drive_frame`.
    at: String,
}

/// The type under each reference and each wrapper of [`WRAPPERS`].
fn peeled<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Ty<'tcx> {
    let mut ty = ty;
    loop {
        match ty.kind() {
            ty::Ref(_, inner, _) => ty = *inner,
            ty::Adt(adt, args)
                if WRAPPERS.contains(&tcx.item_name(adt.did()).as_str())
                    && let Some(inner) = args.types().next() =>
            {
                ty = inner;
            }
            _ => return ty,
        }
    }
}

/// The name and the crate of the type at the head of `ty`, if it is an ADT.
fn head(tcx: TyCtxt<'_>, ty: Ty<'_>) -> Option<(String, String)> {
    match ty.kind() {
        ty::Adt(adt, _) => Some((
            tcx.item_name(adt.did()).to_string(),
            tcx.crate_name(adt.did().krate).to_string(),
        )),
        _ => None,
    }
}

/// Whether `ty` is the ADT `name` of the crate `krate` with a first type
/// argument named `arg`: `Sender<LogicCommand>`.
fn instance(tcx: TyCtxt<'_>, ty: Ty<'_>, krate: &str, name: &str, arg: &str) -> bool {
    let ty::Adt(adt, args) = ty.kind() else {
        return false;
    };
    tcx.crate_name(adt.did().krate).as_str() == krate
        && tcx.item_name(adt.did()).as_str() == name
        && args
            .types()
            .next()
            .and_then(|a| head(tcx, a))
            .is_some_and(|(n, _)| n == arg)
}

/// Whether a value of `ty` holds a payload or its bytes. A byte buffer does,
/// and so does a type named for a payload, a blob or a body. A length or an
/// identifier holds neither, whatever it counts (`SRV-170`).
fn payload<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> bool {
    let ty = peeled(tcx, ty);
    let bytes = |t: Ty<'_>| matches!(t.kind(), ty::Uint(ty::UintTy::U8));
    match ty.kind() {
        ty::Slice(inner) | ty::Array(inner, _) => bytes(*inner),
        ty::Adt(adt, args) => {
            let name = tcx.item_name(adt.did()).to_string();
            let krate = tcx.crate_name(adt.did().krate).to_string();
            let lower = name.to_lowercase();
            (name == "Vec" && args.types().next().is_some_and(bytes))
                || (krate == "bytes" && name.starts_with("Bytes"))
                || PAYLOAD_WORDS.iter().any(|w| lower.contains(w))
        }
        _ => false,
    }
}

/// Whether a macro of [`LOG_MACROS`] wrote `span`, followed through every
/// macro call between it and the source.
fn logged(span: Span) -> bool {
    let mut span = span;
    while span.from_expansion() {
        let data = span.ctxt().outer_expn_data();
        // A macro carries the path its call writes: `tracing::info`.
        if let rustc_span::ExpnKind::Macro(_, name) = data.kind
            && LOG_MACROS.contains(&last_segment(name.as_str()))
        {
            return true;
        }
        span = data.call_site;
    }
    false
}

/// The last segment of a macro path: `info` of `tracing::info`.
pub fn last_segment(path: &str) -> &str {
    path.rsplit("::").next().unwrap_or(path)
}

/// Whether a name holds a passphrase word (`ZK-012`).
fn secret_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("passphrase") || lower.contains("password")
}

/// The function `def` resolves to, when an expression calls it by path.
fn path_callee(typeck: &TypeckResults<'_>, expr: &hir::Expr<'_>) -> Option<DefId> {
    let hir::ExprKind::Path(ref qpath) = expr.kind else {
        return None;
    };
    match typeck.qpath_res(qpath, expr.hir_id) {
        Res::Def(DefKind::Fn | DefKind::AssocFn, def) => Some(def),
        _ => None,
    }
}

/// The expression a binding was made from: `let x = <init>;`. Another
/// binding is no closure, so the walk stops there.
fn binding_init<'tcx>(tcx: TyCtxt<'tcx>, id: hir::HirId) -> Option<&'tcx hir::Expr<'tcx>> {
    match tcx.parent_hir_node(id) {
        hir::Node::LetStmt(local) => local.init,
        _ => None,
    }
}

/// Whether `expr` calls, by a resolved path, an item for which `target`
/// holds. The walk enters each closure, and each binding the expression
/// reads, to the depth [`DEPTH`].
fn calls<'tcx>(
    tcx: TyCtxt<'tcx>,
    expr: &'tcx hir::Expr<'tcx>,
    target: &dyn Fn(DefId) -> bool,
) -> bool {
    struct Seek<'a, 'tcx> {
        tcx: TyCtxt<'tcx>,
        target: &'a dyn Fn(DefId) -> bool,
        depth: usize,
        found: bool,
    }
    impl<'tcx> hir::intravisit::Visitor<'tcx> for Seek<'_, 'tcx> {
        fn visit_expr(&mut self, e: &'tcx hir::Expr<'tcx>) {
            match e.kind {
                hir::ExprKind::Path(hir::QPath::Resolved(_, path)) => match path.res {
                    Res::Def(_, def) if (self.target)(def) => self.found = true,
                    Res::Local(id) if self.depth < DEPTH => {
                        if let Some(init) = binding_init(self.tcx, id) {
                            self.depth += 1;
                            self.visit_expr(init);
                            self.depth -= 1;
                        }
                    }
                    _ => {}
                },
                hir::ExprKind::Closure(closure) => {
                    self.visit_expr(self.tcx.hir_body(closure.body).value);
                }
                _ => {}
            }
            hir::intravisit::walk_expr(self, e);
        }
    }
    let mut seek = Seek {
        tcx,
        target,
        depth: 0,
        found: false,
    };
    hir::intravisit::Visitor::visit_expr(&mut seek, expr);
    seek.found
}

/// The closure `expr` is, or the closure the binding it reads was made from.
fn closure_of<'tcx>(
    tcx: TyCtxt<'tcx>,
    expr: &'tcx hir::Expr<'tcx>,
) -> Option<&'tcx hir::Body<'tcx>> {
    let mut expr = expr;
    for _ in 0..DEPTH {
        match expr.kind {
            hir::ExprKind::Closure(closure) => return Some(tcx.hir_body(closure.body)),
            hir::ExprKind::Path(hir::QPath::Resolved(_, path)) => match path.res {
                Res::Local(id) => expr = binding_init(tcx, id)?,
                _ => return None,
            },
            _ => return None,
        }
    }
    None
}

/// Whether the closure body drains the Logic to Sync queue: it calls a
/// `drain` that takes the receiver of `SyncCommand` (`ARCH-007`).
fn drains<'tcx>(
    tcx: TyCtxt<'tcx>,
    typeck: &TypeckResults<'tcx>,
    body: &'tcx hir::Body<'tcx>,
) -> bool {
    struct Seek<'a, 'tcx> {
        tcx: TyCtxt<'tcx>,
        typeck: &'a TypeckResults<'tcx>,
        found: bool,
    }
    impl<'tcx> hir::intravisit::Visitor<'tcx> for Seek<'_, 'tcx> {
        fn visit_expr(&mut self, e: &'tcx hir::Expr<'tcx>) {
            if let hir::ExprKind::MethodCall(seg, _, args, _) = e.kind
                && seg.ident.as_str() == "drain"
                && args.iter().any(|a| {
                    let ty = peeled(self.tcx, self.typeck.expr_ty(a));
                    instance(self.tcx, ty, "ambre_bus", "Receiver", "SyncCommand")
                })
            {
                self.found = true;
            }
            hir::intravisit::walk_expr(self, e);
        }
    }
    let mut seek = Seek {
        tcx,
        typeck,
        found: false,
    };
    hir::intravisit::Visitor::visit_expr(&mut seek, body.value);
    seek.found
}

impl Gate {
    /// The files of the workspace that hold `span`, as a boundary names
    /// them, read once per source file.
    fn files_of(&mut self, tcx: TyCtxt<'_>, span: Span) -> Vec<String> {
        let file = tcx
            .sess
            .source_map()
            .lookup_source_file(span.source_callsite().lo());
        let name = file.name.prefer_local_unconditionally().to_string();
        if let Some(files) = self.file_names.get(&name) {
            return files.clone();
        }
        let path = place::lexical(&self.unit.cwd.join(&name));
        let files = shown_paths(&self.unit.cwd, &path);
        self.file_names.insert(name, files.clone());
        files
    }

    /// Whether `span` sits in a file for which `test` holds.
    pub fn in_file(&mut self, tcx: TyCtxt<'_>, span: Span, test: impl Fn(&str) -> bool) -> bool {
        self.files_of(tcx, span).iter().any(|f| test(f))
    }

    /// Writes one refusal of a boundary, once per rule and line.
    pub fn wired(&mut self, tcx: TyCtxt<'_>, span: Span, rows: &'static str, what: &str) {
        let at = place::position(tcx, &self.unit, span).at();
        if self.bounded.insert((rows, at.clone())) {
            self.refuse("boundary", format!("{at}: {what} ({rows})"));
        }
    }

    /// Writes one fact of the workspace, for the judge to count.
    fn fact(&mut self, tcx: TyCtxt<'_>, name: &str, value: &str, span: Span) {
        let at = place::position(tcx, &self.unit, span).at();
        self.record(&["fact", name, value, &at]);
    }

    /// The checks of one expression.
    pub fn wiring_expr<'tcx>(&mut self, cx: &LateContext<'tcx>, expr: &'tcx hir::Expr<'tcx>) {
        match self.unit.krate.as_str() {
            "ambre-app" => self.launcher_expr(cx, expr),
            "ambre-sync" if !self.test => self.worker_expr(cx, expr),
            "ambre-server" if !self.test => self.relay_expr(cx, expr),
            "ambre-trame" | "ambre-toile" if !self.test => self.generator_expr(cx, expr),
            _ => {}
        }
    }

    /// `FIXTURE-004`: a generator names no CRDT container. A generator is a
    /// free function of a fixture module, or a method of its `FixtureGen`
    /// impl. The type of each expression is read. So a field, an alias or a
    /// helper that hands a container out is refused as the name is.
    fn generator_expr<'tcx>(&mut self, cx: &LateContext<'tcx>, expr: &'tcx hir::Expr<'tcx>) {
        let tcx = cx.tcx;
        let Some(body) = cx.enclosing_body else {
            return;
        };
        let owner = tcx.hir_body_owner_def_id(body);
        let root = tcx.typeck_root_def_id(owner.to_def_id());
        let Some(root) = root.as_local() else {
            return;
        };
        if !generator(tcx, root) || !self.in_file(tcx, expr.span, |f| FIXTURE_MODULES.contains(&f))
        {
            return;
        }
        let ty = cx.typeck_results().expr_ty(expr);
        let container = ty.walk().filter_map(|a| a.as_type()).find_map(|t| {
            head(tcx, t).filter(|(name, krate)| {
                krate.starts_with("loro") && CONTAINERS.contains(&name.as_str())
            })
        });
        if let Some((name, _)) = container {
            self.wired(
                tcx,
                expr.span,
                "FIXTURE-004",
                &format!("a generator reaches the CRDT: `{name}`"),
            );
        }
    }

    /// `BUS-004`, `ZK-012`: the worker allocates no request id, and spells
    /// no passphrase key.
    fn worker_expr<'tcx>(&mut self, cx: &LateContext<'tcx>, expr: &'tcx hir::Expr<'tcx>) {
        let tcx = cx.tcx;
        if let hir::ExprKind::Lit(lit) = expr.kind
            && let rustc_ast::LitKind::Str(text, _) = lit.node
            && secret_name(text.as_str())
            && text
                .as_str()
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            self.wired(
                tcx,
                lit.span,
                "ZK-012",
                &format!("a request key names a passphrase: `{text}`"),
            );
        }
        let typeck = cx.typeck_results();
        let callee = match expr.kind {
            hir::ExprKind::MethodCall(..) => typeck.type_dependent_def_id(expr.hir_id),
            hir::ExprKind::Path(ref qpath) => match typeck.qpath_res(qpath, expr.hir_id) {
                Res::Def(DefKind::Fn | DefKind::AssocFn | DefKind::AssocConst, def) => Some(def),
                _ => None,
            },
            _ => None,
        };
        let Some(callee) = callee else {
            return;
        };
        let reach = reached(tcx, callee);
        if reach.krate == "ambre_bus" && reach.name == "next_request" {
            self.wired(
                tcx,
                expr.span,
                "BUS-004",
                "the worker allocates a request id: `next_request`",
            );
            return;
        }
        let Some(parent) = tcx.opt_parent(callee) else {
            return;
        };
        let inherent = matches!(tcx.def_kind(parent), DefKind::Impl { of_trait: false });
        let allocator = reach.krate == "ambre_core"
            && reach
                .owner
                .as_deref()
                .is_some_and(|o| ALLOCATORS.contains(&o));
        if !inherent || !allocator {
            return;
        }
        let args = typeck.node_args(expr.hir_id);
        let output = match tcx.def_kind(callee) {
            DefKind::AssocConst => tcx
                .type_of(callee)
                .instantiate(tcx, args)
                .skip_normalization(),
            _ => tcx
                .fn_sig(callee)
                .instantiate(tcx, args)
                .skip_normalization()
                .skip_binder()
                .output(),
        };
        let request = output.walk().filter_map(|a| a.as_type()).any(|t| {
            head(tcx, t).is_some_and(|(name, krate)| name == "Request" && krate == "ambre_bus")
        });
        if request {
            self.wired(
                tcx,
                expr.span,
                "BUS-004",
                &format!(
                    "the worker allocates a request id: `{}::{}`",
                    reach.owner.unwrap_or_default(),
                    reach.name
                ),
            );
        }
    }

    /// `ZK-012`: no serialized type of the worker holds a passphrase field
    /// or a secret, whatever the alias or the wrapper around it.
    pub fn passphrase_fields(&mut self, tcx: TyCtxt<'_>, def: LocalDefId) {
        if self.test
            || self.unit.krate != "ambre-sync"
            || !self.serializes.contains(&def.to_def_id())
        {
            return;
        }
        let adt = tcx.adt_def(def);
        for field in adt.all_fields() {
            let name = field.name.to_string();
            let ty = tcx
                .type_of(field.did)
                .instantiate_identity()
                .skip_normalization();
            let secret = ty.walk().filter_map(|a| a.as_type()).find_map(|t| {
                head(tcx, t)
                    .filter(|(n, k)| k == "ambre_core" && (n == "Confidential" || n == "Secret"))
            });
            let span = tcx.def_span(field.did);
            if secret_name(&name) {
                self.wired(
                    tcx,
                    span,
                    "ZK-012",
                    &format!("a serialized field names a passphrase: `{name}`"),
                );
            } else if let Some((secret, _)) = secret {
                self.wired(
                    tcx,
                    span,
                    "ZK-012",
                    &format!("a serialized field holds a secret: `{name}: {secret}`"),
                );
            }
        }
    }

    /// `SRV-170`, `SRV-001`: no log line or span of the relay reads a payload,
    /// and a file that reads a payload's bytes logs nothing.
    fn relay_expr<'tcx>(&mut self, cx: &LateContext<'tcx>, expr: &'tcx hir::Expr<'tcx>) {
        let tcx = cx.tcx;
        let typeck = cx.typeck_results();
        let in_log = logged(expr.span);
        if in_log {
            let file = self.files_of(tcx, expr.span).concat();
            self.relay_files.entry(file).or_default().logs = true;
        }
        match expr.kind {
            // A value a log macro formats or records reaches it as the
            // argument of one of these calls. Its length is a `usize`, so a
            // length passes.
            hir::ExprKind::Call(func, args) if in_log => {
                let Some(callee) = path_callee(typeck, func) else {
                    return;
                };
                let reach = reached(tcx, callee);
                let formats = (reach.krate == "core"
                    && reach.owner.as_deref() == Some("Argument")
                    && reach.name.starts_with("new"))
                    || (reach.krate.starts_with("tracing")
                        && ["debug", "display"].contains(&reach.name.as_str()));
                if !formats {
                    return;
                }
                for arg in args {
                    let ty = typeck.expr_ty(arg);
                    if payload(tcx, ty) {
                        self.wired(
                            tcx,
                            arg.span,
                            "SRV-170",
                            &format!("a log line or span names a payload: `{ty}`"),
                        );
                    }
                }
            }
            hir::ExprKind::MethodCall(seg, ..) if seg.ident.as_str() == "expose_bytes" => {
                self.payload_read(tcx, expr.span, "expose_bytes");
            }
            _ => {}
        }
    }

    /// Records a read of a payload's bytes in the file of `span`
    /// (`SRV-001`).
    fn payload_read(&mut self, tcx: TyCtxt<'_>, span: Span, what: &str) {
        let file = self.files_of(tcx, span).concat();
        let at = place::position(tcx, &self.unit, span).at();
        let entry = self.relay_files.entry(file).or_default();
        if entry.reads.is_none() {
            entry.reads = Some((at, what.to_owned()));
        }
    }

    /// `SRV-001`: a function of the relay that takes a ciphertext whole, bare
    /// or as the role of `Payload<Ciphertext>`, reads its bytes.
    pub fn relay_params(&mut self, tcx: TyCtxt<'_>, def: LocalDefId) {
        if self.test || self.unit.krate != "ambre-server" {
            return;
        }
        let sig = tcx
            .fn_sig(def)
            .instantiate_identity()
            .skip_normalization()
            .skip_binder();
        for input in sig.inputs() {
            let ty::Ref(_, inner, _) = input.kind() else {
                continue;
            };
            let Some((name, _)) = head(tcx, *inner) else {
                continue;
            };
            let role = match inner.kind() {
                ty::Adt(_, args) => args.types().next().and_then(|a| head(tcx, a)),
                _ => None,
            };
            let whole = name == "Ciphertext"
                || (name == "Payload" && role.is_some_and(|(r, _)| r == "Ciphertext"));
            if whole {
                self.payload_read(tcx, tcx.def_span(def), "a ciphertext taken whole");
            }
        }
    }

    /// `BUS-035`, `BUS-036`, `ARCH-007`, `ARCH-010`, `ARCH-014`: the
    /// launcher's wiring.
    fn launcher_expr<'tcx>(&mut self, cx: &LateContext<'tcx>, expr: &'tcx hir::Expr<'tcx>) {
        let tcx = cx.tcx;
        let typeck = cx.typeck_results();
        if let hir::ExprKind::Path(ref qpath) = expr.kind
            && let Res::Def(DefKind::Fn, def) = typeck.qpath_res(qpath, expr.hir_id)
            && tcx.crate_name(def.krate).as_str() == "ambre_logic"
            && tcx.item_name(def).as_str() == "run"
            && tcx.opt_parent(def).is_some_and(|p| p.is_crate_root())
        {
            // A start in a test counts too: one engine per session.
            self.fact(tcx, "logic-start", "-", expr.span);
        }
        if self.test {
            return;
        }
        self.outbox_expr(cx, expr);
        match expr.kind {
            hir::ExprKind::Call(func, args) => {
                let Some(callee) = path_callee(typeck, func) else {
                    return;
                };
                let reach = reached(tcx, callee);
                if reach.krate == "std" && reach.name == "set_hook" {
                    let latched = args.iter().any(|a| calls(tcx, a, &|d| fatal(tcx, d)));
                    self.latch(tcx, "panic-hook", latched, expr.span);
                }
                if reach.krate == "ambre_canvas" && reach.name == "drive_frame" {
                    self.frame_loop(cx, expr, args);
                }
            }
            hir::ExprKind::MethodCall(seg, recv, args, _)
                if seg.ident.as_str() == "set_onerror" =>
            {
                let worker = head(tcx, peeled(tcx, typeck.expr_ty(recv)))
                    .is_some_and(|(n, k)| n == "Worker" && k == "web_sys");
                if worker {
                    let latched = args.iter().any(|a| calls(tcx, a, &|d| fatal(tcx, d)));
                    self.latch(tcx, "worker-error", latched, expr.span);
                }
            }
            _ => {}
        }
    }

    /// Records a failure path and whether it latches the fatal screen, and
    /// refuses one that does not (`ARCH-014`).
    fn latch(&mut self, tcx: TyCtxt<'_>, path: &str, latched: bool, span: Span) {
        self.fact(tcx, path, if latched { "latched" } else { "open" }, span);
        if !latched {
            self.wired(
                tcx,
                span,
                "ARCH-014",
                &format!("a {path} waits on a queue: it never calls `ambre_canvas::signal_fatal`"),
            );
        }
    }

    /// `ARCH-007`: a frame loop runs a relay the function around it takes as
    /// a parameter. The call sites of that function are read once the unit
    /// is read, in [`Gate::frame_relays`].
    fn frame_loop<'tcx>(
        &mut self,
        cx: &LateContext<'tcx>,
        expr: &'tcx hir::Expr<'tcx>,
        args: &'tcx [hir::Expr<'tcx>],
    ) {
        let tcx = cx.tcx;
        let typeck = cx.typeck_results();
        let at = place::position(tcx, &self.unit, expr.span).at();
        let function = cx
            .enclosing_body
            .map(|b| tcx.typeck_root_def_id(tcx.hir_body_owner_def_id(b).to_def_id()));
        let relay = args.get(1).and_then(|r| closure_of(tcx, r));
        let parameter = function.zip(relay).and_then(|(function, relay)| {
            let inputs = tcx
                .fn_sig(function)
                .instantiate_identity()
                .skip_normalization()
                .skip_binder();
            let params: Vec<u32> = called_params(typeck, relay);
            inputs
                .inputs()
                .iter()
                .position(|i| matches!(i.kind(), ty::Param(p) if params.contains(&p.index)))
                .map(|index| (function, index))
        });
        match parameter {
            Some((function, parameter)) => self.frame_loops.push(FrameLoop {
                function,
                parameter,
                at,
            }),
            None => self.wired(
                tcx,
                expr.span,
                "ARCH-007",
                "the frame loop runs no relay the launcher passes in",
            ),
        }
    }

    /// `BUS-035`, `BUS-036`: the raw end of the UI to Logic queue moves into
    /// the outbox where it is made. The relay flushes the outbox, and tells
    /// the user of its refusal.
    fn outbox_expr<'tcx>(&mut self, cx: &LateContext<'tcx>, expr: &'tcx hir::Expr<'tcx>) {
        let tcx = cx.tcx;
        let typeck = cx.typeck_results();
        let ty = peeled(tcx, typeck.expr_ty(expr));
        if instance(tcx, ty, "ambre_bus", "Sender", "LogicCommand")
            && !self.in_file(tcx, expr.span, |f| f == SHELL_OUTBOX)
        {
            let moved = matches!(expr.kind, hir::ExprKind::Path(_))
                && matches!(
                    tcx.parent_hir_node(expr.hir_id),
                    hir::Node::Expr(hir::Expr { kind: hir::ExprKind::Call(func, _), .. })
                        if path_callee(typeck, func).is_some_and(|d| outbox_new(tcx, d))
                );
            if moved {
                self.fact(tcx, "outbox-move", "-", expr.span);
            } else {
                self.wired(
                    tcx,
                    expr.span,
                    "BUS-035",
                    "the raw UI to Logic sender is used past its outbox",
                );
            }
        }
        match expr.kind {
            hir::ExprKind::Call(func, _) => {
                if let Some(callee) = path_callee(typeck, func)
                    && reached(tcx, callee).name == "to_worker"
                    && tcx.crate_name(callee.krate).as_str() == "ambre_bus"
                    && typeck
                        .node_args(func.hir_id)
                        .types()
                        .next()
                        .and_then(|a| head(tcx, a))
                        .is_some_and(|(n, _)| n == "LogicCommand")
                {
                    self.fact(tcx, "ui-queue", "-", expr.span);
                }
            }
            hir::ExprKind::MethodCall(seg, recv, ..) => {
                let name = seg.ident.as_str();
                let outbox = head(tcx, peeled(tcx, typeck.expr_ty(recv)))
                    .is_some_and(|(n, k)| n == "ShellOutbox" && k == "ambre_app");
                if outbox && (name == "flush" || name == "take_refusal") {
                    self.fact(tcx, &format!("outbox-{name}"), "-", expr.span);
                }
            }
            _ => {}
        }
    }

    /// Once the unit is read, reads each call site of a frame loop's
    /// function. It refuses one whose relay does not drain the Logic to Sync
    /// queue, and records each one that does (`ARCH-007`).
    pub fn frame_relays(&mut self, tcx: TyCtxt<'_>) {
        let loops = std::mem::take(&mut self.frame_loops);
        if loops.is_empty() {
            return;
        }
        let mut sites: BTreeMap<String, (bool, Span)> = BTreeMap::new();
        for owner in tcx.hir_body_owners() {
            if tcx.is_typeck_child(owner.to_def_id()) {
                continue;
            }
            let typeck = tcx.typeck(owner);
            let body = tcx.hir_body_owned_by(owner);
            let mut found = Vec::new();
            collect_calls(tcx, typeck, body.value, &loops, &mut found);
            for (span, drains) in found {
                let at = place::position(tcx, &self.unit, span).at();
                sites.insert(at, (drains, span));
            }
        }
        if sites.is_empty() {
            for frame in &loops {
                self.refuse(
                    "boundary",
                    format!(
                        "{}: no call passes the frame loop a relay (ARCH-007)",
                        frame.at
                    ),
                );
            }
        }
        for (drains, span) in sites.into_values() {
            if drains {
                self.fact(tcx, "frame-relay", "drains", span);
            } else {
                self.wired(
                    tcx,
                    span,
                    "ARCH-007",
                    "the relay does not drain the Logic to Sync queue",
                );
            }
        }
    }

    /// Once the unit is read: refuses each file of the relay that reads a
    /// payload's bytes and holds a log line (`SRV-001`).
    pub fn relay_logs(&mut self) {
        let files = std::mem::take(&mut self.relay_files);
        for file in files.into_values() {
            if let (Some((at, what)), true) = (file.reads, file.logs) {
                self.refuse(
                    "boundary",
                    format!("{at}: a file that logs reads a payload's bytes: `{what}` (SRV-001)"),
                );
            }
        }
    }
}

/// The type parameters whose value a closure body calls: `relay()`, or
/// `(relay.borrow_mut())()`.
fn called_params<'tcx>(typeck: &TypeckResults<'tcx>, body: &hir::Body<'tcx>) -> Vec<u32> {
    struct Seek<'a, 'tcx> {
        typeck: &'a TypeckResults<'tcx>,
        found: Vec<u32>,
    }
    impl<'tcx> hir::intravisit::Visitor<'tcx> for Seek<'_, 'tcx> {
        fn visit_expr(&mut self, e: &'tcx hir::Expr<'tcx>) {
            if let hir::ExprKind::Call(func, _) = e.kind {
                let ty = self.typeck.expr_ty_adjusted(func);
                for t in ty.walk().filter_map(|a| a.as_type()) {
                    if let ty::Param(p) = t.kind() {
                        self.found.push(p.index);
                    }
                }
            }
            hir::intravisit::walk_expr(self, e);
        }
    }
    let mut seek = Seek {
        typeck,
        found: Vec::new(),
    };
    hir::intravisit::Visitor::visit_expr(&mut seek, body.value);
    seek.found
}

/// Each call in `expr` of a frame loop's function, with whether the relay
/// it passes drains the Logic to Sync queue. The walk enters each closure.
fn collect_calls<'tcx>(
    tcx: TyCtxt<'tcx>,
    typeck: &'tcx TypeckResults<'tcx>,
    expr: &'tcx hir::Expr<'tcx>,
    loops: &[FrameLoop],
    found: &mut Vec<(Span, bool)>,
) {
    struct Seek<'a, 'tcx> {
        tcx: TyCtxt<'tcx>,
        typeck: &'tcx TypeckResults<'tcx>,
        loops: &'a [FrameLoop],
        found: &'a mut Vec<(Span, bool)>,
    }
    impl<'tcx> hir::intravisit::Visitor<'tcx> for Seek<'_, 'tcx> {
        fn visit_expr(&mut self, e: &'tcx hir::Expr<'tcx>) {
            match e.kind {
                hir::ExprKind::Call(func, args) => {
                    if let Some(callee) = path_callee(self.typeck, func) {
                        for frame in self.loops.iter().filter(|f| f.function == callee) {
                            let drains = args
                                .get(frame.parameter)
                                .and_then(|a| closure_of(self.tcx, a))
                                .is_some_and(|body| drains(self.tcx, self.typeck, body));
                            self.found.push((e.span, drains));
                        }
                    }
                }
                hir::ExprKind::Closure(closure) => {
                    self.visit_expr(self.tcx.hir_body(closure.body).value);
                }
                _ => {}
            }
            hir::intravisit::walk_expr(self, e);
        }
    }
    let mut seek = Seek {
        tcx,
        typeck,
        loops,
        found,
    };
    hir::intravisit::Visitor::visit_expr(&mut seek, expr);
}

/// Whether a function is a generator: a free function, or one nested in a
/// free function, or a method of an impl of [`FIXTURE_TRAIT`]
/// (`FIXTURE-004`).
fn generator(tcx: TyCtxt<'_>, def: LocalDefId) -> bool {
    let mut at = def;
    loop {
        match tcx.def_kind(at) {
            DefKind::Fn => {
                let parent = tcx.local_parent(at);
                if matches!(tcx.def_kind(parent), DefKind::Mod) {
                    return true;
                }
                at = parent;
            }
            DefKind::AssocFn => {
                let parent = tcx.local_parent(at);
                return tcx.impl_opt_trait_ref(parent).is_some_and(|t| {
                    tcx.item_name(t.skip_binder().def_id).as_str() == FIXTURE_TRAIT
                });
            }
            _ => return false,
        }
    }
}

/// Whether `def` is `ambre_canvas::signal_fatal` (`ARCH-014`).
fn fatal(tcx: TyCtxt<'_>, def: DefId) -> bool {
    tcx.crate_name(def.krate).as_str() == "ambre_canvas"
        && tcx
            .opt_item_name(def)
            .is_some_and(|n| n.as_str() == "signal_fatal")
}

/// Whether `def` is `ShellOutbox::new` (`BUS-035`).
fn outbox_new(tcx: TyCtxt<'_>, def: DefId) -> bool {
    let reach = reached(tcx, def);
    reach.name == "new" && reach.owner.as_deref() == Some("ShellOutbox")
}
