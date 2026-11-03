use std::collections::BTreeSet;

use super::{HEADER, NEWTYPES, census, judge, pruned};

/// The census of one file of the crate `demo`, at `src/lib.rs`.
fn found(source: &str) -> Vec<String> {
    let tree = match syn::parse_file(source) {
        Ok(tree) => tree,
        Err(e) => panic!("the test source does not parse: {e}"),
    };
    census("demo", "src/lib.rs", "crate", &tree)
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

/// A generic struct is a family, and its instances write no struct. A
/// `PhantomData` marker is no field.
#[test]
fn a_family_is_not_counted_and_a_marker_is_no_field() {
    assert_eq!(
        found(
            "pub struct Id<E>(u128, PhantomData<fn() -> E>);\n\
             pub struct Fixed<const N: usize>(u32);\n\
             pub struct Tagged(u32, PhantomData<()>);"
        ),
        [format!("{LIB} | Tagged | u32")]
    );
}

/// Two fields, no field, a typed field, a `bool`, a vector of another
/// element and an array are each out of the census.
#[test]
fn any_other_shape_is_not_counted() {
    assert!(
        found(
            "struct Pair(u32, u32);\n\
             struct Unit;\n\
             struct Empty {}\n\
             struct Typed(Port);\n\
             struct Flag(bool);\n\
             struct Words(Vec<u32>);\n\
             struct Key([u8; 32]);\n\
             struct Text(Box<str>);"
        )
        .is_empty()
    );
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
