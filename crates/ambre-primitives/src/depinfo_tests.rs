use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::{
    Build, COMPILES, Compile, Compiled, Target, Unit, artifact, collect, cross_check, dependencies,
};
use crate::scan::{Disk, Scan, Tree, lexical, scan};

const ROOT: &str = "/w";

/// Files held in memory, under the workspace root, for the checks that read
/// a file or list a tree. A file under `target/` is build output and is not
/// listed, as on disk.
struct Files(BTreeMap<PathBuf, String>);

impl Files {
    fn new(files: &[(&str, &str)]) -> Self {
        Self(
            files
                .iter()
                .map(|(path, text)| (Path::new(ROOT).join(path), (*text).to_owned()))
                .collect(),
        )
    }
}

impl Tree for Files {
    fn read(&self, path: &Path) -> Option<String> {
        self.0.get(&lexical(path)).cloned()
    }

    fn files(&self, dir: &Path) -> Vec<PathBuf> {
        self.0
            .keys()
            .filter(|path| {
                path.strip_prefix(dir)
                    .is_ok_and(|inside| !inside.iter().any(|part| part == "target"))
            })
            .cloned()
            .collect()
    }

    fn canonical(&self, path: &Path) -> PathBuf {
        lexical(path)
    }
}

/// No file at all, for a check that reads none.
fn empty() -> Files {
    Files(BTreeMap::new())
}

/// The paths under the workspace root, as a set.
fn paths(names: &[&str]) -> BTreeSet<PathBuf> {
    names
        .iter()
        .map(|name| Path::new(ROOT).join(name))
        .collect()
}

/// A walk that read `files` from the crate root `root`.
fn walked(root: &str, files: &[&str]) -> Scan {
    let mut scan = Scan::default();
    scan.walked.insert(Path::new(ROOT).join(root), paths(files));
    scan
}

/// One unit of the crate root `root` that read `reads`.
fn unit(root: &str, build: Build, reads: &[&str]) -> Unit {
    Unit {
        root: Path::new(ROOT).join(root),
        build,
        reads: paths(reads),
    }
}

#[test]
fn a_dep_info_file_lists_its_dependencies() {
    let text = "/t/deps/a-1.d: src/lib.rs src/with\\ space.rs data/tokens.ron\n\
                /t/deps/liba-1.rmeta: src/lib.rs src/with\\ space.rs data/tokens.ron\n\
                \n\
                src/lib.rs:\n\
                src/with\\ space.rs:\n\
                data/tokens.ron:\n\
                \n\
                # env-dep:CARGO_PKG_NAME=a\n";
    assert_eq!(
        dependencies(text),
        Ok(["src/lib.rs", "src/with space.rs", "data/tokens.ron"]
            .map(PathBuf::from)
            .to_vec())
    );
}

/// The refusal of the parser, or a panic when it reads the text.
fn refused(text: &str) -> String {
    match dependencies(text) {
        Ok(paths) => panic!("the parser reads {paths:?}"),
        Err(error) => error,
    }
}

#[test]
fn a_path_that_holds_a_newline_fails_the_parser() {
    // rustc writes the file `src/x\ny.rs` unescaped, so it splits into a
    // fragment `src/x` and a line with no target. SEC-1f dropped both.
    let split = refused(
        "/t/a.d: src/lib.rs src/x\n\
         y.rs\n\
         \n\
         src/lib.rs:\n\
         src/x\n\
         y.rs:\n",
    );
    assert!(split.starts_with("line 2 is no rule"), "{split}");
    // A fragment that reads as a rule lists other files than the first.
    let rule = refused(
        "/t/a.d: src/lib.rs src/x\n\
         b: c.rs\n\
         /t/a.rmeta: src/lib.rs src/x\n\
         b: c.rs\n",
    );
    assert!(
        rule.contains("lists other files than the first rule"),
        "{rule}"
    );
    // A dependency with no empty rule of its own is a fragment too.
    let orphan = refused("/t/a.d: src/lib.rs src/x\n\nsrc/lib.rs:\n");
    assert!(
        orphan.contains("\"src/x\" has no rule of its own"),
        "{orphan}"
    );
    // A path with a `:` would read as a rule.
    let colon = refused("/t/a.d: src/lib.rs src/a:b.rs\n");
    assert!(colon.contains("holds a `:`"), "{colon}");
    assert!(refused("").starts_with("no rule lists a dependency"));
}

