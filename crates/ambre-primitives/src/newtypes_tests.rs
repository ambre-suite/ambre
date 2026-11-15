use std::collections::BTreeSet;

use super::{HEADER, NEWTYPES, judge, pruned};

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

// The key the driver writes for a struct of one field `F::Repr` whose instance
// is a `u32` (`ARCH-062`): refused with no line, passed with one.
const MARK: &str = "demo | src/lib.rs | crate | Mark | F::Repr";

#[test]
fn a_newtype_over_an_associated_type_takes_its_line() {
    let bare = format!("{HEADER}\n");
    let refused = judge(Some(&bare), &set(&[MARK]));
    assert_eq!(refused.len(), 1);
    assert!(refused[0].ends_with(MARK));
    let listed = format!("{HEADER}\n{MARK}\tan RTP field at its width\n");
    assert!(judge(Some(&listed), &set(&[MARK])).is_empty());
}

// The key the driver writes for `struct S<T>(T)`, which fails closed on its
// bare parameter (`ARCH-062`): refused with no line, passed with one.
const BARE: &str = "demo | src/lib.rs | crate | S | T";

#[test]
fn a_newtype_over_a_bare_parameter_takes_its_line() {
    let bare = format!("{HEADER}\n");
    let refused = judge(Some(&bare), &set(&[BARE]));
    assert_eq!(refused.len(), 1);
    assert!(refused[0].ends_with(BARE));
    let listed = format!("{HEADER}\n{BARE}\ta wrapper no family fits\n");
    assert!(judge(Some(&listed), &set(&[BARE])).is_empty());
}
