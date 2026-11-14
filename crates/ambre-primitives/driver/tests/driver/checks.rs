//! The checks that read an item whole, and the files a compile reads.

use crate::harness::{Build, Workspace, of};

// A struct of one field over a census primitive is counted with its key. A
// marker is no field, and a generic struct of `ambre-core` is a family
// (`ARCH-062`).
#[test]
fn a_single_field_struct_over_a_primitive_is_counted() {
    let ws = Workspace::new("census");
    ws.file(
        "crates/ambre-core/src/lib.rs",
        "pub struct Count<K>(u32, core::marker::PhantomData<K>);\npub struct Name(String);\n",
    );
    ws.file(
        "crates/ambre-demo/src/lib.rs",
        "pub struct Word(String);\n\
         pub struct Tagged<K>(Option<u8>, core::marker::PhantomData<K>);\n\
         pub struct Bytes<'a>(&'a [u8]);\n\
         pub struct Two(u8, u8);\n\
         pub struct Wrapped(Vec<String>);\n\
         pub fn f() { struct Inner(char); }\n",
    );
    let core = ws.compile("ambre_core", "ambre-core", &[], Build::Shipped);
    assert_eq!(
        of(&core, "newtype")
            .iter()
            .filter_map(|l| l.split('\t').next())
            .collect::<Vec<_>>(),
        ["ambre-core | src/lib.rs | crate | Name | String"]
    );
    let demo = ws.compile("ambre_demo", "ambre-demo", &[], Build::Shipped);
    let found: Vec<String> = of(&demo, "newtype")
        .iter()
        .filter_map(|l| l.split('\t').next())
        .map(str::to_owned)
        .collect();
    assert_eq!(
        found,
        [
            "ambre-demo | src/lib.rs | crate | Bytes | &'a[u8]",
            "ambre-demo | src/lib.rs | crate | Tagged | Option<u8>",
            "ambre-demo | src/lib.rs | crate | Word | String",
            "ambre-demo | src/lib.rs | crate | f::Inner | char",
        ]
    );
}

// A field `F::Repr` reads as each type an impl in the compile gives it, or an
// equality bound of the struct. One whose instance is a `u32` is counted
// (`ARCH-062`).
#[test]
fn a_field_over_an_associated_type_is_counted_through_its_instances() {
    let ws = Workspace::new("census-assoc");
    ws.file(
        "crates/ambre-base/src/lib.rs",
        "pub trait Field { type Repr; }
         pub enum Wide {}
         impl Field for Wide { type Repr = u64; }
",
    );
    ws.file(
        "crates/ambre-demo/src/lib.rs",
        "use core::marker::PhantomData;
         pub trait Width { type Repr; }
         pub enum Stamp {}
         impl Width for Stamp { type Repr = u32; }
         pub struct Mark<F: Width>(F::Repr, PhantomData<F>);
         pub struct Maybe<F: Width>(Option<F::Repr>, PhantomData<F>);
         pub struct Upstream<F: ambre_base::Field>(F::Repr, PhantomData<F>);
         pub trait Shape { type Inner; }
         pub struct Bound<S: Shape<Inner = String>>(S::Inner, PhantomData<S>);
         pub trait Opaque { type Held; }
         pub enum Plain {}
         impl Opaque for Plain { type Held = Vec<String>; }
         pub struct Never<O: Opaque>(O::Held, PhantomData<O>);
",
    );
    ws.compile("ambre_base", "ambre-base", &[], Build::Shipped);
    let demo = ws.compile("ambre_demo", "ambre-demo", &["ambre_base"], Build::Shipped);
    let found: Vec<String> = of(&demo, "newtype")
        .iter()
        .filter_map(|l| l.split('\t').next())
        .map(str::to_owned)
        .collect();
    assert_eq!(
        found,
        [
            "ambre-demo | src/lib.rs | crate | Bound | S::Inner",
            "ambre-demo | src/lib.rs | crate | Mark | F::Repr",
            "ambre-demo | src/lib.rs | crate | Maybe | Option<F::Repr>",
            "ambre-demo | src/lib.rs | crate | Upstream | F::Repr",
        ]
    );
}

// A tuple struct of the workspace holds no field that is not private: a
// newtype hands its value out by a named accessor (`NEWTYPE-001`).
#[test]
fn a_public_field_of_a_tuple_struct_is_refused() {
    let ws = Workspace::new("pub-field");
    ws.file(
        "crates/ambre-demo/src/lib.rs",
        "pub struct Open(pub u32);\npub struct Crate(pub(crate) u8);\npub struct Shut(u16);\npub struct Named { pub x: u8 }\n",
    );
    let records = ws.compile("ambre_demo", "ambre-demo", &[], Build::Shipped);
    let refused: Vec<String> = of(&records, "refuse")
        .into_iter()
        .filter(|l| l.starts_with("pub-field\t"))
        .collect();
    assert_eq!(refused.len(), 2, "{refused:#?}");
    assert!(
        refused
            .iter()
            .any(|l| l.contains("lib.rs:1: the tuple struct Open"))
    );
    assert!(
        refused
            .iter()
            .any(|l| l.contains("lib.rs:2: the tuple struct Crate"))
    );
}

