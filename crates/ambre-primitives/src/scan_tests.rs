use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::{Scan, SiteKey, Tree, scan};

/// A source tree held in memory, so a test states every file it reads.
struct Memory(BTreeMap<PathBuf, String>);

impl Tree for Memory {
    fn read(&self, path: &Path) -> Option<String> {
        self.0.get(path).cloned()
    }

    fn files(&self, dir: &Path) -> Vec<PathBuf> {
        self.0
            .keys()
            .filter(|path| path.starts_with(dir))
            .cloned()
            .collect()
    }
}

const CRATES: &str = "/w/crates";

/// One crate named `demo`, its files given relative to the crate directory.
fn demo(files: &[(&str, &str)]) -> Memory {
    let mut tree = BTreeMap::new();
    tree.insert(PathBuf::from(CRATES).join("demo/Cargo.toml"), String::new());
    for (path, text) in files {
        tree.insert(
            PathBuf::from(CRATES).join("demo").join(path),
            (*text).to_owned(),
        );
    }
    Memory(tree)
}

/// The keys the scan finds, and the errors it reports.
fn run(files: &[(&str, &str)]) -> (Vec<String>, Vec<String>) {
    let result = scan(&demo(files), Path::new(CRATES));
    (
        result.sites.iter().map(ToString::to_string).collect(),
        result.errors,
    )
}

/// The keys alone, for a tree the scan must read without an error.
fn keys(files: &[(&str, &str)]) -> Vec<String> {
    let (keys, errors) = run(files);
    assert_eq!(errors, Vec::<String>::new());
    keys
}

#[test]
fn a_primitive_field_is_a_site_and_a_typed_one_is_not() {
    let found = keys(&[("src/lib.rs", "pub struct Thing { id: u32, name: Name }")]);
    assert_eq!(
        found,
        ["demo | src/lib.rs | crate | Thing.id | field | u32"]
    );
}

#[test]
fn every_kind_of_site_is_read() {
    let found = keys(&[(
        "src/lib.rs",
        "pub const LIMIT: usize = 4;\n\
         static NAME: &str = \"x\";\n\
         type Count = u64;\n\
         pub fn get(index: usize) -> Option<&'static str> { None }",
    )]);
    assert_eq!(
        found,
        [
            "demo | src/lib.rs | crate | Count | alias | u64",
            "demo | src/lib.rs | crate | LIMIT | const | usize",
            "demo | src/lib.rs | crate | NAME | static | &str",
            "demo | src/lib.rs | crate | get() | ret | Option<&'static str>",
            "demo | src/lib.rs | crate | get(index) | param | usize",
        ]
    );
}

#[test]
fn the_item_path_names_the_impl_the_trait_the_variant_and_the_enclosing_fn() {
    let found = keys(&[(
        "src/lib.rs",
        "impl Row { pub fn new(index: u32) -> Self { Row } }\n\
         impl From<u32> for Row { fn from(value: u32) -> Self { Row } }\n\
         pub enum Shape { Line(f64), Box { width: f32 } }\n\
         fn outer() { fn inner(step: u8) {} }",
    )]);
    assert_eq!(
        found,
        [
            "demo | src/lib.rs | crate | <Row as From<u32>>::from(value) | param | u32",
            "demo | src/lib.rs | crate | Row::new(index) | param | u32",
            "demo | src/lib.rs | crate | Shape::Box.width | field | f32",
            "demo | src/lib.rs | crate | Shape::Line.0 | field | f64",
            "demo | src/lib.rs | crate | outer::inner(step) | param | u8",
        ]
    );
}

#[test]
fn the_key_carries_no_line_number() {
    let source = "pub fn f(x: u32) {}";
    let moved = format!("\n\n// A comment above.\n{source}");
    assert_eq!(
        keys(&[("src/lib.rs", source)]),
        keys(&[("src/lib.rs", &moved)])
    );
}

#[test]
fn a_module_file_resolves_where_rustc_finds_it() {
    let found = keys(&[
        ("src/lib.rs", "mod a; mod c;"),
        ("src/a.rs", "mod b;"),
        ("src/a/b.rs", "pub fn f(x: u32) {}"),
        ("src/c/mod.rs", "mod inline { pub fn g(y: bool) {} }"),
    ]);
    assert_eq!(
        found,
        [
            "demo | src/a/b.rs | crate::a::b | f(x) | param | u32",
            "demo | src/c/mod.rs | crate::c::inline | g(y) | param | bool",
        ]
    );
}

