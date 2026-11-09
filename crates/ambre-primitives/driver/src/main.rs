//! The rustc driver of the primitive gate (`ambre-spec-testing.md` section 2).
//!
//! Cargo runs it in place of rustc as `RUSTC_WORKSPACE_WRAPPER`. So it
//! compiles each unit of the workspace as rustc does, and reads the result
//! with types. It writes sites, never verdicts: `ambre-primitives` reads them
//! and judges them against the allowlists.
//!
//! Each unit writes one file, `$AMBRE_GATE_OUT/<unit>.tsv`, one record per
//! line, its first field naming the record. A diagnostic would not survive the
//! `-A warnings` of the wasm `rustflags`, and a file per unit outlives the
//! compile. The file of a unit that compiles test code holds the records that
//! read test code alone: the `bool` literals, the aliases and the files.
//!
//! | Record | Row | What it holds |
//! | :--- | :--- | :--- |
//! | `site` | `ARCH-033` to `ARCH-040` | a site whose type holds a primitive, with the facts the judge reads |
//! | `repr` | `ARCH-036` | an associated type a generic newtype holds as its one field |
//! | `lit` | `ARCH-040` | a `bool` literal a call passes, and the parameter it reaches |
//! | `newtype` | `ARCH-062` | a struct of one field over a primitive of the census list |
//! | `alias` | `ARCH-066` | an alias whose target is an instance of a generic struct of `ambre-core` |
//! | `generic` | `ARCH-066` | a generic struct of `ambre-core` |
//! | `unwrap` | `ARCH-073` | a newtype unwrapped and handed to a workspace callee |
//! | `refuse` | `ARCH-037`, `ARCH-045` to `ARCH-049`, the private field, the crate boundaries | a refusal the code alone settles |
//! | `decl` | `ARCH-006` | a type, an alias or a trait of the protocol, the worker or the relay |
//! | `fact` | `ARCH-007`, `ARCH-010`, `ARCH-014`, `BUS-035`, `BUS-036` | a fact of the launcher's wiring the judge counts over every unit |
//! | `file` | `ARCH-033` | a file the compile read |

#![feature(rustc_private)]

extern crate rustc_ast;
extern crate rustc_data_structures;
extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_lint;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;

mod bounds;
mod checks;
mod literals;
mod place;
mod sites;
mod types;
mod unwrap;
mod wiring;

use std::collections::{BTreeSet, HashMap};

use rustc_hir as hir;
use rustc_hir::def::{DefKind, Res};
use rustc_hir::def_id::DefId;
use rustc_lint::{LateContext, LateLintPass};
use rustc_middle::ty::TyCtxt;

use place::{Origin, Unit};
use sites::{Kind, Privacy};

rustc_session::declare_lint! {
    /// The sites of the primitive gate, read with types.
    pub AMBRE_PRIMITIVE_GATE,
    // rustc skips a late pass whose lints are all allowed, and the wasm
    // `rustflags` allow every warning. The pass emits no diagnostic, so this
    // level only keeps it running.
    Deny,
    "a site the primitive gate reads"
}

/// The environment variable that names the directory of the records.
const OUT: &str = "AMBRE_GATE_OUT";

/// The traits whose derive puts a type on a wire or in a store (`ARCH-038`).
const SERIALISED: &[&str] = &["Serialize", "Deserialize", "FromRow"];

