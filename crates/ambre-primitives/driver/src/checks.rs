//! The checks that read an item whole.
//!
//! | Check | Row |
//! | :--- | :--- |
//! | the newtype census | `ARCH-062` |
//! | the private field of a tuple struct | `NEWTYPE-001` |
//! | a `Deref` to a primitive | `ARCH-037` |
//! | an alias of a family | `ARCH-066` |
//! | the representation a generic newtype holds | `ARCH-036` |
//! | the names that read or wipe a secret | `ARCH-045` to `ARCH-049` |

use std::path::Path;

use rustc_hir as hir;
use rustc_hir::def::{CtorKind, DefKind};
use rustc_hir::def_id::{DefId, LocalDefId};
use rustc_middle::ty::{self, BottomUpFolder, Ty, TyCtxt, TypeFoldable};
use rustc_span::Span;

use crate::Gate;
use crate::place::{self, SEPARATOR, Written};
use crate::sites::held_fields;
use crate::types;

/// The names that read or copy the value of a `Secret` (`ARCH-045`).
const READERS: &[&str] = &["expose", "expose_mut", "clone_secret"];

/// The names the `zeroize` crate gives (`ARCH-046`).
const WIPERS: &[&str] = &["zeroize", "Zeroize", "ZeroizeOnDrop", "Zeroizing"];

/// The crate that declares the families, as the compiler names it.
const CORE: &str = "ambre_core";

/// The file of the Secret module, from the workspace root.
const SECRET_MODULE: &str = "crates/ambre-core/src/secret.rs";

/// The crate whose code reads the value of a secret.
const CRYPTO: &str = "crates/ambre-crypto/";

/// The relay's secrets module (`ARCH-048`).
const RELAY_SECRETS: &str = "crates/ambre-server/src/secrets/";

/// The SFU's secrets module: the TURN secret and the admission (`ARCH-048`, `CALL-014`).
const SFU_SECRETS: &str = "crates/ambre-sfu/src/secrets/";

/// Each sink that lends a secret to one foreign call, with the files that may
/// name it (`ARCH-049`). One file makes the call. A sink of the relay adds the
/// file of `src/secrets/` that declares it.
const SINKS: &[(&str, &[&str])] = &[
    ("present_bearer", &["crates/ambre-sync/src/http.rs"]),
    ("present_link", &["crates/ambre-sync/src/http.rs"]),
    ("present_state", &["crates/ambre-app/src/signin.rs"]),
    (
        "refresh_key",
        &[
            "crates/ambre-server/src/redis_ops.rs",
            "crates/ambre-server/src/secrets/token.rs",
        ],
    ),
    (
        "oidc_state_key",
        &[
            "crates/ambre-server/src/redis_ops.rs",
            "crates/ambre-server/src/secrets/token.rs",
        ],
    ),
    (
        "present_stored",
        &[
            "crates/ambre-server/src/redis_ops.rs",
            "crates/ambre-server/src/secrets/oidc.rs",
        ],
    ),
    (
        "to_client_secret",
        &[
            "crates/ambre-server/src/oidc.rs",
            "crates/ambre-server/src/secrets/oidc.rs",
        ],
    ),
    (
        "to_pkce_verifier",
        &[
            "crates/ambre-server/src/oidc.rs",
            "crates/ambre-server/src/secrets/oidc.rs",
        ],
    ),
    (
        "authorization_code",
        &[
            "crates/ambre-server/src/oidc.rs",
            "crates/ambre-server/src/secrets/oidc.rs",
        ],
    ),
    (
        "decode_access",
        &[
            "crates/ambre-server/src/jwt.rs",
            "crates/ambre-server/src/secrets/token.rs",
        ],
    ),
    (
        "seed_key",
        &[
            "crates/ambre-server/src/jwt.rs",
            "crates/ambre-server/src/secrets/seed.rs",
        ],
    ),
];

