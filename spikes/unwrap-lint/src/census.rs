//! Spike: the checks of the primitive gate as a second late lint pass.
//!
//! Each check writes sites, never verdicts: the judge stays the allowlist
//! code of `ambre-primitives`, and `compare.py` sets the sites beside it.
//! One line per site in `$AMBRE_UNWRAP_OUT/census/<unit>.tsv`:
//! `check  crate  file:line  kind  leaf  detail  origin`.
//!
//! | Check | Row | What the pass reads |
//! | :--- | :--- | :--- |
//! | `sig` | `ARCH-033` | a field, parameter, return, constant, static, alias or associated item whose written type names a primitive |
//! | `newtype` | `ARCH-062` | a struct with one field, `PhantomData` aside, written as a primitive, and that field's visibility |
//! | `field-read` | the newtype rule | a read of a newtype's field outside its module |
//! | `deref` | `ARCH-037` | a `Deref` whose target is a primitive or hands one out |
//! | `secret` | `ARCH-045`, `ARCH-048`, `ARCH-049` | a name that reads a secret or lends it to a sink |
//! | `zeroize` | `ARCH-046` | an item of the `zeroize` crate |
//! | `alias` | `ARCH-066` | an alias that stands for a family of `ambre-core` alone |
//! | `bool-lit` | `ARCH-040` | a `bool` literal handed to a workspace `fn` that is not private |

use std::fmt::Write as _;

use rustc_hir as hir;
use rustc_hir::def::{DefKind, Res};
use rustc_hir::def_id::{DefId, LocalDefId};
use rustc_hir::intravisit::VisitorExt as _;
use rustc_lint::{LateContext, LateLintPass};
use rustc_middle::ty::{self, TyCtxt};
use rustc_span::{Span, Symbol};

rustc_session::declare_lint! {
    /// The sites of the primitive gate, read with types (spike).
    pub AMBRE_PRIMITIVE_CENSUS,
    // A deny level only keeps the pass running; it emits no diagnostic.
    Deny,
    "a site the primitive gate reads"
}

/// The names that read a secret, or lend it to one foreign call.
const SECRET_NAMES: &[&str] = &[
    "expose",
    "expose_mut",
    "clone_secret",
    "present_bearer",
    "present_link",
    "present_state",
];

#[derive(Default)]
pub struct CensusPass {
    sites: Vec<String>,
}

rustc_session::impl_lint_pass!(CensusPass => [AMBRE_PRIMITIVE_CENSUS]);

/// Whether a written type names a primitive anywhere, as the syn gate's
/// `holds_primitive` reads the text, but through name resolution.
struct Primitive {
    found: bool,
}

impl<'v> hir::intravisit::Visitor<'v> for Primitive {
    fn visit_path(&mut self, path: &hir::Path<'v>, _id: hir::HirId) {
        match path.res {
            Res::PrimTy(_) => self.found = true,
            Res::Def(DefKind::Struct, _) | Res::Def(DefKind::TyAlias, _) => {
                let last = path.segments.last().map(|s| s.ident.name);
                if last == Some(Symbol::intern("String")) || last == Some(Symbol::intern("Uuid")) {
                    self.found = true;
                }
            }
            _ => {}
        }
        hir::intravisit::walk_path(self, path);
    }
}

fn names_primitive<'v>(ty: &'v hir::Ty<'v>) -> bool {
    let mut seek = Primitive { found: false };
    seek.visit_ty_unambig(ty);
    seek.found
}

