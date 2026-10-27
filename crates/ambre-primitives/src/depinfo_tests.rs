use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::{
    Build, COMPILES, Compile, Compiled, Target, Unit, artifact, collect, cross_check,
    declares_workspace, dependencies, env_levers, levers, outer_levers, stray_workspaces,
};
use crate::scan::{Disk, NESTED_WORKSPACES, Scan, Tree, lexical, scan};

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
    let disk = Disk::at(root);
    let sources = super::sources(&disk, root);
    let mut errors = cross_check(walk, &compiled, &sources, &canonical, &disk);
    errors.append(&mut stray_workspaces(&disk, root));
    (errors, compiled)
}

/// The refusal of a `mod` keyword a macro holds at `line` of `file`.
fn macro_mod(file: &str, line: usize) -> String {
    format!(
        "{file}:{line}: a macro holds the keyword `mod` outside an inline module of a \
         macro_rules! transcriber. The walk follows no module a macro declares, and the dep-info \
         check cannot tell its file from data, so the scan fails closed (ARCH-033): declare the \
         module in the source"
    )
}

/// The refusal of a manifest that declares a workspace of its own.
fn stray(manifest: &str) -> String {
    format!(
        "{manifest}: the manifest declares a workspace of its own, which builds apart from the \
         compiles the check runs. Make it a member of the root workspace, or name its directory \
         in NESTED_WORKSPACES"
    )
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

    let walk = scan(&Disk::at(&root), &root.join("crates"));
    // The scan refuses the `mod` the macro writes since SEC-1h, and the
    // dep-info check refuses its file too.
    assert_eq!(
        walk.errors,
        [macro_mod("crates/ambre-server/src/lib.rs", 2)]
    );
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

    let walk = scan(&Disk::at(&root), &root.join("crates"));
    let lib = "crates/ambre-server/src/lib.rs";
    assert_eq!(walk.errors, [macro_mod(lib, 2), macro_mod(lib, 4)]);
    assert_eq!(walk.secrets, Vec::<String>::new());
    let (errors, _) = refusals(&root, &walk);
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
            "nested/src/main.rs: no compile of the workspace reads it, so no build checks it"
                .to_owned(),
            stray("nested/Cargo.toml"),
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

#[cfg(unix)]
#[test]
fn a_real_compile_refuses_a_mod_a_macro_declares_over_a_data_file() {
    // The SEC-1g review: a macro writes `mod hid;` over a file a walked
    // file names as data. The dep-info lists it once, so only its parse
    // told it from data, and a parse that fails passed it.
    let (root, scratch) = probe("macro", &["ambre-server"]);
    let krate = "crates/ambre-server";
    let file = |path: &str, text: &str| put(&root, &format!("{krate}/{path}"), text);
    file(
        "src/lib.rs",
        "#![allow(dead_code)]\n\
         macro_rules! m { () => { #[path = \"hid.txt\"] mod hid; }; }\n\
         m!();\n\
         pub const DATA: &str = include_str!(\"hid.txt\");\n",
    );
    file(
        "src/hid.txt",
        "pub fn f() { let expose = 1; let _ = expose; }\n",
    );

    let walk = scan(&Disk::at(&root), &root.join("crates"));
    let lib = "crates/ambre-server/src/lib.rs";
    assert_eq!(walk.errors, [macro_mod(lib, 2)]);
    // The parse guard still refuses the file, as defence in depth.
    let (errors, _) = refusals(&root, &walk);
    assert_eq!(
        errors,
        [format!(
            "crates/ambre-server/src/hid.txt: a walked file of the crate root {lib} names it as \
             data, and it parses as Rust items. The dep-info does not tell a module from data, \
             so the gate cannot show that no `mod` compiles it"
        )]
    );
    drop(scratch);
}

#[test]
fn a_manifest_that_declares_a_workspace_outside_the_nested_ones_is_refused() {
    // The SEC-1g review: any manifest that held `[workspace]` took its
    // directory out of the orphan check.
    let tree = Files::new(&[
        ("Cargo.toml", "[workspace]\n"),
        (
            "crates/demo/Cargo.toml",
            "[package]\nworkspace = \"../..\"\n",
        ),
        ("crates/demo/src/lib.rs", ""),
        ("crates/demo/hide/Cargo.toml", "[package]\n\n[workspace]\n"),
        ("crates/demo/hide/src/lib.rs", ""),
        ("extra/Cargo.toml", "workspace.members = []\n"),
        ("extra/src/main.rs", ""),
        ("tables/Cargo.toml", "  [workspace.dependencies]\n"),
        // The SEC-1h review: the text match missed these spellings.
        ("spaced/Cargo.toml", "[ workspace ]\n"),
        ("inline/Cargo.toml", "workspace = { members = [] }\n"),
        ("quoted/Cargo.toml", "\"workspace\" = {}\n"),
        // `levers` refuses a manifest that does not parse.
        ("broken/Cargo.toml", "[workspace\n"),
        (
            "member/Cargo.toml",
            "[package]\nname = \"m\"\n[dependencies]\nx = { workspace = true }\n",
        ),
        ("fuzz/Cargo.toml", "[package]\n\n[workspace]\n"),
        ("fuzz/fuzz_targets/one.rs", ""),
    ]);
    let root = Path::new(ROOT);
    assert_eq!(
        super::sources(&tree, root),
        paths(&[
            "crates/demo/hide/src/lib.rs",
            "crates/demo/src/lib.rs",
            "extra/src/main.rs",
        ])
    );
    assert_eq!(
        stray_workspaces(&tree, root),
        [
            stray("crates/demo/hide/Cargo.toml"),
            stray("extra/Cargo.toml"),
            stray("inline/Cargo.toml"),
            stray("quoted/Cargo.toml"),
            stray("spaced/Cargo.toml"),
            stray("tables/Cargo.toml"),
        ]
    );
}

#[test]
fn each_nested_workspace_manifest_declares_a_workspace() {
    // The SEC-1h review: the orphan check leaves out each nested workspace
    // by its name, and nothing held that its manifest declares one.
    for dir in NESTED_WORKSPACES {
        let path = workspace().join(dir).join("Cargo.toml");
        let Ok(text) = std::fs::read_to_string(&path) else {
            panic!("{} cannot be read", path.display());
        };
        assert_eq!(declares_workspace(&text), Ok(true), "{}", path.display());
    }
}

/// The two recipe files, with no lever, and `files` beside them.
fn recipes(files: &[(&'static str, &'static str)]) -> Files {
    let mut all = vec![
        (
            "justfile",
            "# `-Z build-std` comes from the config.\nlint:\n    cargo clippy -- -D warnings\n",
        ),
        (
            ".github/workflows/ci.yml",
            "# -Zanything in a comment\nrun: cargo test --workspace\n",
        ),
    ];
    all.extend_from_slice(files);
    Files::new(&all)
}

/// The refusal of a lever that `what` of `shown` holds.
fn lever(shown: &str, what: &str) -> String {
    format!(
        "{shown}: {what} holds a -Z flag, a crate-attr or a feature( lever, which enables syntax \
         the gate may not parse, so the check fails closed (ARCH-033): remove it"
    )
}

/// The refusal of an `[unstable]` key UNSTABLE does not admit.
fn unstable(shown: &str, key: &str) -> String {
    format!(
        "{shown}: `{key}` is an unstable cargo setting that UNSTABLE does not name with this \
         value, so the check fails closed (ARCH-033): remove it, or name it in UNSTABLE with \
         the reason"
    )
}

#[test]
fn a_lever_in_a_cargo_config_is_refused() {
    // The SEC-1h review: `-Zcrate-attr=feature(..)` in a config enabled a
    // syntax that the pinned syn rejects, so the parse guard passed a data
    // file as data.
    let tree = recipes(&[
        (
            ".cargo/config.toml",
            "[build]\nrustflags = [\"-Zcrate-attr=feature(yeet_expr)\"]\n\
             [target.wasm32-unknown-unknown]\nrustflags = [\"-C\", \"target-feature=+atomics\"]\n\
             [target.'cfg(unix)']\nrustflags = \"-C opt-level=1 -Z x\"\n\
             [unstable]\nbuild-std = [\"std\", \"panic_abort\"]\nother = true\n",
        ),
        (
            "crates/demo/.cargo/config",
            "build.rustdocflags = [\"--crate-attr\", \"x\"]\n\
             [unstable]\nbuild-std = [\"std\", \"panic_abort\"]\n",
        ),
        (
            "crates/other/.cargo/config.toml",
            "unstable = 1\n[build]\nrustflags = [\"-A\", \"warnings\"]\n",
        ),
        ("crates/bad/.cargo/config.toml", "[build\n"),
        ("crates/kept/.cargo/other.toml", "unstable = 1\n"),
    ]);
    assert_eq!(
        levers(&tree, Path::new(ROOT)),
        [
            unstable(".cargo/config.toml", "unstable.other"),
            lever(".cargo/config.toml", "`build.rustflags`"),
            lever(".cargo/config.toml", "`target.cfg(unix).rustflags`"),
            "crates/bad/.cargo/config.toml: the file does not parse as TOML, so the check \
             cannot show what it sets, and fails closed: TOML parse error at line 1, column 7"
                .to_owned(),
            unstable("crates/demo/.cargo/config", "unstable.build-std"),
            lever("crates/demo/.cargo/config", "`build.rustdocflags`"),
            unstable("crates/other/.cargo/config.toml", "unstable"),
        ]
    );
}

#[test]
fn a_lever_in_a_manifest_is_refused() {
    let tree = recipes(&[
        (
            "Cargo.toml",
            "cargo-features = [\"profile-rustflags\"]\n[workspace]\n\
             [profile.dev]\nrustflags = [\"-C\", \"opt-level=1\"]\n",
        ),
        (
            "crates/demo/Cargo.toml",
            "[package]\nname = \"demo\"\n[profile.release.package.x]\nrustdocflags = []\n",
        ),
        ("crates/broken/Cargo.toml", "[package\n"),
    ]);
    assert_eq!(
        levers(&tree, Path::new(ROOT)),
        [
            "Cargo.toml: `cargo-features` enables an unstable cargo feature, so the check fails \
             closed (ARCH-033): remove it"
                .to_owned(),
            "Cargo.toml: `profile.dev.rustflags` passes flags to rustc from a manifest, so the \
             check fails closed (ARCH-033): remove it"
                .to_owned(),
            // The SEC-1i review: an unparsable root or nested manifest
            // reached no check, since `stray_workspaces` skips both.
            "crates/broken/Cargo.toml: the file does not parse as TOML, so the check cannot \
             show what it sets, and fails closed: TOML parse error at line 1, column 9"
                .to_owned(),
            "crates/demo/Cargo.toml: `profile.release.package.x.rustdocflags` passes flags to \
             rustc from a manifest, so the check fails closed (ARCH-033): remove it"
                .to_owned(),
        ]
    );
}

#[test]
fn a_manifest_that_renames_sqlx_to_another_package_is_refused() {
    // The SEC-1h review: `sqlx = { package = "other" }` would make a
    // foreign `migrate!` read as data.
    let tree = recipes(&[
        (
            "Cargo.toml",
            "[workspace.dependencies]\nsqlx = { version = \"0.8\", package = \"other\" }\n",
        ),
        (
            "crates/demo/Cargo.toml",
            "[dependencies]\nsqlx = { workspace = true }\n\
             [target.'cfg(unix)'.dev-dependencies.sqlx]\npackage = \"x\"\n\
             [build-dependencies]\nsqlx = { package = \"sqlx\" }\ndb = { package = \"sqlx\" }\n",
        ),
    ]);
    let renamed = |shown: &str, key: &str| {
        format!(
            "{shown}: `{key}.sqlx` names another package. The scan reads `sqlx::migrate!` as a \
             data read by its name, so the check fails closed (ARCH-033): keep the name `sqlx` \
             for the sqlx crate alone"
        )
    };
    assert_eq!(
        levers(&tree, Path::new(ROOT)),
        [
            renamed("Cargo.toml", "workspace.dependencies"),
            renamed(
                "crates/demo/Cargo.toml",
                "target.cfg(unix).dev-dependencies"
            ),
        ]
    );
}

/// The refusal of a key of a cargo config that names the compiler or a
/// wrapper.
fn compiler(shown: &str, key: &str) -> String {
    format!(
        "{shown}: `{key}` names the compiler or a wrapper that runs it, which can pass rustc a \
         flag the check does not read, so the check fails closed (ARCH-033): remove it"
    )
}

#[test]
fn a_rustc_key_in_a_cargo_config_is_refused() {
    // The SEC-1i review: `build.rustc` runs a script in place of rustc.
    let tree = recipes(&[(
        ".cargo/config.toml",
        "[build]\nrustc = \"wrap.sh\"\n[target.x]\nrustc = \"y\"\n",
    )]);
    assert_eq!(
        levers(&tree, Path::new(ROOT)),
        [
            compiler(".cargo/config.toml", "build.rustc"),
            compiler(".cargo/config.toml", "target.x.rustc"),
        ]
    );
}

#[test]
fn a_rustc_wrapper_key_in_a_cargo_config_is_refused() {
    let tree = recipes(&[(
        ".cargo/config",
        "build.rustc-wrapper = \"sccache\"\n[a.b]\nrustc-wrapper = \"\"\n",
    )]);
    assert_eq!(
        levers(&tree, Path::new(ROOT)),
        [
            compiler(".cargo/config", "a.b.rustc-wrapper"),
            compiler(".cargo/config", "build.rustc-wrapper"),
        ]
    );
}

#[test]
fn a_rustc_workspace_wrapper_key_in_a_cargo_config_is_refused() {
    // The SEC-1i review: the wrapper added `-Zcrate-attr` for one crate,
    // and the gate passed with exit 0.
    let tree = recipes(&[(
        ".cargo/config.toml",
        "[build]\nrustc-workspace-wrapper = \"./wrap.sh\"\n",
    )]);
    assert_eq!(
        levers(&tree, Path::new(ROOT)),
        [compiler(
            ".cargo/config.toml",
            "build.rustc-workspace-wrapper"
        )]
    );
}

#[test]
fn an_include_key_in_a_cargo_config_is_refused() {
    // The SEC-1i review: the included file set the lever, unread.
    let tree = recipes(&[
        (
            ".cargo/config.toml",
            "include = [{ path = \"lev.toml\" }]\n",
        ),
        ("crates/demo/.cargo/config", "include = \"x.toml\"\n"),
        (".cargo/lev.toml", "[build]\nrustflags = [\"-Zx\"]\n"),
    ]);
    let include = |shown: &str| {
        format!(
            "{shown}: `include` loads another config file, which the check does not read, so \
             the check fails closed (ARCH-033): remove it"
        )
    };
    assert_eq!(
        levers(&tree, Path::new(ROOT)),
        [
            include(".cargo/config.toml"),
            include("crates/demo/.cargo/config")
        ]
    );
}

#[test]
fn an_env_table_in_a_cargo_config_is_refused() {
    // The SEC-1i review: `[env] RUSTFLAGS` reached rustc past the check.
    let tree = recipes(&[
        (
            ".cargo/config.toml",
            "[env]\nRUSTFLAGS = \"-Zcrate-attr=feature(yeet_expr)\"\n\
             PLAIN = { value = \"x\", force = true }\n",
        ),
        ("crates/demo/.cargo/config.toml", "env = 1\n"),
    ]);
    let env = |shown: &str, key: &str| {
        format!(
            "{shown}: `{key}` sets the environment of rustc, which can carry a flag the check \
             does not read, and CONFIG_ENV does not name it with this value, so the check fails \
             closed (ARCH-033): remove it, or name it in CONFIG_ENV with the reason"
        )
    };
    assert_eq!(super::CONFIG_ENV.len(), 0, "a new entry needs its own test");
    assert_eq!(
        levers(&tree, Path::new(ROOT)),
        [
            env(".cargo/config.toml", "env.PLAIN"),
            env(".cargo/config.toml", "env.RUSTFLAGS"),
            env("crates/demo/.cargo/config.toml", "env"),
        ]
    );
}

#[test]
fn a_lever_in_a_config_above_the_root_or_in_the_cargo_home_is_refused() {
    // The SEC-1i review: cargo reads the config of each directory above the
    // root, and the check read none.
    let tree = Files::new(&[
        ("/.cargo/config.toml", "[build]\nrustflags = [\"-Zx\"]\n"),
        (
            "/.cargo/config",
            "[unstable]\nbuild-std = [\"std\", \"panic_abort\"]\n",
        ),
        (
            "/home/.cargo/config.toml",
            "[build]\nrustc-wrapper = \"w\"\n",
        ),
        ("/home/.cargo/config", "[build\n"),
        (
            "/home/.cargo/other.toml",
            "[build]\nrustflags = [\"-Zx\"]\n",
        ),
        (".cargo/config.toml", "[build]\nrustflags = [\"-Zx\"]\n"),
    ]);
    assert_eq!(
        outer_levers(&tree, Path::new(ROOT), Some(Path::new("/home/.cargo"))),
        [
            unstable("/.cargo/config", "unstable.build-std"),
            lever("/.cargo/config.toml", "`build.rustflags`"),
            "/home/.cargo/config: the file does not parse as TOML, so the check cannot show \
             what it sets, and fails closed: TOML parse error at line 1, column 7"
                .to_owned(),
            compiler("/home/.cargo/config.toml", "build.rustc-wrapper"),
        ]
    );
    assert_eq!(
        outer_levers(&empty(), Path::new(ROOT), None),
        Vec::<String>::new()
    );
}

/// An environment of `pairs`, as the process hands it over.
fn environment(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
    pairs
        .iter()
        .map(|(name, value)| (OsString::from(name), OsString::from(value)))
        .collect()
}

#[test]
fn a_lever_in_a_flag_variable_of_the_environment_is_refused() {
    // The SEC-1i review: `RUSTFLAGS` and `CARGO_ENCODED_RUSTFLAGS` in the
    // shell that ran the gate enabled `yeet_expr`, and the gate passed.
    let found = env_levers(environment(&[
        ("RUSTFLAGS", "-Zcrate-attr=feature(yeet_expr)"),
        ("CARGO_ENCODED_RUSTFLAGS", "-Z\x1fthreads=2"),
        ("CARGO_BUILD_RUSTFLAGS", "--cfg feature(\"x\")"),
        ("CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS", "-Zx"),
        ("RUSTDOCFLAGS", "--crate-attr=x"),
        ("CARGO_ENCODED_RUSTDOCFLAGS", "-Zunstable-options"),
        ("CARGO_INCREMENTAL", "0"),
        ("PATH", "/a-Z/bin"),
    ]));
    let refused = |name: &str| lever("environment", &format!("`{name}`"));
    assert_eq!(
        found,
        [
            refused("CARGO_BUILD_RUSTFLAGS"),
            refused("CARGO_ENCODED_RUSTDOCFLAGS"),
            refused("CARGO_ENCODED_RUSTFLAGS"),
            refused("CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS"),
            refused("RUSTDOCFLAGS"),
            refused("RUSTFLAGS"),
        ]
    );
    // The flags of `lint-wasm` are no lever.
    let plain = environment(&[("RUSTFLAGS", "-Awarnings -C debuginfo=0")]);
    assert_eq!(env_levers(plain), Vec::<String>::new());
}

#[test]
fn a_compiler_or_an_unstable_variable_of_the_environment_is_refused() {
    let found = env_levers(environment(&[
        ("RUSTC", "rustc"),
        ("RUSTC_WRAPPER", ""),
        ("RUSTC_WORKSPACE_WRAPPER", "./wrap.sh"),
        ("CARGO_BUILD_RUSTC", "x"),
        ("CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER", "x"),
        ("CARGO_UNSTABLE_BUILD_STD", "std"),
    ]));
    let compiler = |name: &str| {
        format!(
            "environment: `{name}` names the compiler or a wrapper that runs it, which can pass \
             rustc a flag the check does not read, so the check fails closed (ARCH-033): unset it"
        )
    };
    assert_eq!(
        found,
        [
            compiler("CARGO_BUILD_RUSTC"),
            compiler("CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER"),
            "environment: `CARGO_UNSTABLE_BUILD_STD` is an unstable cargo setting, so the check \
             fails closed (ARCH-033): unset it"
                .to_owned(),
            compiler("RUSTC"),
            compiler("RUSTC_WORKSPACE_WRAPPER"),
            compiler("RUSTC_WRAPPER"),
        ]
    );
}

#[test]
fn a_patch_or_a_path_substitute_of_sqlx_is_refused() {
    // The SEC-1i review: a substitute named `sqlx` kept the name, so its
    // `migrate!` read as data.
    let tree = recipes(&[
        (
            "Cargo.toml",
            "[patch.crates-io]\nsqlx = { path = \"vend\" }\nother = { path = \"o\" }\n\
             [replace]\n\"sqlx:0.8.0\" = { git = \"x\" }\n",
        ),
        (
            "crates/demo/Cargo.toml",
            "[dependencies]\nsqlx = { path = \"../../vend\" }\n\
             [dev-dependencies]\nsqlx = { git = \"x\", version = \"0.8\" }\n\
             [build-dependencies]\nsqlx = { version = \"0.8\" }\n",
        ),
    ]);
    let substituted = |shown: &str, key: &str| {
        format!(
            "{shown}: `{key}` puts another crate under the name sqlx. The scan reads \
             `sqlx::migrate!` as a data read by its name, so the check fails closed (ARCH-033): \
             keep the name `sqlx` for the sqlx crate of the registry alone"
        )
    };
    assert_eq!(
        levers(&tree, Path::new(ROOT)),
        [
            substituted("Cargo.toml", "patch.crates-io.sqlx"),
            substituted("Cargo.toml", "replace.sqlx:0.8.0"),
            substituted("crates/demo/Cargo.toml", "dependencies.sqlx"),
            substituted("crates/demo/Cargo.toml", "dev-dependencies.sqlx"),
        ]
    );
}

#[test]
fn a_lever_on_a_line_of_the_justfile_is_refused() {
    let tree = Files::new(&[
        (
            "justfile",
            "# RUSTFLAGS=\"-Zx\" in a comment\n\
             lint-wasm:\n    RUSTFLAGS=\"-Awarnings -C debuginfo=0\" cargo clippy\n\
             \x20   RUSTFLAGS=\"-Awarnings -Zcrate-attr=feature(yeet_expr)\" cargo check\n\
             \x20   grep '[A-Z]' x\n\
             \x20   cargo +nightly build -Z build-std\n",
        ),
        (".github/workflows/ci.yml", "run: cargo test\n"),
    ]);
    assert_eq!(
        levers(&tree, Path::new(ROOT)),
        [
            lever("justfile:4", "the line"),
            lever("justfile:6", "the line")
        ]
    );
}

#[test]
fn a_lever_on_a_line_of_the_ci_workflow_is_refused() {
    let tree = Files::new(&[
        ("justfile", "lint:\n    cargo clippy\n"),
        (
            ".github/workflows/ci.yml",
            "env:\n  RUSTFLAGS: \"--cfg x\"\n  RUSTDOCFLAGS: \"--crate-attr=x\"\n\
             \x20 # RUSTFLAGS: -Zx\n  CARGO_ENCODED_RUSTFLAGS: \"-Zthreads=2\"\n",
        ),
    ]);
    assert_eq!(
        levers(&tree, Path::new(ROOT)),
        [
            lever(".github/workflows/ci.yml:3", "the line"),
            lever(".github/workflows/ci.yml:5", "the line"),
        ]
    );
    assert_eq!(
        levers(&Files::new(&[]), Path::new(ROOT)),
        [
            "justfile: the check cannot read it, so it cannot show that its flags enable no \
             unstable feature, and fails closed"
                .to_owned(),
            ".github/workflows/ci.yml: the check cannot read it, so it cannot show that its \
             flags enable no unstable feature, and fails closed"
                .to_owned(),
        ]
    );
}

#[cfg(unix)]
#[test]
fn the_listing_leaves_out_the_root_git_and_target_alone() {
    // The SEC-1g review: a `target/` or a `.git/` directory at any depth
    // left the listing, so a `.rs` file under one escaped the orphan check.
    let (root, scratch) = probe("listing", &[]);
    for file in [
        ".git/hooks/x.rs",
        "target/debug/y.rs",
        "fuzz/target/z.rs",
        "fuzz/fuzz_targets/one.rs",
        "crates/demo/src/target/mod.rs",
        "crates/demo/.git/w.rs",
        "crates/demo/fuzz/target/v.rs",
    ] {
        put(&root, file, "");
    }
    let listed: Vec<PathBuf> = Disk::at(&root)
        .files(&root)
        .into_iter()
        .filter_map(|path| path.strip_prefix(&root).ok().map(Path::to_path_buf))
        .collect();
    assert_eq!(
        listed,
        [
            "Cargo.toml",
            "crates/demo/.git/w.rs",
            "crates/demo/fuzz/target/v.rs",
            "crates/demo/src/target/mod.rs",
            "fuzz/fuzz_targets/one.rs",
        ]
        .map(PathBuf::from)
    );
    drop(scratch);
}

#[cfg(unix)]
#[test]
fn a_loop_of_directory_links_ends_the_listing() {
    // The SEC-1g review: the listing followed a link to a directory, so a
    // link to an ancestor listed its files again at each depth.
    let (root, scratch) = probe("links", &[]);
    put(&root, "a/f.rs", "");
    put(&root, "b.rs", "");
    for (link, target) in [("a/up", ".."), ("a/here", "."), ("a/file.rs", "../b.rs")] {
        if let Err(e) = std::os::unix::fs::symlink(target, root.join(link)) {
            panic!("the link {link} cannot be made: {e}");
        }
    }
    let listed: Vec<PathBuf> = Disk::at(&root)
        .files(&root)
        .into_iter()
        .filter_map(|path| path.strip_prefix(&root).ok().map(Path::to_path_buf))
        .collect();
    assert_eq!(
        listed,
        ["Cargo.toml", "a/f.rs", "a/file.rs", "b.rs"].map(PathBuf::from)
    );
    drop(scratch);
}
