//! The sites of `ARCH-033` and the facts of each one (`ARCH-036` to
//! `ARCH-039`, `ARCH-043`, `ARCH-044`), read on fixture crates.

use crate::harness::{Build, Workspace, keys, site};

/// The key of a site of the fixture crate `ambre-demo`, in its root module.
fn key(item: &str, kind: &str, ty: &str) -> String {
    format!("ambre-demo | src/lib.rs | crate | {item} | {kind} | {ty}")
}

const DEMO: &str = r#"
pub struct Thing { pub id: u32, name: String, done: Marker }
pub struct Marker;
pub enum Shape { Dot(bool), Pair(u8, u8) }
pub const LIMIT: usize = 4;
pub static NAME: &str = "x";
pub type Pairs = Vec<(u16, u16)>;
pub fn get(index: usize, call: impl Fn(&str) -> bool) -> Option<char> { let _ = (index, call); None }
pub async fn load() -> u64 { 0 }
pub fn ratio(c: core::ffi::c_int) {}
pub mod inner {
    pub(crate) fn hidden(text: &String) {}
}
impl Thing {
    pub fn new(id: u32) -> Self { Self { id, name: String::new(), done: Marker } }
}
pub trait Space { type Repr; }
pub struct Doc;
impl Space for Doc { type Repr = f32; }
pub struct Px<S: Space>(S::Repr, core::marker::PhantomData<S>);
impl core::ops::Add<u32> for Marker { type Output = Marker; fn add(self, rhs: u32) -> Marker { self } }
pub struct Grid(std::collections::HashMap<u8, ()>);
pub fn wide(map: std::collections::HashMap<Doc, ()>) {}
"#;

// Each kind of site is read, with its key as the allowlist names it, an
// `impl Trait` parameter and an `async fn` return included (`ARCH-033`).
#[test]
fn every_kind_of_site_is_read() {
    let ws = Workspace::new("kinds");
    ws.file("crates/ambre-demo/src/lib.rs", DEMO);
    let records = ws.compile("ambre_demo", "ambre-demo", &[], Build::Shipped);
    let found = keys(&records);
    for expected in [
        key("Thing.id", "field", "u32"),
        key("Thing.name", "field", "String"),
        key("Shape::Dot.0", "field", "bool"),
        key("Shape::Pair.0", "field", "u8"),
        key("LIMIT", "const", "usize"),
        key("NAME", "static", "&str"),
        key("Pairs", "alias", "Vec<(u16,u16)>"),
        key("get(index)", "param", "usize"),
        key("get(call)", "param", "impl Fn(&str)->bool"),
        key("get()", "ret", "Option<char>"),
        key("load()", "ret", "u64"),
        key("Thing::new(id)", "param", "u32"),
        key("<Doc as Space>::Repr", "assoc", "f32"),
        key("<Marker as core::ops::Add<u32>>::add(rhs)", "param", "u32"),
        "ambre-demo | src/lib.rs | crate::inner | hidden(text) | param | &String".to_owned(),
    ] {
        assert!(
            found.contains(&expected),
            "{expected} missing from {found:#?}"
        );
    }
    // A unit struct, `Self`, a marker projection and a default argument hold
    // no primitive the source writes.
    for absent in ["Thing.done", "Thing::new()", "Px.0", "wide(map)"] {
        assert!(
            !found.iter().any(|k| k.contains(&format!("| {absent} |"))),
            "{absent} is no site: {found:#?}"
        );
    }
}

// An alias of a foreign crate reads as its target: `c_int` is an `i32`
// (`ARCH-044`).
#[test]
fn a_foreign_alias_reads_as_its_target() {
    let ws = Workspace::new("foreign-alias");
    ws.file("crates/ambre-demo/src/lib.rs", DEMO);
    let records = ws.compile("ambre_demo", "ambre-demo", &[], Build::Shipped);
    assert!(keys(&records).contains(&key("ratio(c)", "param", "core::ffi::c_int")));
}