impl CensusPass {
    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        tcx: TyCtxt<'_>,
        check: &str,
        span: Span,
        kind: &str,
        leaf: &str,
        detail: &str,
        origin: &str,
    ) {
        let expanded = span.from_expansion();
        let span = span.source_callsite();
        let lo = tcx.sess.source_map().lookup_char_pos(span.lo());
        let mut line = String::new();
        let _ = write!(
            line,
            "{check}\t{}\t{}:{}\t{kind}\t{leaf}\t{detail}\t{}",
            tcx.crate_name(rustc_span::def_id::LOCAL_CRATE),
            lo.file.name.prefer_local_unconditionally(),
            lo.line,
            if expanded { "macro" } else { origin },
        );
        self.sites.push(line);
    }

    fn sig_site<'v>(
        &mut self,
        cx: &LateContext<'_>,
        ty: &'v hir::Ty<'v>,
        kind: &str,
        leaf: &str,
        origin: &str,
    ) {
        if names_primitive(ty) {
            let text = cx
                .tcx
                .sess
                .source_map()
                .span_to_snippet(ty.span)
                .unwrap_or_default();
            let text: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
            self.push(cx.tcx, "sig", ty.span, kind, leaf, &text, origin);
        }
    }

    fn fn_sig<'v>(
        &mut self,
        cx: &LateContext<'_>,
        def: LocalDefId,
        sig: &hir::FnSig<'v>,
        name: &str,
        origin: &str,
    ) {
        let idents = cx.tcx.fn_arg_idents(def.to_def_id());
        let skip = usize::from(sig.decl.implicit_self.has_implicit_self());
        for (index, input) in sig.decl.inputs.iter().enumerate().skip(skip) {
            let param = idents
                .get(index)
                .copied()
                .flatten()
                .map_or_else(|| "_".to_owned(), |i| i.to_string());
            self.sig_site(cx, input, "param", &format!("{name}({param})"), origin);
        }
        if let hir::FnRetTy::Return(ret) = sig.decl.output {
            self.sig_site(cx, ret, "ret", &format!("{name}()"), origin);
        }
    }

    /// Whether `def` sits in an impl of a trait.
    fn in_trait_impl(tcx: TyCtxt<'_>, def: LocalDefId) -> bool {
        let parent = tcx.local_parent(def);
        matches!(tcx.def_kind(parent), DefKind::Impl { of_trait: true })
    }

    fn module_of(tcx: TyCtxt<'_>, def: LocalDefId) -> String {
        let module = tcx.parent_module_from_def_id(def);
        tcx.def_path_str(module.to_def_id())
    }

    fn named_site(&mut self, cx: &LateContext<'_>, callee: DefId, span: Span, owner: LocalDefId) {
        let tcx = cx.tcx;
        let krate = tcx.crate_name(callee.krate);
        if krate.as_str() == "zeroize" {
            let module = Self::module_of(tcx, owner);
            self.push(
                tcx,
                "zeroize",
                span,
                "name",
                &tcx.def_path_str(callee),
                &module,
                "source",
            );
            return;
        }
        if !krate.as_str().starts_with("ambre_") {
            return;
        }
        let Some(name) = tcx.opt_item_name(callee) else {
            return;
        };
        if SECRET_NAMES.contains(&name.as_str()) {
            let module = Self::module_of(tcx, owner);
            self.push(
                tcx,
                "secret",
                span,
                "name",
                name.as_str(),
                &module,
                "source",
            );
        }
    }
}

impl<'tcx> LateLintPass<'tcx> for CensusPass {
    fn check_field_def(&mut self, cx: &LateContext<'tcx>, field: &'tcx hir::FieldDef<'tcx>) {
        self.sig_site(cx, field.ty, "field", field.ident.as_str(), "source");
    }

