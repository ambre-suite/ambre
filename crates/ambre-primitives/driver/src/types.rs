//! What a type holds, read on the type rustc gives a site rather than on its
//! text. Every alias is resolved, a foreign one included (`ARCH-044`).
//!
//! The walk opens a type argument, a reference, a slice, an array and a
//! tuple. It opens a pointer, a function pointer, a trait object and the
//! bounds of an `impl Trait` too. It never opens the fields of a struct: a
//! newtype holds its primitive inside, and a use of it writes none.

use std::collections::BTreeSet;

use rustc_hir::def_id::DefId;
use rustc_middle::ty::{self, Ty, TyCtxt};

/// The crates of the toolchain.
pub const TOOLCHAIN: &[&str] = &["std", "core", "alloc"];

/// Whether `def` belongs to a crate of the workspace.
pub fn is_workspace(tcx: TyCtxt<'_>, def: DefId) -> bool {
    tcx.crate_name(def.krate).as_str().starts_with("ambre_")
}

/// The name of a primitive the rule names, when `ty` is one of them by
/// itself: a scalar, a `str`, a `String` or a `Uuid`.
pub fn primitive_name(tcx: TyCtxt<'_>, ty: Ty<'_>) -> Option<String> {
    match ty.kind() {
        ty::Bool => Some("bool".to_owned()),
        ty::Char => Some("char".to_owned()),
        ty::Str => Some("str".to_owned()),
        ty::Int(int) => Some(int.name_str().to_owned()),
        ty::Uint(uint) => Some(uint.name_str().to_owned()),
        ty::Float(float) => Some(float.name_str().to_owned()),
        ty::Adt(adt, _) => {
            let krate = tcx.crate_name(adt.did().krate);
            let name = tcx.item_name(adt.did());
            let string = name.as_str() == "String" && ["alloc", "std"].contains(&krate.as_str());
            // The `Uuid` of `ambre-core` is the workspace's own spelling of a
            // UUID, which the rule names as one primitive with the crate's.
            let uuid = name.as_str() == "Uuid" && ["uuid", "ambre_core"].contains(&krate.as_str());
            (string || uuid).then(|| name.to_string())
        }
        _ => None,
    }
}

/// What one walk of a type found.
#[derive(Default)]
pub struct Facts {
    /// Each primitive the type names.
    pub primitives: BTreeSet<String>,
    /// Whether the type holds a `bool` outside the signature of a callable.
    pub holds_bool: bool,
    /// Whether the type names a callable: a function pointer, or a closure
    /// trait behind `dyn`, `impl` or a bound.
    pub callable: bool,
    /// Whether the type names a type or a trait of a foreign crate, outside
    /// the toolchain.
    pub foreign: bool,
}

/// The walk, with the generic parameters whose bounds it opens: those of an
/// `impl Trait` parameter.
pub struct Walk<'tcx> {
    tcx: TyCtxt<'tcx>,
    /// The item whose predicates bound a synthetic parameter.
    owner: Option<DefId>,
    pub facts: Facts,
    /// Depth inside a callable signature: a `bool` there is not held.
    callable_depth: usize,
    /// The opaque types and parameters already opened, against a cycle.
    seen: BTreeSet<String>,
    /// The type the source spells `Self`, read as no primitive: the impl
    /// header chose it, and the header is where the rule reads it.
    pub erase: Option<Ty<'tcx>>,
}

impl<'tcx> Walk<'tcx> {
    pub fn new(tcx: TyCtxt<'tcx>, owner: Option<DefId>) -> Self {
        Self {
            tcx,
            owner,
            facts: Facts::default(),
            callable_depth: 0,
            seen: BTreeSet::new(),
            erase: None,
        }
    }

