use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::{Scan, SiteKey, Tree, is_build_output, lexical, scan};

/// A source tree held in memory, so a test states every file it reads.
struct Memory(BTreeMap<PathBuf, String>);

impl Tree for Memory {
    // As on disk, `..` steps out of a directory.
    fn read(&self, path: &Path) -> Option<String> {
        self.0.get(&lexical(path)).cloned()
    }

    // As on disk, a file under a build output directory is not listed.
    fn files(&self, dir: &Path) -> Vec<PathBuf> {
        self.0
            .keys()
            .filter(|path| {
                path.strip_prefix(dir).is_ok_and(|inside| {
                    !inside
                        .parent()
                        .is_some_and(|parent| parent.iter().any(is_build_output))
                })
            })
            .cloned()
            .collect()
    }

    // No link: the path with `..` undone is the file.
    fn canonical(&self, path: &Path) -> PathBuf {
        lexical(path)
    }
}

/// A tree in memory whose links send a file to another, as a symbolic link
/// on disk does.
struct Linked {
    tree: Memory,
    links: BTreeMap<PathBuf, PathBuf>,
}

impl Tree for Linked {
    fn read(&self, path: &Path) -> Option<String> {
        self.tree.read(&self.canonical(path))
    }

    fn files(&self, dir: &Path) -> Vec<PathBuf> {
        let mut files = self.tree.files(dir);
        files.extend(
            self.links
                .keys()
                .filter(|link| link.starts_with(dir))
                .cloned(),
        );
        files.sort();
        files
    }

    fn canonical(&self, path: &Path) -> PathBuf {
        let path = lexical(path);
        self.links.get(&path).cloned().unwrap_or(path)
    }
}

const CRATES: &str = "/w/crates";

/// One crate named `demo`, its files given relative to the crate directory.
fn demo(files: &[(&str, &str)]) -> Memory {
    named("demo", files)
}

/// One crate named `krate`, its files given relative to the crate directory.
fn named(krate: &str, files: &[(&str, &str)]) -> Memory {
    let mut tree = BTreeMap::new();
    tree.insert(
        PathBuf::from(CRATES).join(krate).join("Cargo.toml"),
        String::new(),
    );
    for (path, text) in files {
        tree.insert(
            PathBuf::from(CRATES).join(krate).join(path),
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
            "#[cfg(test)]\nmod a;\n\
             #[cfg(all(test, feature = \"pdf\"))]\nmod b;",
        ),
        ("src/a.rs", "pub fn f(x: u32) {}"),
        ("src/b.rs", "pub fn f(x: u32) {}"),
    ]);
    assert_eq!(found, Vec::<String>::new());
}

