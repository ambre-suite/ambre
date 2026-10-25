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
        "<Row as From<u32>>::from(value)",
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