// A `Deref` to a primitive, to a carrier of one or to a type parameter is
// refused, whoever writes it. A `Deref` to another type passes (`ARCH-037`).
#[test]
fn a_deref_to_a_primitive_is_refused() {
    let ws = Workspace::new("deref");
    ws.file(
        "crates/ambre-demo/src/lib.rs",
        "use core::ops::Deref;\n\
         pub struct A(String);\nimpl Deref for A { type Target = str; fn deref(&self) -> &str { &self.0 } }\n\
         pub struct B(Vec<u8>);\nimpl Deref for B { type Target = Vec<u8>; fn deref(&self) -> &Vec<u8> { &self.0 } }\n\
         pub struct C<T>(T);\nimpl<T> Deref for C<T> { type Target = T; fn deref(&self) -> &T { &self.0 } }\n\
         pub struct D(A);\nimpl Deref for D { type Target = A; fn deref(&self) -> &A { &self.0 } }\n\
         macro_rules! hand { ($t:ident) => { pub struct $t(u8); impl Deref for $t { type Target = u8; fn deref(&self) -> &u8 { &self.0 } } } }\n\
         hand!(E);\n",
    );
    let records = ws.compile("ambre_demo", "ambre-demo", &[], Build::Shipped);
    let refused: Vec<String> = of(&records, "refuse")
        .into_iter()
        .filter(|l| l.starts_with("deref\t"))
        .collect();
    assert_eq!(refused.len(), 4, "{refused:#?}");
    for name in [
        "A implements Deref",
        "B implements Deref",
        "C<T> implements Deref",
        "E implements Deref",
    ] {
        assert!(
            refused.iter().any(|l| l.contains(name)),
            "{name}: {refused:#?}"
        );
    }
}

// An alias that stands for a generic struct of `ambre-core` is recorded for
// the judge, test code included. References and aliases are set apart. A
// tuple that holds one is no record (`ARCH-066`).
#[test]
fn an_alias_of_a_generic_struct_of_core_is_recorded() {
    let ws = Workspace::new("alias");
    ws.file(
        "crates/ambre-core/src/lib.rs",
        "pub struct Count<K>(u32, core::marker::PhantomData<K>);\n",
    );
    ws.file(
        "crates/ambre-demo/src/lib.rs",
        "pub struct Edit;\n\
         pub type Edits = ambre_core::Count<Edit>;\n\
         pub type Ref<'a> = &'a Edits;\n\
         pub type Pair = (ambre_core::Count<Edit>, u8);\n\
         #[cfg(test)]\nmod tests { pub type Local = ambre_core::Count<super::Edit>; }\n",
    );
    let core = ws.compile("ambre_core", "ambre-core", &[], Build::Shipped);
    assert_eq!(of(&core, "generic"), ["Count"]);
    let demo = ws.compile("ambre_demo", "ambre-demo", &["ambre_core"], Build::Shipped);
    let names: Vec<String> = of(&demo, "alias")
        .iter()
        .map(|l| l.split('\t').nth(1).unwrap_or_default().to_owned())
        .collect();
    assert_eq!(names, ["Edits", "Ref"]);
    let test = ws.compile("ambre_demo", "ambre-demo", &["ambre_core"], Build::Test);
    assert!(
        of(&test, "alias").iter().any(|l| l.contains("\tLocal\t")),
        "{test:#?}"
    );
}

// The driver records each file rustc read. A module a `#[path]` names and a
// file an `include!` splices are among them (`ARCH-050`).
#[test]
fn each_file_rustc_reads_is_recorded() {
    let ws = Workspace::new("files");
    ws.file(
        "crates/ambre-demo/src/lib.rs",
        "#[path = \"elsewhere/odd.txt\"]\nmod odd;\ninclude!(\"spliced.rs\");\nmod plain;\n",
    );
    ws.file(
        "crates/ambre-demo/src/elsewhere/odd.txt",
        "pub fn odd(n: u8) {}\n",
    );
    ws.file(
        "crates/ambre-demo/src/spliced.rs",
        "pub fn spliced(n: u16) {}\n",
    );
    ws.file(
        "crates/ambre-demo/src/plain.rs",
        "pub fn plain(n: u32) {}\n",
    );
    let records = ws.compile("ambre_demo", "ambre-demo", &[], Build::Shipped);
    let files = of(&records, "file");
    for name in [
        "src/lib.rs",
        "src/elsewhere/odd.txt",
        "src/spliced.rs",
        "src/plain.rs",
    ] {
        let path = ws.root().join("crates/ambre-demo").join(name);
        assert!(
            files.contains(&path.display().to_string()),
            "{name}: {files:#?}"
        );
    }
    let keys = crate::harness::keys(&records);
    assert!(
        keys.iter()
            .any(|k| k.starts_with("ambre-demo | src/elsewhere/odd.txt | crate::odd | odd(n)")),
        "{keys:#?}"
    );
    assert!(
        keys.iter()
            .any(|k| k.starts_with("ambre-demo | src/spliced.rs | crate | spliced(n)")),
        "{keys:#?}"
    );
}