impl Gate {
    /// The census line of a struct of one field over a census primitive. Also
    /// the refusal of a public field of a tuple struct.
    pub fn struct_checks(&mut self, tcx: TyCtxt<'_>, item: &hir::Item<'_>, def: LocalDefId) {
        let adt = tcx.adt_def(def);
        let variant = adt.non_enum_variant();
        if variant.ctor_kind() == Some(CtorKind::Fn) {
            for field in &variant.fields {
                let Some(local) = field.did.as_local() else {
                    continue;
                };
                let hir::Node::Field(hir_field) = tcx.hir_node_by_def_id(local) else {
                    continue;
                };
                if place::written_vis(tcx, hir_field.vis_span) != Written::Private {
                    let at = place::position(tcx, &self.unit, hir_field.span);
                    self.refuse(
                        "pub-field",
                        format!(
                            "{}: the tuple struct {} holds a field that is not private. A \
                             newtype hands its value out by a named accessor such as get() \
                             or into_inner(), never by its field",
                            at.at(),
                            tcx.def_path_str(def.to_def_id())
                        ),
                    );
                }
            }
        }
        let family = tcx.crate_name(def.to_def_id().krate).as_str() == CORE
            && tcx
                .generics_of(def)
                .own_params
                .iter()
                .any(|p| !matches!(p.kind, ty::GenericParamDefKind::Lifetime));
        if family || held_fields(tcx, adt) != 1 {
            return;
        }
        let Some(field) = adt.all_fields().find(|f| {
            !tcx.type_of(f.did)
                .instantiate_identity()
                .skip_normalization()
                .is_phantom_data()
        }) else {
            return;
        };
        let ty = tcx
            .type_of(field.did)
            .instantiate_identity()
            .skip_normalization();
        if !census_instance(tcx, def, ty) {
            return;
        }
        let at = place::position(tcx, &self.unit, item.span);
        let Some(file) = at.file.clone() else {
            return;
        };
        let span = field
            .did
            .as_local()
            .and_then(|l| match tcx.hir_node_by_def_id(l) {
                hir::Node::Field(f) => Some(f.ty.span),
                _ => None,
            })
            .unwrap_or(item.span);
        let text = place::written(tcx, span).unwrap_or_else(|| place::short(&ty.to_string()));
        let mut name = fn_scope(tcx, def);
        name.push(tcx.item_name(def.to_def_id()).to_string());
        let key = [
            self.unit.krate.clone(),
            file,
            place::module_path(tcx, def),
            name.join("::"),
            text,
        ]
        .join(SEPARATOR);
        self.record(&["newtype", &key, &at.at()]);
    }

    /// The record of the associated type a struct of one field holds as that
    /// field, `<P as Trait>::Name` over its own parameter `P` (`ARCH-036`).
    pub fn representation(&mut self, tcx: TyCtxt<'_>, def: LocalDefId) {
        let adt = tcx.adt_def(def);
        if held_fields(tcx, adt) != 1 || in_fn_body(tcx, def) {
            return;
        }
        let Some(field) = adt.all_fields().find(|f| {
            !tcx.type_of(f.did)
                .instantiate_identity()
                .skip_normalization()
                .is_phantom_data()
        }) else {
            return;
        };
        let ty = tcx
            .type_of(field.did)
            .instantiate_identity()
            .skip_normalization();
        if let ty::Alias(_, alias) = ty.kind()
            && let ty::Projection { def_id } = alias.kind
            && matches!(alias.self_ty().kind(), ty::Param(_))
        {
            let trait_def = tcx.parent(def_id);
            let path = format!(
                "{}::{}",
                place::stable_path(tcx, trait_def),
                tcx.item_name(def_id)
            );
            self.record(&["repr", &path]);
        }
    }

    /// Records a generic struct of `ambre-core`, one the families table of
    /// `ambre-spec-newtypes.md` may name.
    pub fn core_generic(&mut self, tcx: TyCtxt<'_>, def: LocalDefId) {
        let generic = tcx
            .generics_of(def)
            .own_params
            .iter()
            .any(|p| !matches!(p.kind, ty::GenericParamDefKind::Lifetime));
        if generic && tcx.crate_name(def.to_def_id().krate).as_str() == CORE {
            let name = tcx.item_name(def.to_def_id()).to_string();
            self.record(&["generic", &name]);
        }
    }

