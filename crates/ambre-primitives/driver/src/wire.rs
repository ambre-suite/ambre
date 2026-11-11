//! The fields of a message and of the JSON wire, read with their types.
//!
//! `crate_boundaries.py` read the text of a field's type. So an alias such
//! as `type SheetId = String` passed for a type of its own, and an
//! identifier the core family spells `Id<Snapshot>` passed for no
//! identifier. Here each field holds the type rustc resolves.
//!
//! | Check | Rows |
//! | :--- | :--- |
//! | an identifier field of a message held as a bare primitive | `BUS-029` |
//! | an identifier field of the JSON wire held as a `String`, a `str` or a `Uuid` | `SRV-179` |
//! | a core identifier a serde field writes as bytes, with no text adapter | `SRV-179` |
//! | a closed-set field of the JSON wire held as text | `SRV-180`, `SRV-182` |

use rustc_hir as hir;
use rustc_hir::def_id::LocalDefId;
use rustc_middle::ty::{self, Ty, TyCtxt};

use crate::Gate;
use crate::views::bare_payload;

/// The message vocabulary each application declares in its own crate
/// (`BUS-029`, `BUS-032`).
const APP_MESSAGES: &[&str] = &[
    "crates/ambre-trame/src/message",
    "crates/ambre-toile/src/message",
    "crates/ambre-prose/src/message",
    "crates/ambre-murmure/src/message",
];

/// The relay's rows and routes, its refusal, and the HTTP layer of Sync
/// (`SRV-179`, `SRV-180`, `SRV-182`).
fn wire_file(f: &str) -> bool {
    f.starts_with("crates/ambre-server/src/routes/")
        || f.starts_with("crates/ambre-server/src/db/")
        || f == "crates/ambre-server/src/db.rs"
        || f == "crates/ambre-sync/src/http.rs"
        || f == "crates/ambre-server/src/error.rs"
}

/// Whether a field name carries an identifier: `id`, `*_id` or `*_ids`.
fn id_name(name: &str) -> bool {
    name == "id" || name == "ids" || name.ends_with("_id") || name.ends_with("_ids")
}

/// Whether a field name carries a word of a closed set. A refusal's
/// `error` names its code (`SRV-182`).
fn closed_name(name: &str) -> bool {
    ["doc_type", "role", "kind", "status", "error"].contains(&name)
}

/// The text a type holds anywhere inside: a `String`, a `str` or a `Uuid`.
fn untyped<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<String> {
    ty.walk()
        .filter_map(|a| a.as_type())
        .find_map(|t| match t.kind() {
            ty::Str => Some("str".to_owned()),
            ty::Adt(adt, _) => {
                let name = tcx.item_name(adt.did()).to_string();
                let krate = tcx.crate_name(adt.did().krate);
                let text = name == "String" && ["alloc", "std"].contains(&krate.as_str());
                let uuid = name == "Uuid" && krate.as_str() == "uuid";
                (text || uuid).then_some(name)
            }
            _ => None,
        })
}

/// Whether a type holds an identifier of the core family anywhere inside.
fn core_id<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> bool {
    ty.walk().filter_map(|a| a.as_type()).any(|t| {
        matches!(t.kind(), ty::Adt(adt, _)
            if tcx.crate_name(adt.did().krate).as_str() == "ambre_core"
                && tcx.item_name(adt.did()).as_str().ends_with("Id"))
    })
}

impl Gate {
    /// Refuses an identifier field of a message held as a bare primitive
    /// (`BUS-029`).
    pub fn bus_ids(&mut self, tcx: TyCtxt<'_>, item: &hir::Item<'_>, def: LocalDefId) {
        if self.test
            || !matches!(
                item.kind,
                hir::ItemKind::Struct(..) | hir::ItemKind::Enum(..)
            )
        {
            return;
        }
        let span = tcx.def_span(def);
        let bus = self.unit.krate == "ambre-bus";
        if !bus && !self.in_file(tcx, span, |f| APP_MESSAGES.iter().any(|m| f.starts_with(m))) {
            return;
        }
        for field in tcx.adt_def(def).all_fields() {
            let name = field.name.to_string();
            if !id_name(&name) {
                continue;
            }
            let ty = tcx
                .type_of(field.did)
                .instantiate_identity()
                .skip_normalization();
            if let Some(bare) = bare_payload(tcx, ty) {
                let what = format!("the `{name}` field carries an identifier as a bare `{bare}`");
                self.wired(tcx, tcx.def_span(field.did), "BUS-029", &what);
            }
        }
    }

    /// Refuses a field of the JSON wire that holds an identifier or a word
    /// of a closed set as text (`SRV-179`, `SRV-180`, `SRV-182`). Refuses a
    /// serde field that writes a core identifier as bytes too.
    pub fn wire_fields(&mut self, tcx: TyCtxt<'_>, item: &hir::Item<'_>, def: LocalDefId) {
        if self.test {
            return;
        }
        let hir::ItemKind::Struct(_, _, hir::VariantData::Struct { fields, .. }) = item.kind else {
            return;
        };
        // A struct a derive writes, such as the wrapper serde builds for a
        // `with` adapter, is the derive's, and no struct of the wire.
        if !self.in_file(tcx, item.span, wire_file) || Self::foreign_item(tcx, def) {
            return;
        }
        let serde = self.serde.contains(&def.to_def_id());
        let mut before = item.kind.ident().map_or(item.span.lo(), |i| i.span.hi());
        for field in fields {
            let adapted = Self::text_adapter(tcx, before, field.span);
            before = field.span.hi();
            let name = field.ident.to_string();
            let ty = tcx
                .type_of(field.def_id)
                .instantiate_identity()
                .skip_normalization();
            let text = untyped(tcx, ty);
            if id_name(&name)
                && let Some(text) = &text
            {
                let what = format!("the `{name}` field carries an identifier as `{text}`");
                self.wired(tcx, field.span, "SRV-179", &what);
            } else if serde && core_id(tcx, ty) && !adapted {
                let what = format!(
                    "the `{name}` field writes its identifier as bytes, with no text adapter"
                );
                self.wired(tcx, field.span, "SRV-179", &what);
            } else if closed_name(&name)
                && let Some(text) = text.filter(|t| t != "Uuid")
            {
                let rows = if name == "error" {
                    "SRV-182"
                } else {
                    "SRV-180"
                };
                let what = format!("the `{name}` field carries a closed set as `{text}`");
                self.wired(tcx, field.span, rows, &what);
            }
        }
    }

    /// Whether a serde attribute of a field names a text adapter of
    /// `ambre_core::serde_id`. The text read runs from the end of the field
    /// before it. The derive reads the path as a string, and its helper
    /// attribute never reaches the HIR, so the source is read.
    fn text_adapter(tcx: TyCtxt<'_>, before: rustc_span::BytePos, field: rustc_span::Span) -> bool {
        let span = field.with_lo(before).with_hi(field.lo());
        let text = tcx
            .sess
            .source_map()
            .span_to_snippet(span)
            .unwrap_or_default();
        let text: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        text.split("#[").any(|attr| {
            attr.starts_with("serde(")
                && (attr.contains("with=\"ambre_core::serde_id::")
                    || attr.contains("with=\"::ambre_core::serde_id::"))
        })
    }
}
