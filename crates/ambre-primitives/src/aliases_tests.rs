use super::{families, judge};
use crate::records::Records;

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

/// The records of `ambre-core`'s generic structs, then `extra`.
fn records(extra: &str) -> Records {
    let mut records = Records::default();
    records.add(&format!(
        "generic\tCount\ngeneric\tPos\ngeneric\tLen\ngeneric\tSecret\n{extra}"
    ));
    assert!(records.errors.is_empty(), "{:?}", records.errors);
    records
}

#[test]
fn the_families_are_the_names_of_the_table_of_section_2() {
    let found: Vec<String> = families(SPEC).into_iter().collect();
    assert_eq!(found, ["Count", "Len", "Pos"]);
}

#[test]
fn an_alias_of_a_bare_family_is_refused() {
    let refusals = judge(
        Some(SPEC),
        &records("alias\tCount\tEdits\tcrates/demo/src/lib.rs:3\n"),
    );
    let [refusal] = refusals.as_slice() else {
        panic!("one refusal expected: {refusals:?}");
    };
    assert!(
        refusal
            .starts_with("crates/demo/src/lib.rs:3: the alias Edits stands for the family Count"),
        "{refusal}"
    );
}

// The driver records each alias whose target is a generic struct of
// `ambre-core`; one that is no family of the table passes.
#[test]
fn an_alias_of_a_generic_struct_that_is_no_family_passes() {
    let refusals = judge(
        Some(SPEC),
        &records("alias\tSecret\tKey\tcrates/ambre-core/src/secret.rs:9\n"),
    );
    assert_eq!(refusals, Vec::<String>::new());
}

#[test]
fn a_family_the_core_declares_as_no_generic_struct_is_refused() {
    let mut records = Records::default();
    records.add("generic\tCount\ngeneric\tPos\n");
    let refusals = judge(Some(SPEC), &records);
    let [refusal] = refusals.as_slice() else {
        panic!("one refusal expected: {refusals:?}");
    };
    assert!(refusal.contains("names Len"), "{refusal}");
}

#[test]
fn a_missing_spec_or_an_empty_table_is_refused() {
    assert_eq!(judge(None, &records("")).len(), 1);
    let refusals = judge(Some("## 2. THE FAMILIES\n"), &records(""));
    assert!(
        refusals
            .iter()
            .any(|refusal| refusal.contains("names no family")),
        "{refusals:?}"
    );
}