    /// Walks `ty` whole.
    pub fn ty(&mut self, ty: Ty<'tcx>) {
        if self.erase == Some(ty) {
            return;
        }
        if let Some(name) = primitive_name(self.tcx, ty) {
            if name == "bool" && self.callable_depth == 0 {
                self.facts.holds_bool = true;
            }
            self.facts.primitives.insert(name);
        }
        match ty.kind() {
            ty::Adt(adt, args) => {
                self.def(adt.did());
                // A default argument is no argument the source writes.
                let written = self
                    .tcx
                    .generics_of(adt.did())
                    .own_args_no_defaults(self.tcx, args.as_slice());
                for arg in written {
                    if let Some(ty) = arg.as_type() {
                        self.ty(ty);
                    }
                }
            }
            ty::Ref(_, inner, _) | ty::Slice(inner) | ty::Array(inner, _) => self.ty(*inner),
            ty::RawPtr(inner, _) => self.ty(*inner),
            ty::Tuple(items) => {
                for item in items.iter() {
                    self.ty(item);
                }
            }
            ty::FnPtr(sig, _) => {
                self.facts.callable = true;
                self.callable_depth += 1;
                for item in sig.skip_binder().inputs_and_output.iter() {
                    self.ty(item);
                }
                self.callable_depth -= 1;
            }
            ty::Dynamic(predicates, _) => {
                for predicate in predicates.iter() {
                    match predicate.skip_binder() {
                        ty::ExistentialPredicate::Trait(t) => {
                            self.trait_args(t.def_id, t.args);
                        }
                        ty::ExistentialPredicate::Projection(p) => {
                            let callable = self.is_fn_trait(self.tcx.parent(p.def_id));
                            self.enter(callable);
                            self.args(p.args);
                            if let Some(term) = p.term.as_type() {
                                self.ty(term);
                            }
                            self.leave(callable);
                        }
                        ty::ExistentialPredicate::AutoTrait(def) => self.def(def),
                    }
                }
            }
            ty::Alias(
                _,
                ty::AliasTy {
                    kind: ty::Opaque { def_id },
                    ..
                },
            ) => {
                let key = format!("{def_id:?}");
                if self.seen.insert(key) {
                    let bounds = self.tcx.explicit_item_bounds(*def_id);
                    for (clause, _) in bounds
                        .iter_identity_copied()
                        .map(|c| c.skip_normalization())
                    {
                        self.clause(clause);
                    }
                }
            }
            // The return of an `impl Trait` in a trait reads as its bounds.
            ty::Alias(
                _,
                ty::AliasTy {
                    kind: ty::Projection { def_id },
                    ..
                },
            ) if self.tcx.is_impl_trait_in_trait(*def_id) => {
                let key = format!("{def_id:?}");
                if self.seen.insert(key) {
                    let bounds = self.tcx.explicit_item_bounds(*def_id);
                    for (clause, _) in bounds
                        .iter_identity_copied()
                        .map(|c| c.skip_normalization())
                    {
                        self.clause(clause);
                    }
                }
            }
            // A projection names its trait's arguments, never as a value:
            // `<T as TryFrom<String>>::Error` holds no `String`.
            ty::Alias(_, alias) => {
                if let Some(self_ty) = alias.args.types().next() {
                    self.ty(self_ty);
                }
            }
            ty::Param(param) => {
                let Some(owner) = self.owner else {
                    return;
                };
                let generics = self.tcx.generics_of(owner);
                let synthetic = (param.index as usize) < generics.count()
                    && generics.type_param(*param, self.tcx).kind.is_synthetic();
                if synthetic && self.seen.insert(format!("param {}", param.index)) {
                    for (clause, _) in self.tcx.clauses_of(owner).instantiate_identity(self.tcx) {
                        let clause = clause.skip_normalization();
                        let mentions = clause
                            .as_trait_clause()
                            .is_some_and(|t| t.skip_binder().self_ty() == ty)
                            || clause
                                .as_projection_clause()
                                .is_some_and(|p| p.skip_binder().self_ty() == ty);
                        if mentions {
                            self.clause(clause);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn enter(&mut self, callable: bool) {
        if callable {
            self.facts.callable = true;
            self.callable_depth += 1;
        }
    }

    fn leave(&mut self, callable: bool) {
        if callable {
            self.callable_depth -= 1;
        }
    }

    fn is_fn_trait(&self, def: DefId) -> bool {
        self.tcx.fn_trait_kind_from_def_id(def).is_some()
    }

    fn def(&mut self, def: DefId) {
        let krate = self.tcx.crate_name(def.krate);
        if !is_workspace(self.tcx, def) && !TOOLCHAIN.contains(&krate.as_str()) {
            self.facts.foreign = true;
        }
    }

    fn trait_args(&mut self, def: DefId, args: ty::GenericArgs<'tcx>) {
        self.def(def);
        let callable = self.is_fn_trait(def);
        self.enter(callable);
        self.args(args);
        self.leave(callable);
    }

    fn args(&mut self, args: ty::GenericArgs<'tcx>) {
        for arg in args.iter() {
            if let Some(ty) = arg.as_type() {
                self.ty(ty);
            }
        }
    }

    fn clause(&mut self, clause: ty::Clause<'tcx>) {
        if let Some(t) = clause.as_trait_clause() {
            let t = t.skip_binder();
            self.def(t.def_id());
            let callable = self.is_fn_trait(t.def_id());
            self.enter(callable);
            // The first argument is the bounded type itself.
            for arg in t.trait_ref.args.iter().skip(1) {
                if let Some(ty) = arg.as_type() {
                    self.ty(ty);
                }
            }
            self.leave(callable);
        } else if let Some(p) = clause.as_projection_clause() {
            let p = p.skip_binder();
            let item = p.projection_term.expect_projection_def_id();
            let callable = self.is_fn_trait(self.tcx.parent(item));
            self.enter(callable);
            for arg in p.projection_term.args.iter().skip(1) {
                if let Some(ty) = arg.as_type() {
                    self.ty(ty);
                }
            }
            if let Some(term) = p.term.as_type() {
                self.ty(term);
            }
            self.leave(callable);
        }
    }
}

/// The facts of `ty`, whose `impl Trait` parameters `owner` bounds.
pub fn facts<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>, owner: Option<DefId>) -> Facts {
    facts_erasing(tcx, ty, owner, None)
}

/// The facts of `ty`, with `erase` read as no primitive.
pub fn facts_erasing<'tcx>(
    tcx: TyCtxt<'tcx>,
    ty: Ty<'tcx>,
    owner: Option<DefId>,
    erase: Option<Ty<'tcx>>,
) -> Facts {
    let mut walk = Walk::new(tcx, owner);
    walk.erase = erase;
    walk.ty(ty);
    walk.facts
}

/// Whether `ty` is a primitive of the census list of `ambre-spec-newtypes.md`
/// section 1.
///
/// * a scalar, a `String`, a `Uuid` or a `Vec<u8>`;
/// * a `str` or a `[u8]` behind a reference or a `Cow`, or a bare `[u8]`;
/// * an `Option` of any of these.
pub fn census_primitive<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> bool {
    if primitive_name(tcx, ty).is_some_and(|name| name != "str") {
        return true;
    }
    match ty.kind() {
        ty::Ref(_, inner, _) => text_or_bytes(*inner),
        ty::Slice(_) => text_or_bytes(ty),
        ty::Adt(adt, args) => {
            let name = tcx.item_name(adt.did());
            let first = args.types().next();
            match (name.as_str(), first) {
                ("Vec", Some(inner)) => is_u8(inner),
                ("Cow", Some(inner)) => text_or_bytes(inner),
                ("Option", Some(inner)) => census_primitive(tcx, inner),
                _ => false,
            }
        }
        _ => false,
    }
}

fn is_u8(ty: Ty<'_>) -> bool {
    matches!(ty.kind(), ty::Uint(ty::UintTy::U8))
}

fn text_or_bytes(ty: Ty<'_>) -> bool {
    match ty.kind() {
        ty::Str => true,
        ty::Slice(inner) => is_u8(*inner),
        _ => false,
    }
}