    fn check_item(&mut self, cx: &LateContext<'tcx>, item: &'tcx hir::Item<'tcx>) {
        let tcx = cx.tcx;
        let def = item.owner_id.def_id;
        match item.kind {
            hir::ItemKind::Const(ident, _, ty, _) => {
                self.sig_site(cx, ty, "const", ident.as_str(), "source")
            }
            hir::ItemKind::Static(_, ident, ty, _) => {
                self.sig_site(cx, ty, "static", ident.as_str(), "source");
            }
            hir::ItemKind::TyAlias(ident, _, ty) => {
                self.sig_site(cx, ty, "alias", ident.as_str(), "source");
                // An alias whose written target is one family of ambre-core.
                let mut target = ty;
                while let hir::TyKind::Ref(_, mt) = target.kind {
                    target = mt.ty;
                }
                if let hir::TyKind::Path(hir::QPath::Resolved(None, path)) = target.kind
                    && let Res::Def(DefKind::Struct, did) = path.res
                    && tcx.crate_name(did.krate).as_str() == "ambre_core"
                    && !tcx.generics_of(did).own_params.is_empty()
                {
                    let family = tcx.def_path_str(did);
                    self.push(
                        tcx,
                        "alias",
                        item.span,
                        "alias",
                        ident.as_str(),
                        &family,
                        "source",
                    );
                }
            }
            hir::ItemKind::Fn { ident, sig, .. } => {
                self.fn_sig(cx, def, &sig, ident.as_str(), "source")
            }
            hir::ItemKind::Struct(ident, _, _) => {
                let adt = tcx.adt_def(def);
                let fields: Vec<&ty::FieldDef> = adt
                    .all_fields()
                    .filter(|f| !tcx.type_of(f.did).instantiate_identity().is_phantom_data())
                    .collect();
                if let [field] = fields[..]
                    && let Some(local) = field.did.as_local()
                    && let hir::Node::Field(hir_field) = tcx.hir_node_by_def_id(local)
                    && names_primitive(hir_field.ty)
                {
                    let module = tcx.parent_module_from_def_id(def);
                    let vis = match tcx.visibility(field.did) {
                        ty::Visibility::Public => "pub",
                        ty::Visibility::Restricted(m) if m == module.to_def_id() => "private",
                        ty::Visibility::Restricted(_) => "restricted",
                    };
                    let text = tcx
                        .sess
                        .source_map()
                        .span_to_snippet(hir_field.ty.span)
                        .unwrap_or_default();
                    self.push(
                        tcx,
                        "newtype",
                        item.span,
                        vis,
                        ident.as_str(),
                        &text,
                        "source",
                    );
                }
            }
            hir::ItemKind::Impl(imp) => {
                let Some(trait_ref) = tcx.impl_opt_trait_ref(def) else {
                    return;
                };
                let trait_path = tcx.def_path_str(trait_ref.skip_binder().def_id);
                if trait_path != "std::ops::Deref" && trait_path != "core::ops::Deref" {
                    return;
                }
                let _ = imp;
                for assoc in tcx.associated_items(def).in_definition_order() {
                    if tcx.item_name(assoc.def_id).as_str() != "Target" {
                        continue;
                    }
                    let target = tcx.type_of(assoc.def_id).instantiate_identity();
                    let hands_out = match target.kind() {
                        ty::Ref(..) | ty::Slice(_) | ty::Array(..) | ty::Str => true,
                        ty::Bool | ty::Char | ty::Int(_) | ty::Uint(_) | ty::Float(_) => true,
                        ty::Adt(adt, args) => {
                            let path = tcx.def_path_str(adt.did());
                            let carrier = ["Vec", "Box", "Rc", "Arc", "Cow", "String"]
                                .iter()
                                .any(|c| path.ends_with(&format!("::{c}")));
                            carrier
                                && (path.ends_with("String")
                                    || args.types().any(|t| {
                                        let t = t.peel_refs();
                                        t.is_primitive()
                                            || t.is_str()
                                            || matches!(t.kind(), ty::Slice(_))
                                    }))
                        }
                        _ => false,
                    };
                    if hands_out {
                        let owner = tcx.type_of(def).instantiate_identity().to_string();
                        self.push(
                            tcx,
                            "deref",
                            item.span,
                            "deref",
                            &owner,
                            &target.to_string(),
                            "source",
                        );
                    }
                }
            }
            _ => {}
        }
    }

    fn check_impl_item(&mut self, cx: &LateContext<'tcx>, item: &'tcx hir::ImplItem<'tcx>) {
        let def = item.owner_id.def_id;
        let origin = if Self::in_trait_impl(cx.tcx, def) {
            "trait-impl"
        } else {
            "source"
        };
        match item.kind {
            hir::ImplItemKind::Fn(sig, _) => {
                self.fn_sig(cx, def, &sig, item.ident.as_str(), origin)
            }
            hir::ImplItemKind::Const(ty, _) | hir::ImplItemKind::Type(ty) => {
                self.sig_site(cx, ty, "assoc", item.ident.as_str(), origin);
            }
        }
    }

    fn check_trait_item(&mut self, cx: &LateContext<'tcx>, item: &'tcx hir::TraitItem<'tcx>) {
        let def = item.owner_id.def_id;
        match item.kind {
            hir::TraitItemKind::Fn(sig, _) => {
                self.fn_sig(cx, def, &sig, item.ident.as_str(), "trait")
            }
            hir::TraitItemKind::Const(ty, _, _) => {
                self.sig_site(cx, ty, "assoc", item.ident.as_str(), "trait")
            }
            hir::TraitItemKind::Type(..) => {}
        }
    }

    fn check_path(&mut self, cx: &LateContext<'tcx>, path: &hir::Path<'tcx>, id: hir::HirId) {
        if let Res::Def(_, did) = path.res {
            let owner = cx.tcx.hir_get_parent_item(id).def_id;
            self.named_site(cx, did, path.span, owner);
        }
    }

