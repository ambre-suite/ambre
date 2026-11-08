//! Spike: the ARCH-073 unwrap report as a late rustc lint pass.
//!
//! The driver stands in for rustc as `RUSTC_WORKSPACE_WRAPPER`, so cargo runs
//! it on the workspace members alone. It registers one late lint pass. The
//! pass reads each body with its typeck results, so every receiver has its
//! type, every callee its `DefId`, and every name its crate.
//!
//! The pass writes one line per site to `$AMBRE_UNWRAP_OUT/<unit>.tsv`. A
//! diagnostic would not survive the `-A warnings` of the wasm `rustflags`, and
//! a file per unit survives the cache of a fresh unit.
//!
//! The rule it holds is the rule of `ARCH-073`, read with types:
//!
//!   * a newtype is a struct of a workspace crate with one field that is not
//!     `PhantomData`, whose type, instantiated, is a primitive, `str`,
//!     `String`, or bytes;
//!   * an unwrap is an inherent method of the newtype that takes `self` alone
//!     and returns that inner type or a borrow of it, `len` and the `is_*` and
//!     `has_*` predicates excepted; `as_ref`, `borrow`, `to_string` and `into`
//!     on the newtype when they return an inner type; `From::from` of the
//!     newtype into an inner type; and a read of its field;
//!   * the unwrap travels through `&`, `*`, a `let` binding and the peels of
//!     the syn report;
//!   * a site is an unwrap handed, as an argument or a receiver, to a callee
//!     whose crate is a workspace crate;
//!   * the module of the newtype, an impl whose self type is the newtype, a
//!     trait impl whose first argument is the newtype, and a named conversion
//!     are exempt.

#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_lint;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;

use std::collections::HashMap;
use std::fmt::Write as _;

use rustc_hir as hir;
use rustc_hir::def::{DefKind, Res};
use rustc_hir::def_id::{DefId, LocalDefId};
use rustc_lint::{LateContext, LateLintPass};
use rustc_middle::ty::{self, Ty, TyCtxt};
use rustc_span::Span;

rustc_session::declare_lint! {
    /// A workspace newtype unwrapped before a workspace callee (`ARCH-073`).
    pub AMBRE_NEWTYPE_UNWRAP,
    // rustc skips a late pass whose lints are all allowed, and the wasm
    // `rustflags` allow every warning. The pass emits no diagnostic, so a
    // deny level only keeps it running.
    Deny,
    "a newtype unwrapped and handed to a workspace callee"
}

/// The conversions and views an unwrap travels through, as the syn report
/// lists them in `PEELS`.
const PEELS: &[&str] = &[
    "clone",
    "to_owned",
    "into",
    "to_string",
    "to_vec",
    "as_ref",
    "as_str",
    "as_bytes",
    "borrow",
];

/// The trait methods that unwrap a newtype when they return its inner type.
const TRAIT_UNWRAPS: &[&str] = &["as_ref", "borrow", "to_string", "into"];

#[derive(Default)]
struct UnwrapPass {
    sites: Vec<String>,
}

rustc_session::impl_lint_pass!(UnwrapPass => [AMBRE_NEWTYPE_UNWRAP]);

/// One unwrap: the newtype it opens and the name of the opening.
#[derive(Clone)]
struct Unwrap {
    newtype: DefId,
    how: String,
}

fn is_workspace(tcx: TyCtxt<'_>, def: DefId) -> bool {
    tcx.crate_name(def.krate).as_str().starts_with("ambre_")
}

/// Whether `ty`, its references peeled, is an inner type of a newtype.
fn is_inner(tcx: TyCtxt<'_>, ty: Ty<'_>) -> bool {
    let ty = ty.peel_refs();
    match ty.kind() {
        ty::Bool | ty::Char | ty::Int(_) | ty::Uint(_) | ty::Float(_) | ty::Str => true,
        ty::Slice(e) | ty::Array(e, _) => matches!(e.kind(), ty::Uint(ty::UintTy::U8)),
        ty::Adt(adt, args) => {
            let path = tcx.def_path_str(adt.did());
            path == "std::string::String"
                || path == "alloc::string::String"
                || ((path == "std::vec::Vec" || path == "alloc::vec::Vec")
                    && matches!(args.type_at(0).kind(), ty::Uint(ty::UintTy::U8)))
        }
        _ => false,
    }
}

