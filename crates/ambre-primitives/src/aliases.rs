//! The bare-alias check of the gate (`ARCH-066`, `ambre-spec-newtypes.md`
//! section 3).
//!
//! A type alias hides the family it names, so no alias stands for a family
//! instance alone. The driver reads every module-level `type` alias of the
//! workspace, test code included, with its target resolved. Each path, each
//! reference and each alias is set apart. It records the alias whose target
//! is then a generic struct of `ambre-core`. This judge refuses it when that
//! struct is a family of the spec.
//!
//! The set of families is the `Family` column of section 2 of the spec. The
//! check refuses a name of that column that `ambre-core` declares as no
//! generic struct, so the table and the source cannot drift apart. The source
//! alone cannot give the set: `Secret<T>` and `Issuer<K>` are generic and are
//! no family.
//!
//! A compound target holds a family in a tuple, a container or a closure. Such
//! a target is out of reach of the check: `clippy::type_complexity` decides
//! whether its alias stays, and a reviewer holds the decision (`ARCH-067`).

use std::collections::BTreeSet;

use crate::records::Records;

/// The spec whose section 2 names the families, relative to the workspace
/// root.
pub const SPEC: &str = "specs/1-Foundations/ambre-spec-newtypes.md";

/// The heading of the families table.
const FAMILIES: &str = "## 2. THE FAMILIES";

/// Every refusal of the check, from the spec text and the records.
pub fn judge(spec: Option<&str>, records: &Records) -> Vec<String> {
    let Some(spec) = spec else {
        return vec![format!("cannot read {SPEC}")];
    };
    let families = families(spec);
    let mut problems = Vec::new();
    if families.is_empty() {
        problems.push(format!(
            "{SPEC}: the table under {FAMILIES:?} names no family"
        ));
    }
    for family in &families {
        if !records.generics.contains(family) {
            problems.push(format!(
                "{SPEC}: the families table names {family}, and ambre-core declares no generic \
                 struct of that name. Correct the table"
            ));
        }
    }
    for (family, alias, at) in &records.aliases {
        if families.contains(family) {
            problems.push(format!(
                "{at}: the alias {alias} stands for the family {family} alone. Remove the alias \
                 and write the family in full at each use (ambre-spec-newtypes.md section 3)"
            ));
        }
    }
    problems
}

/// The family names of the table under [`FAMILIES`]: each name a backtick
/// opens in its second column, before its generic arguments.
pub fn families(spec: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut inside = false;
    for line in spec.lines() {
        if line.starts_with("## ") {
            inside = line.trim_end() == FAMILIES;
            continue;
        }
        if !inside || !line.starts_with('|') {
            continue;
        }
        let Some(column) = line.split('|').nth(2) else {
            continue;
        };
        for (i, quoted) in column.split('`').enumerate() {
            // The odd pieces sit between two backticks.
            if i % 2 == 0 {
                continue;
            }
            if let Some((name, _)) = quoted.split_once('<')
                && !name.is_empty()
                && name.chars().all(|c| c.is_alphanumeric() || c == '_')
            {
                names.insert(name.to_owned());
            }
        }
    }
    names
}

#[cfg(test)]
#[path = "aliases_tests.rs"]
mod tests;