#[test]
fn a_feature_named_for_tests_is_no_test_build() {
    // A feature can be turned on in a release build, whatever its name.
    let found = keys(&[
        (
            "src/lib.rs",
            "#[cfg(any(test, feature = \"testing\"))]\nmod a;\n\
             #[cfg(feature = \"test-helpers\")]\nmod c;",
        ),
        ("src/a.rs", "pub fn f(x: u32) {}"),
        ("src/c.rs", "pub fn f(x: u32) {}"),
    ]);
    assert_eq!(found.len(), 2);
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
fn a_tests_module_with_no_test_cfg_is_scanned() {
    // The name is no test gate: `mod grid_tests;` alone is built.
    let found = keys(&[
        ("src/lib.rs", "mod grid_tests;"),
        ("src/grid_tests.rs", "fn f(x: u32) {}"),
    ]);
    assert_eq!(
        found,
        ["demo | src/grid_tests.rs | crate::grid_tests | f(x) | param | u32"]
    );
    let skipped = keys(&[
        ("src/lib.rs", "#[cfg(test)]\nmod grid_tests;"),
        ("src/grid_tests.rs", "fn f(x: u32) {}"),
    ]);
    assert_eq!(skipped, Vec::<String>::new());
}

#[test]
fn a_secret_read_in_a_tests_module_with_no_test_cfg_is_refused() {
    // Probe p2 of the SEC-1 review: this module is built, so its read counts.
    let result = scan(
        &demo(&[
            (
                "src/lib.rs",
                "mod leak_tests;\n#[cfg(test)]\nmod real_tests;",
            ),
            (
                "src/leak_tests.rs",
                "pub fn a(s: &Secret<String>) -> String { s.expose().clone() }",
            ),
            (
                "src/real_tests.rs",
                "pub fn b(s: &Secret<String>) -> String { s.expose().clone() }",
            ),
        ]),
        Path::new(CRATES),
    );
    assert_eq!(result.errors, Vec::<String>::new());
    assert_eq!(result.secrets.len(), 1, "{:?}", result.secrets);
    assert!(
        result.secrets[0].starts_with("crates/demo/src/leak_tests.rs:1: `expose`"),
        "{}",
        result.secrets[0]
    );
}

#[test]
fn a_path_inside_a_cfg_attr_fails_the_scan_at_any_depth() {
    // The probe of the SEC-1c/1d review. Rustc compiles `leak.rs`, outside
    // the crate. A walk that drops the `cfg_attr` reads the harmless
    // `innocent/mod.rs` instead, and reports no read of a secret.
    let result = scan(
        &demo(&[
            (
                "src/lib.rs",
                "#[cfg_attr(all(), path = \"../../../leak.rs\")] mod innocent;\n\
                 #[cfg_attr(all(), cfg_attr(unix, path = \"deep.rs\"))] mod deeper;\n\
                 #[cfg(test)]\n#[cfg_attr(all(), path = \"t.rs\")] mod checks;",
            ),
            ("src/innocent/mod.rs", "pub fn f() {}"),
            ("src/deeper.rs", "pub fn g() {}"),
        ]),
        Path::new(CRATES),
    );
    for module in ["crate::innocent", "crate::deeper", "crate::checks"] {
        assert!(
            result.errors.iter().any(|error| error.contains(&format!(
                "the module {module} is placed by a `path` inside a `cfg_attr`"
            ))),
            "{module}: {:?}",
            result.errors
        );
    }
}

#[test]
fn a_path_outside_the_crate_to_another_kind_or_under_target_fails_the_scan() {
    let (_, errors) = run(&[
        (
            "src/lib.rs",
            "#[path = \"../../../leak.rs\"] mod outside;\n\
             #[path = \"leak.txt\"] mod text;\n\
             #[path = \"target/leak.rs\"] mod built;",
        ),
        (
            "src/leak.txt",
            "pub fn f(s: &Secret<String>) { s.expose(); }",
        ),
        (
            "src/target/leak.rs",
            "pub fn f(s: &Secret<String>) { s.expose(); }",
        ),
    ]);
    // One refusal each. The file under `target/` is no orphan either: the
    // walk and the orphan check leave out the same directories.
    assert_eq!(errors.len(), 3, "{errors:?}");
    for (module, why) in [
        ("outside", "a file outside the directory of the crate"),
        ("text", "a file that is not `.rs`"),
        ("built", "a file under a `target/` directory"),
    ] {
        assert!(
            errors.iter().any(|error| error.contains(&format!(
                "crate::{module} is placed by a `#[path]` to {why}"
            ))),
            "{module}: {errors:?}"
        );
    }
}

#[test]
fn a_path_on_an_inline_module_or_off_a_string_literal_fails_the_scan() {
    let (_, errors) = run(&[
        (
            "src/lib.rs",
            "#[path = \"elsewhere\"] mod inner { mod leaf; }\n\
             #[path = concat!(\"a\", \".rs\")] mod a;",
        ),
        ("src/inner/leaf.rs", "pub fn f() {}"),
        ("src/a.rs", "pub fn g() {}"),
    ]);
    assert!(
        errors.iter().any(
            |error| error.contains("crate::inner is placed by a `#[path]` on an inline module")
        ),
        "{errors:?}"
    );
    assert!(
        errors.iter().any(|error| error
            .contains("crate::a is placed by a `path` attribute that is not one string literal")),
        "{errors:?}"
    );
}

#[test]
fn a_plain_path_inside_the_crate_is_read_and_its_secret_read_refused() {
    let result = scan(
        &demo(&[
            ("src/lib.rs", "#[path = \"other/real.rs\"] mod renamed;"),
            (
                "src/other/real.rs",
                "pub fn a(s: &Secret<String>) -> String { s.expose().clone() }",
            ),
        ]),
        Path::new(CRATES),
    );
    assert_eq!(result.errors, Vec::<String>::new());
    assert_eq!(result.secrets.len(), 1, "{:?}", result.secrets);
    assert!(
        result.secrets[0].starts_with("crates/demo/src/other/real.rs:1: `expose`"),
        "{}",
        result.secrets[0]
    );
}

#[test]
fn a_test_only_file_keeps_the_modules_it_declares_out() {
    let (found, errors) = run(&[
        ("src/lib.rs", "mod probe;"),
        ("src/probe.rs", "#![cfg(test)]\nmod deeper;"),
        ("src/probe/deeper.rs", "fn f(x: u32) {}"),
    ]);
    assert_eq!(found, Vec::<String>::new());
    assert_eq!(errors, Vec::<String>::new());
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
fn the_walk_records_each_file_it_reads_per_crate_root() {
    // The dep-info check holds this record to rustc. A file under
    // `#![cfg(test)]` is read, as rustc reads it, and a test module is not.
    let result = scan(
        &demo(&[
            (
                "src/lib.rs",
                "mod a;
mod gated;
#[cfg(test)]
mod tests;",
            ),
            ("src/main.rs", "mod a;"),
            ("src/a.rs", "pub fn f(x: u32) {}"),
            (
                "src/gated.rs",
                "#![cfg(test)]
mod deeper;",
            ),
            ("src/gated/deeper.rs", ""),
            ("src/tests.rs", ""),
            ("src/orphan.rs", "pub fn f(x: u32) {}"),
        ]),
        Path::new(CRATES),
    );
    assert_eq!(result.errors, Vec::<String>::new());
    let src = Path::new(CRATES).join("demo/src");
    let files =
        |names: &[&str]| -> BTreeSet<PathBuf> { names.iter().map(|name| src.join(name)).collect() };
    assert_eq!(
        result.walked,
        BTreeMap::from([
            (src.join("lib.rs"), files(&["lib.rs", "a.rs", "gated.rs"])),
            (src.join("main.rs"), files(&["main.rs", "a.rs"])),
        ])
    );
    // A file two roots declare holds its sites once.
    let a = result
        .sites
        .iter()
        .filter(|key| key.to_string().contains("src/a.rs"))
        .count();
    assert_eq!(a, 1);
}

#[test]
fn the_walk_records_the_literal_target_of_each_data_read_per_crate_root() {
    // The dep-info check admits a file the walk does not read only when a
    // string literal of a walked file names it as data. rustc
    // resolves the literal beside the file that calls the macro, an inline
    // module's file included, and `sqlx` resolves a `migrate!` under the
    // crate directory.
    let result = scan(
        &demo(&[
            (
                "src/lib.rs",
                "#![doc = include_str!(\"../README.md\")]
pub const A: &str = include_str!(\"a.txt\");
pub const B: &[u8] = ::core::include_bytes!(r\"../data/b.bin\",);
pub fn c() -> Vec<&'static str> { vec![std::include_str!(\"c.txt\")] }
mod inner { pub const D: &str = include_str!(\"d.txt\"); }
pub const BUILT: &str = include_str!(concat!(\"e\", \".txt\"));
macro_rules! m { ($p:expr) => { include_str!($p) }; }
#[cfg(test)]
const T: &str = include_str!(\"t.txt\");
#[cfg(test)]
mod tests { const U: &str = include_str!(\"u.txt\"); }
pub async fn run() { sqlx::migrate!(\"./migrations\"); }
mod sub;",
            ),
            ("src/sub.rs", "pub const S: &str = include_str!(\"s.txt\");"),
            ("src/main.rs", "fn main() { sqlx::migrate!(); }"),
            ("migrations/1_init.sql", ""),
            ("migrations/deeper/2_more.sql", ""),
        ]),
        Path::new(CRATES),
    );
    assert_eq!(result.errors, Vec::<String>::new());
    let dir = Path::new(CRATES).join("demo");
    let files =
        |names: &[&str]| -> BTreeSet<PathBuf> { names.iter().map(|name| dir.join(name)).collect() };
    assert_eq!(
        result.data,
        BTreeMap::from([
            (
                dir.join("src/lib.rs"),
                files(&[
                    "README.md",
                    "src/a.txt",
                    "data/b.bin",
                    "src/c.txt",
                    "src/d.txt",
                    "src/s.txt",
                    "migrations/1_init.sql",
                ])
            ),
            (dir.join("src/main.rs"), files(&["migrations/1_init.sql"])),
        ])
    );
}

#[test]
fn a_raw_path_attribute_is_followed_and_a_raw_cfg_attr_fails_the_scan() {
    // Probe of the SEC-1e review: rustc reads `#[r#path]` as `#[path]`, and
    // `#[r#cfg_attr]` as `#[cfg_attr]`.
    let result = scan(
        &demo(&[
            (
                "src/lib.rs",
                "#[r#path = \"elsewhere/real.rs\"] mod raw;\n\
                 #[r#cfg_attr(all(), path = \"../../../leak.rs\")] mod hidden;",
            ),
            (
                "src/elsewhere/real.rs",
                "pub fn a(s: &Secret<String>) -> String { s.expose().clone() }",
            ),
            ("src/hidden.rs", ""),
        ]),
        Path::new(CRATES),
    );
    assert_eq!(result.errors.len(), 1, "{:?}", result.errors);
    assert!(
        result.errors[0]
            .contains("the module crate::hidden is placed by a `path` inside a `cfg_attr`"),
        "{:?}",
        result.errors
    );
    assert_eq!(result.secrets.len(), 1, "{:?}", result.secrets);
    assert!(
        result.secrets[0].starts_with("crates/demo/src/elsewhere/real.rs:1: `expose`"),
        "{}",
        result.secrets[0]
    );
}

#[test]
fn a_file_a_path_attribute_loads_holds_its_modules_beside_it() {
    // Probe of the SEC-1e review: rustc reads `a/child.rs`, as for a
    // `mod.rs`. A walk that reads `a/file/child.rs` scans the decoy.
    let result = scan(
        &demo(&[
            ("src/lib.rs", "#[path = \"a/file.rs\"] mod file;"),
            ("src/a/file.rs", "mod child;"),
            (
                "src/a/child.rs",
                "pub fn a(s: &Secret<String>) -> String { s.expose().clone() }",
            ),
            ("src/a/file/child.rs", "pub fn decoy() {}"),
        ]),
        Path::new(CRATES),
    );
    assert_eq!(result.errors, Vec::<String>::new());
    assert_eq!(result.secrets.len(), 1, "{:?}", result.secrets);
    assert!(
        result.secrets[0].starts_with("crates/demo/src/a/child.rs:1: `expose`"),
        "{}",
        result.secrets[0]
    );
}

#[test]
fn a_dot_dot_out_of_the_relay_secrets_is_no_exemption() {
    // Probe of the SEC-1e review: the raw path `src/secrets/../../leak.rs`
    // starts with `src/secrets/`, and the file sits at the crate root.
    let result = scan(
        &named(
            "ambre-server",
            &[
                (
                    "src/lib.rs",
                    "mod secrets;\n#[path = \"secrets/../../leak.rs\"] mod leak;",
                ),
                (
                    "src/secrets/mod.rs",
                    "pub fn held(s: &Secret<String>) -> String { s.expose().clone() }",
                ),
                (
                    "leak.rs",
                    "pub fn a(s: &Secret<String>) -> String { s.expose().clone() }",
                ),
            ],
        ),
        Path::new(CRATES),
    );
    assert_eq!(result.errors, Vec::<String>::new());
    assert_eq!(result.secrets.len(), 1, "{:?}", result.secrets);
    assert!(
        result.secrets[0].starts_with("crates/ambre-server/leak.rs:1: `expose`"),
        "{}",
        result.secrets[0]
    );
}

#[test]
fn a_link_out_of_the_relay_secrets_is_no_exemption() {
    // `src/secrets/token.rs` names a file of the relay's secrets module, and
    // the disk holds `src/leak.rs`: the exemptions key on both.
    let dir = Path::new(CRATES).join("ambre-server");
    let result = scan(
        &Linked {
            tree: named(
                "ambre-server",
                &[
                    ("src/lib.rs", "mod secrets;"),
                    ("src/secrets/mod.rs", "mod token;"),
                    (
                        "src/leak.rs",
                        "pub fn a(s: &Secret<String>) -> String { s.expose().clone() }\n\
                         pub fn b() { refresh_key(); }",
                    ),
                ],
            ),
            links: BTreeMap::from([(dir.join("src/secrets/token.rs"), dir.join("src/leak.rs"))]),
        },
        Path::new(CRATES),
    );
    assert_eq!(result.errors, Vec::<String>::new());
    let named: Vec<&str> = result
        .secrets
        .iter()
        .filter_map(|secret| secret.split_once(" is named here").map(|(head, _)| head))
        .collect();
    assert_eq!(
        named,
        [
            "crates/ambre-server/src/secrets/token.rs:1: `expose`",
            "crates/ambre-server/src/secrets/token.rs:2: `refresh_key`"
        ],
        "{:?}",
        result.secrets
    );
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
         mod x { pub(in crate::x) fn p(n: u32) {} }\n\
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
        "p(n)",
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
        "M<T>",
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

#[test]
fn an_alias_a_module_path_names_resolves_through_a_child_module_and_a_use() {
    let scan = scanned(
        "mod flags { pub type Yes = bool; }\n\
         pub struct S { pub on: flags::Yes }\n\
         mod deep { pub mod inner { pub type No = bool; } }\n\
         use deep::inner;\n\
         pub struct T { pub off: inner::No }",
    );
    for item in ["S.on", "T.off"] {
        assert!(scan.bool_field(&site(&scan, item)).is_err(), "{item}");
    }
}

#[test]
fn an_alias_path_the_gate_cannot_place_fails_the_scan() {
    let (_, errors) = run(&[(
        "src/lib.rs",
        "mod flags { pub type Yes = bool; }\nmod empty {}\npub struct X { pub on: empty::Yes }",
    )]);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors
            .iter()
            .all(|error| error.contains("X.on") && error.contains("fails closed")),
        "{errors:?}"
    );
}

#[test]
fn an_alias_argument_resolves_where_the_use_writes_it() {
    let scan = crates(&[
        ("demo/Cargo.toml", ""),
        (
            "demo/src/lib.rs",
            "pub mod m { pub type Wrap<T> = Option<T>; pub type T = u32; }\n\
             mod n { type Own = bool; pub struct G { pub on: crate::m::Wrap<Own> } }\n\
             pub struct Name;\n\
             pub struct H { pub n: m::Wrap<Name> }",
        ),
        (
            "user/Cargo.toml",
            "[dependencies]\ndemo = { path = \"../demo\" }",
        ),
        (
            "user/src/lib.rs",
            "type Mine = bool;\npub struct Cross { pub on: demo::m::Wrap<Mine> }",
        ),
    ]);
    for item in ["G.on", "Cross.on"] {
        assert!(scan.bool_field(&site(&scan, item)).is_err(), "{item}");
    }
    // The parameter `T` of `Wrap` stands for the argument, never for the
    // alias `T` its module declares.
    assert!(
        scan.sites
            .iter()
            .all(|key| !key.to_string().contains("H.n")),
        "{:?}",
        scan.sites
    );
}

#[test]
fn a_use_as_rename_is_undone_before_the_alias_lookup() {
    let scan = scanned(
        "mod flags { pub type Yes = bool; }\n\
         use flags::Yes as Renamed;\n\
         use std::string::String as Text;\n\
         pub struct R { pub on: Renamed, pub t: Text }",
    );
    assert!(scan.bool_field(&site(&scan, "R.on")).is_err());
    assert!(scan.local_site(&site(&scan, "R.t")).is_err());
}

#[test]
fn an_alias_chain_resolves_whole_and_a_cycle_fails_the_scan() {
    let mut chain = String::from("type L0 = bool;\n");
    for i in 1..10 {
        chain.push_str(&format!("type L{i} = L{};\n", i - 1));
    }
    chain.push_str("pub struct C { pub on: L9 }");
    let scan = scanned(&chain);
    assert!(scan.bool_field(&site(&scan, "C.on")).is_err());
    let (_, errors) = run(&[("src/lib.rs", "type L = Vec<L>;\npub struct D { pub l: L }")]);
    assert!(
        errors
            .iter()
            .any(|error| error.contains("D.l") && error.contains("more than 32 aliases")),
        "{errors:?}"
    );
}

#[test]
fn an_alias_that_opens_too_many_aliases_fails_the_scan_and_ends() {
    let (_, errors) = run(&[(
        "src/lib.rs",
        "type A = (A, A, A, A, A, A, A, A, A, A);\npub struct E { pub a: A }",
    )]);
    assert!(
        errors.iter().any(|error| error.contains("E.a")),
        "{errors:?}"
    );
    let mut wide = String::from("type W0 = bool;\n");
    for i in 1..6 {
        let p = i - 1;
        wide.push_str(&format!(
            "type W{i} = (W{p}, W{p}, W{p}, W{p}, W{p}, W{p}, W{p}, W{p});\n"
        ));
    }
    wide.push_str("pub struct F { pub w: W5 }");
    let (_, errors) = run(&[("src/lib.rs", &wide)]);
    assert!(
        errors
            .iter()
            .any(|error| error.contains("F.w") && error.contains("more than 4096 aliases")),
        "{errors:?}"
    );
}

#[test]
fn an_imported_name_that_is_no_alias_shadows_an_alias_of_that_name() {
    let scan = crates(&[
        ("demo/Cargo.toml", ""),
        (
            "demo/src/lib.rs",
            "pub type Id = u32;\npub mod b { pub struct Id; }",
        ),
        (
            "user/Cargo.toml",
            "[dependencies]\ndemo = { path = \"../demo\" }\nforeign = \"1\"",
        ),
        (
            "user/src/lib.rs",
            "mod b { pub struct Id; }\n\
             mod c { use crate::b::Id; pub struct T { pub id: Id } }\n\
             mod d { use foreign::Id; pub struct U { pub id: Id } }\n\
             mod e { use demo::b::Id; pub struct V { pub id: Id } }\n\
             use demo::Id as Num;\n\
             pub struct P { pub n: Num }",
        ),
    ]);
    let keys: Vec<String> = scan.sites.iter().map(ToString::to_string).collect();
    for item in ["T.id", "U.id", "V.id"] {
        assert!(
            keys.iter().all(|key| !key.contains(item)),
            "{item} in {keys:?}"
        );
    }
    assert!(scan.local_site(&site(&scan, "P.n")).is_err());
}

#[test]
fn a_deref_a_macro_rules_body_writes_is_read_through_its_fragments_or_fails_closed() {
    let scan = scanned(
        "macro_rules! newtype {\n\
             ($v:vis $name:ident, $l:lifetime, $b:block) => {\n\
                 $v struct $name(u32);\n\
                 impl<$l> core::ops::Deref for $name { type Target = u32; fn deref(&self) -> &u32 $b }\n\
             };\n\
         }\n\
         macro_rules! odd { ($t:tt) => { impl Deref for X { type Target = u8; } => => }; }",
    );
    assert_eq!(scan.derefs.len(), 2, "{:?}", scan.derefs);
    assert!(
        scan.derefs
            .iter()
            .any(|refusal| refusal.contains("Deref of __ambre_meta_name to u32")),
        "{:?}",
        scan.derefs
    );
    assert!(
        scan.derefs
            .iter()
            .any(|refusal| refusal.contains("does not parse")),
        "{:?}",
        scan.derefs
    );
}

#[test]
fn a_deref_nested_in_a_constant_a_static_or_a_macro_constant_is_read() {
    let scan = scanned(
        "pub struct K(u8);\n\
         const _: () = { impl core::ops::Deref for K { type Target = u8; fn deref(&self) -> &u8 { &self.0 } } };\n\
         macro_rules! inner { () => { const _: () = { impl core::ops::Deref for crate::K2 { type Target = u8; } }; }; }\n\
         pub struct K2(u8);\n\
         static S: () = { impl core::ops::Deref for K3 { type Target = u8; } };\n\
         pub struct K3(u8);",
    );
    for ty in ["K", "crate::K2", "K3"] {
        assert!(
            scan.derefs
                .iter()
                .any(|refusal| refusal.contains(&format!("Deref of {ty} to u8"))),
            "{ty} in {:?}",
            scan.derefs
        );
    }
    assert_eq!(scan.derefs.len(), 3, "{:?}", scan.derefs);
}

#[test]
fn a_generic_deref_target_fails_closed_bound_or_not() {
    let scan = scanned(
        "pub struct Body<T>(T);\nimpl<T> Deref for Body<T> { type Target = T; }\n\
         fn f() { let b: Body<u32> = Body(1); }\n\
         pub struct P<T>(T);\nimpl<T: Tr> Deref for P<T> { type Target = T::Assoc; }\n\
         pub struct V<T>(T);\nimpl<T: AsRef<str>> Deref for V<T> { type Target = T; }",
    );
    for (ty, why) in [
        ("Body<T>", "the type parameter T"),
        ("P<T>", "an associated type"),
        ("V<T>", "names str"),
    ] {
        assert!(
            scan.derefs
                .iter()
                .any(|refusal| refusal.contains(&format!("Deref of {ty}")) && refusal.contains(why)),
            "{ty} in {:?}",
            scan.derefs
        );
    }
}

#[test]
fn a_deref_mut_takes_the_target_of_the_deref_in_its_own_module() {
    let scan = scanned(
        "mod a { pub struct T(u32); impl Deref for T { type Target = u32; } impl DerefMut for T {} }\n\
         mod b { pub struct T(Core); impl Deref for T { type Target = Core; } impl DerefMut for T {} }",
    );
    assert_eq!(scan.derefs.len(), 2, "{:?}", scan.derefs);
    for refused in ["Deref of T to u32", "DerefMut of T to u32"] {
        assert_eq!(
            scan.derefs
                .iter()
                .filter(|refusal| refusal.contains(refused))
                .count(),
            1,
            "{refused} in {:?}",
            scan.derefs
        );
    }
    assert!(
        scan.derefs.iter().all(|refusal| !refusal.contains("Core")),
        "{:?}",
        scan.derefs
    );
}

#[test]
fn a_literal_is_found_behind_a_reference_a_not_a_block_a_tuple_an_array_and_a_rename() {
    let scan = crates(&[
        ("demo/Cargo.toml", ""),
        (
            "demo/src/lib.rs",
            "pub fn by_ref(on: &bool) {}\n\
             pub fn by_tuple(p: (bool, u8)) {}\n\
             pub fn by_block(on: bool) {}\n\
             pub fn by_not(on: bool) {}\n\
             pub fn by_array(on: [bool; 1]) {}\n\
             pub fn by_vec(on: Vec<bool>) {}\n\
             pub fn by_repeat(on: [bool; 2]) {}\n\
             pub fn ctrl(on: bool) {}",
        ),
        (
            "user/Cargo.toml",
            "[dependencies]\ndemo = { path = \"../demo\" }",
        ),
        (
            "user/src/lib.rs",
            "use demo::ctrl as c;\n\
             fn go() {\n\
                 demo::by_ref(&true);\n\
                 demo::by_tuple((true, 1));\n\
                 demo::by_block({ true });\n\
                 demo::by_not(!false);\n\
                 demo::by_array([true]);\n\
                 demo::by_vec(vec![false]);\n\
                 demo::by_repeat([true; 2]);\n\
                 c(true);\n\
             }",
        ),
    ]);
    let listed = all(&scan);
    for item in [
        "by_ref(on)",
        "by_tuple(p)",
        "by_block(on)",
        "by_not(on)",
        "by_array(on)",
        "by_vec(on)",
        "by_repeat(on)",
        "ctrl(on)",
    ] {
        assert!(
            scan.bool_literal(&site(&scan, item), &listed).is_err(),
            "{item}"
        );
    }
}

#[test]
fn a_literal_in_a_macro_rules_body_is_read_once_each_metavariable_is_a_word() {
    let scan = scanned(
        "pub fn in_rules(n: u32, on: bool) {}\n\
         macro_rules! m { ($x:expr) => { in_rules($x, true) }; }",
    );
    let listed = all(&scan);
    assert!(
        scan.bool_literal(&site(&scan, "in_rules(on)"), &listed)
            .is_err()
    );
}

#[test]
fn a_trait_line_is_refuted_on_an_impl_for_a_primitive() {
    let scan = scanned(
        "pub struct Px(u32);\n\
         impl core::ops::Add<Px> for u32 { type Output = u32; fn add(self, rhs: Px) -> u32 { 0 } }",
    );
    let refusal = scan.foreign_signature(&site(&scan, "<u32 as core::ops::Add<Px>>::add()"));
    assert!(
        refusal
            .as_ref()
            .is_err_and(|reason| reason.contains("implements for")),
        "{refusal:?}"
    );
}

#[test]
fn an_alias_parameter_takes_its_default_when_the_use_writes_none() {
    let scan = scanned(
        "pub type A<T = bool> = Option<T>;\n\
         pub struct S { pub on: A }\n\
         pub type B<T = Flag> = Vec<T>;\n\
         type Flag = bool;\n\
         pub struct E { pub on: B }",
    );
    for item in ["S.on", "E.on"] {
        assert!(scan.bool_field(&site(&scan, item)).is_err(), "{item}");
    }
}

#[test]
fn an_alias_argument_pairs_with_a_parameter_of_its_own_kind() {
    let scan = scanned(
        "pub type Arr<'a, const N: usize, T> = &'a [T; N];\n\
         pub const N: usize = 3;\n\
         pub struct S<'a> { pub a: Arr<'a, N, bool> }",
    );
    assert!(scan.bool_field(&site(&scan, "S.a")).is_err());
}

#[test]
fn an_alias_parameter_takes_its_value_in_its_own_frame_only() {
    // `Deep` names the struct `U`, never the parameter `U` of `Inner`.
    let found = keys(&[(
        "src/lib.rs",
        "pub struct U;\n\
         pub type Deep = Vec<U>;\n\
         pub type Inner<U> = Deep;\n\
         pub struct S { pub s: Inner<bool> }",
    )]);
    assert!(found.iter().all(|key| !key.contains("S.s")), "{found:?}");
}

#[test]
fn a_generic_parameter_or_a_block_item_shadows_an_alias_of_its_name() {
    let found = keys(&[(
        "src/lib.rs",
        "type Flag = bool;\n\
         pub struct G<Flag> { pub on: Flag }\n\
         pub struct H<T>(T);\n\
         impl<Flag> H<Flag> { pub fn f(x: Flag) {} pub fn g<Flag>(y: Flag) {} }\n\
         pub fn k<Flag>(z: Flag) {}\n\
         pub fn body() { struct Flag; pub struct I { pub on: Flag } }",
    )]);
    assert!(
        found.iter().all(|key| !key.ends_with("| Flag")),
        "{found:?}"
    );
}

#[test]
fn an_alias_or_a_use_a_block_declares_fails_the_scan() {
    let (_, errors) = run(&[(
        "src/lib.rs",
        "mod flags { pub type Yes = bool; }\n\
         pub fn f() { type Flag = bool; pub struct S { pub on: Flag } }\n\
         pub fn g() { use crate::flags::Yes; pub struct T { pub on: Yes } }\n\
         pub fn h() { use crate::flags as fl; pub struct U { pub on: fl::Yes } }\n\
         pub fn i() { use crate::flags::*; pub struct V { pub on: Yes } }\n\
         pub fn j() { use crate::flags::Yes as Other; pub struct W { pub on: Other } }\n\
         pub fn k() { use std::primitive::bool as B; pub struct X { pub on: B } }\n\
         pub fn l() { use crate::flags::Yes as Other; pub fn set(on: Other) {} set(true); }\n\
         pub fn m() { use crate::flags::Yes as Other; pub fn get() -> Other { true } }\n\
         pub fn n() { extern crate core as s2; pub struct Y { pub on: s2::primitive::bool } }",
    )]);
    for item in [
        "f::S.on", "g::T.on", "h::U.on", "i::V.on", "j::W.on", "k::X.on", "set(on)", "get()",
        "n::Y.on",
    ] {
        assert!(
            errors
                .iter()
                .any(|error| error.contains(item) && error.contains("fails closed")),
            "{item} in {errors:?}"
        );
    }
    assert_eq!(errors.len(), 9, "{errors:?}");
}

#[test]
fn a_glob_chain_resolves_at_any_depth_and_a_glob_it_cannot_place_fails_the_scan() {
    let mut chain = String::new();
    for i in 0..7 {
        chain.push_str(&format!(
            "pub mod m{i} {{ pub use super::m{}::*; }}\n",
            i + 1
        ));
    }
    chain.push_str(
        "pub mod m7 { pub type Flag = u32; }\n\
         pub mod user { use super::m0::*; pub struct S { pub on: Flag } }\n\
         pub mod a { pub use super::b::*; }\n\
         pub mod b { pub use super::a::*; }\n\
         pub mod looped { use super::a::*; pub struct L { pub on: Flag } }",
    );
    let found = keys(&[("src/lib.rs", &chain)]);
    assert!(
        found
            .iter()
            .any(|key| key == "demo | src/lib.rs | crate::user | S.on | field | Flag"),
        "{found:?}"
    );
    assert!(found.iter().all(|key| !key.contains("L.on")), "{found:?}");
    let (_, errors) = run(&[(
        "src/lib.rs",
        "pub mod m { pub type Flag = bool; }\n\
         pub mod user { use super::nowhere::*; pub struct S { pub on: Flag } }",
    )]);
    assert!(
        errors
            .iter()
            .any(|error| error.contains("S.on") && error.contains("fails closed")),
        "{errors:?}"
    );
}

#[test]
fn a_glob_search_runs_once_per_module_and_name() {
    // Sixty modules that each glob the root, which globs each of them. A
    // walk that reads a module twice opens the sixty again at each step. It
    // took twenty seconds. The bound is a hundred times what one search
    // per module and name takes.
    let started = std::time::Instant::now();
    let mut tree = String::new();
    for i in 1..=60 {
        tree.push_str(&format!("pub use m{i}::*;\n"));
    }
    for i in 1..=60 {
        tree.push_str(&format!(
            "pub mod m{i} {{ use super::*; pub struct T{i}; pub fn f() -> Result<(), ()> {{ Ok(()) }} }}\n"
        ));
    }
    let scan = crates(&[
        ("demo/Cargo.toml", ""),
        ("demo/src/lib.rs", &tree),
        ("other/Cargo.toml", ""),
        (
            "other/src/lib.rs",
            "pub type Result<T> = core::result::Result<T, ()>;",
        ),
    ]);
    assert!(scan.sites.is_empty(), "{:?}", scan.sites);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn a_renamed_toolchain_primitive_is_read_through_an_import_chain() {
    let scan = scanned(
        "pub mod a { pub use std::primitive::bool as Flag; }\n\
         pub mod b { use crate::a::Flag; pub struct S { pub on: Flag } }\n\
         pub mod c { use crate::a; pub struct T { pub on: a::Flag } }",
    );
    for item in ["S.on", "T.on"] {
        assert!(scan.bool_field(&site(&scan, item)).is_err(), "{item}");
    }
}

#[test]
fn an_extern_crate_rename_names_the_crate() {
    let scan = crates(&[
        ("demo/Cargo.toml", ""),
        ("demo/src/lib.rs", "pub type Flag = bool;"),
        (
            "user/Cargo.toml",
            "[dependencies]\ndemo = { path = \"../demo\" }",
        ),
        (
            "user/src/lib.rs",
            "extern crate demo as d;\npub struct U { pub on: d::Flag }",
        ),
    ]);
    assert!(scan.bool_field(&site(&scan, "U.on")).is_err());
}

#[test]
fn a_literal_is_found_in_a_prop_expression_an_unsafe_block_and_a_branch() {
    let scan = scanned(
        "pub fn h(on: bool) {}\n\
         pub fn j(on: bool) {}\n\
         pub fn k(on: bool) {}\n\
         #[component]\n\
         pub fn Toggle(checked: Option<bool>, other: bool, third: bool) -> impl IntoView { () }\n\
         fn calls(x: u8) {\n\
             h(unsafe { true });\n\
             j(if x > 1 { false } else { true });\n\
             k(match x { 0 => true, _ => false });\n\
             view! { <Toggle checked=Some(true) other=!false third=(true)/> };\n\
         }",
    );
    let listed = all(&scan);
    for item in [
        "h(on)",
        "j(on)",
        "k(on)",
        "Toggle(checked)",
        "Toggle(other)",
        "Toggle(third)",
    ] {
        assert!(
            scan.bool_literal(&site(&scan, item), &listed).is_err(),
            "{item}"
        );
    }
}

#[test]
fn a_literal_reaches_a_fn_under_the_name_a_pub_use_gives_it() {
    let scan = crates(&[
        ("demo/Cargo.toml", ""),
        (
            "demo/src/lib.rs",
            "mod inner { pub fn ctrl(on: bool) {} }\npub use inner::ctrl as switch;",
        ),
        (
            "user/Cargo.toml",
            "[dependencies]\ndemo = { path = \"../demo\" }",
        ),
        ("user/src/lib.rs", "fn f() { demo::switch(true); }"),
    ]);
    let listed = all(&scan);
    assert!(
        scan.bool_literal(&site(&scan, "ctrl(on)"), &listed)
            .is_err()
    );
}

#[test]
fn a_fn_rename_holds_in_its_own_module_only() {
    let scan = scanned(
        "pub mod x { pub fn set(on: bool) {} }\n\
         mod a { use crate::x::set as go; }\n\
         pub mod b { pub fn go(on: bool) {} fn f() { go(true); } }",
    );
    let listed = all(&scan);
    assert!(scan.bool_literal(&site(&scan, "go(on)"), &listed).is_err());
    assert_eq!(scan.bool_literal(&site(&scan, "set(on)"), &listed), Ok(()));
}

#[test]
fn a_deref_any_macro_call_writes_fails_closed() {
    let scan = scanned(
        "pub struct K(u8);\n\
         cfg_if::cfg_if! { if #[cfg(unix)] { impl core::ops::Deref for K { type Target = u8; } } }\n\
         macro_rules! m { ($t:ty) => { paste::paste! { impl core::ops::Deref for $t { type Target = u8; } } }; }\n\
         fn f() { macro_rules! n { () => { impl core::ops::Deref for crate::K { type Target = u8; } }; } }\n\
         fn g() { other!(impl core::ops::DerefMut for K {}); }\n\
         fn h() { println!(\"no impl here\"); }",
    );
    for refused in [
        "a call of cfg_if::cfg_if!",
        "a call of paste::paste!",
        "Deref of crate::K to u8",
        "a call of other!",
    ] {
        assert!(
            scan.derefs.iter().any(|refusal| refusal.contains(refused)),
            "{refused} in {:?}",
            scan.derefs
        );
    }
    assert_eq!(scan.derefs.len(), 4, "{:?}", scan.derefs);
}

#[test]
fn a_deref_to_an_associated_type_of_self_or_in_a_discriminant_is_read() {
    let scan = scanned(
        "pub struct K<T>(T);\n\
         impl<T> core::ops::Deref for K<T> { type Target = Self::Inner; }\n\
         enum E { A = { struct Y(u8); impl core::ops::Deref for Y { type Target = u8; } 0 } }",
    );
    for refused in ["an associated type of Self", "Deref of Y to u8"] {
        assert!(
            scan.derefs.iter().any(|refusal| refusal.contains(refused)),
            "{refused} in {:?}",
            scan.derefs
        );
    }
}

/// The refusal of a `mod` keyword a macro holds at `line` of the demo
/// crate's `src/lib.rs`.
fn macro_mod(line: usize) -> String {
    format!(
        "crates/demo/src/lib.rs:{line}: a macro holds the keyword `mod` outside an inline module. \
         The walk follows no module a macro declares, and the dep-info check cannot tell its \
         file from data, so the scan fails closed (ARCH-033): declare the module in the source"
    )
}

#[test]
fn a_mod_a_macro_declares_fails_the_scan_in_every_form() {
    // The SEC-1g review: a `mod` a macro writes compiles a data file as
    // code, and the parse guard of the dep-info check fails open.
    let (_, errors) = run(&[(
        "src/lib.rs",
        "macro_rules! never { () => { #[path = \"hid.txt\"] mod hid; }; }\n\
         m! { mod hid; }\n\
         pub fn f() { let _ = m!(mod hid); }\n\
         pub fn g() { n! { #[path = \"hid.txt\"] mod hid; } }\n\
         macro_rules! a { () => { macro_rules! b { () => { { mod deep; } }; } }; }\n\
         m!(r#mod raw;);\n\
         macro_rules! named { ($n:ident) => { mod $n; }; }\n\
         macro_rules! spliced { ($kw:tt $n:ident) => { $kw $n; }; }\n\
         spliced!(mod hid);\n\
         #[cfg_attr(all(), doc = m!(mod hid;))]\n\
         pub struct S;\n\
         impl S { m!(mod hid;); }\n\
         #[cfg(test)]\n\
         mod tests { m! { mod hid; } }\n\
         #[cfg(test)]\n\
         m! { mod hid; }",
    )]);
    let lines = [1, 2, 3, 4, 5, 6, 7, 9, 10, 12];
    assert_eq!(errors, lines.map(macro_mod));
}

#[test]
fn an_inline_module_a_macro_writes_passes_and_its_path_child_fails() {
    // `define_serde_adapter!` of `ambre-core` and the command tables of
    // Trame and Toile write inline modules, which read no file.
    let (_, errors) = run(&[(
        "src/lib.rs",
        "macro_rules! adapter { ($name:ident) => { pub mod $name { pub mod option {} } }; }\n\
         macro_rules! titles { () => { mod titles { pub fn f() {} } }; }\n\
         macro_rules! outer { () => { mod outer {\n\
         #[path = \"hid.txt\"] mod hid; } }; }\n\
         macro_rules! glued { ($b:tt) => { mod glued $b }; }",
    )]);
    assert_eq!(errors, [4, 5].map(macro_mod));
}

#[test]
fn an_unstable_feature_in_a_crate_root_fails_the_scan() {
    // A `#![feature]` enables syntax the pinned syn may not parse, so a data
    // file could pass the parse guard and compile on the nightly toolchain.
    let refusal = |file: &str, line: usize| {
        format!(
            "crates/demo/{file}:{line}: a crate root enables an unstable feature, whose syntax \
             the gate may not parse, so the scan fails closed (ARCH-033): remove it, or name the \
             root in FEATURE_ROOTS with the reason"
        )
    };
    let (_, errors) = run(&[
        (
            "src/lib.rs",
            "#![feature(never_type)]\n\
             #![cfg_attr(all(), r#feature(box_patterns))]\n\
             #![allow(dead_code)]\n\
             mod a;",
        ),
        ("src/a.rs", "#![allow(unused)]\npub fn f() {}"),
        (
            "src/main.rs",
            "#![cfg_attr(nightly, allow(dead_code), feature(yeet_expr))]\nfn main() {}",
        ),
        ("build.rs", "#![feature(try_blocks)]\nfn main() {}"),
    ]);
    assert_eq!(
        errors,
        [
            refusal("src/main.rs", 1),
            refusal("src/lib.rs", 1),
            refusal("src/lib.rs", 2),
            refusal("build.rs", 1),
        ]
    );
}

#[test]
fn a_migrate_call_names_data_only_as_the_sqlx_macro() {
    // The SEC-1g review: a bare `migrate!` of the workspace's own would make
    // every file of its directory data.
    let result = scan(
        &demo(&[
            (
                "src/lib.rs",
                "use sqlx::migrate;\n\
                 pub fn a() { sqlx::migrate!(\"a\"); }\n\
                 pub fn b() { let _ = ::sqlx::migrate!(\"b\"); }\n\
                 pub fn c() { migrate!(\"c\"); }\n\
                 pub fn e() { other::migrate!(\"e\"); }\n\
                 pub fn f() { x::sqlx::migrate!(\"f\"); }\n\
                 mod inner { pub fn g() { migrate!(\"g\"); } }\n\
                 mod imports { use sqlx::migrate; pub fn h() { migrate!(\"h\"); } }",
            ),
            (
                "src/main.rs",
                "use sqlx::migrate as run;\n\
                 use sqlx::*;\n\
                 fn main() { migrate!(\"d\"); run!(\"d\"); }",
            ),
            ("a/1.sql", ""),
            ("b/1.sql", ""),
            ("c/1.sql", ""),
            ("d/1.sql", ""),
            ("e/1.sql", ""),
            ("f/1.sql", ""),
            ("g/1.sql", ""),
            ("h/1.sql", ""),
        ]),
        Path::new(CRATES),
    );
    assert_eq!(result.errors, Vec::<String>::new());
    let dir = Path::new(CRATES).join("demo");
    assert_eq!(
        result.data,
        BTreeMap::from([
            (
                dir.join("src/lib.rs"),
                ["a/1.sql", "b/1.sql", "c/1.sql", "h/1.sql"]
                    .iter()
                    .map(|name| dir.join(name))
                    .collect::<BTreeSet<PathBuf>>()
            ),
            (dir.join("src/main.rs"), BTreeSet::new()),
        ])
    );
}