    /// The record of a module-level alias whose target, references set
    /// apart, is an instance of a generic struct of `ambre-core` (`ARCH-066`).
    pub fn alias_check(&mut self, tcx: TyCtxt<'_>, item: &hir::Item<'_>, def: LocalDefId) {
        if !matches!(tcx.def_kind(tcx.local_parent(def)), DefKind::Mod) {
            return;
        }
        let mut target = tcx.type_of(def).instantiate_identity().skip_normalization();
        while let ty::Ref(_, inner, _) = target.kind() {
            target = *inner;
        }
        let ty::Adt(adt, _) = target.kind() else {
            return;
        };
        if tcx.crate_name(adt.did().krate).as_str() != CORE
            || !tcx
                .generics_of(adt.did())
                .own_params
                .iter()
                .any(|p| !matches!(p.kind, ty::GenericParamDefKind::Lifetime))
        {
            return;
        }
        let at = place::position(tcx, &self.unit, item.span);
        let family = tcx.item_name(adt.did()).to_string();
        let name = tcx.item_name(def.to_def_id()).to_string();
        self.record(&["alias", &family, &name, &at.at()]);
    }

    /// The refusal of a `Deref` or a `DerefMut` impl whose target hands out a
    /// primitive (`ARCH-037`).
    pub fn deref_check(&mut self, tcx: TyCtxt<'_>, item: &hir::Item<'_>, def: LocalDefId) {
        let Some(trait_ref) = tcx.impl_opt_trait_ref(def) else {
            return;
        };
        let trait_ref = trait_ref.instantiate_identity().skip_normalization();
        let items = tcx.lang_items();
        let deref = items.deref_trait() == Some(trait_ref.def_id);
        let deref_mut = items.deref_mut_trait() == Some(trait_ref.def_id);
        if !deref && !deref_mut {
            return;
        }
        let self_ty = tcx.type_of(def).instantiate_identity().skip_normalization();
        let target = match items.deref_target() {
            Some(target_def) if deref => tcx
                .associated_items(def)
                .in_definition_order()
                .find(|assoc| assoc.trait_item_def_id() == Some(target_def))
                .map(|assoc| {
                    tcx.type_of(assoc.def_id)
                        .instantiate_identity()
                        .skip_normalization()
                }),
            Some(target_def) => {
                let projection = Ty::new_projection(tcx, ty::IsRigid::No, target_def, [self_ty]);
                let env = ty::TypingEnv::non_body_analysis(tcx, def);
                tcx.try_normalize_erasing_regions(env, ty::Unnormalized::new_wip(projection))
                    .ok()
            }
            None => None,
        };
        let at = place::position(tcx, &self.unit, item.span);
        let which = if deref { "Deref" } else { "DerefMut" };
        match target {
            Some(target) if !exposes(tcx, target) => {}
            Some(target) => self.refuse(
                "deref",
                format!(
                    "{}: {self_ty} implements {which} to {target}, which hands out a primitive. \
                     A newtype never dereferences to its primitive, and no line of the \
                     allowlist admits one (ARCH-037)",
                    at.at()
                ),
            ),
            None => self.refuse(
                "deref",
                format!(
                    "{}: the gate cannot read the target of the {which} of {self_ty}, and fails \
                     closed (ARCH-037)",
                    at.at()
                ),
            ),
        }
    }

