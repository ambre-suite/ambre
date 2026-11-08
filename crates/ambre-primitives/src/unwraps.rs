//! The judge of the unwrap check (`ARCH-073`): the sites the driver reads,
//! against `specs/1-Foundations/ambre-unwrap.tsv`.
//!
//! The driver reads each body with its typeck results, so every receiver has
//! its type and every callee its definition. A site is refused or not: no
//! site is ambiguous. A line of the file excuses one key with a count of
//! sites and a reason. The report fails on a missing or malformed file. A
//! strict report also fails on a key no line names, on a key with more sites
//! than its count, and on a stale line.

use std::collections::BTreeMap;
use std::fmt;

use crate::allowlist::LineNo;

/// The unwrap allowlist, relative to the workspace root.
pub const UNWRAP_ALLOWLIST: &str = "specs/1-Foundations/ambre-unwrap.tsv";

/// The header line, the first line of the file that is not a comment.
pub const HEADER: &str = "key\tcount\treason";

/// A number of sites the report counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct SiteCount(usize);

impl SiteCount {
    /// A count of `n` sites.
    pub const fn new(n: usize) -> Self {
        Self(n)
    }

    /// The count one site later.
    const fn incremented(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

impl fmt::Display for SiteCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// One line of the allowlist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub line: LineNo,
    pub count: SiteCount,
}

/// Whether the report fails on a refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strictness {
    /// Print the refusals, and fail on an allowlist that does not parse.
    Report,
    /// Fail on any refusal or line the allowlist and the report disagree on.
    Strict,
}

/// How a run of the report ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Pass,
    Fail,
}

/// What the allowlist says of one report.
#[derive(Debug)]
pub struct Judgement {
    pub outcome: Outcome,
    /// The refusal keys no line names.
    pub unlisted: SiteCount,
    /// The lines whose key has more sites than the count allows.
    pub over: SiteCount,
    /// The lines whose key has fewer sites than the count, none included.
    pub stale: SiteCount,
    /// One message per key no line names, and per line over or stale.
    pub problems: Vec<String>,
}

/// Parses the allowlist.
///
/// # Errors
///
/// One message per line that breaks the format.
///
/// * a missing header, or a line of other than three fields;
/// * a count that is no whole number above 0, or an empty reason;
/// * a key named twice.
pub fn parse_allowlist(text: &str) -> Result<BTreeMap<String, Row>, Vec<String>> {
    let mut rows = BTreeMap::new();
    let mut errors = Vec::new();
    let mut header = false;
    for (index, line) in text.lines().enumerate() {
        let number = LineNo::new(index + 1);
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        if !header {
            if line != HEADER {
                errors.push(format!("line {number}: the header is {HEADER:?}"));
            }
            header = true;
            continue;
        }
        let mut fields = line.splitn(3, '\t');
        let (Some(key), Some(count), Some(reason)) = (fields.next(), fields.next(), fields.next())
        else {
            errors.push(format!(
                "line {number}: a key, a tab, a count, a tab, then the reason"
            ));
            continue;
        };
        let count = match count.parse::<usize>() {
            Ok(count) if count > 0 => SiteCount::new(count),
            _ => {
                errors.push(format!(
                    "line {number}: the count is a whole number above 0"
                ));
                continue;
            }
        };
        if reason.trim().is_empty() {
            errors.push(format!("line {number}: the reason is empty"));
        }
        let row = Row {
            line: number,
            count,
        };
        if rows.insert(key.to_owned(), row).is_some() {
            errors.push(format!("line {number}: the key is named twice"));
        }
    }
    if !header {
        errors.push(format!("the header {HEADER:?} is missing"));
    }
    if errors.is_empty() {
        Ok(rows)
    } else {
        Err(errors)
    }
}

/// Compares the refusal keys with the allowlist text, `None` when the file is
/// missing. Each key comes once per site.
///
/// # Errors
///
/// A missing or malformed allowlist, in both modes; each message names one
/// defect.
pub fn judge<'k>(
    keys: impl IntoIterator<Item = &'k str>,
    allowlist: Option<&str>,
    strictness: Strictness,
) -> Result<Judgement, Vec<String>> {
    let Some(text) = allowlist else {
        return Err(vec!["cannot read it".to_owned()]);
    };
    let rows = parse_allowlist(text)?;
    let mut sites: BTreeMap<&str, SiteCount> = BTreeMap::new();
    for key in keys {
        let count = sites.entry(key).or_default();
        *count = count.incremented();
    }
    let mut judgement = Judgement {
        outcome: Outcome::Pass,
        unlisted: SiteCount::default(),
        over: SiteCount::default(),
        stale: SiteCount::default(),
        problems: Vec::new(),
    };
    for (key, found) in &sites {
        match rows.get(*key) {
            // The message names the key, so the line can be written from it.
            None => {
                judgement.unlisted = judgement.unlisted.incremented();
                judgement.problems.push(format!(
                    "no line names the key, the report finds {found}\n  {key}"
                ));
            }
            Some(row) if *found > row.count => {
                judgement.over = judgement.over.incremented();
                judgement.problems.push(format!(
                    "line {} allows {} sites, the report finds {found}\n  {key}",
                    row.line, row.count
                ));
            }
            Some(_) => {}
        }
    }
    for (key, row) in &rows {
        let found = sites.get(key.as_str()).copied().unwrap_or_default();
        if found < row.count {
            judgement.stale = judgement.stale.incremented();
            judgement.problems.push(format!(
                "line {} allows {} sites, the report finds {found}\n  {key}",
                row.line, row.count
            ));
        }
    }
    let clean = judgement.unlisted == SiteCount::default()
        && judgement.over == SiteCount::default()
        && judgement.stale == SiteCount::default();
    if strictness == Strictness::Strict && !clean {
        judgement.outcome = Outcome::Fail;
    }
    Ok(judgement)
}

#[cfg(test)]
#[path = "unwraps_tests.rs"]
mod tests;