/// The newtype `ty` names, its references peeled, if it is one.
fn newtype_of<'tcx>(cx: &LateContext<'tcx>, ty: Ty<'tcx>) -> Option<DefId> {
    let tcx = cx.tcx;
    let ty::Adt(adt, args) = ty.peel_refs().kind() else {
        return None;
    };
    if !adt.is_struct() || !is_workspace(tcx, adt.did()) {
        return None;
    }
    let fields: Vec<Ty<'tcx>> = adt
        .all_fields()
        .map(|f| f.ty(tcx, args))
        .filter(|t| !t.is_phantom_data())
        .collect();
    let [field] = fields[..] else {
        return None;
    };
    let field = tcx
        .try_normalize_erasing_regions(cx.typing_env(), field)
        .unwrap_or(field);
    is_inner(tcx, field).then_some(adt.did())
}

fn name(tcx: TyCtxt<'_>, def: DefId) -> String {
    tcx.item_name(def).to_string()
}

struct BodyScan<'a, 'tcx> {
    cx: &'a LateContext<'tcx>,
    typeck: &'tcx ty::TypeckResults<'tcx>,
    owner: LocalDefId,
    tainted: HashMap<hir::HirId, Unwrap>,
    sites: &'a mut Vec<String>,
}

impl<'a, 'tcx> BodyScan<'a, 'tcx> {
    /// The unwrap that `expr` evaluates to, if any.
    fn unwrap_of(&self, expr: &'tcx hir::Expr<'tcx>) -> Option<Unwrap> {
        let tcx = self.cx.tcx;
        match expr.kind {
            hir::ExprKind::AddrOf(_, _, inner)
            | hir::ExprKind::DropTemps(inner)
            | hir::ExprKind::Unary(hir::UnOp::Deref, inner) => self.unwrap_of(inner),
            hir::ExprKind::Path(hir::QPath::Resolved(None, path)) => match path.res {
                Res::Local(id) => self.tainted.get(&id).cloned(),
                _ => None,
            },
            hir::ExprKind::Field(base, field) => {
                let newtype = newtype_of(self.cx, self.typeck.expr_ty_adjusted(base))?;
                if !is_inner(tcx, self.typeck.expr_ty(expr)) {
                    return None;
                }
                Some(Unwrap {
                    newtype,
                    how: format!(".{field}"),
                })
            }
            hir::ExprKind::MethodCall(seg, recv, args, _) => {
                let method = self.typeck.type_dependent_def_id(expr.hir_id)?;
                let method_name = seg.ident.as_str();
                if let Some(inner) = self.unwrap_of(recv) {
                    return PEELS.contains(&method_name).then_some(inner);
                }
                if !args.is_empty() {
                    return None;
                }
                self.accessor(method, self.typeck.expr_ty(recv), self.typeck.expr_ty(expr))
            }
            hir::ExprKind::Call(func, args) => {
                let hir::ExprKind::Path(ref qpath) = func.kind else {
                    return None;
                };
                let Res::Def(DefKind::AssocFn, callee) = self.typeck.qpath_res(qpath, func.hir_id)
                else {
                    return None;
                };
                let [arg] = args else {
                    return None;
                };
                if let Some(inner) = self.unwrap_of(arg) {
                    // `String::from(name.as_str())` keeps the unwrap, as the
                    // syn report reads it; a numeric `f32::from` does not.
                    let string = matches!(self.typeck.expr_ty(expr).kind(),
                        ty::Adt(adt, _) if name(tcx, adt.did()) == "String");
                    return (name(tcx, callee) == "from" && string).then_some(inner);
                }
                let from = name(tcx, callee) == "from"
                    && tcx
                        .trait_of_assoc(callee)
                        .is_some_and(|t| name(tcx, t) == "From");
                let arg_ty = self.typeck.expr_ty(arg);
                let ret = self.typeck.expr_ty(expr);
                if from {
                    let newtype = newtype_of(self.cx, arg_ty)?;
                    return is_inner(tcx, ret).then(|| Unwrap {
                        newtype,
                        how: "from".to_owned(),
                    });
                }
                self.accessor(callee, arg_ty, ret)
            }
            _ => None,
        }
    }