#[test]
fn a_binary_and_a_build_script_are_roots() {
    let found = keys(&[
        ("src/main.rs", "fn run(code: i32) {}"),
        ("src/bin/tool.rs", "fn go(n: u8) {}"),
        ("build.rs", "fn emit(line: &str) {}"),
    ]);
    assert_eq!(found.len(), 3);
}

#[test]
fn a_test_module_in_a_file_of_its_own_is_never_read() {
    let found = keys(&[
        (
            "src/lib.rs",
            "#[cfg(test)]\nmod tests;\n#[cfg(test)]\nmod deep;",
        ),
        ("src/tests.rs", "fn helper(x: u32) {}"),
        ("src/deep/mod.rs", "mod more;"),
        ("src/deep/more.rs", "fn helper(x: u32) {}"),
    ]);
    assert_eq!(found, Vec::<String>::new());
}

#[test]
fn a_test_module_behind_a_path_attribute_is_never_read() {
    let found = keys(&[
        ("src/lib.rs", "mod view;"),
        (
            "src/view.rs",
            "#[cfg(test)]\n#[path = \"view_tests.rs\"]\nmod tests;",
        ),
        ("src/view_tests.rs", "fn helper(x: u32) {}"),
    ]);
    assert_eq!(found, Vec::<String>::new());
}

#[test]
fn a_module_only_a_test_build_compiles_is_skipped() {
    let found = keys(&[
        (
            "src/lib.rs",
            "#[cfg(any(test, feature = \"testing\"))]\nmod a;\n\
             #[cfg(all(test, feature = \"pdf\"))]\nmod b;\n\
             #[cfg(feature = \"test-helpers\")]\nmod c;",
        ),
        ("src/a.rs", "pub fn f(x: u32) {}"),
        ("src/b.rs", "pub fn f(x: u32) {}"),
        ("src/c.rs", "pub fn f(x: u32) {}"),
    ]);
    assert_eq!(found, Vec::<String>::new());
}

#[test]
fn a_module_another_build_compiles_is_scanned() {
    let found = keys(&[
        (
            "src/lib.rs",
            "#[cfg(any(test, not(target_arch = \"wasm32\")))]\nmod host;\n\
             #[cfg(not(test))]\nmod live;\n\
             #[cfg(feature = \"pdf\")]\nmod pdf;",
        ),
        ("src/host.rs", "pub fn f(x: u32) {}"),
        ("src/live.rs", "pub fn f(x: u32) {}"),
        ("src/pdf.rs", "pub fn f(x: u32) {}"),
    ]);
    assert_eq!(found.len(), 3);
}

#[test]
fn a_test_function_and_a_test_item_are_skipped() {
    let found = keys(&[(
        "src/lib.rs",
        "#[test]\nfn a(x: u32) {}\n\
         #[wasm_bindgen_test]\nfn b(x: u32) {}\n\
         #[tokio::test]\nasync fn c(x: u32) {}\n\
         #[cfg(test)]\nstruct D { x: u32 }\n\
         #[cfg(test)]\nimpl E { fn e(x: u32) {} }",
    )]);
    assert_eq!(found, Vec::<String>::new());
}

#[test]
fn a_tests_file_is_skipped_by_its_name() {
    let found = keys(&[
        ("src/lib.rs", "mod grid_tests;"),
        ("src/grid_tests.rs", "fn f(x: u32) {}"),
    ]);
    assert_eq!(found, Vec::<String>::new());
}

#[test]
fn an_item_a_macro_writes_is_out_of_reach() {
    let found = keys(&[(
        "src/lib.rs",
        "macro_rules! m { () => { pub fn f(x: u32) {} } }\n\
         m!();\n\
         define_ids! { pub struct Id(u32); }",
    )]);
    assert_eq!(found, Vec::<String>::new());
}

#[test]
fn a_file_no_module_reaches_fails_the_scan() {
    let (_, errors) = run(&[("src/lib.rs", ""), ("src/orphan.rs", "pub fn f(x: u32) {}")]);
    assert_eq!(errors.len(), 1);
    assert!(errors.iter().all(|error| error.contains("orphan.rs")));
}

#[test]
fn a_declared_module_with_no_file_fails_the_scan() {
    let (_, errors) = run(&[("src/lib.rs", "mod gone;")]);
    assert_eq!(errors.len(), 1);
}

