//! The newtype census of the gate (`ARCH-062`).
//!
//! A newtype of the workspace is an instance of a generic family of
//! `ambre-core`, or a hand-written exception with a reason
//! (`ambre-spec-newtypes.md`). The driver reads every struct outside test code
//! that:
//!
//!   * holds exactly one field once each `PhantomData` marker is set apart;
//!   * holds that field as a primitive of the census list, its aliases
//!     resolved:
//!     * an integer, a float, a `bool`, a `char`, a `String`, a `Uuid` or a
//!       `Vec<u8>`;
//!     * a `str` or a `[u8]`, bare or behind a reference;
//!     * a `Cow` of a `str` or of a `[u8]`;
//!     * an `Option` of any of these;
//!   * declares no type or const parameter, or lies outside `ambre-core`.
//!     A lifetime parameter makes no family.
//!
//! A struct a macro of the workspace writes counts; one a foreign macro
//! writes takes its fields from that crate, and does not. Each such struct
//! needs one line of the exceptions file, with the reason it is no instance
//! of a family. A struct no line names fails, and so does a line that names
//! no struct: the file only shrinks on its own.

use std::collections::{BTreeMap, BTreeSet};

use crate::allowlist::LineNo;

/// The exceptions file, relative to the workspace root.
pub const NEWTYPES: &str = "specs/1-Foundations/ambre-newtypes.tsv";

/// The header line, the first line of the file that is not a comment.
pub const HEADER: &str = "key\treason";

/// The separator between two fields of a key, as in the primitive allowlist.
const SEPARATOR: &str = " | ";

/// The number of fields a key carries: crate, file, module, struct, type.
const FIELDS: usize = 5;

/// The defects of the exceptions file against the census. A line may break
/// the format, a newtype may have no line, and a line may name no newtype.
/// `text` is `None` when the file cannot be read.
pub fn judge(text: Option<&str>, found: &BTreeSet<String>) -> Vec<String> {
    let Some(text) = text else {
        return vec![format!("cannot read {NEWTYPES}")];
    };
    let (listed, mut problems) = parse(text);
    for key in found.difference(&listed.keys().cloned().collect()) {
        problems.push(format!(
            "a hand-written newtype no line of {NEWTYPES} names. Make it an instance of a \
             generic family of ambre-core, or add the line by hand with the reason it is none \
             (ambre-spec-newtypes.md)\n  {key}"
        ));
    }
    for (key, line) in &listed {
        if !found.contains(key) {
            problems.push(format!(
                "{NEWTYPES}:{line} names no hand-written newtype. Remove it with `--prune`\n  {key}"
            ));
        }
    }
    problems
}

/// The file with each line that names no newtype removed, and nothing else.
pub fn pruned(text: &str, found: &BTreeSet<String>) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let stale = line
            .split_once('\t')
            .is_some_and(|(key, _)| key.split(SEPARATOR).count() == FIELDS && !found.contains(key));
        if !stale {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// The lines of the file by key, with their number, and the defects of its
/// format. The format breaks on five defects:
///
/// * a missing header;
/// * a line of other than two fields;
/// * a key that does not split into five fields;
/// * an empty reason;
/// * a key that two lines carry.
fn parse(text: &str) -> (BTreeMap<String, LineNo>, Vec<String>) {
    let mut listed = BTreeMap::new();
    let mut problems = Vec::new();
    let mut header = false;
    for (i, line) in text.lines().enumerate() {
        let number = LineNo::new(i + 1);
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        if !header {
            if line != HEADER {
                problems.push(format!(
                    "{NEWTYPES}:{number}: the header must read {HEADER:?}"
                ));
            }
            header = true;
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        let [key, reason] = fields.as_slice() else {
            problems.push(format!(
                "{NEWTYPES}:{number}: {} fields, want 2: key, reason",
                fields.len()
            ));
            continue;
        };
        let parts: Vec<&str> = key.split(SEPARATOR).collect();
        if parts.len() != FIELDS || parts.iter().any(|part| part.trim().is_empty()) {
            problems.push(format!(
                "{NEWTYPES}:{number}: the key does not split into five fields on {SEPARATOR:?}"
            ));
            continue;
        }
        if reason.trim().is_empty() {
            problems.push(format!(
                "{NEWTYPES}:{number}: the reason is empty -- say why no generic family fits"
            ));
            continue;
        }
        if listed.insert((*key).to_owned(), number).is_some() {
            problems.push(format!(
                "{NEWTYPES}:{number}: a second line for the key {key}"
            ));
        }
    }
    if !header {
        problems.push(format!(
            "{NEWTYPES}: the file has no header line {HEADER:?}"
        ));
    }
    (listed, problems)
}

#[cfg(test)]
#[path = "newtypes_tests.rs"]
mod tests;