#[test]
fn a_path_that_holds_a_backslash_fails_with_its_own_message() {
    // rustc writes the file `a\ b.rs` as `a\\ b.rs`, which reads back as
    // `a\` and `b.rs` or as `a\ b.rs` alike. SEC-1f split it and refused
    // `b.rs`, a file that does not exist.
    let error = refused(
        "/t/a.d: src/lib.rs src/a\\\\ b.rs\n\
         \n\
         src/lib.rs:\n\
         src/a\\\\ b.rs:\n",
    );
    assert!(
        error.starts_with("line 1: a path holds a backslash that escapes no space"),
        "{error}"
    );
}

#[test]
fn an_artifact_with_no_test_flag_fails_closed() {
    // SEC-1f read a missing flag as a test build, which no check reads.
    let message = r#"{"reason":"compiler-artifact","package_id":"path+file:///w/crates/demo#0.1.0","manifest_path":"/w/crates/demo/Cargo.toml","target":{"kind":["lib"],"src_path":"/w/crates/demo/src/lib.rs"},"profile":{},"filenames":["/w/target/debug/deps/libdemo-1.rmeta"]}"#;
    let Err(error) = artifact(Path::new(ROOT), message) else {
        panic!("an artifact with no test flag is read");
    };
    assert!(error.contains("names no test flag"), "{error}");
}

#[test]
fn a_missing_dep_info_names_how_to_rebuild_it() {
    let message = |id: &str| {
        format!(
            r#"{{"reason":"compiler-artifact","package_id":"{id}","manifest_path":"/w/crates/demo/Cargo.toml","target":{{"kind":["lib"],"src_path":"/w/crates/demo/src/lib.rs"}},"profile":{{"test":false}},"filenames":["/w/target/debug/deps/libdemo-1.rmeta"]}}"#
        )
    };
    for id in [
        "path+file:///w/crates/demo#0.1.0",
        "path+file:///w/crates/other#demo@0.1.0",
    ] {
        let Err(error) = artifact(Path::new(ROOT), &message(id)) else {
            panic!("a unit with no dep-info is read");
        };
        assert!(
            error.contains("Touch a source file of demo, or run `cargo clean -p demo`"),
            "{error}"
        );
    }
}

#[test]
fn a_file_a_product_build_reads_and_the_walk_does_not_is_refused() {
    let scan = walked(
        "crates/demo/src/lib.rs",
        &["crates/demo/src/lib.rs", "crates/demo/src/a.rs"],
    );
    let compiled = Compiled {
        units: vec![unit(
            "crates/demo/src/lib.rs",
            Build::Product,
            &[
                "crates/demo/src/lib.rs",
                "crates/demo/src/a.rs",
                "crates/demo/src/hidden.rs",
            ],
        )],
    };
    let sources = paths(&[
        "crates/demo/src/lib.rs",
        "crates/demo/src/a.rs",
        "crates/demo/src/hidden.rs",
    ]);
    assert_eq!(
        cross_check(&scan, &compiled, &sources, Path::new(ROOT), &empty()),
        [
            "crates/demo/src/hidden.rs: rustc reads it for the crate root crates/demo/src/lib.rs, \
             and the walk neither reads it as a module nor finds a string literal that names it \
             as data, so the gate cannot place it"
        ]
    );
}

#[test]
fn a_file_only_a_test_build_reads_is_no_refusal_and_no_orphan() {
    let scan = walked("crates/demo/src/lib.rs", &["crates/demo/src/lib.rs"]);
    let compiled = Compiled {
        units: vec![
            unit(
                "crates/demo/src/lib.rs",
                Build::Product,
                &["crates/demo/src/lib.rs"],
            ),
            unit(
                "crates/demo/src/lib.rs",
                Build::Test,
                &[
                    "crates/demo/src/lib.rs",
                    "crates/demo/src/tests.rs",
                    "crates/demo/src/hid.txt",
                    "extra/fixture",
                ],
            ),
            unit(
                "crates/demo/tests/it.rs",
                Build::Test,
                &["crates/demo/tests/it.rs"],
            ),
        ],
    };
    let sources = paths(&[
        "crates/demo/src/lib.rs",
        "crates/demo/src/tests.rs",
        "crates/demo/tests/it.rs",
    ]);
    assert_eq!(
        cross_check(&scan, &compiled, &sources, Path::new(ROOT), &empty()),
        Vec::<String>::new()
    );
}