#[test]
fn two_cfg_twins_keep_a_key_each() {
    let found = keys(&[(
        "src/lib.rs",
        "#[cfg(target_arch = \"wasm32\")]\nfn f(x: u32) {}\n\
         #[cfg(not(target_arch = \"wasm32\"))]\nfn f(x: u32) {}",
    )]);
    assert_eq!(
        found,
        [
            "demo | src/lib.rs | crate | f(x) | param | u32",
            "demo | src/lib.rs | crate | f(x)#2 | param | u32"
        ]
    );
}

/// The scan of one `src/lib.rs`, read without an error.
fn scanned(source: &str) -> Scan {
    let result = scan(&demo(&[("src/lib.rs", source)]), Path::new(CRATES));
    assert_eq!(result.errors, Vec::<String>::new());
    result
}

/// The one key whose item field reads `item`.
fn site(scan: &Scan, item: &str) -> SiteKey {
    let found: Vec<&SiteKey> = scan
        .sites
        .iter()
        .filter(|key| key.to_string().split(SiteKey::SEPARATOR).nth(3) == Some(item))
        .collect();
    assert_eq!(found.len(), 1, "{item} in {:?}", scan.sites);
    found.into_iter().next().cloned().expect("one key")
}

#[test]
fn a_newtype_inner_line_holds_on_the_field_the_constructor_and_the_accessor() {
    let scan = scanned(
        "pub struct Row(u32);\n\
         pub struct Name { text: String }\n\
         impl Row {\n\
             pub fn new(index: u32) -> Self { Row(index) }\n\
             pub fn checked(index: u32) -> Option<Self> { None }\n\
             pub fn parse(text: &str) -> Result<Row, Error> { todo() }\n\
             pub fn get(self) -> u32 { self.0 }\n\
             pub fn plus(self, step: u32) -> Self { self }\n\
         }\n\
         impl TryFrom<u8> for Row { fn try_from(value: u8) -> Result<Self, Self::Error> { x() } }\n\
         impl Name { pub fn as_str(&self) -> &str { &self.text } }",
    );
    for item in [
        "Row.0",
        "Name.text",
        "Row::new(index)",
        "Row::checked(index)",
        "Row::parse(text)",
        "Row::get()",
        "Row::plus(step)",
        "<Row as TryFrom<u8>>::try_from(value)",
        "Name::as_str()",
    ] {
        assert_eq!(scan.newtype_inner(&site(&scan, item)), Ok(()), "{item}");
    }
}

#[test]
fn a_newtype_inner_line_holds_on_a_type_a_define_macro_writes() {
    let scan = scanned(
        "define_index! { /// A row.\n pub struct Row(u32); }\n\
         define_length! { /// A length.\n DocPx(f32) }\n\
         impl Row { pub fn get(self) -> u32 { 0 } }\n\
         impl DocPx { pub fn new(value: f32) -> Self { x() } }",
    );
    for item in ["Row::get()", "DocPx::new(value)"] {
        assert_eq!(scan.newtype_inner(&site(&scan, item)), Ok(()), "{item}");
    }
}

#[test]
fn a_newtype_inner_line_is_refuted_off_a_single_field_struct() {
    let scan = scanned(
        "pub struct Rect { w: u32, h: u32 }\n\
         pub struct Row(u32);\n\
         pub enum Key { Char(char) }\n\
         pub enum Align { Left }\n\
         impl Align { pub fn code(self) -> u8 { 0 } }\n\
         impl Rect { pub fn new(w: u32) -> Self { x() } }\n\
         define_layout_version! { pub enum Shape { V1 = 1 } }\n\
         impl Shape { pub fn get(self) -> u8 { 1 } }\n\
         impl Row {\n\
             pub const MAX: u32 = 9;\n\
             pub fn width(index: u32) -> u32 { index }\n\
             pub fn zero() -> u32 { 0 }\n\
             pub fn get(self) -> u32 { fn inner(x: u32) {} 0 }\n\
         }\n\
         pub const TAG: u8 = 1;\n\
         pub fn free(index: u32) -> Row { Row(index) }",
    );
    for item in [
        "Rect.w",
        "Key::Char.0",
        "Align::code()",
        "Rect::new(w)",
        "Shape::get()",
        "Row::MAX",
        "Row::width(index)",
        "Row::zero()",
        "Row::get::inner(x)",
        "TAG",
        "free(index)",
    ] {
        assert!(scan.newtype_inner(&site(&scan, item)).is_err(), "{item}");
    }
}

