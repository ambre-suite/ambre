//! The sites of `ARCH-033`, with the facts the allowlist judge reads from the
//! code (`ARCH-036` to `ARCH-040`). A site is a field, a parameter, a return,
//! a constant, a static, an alias or an associated type of an impl. Its type
//! holds a primitive.
//!
//! A site record is one line:
//! `site  key  privacy  shape  role  inner  trait  slot  origin  at`.
//! `inner` and `trait` read `ok`, `repr:<trait>::<name>` for an associated
//! type the judge places on a generic newtype, or the reason the claim fails.

use std::collections::BTreeSet;

use rustc_hir as hir;
use rustc_hir::def::DefKind;
use rustc_hir::def_id::{DefId, LocalDefId};
use rustc_middle::ty::{self, Ty, TyCtxt};
use rustc_span::Span;

use crate::Gate;
use crate::place::{self, SEPARATOR, Written};
use crate::types::{self, Facts};

/// Who reaches a site (`ARCH-038`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Privacy {
    Private,
    Restricted,
    Public,
    TraitImpl,
    BusType,
    Serialised,
}

impl Privacy {
    fn word(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Restricted => "restricted",
            Self::Public => "public",
            Self::TraitImpl => "trait-impl",
            Self::BusType => "bus",
            Self::Serialised => "serialised",
        }
    }

    fn of(written: Written) -> Self {
        match written {
            Written::Private => Self::Private,
            Written::Restricted => Self::Restricted,
            Written::Public => Self::Public,
        }
    }
}

/// What a site declares, the fifth field of its key.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Field,
    Param,
    Ret,
    Const,
    Static,
    Alias,
    Assoc,
}

impl Kind {
    fn word(self) -> &'static str {
        match self {
            Self::Field => "field",
            Self::Param => "param",
            Self::Ret => "ret",
            Self::Const => "const",
            Self::Static => "static",
            Self::Alias => "alias",
            Self::Assoc => "assoc",
        }
    }
}

/// One site, before it is written.
pub struct Site<'tcx> {
    /// The item whose scope names the site.
    pub scope: LocalDefId,
    pub leaf: String,
    pub kind: Kind,
    pub ty: Ty<'tcx>,
    /// The span of the written type.
    pub span: Span,
    /// The item whose predicates bound an `impl Trait` parameter.
    pub owner: Option<DefId>,
    pub privacy: Privacy,
    /// Whether the site is the one field of a tuple variant of one field.
    pub variant_value: bool,
    pub inner: String,
    pub trait_claim: String,
    pub slot: String,
}