// A test build writes no site: test code is out of the rule.
#[test]
fn test_code_is_not_read() {
    let ws = Workspace::new("test-code");
    ws.file(
        "crates/ambre-demo/src/lib.rs",
        "pub fn kept(n: u8) {}\n#[cfg(test)]\nmod tests { pub fn helper(n: u16) {} }\n",
    );
    let shipped = ws.compile("ambre_demo", "ambre-demo", &[], Build::Shipped);
    assert_eq!(keys(&shipped), [key("kept(n)", "param", "u8")]);
    let test = ws.compile("ambre_demo", "ambre-demo", &[], Build::Test);
    assert_eq!(keys(&test), Vec::<String>::new());
}

// Who reaches a site is read from the visibility the source writes. A
// `pub(crate)` at the crate root is restricted, a trait impl is never
// private, and a variant field shares its enum's (`ARCH-038`).
#[test]
fn the_privacy_of_each_site_is_read_as_written() {
    let ws = Workspace::new("privacy");
    ws.file("crates/ambre-demo/src/lib.rs", DEMO);
    let records = ws.compile("ambre_demo", "ambre-demo", &[], Build::Shipped);
    let privacy = |k: &str| site(&records, k)[0].clone();
    assert_eq!(privacy(&key("Thing.id", "field", "u32")), "public");
    assert_eq!(privacy(&key("Thing.name", "field", "String")), "private");
    assert_eq!(privacy(&key("Shape::Dot.0", "field", "bool")), "public");
    assert_eq!(
        privacy("ambre-demo | src/lib.rs | crate::inner | hidden(text) | param | &String"),
        "restricted"
    );
    assert_eq!(
        privacy(&key(
            "<Marker as core::ops::Add<u32>>::add(rhs)",
            "param",
            "u32"
        )),
        "trait-impl"
    );
}

// A field of a type that derives a serde trait crosses a wire. A field of a
// type of `ambre-bus` crosses a thread. Neither is private (`ARCH-038`).
#[test]
fn a_serialised_or_bus_field_is_not_private() {
    let ws = Workspace::new("serialised");
    ws.file("crates/serde/src/lib.rs", "pub trait Serialize {}\n");
    ws.file(
        "crates/ambre-demo/src/lib.rs",
        "pub struct Wire { flag: bool }\n#[automatically_derived]\nimpl serde::Serialize for Wire {}\n",
    );
    ws.file(
        "crates/ambre-bus/src/lib.rs",
        "pub struct Msg { count: u32 }\n",
    );
    ws.compile("serde", "serde", &[], Build::Shipped);
    let demo = ws.compile("ambre_demo", "ambre-demo", &["serde"], Build::Shipped);
    assert_eq!(
        site(&demo, &key("Wire.flag", "field", "bool"))[0],
        "serialised"
    );
    let bus = ws.compile("ambre_bus", "ambre-bus", &[], Build::Shipped);
    assert_eq!(
        site(
            &bus,
            "ambre-bus | src/lib.rs | crate | Msg.count | field | u32"
        )[0],
        "bus"
    );
}

// The shape of a `bool`: bare, wrapped, or in a callable's signature, where
// it is no value the site holds (`ARCH-039`).
#[test]
fn the_shape_of_a_bool_is_read_outside_callables() {
    let ws = Workspace::new("shape");
    ws.file(
        "crates/ambre-demo/src/lib.rs",
        "pub struct S { pub a: bool, pub b: Option<bool>, pub c: Box<dyn Fn(bool)>, pub d: fn() -> bool }\n",
    );
    let records = ws.compile("ambre_demo", "ambre-demo", &[], Build::Shipped);
    let shape = |item: &str, ty: &str| site(&records, &key(item, "field", ty))[1].clone();
    assert_eq!(shape("S.a", "bool"), "bool");
    assert_eq!(shape("S.b", "Option<bool>"), "holds-bool");
    assert_eq!(shape("S.c", "Box<dyn Fn(bool)>"), "other");
    assert_eq!(shape("S.d", "fn()->bool"), "other");
}

