//! The `bool` literals each call of the workspace passes, tests included
//! (`ARCH-040`). The judge joins each one with the parameter site it reaches.
//!
//! A record is `lit  slot  at`. The slot names the callee the same way from
//! every crate, then the position of the parameter, `self` left out. A call of
//! a trait method writes the slot of the trait item and of the impl it
//! resolves to. A prop of a Leptos component writes the slot of the
//! component's parameter, by name.

use rustc_hir as hir;
use rustc_hir::def::{DefKind, Res};
use rustc_hir::def_id::DefId;
use rustc_lint::LateContext;
use rustc_middle::ty::{self, TyCtxt};

use crate::Gate;
use crate::place;
use crate::types;

impl Gate {
    /// The records of one call: `callee`, then its arguments from `offset`.
    pub fn literals<'tcx>(
        &mut self,
        cx: &LateContext<'tcx>,
        expr: &'tcx hir::Expr<'tcx>,
        callee: DefId,
        args: &'tcx [hir::Expr<'tcx>],
        method: bool,
    ) {
        let tcx = cx.tcx;
        if !types::is_workspace(tcx, callee) {
            return;
        }
        let has_self = matches!(tcx.def_kind(callee), DefKind::AssocFn)
            && tcx.associated_item(callee).is_method();
        // A path call passes the receiver as its first argument.
        let skip = usize::from(has_self && !method);
        let mut slots = vec![place::stable_path(tcx, callee)];
        if let Some(args_of) = cx.typeck_results().node_args_opt(expr.hir_id)
            && let Ok(Some(instance)) =
                ty::Instance::try_resolve(tcx, cx.typing_env(), callee, args_of)
            && instance.def_id() != callee
        {
            slots.push(place::stable_path(tcx, instance.def_id()));
        }
        let component = component_of(tcx, callee);
        for (index, arg) in args.iter().enumerate() {
            if index < skip || !literal(tcx, arg) {
                continue;
            }
            let at = place::position(tcx, &self.unit, arg.span).at();
            let position = index - skip;
            for slot in &slots {
                self.record(&["lit", &format!("{slot}#{position}"), &at]);
            }
            if let Some(component) = &component {
                self.record(&["lit", component, &at]);
            }
        }
    }
}

/// The slot of a component parameter when `callee` is the method of a props
/// builder that a Leptos `#[component]` writes: `component:<path>:<prop>`.
fn component_of(tcx: TyCtxt<'_>, callee: DefId) -> Option<String> {
    let imp = tcx.impl_of_assoc(callee)?;
    let self_ty = tcx.type_of(imp).instantiate_identity();
    let ty::Adt(adt, _) = self_ty.kind() else {
        return None;
    };
    let builder = tcx.item_name(adt.did()).to_string();
    let component = builder.strip_suffix("PropsBuilder")?;
    let module = tcx.parent(adt.did());
    let prop = tcx.item_name(callee);
    Some(format!(
        "component:{}::{component}:{prop}",
        place::stable_path(tcx, module)
    ))
}

/// Whether `arg` is a `bool` literal in a shape the rule counts.
///
/// * bare, or in a block of one expression;
/// * behind `&` or `!`, or under `Some`;
/// * in a tuple, an array, a repeat or a `vec!`;
/// * in a branch of an `if` or an arm of a `match`.
fn literal<'tcx>(tcx: TyCtxt<'tcx>, arg: &'tcx hir::Expr<'tcx>) -> bool {
    if from_vec(arg) {
        return any_literal(arg);
    }
    match arg.kind {
        // A literal the source writes: the `true` and `false` a macro such as
        // `matches!` writes are no literal of the caller (`ARCH-041`).
        hir::ExprKind::Lit(lit) => {
            matches!(lit.node, rustc_ast::LitKind::Bool(_)) && !lit.span.from_expansion()
        }
        hir::ExprKind::Unary(hir::UnOp::Not, inner)
        | hir::ExprKind::AddrOf(_, _, inner)
        | hir::ExprKind::DropTemps(inner) => literal(tcx, inner),
        hir::ExprKind::Block(block, _) => {
            block.stmts.is_empty() && block.expr.is_some_and(|e| literal(tcx, e))
        }
        hir::ExprKind::Tup(items) | hir::ExprKind::Array(items) => {
            items.iter().any(|e| literal(tcx, e))
        }
        hir::ExprKind::Repeat(inner, _) => literal(tcx, inner),
        hir::ExprKind::If(_, then, other) => {
            literal(tcx, then) || other.is_some_and(|e| literal(tcx, e))
        }
        hir::ExprKind::Match(_, arms, hir::MatchSource::Normal) => {
            arms.iter().any(|arm| literal(tcx, arm.body))
        }
        hir::ExprKind::Call(func, [inner]) => {
            let some = matches!(func.kind, hir::ExprKind::Path(hir::QPath::Resolved(_, path))
                if matches!(path.res, Res::Def(DefKind::Ctor(..), ctor)
                    if tcx.lang_items().option_some_variant() == Some(tcx.parent(ctor))));
            some && literal(tcx, inner)
        }
        _ => false,
    }
}

/// Whether a `vec!` call wrote `expr`.
fn from_vec(expr: &hir::Expr<'_>) -> bool {
    let data = expr.span.ctxt().outer_expn_data();
    matches!(data.kind, rustc_span::ExpnKind::Macro(_, name) if name.as_str() == "vec")
}

/// Whether a `bool` literal sits anywhere in `expr`.
fn any_literal<'tcx>(expr: &'tcx hir::Expr<'tcx>) -> bool {
    struct Seek(bool);
    impl<'tcx> hir::intravisit::Visitor<'tcx> for Seek {
        fn visit_expr(&mut self, e: &'tcx hir::Expr<'tcx>) {
            if let hir::ExprKind::Lit(lit) = e.kind
                && matches!(lit.node, rustc_ast::LitKind::Bool(_))
                && !lit.span.from_expansion()
            {
                self.0 = true;
            }
            hir::intravisit::walk_expr(self, e);
        }
    }
    let mut seek = Seek(false);
    hir::intravisit::Visitor::visit_expr(&mut seek, expr);
    seek.0
}