#[test]
fn a_file_no_compile_reads_is_refused_whatever_its_name() {
    // The orphan check of SEC-1e left a `*_tests.rs` file out by its name.
    let scan = walked("crates/demo/src/lib.rs", &["crates/demo/src/lib.rs"]);
    let compiled = Compiled {
        units: vec![unit(
            "crates/demo/src/lib.rs",
            Build::Product,
            &["crates/demo/src/lib.rs"],
        )],
    };
    let sources = paths(&[
        "crates/demo/src/lib.rs",
        "crates/demo/src/orphan.rs",
        "crates/demo/src/leak_tests.rs",
    ]);
    assert_eq!(
        cross_check(&scan, &compiled, &sources, Path::new(ROOT), &empty()),
        [
            "crates/demo/src/leak_tests.rs: no compile of the workspace reads it, so no build \
             checks it",
            "crates/demo/src/orphan.rs: no compile of the workspace reads it, so no build checks it",
        ]
    );
}

#[test]
fn a_crate_root_the_walk_does_not_start_from_is_refused() {
    let scan = walked("crates/demo/src/lib.rs", &["crates/demo/src/lib.rs"]);
    let compiled = Compiled {
        units: vec![unit(
            "crates/demo/src/tool/main.rs",
            Build::Product,
            &["crates/demo/src/tool/main.rs"],
        )],
    };
    let sources = paths(&["crates/demo/src/lib.rs", "crates/demo/src/tool/main.rs"]);
    let errors = cross_check(&scan, &compiled, &sources, Path::new(ROOT), &empty());
    assert_eq!(
        errors[0],
        "crates/demo/src/tool/main.rs: rustc compiles this crate root, and the walk does not \
         start from it"
    );
    assert_eq!(errors.len(), 2, "{errors:?}");
}

#[test]
fn a_file_a_product_build_reads_is_refused_whatever_its_name_unless_named_as_data() {
    // The SEC-1f review: a `mod` a macro writes with `#[path = "hid.txt"]`
    // compiles `hid.txt` as code, and the check kept `.rs` reads alone. A
    // file with no extension outside `crates/` passed the same way.
    let mut scan = walked("crates/demo/src/lib.rs", &["crates/demo/src/lib.rs"]);
    scan.data.insert(
        Path::new(ROOT).join("crates/demo/src/lib.rs"),
        paths(&["crates/demo/src/data.txt"]),
    );
    let compiled = Compiled {
        units: vec![unit(
            "crates/demo/src/lib.rs",
            Build::Product,
            &[
                "crates/demo/src/lib.rs",
                "crates/demo/src/data.txt",
                "crates/demo/src/hid.txt",
                "extra/hid",
            ],
        )],
    };
    let tree = Files::new(&[("crates/demo/src/data.txt", "plain text")]);
    let sources = paths(&["crates/demo/src/lib.rs"]);
    let refused = |file: &str| {
        format!(
            "{file}: rustc reads it for the crate root crates/demo/src/lib.rs, and the walk \
             neither reads it as a module nor finds a string literal that names it as data, so \
             the gate cannot place it"
        )
    };
    assert_eq!(
        cross_check(&scan, &compiled, &sources, Path::new(ROOT), &tree),
        [refused("crates/demo/src/hid.txt"), refused("extra/hid")]
    );
}

#[test]
fn a_data_file_that_parses_as_rust_items_is_refused() {
    // The dep-info lists a file once, whether an `include_str!` reads it or
    // a `mod` compiles it. Only a file with no Rust item cannot be a module.
    let mut scan = walked("crates/demo/src/lib.rs", &["crates/demo/src/lib.rs"]);
    scan.data.insert(
        Path::new(ROOT).join("crates/demo/src/lib.rs"),
        paths(&[
            "crates/demo/src/code.txt",
            "crates/demo/src/empty.txt",
            "crates/demo/src/shader.wgsl",
        ]),
    );
    let compiled = Compiled {
        units: vec![unit(
            "crates/demo/src/lib.rs",
            Build::Product,
            &[
                "crates/demo/src/lib.rs",
                "crates/demo/src/code.txt",
                "crates/demo/src/empty.txt",
                "crates/demo/src/shader.wgsl",
            ],
        )],
    };
    let tree = Files::new(&[
        ("crates/demo/src/code.txt", "pub fn f() { let expose = 1; }"),
        ("crates/demo/src/empty.txt", "// a comment\n"),
        (
            "crates/demo/src/shader.wgsl",
            "@vertex fn vs() -> @builtin(position) vec4<f32> { return vec4<f32>(); }",
        ),
    ]);
    let sources = paths(&["crates/demo/src/lib.rs"]);
    assert_eq!(
        cross_check(&scan, &compiled, &sources, Path::new(ROOT), &tree),
        [
            "crates/demo/src/code.txt: a walked file of the crate root crates/demo/src/lib.rs \
             names it as data, and it parses as Rust items. The dep-info does not tell a module \
             from data, so the gate cannot show that no `mod` compiles it"
        ]
    );
}

