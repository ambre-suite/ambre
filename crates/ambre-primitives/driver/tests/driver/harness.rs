//! A workspace of fixture crates, compiled through the driver as cargo would.
//!
//! Each test writes its crates into a fresh directory. It compiles them in
//! order with the driver in place of rustc, and reads the records of each.
//! A crate whose name starts with `ambre_` is a crate of the workspace to the
//! driver; any other name is a foreign crate.

use std::path::{Path, PathBuf};
use std::process::Command;

/// One fixture workspace, removed on drop.
pub struct Workspace {
    root: PathBuf,
}

/// How a crate compiles: as a shipped build, or as a test build.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Build {
    Shipped,
    Test,
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Workspace {
    /// A fresh, empty workspace.
    pub fn new(name: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("ambre-driver-{name}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("out")).expect("a fixture directory");
        Self {
            root: std::fs::canonicalize(&root).expect("a canonical root"),
        }
    }

    /// The root of the workspace.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Writes `text` at `path`, from the root.
    pub fn file(&self, path: &str, text: &str) -> &Self {
        let path = self.root.join(path);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("a directory");
        std::fs::write(path, text).expect("a fixture file");
        self
    }

    /// Compiles the crate `name` of the package in the directory `dir` of
    /// `crates`, from its `src/lib.rs`, against the crates `externs`. Returns
    /// the records of the unit, one per line.
    pub fn compile(&self, name: &str, dir: &str, externs: &[&str], build: Build) -> Vec<String> {
        let package = ["crates", dir].join("/");
        let out = self.root.join("out");
        let records = out.join(match build {
            Build::Shipped => "records",
            Build::Test => "records-test",
        });
        let mut command = Command::new(env!("CARGO_BIN_EXE_ambre-primitives-driver"));
        command
            .current_dir(&self.root)
            .arg("rustc")
            .args([
                "--edition",
                "2024",
                "--crate-name",
                name,
                "--crate-type",
                "lib",
            ])
            .args(["--emit", "metadata,link", "-A", "warnings"])
            .arg("--out-dir")
            .arg(&out)
            .arg("-L")
            .arg(&out)
            .env("AMBRE_GATE_OUT", &records)
            .env("CARGO_MANIFEST_DIR", self.root.join(&package))
            .env("LD_LIBRARY_PATH", sysroot().join("lib"));
        if build == Build::Test {
            command.arg("--test");
        }
        for krate in externs {
            command.arg("--extern").arg(format!(
                "{krate}={}",
                out.join(format!("lib{krate}.rlib")).display()
            ));
        }
        command.arg(format!("{package}/src/lib.rs"));
        let output = command.output().expect("the driver runs");
        assert!(
            output.status.success(),
            "{name} does not compile:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = std::fs::read_to_string(records.join(format!("{name}.tsv")))
            .expect("the records of the unit");
        text.lines().map(str::to_owned).collect()
    }
}

/// The sysroot of the toolchain the tests run on, whose `lib` holds the
/// `rustc_driver` the driver links.
fn sysroot() -> PathBuf {
    let output = Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .expect("rustc runs");
    PathBuf::from(String::from_utf8_lossy(&output.stdout).trim())
}

/// The records whose first field is `kind`, that field left out.
pub fn of(records: &[String], kind: &str) -> Vec<String> {
    records
        .iter()
        .filter_map(|line| line.strip_prefix(&format!("{kind}\t")))
        .map(str::to_owned)
        .collect()
}

/// The key of each site record.
pub fn keys(records: &[String]) -> Vec<String> {
    of(records, "site")
        .iter()
        .filter_map(|line| line.split('\t').next())
        .map(str::to_owned)
        .collect()
}

/// The fields of the site record of `key`, the key left out.
pub fn site(records: &[String], key: &str) -> Vec<String> {
    of(records, "site")
        .iter()
        .find(|line| line.split('\t').next() == Some(key))
        .unwrap_or_else(|| panic!("no site {key} in {records:#?}"))
        .split('\t')
        .skip(1)
        .map(str::to_owned)
        .collect()
}