impl Gate {
    /// Writes `site` when its type holds a primitive.
    pub fn site<'tcx>(&mut self, tcx: TyCtxt<'tcx>, site: Site<'tcx>) {
        let erase = spelled_self(tcx, site.scope, site.span);
        let facts = types::facts_erasing(tcx, site.ty, site.owner, erase);
        if facts.primitives.is_empty() {
            return;
        }
        let at = place::position(tcx, &self.unit, site.span);
        let Some(file) = at.file.clone() else {
            self.refuse(
                "outside",
                format!(
                    "{}: a site of {} sits in a file outside its package, so no key can name it",
                    at.at(),
                    self.unit.krate
                ),
            );
            return;
        };
        let text =
            place::written(tcx, site.span).unwrap_or_else(|| place::short(&site.ty.to_string()));
        let key = [
            self.unit.krate.clone(),
            file,
            place::module_path(tcx, site.scope),
            place::item(tcx, site.scope, &site.leaf),
            site.kind.word().to_owned(),
            text,
        ]
        .join(SEPARATOR);
        let shape = if site.ty.is_bool() {
            "bool"
        } else if facts.holds_bool {
            "holds-bool"
        } else {
            "other"
        };
        let role = if site.variant_value {
            "variant-value"
        } else {
            "member"
        };
        let trait_claim = match site.trait_claim.as_str() {
            "callable" => callable_claim(&facts),
            other => other.to_owned(),
        };
        let trait_claim = match trait_claim.strip_prefix("chosen:") {
            Some(chosen) => chosen_claim(&facts, chosen),
            None => trait_claim,
        };
        let origin = place::origin(tcx, site.span).word();
        self.record(&[
            "site",
            &key,
            site.privacy.word(),
            shape,
            role,
            &site.inner,
            &trait_claim,
            &site.slot,
            &origin,
            &at.at(),
        ]);
    }

    /// The sites of the fields of the struct, enum or union `def`.
    pub fn adt_sites(&mut self, tcx: TyCtxt<'_>, def: LocalDefId, vis: Written) {
        let adt = tcx.adt_def(def);
        let holder = if self.unit.krate == "ambre-bus" {
            Privacy::BusType
        } else if self.serialised.contains(&def.to_def_id()) {
            Privacy::Serialised
        } else {
            Privacy::Private
        };
        let base = if adt.is_enum() {
            Privacy::of(vis)
        } else {
            Privacy::Private
        };
        let name = tcx.item_name(def.to_def_id()).to_string();
        let held = held_fields(tcx, adt);
        for variant in adt.variants() {
            let owner = if adt.is_enum() {
                format!("{name}::{}", variant.name)
            } else {
                name.clone()
            };
            let tuple_of_one = adt.is_enum()
                && variant.ctor_kind() == Some(hir::def::CtorKind::Fn)
                && variant.fields.len() == 1;
            for (index, field) in variant.fields.iter().enumerate() {
                let Some(local) = field.did.as_local() else {
                    continue;
                };
                let hir::Node::Field(hir_field) = tcx.hir_node_by_def_id(local) else {
                    continue;
                };
                let field_vis = Privacy::of(place::written_vis(tcx, hir_field.vis_span));
                let privacy = [field_vis, base, holder]
                    .into_iter()
                    .find(|p| *p != Privacy::Private)
                    .unwrap_or(Privacy::Private);
                let leaf_name = if variant.ctor_kind() == Some(hir::def::CtorKind::Fn) {
                    index.to_string()
                } else {
                    field.name.to_string()
                };
                let inner = if adt.is_union() {
                    "a field of a union, not of a struct or an enum".to_owned()
                } else if adt.is_enum() {
                    if tuple_of_one {
                        "ok".to_owned()
                    } else {
                        "a field of a variant with two fields or more, or with a named field"
                            .to_owned()
                    }
                } else if held == 1 {
                    "ok".to_owned()
                } else {
                    format!("a field of a struct of {held} fields, which is no newtype")
                };
                self.site(
                    tcx,
                    Site {
                        scope: def,
                        leaf: format!("{owner}.{leaf_name}"),
                        kind: Kind::Field,
                        ty: tcx.type_of(field.did).instantiate_identity(),
                        span: hir_field.ty.span,
                        owner: None,
                        privacy,
                        variant_value: tuple_of_one,
                        inner,
                        trait_claim: "callable".to_owned(),
                        slot: "-".to_owned(),
                    },
                );
            }
        }
    }

    /// The sites of the signature of the `fn` `def`.
    ///
    /// `component` names the Leptos component whose body `def` holds: its
    /// parameters are keyed under that name, and a prop reaches them by name.
    pub fn fn_sites_as(
        &mut self,
        tcx: TyCtxt<'_>,
        def: LocalDefId,
        privacy: Privacy,
        component: Option<&str>,
    ) {
        let Some(decl) = tcx.hir_node_by_def_id(def).fn_decl() else {
            return;
        };
        let sig = tcx.fn_sig(def).instantiate_identity().skip_binder();
        let name =
            component.map_or_else(|| tcx.item_name(def.to_def_id()).to_string(), str::to_owned);
        let names = param_names(tcx, def);
        let has_self = names.first().is_some_and(|n| n == "self");
        let host = Host::of(tcx, def);
        let stable = place::stable_path(tcx, def.to_def_id());
        let module = place::stable_path(tcx, tcx.parent(def.to_def_id()));
        for (index, (ty, hir_ty)) in sig.inputs().iter().zip(decl.inputs).enumerate() {
            if has_self && index == 0 {
                continue;
            }
            let position = index - usize::from(has_self);
            let param = names.get(index).cloned().unwrap_or_else(|| "_".to_owned());
            let inner = host.codec(tcx, Kind::Param);
            self.site(
                tcx,
                Site {
                    scope: def,
                    leaf: format!("{name}({param})"),
                    kind: Kind::Param,
                    ty: *ty,
                    span: hir_ty.span,
                    owner: Some(def.to_def_id()),
                    privacy,
                    variant_value: false,
                    inner,
                    trait_claim: host.trait_claim(tcx),
                    slot: match component {
                        Some(component) => format!("component:{module}::{component}:{param}"),
                        None => format!("{stable}#{position}"),
                    },
                },
            );
        }
        if let hir::FnRetTy::Return(ret) = decl.output {
            let inner = host.codec(tcx, Kind::Ret);
            self.site(
                tcx,
                Site {
                    scope: def,
                    leaf: format!("{name}()"),
                    kind: Kind::Ret,
                    ty: sig.output(),
                    span: ret.span,
                    owner: Some(def.to_def_id()),
                    privacy,
                    variant_value: false,
                    inner,
                    trait_claim: host.trait_claim(tcx),
                    slot: "-".to_owned(),
                },
            );
        }
    }

    /// The site of a constant, a static or an alias `def`, whose type the
    /// source writes at `span`.
    pub fn item_site(
        &mut self,
        tcx: TyCtxt<'_>,
        def: LocalDefId,
        kind: Kind,
        span: Span,
        privacy: Privacy,
    ) {
        let host = Host::of(tcx, def);
        let inner = match kind {
            Kind::Assoc => host.assoc(tcx, def),
            _ => format!("a {} is never the inside of a newtype", kind.word()),
        };
        let trait_claim = match kind {
            Kind::Assoc => {
                "an associated type, whose value the impl chooses, not the trait".to_owned()
            }
            _ => host.trait_claim(tcx),
        };
        self.site(
            tcx,
            Site {
                scope: def,
                leaf: tcx.item_name(def.to_def_id()).to_string(),
                kind,
                ty: tcx.type_of(def).instantiate_identity(),
                span,
                owner: None,
                privacy,
                variant_value: false,
                inner,
                trait_claim,
                slot: "-".to_owned(),
            },
        );
    }
}

