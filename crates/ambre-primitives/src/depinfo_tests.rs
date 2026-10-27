use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::{Build, COMPILES, Compile, Compiled, Target, Unit, collect, cross_check, dependencies};
use crate::scan::{Disk, Scan, scan};

const ROOT: &str = "/w";

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
                \n\
                src/lib.rs:\n\
                # env-dep:CARGO_PKG_NAME=a\n";
    assert_eq!(
        dependencies(text),
        ["src/lib.rs", "src/with space.rs", "data/tokens.ron"].map(PathBuf::from)
    );
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
        cross_check(&scan, &compiled, &sources, Path::new(ROOT)),
        [
            "crates/demo/src/hidden.rs: rustc reads it for the crate root crates/demo/src/lib.rs, \
          and the walk does not, so the gate cannot place it"
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
                &["crates/demo/src/lib.rs", "crates/demo/src/tests.rs"],
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
        cross_check(&scan, &compiled, &sources, Path::new(ROOT)),
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
        cross_check(&scan, &compiled, &sources, Path::new(ROOT)),
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
    let errors = cross_check(&scan, &compiled, &sources, Path::new(ROOT));
    assert_eq!(
        errors[0],
        "crates/demo/src/tool/main.rs: rustc compiles this crate root, and the walk does not \
         start from it"
    );
    assert_eq!(errors.len(), 2, "{errors:?}");
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

#[cfg(unix)]
#[test]
fn a_real_compile_reads_the_files_the_walk_misses() {
    // The probes of the SEC-1e review, compiled by cargo in a workspace of
    // their own. The dep-info of each is what rustc read.
    let root = std::env::temp_dir().join(format!("ambre-depinfo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let scratch = Scratch(root.clone());
    put(
        &root,
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/ambre-server\"]\nresolver = \"2\"\n",
    );
    let krate = "crates/ambre-server";
    let file = |path: &str, text: &str| put(&root, &format!("{krate}/{path}"), text);
    file(
        "Cargo.toml",
        "[package]\nname = \"ambre-server\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
    );
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

    let cargo = PathBuf::from(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    let rustc = cargo.with_file_name("rustc");
    let target = root.join("target");
    let compiles = [Compile {
        target: Target::Host,
        args: &["--workspace", "--all-targets"],
    }];
    let compiled = collect(&root, &compiles, &|command| {
        command
            .arg("--offline")
            .env("CARGO_TARGET_DIR", &target)
            .env_remove("RUSTFLAGS")
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .env_remove("CARGO_BUILD_TARGET");
        if rustc.is_file() {
            command.env("RUSTC", &rustc);
        }
    });
    let compiled = match compiled {
        Ok(compiled) => compiled,
        Err(errors) => panic!("the probe does not compile: {errors:?}"),
    };
    let canonical = match std::fs::canonicalize(&root) {
        Ok(canonical) => canonical,
        Err(e) => panic!("{}: {e}", root.display()),
    };
    let sources = super::sources(&Disk, &root.join("crates"));
    let errors = cross_check(&walk, &compiled, &sources, &canonical);
    let lib = "crates/ambre-server/src/lib.rs";
    assert_eq!(
        errors,
        [
            format!(
                "crates/ambre-server/src/leak_tests.rs: rustc reads it for the crate root {lib}, \
                 and the walk does not, so the gate cannot place it"
            ),
            format!(
                "crates/ambre-server/src/spliced_tests.rs: rustc reads it for the crate root \
                 {lib}, and the walk does not, so the gate cannot place it"
            ),
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
