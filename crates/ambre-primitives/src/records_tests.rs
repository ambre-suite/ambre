use super::{Claim, Privacy, Records, Role, Shape, SiteKey};

const KEY: &str = "demo | src/lib.rs | crate | Toggle.on | field | bool";

#[test]
fn a_site_record_reads_back_with_its_facts() {
    let mut records = Records::default();
    records.add(&format!(
        "unit\tdemo\tdemo\tbuild\n\
         site\t{KEY}\tpublic\tbool\tvariant-value\tok\tno foreign trait\t-\tsource\tcrates/demo/src/lib.rs:4\n"
    ));
    assert!(records.errors.is_empty(), "{:?}", records.errors);
    let key = SiteKey::parse(KEY).expect("a six-field key");
    let site = records.sites.get(&key).expect("the site");
    assert_eq!(site.privacy, Privacy::Public);
    assert_eq!(site.shape, Shape::Bool);
    assert_eq!(site.role, Role::VariantValue);
    assert_eq!(site.inner, Claim::Holds);
    assert_eq!(
        site.trait_claim,
        Claim::Fails("no foreign trait".to_owned())
    );
    assert_eq!(site.slot, None);
    assert_eq!(key.kind(), "field");
}

// The host and the wasm unit of one file write the same site: the judge
// reads it once.
#[test]
fn a_site_two_units_write_is_one_site() {
    let line = format!("site\t{KEY}\tprivate\tbool\tmember\tx\tx\t-\tsource\tsrc/lib.rs:4\n");
    let mut records = Records::default();
    records.add(&line);
    records.add(&line);
    assert_eq!(records.sites.len(), 1);
}

#[test]
fn a_record_the_judge_cannot_read_is_an_error() {
    let mut records = Records::default();
    records.add("site\tnot a key\n");
    records.add("site\ta | b\tpublic\tbool\tmember\tok\tok\t-\tsource\tx:1\n");
    records.add("bogus\tx\n");
    assert_eq!(records.errors.len(), 3, "{:?}", records.errors);
}

#[test]
fn every_other_record_lands_in_its_set() {
    let mut records = Records::default();
    records.add(
        "repr\tdemo::Space::Repr\n\
         lit\tdemo::set#0\tsrc/lib.rs:2\n\
         newtype\tdemo | src/lib.rs | crate | Name | String\tsrc/lib.rs:3\n\
         alias\tCount\tEdits\tsrc/lib.rs:4\n\
         generic\tCount\n\
         unwrap\tdemo | src/lib.rs | crate::f | Name::as_str -> g\tsrc/lib.rs:5:9\n\
         refuse\tderef\tsrc/lib.rs:6: a Deref\n\
         file\t/w/crates/demo/src/lib.rs\n",
    );
    assert!(records.errors.is_empty(), "{:?}", records.errors);
    assert_eq!(records.representations.len(), 1);
    assert_eq!(records.literals.len(), 1);
    assert_eq!(records.newtypes.len(), 1);
    assert_eq!(records.aliases.len(), 1);
    assert_eq!(records.generics.len(), 1);
    assert_eq!(records.unwraps.len(), 1);
    assert_eq!(records.refusals.len(), 1);
    assert_eq!(records.files.len(), 1);
}