#[test]
fn a_newtype_inner_line_reads_the_type_its_own_module_declares() {
    let scan = scanned(
        "mod a { pub struct Media(String); impl Media { pub fn as_str(&self) -> &str { x() } } }\n\
         mod b { pub enum Media { Image } impl Media { pub fn tag(self) -> &'static str { x() } } }",
    );
    assert_eq!(scan.newtype_inner(&site(&scan, "Media.0")), Ok(()));
    assert_eq!(scan.newtype_inner(&site(&scan, "Media::as_str()")), Ok(()));
    assert!(scan.newtype_inner(&site(&scan, "Media::tag()")).is_err());
}

#[test]
fn a_newtype_inner_line_is_refuted_in_an_impl_of_another_crate() {
    let mut tree = demo(&[("src/lib.rs", "pub struct Row(u32);")]);
    tree.0
        .insert(PathBuf::from(CRATES).join("user/Cargo.toml"), String::new());
    tree.0.insert(
        PathBuf::from(CRATES).join("user/src/lib.rs"),
        "impl Line for demo::Row { fn index(&self) -> usize { 0 } }".to_owned(),
    );
    let scan = scan(&tree, Path::new(CRATES));
    let refusal = scan.newtype_inner(&site(&scan, "<demo::Row as Line>::index()"));
    assert!(
        refusal
            .as_ref()
            .is_err_and(|reason| reason.contains("another crate")),
        "{refusal:?}"
    );
}

#[test]
fn a_trait_line_holds_on_a_foreign_trait_and_a_foreign_closure() {
    let scan = scanned(
        "pub struct Row(u32);\n\
         impl From<u32> for Row { fn from(value: u32) -> Self { Row(value) } }\n\
         impl std::str::FromStr for Row { fn from_str(s: &str) -> Result<Self, E> { x() } }\n\
         impl serde::Serialize for Row { const NAME: &'static str = \"row\"; }\n\
         pub fn open(go: &impl Fn(&str, leptos_router::NavigateOptions)) {}\n\
         pub fn hook(f: fn(u32, other::Event)) {}",
    );
    for item in [
        "<Row as std::str::FromStr>::from_str(s)",
        "<Row as serde::Serialize>::NAME",
        "open(go)",
        "hook(f)",
    ] {
        assert_eq!(scan.foreign_signature(&site(&scan, item)), Ok(()), "{item}");
    }
}

#[test]
fn a_trait_line_is_refuted_off_a_foreign_signature() {
    let scan = scanned(
        "pub trait Sink { fn put(&self, line: u64); }\n\
         pub struct Row(u32);\n\
         pub struct Session { sent: i32 }\n\
         impl Sink for Row { fn put(&self, line: u64) {} }\n\
         impl crate::glyph::Glyph for Row { fn size(&self) -> f32 { 0.0 } }\n\
         impl demo::Fixture for Row { fn seed(&self) -> u64 { 0 } }\n\
         impl From<u32> for Row { fn from(value: u32) -> Self { fn inner(step: u8) {} Row(value) } }\n\
         pub struct Px(f32);\n\
         impl core::ops::Add<u32> for Px { type Output = Px; fn add(self, rhs: u32) -> Px { self } }\n\
         pub fn follow(go: impl FnOnce(&str)) {}\n\
         pub fn wait(f: impl Fn(std::time::Duration, u32)) {}\n\
         pub fn plain(text: &str) {}",
    );
    for item in [
        "Sink::put(line)",
        "<Row as Sink>::put(line)",
        "<Row as crate::glyph::Glyph>::size()",
        "<Row as demo::Fixture>::seed()",
        "<Row as From<u32>>::from::inner(step)",
        "<Row as From<u32>>::from(value)",
        "<Px as core::ops::Add<u32>>::add(rhs)",
        "Session.sent",
        "follow(go)",
        "wait(f)",
        "plain(text)",
    ] {
        assert!(
            scan.foreign_signature(&site(&scan, item)).is_err(),
            "{item}"
        );
    }
}

#[test]
fn a_phantom_marker_is_no_field_of_a_newtype() {
    let scan = scanned(
        "pub struct Length<P> { pub value: f32, space: std::marker::PhantomData<P> }\n\
         pub struct Size<P> { pub width: u32, pub height: u32, space: PhantomData<P> }",
    );
    assert_eq!(scan.newtype_inner(&site(&scan, "Length.value")), Ok(()));
    assert!(scan.newtype_inner(&site(&scan, "Size.width")).is_err());
}