#[test]
fn the_orphan_check_covers_the_whole_root_but_a_nested_workspace() {
    // SEC-1f read `.rs` files under `crates/` alone.
    let tree = Files::new(&[
        ("Cargo.toml", "[workspace]\n"),
        ("crates/demo/Cargo.toml", "[package]\n"),
        ("crates/demo/src/lib.rs", ""),
        ("extra/orphan.rs", ""),
        (
            "fuzz/Cargo.toml",
            "[package]\nname = \"fuzz\"\n\n[workspace]\n",
        ),
        ("fuzz/fuzz_targets/one.rs", ""),
        ("target/debug/build/out.rs", ""),
        ("scripts/gate.py", ""),
    ]);
    assert_eq!(
        super::sources(&tree, Path::new(ROOT)),
        paths(&["crates/demo/src/lib.rs", "extra/orphan.rs"])
    );
}

/// The workspace root, two levels above this crate's manifest.
fn workspace() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(Path::parent)
        .unwrap_or(manifest)
        .to_path_buf()
}

#[test]
fn the_wasm_compiles_are_the_lines_of_lint_wasm() {
    let Ok(justfile) = std::fs::read_to_string(workspace().join("justfile")) else {
        panic!("the justfile cannot be read");
    };
    let recipe: Vec<Vec<String>> = justfile
        .lines()
        .skip_while(|line| *line != "lint-wasm:")
        .skip(1)
        .take_while(|line| line.starts_with("    ") || line.is_empty())
        .filter_map(|line| line.split_once("--target wasm32-unknown-unknown "))
        .map(|(_, args)| args.split_whitespace().map(str::to_owned).collect())
        .collect();
    let compiled: Vec<Vec<String>> = COMPILES
        .iter()
        .filter(|compile| compile.target == Target::Wasm)
        .map(|compile| compile.args.iter().map(|arg| (*arg).to_owned()).collect())
        .collect();
    assert!(!recipe.is_empty(), "no wasm line in lint-wasm");
    assert_eq!(recipe, compiled);
}

/// A directory removed when the test ends, pass or fail.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Writes `text` at `path` under `root`, its directories made first.
fn put(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    if let Some(dir) = path.parent()
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        panic!("{}: {e}", dir.display());
    }
    if let Err(e) = std::fs::write(&path, text) {
        panic!("{}: {e}", path.display());
    }
}

/// Each secret refusal cut to its file and the name it refuses.
fn heads(found: &[String]) -> BTreeSet<String> {
    found
        .iter()
        .filter_map(|refusal| {
            let (head, _) = refusal.split_once(" is named here")?;
            let (file, _) = head.split_once(':')?;
            let (_, name) = head.rsplit_once(' ')?;
            Some(format!("{file} {name}"))
        })
        .collect()
}