/// The fields of `adt` that are no `PhantomData` marker, for a struct.
pub fn held_fields(tcx: TyCtxt<'_>, adt: ty::AdtDef<'_>) -> usize {
    adt.all_fields()
        .filter(|f| !tcx.type_of(f.did).instantiate_identity().is_phantom_data())
        .count()
}

/// The name of each parameter of `def`, as its pattern writes it.
fn param_names(tcx: TyCtxt<'_>, def: LocalDefId) -> Vec<String> {
    if let Some(body) = tcx.hir_maybe_body_owned_by(def) {
        return body
            .params
            .iter()
            .map(|param| match param.pat.kind {
                // An `async fn` moves each pattern into its body and binds
                // `__argN` in its place: the key keeps the written pattern.
                hir::PatKind::Binding(_, _, ident, None)
                    if !ident.as_str().starts_with("__arg") =>
                {
                    ident.to_string()
                }
                _ => place::written(tcx, param.pat.span).unwrap_or_else(|| "_".to_owned()),
            })
            .collect();
    }
    tcx.fn_arg_idents(def.to_def_id())
        .iter()
        .map(|ident| ident.map_or_else(|| "_".to_owned(), |i| i.to_string()))
        .collect()
}

/// The impl a signature, a constant or an associated type sits in directly.
struct Host {
    imp: Option<LocalDefId>,
    item: LocalDefId,
}

impl Host {
    fn of(tcx: TyCtxt<'_>, item: LocalDefId) -> Self {
        let parent = tcx.local_parent(item);
        let imp = matches!(tcx.def_kind(parent), DefKind::Impl { .. }).then_some(parent);
        Self { imp, item }
    }

    /// The claim of a `newtype-inner` line on a parameter or a return.
    fn codec(&self, tcx: TyCtxt<'_>, kind: Kind) -> String {
        let Some(imp) = self.imp else {
            return "a signature outside an impl, or nested in a body".to_owned();
        };
        let self_ty = tcx.type_of(imp).instantiate_identity();
        let ty::Adt(adt, _) = self_ty.kind() else {
            return format!(
                "a signature of an impl for {self_ty}, which is no type of the workspace"
            );
        };
        let own = adt.did().is_local();
        let trait_local = tcx
            .impl_opt_trait_ref(imp)
            .is_some_and(|t| t.skip_binder().def_id.is_local());
        if !(own || trait_local && types::is_workspace(tcx, adt.did())) {
            return format!(
                "a signature of an impl for {self_ty}, a type of another crate, through a trait \
                 of another crate"
            );
        }
        let representation = if adt.is_struct() {
            held_fields(tcx, *adt) == 1
        } else if adt.is_enum() {
            adt.variants().iter().all(|v| v.fields.is_empty())
        } else {
            false
        };
        if !representation {
            return format!(
                "{self_ty} is neither a struct of one field nor a fieldless enum, so it has no \
                 codec of its own"
            );
        }
        let sig = tcx.fn_sig(self.item).instantiate_identity().skip_binder();
        match kind {
            Kind::Param => {
                let out = sig.output();
                let itself = out == self_ty
                    || matches!(out.kind(), ty::Adt(wrap, args)
                        if ["Option", "Result"].contains(&tcx.item_name(wrap.did()).as_str())
                            && args.types().next() == Some(self_ty));
                if itself {
                    "ok".to_owned()
                } else {
                    format!(
                        "a parameter of a fn of {self_ty} that returns neither Self, Option<Self> \
                         nor Result<Self, _>"
                    )
                }
            }
            _ => {
                if tcx.associated_item(self.item.to_def_id()).is_method() {
                    "ok".to_owned()
                } else {
                    format!("the return of a fn of {self_ty} that takes no self")
                }
            }
        }
    }

