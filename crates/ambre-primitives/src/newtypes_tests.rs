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
