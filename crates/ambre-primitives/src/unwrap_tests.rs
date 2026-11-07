use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use syn::visit::Visit;

use super::{
    CatalogVisit, Finding, HEADER, Outcome, Phase, Raw, Report, Row, Scope, SiteCount, Strictness,
    Uses, judge, parse_allowlist, report,
};
use crate::allowlist::LineNo;
use crate::scan::{META, Tree, lexical, scan};

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
    run_crates(&[("demo", "", lib)])
}

/// The report over crates, each a name, a manifest and a `src/lib.rs`.
fn run_crates(crates: &[(&str, &str, &str)]) -> Report {
    let mut tree = BTreeMap::new();
    for (name, manifest, lib) in crates {
        let dir = PathBuf::from(CRATES).join(name);
        tree.insert(dir.join("Cargo.toml"), (*manifest).to_owned());
        tree.insert(dir.join("src/lib.rs"), (*lib).to_owned());
    }
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
    assert_eq!(report.sites, SiteCount::new(1));
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
    assert_eq!(report.foreign, SiteCount::new(2));
}

#[test]
fn a_generic_newtype_over_a_marker_is_a_newtype_and_its_accessor_an_unwrap() {
    let lib = "pub mod id {\n\
        pub trait Kind { type Repr; }\n\
        pub struct Id<E: Kind>(E::Repr);\n\
        impl<E: Kind<Repr = u64>> Id<E> {\n\
            pub fn raw(&self) -> <E as Kind>::Repr { self.0 }\n\
        }\n\
        pub trait Space { type Repr; }\n\
        pub struct Px<S: Space>(S::Repr, core::marker::PhantomData<S>);\n\
        impl<S: Space<Repr = u64>> Px<S> {\n\
            pub fn get(self) -> S::Repr { self.0 }\n\
        }\n\
        pub struct Wrapper<T>(T);\n\
        impl<T> Wrapper<T> {\n\
            pub fn take(self) -> T { self.0 }\n\
        }\n\
    }\n\
    use id::{Id, Px, Wrapper};\n\
    pub struct Doc; impl id::Kind for Doc { type Repr = u64; } \
    impl id::Space for Doc { type Repr = u64; }\n\
    pub fn store(text: u64) {}\n\
    pub fn save(id: &Id<Doc>, px: Px<Doc>, w: Wrapper<u64>) {\n\
        store(id.raw());\n\
        store(px.get());\n\
        store(w.take());\n\
    }\n";
    assert_eq!(
        refusals(&run(lib)),
        [
            "demo | src/lib.rs:21 | Id::raw -> store",
            "demo | src/lib.rs:22 | Px::get -> store",
        ]
    );
}

#[test]
fn a_generic_struct_over_no_primitive_and_its_marker_are_no_unwrap() {
    let lib = "pub mod id {\n\
        pub trait Kind { type Repr; }\n\
        pub struct Handle<K: Kind>(K::Repr);\n\
        impl<K: Kind> Handle<K> {\n\
            pub fn inner(&self) -> &K::Repr { &self.0 }\n\
        }\n\
        pub trait Space { type Repr; }\n\
        pub struct Px<S: Space>(S::Repr, core::marker::PhantomData<S>);\n\
        impl<S: Space + Default> Px<S> {\n\
            pub fn space(&self) -> S { S::default() }\n\
        }\n\
    }\n\
    use id::{Handle, Px};\n\
    pub struct Config;\n\
    pub struct Doc; impl id::Kind for Doc { type Repr = Config; } \
    impl id::Space for Doc { type Repr = u64; }\n\
    pub fn apply(config: &Config) {}\n\
    pub fn shape(doc: Doc) {}\n\
    pub fn save(h: &Handle<Doc>, px: Px<Doc>) {\n\
        apply(h.inner());\n\
        shape(px.space());\n\
    }\n";
    assert!(refusals(&run(lib)).is_empty(), "{:?}", refusals(&run(lib)));
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
    assert_eq!(report.sites, SiteCount::new(0));
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
    assert_eq!(report.sites, SiteCount::new(0));
}