    fn check_expr(&mut self, cx: &LateContext<'tcx>, expr: &'tcx hir::Expr<'tcx>) {
        let tcx = cx.tcx;
        let owner = tcx.hir_get_parent_item(expr.hir_id).def_id;
        match expr.kind {
            hir::ExprKind::MethodCall(seg, _, args, _) => {
                if let Some(callee) = cx.typeck_results().type_dependent_def_id(expr.hir_id) {
                    self.named_site(cx, callee, seg.ident.span, owner);
                    self.bool_literals(cx, callee, args, 1, owner);
                }
            }
            hir::ExprKind::Call(func, args) => {
                if let hir::ExprKind::Path(ref qpath) = func.kind
                    && let Res::Def(DefKind::Fn | DefKind::AssocFn, callee) =
                        cx.typeck_results().qpath_res(qpath, func.hir_id)
                {
                    self.bool_literals(cx, callee, args, 0, owner);
                }
            }
            hir::ExprKind::Field(base, field) => {
                let base_ty = cx.typeck_results().expr_ty_adjusted(base);
                if let Some(newtype) = crate::newtype_of(cx, base_ty)
                    && let Some(local) = newtype.as_local()
                    && tcx.parent_module_from_def_id(local) != tcx.parent_module_from_def_id(owner)
                {
                    let name = tcx.item_name(newtype);
                    self.push(
                        tcx,
                        "field-read",
                        expr.span,
                        "field",
                        field.as_str(),
                        name.as_str(),
                        "source",
                    );
                } else if let Some(newtype) = crate::newtype_of(cx, base_ty)
                    && !newtype.is_local()
                {
                    let name = tcx.item_name(newtype);
                    self.push(
                        tcx,
                        "field-read",
                        expr.span,
                        "field",
                        field.as_str(),
                        name.as_str(),
                        "source",
                    );
                }
            }
            _ => {}
        }
    }

    fn check_crate_post(&mut self, cx: &LateContext<'tcx>) {
        let Ok(dir) = std::env::var("AMBRE_UNWRAP_OUT") else {
            return;
        };
        let tcx = cx.tcx;
        let unit = format!(
            "{}-{}-{}",
            tcx.crate_name(rustc_span::def_id::LOCAL_CRATE),
            tcx.sess.opts.target_triple,
            tcx.stable_crate_id(rustc_span::def_id::LOCAL_CRATE)
                .as_u64(),
        );
        let mut text = String::new();
        for site in &self.sites {
            text.push_str(site);
            text.push('\n');
        }
        let _ = std::fs::create_dir_all(format!("{dir}/census"));
        let _ = std::fs::write(format!("{dir}/census/{unit}.tsv"), text);
    }
}

impl CensusPass {
    /// A `bool` literal handed to a workspace `fn` that is not private.
    fn bool_literals(
        &mut self,
        cx: &LateContext<'_>,
        callee: DefId,
        args: &[hir::Expr<'_>],
        offset: usize,
        owner: LocalDefId,
    ) {
        let tcx = cx.tcx;
        if !tcx.crate_name(callee.krate).as_str().starts_with("ambre_") {
            return;
        }
        let private = callee.as_local().is_some_and(|local| {
            matches!(tcx.visibility(callee), ty::Visibility::Restricted(m)
                if m == tcx.parent_module_from_def_id(local).to_def_id())
        });
        if private {
            return;
        }
        let _ = owner;
        let sig = tcx.fn_sig(callee).instantiate_identity().skip_binder();
        for (index, arg) in args.iter().enumerate() {
            // Only a parameter typed `bool`: `impl Into<T>` takes a literal freely.
            if !sig
                .inputs()
                .get(index + offset)
                .is_some_and(|t| t.is_bool())
            {
                continue;
            }
            let mut inner = arg;
            while let hir::ExprKind::Unary(hir::UnOp::Not, e) | hir::ExprKind::AddrOf(_, _, e) =
                inner.kind
            {
                inner = e;
            }
            if let hir::ExprKind::Lit(lit) = inner.kind
                && matches!(lit.node, rustc_ast::LitKind::Bool(_))
            {
                let name = tcx.def_path_str(callee);
                self.push(tcx, "bool-lit", arg.span, "arg", &name, "", "source");
            }
        }
    }
}