/// The tree of `demo` and two more crates. The crate `user` depends on `demo`,
/// and its `src/lib.rs` reads `user`. The crate `other` declares an `enum Row`
/// that `user` cannot reach.
fn with_user(demo_lib: &str, user: &str) -> Scan {
    let mut tree = demo(&[("src/lib.rs", demo_lib)]);
    tree.0.insert(
        PathBuf::from(CRATES).join("user/Cargo.toml"),
        "[dependencies]\ndemo = { path = \"../demo\" }\n".to_owned(),
    );
    tree.0.insert(
        PathBuf::from(CRATES).join("other/Cargo.toml"),
        String::new(),
    );
    tree.0.insert(
        PathBuf::from(CRATES).join("other/src/lib.rs"),
        "pub enum Row { A, B }".to_owned(),
    );
    tree.0.insert(
        PathBuf::from(CRATES).join("user/src/lib.rs"),
        user.to_owned(),
    );
    scan(&tree, Path::new(CRATES))
}

#[test]
fn a_newtype_inner_line_holds_in_an_impl_the_orphan_rule_places_in_the_trait_crate() {
    let scan = with_user(
        "pub struct Row(u32);\npub struct Rect { w: u32, h: u32 }",
        "pub trait Line { fn index(&self) -> usize; }\n\
         impl Line for demo::Row { fn index(&self) -> usize { 0 } }\n\
         impl crate::Line for demo::Rect { fn index(&self) -> usize { 0 } }",
    );
    assert_eq!(
        scan.newtype_inner(&site(&scan, "<demo::Row as Line>::index()")),
        Ok(())
    );
    assert!(
        scan.newtype_inner(&site(&scan, "<demo::Rect as crate::Line>::index()"))
            .is_err()
    );
}

#[test]
fn a_deref_to_a_primitive_is_refused_whatever_the_allowlist_says() {
    let scan = scanned(
        "pub struct Secret(Vec<u8>);\n\
         pub struct Name(String);\n\
         pub struct Editor { core: Core }\n\
         impl Deref for Secret { type Target = [u8]; fn deref(&self) -> &[u8] { &self.0 } }\n\
         impl DerefMut for Secret { fn deref_mut(&mut self) -> &mut [u8] { &mut self.0 } }\n\
         impl std::ops::Deref for Name { type Target = str; fn deref(&self) -> &str { &self.0 } }\n\
         impl Deref for Editor { type Target = Core; fn deref(&self) -> &Core { &self.core } }\n\
         impl DerefMut for Editor { fn deref_mut(&mut self) -> &mut Core { &mut self.core } }",
    );
    assert_eq!(scan.derefs.len(), 3, "{:?}", scan.derefs);
    assert!(
        scan.derefs
            .iter()
            .any(|refusal| refusal.contains("DerefMut of Secret"))
    );
    assert!(
        scan.derefs
            .iter()
            .all(|refusal| !refusal.contains("Editor"))
    );
}

/// A tree of several crates, each path given from the crates directory.
fn crates(files: &[(&str, &str)]) -> Scan {
    let tree = Memory(
        files
            .iter()
            .map(|(path, text)| (PathBuf::from(CRATES).join(path), (*text).to_owned()))
            .collect(),
    );
    let result = scan(&tree, Path::new(CRATES));
    assert_eq!(result.errors, Vec::<String>::new());
    result
}

#[test]
fn a_local_line_holds_on_a_private_site_only() {
    let scan = scanned(
        "pub struct A { pub x: u32, y: u32 }\n\
         pub(crate) fn f(n: u32) {}\n\
         fn g(n: u32) {}\n\
         pub(self) fn s(n: u32) {}\n\
         impl A { pub(super) fn h(&self, n: u32) {} fn k(&self, n: u32) {} }\n\
         impl From<u32> for A { fn from(v: u32) -> Self { x() } }\n\
         pub enum E { V(u32) }\n\
         enum F { W(u32) }\n\
         #[derive(Debug, serde::Serialize)]\nstruct S { z: u32 }\n\
         #[derive(sqlx::FromRow)]\nstruct R { z: u32 }",
    );
    for item in [
        "A.x",
        "f(n)",
        "A::h(n)",
        "<A as From<u32>>::from(v)",
        "E::V.0",
        "S.z",
        "R.z",
    ] {
        assert!(scan.local_site(&site(&scan, item)).is_err(), "{item}");
    }
    for item in ["A.y", "g(n)", "s(n)", "A::k(n)", "F::W.0"] {
        assert_eq!(scan.local_site(&site(&scan, item)), Ok(()), "{item}");
    }
}

