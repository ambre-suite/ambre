use std::collections::BTreeSet;

use super::{HEADER, NEWTYPES, census, judge, pruned};

/// The census of one file of the crate `demo`, at `src/lib.rs`.
fn found(source: &str) -> Vec<String> {
    found_in("demo", source)
}

/// The census of one file of the crate `krate`, at `src/lib.rs`.
fn found_in(krate: &str, source: &str) -> Vec<String> {
    let tree = match syn::parse_file(source) {
        Ok(tree) => tree,
        Err(e) => panic!("the test source does not parse: {e}"),
    };
    census(krate, "src/lib.rs", "crate", &tree)
}

const LIB: &str = "demo | src/lib.rs | crate";

#[test]
fn a_single_field_struct_over_a_primitive_is_counted() {
    assert_eq!(
        found(
            "pub struct Port(u16);\n\
             pub struct Name { text: String }\n\
             pub struct Blob(Vec<u8>);\n\
             pub struct Ratio(f64);\n\
             struct Wide(std::primitive::u64);"
        ),
        [
            format!("{LIB} | Port | u16"),
            format!("{LIB} | Name | String"),
            format!("{LIB} | Blob | Vec<u8>"),
            format!("{LIB} | Ratio | f64"),
            format!("{LIB} | Wide | std::primitive::u64"),
        ]
    );
}

/// A generic struct of `ambre-core` is a family, and its instances write no
/// struct. A lifetime parameter makes no family. A `PhantomData` marker is no
/// field.
#[test]
fn a_family_is_not_counted_and_a_marker_is_no_field() {
    assert_eq!(
        found_in(
            "ambre-core",
            "pub struct Id<E>(u128, PhantomData<fn() -> E>);\n\
             pub struct Fixed<const N: usize>(u32);\n\
             pub struct View<'a>(&'a str);\n\
             pub struct Tagged(u32, PhantomData<()>);"
        ),
        [
            "ambre-core | src/lib.rs | crate | View | &'a str".to_owned(),
            "ambre-core | src/lib.rs | crate | Tagged | u32".to_owned(),
        ]
    );
}

/// A generic struct outside `ambre-core` is a family no line of the spec
/// declares, so the census reads it as a newtype.
#[test]
fn a_generic_struct_outside_core_is_counted() {
    assert_eq!(
        found(
            "pub struct Url<K: UrlKind> { text: String, kind: PhantomData<fn() -> K> }\n\
             pub struct Fixed<const N: usize>(u32);\n\
             pub struct Wrap<T>(T);"
        ),
        [
            format!("{LIB} | Url | String"),
            format!("{LIB} | Fixed | u32"),
        ]
    );
}

/// The census reads each primitive of the wider list. That holds for a
/// `char`, a `bool`, a reference to text or to bytes, and a `Uuid`. It holds
/// for a `Cow` of text or bytes and for an `Option` of a primitive too.
#[test]
fn a_wrapper_of_each_other_primitive_is_counted() {
    assert_eq!(
        found(
            "pub struct KeyChar(char);\n\
             struct Latch(bool);\n\
             pub struct Name(&'static str);\n\
             struct View<'a>(&'a [u8]);\n\
             pub struct Instance(uuid::Uuid);\n\
             pub struct Core(Uuid);\n\
             pub struct Cursor(Cow<'static, str>);\n\
             pub struct Bytes<'a>(std::borrow::Cow<'a, [u8]>);\n\
             pub struct Seq(Option<i64>);\n\
             pub struct Maybe(Option<Option<Vec<u8>>>);"
        ),
        [
            format!("{LIB} | KeyChar | char"),
            format!("{LIB} | Latch | bool"),
            format!("{LIB} | Name | &'static str"),
            format!("{LIB} | View | &'a[u8]"),
            format!("{LIB} | Instance | uuid::Uuid"),
            format!("{LIB} | Core | Uuid"),
            format!("{LIB} | Cursor | Cow<'static,str>"),
            format!("{LIB} | Bytes | std::borrow::Cow<'a,[u8]>"),
            format!("{LIB} | Seq | Option<i64>"),
            format!("{LIB} | Maybe | Option<Option<Vec<u8>>>"),
        ]
    );
}

/// Each shape below is out of the census:
///
/// * two fields, no field, or a typed field;
/// * a vector of another element, an array or a box;
/// * a `Cow` or an `Option` of a typed value;
/// * a reference to a slice of another element.
#[test]
fn any_other_shape_is_not_counted() {
    assert!(
        found(
            "struct Pair(u32, u32);\n\
             struct Unit;\n\
             struct Empty {}\n\
             struct Typed(Port);\n\
             struct Words(Vec<u32>);\n\
             struct Key([u8; 32]);\n\
             struct Text(Box<str>);\n\
             struct Path<'a>(Cow<'a, std::path::Path>);\n\
             struct Maybe(Option<Port>);\n\
             struct Many<'a>(&'a [u32]);\n\
             struct Rows<'a>(&'a [&'a str]);"
        )
        .is_empty()
    );
}