    /// Refuses `name` at `span` when the secret checks forbid it there. `def`
    /// is what the name resolves to, if anything.
    pub fn name_check(&mut self, tcx: TyCtxt<'_>, name: &str, def: Option<DefId>, span: Span) {
        let reader = READERS.contains(&name);
        let wiper = WIPERS.contains(&name)
            || def.is_some_and(|d| tcx.crate_name(d.krate).as_str() == "zeroize");
        let sink = SINKS.iter().find(|(sink, _)| *sink == name);
        if !reader && !wiper && sink.is_none() {
            return;
        }
        let at = place::position(tcx, &self.unit, span);
        let files = shown_paths(&self.unit.cwd, &at.path);
        let all = |allowed: &dyn Fn(&str) -> bool| files.iter().all(|f| allowed(f));
        let in_crypto = |f: &str| f.starts_with(CRYPTO);
        let in_secret = |f: &str| f == SECRET_MODULE;
        let in_server = |f: &str| f.starts_with(RELAY_SECRETS) || f.starts_with(SFU_SECRETS);
        if reader && !all(&|f| in_crypto(f) || in_secret(f) || in_server(f)) {
            self.refuse(
                "secret",
                format!(
                    "{}: {name} reads a secret outside ambre-crypto, the Secret module and the \
                     src/secrets/ of the relay and of the SFU (ARCH-045, ARCH-048). Move the \
                     read into ambre-crypto, \
                     and hold the value in an ambre_core::Secret",
                    at.at()
                ),
            );
        }
        if wiper && !all(&in_secret) {
            self.refuse(
                "zeroize",
                format!(
                    "{}: {name} names the zeroize crate outside the Secret module of ambre-core \
                     (ARCH-046). Hold the value in an ambre_core::Secret, which wipes it",
                    at.at()
                ),
            );
        }
        if let Some((_, allowed)) = sink
            && !all(&|f| in_crypto(f) || in_secret(f) || allowed.contains(&f))
        {
            self.refuse(
                "sink",
                format!(
                    "{}: the sink {name} is named outside the file of its foreign call (ARCH-049)",
                    at.at()
                ),
            );
        }
    }
}

/// The path of a file from the workspace root, once with each `..` undone and
/// once with each link followed. An exemption holds only when both sit in it.
pub fn shown_paths(root: &Path, path: &Path) -> Vec<String> {
    let lexical = place::lexical(path);
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| lexical.clone());
    let root_canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    [
        lexical.strip_prefix(root).map(Path::to_path_buf),
        canonical
            .strip_prefix(&root_canonical)
            .map(Path::to_path_buf),
    ]
    .into_iter()
    .map(|p| {
        p.map_or_else(
            |_| String::new(),
            |p| p.to_string_lossy().replace('\\', "/"),
        )
    })
    .collect()
}

/// Whether a dereference target hands out a primitive. A primitive does, and
/// so does a reference, a slice, an array or a `Vec`, `Box`, `Rc`, `Arc` or
/// `Cow` of one. A type parameter or a projection fails closed.
fn exposes<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> bool {
    if types::primitive_name(tcx, ty).is_some() {
        return true;
    }
    match ty.kind() {
        ty::Ref(_, inner, _) | ty::Slice(inner) | ty::Array(inner, _) => exposes(tcx, *inner),
        ty::Param(_) | ty::Alias(..) => true,
        ty::Adt(adt, args) => {
            let name = tcx.item_name(adt.did());
            let toolchain = types::TOOLCHAIN.contains(&tcx.crate_name(adt.did().krate).as_str());
            toolchain
                && ["Vec", "Box", "Rc", "Arc", "Cow"].contains(&name.as_str())
                && args.types().next().is_some_and(|inner| exposes(tcx, inner))
        }
        _ => false,
    }
}

/// Whether `ty`, the one field of the struct `def`, holds a primitive of the
/// census list in some instance (`ARCH-062`). A field over an associated type
/// `<P as Trait>::Name` of a parameter `P` resolves to no type on its own. So
/// the census reads it as each type that `Name` takes in one of two places:
///
/// * an impl of `Trait` that this compile sees;
/// * an equality bound of the struct, as `P: Trait<Name = u32>`.
///
/// An impl in a crate downstream of the struct is out of this compile. A field
/// over a bare type parameter fails closed, as `over_bare_parameter` states.
fn census_instance<'tcx>(tcx: TyCtxt<'tcx>, def: LocalDefId, ty: Ty<'tcx>) -> bool {
    if types::census_primitive(tcx, ty) || over_bare_parameter(tcx, ty) {
        return true;
    }
    let projections: Vec<Ty<'tcx>> = ty
        .walk()
        .filter_map(|arg| arg.as_type())
        .filter(|held| {
            matches!(held.kind(), ty::Alias(_, alias)
                if matches!(alias.kind, ty::Projection { .. })
                    && matches!(alias.self_ty().kind(), ty::Param(_)))
        })
        .collect();
    projections.into_iter().any(|projection| {
        instances(tcx, def, projection).into_iter().any(|instance| {
            let read = ty.fold_with(&mut BottomUpFolder {
                tcx,
                ty_op: |t| if t == projection { instance } else { t },
                lt_op: |r| r,
                ct_op: |c| c,
            });
            types::census_primitive(tcx, read)
        })
    })
}

