//! The unwrap check (`ARCH-073`): a workspace newtype unwrapped and handed to
//! a workspace callee, read with the typeck results of each body.
//!
//! * A newtype is a struct of a workspace crate with one field, `PhantomData`
//!   apart. That field, instantiated, is a primitive, a `str`, a `String` or
//!   bytes.
//! * An unwrap is an inherent method of the newtype that takes `self` alone
//!   and returns that inner type or a borrow of it. `len` and the `is_*` and
//!   `has_*` predicates are no unwrap.
//! * `as_ref`, `borrow`, `to_string` and `into` on the newtype unwrap it when
//!   they return an inner type. So do `From::from` into an inner type and a
//!   read of its field.
//! * The unwrap travels through `&`, `*`, a `let` binding and the views and
//!   conversions of `PEELS`.
//! * A site is an unwrap handed to a callee of a workspace crate, as an
//!   argument or a receiver. A parameter `impl Into<T>` with `T` foreign is a
//!   foreign call.
//! * The module of the newtype and an impl whose self type is the newtype are
//!   exempt. So are a trait impl whose first argument is the newtype and a
//!   named conversion.
//!
//! A record is `unwrap  key  at`, the key as `ambre-unwrap.tsv` names it:
//! `crate | file | fn | Newtype::accessor -> callee`.

use std::collections::HashMap;

use rustc_hir as hir;
use rustc_hir::def::{DefKind, Res};
use rustc_hir::def_id::{DefId, LocalDefId};
use rustc_lint::LateContext;
use rustc_middle::ty::{self, Ty, TyCtxt};

use crate::place;
use crate::types::is_workspace;

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

/// One unwrap: the newtype it opens and the name of the opening.
#[derive(Clone)]
pub struct Unwrap {
    newtype: DefId,
    how: String,
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
pub(crate) fn newtype_of<'tcx>(cx: &LateContext<'tcx>, ty: Ty<'tcx>) -> Option<DefId> {
    let tcx = cx.tcx;
    let ty::Adt(adt, args) = ty.peel_refs().kind() else {
        return None;
    };
    if !adt.is_struct() || !is_workspace(tcx, adt.did()) {
        return None;
    }
    let fields: Vec<ty::Unnormalized<'tcx, Ty<'tcx>>> = adt
        .all_fields()
        .map(|f| f.ty(tcx, *args))
        .filter(|t| !t.skip_normalization().is_phantom_data())
        .collect();
    let [field] = fields[..] else {
        return None;
    };
    let field = tcx
        .try_normalize_erasing_regions(cx.typing_env(), field)
        .unwrap_or(field.skip_normalization());
    is_inner(tcx, field).then_some(adt.did())
}

fn name(tcx: TyCtxt<'_>, def: DefId) -> String {
    tcx.item_name(def).to_string()
}

pub struct BodyScan<'a, 'tcx> {
    pub cx: &'a LateContext<'tcx>,
    pub typeck: &'tcx ty::TypeckResults<'tcx>,
    pub owner: LocalDefId,
    pub tainted: HashMap<hir::HirId, Unwrap>,
    pub unit: &'a place::Unit,
    /// Each site, as its key and its position.
    pub sites: &'a mut Vec<(String, String)>,
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
                    how: field.to_string(),
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
            let self_ty = tcx.type_of(imp).instantiate_identity().skip_normalization();
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
                    let sig = tcx
                        .fn_sig(def)
                        .instantiate_identity()
                        .skip_normalization()
                        .skip_binder();
                    let self_ty = tcx.type_of(imp).instantiate_identity().skip_normalization();
                    if let [input] = sig.inputs()
                        && names(*input)
                        && sig.output() == self_ty
                    {
                        return true;
                    }
                }
            }
            if let DefKind::Impl { of_trait } = tcx.def_kind(def) {
                if names(tcx.type_of(def).instantiate_identity().skip_normalization()) {
                    return true;
                }
                if of_trait
                    && let Some(trait_ref) = tcx.impl_opt_trait_ref(def)
                    && let Some(arg) = trait_ref
                        .instantiate_identity()
                        .skip_normalization()
                        .args
                        .types()
                        .nth(1)
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
        let sig = tcx
            .fn_sig(callee)
            .instantiate_identity()
            .skip_normalization()
            .skip_binder();
        let Some(param) = sig.inputs().get(index) else {
            return false;
        };
        if !matches!(param.kind(), ty::Param(_)) {
            return false;
        }
        tcx.clauses_of(callee)
            .instantiate_identity(tcx)
            .clauses
            .iter()
            .filter_map(|clause| clause.skip_normalization().as_trait_clause())
            .map(|pred| pred.skip_binder())
            .any(|pred| {
                pred.self_ty() == *param
                    && name(tcx, pred.def_id()) == "Into"
                    && pred.trait_ref.args.types().nth(1).is_some_and(|target| {
                        !matches!(target.kind(), ty::Adt(adt, _) if is_workspace(tcx, adt.did()))
                    })
            })
    }

    fn judge(&mut self, site: &'tcx hir::Expr<'tcx>, callee: DefId, index: usize, shown: &str) {
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
        let at = place::position(tcx, self.unit, site.span);
        let Some(file) = at.file.clone() else {
            return;
        };
        let mut owner = self.owner;
        while matches!(tcx.def_kind(owner), DefKind::Closure | DefKind::AnonConst) {
            owner = tcx.local_parent(owner);
        }
        let leaf = tcx
            .opt_item_name(owner.to_def_id())
            .map_or_else(String::new, |n| n.to_string());
        let scope = format!(
            "{}::{}",
            place::module_path(tcx, owner),
            place::item(tcx, owner, &leaf)
        );
        let key = format!(
            "{} | {} | {} | {}::{} -> {}",
            self.unit.krate,
            file,
            scope,
            name(tcx, unwrap.newtype),
            unwrap.how,
            shown,
        );
        self.sites.push((key, at.at()));
    }
}

/// The callee as the call writes it: the segments of its path, or `.name` for
/// a method.
fn path_shown(qpath: &hir::QPath<'_>) -> String {
    match qpath {
        hir::QPath::Resolved(_, path) => path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect::<Vec<_>>()
            .join("::"),
        hir::QPath::TypeRelative(ty, seg) => {
            let head = match ty.kind {
                hir::TyKind::Path(ref inner) => path_shown(inner),
                _ => String::new(),
            };
            if head.is_empty() {
                seg.ident.to_string()
            } else {
                format!("{head}::{}", seg.ident)
            }
        }
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
                    let shown = path_shown(qpath);
                    for (index, arg) in args.iter().enumerate() {
                        self.judge(arg, callee, index, &shown);
                    }
                }
            }
            hir::ExprKind::MethodCall(seg, recv, args, _) => {
                if let Some(callee) = self.typeck.type_dependent_def_id(expr.hir_id) {
                    let shown = format!(".{}", seg.ident);
                    self.judge(recv, callee, 0, &shown);
                    for (index, arg) in args.iter().enumerate() {
                        self.judge(arg, callee, index + 1, &shown);
                    }
                }
            }
            _ => {}
        }
        hir::intravisit::walk_expr(self, expr);
    }
}