#[test]
fn a_receiver_of_another_type_is_no_unwrap() {
    let lib = format!("{NAME}pub fn save(text: &String) {{ store(text.as_str()); }}\n");
    let report = run(&lib);
    assert!(report.refusals.is_empty(), "{:?}", refusals(&report));
    assert_eq!(report.sites, SiteCount::new(0));
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
    assert_eq!(report.ambiguous_receivers(), SiteCount::new(1));
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

#[test]
fn a_field_or_a_binding_a_type_parameter_types_is_untyped() {
    // `width` of `Size<P>` holds any newtype, so its call is judged. The
    // accessor `spell` no std type shares, so the receiver is known.
    let lib = "pub mod len {\n\
            pub struct Len(u32);\n\
            impl Len { pub fn spell(self) -> u32 { self.0 } }\n\
        }\n\
        pub fn keep(n: u32) {}\n\
        pub struct Size<P> { pub width: P, pub height: P }\n\
        impl<P> Size<P> { pub fn keep(&self) { keep(self.width.spell()); } }\n\
        pub fn save(size: &Size<len::Len>) { keep(size.height.spell()); }\n\
        pub fn spell<K>(key: K) { keep(key.spell()); }\n";
    let report = run(lib);
    assert!(report.ambiguous.is_empty(), "{:?}", ambiguous(&report));
    assert_eq!(
        refusals(&report),
        [
            "demo | src/lib.rs:7 | Len::spell -> keep",
            "demo | src/lib.rs:8 | Len::spell -> keep",
            "demo | src/lib.rs:9 | Len::spell -> keep",
        ]
    );
}

#[test]
fn a_field_of_an_untyped_base_with_a_generic_name_keeps_the_name_filter() {
    // A kurbo point holds `x` too: an untyped base names no `Point<P>`.
    let lib = format!(
        "{NAME}pub struct Point<P> {{ pub x: P }}\n\
         pub fn save() {{ let at = make(); store(at.x.as_str()); }}\n"
    );
    let report = run(&lib);
    assert!(report.refusals.is_empty(), "{:?}", refusals(&report));
    assert!(report.ambiguous.is_empty(), "{:?}", ambiguous(&report));
}

#[test]
fn a_public_tuple_field_is_an_unwrap_and_a_private_one_is_not() {
    let lib = "pub mod key {\n\
        pub struct Key(pub String);\n\
        pub struct Seal(String);\n\
        pub mod inner {\n\
            pub fn hide(seal: &super::Seal) { crate::store(&seal.0); }\n\
        }\n\
    }\n\
    pub fn store(text: &str) {}\n\
    pub fn save(key: &key::Key) { store(&key.0); }\n";
    let report = run(lib);
    assert_eq!(refusals(&report), ["demo | src/lib.rs:9 | Key::0 -> store"]);
    assert_eq!(report.sites, SiteCount::new(1));
}

#[test]
fn to_string_is_an_unwrap_when_the_display_reads_a_field() {
    let lib = "pub mod name {\n\
        pub struct Name(String);\n\
        impl std::fmt::Display for Name {\n\
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {\n\
                f.write_str(&self.0)\n\
            }\n\
        }\n\
        pub struct Token(String);\n\
        impl std::fmt::Display for Token {\n\
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {\n\
                f.write_str(\"[token]\")\n\
            }\n\
        }\n\
    }\n\
    pub fn store(text: &str) {}\n\
    pub fn save(name: &name::Name, token: &name::Token) {\n\
        store(&name.to_string());\n\
        store(&token.to_string());\n\
    }\n";
    assert_eq!(
        refusals(&run(lib)),
        ["demo | src/lib.rs:17 | Name::to_string -> store"]
    );
}

#[test]
fn the_tuple_struct_of_a_newtype_is_refused_like_its_new() {
    let lib = "pub mod offset {\n\
        pub struct DocOffset(pub u64);\n\
        impl DocOffset { pub fn new(at: u64) -> Self { Self(at) } }\n\
        pub struct RunLen(u64);\n\
        impl RunLen { pub fn get(&self) -> u64 { self.0 } }\n\
    }\n\
    use offset::{DocOffset, RunLen};\n\
    pub fn place(run: &RunLen) {\n\
        DocOffset::new(run.get());\n\
        DocOffset(run.get());\n\
        Some(run.get());\n\
    }\n";
    assert_eq!(
        refusals(&run(lib)),
        [
            "demo | src/lib.rs:9 | RunLen::get -> DocOffset::new",
            "demo | src/lib.rs:10 | RunLen::get -> DocOffset",
        ]
    );
}

#[test]
fn the_impl_of_a_newtype_unwraps_that_newtype_alone() {
    let lib = format!(
        "{NAME}pub mod key {{\n\
         pub struct Key(String);\n\
         impl Key {{ pub fn as_str(&self) -> &str {{ &self.0 }} }}\n\
         }}\n\
         impl Name {{\n\
         pub fn mine(&self) {{ store(self.as_str()); }}\n\
         pub fn theirs(&self, key: &key::Key) {{ store(key.as_str()); }}\n\
         }}\n"
    );
    assert_eq!(
        refusals(&run(&lib)),
        ["demo | src/lib.rs:15 | Key::as_str -> store"]
    );
}

#[test]
fn a_conversion_impl_unwraps_the_newtype_it_converts() {
    let lib = format!(
        "{NAME}pub struct Label;\n\
         impl From<Name> for Label {{\n\
         fn from(name: Name) -> Self {{ store(name.as_str()); Label }}\n\
         }}\n"
    );
    let report = run(&lib);
    assert!(report.refusals.is_empty(), "{:?}", refusals(&report));
    assert_eq!(report.sites, SiteCount::new(0));
}

#[test]
fn a_from_impl_into_a_primitive_makes_into_and_from_unwraps() {
    let lib = "pub mod query {\n\
        pub struct Query(String);\n\
        impl From<Query> for String { fn from(q: Query) -> String { q.0 } }\n\
    }\n\
    use query::Query;\n\
    pub fn store(text: &str) {}\n\
    pub fn save(q: Query, r: Query) {\n\
        let text: String = q.into();\n\
        store(&text);\n\
        store(&String::from(r));\n\
        store(&make().into());\n\
    }\n";
    let report = run(lib);
    assert_eq!(
        refusals(&report),
        [
            "demo | src/lib.rs:9 | Query::into -> store",
            "demo | src/lib.rs:10 | Query::from -> store",
        ]
    );
    assert!(report.ambiguous.is_empty());
}

#[test]
fn a_hand_written_as_ref_of_bytes_is_an_accessor() {
    let lib = "pub mod blob {\n\
        pub struct Blob(Vec<u8>);\n\
        impl AsRef<[u8]> for Blob { fn as_ref(&self) -> &[u8] { &self.0 } }\n\
    }\n\
    pub fn keep(bytes: &[u8]) {}\n\
    pub fn save(blob: &blob::Blob) { keep(blob.as_ref()); }\n";
    assert_eq!(
        refusals(&run(lib)),
        ["demo | src/lib.rs:6 | Blob::as_ref -> keep"]
    );
}

#[test]
fn an_accessor_many_newtypes_share_is_ambiguous_on_an_untyped_receiver() {
    let lib = "pub mod units {\n\
        pub struct A(u32); impl A { pub fn to_usize(self) -> usize { 0 } }\n\
        pub struct B(u32); impl B { pub fn to_usize(self) -> usize { 0 } }\n\
        pub struct C(u32); impl C { pub fn to_usize(self) -> usize { 0 } }\n\
        pub struct D(u32); impl D { pub fn to_usize(self) -> usize { 0 } }\n\
    }\n\
    pub fn keep(n: usize) {}\n\
    pub fn save() { keep(make().to_usize()); }\n";
    let report = run(lib);
    assert!(report.refusals.is_empty(), "{:?}", refusals(&report));
    let ambiguous: Vec<String> = report.ambiguous.iter().map(ToString::to_string).collect();
    assert_eq!(ambiguous, ["demo | src/lib.rs:8 | *::to_usize -> keep"]);
    assert_eq!(report.ambiguous_receivers(), SiteCount::new(1));
}

#[test]
fn an_untyped_receiver_drops_the_site_only_when_every_candidate_is_local() {
    let lib = "pub mod here {\n\
        pub struct Mine(u32);\n\
        impl Mine { pub fn spell(self) -> u32 { self.0 } }\n\
        pub fn keep(n: u32) {}\n\
        pub fn save() { keep(make().spell()); }\n\
    }\n\
    pub mod there {\n\
        pub struct Theirs(u32);\n\
        impl Theirs { pub fn spell_out(self) -> u32 { self.0 } }\n\
        pub struct Other(u32);\n\
        impl Other { pub fn spell(self) -> u32 { self.0 } }\n\
        pub fn keep(n: u32) {}\n\
        pub fn save() { keep(make().spell_out()); }\n\
    }\n";
    let report = run(lib);
    assert!(report.refusals.is_empty(), "{:?}", refusals(&report));
    let ambiguous: Vec<String> = report.ambiguous.iter().map(ToString::to_string).collect();
    assert_eq!(
        ambiguous,
        ["demo | src/lib.rs:5 | Mine|Other::spell -> keep"]
    );
}

/// The allowlist text over the header, one line per row.
fn allowlist(rows: &[&str]) -> String {
    let mut text = format!("# comment\n{HEADER}\n");
    for row in rows {
        text.push_str(row);
        text.push('\n');
    }
    text
}

#[test]
fn the_allowlist_names_each_defect_of_its_lines() {
    let text = "key\treason\n\
        a\t0\tzero\n\
        b\tx\tnot a number\n\
        c\t1\t \n\
        d\n\
        e\t1\tfine\n\
        e\t2\ttwice\n";
    let errors = parse_allowlist(text).unwrap_err();
    assert_eq!(
        errors,
        [
            format!("line 1: the header is {HEADER:?}"),
            "line 2: the count is a whole number above 0".to_owned(),
            "line 3: the count is a whole number above 0".to_owned(),
            "line 4: the reason is empty".to_owned(),
            "line 5: a key, a tab, a count, a tab, then the reason".to_owned(),
            "line 7: the key is named twice".to_owned(),
        ]
    );
    assert_eq!(
        parse_allowlist("# only a comment\n").unwrap_err(),
        [format!("the header {HEADER:?} is missing")]
    );
    let rows = parse_allowlist(&allowlist(&["k\t2\tthe reason"])).unwrap();
    assert_eq!(
        rows.get("k"),
        Some(&Row {
            line: LineNo::new(3),
            count: SiteCount::new(2)
        })
    );
}

/// A report with two sites of one key, `Name::as_str -> store` in `save`.
fn two_sites() -> (Report, String) {
    let lib = format!(
        "{NAME}pub fn save(name: &Name) {{\n\
         store(name.as_str());\n\
         store(name.as_str());\n\
         }}\n"
    );
    let report = run(&lib);
    let key = report.refusals.first().map(Finding::key).unwrap();
    assert_eq!(report.refusals.len(), 2);
    assert_eq!(
        key,
        "demo | src/lib.rs | crate::save | Name::as_str -> store"
    );
    (report, key)
}

#[test]
fn report_mode_passes_on_refusals_and_strict_mode_fails() {
    let (report, key) = two_sites();
    let empty = allowlist(&[]);
    let lenient = judge(&report, Some(&empty), Strictness::Report).unwrap();
    assert_eq!(lenient.outcome, Outcome::Pass);
    assert_eq!(lenient.unlisted, SiteCount::new(1));
    assert_eq!(
        lenient.problems,
        [format!(
            "no line names the key, the report finds 2\n  {key}"
        )]
    );
    let strict = judge(&report, Some(&empty), Strictness::Strict).unwrap();
    assert_eq!(strict.outcome, Outcome::Fail);
}

#[test]
fn a_missing_or_malformed_allowlist_fails_both_modes() {
    let (report, _) = two_sites();
    for strictness in [Strictness::Report, Strictness::Strict] {
        assert!(judge(&report, None, strictness).is_err());
        assert!(judge(&report, Some("key\treason\n"), strictness).is_err());
    }
}

#[test]
fn strict_mode_holds_each_line_to_its_count() {
    let (report, key) = two_sites();
    let exact = allowlist(&[&format!("{key}\t2\tthe reason")]);
    let passed = judge(&report, Some(&exact), Strictness::Strict).unwrap();
    assert_eq!(passed.outcome, Outcome::Pass);
    assert!(passed.problems.is_empty());

    let short = allowlist(&[&format!("{key}\t1\tthe reason")]);
    let over = judge(&report, Some(&short), Strictness::Strict).unwrap();
    assert_eq!(over.outcome, Outcome::Fail);
    assert_eq!(over.over, SiteCount::new(1));

    let long = allowlist(&[&format!("{key}\t3\tthe reason")]);
    let stale = judge(&report, Some(&long), Strictness::Strict).unwrap();
    assert_eq!(stale.outcome, Outcome::Fail);
    assert_eq!(stale.stale, SiteCount::new(1));

    let gone = allowlist(&[&format!("{key}\t2\tthe reason"), "no such key\t1\tgone"]);
    let judged = judge(&report, Some(&gone), Strictness::Strict).unwrap();
    assert_eq!(judged.outcome, Outcome::Fail);
    assert_eq!(judged.stale, SiteCount::new(1));
    assert_eq!(
        judged.problems,
        ["line 4 allows 1 sites, the report finds 0\n  no such key"]
    );
    let lenient = judge(&report, Some(&gone), Strictness::Report).unwrap();
    assert_eq!(lenient.outcome, Outcome::Pass);
}

/// A newtype whose `Display` prints its text, so `to_string` unwraps it,
/// and a workspace type whose `push` and `contains` std shares.
const SHOWN: &str = "pub mod name {\n\
    pub struct Name(String);\n\
    impl Name { pub fn as_str(&self) -> &str { &self.0 } }\n\
    impl std::fmt::Display for Name {\n\
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(&self.0) }\n\
    }\n\
    pub struct Bag;\n\
    impl Bag {\n\
        pub fn push(&mut self, text: &str) {}\n\
        pub fn contains(&self, text: &str) -> bool { false }\n\
    }\n\
}\n\
use name::Name;\n\
pub fn store(text: &str) {}\n";

/// Each ambiguous site, as the report prints it.
fn ambiguous(report: &Report) -> Vec<String> {
    report.ambiguous.iter().map(ToString::to_string).collect()
}

#[test]
fn a_name_a_foreign_variant_binds_holds_a_foreign_value() {
    let lib = format!(
        "{SHOWN}use proc_macro2::TokenTree;\n\
         pub fn words(tree: Tree) {{\n\
         if let TokenTree::Ident(ident) = make() {{ store(&ident.to_string()); }}\n\
         if let syn::Pat::Ident(p) = make() {{ store(&p.to_string()); }}\n\
         if let Some(other) = make() {{ store(&other.to_string()); }}\n\
         }}\n"
    );
    let report = run(&lib);
    assert!(report.refusals.is_empty(), "{:?}", refusals(&report));
    assert_eq!(
        ambiguous(&report),
        ["demo | src/lib.rs:19 | Name::to_string -> store"]
    );
}

#[test]
fn a_const_types_its_receiver() {
    let lib = format!(
        "{SHOWN}const WORDS: &[&str] = &[\"a\"];\n\
         const SEED: Name = make();\n\
         pub fn check(name: &Name) {{\n\
         WORDS.contains(&name.as_str());\n\
         store(SEED.as_str());\n\
         }}\n"
    );
    let report = run(&lib);
    assert_eq!(
        refusals(&report),
        ["demo | src/lib.rs:19 | Name::as_str -> store"]
    );
    assert!(report.ambiguous.is_empty(), "{:?}", ambiguous(&report));
    assert_eq!(report.foreign, SiteCount::new(1));
}

#[test]
fn a_std_constructor_a_macro_an_array_and_a_tuple_field_type_their_receiver() {
    let lib = format!(
        "{SHOWN}pub struct Pair(Vec<String>, u32);\n\
         impl Pair {{\n\
         pub fn fill(&mut self, name: &Name) {{\n\
         let mut out = Vec::new();\n\
         out.push(name.as_str());\n\
         let mut text = format!(\"x\");\n\
         text.push(name.as_str());\n\
         [\"a\"].contains(&name.as_str());\n\
         self.0.push(name.as_str());\n\
         }}\n\
         }}\n"
    );
    let report = run(&lib);
    assert!(report.refusals.is_empty(), "{:?}", refusals(&report));
    assert!(report.ambiguous.is_empty(), "{:?}", ambiguous(&report));
    assert_eq!(report.foreign, SiteCount::new(4));
}

#[test]
fn a_method_whose_return_std_or_a_dependency_fixes_types_its_value() {
    let lib = format!(
        "{SHOWN}pub fn spell(ty: Ty, path: Path) {{\n\
         store(&ty.to_token_stream().to_string());\n\
         store(&path.display().to_string());\n\
         let text = make().to_string();\n\
         store(text.as_str());\n\
         }}\n"
    );
    let report = run(&lib);
    assert!(report.refusals.is_empty(), "{:?}", refusals(&report));
    assert_eq!(
        ambiguous(&report),
        ["demo | src/lib.rs:19 | Name::to_string -> store"]
    );
}

#[test]
fn a_field_no_workspace_struct_holds_as_a_newtype_holds_none() {
    let lib = format!(
        "{SHOWN}pub struct Doc {{ title: Name }}\n\
         pub fn spell() {{\n\
         let f = make();\n\
         store(&f.ident.to_string());\n\
         store(f.title.as_str());\n\
         }}\n"
    );
    let report = run(&lib);
    assert!(report.refusals.is_empty(), "{:?}", refusals(&report));
    assert_eq!(
        ambiguous(&report),
        ["demo | src/lib.rs:19 | Name::as_str -> store"]
    );
}

/// Each finding as its newtype, accessor and callee, the line left out.
fn shapes(findings: &[Finding]) -> Vec<String> {
    findings
        .iter()
        .map(|f| format!("{}::{} -> {}", f.newtype, f.accessor, f.callee))
        .collect()
}

/// Two counts in their own module, each with its accessor.
const DELTAS: &str = "pub mod delta {\n\
    pub struct RowDelta(i64);\n\
    impl RowDelta { pub fn get(self) -> i64 { self.0 } }\n\
    pub struct ColDelta(i64);\n\
    impl ColDelta { pub fn get(self) -> i64 { self.0 } }\n\
}\n\
use delta::{ColDelta, RowDelta};\n";

#[test]
fn a_named_conversion_unwraps_its_one_newtype_freely() {
    let lib = format!(
        "{DELTAS}pub struct Term(i64);\n\
         impl Term {{\n\
         pub fn of_row(delta: RowDelta) -> Self {{ Self(delta.get()) }}\n\
         pub fn of_col(delta: &ColDelta) -> Term {{ Term(delta.get()) }}\n\
         }}\n"
    );
    let report = run(&lib);
    assert!(report.refusals.is_empty(), "{:?}", refusals(&report));
}

#[test]
fn a_two_parameter_fn_or_a_method_is_no_named_conversion() {
    let lib = format!(
        "{DELTAS}pub struct Term(i64);\n\
         impl Term {{\n\
         pub fn of_both(row: RowDelta, col: ColDelta) -> Self {{ Self(row.get()) }}\n\
         pub fn shifted(&self, delta: RowDelta) -> Self {{ Self(delta.get()) }}\n\
         }}\n"
    );
    assert_eq!(
        shapes(&run(&lib).refusals),
        ["RowDelta::get -> Self", "RowDelta::get -> Self"]
    );
}

/// A `macro_rules!` that writes a typed handle over a foreign map.
const FIELDS: &str = "macro_rules! fields {\n\
    ($vis:vis) => {\n\
        $vis struct Fields { map: ::loro::LoroMap }\n\
        impl Fields {\n\
            $vis fn of(map: ::loro::LoroMap) -> Self { Self { map } }\n\
            $vis fn insert(&self, key: u8, value: impl Into<::loro::LoroValue>) {}\n\
            $vis fn label(&self, key: u8, text: &str) {}\n\
        }\n\
    };\n\
}\n\
fields!(pub);\n";

#[test]
fn an_impl_into_foreign_parameter_is_a_boundary() {
    let lib = format!(
        "{NAME}{FIELDS}pub fn save(name: &Name, map: Map) {{\n\
         let fields = Fields::of(map);\n\
         fields.insert(1, name.as_str());\n\
         fields.label(1, name.as_str());\n\
         }}\n"
    );
    let report = run(&lib);
    assert_eq!(shapes(&report.refusals), ["Name::as_str -> .label"]);
    assert!(report.ambiguous.is_empty(), "{:?}", ambiguous(&report));
    assert_eq!(report.foreign, SiteCount::new(1));
}

#[test]
fn a_return_type_types_the_value_a_call_returns() {
    let lib = format!(
        "{NAME}pub fn named() -> Name {{ make() }}\n\
         pub fn parsed() -> Option<Name> {{ make() }}\n\
         pub fn save() -> Option<()> {{\n\
         store(named().as_str());\n\
         store(parsed()?.as_str());\n\
         None\n\
         }}\n"
    );
    assert_eq!(
        shapes(&run(&lib).refusals),
        ["Name::as_str -> store", "Name::as_str -> store"]
    );
}

#[test]
fn a_method_of_a_crate_out_of_reach_is_foreign() {
    let tool = "pub struct Visit;\n\
        impl Visit { pub fn bind(&mut self, text: &str) {} }\n";
    let lib = format!("{NAME}pub fn save(name: &Name) {{ make().bind(name.as_str()); }}\n");
    let apart = run_crates(&[("demo", "", &lib), ("tool", "", tool)]);
    assert!(apart.refusals.is_empty(), "{:?}", refusals(&apart));
    assert!(apart.ambiguous.is_empty(), "{:?}", ambiguous(&apart));
    assert_eq!(apart.foreign, SiteCount::new(1));
    let manifest = "[dependencies]\ntool = { path = \"../tool\" }\n";
    let reached = run_crates(&[("demo", manifest, &lib), ("tool", "", tool)]);
    assert_eq!(shapes(&reached.ambiguous), ["Name::as_str -> .bind"]);
}

#[test]
fn the_crate_a_path_names_decides_its_type() {
    let draw = "pub use kurbo::Size;\n";
    let lib = format!(
        "{NAME}pub struct Size(u32);\n\
         impl Size {{ pub fn new(text: &str) -> Self {{ Self(0) }} }}\n\
         pub fn save(name: &Name) {{\n\
         draw::Size::new(name.as_str());\n\
         Size::new(name.as_str());\n\
         }}\n"
    );
    let report = run_crates(&[
        ("demo", "[dependencies]\ndraw = \"1\"\n", &lib),
        ("draw", "", draw),
    ]);
    assert_eq!(shapes(&report.refusals), ["Name::as_str -> Size::new"]);
    assert!(report.ambiguous.is_empty(), "{:?}", ambiguous(&report));
}

#[test]
fn option_as_ref_and_refcell_borrow_on_an_untyped_receiver_are_no_unwrap() {
    let lib = "pub mod blob {\n\
        pub struct Blob(Vec<u8>);\n\
        impl AsRef<[u8]> for Blob { fn as_ref(&self) -> &[u8] { &self.0 } }\n\
        impl std::borrow::Borrow<[u8]> for Blob { fn borrow(&self) -> &[u8] { &self.0 } }\n\
    }\n\
    pub fn keep(bytes: &[u8]) {}\n\
    pub fn save() { keep(make().as_ref()); keep(cell().borrow()); }\n";
    let report = run(lib);
    assert!(report.refusals.is_empty(), "{:?}", refusals(&report));
    assert!(report.ambiguous.is_empty(), "{:?}", ambiguous(&report));
    assert_eq!(report.sites, SiteCount::new(0));
}

#[test]
fn an_error_value_is_no_newtype() {
    let lib = format!(
        "{SHOWN}pub fn save() -> Result<(), String> {{\n\
         if let Err(error) = make() {{ store(&error.to_string()); }}\n\
         make().map_err(|e| store(&e.to_string()))\n\
         }}\n"
    );
    let report = run(&lib);
    assert!(report.refusals.is_empty(), "{:?}", refusals(&report));
    assert!(report.ambiguous.is_empty(), "{:?}", ambiguous(&report));
}

#[test]
fn std_and_foreign_values_type_their_receiver() {
    let lib = format!(
        "{SHOWN}pub fn save(text: &str) {{\n\
         store(&uuid::Uuid::new_v4().to_string());\n\
         store(&text.trim().to_string());\n\
         store(&make().unwrap_or(\"x\").to_string());\n\
         for c in text.chars() {{ store(&c.to_string()); }}\n\
         if let pulldown_cmark::Tag::Link {{ dest_url, .. }} = make() {{ store(&dest_url.to_string()); }}\n\
         }}\n"
    );
    let report = run(&lib);
    assert!(report.refusals.is_empty(), "{:?}", refusals(&report));
    assert!(report.ambiguous.is_empty(), "{:?}", ambiguous(&report));
}

#[test]
fn an_impl_into_std_or_workspace_parameter_is_no_boundary() {
    let lib = format!(
        "{NAME}use loro::LoroValue;\n\
         pub struct Intent;\n\
         impl Intent {{\n\
         pub fn publish(&self, text: impl Into<String>) {{}}\n\
         pub fn local(&self, name: impl Into<Name>) {{}}\n\
         pub fn put(&self, value: impl Into<LoroValue>) {{}}\n\
         }}\n\
         pub fn save(name: &Name, intent: &Intent) {{\n\
         intent.publish(name.as_str());\n\
         intent.local(name.as_str());\n\
         intent.put(name.as_str());\n\
         }}\n"
    );
    let report = run(&lib);
    assert_eq!(
        shapes(&report.refusals),
        ["Name::as_str -> .publish", "Name::as_str -> .local"]
    );
    assert_eq!(report.foreign, SiteCount::new(1));
}

#[test]
fn as_ref_on_a_field_of_an_untyped_base_is_still_read() {
    let lib = "pub mod blob {\n\
        pub struct Payload(Vec<u8>);\n\
        impl AsRef<[u8]> for Payload { fn as_ref(&self) -> &[u8] { &self.0 } }\n\
    }\n\
    pub struct Envelope { payload: blob::Payload }\n\
    pub fn keep(bytes: &[u8]) {}\n\
    pub fn save() { let env = make(); keep(env.payload.as_ref()); }\n";
    let report = run(lib);
    assert!(report.refusals.is_empty(), "{:?}", refusals(&report));
    assert_eq!(shapes(&report.ambiguous), ["Payload::as_ref -> keep"]);
}

#[test]
fn a_name_inside_an_err_variant_is_no_error() {
    let lib = format!(
        "{SHOWN}pub enum AppError {{ Conflict(Name) }}\n\
         pub fn save() {{\n\
         if let Err(AppError::Conflict(id)) = make() {{ store(id.as_str()); }}\n\
         }}\n"
    );
    let report = run(&lib);
    assert!(report.refusals.is_empty(), "{:?}", refusals(&report));
    assert_eq!(shapes(&report.ambiguous), ["Name::as_str -> store"]);
}

#[test]
fn a_renamed_dependency_reaches_its_package() {
    let tool = "pub struct Visit;\n\
        impl Visit { pub fn bind(&mut self, text: &str) {} }\n";
    let lib = format!("{NAME}pub fn save(name: &Name) {{ make().bind(name.as_str()); }}\n");
    let manifest = "[dependencies]\nhelper = { package = \"tool\", path = \"../tool\" }\n";
    let report = run_crates(&[("demo", manifest, &lib), ("tool", "", tool)]);
    assert_eq!(shapes(&report.ambiguous), ["Name::as_str -> .bind"]);
}

#[test]
fn a_foreign_fn_a_use_imports_shadows_a_workspace_fn() {
    let lib = format!(
        "{NAME}pub mod parse {{ pub fn from_str() -> super::Name {{ make() }} }}\n\
         use serde_json::from_str;\n\
         pub fn save() {{ store(from_str().as_str()); }}\n"
    );
    let report = run(&lib);
    assert!(report.refusals.is_empty(), "{:?}", refusals(&report));
    assert_eq!(shapes(&report.ambiguous), ["Name::as_str -> store"]);
}

#[test]
fn a_one_newtype_fn_that_returns_another_type_is_no_named_conversion() {
    let lib = format!(
        "{DELTAS}pub fn keep(n: i64) -> String {{ String::new() }}\n\
         pub struct Term(i64);\n\
         impl Term {{\n\
         pub fn label(delta: RowDelta) -> String {{ keep(delta.get()) }}\n\
         }}\n"
    );
    assert_eq!(shapes(&run(&lib).refusals), ["RowDelta::get -> keep"]);
}

#[test]
fn a_macro_body_catalogues_no_item_a_metavariable_names() {
    let lib = "macro_rules! items {\n\
        ($name:ident, $alias:ident) => {\n\
            pub enum $name {}\n\
            pub type $alias = u8;\n\
            pub fn $name() {}\n\
            pub enum Written {}\n\
        };\n\
    }\n";
    let file = syn::parse_file(lib).unwrap();
    let mut raw = Raw::default();
    for phase in [Phase::Declare, Phase::Catalog] {
        let mut visit = CatalogVisit {
            phase,
            module: vec!["crate".to_owned()],
            scope: Scope {
                krate: "demo".to_owned(),
                uses: Uses::default(),
            },
            raw: &mut raw,
        };
        for item in &file.items {
            visit.visit_item(item);
        }
    }
    let types: Vec<&String> = raw.catalog.types.iter().collect();
    assert_eq!(types, ["Written"]);
    assert!(raw.catalog.fns.0.keys().all(|name| !name.starts_with(META)));
}

#[test]
fn a_type_name_two_crates_and_a_foreign_import_share_is_judged_by_each_type() {
    let units = "pub mod px {\n\
            pub struct Px(u32);\n\
            impl Px { pub fn get(self) -> u32 { self.0 } }\n\
            pub struct Pos(usize);\n\
            impl Pos { pub fn to_usize(self) -> usize { self.0 } }\n\
        }\n\
        pub use px::{Pos, Px};\n";
    // `grid` holds a `Px` in `Layout::width`, so its `get` is an unwrap.
    let grid = "use units::Px;\n\
        pub struct Layout { pub width: Px }\n\
        pub fn keep(n: u32) {}\n\
        pub fn draw(layout: &Layout) { keep(layout.width.get()); }\n";
    // `sheet` holds no newtype in its own `Layout::width`.
    let sheet = "pub struct Wide;\n\
        impl Wide { pub fn get(self) -> u32 { 0 } }\n\
        pub struct Layout { pub width: Wide }\n\
        pub fn keep(n: u32) {}\n\
        pub fn draw(layout: &Layout) { keep(layout.width.get()); }\n";
    // `prose` names the `Layout` of a dependency: its `get` is foreign.
    let prose = "use parley::Layout;\n\
        use units::Pos;\n\
        pub struct Run { layout: Layout }\n\
        impl Run { pub fn line(&self, at: Pos) { self.layout.get(at.to_usize()); } }\n";
    let deps = "[dependencies]\nunits = \"1\"\n";
    let report = run_crates(&[
        ("grid", deps, grid),
        (
            "prose",
            "[dependencies]\nparley = \"1\"\nunits = \"1\"\n",
            prose,
        ),
        ("sheet", deps, sheet),
        ("units", "", units),
    ]);
    assert_eq!(refusals(&report), ["grid | src/lib.rs:4 | Px::get -> keep"]);
    assert!(report.ambiguous.is_empty(), "{:?}", ambiguous(&report));
    assert_eq!(report.foreign, SiteCount::new(1));
}

#[test]
fn a_re_export_resolves_through_the_pub_use_of_its_crate() {
    // `draw` re-exports the kurbo `Point`, and one of its files imports
    // the `Point` of `units` for its own use.
    let units = "pub struct Point(u32);\n\
        impl Point { pub fn new(text: &str) -> Self { Self(0) } }\n";
    let draw = "pub use kurbo::Point;\n\
        mod kinetic { use units::Point; }\n";
    let lib = format!(
        "{NAME}use draw::Point;\n\
         pub fn save(name: &Name) {{ Point::new(name.as_str()); units::Point::new(name.as_str()); }}\n"
    );
    let report = run_crates(&[
        (
            "demo",
            "[dependencies]\ndraw = \"1\"\nunits = \"1\"\n",
            &lib,
        ),
        ("draw", "[dependencies]\nunits = \"1\"\n", draw),
        ("units", "", units),
    ]);
    assert_eq!(
        shapes(&report.refusals),
        ["Name::as_str -> units::Point::new"]
    );
    assert!(report.ambiguous.is_empty(), "{:?}", ambiguous(&report));
    assert_eq!(report.foreign, SiteCount::new(1));
}