/// The pass: one per unit.
pub struct Gate {
    unit: Unit,
    /// Whether the unit compiles test code.
    test: bool,
    /// Each record, once.
    records: BTreeSet<String>,
    /// The types that derive a trait of [`SERIALISED`].
    serialised: rustc_data_structures::fx::FxHashSet<DefId>,
    /// The `fn` a Leptos `#[component]` writes for each component, by the
    /// snake-case name it gives it.
    components: HashMap<String, String>,
    /// Each boundary refused on a line, so a line is refused once per rule.
    bounded: BTreeSet<(&'static str, String)>,
    /// The types that derive `Serialize` alone (`ZK-012`).
    serializes: rustc_data_structures::fx::FxHashSet<DefId>,
    /// The files of the workspace each source file is, as a boundary names
    /// them, read once per file.
    file_names: HashMap<String, Vec<String>>,
    /// What each file of the relay reads and logs (`SRV-001`).
    relay_files: std::collections::BTreeMap<String, wiring::RelayFile>,
    /// Each frame loop of the unit, its call sites read at the end
    /// (`ARCH-007`).
    frame_loops: Vec<wiring::FrameLoop>,
}

rustc_session::impl_lint_pass!(Gate => [AMBRE_PRIMITIVE_GATE]);

impl Gate {
    fn new(test: bool) -> Self {
        Self {
            unit: Unit::current(),
            test,
            records: BTreeSet::new(),
            serialised: Default::default(),
            components: HashMap::new(),
            bounded: BTreeSet::new(),
            serializes: Default::default(),
            file_names: HashMap::new(),
            relay_files: std::collections::BTreeMap::new(),
            frame_loops: Vec::new(),
        }
    }

    /// Writes one record, its fields joined by tabs.
    pub fn record(&mut self, fields: &[&str]) {
        let line: Vec<String> = fields
            .iter()
            .map(|f| f.replace(['\t', '\n'], " "))
            .collect();
        self.records.insert(line.join("\t"));
    }

    /// Writes one refusal.
    pub fn refuse(&mut self, check: &str, message: String) {
        self.record(&["refuse", check, &message]);
    }

    /// Whether a macro of a foreign crate wrote the item `def`: its name is
    /// no token of the source nor of a workspace macro. What such a macro
    /// writes takes its types from that crate, so no site is read there.
    fn foreign_item(tcx: TyCtxt<'_>, def: rustc_hir::def_id::LocalDefId) -> bool {
        let module = tcx.parent_module_from_def_id(def).to_local_def_id();
        let mut at = def;
        while at != module {
            // A name tells who wrote an item; an impl has none, so its whole
            // span does.
            let span = tcx.def_ident_span(at).unwrap_or_else(|| tcx.def_span(at));
            let foreign = matches!(
                place::origin(tcx, span),
                Origin::Macro {
                    workspace: false,
                    ..
                }
            );
            if foreign {
                return true;
            }
            at = tcx.local_parent(at);
        }
        false
    }