#[test]
fn a_field_of_a_bus_type_is_not_private() {
    let scan = crates(&[
        ("ambre-bus/Cargo.toml", ""),
        (
            "ambre-bus/src/lib.rs",
            "struct Cmd { flag: bool, size: u32 }",
        ),
    ]);
    let flag = site(&scan, "Cmd.flag");
    assert!(scan.local_site(&flag).is_err());
    assert!(scan.bool_field(&flag).is_err());
    assert!(scan.local_site(&site(&scan, "Cmd.size")).is_err());
}

#[test]
fn a_bool_field_that_is_not_private_is_refused() {
    let scan = scanned(
        "pub struct A { pub on: bool, pub(crate) cell: Cell<bool>, open: Option<bool>, pub n: u32,\n\
         pub cb: Box<dyn Fn(bool)>, pub hook: fn(bool) -> u8 }\n\
         #[derive(Debug, serde::Deserialize)]\nstruct Claims { email_verified: bool }\n\
         pub enum Event { Moved { done: bool } }\n\
         struct Latch { set: bool }",
    );
    for item in [
        "A.on",
        "A.cell",
        "Claims.email_verified",
        "Event::Moved.done",
    ] {
        assert!(scan.bool_field(&site(&scan, item)).is_err(), "{item}");
    }
    for item in ["A.open", "A.n", "Latch.set", "A.cb", "A.hook"] {
        assert_eq!(scan.bool_field(&site(&scan, item)), Ok(()), "{item}");
    }
}

/// Every key of the scan, the allowlist a literal check reads beside.
fn all(scan: &Scan) -> std::collections::BTreeSet<SiteKey> {
    scan.sites.clone()
}

#[test]
fn a_literal_passed_to_a_bool_parameter_that_is_not_private_is_refused() {
    let scan = crates(&[
        ("demo/Cargo.toml", ""),
        (
            "demo/src/lib.rs",
            "pub fn set(on: bool) {}\n\
             fn quiet(on: bool) {}\n\
             pub fn pair(n: u32, on: bool) {}\n\
             pub fn never(on: bool) {}\n\
             pub struct W;\n\
             impl W { pub fn show(&self, on: bool) {} pub fn make(on: bool) -> W { W } }\n\
             fn run() { quiet(true); pair(true, 1); W::show(&W, false); }",
        ),
        (
            "demo/tests/t.rs",
            "fn t() {\n    assert!(set(true) == ());\n    W.show(true);\n    W::make(false);\n}",
        ),
    ]);
    let listed = all(&scan);
    for item in ["set(on)", "W::show(on)", "W::make(on)"] {
        assert!(
            scan.bool_literal(&site(&scan, item), &listed).is_err(),
            "{item}"
        );
    }
    for item in ["quiet(on)", "pair(on)", "never(on)"] {
        assert_eq!(
            scan.bool_literal(&site(&scan, item), &listed),
            Ok(()),
            "{item}"
        );
    }
    let refusal = scan.bool_literal(&site(&scan, "set(on)"), &listed);
    assert!(
        refusal
            .as_ref()
            .is_err_and(|reason| reason.contains("crates/demo/tests/t.rs:2")),
        "{refusal:?}"
    );
}

#[test]
fn a_literal_that_matches_two_sites_refutes_both() {
    let scan = scanned(
        "pub struct A;\npub struct B;\n\
         impl A { pub fn set_on(&self, on: bool) {} }\n\
         impl B { pub fn set_on(&self, on: bool) {} }\n\
         fn run(a: A) { a.set_on(true); }",
    );
    let listed = all(&scan);
    for item in ["A::set_on(on)", "B::set_on(on)"] {
        let refusal = scan.bool_literal(&site(&scan, item), &listed);
        assert!(
            refusal
                .as_ref()
                .is_err_and(|reason| reason.contains("match another site")),
            "{item}: {refusal:?}"
        );
    }
}