    /// The claim of a `newtype-inner` line on an associated type: the judge
    /// places it on a generic newtype that holds it as its one field.
    fn assoc(&self, tcx: TyCtxt<'_>, def: LocalDefId) -> String {
        let Some(imp) = self.imp else {
            return "an associated type outside an impl of a trait".to_owned();
        };
        let Some(trait_ref) = tcx.impl_opt_trait_ref(imp) else {
            return "an associated type outside an impl of a trait".to_owned();
        };
        let trait_def = trait_ref.skip_binder().def_id;
        if !types::is_workspace(tcx, trait_def) {
            return format!(
                "the trait {} is not a trait of the workspace, so no generic newtype of the \
                 workspace reads its associated type",
                tcx.def_path_str(trait_def)
            );
        }
        let in_body = tcx
            .hir_parent_iter(tcx.local_def_id_to_hir_id(imp))
            .any(|(_, node)| matches!(node, hir::Node::Item(i) if matches!(i.kind, hir::ItemKind::Fn { .. })));
        if in_body {
            return "an impl in a fn body, whose trait a local item may shadow".to_owned();
        }
        if let hir::Node::Item(item) = tcx.hir_node_by_def_id(imp)
            && let hir::ItemKind::Impl(i) = item.kind
            && let Some(header) = i.of_trait
        {
            let path = header.trait_ref.path;
            let unplain = path.segments.iter().any(|s| {
                s.args
                    .is_some_and(|a| !a.args.is_empty() || !a.constraints.is_empty())
            }) || place::written(tcx, path.span).is_some_and(|t| t.starts_with("::"));
            if unplain {
                return format!(
                    "the trait {} carries generic arguments or a leading `::`, and no generic \
                     newtype reads such a trait",
                    tcx.def_path_str(trait_def)
                );
            }
        }
        format!(
            "repr:{}::{}",
            place::stable_path(tcx, trait_def),
            tcx.item_name(def.to_def_id())
        )
    }

    /// The claim of a `trait` line, settled once the type is walked:
    /// `ok`, a reason, `chosen:<primitives>` or `callable`.
    fn trait_claim(&self, tcx: TyCtxt<'_>) -> String {
        let Some(imp) = self.imp else {
            return "callable".to_owned();
        };
        let Some(trait_ref) = tcx.impl_opt_trait_ref(imp) else {
            return "callable".to_owned();
        };
        let trait_ref = trait_ref.instantiate_identity();
        if types::is_workspace(tcx, trait_ref.def_id) {
            return format!(
                "the trait {} is declared in the workspace, so its signature takes types",
                tcx.def_path_str(trait_ref.def_id)
            );
        }
        let mut chosen = BTreeSet::new();
        for arg in trait_ref.args.iter() {
            if let Some(ty) = arg.as_type() {
                chosen.extend(types::facts(tcx, ty, None).primitives);
            }
        }
        format!(
            "chosen:{};{}",
            chosen.into_iter().collect::<Vec<_>>().join(","),
            tcx.def_path_str(trait_ref.def_id)
        )
    }
}

/// The claim of a `trait` line outside a trait impl.
fn callable_claim(facts: &Facts) -> String {
    match (facts.callable, facts.foreign) {
        (true, true) => "ok".to_owned(),
        (true, false) => "a closure type that names no type of a foreign crate".to_owned(),
        (false, _) => "neither in an impl of a foreign trait nor a closure type".to_owned(),
    }
}

/// The claim of a `trait` line in an impl of a foreign trait, whose generic
/// arguments and self type name the primitives `chosen`.
fn chosen_claim(facts: &Facts, chosen: &str) -> String {
    let (list, name) = chosen.split_once(';').unwrap_or(("", chosen));
    let chosen: BTreeSet<&str> = list.split(',').filter(|p| !p.is_empty()).collect();
    let shared: Vec<&str> = facts
        .primitives
        .iter()
        .map(String::as_str)
        .filter(|p| chosen.contains(p))
        .collect();
    if shared.is_empty() {
        "ok".to_owned()
    } else {
        format!(
            "the impl writes {} as a generic argument of {name} or as the type it implements \
             for, so the workspace chose the primitive, not the trait: the site takes a type",
            shared.join(", ")
        )
    }
}

/// The self type of the impl around `def`, when the type at `span` spells it
/// `Self`. The source writes no primitive there, whatever the impl is for.
fn spelled_self<'tcx>(tcx: TyCtxt<'tcx>, def: LocalDefId, span: Span) -> Option<Ty<'tcx>> {
    let text = place::written(tcx, span)?;
    let spelled = text
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|word| word == "Self");
    if !spelled {
        return None;
    }
    let mut at = def;
    loop {
        if matches!(tcx.def_kind(at), DefKind::Impl { .. }) {
            return Some(tcx.type_of(at).instantiate_identity());
        }
        let parent = tcx.local_parent(at);
        if parent == at || matches!(tcx.def_kind(parent), DefKind::Mod) {
            return None;
        }
        at = parent;
    }
}