    /// The written privacy of a signature or a constant of an impl item.
    fn impl_privacy(tcx: TyCtxt<'_>, item: &hir::ImplItem<'_>) -> Privacy {
        let parent = tcx.local_parent(item.owner_id.def_id);
        if matches!(tcx.def_kind(parent), DefKind::Impl { of_trait: true }) {
            return Privacy::TraitImpl;
        }
        match item.vis_span().map_or(place::Written::Private, |span| {
            place::written_vis(tcx, span)
        }) {
            place::Written::Private => Privacy::Private,
            place::Written::Restricted => Privacy::Restricted,
            place::Written::Public => Privacy::Public,
        }
    }
}

fn privacy(written: place::Written) -> Privacy {
    match written {
        place::Written::Private => Privacy::Private,
        place::Written::Restricted => Privacy::Restricted,
        place::Written::Public => Privacy::Public,
    }
}

/// The snake-case name a Leptos `#[component]` gives the `fn` that holds a
/// component's body: `__component_` and the name in snake case.
fn snake(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

impl<'tcx> LateLintPass<'tcx> for Gate {
    fn check_crate(&mut self, cx: &LateContext<'tcx>) {
        let tcx = cx.tcx;
        for def in tcx.hir_crate_items(()).definitions() {
            match tcx.def_kind(def) {
                DefKind::Impl { of_trait: true }
                    if tcx.is_automatically_derived(def.to_def_id()) =>
                {
                    let Some(trait_ref) = tcx.impl_opt_trait_ref(def) else {
                        continue;
                    };
                    let name = tcx.item_name(trait_ref.skip_binder().def_id);
                    if SERIALISED.contains(&name.as_str())
                        && let rustc_middle::ty::Adt(adt, _) =
                            tcx.type_of(def).instantiate_identity().kind()
                    {
                        self.serialised.insert(adt.did());
                        if name.as_str() == "Serialize" {
                            self.serializes.insert(adt.did());
                        }
                    }
                }
                DefKind::Fn => {
                    // A component `fn` takes its props struct alone.
                    let name = tcx.item_name(def.to_def_id()).to_string();
                    let sig = tcx.fn_sig(def).instantiate_identity().skip_binder();
                    if let [input] = sig.inputs()
                        && let rustc_middle::ty::Adt(adt, _) = input.kind()
                        && tcx.item_name(adt.did()).as_str() == format!("{name}Props")
                    {
                        self.components
                            .insert(format!("__component_{}", snake(&name)), name);
                    }
                }
                _ => {}
            }
        }
    }

    fn check_item(&mut self, cx: &LateContext<'tcx>, item: &'tcx hir::Item<'tcx>) {
        let tcx = cx.tcx;
        let def = item.owner_id.def_id;
        if let hir::ItemKind::TyAlias(..) = item.kind {
            // An alias of a family is refused in test code too (`ARCH-066`).
            self.alias_check(tcx, item, def);
        }
        if let hir::ItemKind::Struct(..) = item.kind {
            self.core_generic(tcx, def);
        }
        if let hir::ItemKind::Impl(_) = item.kind {
            self.deref_check(tcx, item, def);
        }
        if let hir::ItemKind::Fn { ident, .. } = item.kind {
            self.name_check(tcx, ident.as_str(), None, ident.span);
        }
        if !self.test {
            self.declaration(tcx, item, def);
        }
        match item.kind {
            hir::ItemKind::Struct(..) | hir::ItemKind::Enum(..) => {
                self.passphrase_fields(tcx, def);
            }
            hir::ItemKind::Fn { .. } => self.relay_params(tcx, def),
            _ => {}
        }
        if !self.test
            && let hir::ItemKind::Fn { ident, .. } = item.kind
            && let Some(component) = self.components.get(ident.as_str()).cloned()
        {
            self.component_sites(tcx, def, &component, item);
            return;
        }
        if self.test || Self::foreign_item(tcx, def) {
            return;
        }
        let vis = place::written_vis(tcx, item.vis_span);
        match item.kind {
            hir::ItemKind::Struct(..) => {
                self.adt_sites(tcx, def, vis);
                self.struct_checks(tcx, item, def);
                self.representation(tcx, def);
            }
            hir::ItemKind::Enum(..) | hir::ItemKind::Union(..) => self.adt_sites(tcx, def, vis),
            hir::ItemKind::Fn { .. } => self.fn_sites_as(tcx, def, privacy(vis), None),
            hir::ItemKind::Const(_, _, ty, _) => {
                self.item_site(tcx, def, Kind::Const, ty.span, privacy(vis));
            }
            hir::ItemKind::Static(_, _, ty, _) => {
                self.item_site(tcx, def, Kind::Static, ty.span, privacy(vis));
            }
            hir::ItemKind::TyAlias(_, _, ty) => {
                self.item_site(tcx, def, Kind::Alias, ty.span, privacy(vis));
            }
            _ => {}
        }
    }

    fn check_impl_item(&mut self, cx: &LateContext<'tcx>, item: &'tcx hir::ImplItem<'tcx>) {
        let tcx = cx.tcx;
        let def = item.owner_id.def_id;
        if let hir::ImplItemKind::Fn(..) = item.kind {
            self.name_check(tcx, item.ident.as_str(), None, item.ident.span);
            self.relay_params(tcx, def);
        }
        if self.test || Self::foreign_item(tcx, def) {
            return;
        }
        let privacy = Self::impl_privacy(tcx, item);
        match item.kind {
            hir::ImplItemKind::Fn(..) => self.fn_sites_as(tcx, def, privacy, None),
            hir::ImplItemKind::Const(ty, _) => {
                self.item_site(tcx, def, Kind::Const, ty.span, privacy);
            }
            hir::ImplItemKind::Type(ty) => {
                self.item_site(tcx, def, Kind::Assoc, ty.span, Privacy::TraitImpl);
            }
        }
    }

    fn check_trait_item(&mut self, cx: &LateContext<'tcx>, item: &'tcx hir::TraitItem<'tcx>) {
        let tcx = cx.tcx;
        let def = item.owner_id.def_id;
        if let hir::TraitItemKind::Fn(..) = item.kind {
            self.name_check(tcx, item.ident.as_str(), None, item.ident.span);
        }
        if self.test || Self::foreign_item(tcx, def) {
            return;
        }
        if let hir::TraitItemKind::Fn(..) = item.kind {
            let trait_def = tcx.local_parent(def);
            let vis = match tcx.hir_node_by_def_id(trait_def) {
                hir::Node::Item(t) => place::written_vis(tcx, t.vis_span),
                _ => place::Written::Private,
            };
            self.fn_sites_as(tcx, def, privacy(vis), None);
        }
    }

    fn check_path(&mut self, cx: &LateContext<'tcx>, path: &hir::Path<'tcx>, _: hir::HirId) {
        let def = match path.res {
            Res::Def(_, def) => Some(def),
            _ => None,
        };
        for segment in path.segments {
            if let Res::Def(_, reached) = segment.res {
                self.bound_check(cx.tcx, reached, segment.ident.span);
            }
        }
        if let Some(reached) = def {
            self.bound_check(cx.tcx, reached, path.span);
        }
        if self.test {
            return;
        }
        for segment in path.segments {
            let resolved = match segment.res {
                Res::Def(_, def) => Some(def),
                _ => def,
            };
            self.name_check(cx.tcx, segment.ident.as_str(), resolved, segment.ident.span);
        }
    }

    fn check_expr(&mut self, cx: &LateContext<'tcx>, expr: &'tcx hir::Expr<'tcx>) {
        let tcx = cx.tcx;
        self.wiring_expr(cx, expr);
        match expr.kind {
            hir::ExprKind::MethodCall(seg, _, args, _) => {
                let callee = cx.typeck_results().type_dependent_def_id(expr.hir_id);
                if let Some(callee) = callee {
                    self.bound_check(tcx, callee, seg.ident.span);
                }
                if !self.test {
                    self.name_check(tcx, seg.ident.as_str(), callee, seg.ident.span);
                }
                if let Some(callee) = callee {
                    self.literals(cx, expr, callee, args, true);
                }
            }
            hir::ExprKind::Call(func, args) => {
                if let hir::ExprKind::Path(ref qpath) = func.kind
                    && let Res::Def(DefKind::Fn | DefKind::AssocFn, callee) =
                        cx.typeck_results().qpath_res(qpath, func.hir_id)
                {
                    self.literals(cx, func, callee, args, false);
                }
            }
            hir::ExprKind::Path(hir::QPath::TypeRelative(_, seg)) => {
                let callee = cx.typeck_results().type_dependent_def_id(expr.hir_id);
                if let Some(callee) = callee {
                    self.bound_check(tcx, callee, seg.ident.span);
                }
                if !self.test {
                    self.name_check(tcx, seg.ident.as_str(), callee, seg.ident.span);
                }
            }
            hir::ExprKind::Lit(lit) => {
                self.embedded_literal(tcx, lit.span);
                if let rustc_ast::LitKind::Str(text, _) = lit.node {
                    self.density_literal(tcx, text.as_str(), lit.span);
                }
            }
            _ => {}
        }
    }

    fn check_body(&mut self, cx: &LateContext<'tcx>, body: &hir::Body<'tcx>) {
        if self.test {
            return;
        }
        let tcx = cx.tcx;
        let owner = tcx.hir_body_owner_def_id(body.id());
        let mut found = Vec::new();
        let mut scan = unwrap::BodyScan {
            cx,
            typeck: tcx.typeck_body(body.id()),
            owner,
            tainted: HashMap::new(),
            unit: &self.unit,
            sites: &mut found,
        };
        let value: &'tcx hir::Expr<'tcx> = tcx.hir_body(body.id()).value;
        hir::intravisit::Visitor::visit_expr(&mut scan, value);
        for (key, at) in found {
            // Two sites of one key on one line count twice: the record
            // keeps the column.
            self.record(&["unwrap", &key, &at]);
        }
    }

    fn check_crate_post(&mut self, cx: &LateContext<'tcx>) {
        let tcx = cx.tcx;
        self.frame_relays(tcx);
        self.relay_logs();
        for file in tcx.sess.source_map().files().iter() {
            if let rustc_span::FileName::Real(ref real) = file.name {
                let Some(name) = real.local_path() else {
                    continue;
                };
                let path = place::lexical(&self.unit.cwd.join(name));
                let path = path.to_string_lossy().into_owned();
                self.record(&["file", &path]);
            }
        }
        let Ok(dir) = std::env::var(OUT) else {
            return;
        };
        let krate = tcx.crate_name(rustc_span::def_id::LOCAL_CRATE);
        // Named as cargo names the unit's output, so the gate finds the
        // record of each unit cargo reports.
        let unit = format!("{krate}{}", tcx.sess.opts.cg.extra_filename);
        let mut text = format!(
            "unit\t{}\t{}\t{}\n",
            self.unit.krate,
            krate,
            if self.test { "test" } else { "build" }
        );
        for record in &self.records {
            text.push_str(record);
            text.push('\n');
        }
        let path = std::path::Path::new(&dir).join(format!("{unit}.tsv"));
        if std::fs::create_dir_all(&dir)
            .and_then(|()| std::fs::write(&path, text))
            .is_err()
        {
            tcx.dcx().fatal(format!(
                "the primitive gate cannot write {}",
                path.display()
            ));
        }
    }
}

impl Gate {
    /// The parameters of a Leptos component, read on the `fn` its macro writes
    /// to hold the body, and keyed as the source writes the component.
    fn component_sites(
        &mut self,
        tcx: TyCtxt<'_>,
        def: rustc_hir::def_id::LocalDefId,
        component: &str,
        item: &hir::Item<'_>,
    ) {
        let vis = place::written_vis(tcx, item.vis_span);
        self.fn_sites_as(tcx, def, privacy(vis), Some(component));
    }
}

struct Callbacks;

impl rustc_driver::Callbacks for Callbacks {
    fn config(&mut self, config: &mut rustc_interface::interface::Config) {
        // A bench, an example or an integration test is test code, whether
        // cargo builds it with `--test` or as a binary.
        let root = config
            .input
            .opt_path()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let test = config.opts.test
            || ["benches/", "examples/", "tests/"]
                .iter()
                .any(|dir| root.contains(&format!("/{dir}")) || root.starts_with(dir));
        config.register_lints = Some(Box::new(move |_sess, store| {
            store.register_lints(&[AMBRE_PRIMITIVE_GATE]);
            store.register_late_pass(move |_| Box::new(Gate::new(test)));
        }));
    }
}

fn main() {
    // Cargo calls a workspace wrapper as `wrapper <rustc> <args>`.
    let args: Vec<String> = std::env::args().skip(1).collect();
    rustc_driver::install_ice_hook("https://github.com/ambre-suite/ambre", |_| ());
    rustc_driver::run_compiler(&args, &mut Callbacks);
}
