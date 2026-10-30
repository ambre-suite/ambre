use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::{Report, Tally, report};
use crate::scan::{Tree, lexical, scan};

/// A source tree held in memory, so a test states every file it reads.
struct Memory(BTreeMap<PathBuf, String>);

impl Tree for Memory {
    fn read(&self, path: &Path) -> Option<String> {
        self.0.get(&lexical(path)).cloned()
    }

    fn files(&self, dir: &Path) -> Vec<PathBuf> {
        self.0
            .keys()
            .filter(|path| path.starts_with(dir))
            .cloned()
            .collect()
    }

    fn canonical(&self, path: &Path) -> PathBuf {
        lexical(path)
    }
}

const CRATES: &str = "/w/crates";

/// The report over one crate `demo` whose `src/lib.rs` is `lib`.
fn run(lib: &str) -> Report {
    let mut tree = BTreeMap::new();
    tree.insert(PathBuf::from(CRATES).join("demo/Cargo.toml"), String::new());
    tree.insert(
        PathBuf::from(CRATES).join("demo/src/lib.rs"),
        lib.to_owned(),
    );
    let tree = Memory(tree);
    let scanned = scan(&tree, Path::new(CRATES));
    assert!(scanned.errors.is_empty(), "{:?}", scanned.errors);
    report(&tree, &scanned)
}

/// The refusals, each as the report prints it.
fn refusals(report: &Report) -> Vec<String> {
    report.refusals.iter().map(ToString::to_string).collect()
}

/// A newtype over a `String` with its accessor, in a module of its own.
const NAME: &str = "pub mod name {\n\
    pub struct Name(String);\n\
    impl Name {\n\
        pub fn as_str(&self) -> &str { &self.0 }\n\
    }\n\
}\n\
use name::Name;\n\
pub fn store(text: &str) {}\n";

#[test]
fn an_unwrap_handed_to_a_workspace_fn_is_refused() {
    let lib = format!("{NAME}pub fn save(name: &Name) {{ store(name.as_str()); }}\n");
    let report = run(&lib);
    assert_eq!(
        refusals(&report),
        ["demo | src/lib.rs:9 | Name::as_str -> store"]
    );
    assert_eq!(report.sites, Tally::new(1));
}

#[test]
fn an_unwrap_kept_in_a_binding_is_refused_where_the_binding_reaches_the_workspace() {
    let lib = format!(
        "{NAME}pub fn save(name: &Name) {{\n\
         let text = name.as_str();\n\
         store(text);\n\
         }}\n"
    );
    assert_eq!(
        refusals(&run(&lib)),
        ["demo | src/lib.rs:11 | Name::as_str -> store"]
    );
}

#[test]
fn an_unwrap_handed_to_a_foreign_call_passes() {
    let lib = format!(
        "{NAME}pub fn save(name: &Name, out: &mut String) {{\n\
         out.push_str(name.as_str());\n\
         std::mem::drop(name.as_str());\n\
         }}\n"
    );
    let report = run(&lib);
    assert!(report.refusals.is_empty(), "{:?}", refusals(&report));
    assert!(report.ambiguous.is_empty());
    assert_eq!(report.foreign, Tally::new(2));
}

#[test]
fn the_own_impl_and_the_defining_module_of_a_newtype_unwrap_it_freely() {
    let lib = "pub mod name {\n\
        pub struct Name(String);\n\
        impl Name {\n\
            pub fn as_str(&self) -> &str { &self.0 }\n\
        }\n\
        pub fn spell(name: &Name) { super::store(name.as_str()); }\n\
    }\n\
    pub fn store(text: &str) {}\n\
    impl name::Name {\n\
        pub fn keep(&self) { store(self.as_str()); }\n\
    }\n";
    let report = run(lib);
    assert!(report.refusals.is_empty(), "{:?}", refusals(&report));
    assert_eq!(report.sites, Tally::new(0));
}

#[test]
fn a_method_both_the_workspace_and_std_define_is_ambiguous() {
    let lib = format!(
        "{NAME}pub struct Bag;\n\
         impl Bag {{ pub fn push(&mut self, text: &str) {{}} }}\n\
         pub fn save(name: &Name) {{\n\
         let mut bag = make();\n\
         bag.push(name.as_str());\n\
         }}\n"
    );
    let report = run(&lib);
    assert!(report.refusals.is_empty(), "{:?}", refusals(&report));
    let ambiguous: Vec<String> = report.ambiguous.iter().map(ToString::to_string).collect();
    assert_eq!(ambiguous, ["demo | src/lib.rs:13 | Name::as_str -> .push"]);
}

#[test]
fn a_method_only_the_workspace_defines_is_refused() {
    let lib = format!(
        "{NAME}pub struct Bag;\n\
         impl Bag {{ pub fn keep_text(&mut self, text: &str) {{}} }}\n\
         pub fn save(name: &Name) {{\n\
         let mut bag = make();\n\
         bag.keep_text(name.as_str());\n\
         }}\n"
    );
    assert_eq!(
        refusals(&run(&lib)),
        ["demo | src/lib.rs:13 | Name::as_str -> .keep_text"]
    );
}

#[test]
fn test_code_is_not_read() {
    let lib = format!(
        "{NAME}#[cfg(test)]\n\
         mod tests {{\n\
         fn save(name: &super::Name) {{ super::store(name.as_str()); }}\n\
         }}\n\
         #[test]\n\
         fn saves(name: &Name) {{ store(name.as_str()); }}\n"
    );
    let report = run(&lib);
    assert!(report.refusals.is_empty(), "{:?}", refusals(&report));
    assert_eq!(report.sites, Tally::new(0));
}

#[test]
fn a_receiver_of_another_type_is_no_unwrap() {
    let lib = format!("{NAME}pub fn save(text: &String) {{ store(text.as_str()); }}\n");
    let report = run(&lib);
    assert!(report.refusals.is_empty(), "{:?}", refusals(&report));
    assert_eq!(report.sites, Tally::new(0));
}

#[test]
fn a_std_accessor_on_an_unknown_receiver_is_ambiguous() {
    let lib = format!(
        "{NAME}pub fn save() {{\n\
         let text = make();\n\
         store(text.as_str());\n\
         }}\n"
    );
    let report = run(&lib);
    assert!(report.refusals.is_empty(), "{:?}", refusals(&report));
    assert_eq!(report.ambiguous_receivers(), Tally::new(1));
}

#[test]
fn a_newtype_a_macro_family_writes_carries_the_accessors_of_the_macro() {
    let lib = "macro_rules! define_id {\n\
        ($name:ident) => {\n\
            pub struct $name(Uuid);\n\
            impl $name {\n\
                pub fn as_bytes(&self) -> &[u8; 16] { self.0.as_bytes() }\n\
                pub fn is_nil(&self) -> bool { false }\n\
            }\n\
        };\n\
    }\n\
    pub mod ids { define_id!(DocId); }\n\
    pub fn keep(bytes: &[u8; 16]) {}\n\
    pub fn save(id: &ids::DocId) { keep(id.as_bytes()); }\n";
    assert_eq!(
        refusals(&run(lib)),
        ["demo | src/lib.rs:12 | DocId::as_bytes -> keep"]
    );
}

#[test]
fn a_field_of_a_known_struct_types_its_receiver() {
    let lib = format!(
        "{NAME}pub struct Doc {{ name: Name }}\n\
         impl Doc {{ pub fn save(&self) {{ store(self.name.as_str()); }} }}\n"
    );
    assert_eq!(
        refusals(&run(&lib)),
        ["demo | src/lib.rs:10 | Name::as_str -> store"]
    );
}
