use super::{Source, families, judge};

/// A families table of two rows, as section 2 of the spec writes it.
const SPEC: &str = "\
## 1. THE RULE\n\
\n\
| Need | Family |\n\
| :--- | :--- |\n\
| ignored | `Ignored<K>` |\n\
\n\
## 2. THE FAMILIES\n\
\n\
| Need | Family | Marker trait |\n\
| :--- | :--- | :--- |\n\
| How many things | `Count<K>` | `CountKind` |\n\
| A position and a length | `Pos<A>`, `Len<A>` | `Axis` |\n\
\n\
## 3. THE MARKER GRAMMAR\n";

/// The families as `ambre-core` declares them.
const CORE: &str = "\
pub struct Count<K: CountKind>(u32, PhantomData<fn() -> K>);\n\
pub struct Pos<A: Axis>(u32, PhantomData<fn() -> A>);\n\
pub struct Len<A: Axis>(u32, PhantomData<fn() -> A>);";

fn source(krate: &str, shown: &str, text: &str) -> Source {
    Source {
        krate: krate.to_owned(),
        shown: shown.to_owned(),
        text: text.to_owned(),
    }
}

/// The refusals over `ambre-core` and one file of the crate `demo`.
fn refused(text: &str) -> Vec<String> {
    judge(
        Some(SPEC),
        &[
            source("ambre-core", "crates/ambre-core/src/lib.rs", CORE),
            source("demo", "crates/demo/src/lib.rs", text),
        ],
    )
}

/// The aliases a refusal names, by name.
fn names(text: &str) -> Vec<String> {
    refused(text)
        .iter()
        .map(|refusal| {
            refusal
                .split_once("the alias ")
                .and_then(|(_, rest)| rest.split_once(' '))
                .map_or_else(|| refusal.clone(), |(name, _)| name.to_owned())
        })
        .collect()
}

#[test]
fn the_families_are_the_names_of_the_table_of_section_2() {
    assert_eq!(
        families(SPEC).into_iter().collect::<Vec<_>>(),
        ["Count", "Len", "Pos"]
    );
}

#[test]
fn an_alias_of_a_bare_family_is_refused() {
    let refusals = refused(
        "use ambre_core::Count;\n\
         type Updates = Count<EditUpdates>;\n\
         pub type Cursor = ambre_core::index::Pos<DocChar>;",
    );
    assert_eq!(refusals.len(), 2);
    assert!(refusals[0].starts_with(
        "crates/demo/src/lib.rs: the alias Updates stands for the family Count alone. \
         Remove the alias and write the family in full at each use, as Count<EditUpdates>"
    ));
    assert!(refusals[1].contains("the alias Cursor stands for the family Pos alone"));
}

#[test]
fn an_alias_of_a_reference_to_a_family_is_refused() {
    assert_eq!(
        names(
            "use ambre_core::{Count, Len};\n\
             type Borrowed<'a> = &'a Count<EditUpdates>;\n\
             type Grouped = (Len<DocChar>);"
        ),
        ["Borrowed", "Grouped"]
    );
}

#[test]
fn an_alias_of_an_alias_of_a_family_is_refused() {
    let refusals = refused(
        "use ambre_core::Count;\n\
         type Inner<K> = Count<K>;\n\
         type Outer = &'static Inner<EditUpdates>;",
    );
    assert_eq!(refusals.len(), 2);
    assert!(
        refusals[1].contains(
            "the alias Outer stands for the family Count alone, through the alias Inner."
        )
    );
}

/// A family a workspace crate re-exports, or a module of the crate imports,
/// is the family still.
#[test]
fn a_family_through_a_re_export_or_a_module_import_is_refused() {
    let refusals = judge(
        Some(SPEC),
        &[
            source("ambre-core", "crates/ambre-core/src/lib.rs", CORE),
            source(
                "ambre-bus",
                "crates/ambre-bus/src/lib.rs",
                "pub use ambre_core::Len;",
            ),
            source(
                "demo",
                "crates/demo/src/units.rs",
                "pub(crate) use ambre_core::Count;",
            ),
            source(
                "demo",
                "crates/demo/src/lib.rs",
                "type Width = ambre_bus::Len<Columns>;\n\
                 type Rows = crate::units::Count<Lines>;",
            ),
        ],
    );
    assert_eq!(refusals.len(), 2);
    assert!(refusals[0].contains("the alias Width stands for the family Len alone"));
    assert!(refusals[1].contains("the alias Rows stands for the family Count alone"));
}

#[test]
fn a_tuple_or_a_container_alias_that_holds_a_family_passes() {
    assert!(
        refused(
            "use ambre_core::{Count, Pos};\n\
             type Spot = (BlockId, Pos<DocChar>);\n\
             type Place = Option<Pos<DocChar>>;\n\
             type Shared = Rc<RefCell<Vec<Count<EditUpdates>>>>;\n\
             type Clock = Rc<dyn Fn() -> Count<EditUpdates>>;\n\
             type Slice = [Pos<DocChar>];"
        )
        .is_empty()
    );
}

/// A type of a family's name that is no family, a generic parameter and a
/// type of the workspace are each out of the rule.
#[test]
fn an_alias_of_no_family_passes() {
    assert!(
        refused(
            "use std::time::Instant;\n\
             struct Len(u32);\n\
             type Clock = Instant;\n\
             type Width = Len;\n\
             type Same<Count> = Count;\n\
             type Home = (String, String);\n\
             type Rows = other::Count<Lines>;"
        )
        .is_empty()
    );
}

#[test]
fn an_alias_in_a_test_module_is_held_the_same_way() {
    assert_eq!(
        names(
            "#[cfg(test)]\n\
             mod tests {\n\
                 use super::*;\n\
                 type Probe = Count<EditUpdates>;\n\
                 mod deeper { type Deep = ambre_core::Len<DocChar>; }\n\
             }"
        ),
        ["Probe", "Deep"]
    );
}

/// An alias in a `fn` body, and an associated type, are no module-level
/// alias.
#[test]
fn an_alias_in_a_body_or_an_impl_is_out_of_reach() {
    assert!(
        refused(
            "use ambre_core::Count;\n\
             fn build() { type Local = Count<EditUpdates>; }\n\
             impl Iterator for Walk { type Item = Count<EditUpdates>; }"
        )
        .is_empty()
    );
}

#[test]
fn a_family_the_core_declares_as_no_generic_struct_is_refused() {
    let refusals = judge(
        Some(SPEC),
        &[source(
            "ambre-core",
            "crates/ambre-core/src/lib.rs",
            "pub struct Count<K>(u32, PhantomData<K>);\npub struct Pos(u32);",
        )],
    );
    assert_eq!(refusals.len(), 2);
    assert!(refusals[0].contains("names Len, and ambre-core declares no generic struct"));
    assert!(refusals[1].contains("names Pos, and ambre-core declares no generic struct"));
}

#[test]
fn a_missing_spec_or_an_empty_table_is_refused() {
    assert_eq!(
        judge(None, &[]),
        ["cannot read specs/1-Foundations/ambre-spec-newtypes.md"]
    );
    assert_eq!(judge(Some("## 2. THE FAMILIES\n"), &[]).len(), 1);
}

#[test]
fn a_file_that_does_not_parse_is_refused() {
    let refusals = refused("type = ;");
    assert_eq!(refusals.len(), 1);
    assert!(refusals[0].starts_with("crates/demo/src/lib.rs: does not parse"));
}