#[test]
fn a_literal_reaches_a_pub_site_of_a_crate_it_depends_on() {
    let scan = crates(&[
        ("demo/Cargo.toml", ""),
        (
            "demo/src/lib.rs",
            "pub fn far(on: bool) {}\npub(crate) fn near(on: bool) {}\npub fn lone(on: bool) {}",
        ),
        (
            "user/Cargo.toml",
            "[package]\nname = \"user\"\n\n[dev-dependencies]\ndemo = { path = \"../demo\" }",
        ),
        (
            "user/src/lib.rs",
            "fn go() { demo::far(true); near(true); }",
        ),
        ("stranger/Cargo.toml", "[dependencies]\nother = \"1\""),
        ("stranger/src/lib.rs", "fn go() { lone(false); }"),
    ]);
    let listed = all(&scan);
    assert!(scan.bool_literal(&site(&scan, "far(on)"), &listed).is_err());
    for item in ["near(on)", "lone(on)"] {
        assert_eq!(
            scan.bool_literal(&site(&scan, item), &listed),
            Ok(()),
            "{item}"
        );
    }
}

#[test]
fn a_trait_imported_under_another_name_is_the_workspace_trait() {
    let scan = crates(&[
        ("demo/Cargo.toml", ""),
        (
            "demo/src/lib.rs",
            "pub trait Tree { fn at(&self, i: u32); }",
        ),
        (
            "user/Cargo.toml",
            "[dependencies]\ndemo = { path = \"../demo\" }",
        ),
        (
            "user/src/lib.rs",
            "use demo::Tree as T;\npub struct P;\nimpl T for P { fn at(&self, i: u32) {} }",
        ),
    ]);
    let refusal = scan.foreign_signature(&site(&scan, "<P as T>::at(i)"));
    assert!(
        refusal
            .as_ref()
            .is_err_and(|reason| reason.contains("declared in the workspace")),
        "{refusal:?}"
    );
}

#[test]
fn an_alias_resolves_to_its_target_before_every_check() {
    let scan = crates(&[
        ("demo/Cargo.toml", ""),
        (
            "demo/src/lib.rs",
            "pub type Flag = bool;\n\
             pub struct A { pub f: Flag }\n\
             pub fn set(on: Flag) {}\n\
             pub mod inner { pub type Id = u32; pub struct B { pub id: Id, pub up: super::Flag } }\n\
             fn run() { set(true); }",
        ),
        (
            "user/Cargo.toml",
            "[dependencies]\ndemo = { path = \"../demo\" }",
        ),
        ("user/src/lib.rs", "pub struct U { pub g: demo::Flag }"),
    ]);
    let keys: Vec<String> = scan.sites.iter().map(ToString::to_string).collect();
    for written in [
        "demo | src/lib.rs | crate | A.f | field | Flag",
        "demo | src/lib.rs | crate | set(on) | param | Flag",
        "demo | src/lib.rs | crate::inner | B.id | field | Id",
        "demo | src/lib.rs | crate::inner | B.up | field | super::Flag",
        "user | src/lib.rs | crate | U.g | field | demo::Flag",
    ] {
        assert!(
            keys.iter().any(|key| key == written),
            "{written} in {keys:?}"
        );
    }
    for item in ["A.f", "B.up", "U.g"] {
        assert!(scan.bool_field(&site(&scan, item)).is_err(), "{item}");
    }
    let listed = all(&scan);
    assert!(scan.bool_literal(&site(&scan, "set(on)"), &listed).is_err());
}