    /// The unwrap a call of `method` on a receiver of type `recv` makes.
    fn accessor(&self, method: DefId, recv: Ty<'tcx>, ret: Ty<'tcx>) -> Option<Unwrap> {
        let tcx = self.cx.tcx;
        let newtype = newtype_of(self.cx, recv)?;
        if !is_inner(tcx, ret) {
            return None;
        }
        let method_name = name(tcx, method);
        if method_name == "len" || method_name.starts_with("is_") || method_name.starts_with("has_")
        {
            return None;
        }
        if let Some(imp) = tcx.impl_of_assoc(method) {
            // An inherent method of the newtype itself.
            if tcx.impl_opt_trait_ref(imp).is_some() {
                return None;
            }
            let self_ty = tcx.type_of(imp).instantiate_identity();
            let same = matches!(self_ty.kind(), ty::Adt(adt, _) if adt.did() == newtype);
            if !same
                || tcx
                    .fn_sig(method)
                    .skip_binder()
                    .inputs()
                    .skip_binder()
                    .len()
                    != 1
            {
                return None;
            }
            return Some(Unwrap {
                newtype,
                how: method_name,
            });
        }
        if tcx.trait_of_assoc(method).is_some() && TRAIT_UNWRAPS.contains(&method_name.as_str()) {
            return Some(Unwrap {
                newtype,
                how: method_name,
            });
        }
        None
    }

    /// Whether the body may unwrap `newtype` freely.
    fn exempt(&self, newtype: DefId) -> bool {
        let tcx = self.cx.tcx;
        if let Some(local) = newtype.as_local()
            && tcx.parent_module_from_def_id(local) == tcx.parent_module_from_def_id(self.owner)
        {
            return true;
        }
        let names =
            |ty: Ty<'tcx>| matches!(ty.peel_refs().kind(), ty::Adt(adt, _) if adt.did() == newtype);
        let mut def = self.owner.to_def_id();
        let mut first_fn = true;
        while let Some(parent) = tcx.opt_parent(def) {
            if matches!(tcx.def_kind(def), DefKind::AssocFn) && first_fn {
                first_fn = false;
                // A named conversion: one newtype parameter, returns `Self`.
                if let Some(imp) = tcx.impl_of_assoc(def)
                    && tcx.impl_opt_trait_ref(imp).is_none()
                    && !tcx.associated_item(def).is_method()
                {
                    let sig = tcx.fn_sig(def).instantiate_identity().skip_binder();
                    let self_ty = tcx.type_of(imp).instantiate_identity();
                    if let [input] = sig.inputs()
                        && names(*input)
                        && sig.output() == self_ty
                    {
                        return true;
                    }
                }
            }
            if let DefKind::Impl { of_trait } = tcx.def_kind(def) {
                if names(tcx.type_of(def).instantiate_identity()) {
                    return true;
                }
                if of_trait
                    && let Some(trait_ref) = tcx.impl_opt_trait_ref(def)
                    && let Some(arg) = trait_ref.instantiate_identity().args.types().nth(1)
                    && names(arg)
                {
                    return true;
                }
            }
            def = parent;
        }
        false
    }

    /// Whether parameter `index` of `callee` is `impl Into<T>` with `T`
    /// foreign: its argument goes on to a foreign call (`ARCH-073`).
    fn boundary(&self, callee: DefId, index: usize) -> bool {
        let tcx = self.cx.tcx;
        let sig = tcx.fn_sig(callee).instantiate_identity().skip_binder();
        let Some(param) = sig.inputs().get(index) else {
            return false;
        };
        if !matches!(param.kind(), ty::Param(_)) {
            return false;
        }
        tcx.predicates_of(callee)
            .instantiate_identity(tcx)
            .predicates
            .iter()
            .filter_map(|clause| clause.as_trait_clause())
            .map(|pred| pred.skip_binder())
            .any(|pred| {
                pred.self_ty() == *param
                    && name(tcx, pred.def_id()) == "Into"
                    && pred.trait_ref.args.types().nth(1).is_some_and(|target| {
                        !matches!(target.kind(), ty::Adt(adt, _) if is_workspace(tcx, adt.did()))
                    })
            })
    }