/// Whether `ty` holds a primitive of the census list once a primitive stands
/// for one of its bare type parameters (`ARCH-062`). An instance `S<u32>` can
/// sit in any crate downstream of the struct, so the census cannot list the
/// instances. It fails closed: a parameter a primitive can stand for counts.
/// A parameter under an associated type is the case of `instances`, and a
/// primitive there names no type, so this reading leaves it alone.
fn over_bare_parameter<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> bool {
    let stand_ins = [
        tcx.types.u8,
        tcx.types.str_,
        Ty::new_slice(tcx, tcx.types.u8),
    ];
    ty.walk()
        .filter_map(|arg| arg.as_type())
        .filter(|held| matches!(held.kind(), ty::Param(_)))
        .any(|param| {
            stand_ins.iter().any(|&stand_in| {
                let read = ty.fold_with(&mut BottomUpFolder {
                    tcx,
                    ty_op: |t| if t == param { stand_in } else { t },
                    lt_op: |r| r,
                    ct_op: |c| c,
                });
                types::census_primitive(tcx, read)
            })
        })
}

/// The types the associated type `projection` takes. Each impl of its trait
/// in this compile gives one, and so does each equality bound of the struct
/// `def` on the same parameter.
fn instances<'tcx>(tcx: TyCtxt<'tcx>, def: LocalDefId, projection: Ty<'tcx>) -> Vec<Ty<'tcx>> {
    let ty::Alias(_, alias) = projection.kind() else {
        return Vec::new();
    };
    let ty::Projection { def_id: name } = alias.kind else {
        return Vec::new();
    };
    let from_impls = tcx.all_impls(tcx.parent(name)).filter_map(|imp| {
        tcx.associated_items(imp)
            .in_definition_order()
            .find(|assoc| assoc.trait_item_def_id() == Some(name))
            .map(|assoc| {
                tcx.type_of(assoc.def_id)
                    .instantiate_identity()
                    .skip_normalization()
            })
    });
    let from_bounds = tcx
        .explicit_clauses_of(def)
        .clauses
        .iter()
        .filter_map(|(clause, _)| clause.as_projection_clause())
        .filter_map(|bound| {
            let bound = bound.skip_binder();
            (matches!(bound.projection_term.kind,
                ty::AliasTermKind::ProjectionTy { def_id } if def_id == name)
                && bound.projection_term.self_ty() == alias.self_ty())
            .then(|| bound.term.as_type())
            .flatten()
        });
    from_impls.chain(from_bounds).collect()
}

/// The names of the `fn` items around `def`, from the outermost, as the census
/// key writes its struct.
fn fn_scope(tcx: TyCtxt<'_>, def: LocalDefId) -> Vec<String> {
    let module = tcx.parent_module_from_def_id(def).to_local_def_id();
    let mut names = Vec::new();
    let mut at = tcx.local_parent(def);
    while at != module {
        if matches!(tcx.def_kind(at), DefKind::Fn | DefKind::AssocFn) {
            names.push(tcx.item_name(at.to_def_id()).to_string());
        }
        at = tcx.local_parent(at);
    }
    names.reverse();
    names
}

/// Whether `def` sits in the body of a `fn`.
fn in_fn_body(tcx: TyCtxt<'_>, def: LocalDefId) -> bool {
    !fn_scope(tcx, def).is_empty()
}