#[test]
fn a_deref_target_is_read_through_paths_wrappers_aliases_generics_and_macros() {
    let scan = scanned(
        "pub struct A(String);\nimpl std::ops::Deref for A { type Target = std::string::String; }\n\
         pub struct B(u32);\nimpl Deref for B { type Target = std::primitive::u32; }\n\
         pub struct C(Box<str>);\nimpl Deref for C { type Target = Box<str>; }\n\
         pub struct D(Rc<str>);\nimpl Deref for D { type Target = Rc<str>; }\n\
         pub struct E(Vec<u8>);\nimpl Deref for E { type Target = std::vec::Vec<u8>; }\n\
         pub struct F([[u8; 4]; 4]);\nimpl Deref for F { type Target = [[u8; 4]; 4]; }\n\
         type Raw = u64;\npub struct G(Raw);\nimpl Deref for G { type Target = Raw; }\n\
         pub struct W<T>(T);\nimpl<T> Deref for W<T> { type Target = T; }\n\
         pub struct Uses { w: W<u32> }\n\
         pub struct V<T>(T);\nimpl<T: AsRef<str>> Deref for V<T> { type Target = T; }\n\
         pub struct Q<T>(T);\nimpl<T: Shape> Deref for Q<T> { type Target = <T as Shape>::Out; }\n\
         macro_rules! wrap { ($name:ident) => { impl Deref for $name { type Target = str; } }; }\n\
         pub struct M<T>(T);\nimpl<T> Deref for M<T> { type Target = T; }\n\
         pub struct Editor { core: Core }\nimpl Deref for Editor { type Target = Core; }",
    );
    let refused = [
        "A",
        "B",
        "C",
        "D",
        "E",
        "F",
        "G",
        "W<T>",
        "V<T>",
        "Q<T>",
        "__ambre_meta_name",
    ];
    for ty in refused {
        assert!(
            scan.derefs
                .iter()
                .any(|refusal| refusal.contains(&format!("Deref of {ty} to"))
                    || refusal.contains(&format!("Deref of {ty}:"))),
            "{ty} in {:?}",
            scan.derefs
        );
    }
    assert_eq!(scan.derefs.len(), refused.len(), "{:?}", scan.derefs);
}

#[test]
fn a_literal_is_found_through_reexports_renames_macros_and_props() {
    let scan = crates(&[
        ("core-a/Cargo.toml", ""),
        (
            "core-a/src/lib.rs",
            "pub fn real(on: bool) {}\n\
             pub fn opt(on: Option<bool>) {}\n\
             pub fn three(on: bool) {}\n\
             pub fn sel(on: bool) {}\n\
             pub fn rep(on: bool) -> u8 { 0 }\n\
             #[component]\npub fn Toggle(checked: bool, label: u32) -> impl IntoView { () }\n\
             pub fn quiet(on: bool) {}",
        ),
        (
            "mid/Cargo.toml",
            "[dependencies]\ncore-a = { path = \"../core-a\" }",
        ),
        ("mid/src/lib.rs", "pub use core_a::real;"),
        (
            "far/Cargo.toml",
            "[dependencies]\nmid = { path = \"../mid\" }",
        ),
        ("far/src/lib.rs", "fn go() { mid::real(true); }"),
        (
            "app/Cargo.toml",
            "[dependencies]\nrenamed = { package = \"core-a\", path = \"../core-a\" }",
        ),
        (
            "app/src/lib.rs",
            "fn go() {\n\
             renamed::opt(Some(false));\n\
             renamed::three((true));\n\
             let v = vec![renamed::rep(true); 2];\n\
             tokio::select! { _ = x => renamed::sel(false), }\n\
             view! { <renamed::Toggle checked=true label=1/> <Other on:click=true/> }\n\
             }",
        ),
    ]);
    let listed = all(&scan);
    for item in [
        "real(on)",
        "opt(on)",
        "three(on)",
        "sel(on)",
        "rep(on)",
        "Toggle(checked)",
    ] {
        assert!(
            scan.bool_literal(&site(&scan, item), &listed).is_err(),
            "{item}"
        );
    }
    assert_eq!(
        scan.bool_literal(&site(&scan, "quiet(on)"), &listed),
        Ok(())
    );
}

#[test]
fn the_one_bool_of_a_tuple_variant_is_the_value_it_names() {
    let scan = scanned(
        "pub enum CellValue { Boolean(bool), Maybe(Option<bool>), Pair(bool, u8), Named { on: bool } }",
    );
    assert_eq!(
        scan.bool_field(&site(&scan, "CellValue::Boolean.0")),
        Ok(())
    );
    for item in [
        "CellValue::Maybe.0",
        "CellValue::Pair.0",
        "CellValue::Named.on",
    ] {
        assert!(scan.bool_field(&site(&scan, item)).is_err(), "{item}");
    }
}

#[test]
fn a_private_field_of_a_pub_struct_is_private_off_every_wire() {
    let scan = scanned(
        "pub struct P { on: bool, n: u32 }\n\
         #[derive(serde::Serialize)]\npub struct Q { on: bool }",
    );
    assert_eq!(scan.bool_field(&site(&scan, "P.on")), Ok(()));
    assert_eq!(scan.local_site(&site(&scan, "P.n")), Ok(()));
    assert!(scan.bool_field(&site(&scan, "Q.on")).is_err());
}