    fn judge(&mut self, site: &'tcx hir::Expr<'tcx>, callee: DefId, index: usize) {
        let tcx = self.cx.tcx;
        if !is_workspace(tcx, callee) || self.boundary(callee, index) {
            return;
        }
        let Some(unwrap) = self.unwrap_of(site) else {
            return;
        };
        if self.exempt(unwrap.newtype) {
            return;
        }
        let span: Span = site.span;
        let expanded = span.from_expansion();
        let span = span.source_callsite();
        let sm = tcx.sess.source_map();
        let lo = sm.lookup_char_pos(span.lo());
        let file = lo.file.name.prefer_local_unconditionally().to_string();
        let mut line = String::new();
        let _ = write!(
            line,
            "{}\t{}:{}\t{}::{} -> {}\t{}",
            tcx.crate_name(rustc_span::def_id::LOCAL_CRATE),
            file,
            lo.line,
            name(tcx, unwrap.newtype),
            unwrap.how,
            tcx.def_path_str(callee),
            if expanded { "macro" } else { "source" },
        );
        self.sites.push(line);
    }
}

impl<'a, 'tcx> hir::intravisit::Visitor<'tcx> for BodyScan<'a, 'tcx> {
    fn visit_local(&mut self, local: &'tcx hir::LetStmt<'tcx>) {
        if let Some(init) = local.init
            && let hir::PatKind::Binding(_, id, _, None) = local.pat.kind
            && let Some(unwrap) = self.unwrap_of(init)
        {
            self.tainted.insert(id, unwrap);
        }
        hir::intravisit::walk_local(self, local);
    }

    fn visit_expr(&mut self, expr: &'tcx hir::Expr<'tcx>) {
        match expr.kind {
            hir::ExprKind::Call(func, args) => {
                if let hir::ExprKind::Path(ref qpath) = func.kind
                    && let Res::Def(DefKind::Fn | DefKind::AssocFn | DefKind::Ctor(..), callee) =
                        self.typeck.qpath_res(qpath, func.hir_id)
                {
                    for (index, arg) in args.iter().enumerate() {
                        self.judge(arg, callee, index);
                    }
                }
            }
            hir::ExprKind::MethodCall(_, recv, args, _) => {
                if let Some(callee) = self.typeck.type_dependent_def_id(expr.hir_id) {
                    self.judge(recv, callee, 0);
                    for (index, arg) in args.iter().enumerate() {
                        self.judge(arg, callee, index + 1);
                    }
                }
            }
            _ => {}
        }
        hir::intravisit::walk_expr(self, expr);
    }
}

impl<'tcx> LateLintPass<'tcx> for UnwrapPass {
    fn check_body(&mut self, cx: &LateContext<'tcx>, body: &hir::Body<'tcx>) {
        let tcx = cx.tcx;
        let owner = tcx.hir_body_owner_def_id(body.id());
        // A closure is its own body, and the pass meets it again there.
        let typeck = tcx.typeck_body(body.id());
        let mut scan = BodyScan {
            cx,
            typeck,
            owner,
            tainted: HashMap::new(),
            sites: &mut self.sites,
        };
        let value: &'tcx hir::Expr<'tcx> = tcx.hir_body(body.id()).value;
        hir::intravisit::Visitor::visit_expr(&mut scan, value);
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
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(format!("{dir}/{unit}.tsv"), text);
    }
}

struct Callbacks;

impl rustc_driver::Callbacks for Callbacks {
    fn config(&mut self, config: &mut rustc_interface::interface::Config) {
        config.register_lints = Some(Box::new(|_sess, store| {
            store.register_lints(&[AMBRE_NEWTYPE_UNWRAP]);
            store.register_late_pass(|_| Box::new(UnwrapPass::default()));
        }));
    }
}

fn main() {
    // Cargo calls a workspace wrapper as `wrapper <rustc> <args>`.
    let args: Vec<String> = std::env::args().skip(1).collect();
    rustc_driver::install_ice_hook("https://github.com/ambre-suite/ambre", |_| ());
    rustc_driver::run_compiler(&args, &mut Callbacks);
}