// A `newtype-inner` claim holds on the field of a struct of one field and on
// the codec of a newtype. It holds on an associated type a generic newtype
// holds too (`ARCH-036`).
#[test]
fn a_newtype_inner_claim_holds_on_the_representation_alone() {
    let ws = Workspace::new("inner");
    ws.file("crates/ambre-demo/src/lib.rs", DEMO);
    let records = ws.compile("ambre_demo", "ambre-demo", &[], Build::Shipped);
    let inner = |k: &str| site(&records, k)[3].clone();
    assert_eq!(inner(&key("Shape::Dot.0", "field", "bool")), "ok");
    assert!(inner(&key("Shape::Pair.0", "field", "u8")).starts_with("a field of a variant"));
    assert!(inner(&key("Thing.id", "field", "u32")).starts_with("a field of a struct of 3"));
    assert!(
        inner(&key("Thing::new(id)", "param", "u32")).contains("neither a struct of one field")
    );
    assert_eq!(
        inner(&key("<Doc as Space>::Repr", "assoc", "f32")),
        "repr:ambre_demo::Space::Repr"
    );
    assert!(crate::harness::of(&records, "repr").contains(&"ambre_demo::Space::Repr".to_owned()));
}

// A `trait` claim holds on a signature of a foreign trait whose arguments the
// impl did not choose. It holds on a closure of a foreign type (`ARCH-036`).
#[test]
fn a_trait_claim_holds_on_a_foreign_signature_alone() {
    let ws = Workspace::new("trait");
    ws.file(
        "crates/gloo/src/lib.rs",
        "pub trait Handler { fn call(&self, on: bool); }\npub struct Event;\n",
    );
    ws.file(
        "crates/ambre-demo/src/lib.rs",
        "pub struct Px;\n\
         impl gloo::Handler for Px { fn call(&self, on: bool) {} }\n\
         impl core::ops::Add<u32> for Px { type Output = Px; fn add(self, rhs: u32) -> Px { self } }\n\
         pub trait Own { fn own(&self, n: u8); }\n\
         pub fn listen(f: Box<dyn Fn(gloo::Event) -> bool>) {}\n\
         pub fn local(f: Box<dyn Fn(u8)>) {}\n",
    );
    ws.compile("gloo", "gloo", &[], Build::Shipped);
    let records = ws.compile("ambre_demo", "ambre-demo", &["gloo"], Build::Shipped);
    let claim = |k: &str| site(&records, k)[4].clone();
    assert_eq!(
        claim(&key("<Px as gloo::Handler>::call(on)", "param", "bool")),
        "ok"
    );
    assert!(
        claim(&key(
            "<Px as core::ops::Add<u32>>::add(rhs)",
            "param",
            "u32"
        ))
        .contains("chose the primitive")
    );
    assert!(
        claim(&key("Own::own(n)", "param", "u8")).contains("neither in an impl of a foreign trait")
    );
    assert_eq!(
        claim(&key("listen(f)", "param", "Box<dyn Fn(gloo::Event)->bool>")),
        "ok"
    );
    assert!(
        claim(&key("local(f)", "param", "Box<dyn Fn(u8)>"))
            .contains("names no type of a foreign crate")
    );
}

// A site a macro of the workspace writes is read as the source's, keyed at
// the call (`ARCH-043`). What a foreign macro writes takes its types from
// that crate and is no site.
#[test]
fn a_site_a_workspace_macro_writes_is_read() {
    let ws = Workspace::new("macro");
    ws.file(
        "crates/ambre-demo/src/lib.rs",
        "macro_rules! flag { ($name:ident) => { pub struct $name; impl $name { pub fn as_bool(&self) -> bool { true } } } }\n\
         flag!(Open);\n\
         #[derive(PartialEq)]\npub struct Plain;\n",
    );
    let records = ws.compile("ambre_demo", "ambre-demo", &[], Build::Shipped);
    let found = keys(&records);
    let written = key("Open::as_bool()", "ret", "bool");
    assert!(found.contains(&written), "{found:#?}");
    assert_eq!(site(&records, &written)[6], "macro:flag");
    assert!(!found.iter().any(|k| k.contains("eq")), "{found:#?}");
}