/// Each shape the census reads beyond the first four primitives fails
/// without a line, and passes with one.
#[test]
fn a_wider_shape_is_refused_without_a_line_and_passes_with_one() {
    let keys: BTreeSet<String> = found(
        "pub struct KeyChar(char);\n\
         pub struct Cursor(Cow<'static, str>);\n\
         pub struct Seq(Option<i64>);\n\
         pub struct Url<K: UrlKind> { text: String, kind: PhantomData<fn() -> K> }",
    )
    .into_iter()
    .collect();
    assert_eq!(keys.len(), 4);
    let refused = judge(Some(&format!("{HEADER}\n")), &keys);
    assert_eq!(refused.len(), 4);
    assert!(refused.iter().all(|refusal| refusal.contains("no line")));
    let listed: String = keys
        .iter()
        .map(|key| format!("{key}\ta reason\n"))
        .collect();
    assert!(judge(Some(&format!("{HEADER}\n{listed}")), &keys).is_empty());
}

#[test]
fn the_key_names_the_inline_module_and_the_enclosing_fn() {
    assert_eq!(
        found(
            "mod inner { pub struct Seq(u64); }\n\
             fn build() { struct Local(i32); }\n\
             impl Port { fn make() { struct Inside(u8); } }"
        ),
        [
            "demo | src/lib.rs | crate::inner | Seq | u64".to_owned(),
            format!("{LIB} | build::Local | i32"),
            format!("{LIB} | make::Inside | u8"),
        ]
    );
}

#[test]
fn test_code_is_not_counted() {
    assert!(
        found(
            "#[cfg(test)] struct Fake(u32);\n\
             #[cfg(test)] mod tests { struct Probe(u32); }\n\
             #[test] fn probe() { struct Probe(u32); }"
        )
        .is_empty()
    );
}

fn set(keys: &[&str]) -> BTreeSet<String> {
    keys.iter().map(|key| (*key).to_owned()).collect()
}

const PORT: &str = "demo | src/lib.rs | crate | Port | u16";

#[test]
fn a_listed_newtype_with_a_reason_passes() {
    let text = format!("# comment\n{HEADER}\n{PORT}\tan operating-system port\n");
    assert!(judge(Some(&text), &set(&[PORT])).is_empty());
}

#[test]
fn a_newtype_no_line_names_is_refused() {
    let text = format!("{HEADER}\n");
    let refused = judge(Some(&text), &set(&[PORT]));
    assert_eq!(refused.len(), 1);
    assert!(refused[0].contains("no line") && refused[0].ends_with(PORT));
}

#[test]
fn a_line_that_names_no_newtype_is_stale() {
    let text = format!("{HEADER}\n{PORT}\ta port\n");
    let refused = judge(Some(&text), &set(&[]));
    assert_eq!(refused.len(), 1);
    assert!(refused[0].contains(":2 names no hand-written newtype"));
}

#[test]
fn a_line_that_breaks_the_format_is_refused() {
    let text = format!(
        "{HEADER}\n{PORT}\t \n{PORT}\ta port\n{PORT}\ta port again\ndemo | Port\tshort key\nno tab\n"
    );
    let refused = judge(Some(&text), &set(&[PORT]));
    assert_eq!(
        refused
            .iter()
            .map(|refusal| refusal.split_once(": ").map_or("", |(_, rest)| rest))
            .collect::<Vec<_>>(),
        [
            "the reason is empty -- say why no generic family fits",
            &format!("a second line for the key {PORT}"),
            "the key does not split into five fields on \" | \"",
            "1 fields, want 2: key, reason",
        ]
    );
}

#[test]
fn a_missing_header_or_file_is_refused() {
    assert_eq!(
        judge(Some(""), &set(&[])),
        [format!(
            "{NEWTYPES}: the file has no header line {HEADER:?}"
        )]
    );
    // A first line that is no header is refused, and names no newtype.
    let refused = judge(Some(&format!("{PORT}\ta port\n")), &set(&[]));
    assert_eq!(
        refused,
        [format!("{NEWTYPES}:1: the header must read {HEADER:?}")]
    );
    assert_eq!(judge(None, &set(&[])), [format!("cannot read {NEWTYPES}")]);
}

#[test]
fn a_prune_removes_the_stale_lines_and_nothing_else() {
    let other = "demo | src/lib.rs | crate | Gone | u8";
    let text = format!("# comment\n{HEADER}\n{PORT}\ta port\n{other}\tgone\n");
    assert_eq!(
        pruned(&text, &set(&[PORT])),
        format!("# comment\n{HEADER}\n{PORT}\ta port\n")
    );
}