/// A scratch workspace in the temporary directory, named for one test, with
/// one member crate per name of `members`.
fn probe(test: &str, members: &[&str]) -> (PathBuf, Scratch) {
    let root = std::env::temp_dir().join(format!("ambre-depinfo-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let scratch = Scratch(root.clone());
    let list: Vec<String> = members
        .iter()
        .map(|member| format!("\"crates/{member}\""))
        .collect();
    put(
        &root,
        "Cargo.toml",
        &format!(
            "[workspace]\nmembers = [{}]\nresolver = \"2\"\n",
            list.join(", ")
        ),
    );
    for member in members {
        put(
            &root,
            &format!("crates/{member}/Cargo.toml"),
            &format!("[package]\nname = \"{member}\"\nversion = \"0.0.0\"\nedition = \"2021\"\n"),
        );
    }
    (root, scratch)
}

/// What cargo compiles in the scratch workspace `root`, every target of
/// every member, in a target directory of its own.
fn compile(root: &Path) -> Result<Compiled, Vec<String>> {
    let cargo = PathBuf::from(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    let rustc = cargo.with_file_name("rustc");
    let target = root.join("target");
    let compiles = [Compile {
        target: Target::Host,
        args: &["--workspace", "--all-targets"],
    }];
    collect(root, &compiles, &|command| {
        command
            .arg("--offline")
            .env("CARGO_TARGET_DIR", &target)
            .env_remove("RUSTFLAGS")
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .env_remove("CARGO_BUILD_TARGET");
        if rustc.is_file() {
            command.env("RUSTC", &rustc);
        }
    })
}

/// The refusals of the dep-info check on the scratch workspace `root`.
fn refusals(root: &Path, walk: &Scan) -> (Vec<String>, Compiled) {
    let compiled = match compile(root) {
        Ok(compiled) => compiled,
        Err(errors) => panic!("the probe does not compile: {errors:?}"),
    };
    let canonical = match std::fs::canonicalize(root) {
        Ok(canonical) => canonical,
        Err(e) => panic!("{}: {e}", root.display()),
    };
    let sources = super::sources(&Disk, root);
    let errors = cross_check(walk, &compiled, &sources, &canonical, &Disk);
    (errors, compiled)
}

/// The refusal of a file that a product build reads from `root` and the
/// walk does not place.
fn unplaced(file: &str, root: &str) -> String {
    format!(
        "{file}: rustc reads it for the crate root {root}, and the walk neither reads it as a \
         module nor finds a string literal that names it as data, so the gate cannot place it"
    )
}

#[cfg(unix)]
#[test]
fn a_real_compile_reads_the_files_the_walk_misses() {
    // The probes of the SEC-1e review, compiled by cargo in a workspace of
    // their own. The dep-info of each is what rustc read.
    let (root, scratch) = probe("walk", &["ambre-server"]);
    let krate = "crates/ambre-server";
    let file = |path: &str, text: &str| put(&root, &format!("{krate}/{path}"), text);
    let read = "pub fn f() { let expose = 1; let _ = expose; }\n";
    file(
        "src/lib.rs",
        "#![allow(dead_code)]\n\
         macro_rules! hidden { () => { mod leak_tests; } }\n\
         hidden!();\n\
         include!(\"spliced_tests.rs\");\n\
         #[r#path = \"raw_tests.rs\"]\nmod raw;\n\
         #[path = \"a/file.rs\"]\nmod file;\n\
         mod gated;\n\
         mod secrets;\n\
         #[path = \"secrets/../../leak.rs\"]\nmod leak;\n\
         #[cfg(test)]\nmod tests;\n",
    );
    file("src/leak_tests.rs", read);
    file("src/spliced_tests.rs", "pub fn spliced() {}\n");
    file("src/raw_tests.rs", read);
    file("src/a/file.rs", "mod child_tests;\n");
    file("src/a/child_tests.rs", read);
    file("src/gated.rs", "#![cfg(test)]\npub fn g() {}\n");
    file("src/secrets/mod.rs", "mod token;\n");
    file("src/leak2.rs", read);
    file("leak.rs", read);
    file("src/tests.rs", "");
    file("src/orphan_tests.rs", read);
    file("tests/it.rs", "#[test]\nfn t() {}\n");
    let dir = root.join(krate);
    if let Err(e) = std::os::unix::fs::symlink("../leak2.rs", dir.join("src/secrets/token.rs")) {
        panic!("the link cannot be made: {e}");
    }

    let walk = scan(&Disk, &root.join("crates"));
    assert_eq!(walk.errors, Vec::<String>::new());
    assert_eq!(
        heads(&walk.secrets),
        [
            "crates/ambre-server/leak.rs `expose`",
            "crates/ambre-server/src/a/child_tests.rs `expose`",
            "crates/ambre-server/src/lib.rs `include`",
            "crates/ambre-server/src/raw_tests.rs `expose`",
            "crates/ambre-server/src/secrets/token.rs `expose`",
        ]
        .map(str::to_owned)
        .into_iter()
        .collect::<BTreeSet<String>>()
    );

    let (errors, compiled) = refusals(&root, &walk);
    let lib = "crates/ambre-server/src/lib.rs";
    assert_eq!(
        errors,
        [
            unplaced("crates/ambre-server/src/leak_tests.rs", lib),
            unplaced("crates/ambre-server/src/spliced_tests.rs", lib),
            "crates/ambre-server/src/orphan_tests.rs: no compile of the workspace reads it, so \
             no build checks it"
                .to_owned(),
        ]
    );
    let builds: BTreeMap<Build, usize> =
        compiled.units.iter().fold(BTreeMap::new(), |mut n, unit| {
            *n.entry(unit.build).or_default() += 1;
            n
        });
    assert_eq!(
        builds,
        BTreeMap::from([(Build::Product, 1), (Build::Test, 2)])
    );
    drop(scratch);
}

#[cfg(unix)]
#[test]
fn a_real_compile_refuses_a_data_file_the_walk_does_not_name() {
    // The B1 probe of the SEC-1f review. A `mod` a macro writes compiles
    // `hid.txt` and an extensionless file outside `crates/` as code. The
    // check of SEC-1f kept `.rs` reads alone.
    let (root, scratch) = probe("data", &["ambre-server"]);
    let krate = "crates/ambre-server";
    let file = |path: &str, text: &str| put(&root, &format!("{krate}/{path}"), text);
    let read = "pub fn f() { let expose = 1; let _ = expose; }\n";
    file(
        "src/lib.rs",
        "#![allow(dead_code)]\n\
         macro_rules! hidden { () => { #[path = \"hid.txt\"] mod hid; } }\n\
         hidden!();\n\
         macro_rules! outside { () => { #[path = \"../../../extra/hid\"] mod outside; } }\n\
         outside!();\n\
         pub const DATA: &str = include_str!(\"data.txt\");\n\
         pub const BLOB: &[u8] = include_bytes!(\"blob.bin\");\n\
         pub const BUILT: &str = include_str!(concat!(\"bu\", \"ilt.txt\"));\n\
         pub const CODE: &str = include_str!(\"code.txt\");\n\
         #[cfg(test)]\nmod tests { const T: &str = include_str!(\"fixture.txt\"); }\n",
    );
    file("src/hid.txt", read);
    file("src/data.txt", "plain text\n");
    file("src/built.txt", "plain text\n");
    file("src/code.txt", read);
    file("src/fixture.txt", read);
    put(&root, &format!("{krate}/src/blob.bin"), "\u{0}\u{1}");
    put(&root, "extra/hid", read);
    put(&root, "extra/orphan.rs", read);
    put(
        &root,
        "nested/Cargo.toml",
        "[package]\nname = \"nested\"\n\n[workspace]\n",
    );
    put(&root, "nested/src/main.rs", read);

    let walk = scan(&Disk, &root.join("crates"));
    assert_eq!(walk.errors, Vec::<String>::new());
    assert_eq!(walk.secrets, Vec::<String>::new());
    let (errors, _) = refusals(&root, &walk);
    let lib = "crates/ambre-server/src/lib.rs";
    assert_eq!(
        errors,
        [
            unplaced("crates/ambre-server/src/built.txt", lib),
            format!(
                "crates/ambre-server/src/code.txt: a walked file of the crate root {lib} names \
                 it as data, and it parses as Rust items. The dep-info does not tell a module \
                 from data, so the gate cannot show that no `mod` compiles it"
            ),
            unplaced("crates/ambre-server/src/hid.txt", lib),
            unplaced("extra/hid", lib),
            "extra/orphan.rs: no compile of the workspace reads it, so no build checks it"
                .to_owned(),
        ]
    );
    drop(scratch);
}

#[cfg(unix)]
#[test]
fn a_real_dep_info_with_a_path_rustc_cannot_escape_fails_closed() {
    // The B2 probe of the SEC-1f review, and its N4: rustc writes a newline
    // and a backslash unescaped, so both paths split.
    let (root, scratch) = probe("escape", &["split", "slash"]);
    put(
        &root,
        "crates/split/src/lib.rs",
        "#[path = \"x\\ny.rs\"]\nmod x;\n",
    );
    put(&root, "crates/split/src/x\ny.rs", "pub fn f() {}\n");
    put(
        &root,
        "crates/slash/src/lib.rs",
        "#[path = \"a\\\\ b.rs\"]\nmod a;\n",
    );
    put(&root, "crates/slash/src/a\\ b.rs", "pub fn f() {}\n");
    let Err(errors) = compile(&root) else {
        panic!("the dep-info of a split path is read");
    };
    // Each crate has a product unit and a test unit, and each fails alone.
    let joined = errors.join("\n");
    let count = |crate_name: &str, why: &str| {
        errors
            .iter()
            .filter(|error| error.contains(&format!("/{crate_name}-")) && error.contains(why))
            .count()
    };
    assert_eq!(
        count("split", "line 2 is no rule `target: dependencies`"),
        2,
        "{joined}"
    );
    assert_eq!(
        count(
            "slash",
            "line 1: a path holds a backslash that escapes no space"
        ),
        2,
        "{joined}"
    );
    assert_eq!(errors.len(), 4, "{joined}");
    drop(scratch);
}
