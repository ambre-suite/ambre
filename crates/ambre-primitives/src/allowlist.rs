//! The allowlist: one line per site that keeps its primitive, with the reason
//! it may.
//!
//! The file only shrinks on its own. A site the scan finds and no line names
//! fails the gate, and so does a line that names no site (`ARCH-033`,
//! `ARCH-034`). Adding a line is a hand edit, held by a reviewer (`ARCH-035`).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::scan::SiteKey;

/// The header line, the first line of the file that is not a comment.
pub const HEADER: &str = "key\tcategory\tnote";

/// Why a site may keep its primitive. A closed set: a line naming another
/// word fails the gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    /// Free text with no rule: a label, a message, user content.
    FreeText,
    /// The call into a foreign API that takes the primitive.
    ForeignApi,
    /// The inside of a newtype, or its own constructor or accessor.
    NewtypeInner,
    /// A signature that a foreign trait dictates.
    Trait,
    /// A truly generic helper over the primitive: a codec, a hash, a parser.
    Generic,
    /// Arithmetic local to an algorithm, which never crosses a boundary.
    Math,
    /// A domain value that is not typed yet. The count of these lines is the
    /// debt the gate prints.
    Debt,
}

impl Category {
    /// Every category, in the order the gate prints the counts.
    pub const ALL: [Self; 7] = [
        Self::FreeText,
        Self::ForeignApi,
        Self::NewtypeInner,
        Self::Trait,
        Self::Generic,
        Self::Math,
        Self::Debt,
    ];

    /// The word the allowlist writes.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FreeText => "free-text",
            Self::ForeignApi => "foreign-api",
            Self::NewtypeInner => "newtype-inner",
            Self::Trait => "trait",
            Self::Generic => "generic",
            Self::Math => "math",
            Self::Debt => "debt",
        }
    }

    /// Reads a category back. Returns `None` for a word outside the set.
    pub fn parse(word: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|category| category.as_str() == word)
    }

    /// The whole set, as the error messages list it.
    pub fn list() -> String {
        Self::ALL.map(Self::as_str).join(", ")
    }
}

impl fmt::Display for Category {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One line of the file. A comment and the header are kept as they are, so a
/// prune rewrites only the lines it removes.
#[derive(Debug)]
enum Line {
    Text(String),
    Entry {
        key: SiteKey,
        category: Category,
        line: LineNo,
    },
}

/// The 1-based number of a line in the file, for the error messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineNo(usize);

impl fmt::Display for LineNo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The parsed file: every line, in order.
#[derive(Debug)]
pub struct Allowlist {
    lines: Vec<Line>,
    /// The raw text of each entry line, keyed by its key, so a prune writes
    /// the kept lines byte for byte.
    raw: BTreeMap<SiteKey, String>,
}

/// What the gate found, comparing the scan with the file.
#[derive(Debug, Default)]
pub struct Verdict {
    /// The sites no line names.
    pub missing: Vec<SiteKey>,
    /// The lines that name no site, with their line numbers.
    pub stale: Vec<(LineNo, SiteKey)>,
    /// The lines per category, stale ones included.
    pub counts: BTreeMap<Category, usize>,
}

impl Verdict {
    /// Whether the gate passes.
    pub fn is_clean(&self) -> bool {
        self.missing.is_empty() && self.stale.is_empty()
    }
}

impl Allowlist {
    /// Parses the file.
    ///
    /// # Errors
    ///
    /// Every line that breaks the format, with its number. The format breaks
    /// on six defects:
    ///
    /// * a missing header;
    /// * a line of other than three fields;
    /// * a key that does not split into six fields;
    /// * a category outside the closed set;
    /// * an empty note;
    /// * a key that two lines carry.
    pub fn parse(text: &str) -> Result<Self, Vec<String>> {
        let mut lines = Vec::new();
        let mut raw = BTreeMap::new();
        let mut errors = Vec::new();
        let mut header = false;
        for (i, text) in text.lines().enumerate() {
            let line = LineNo(i + 1);
            if text.starts_with('#') || text.trim().is_empty() {
                lines.push(Line::Text(text.to_owned()));
                continue;
            }
            if !header {
                if text != HEADER {
                    errors.push(format!("line {line}: the header must read {HEADER:?}"));
                }
                header = true;
                lines.push(Line::Text(text.to_owned()));
                continue;
            }
            let fields: Vec<&str> = text.split('\t').collect();
            let [key, category, note] = fields.as_slice() else {
                errors.push(format!(
                    "line {line}: {} fields, want 3: key, category, note",
                    fields.len()
                ));
                continue;
            };
            let Some(key) = SiteKey::parse(key) else {
                errors.push(format!(
                    "line {line}: the key does not split into six fields on {:?}",
                    SiteKey::SEPARATOR
                ));
                continue;
            };
            let Some(category) = Category::parse(category) else {
                errors.push(format!(
                    "line {line}: the category {category:?} is outside the closed set: {}",
                    Category::list()
                ));
                continue;
            };
            if note.trim().is_empty() {
                errors.push(format!(
                    "line {line}: the note is empty -- say why the site keeps its primitive"
                ));
                continue;
            }
            if raw.insert(key.clone(), text.to_owned()).is_some() {
                errors.push(format!("line {line}: a second line for the key {key}"));
                continue;
            }
            lines.push(Line::Entry {
                key,
                category,
                line,
            });
        }
        if !header {
            errors.push(format!("the file has no header line {HEADER:?}"));
        }
        if errors.is_empty() {
            Ok(Self { lines, raw })
        } else {
            Err(errors)
        }
    }

    /// Compares the file with the sites the scan found.
    pub fn compare(&self, sites: &BTreeSet<SiteKey>) -> Verdict {
        let mut verdict = Verdict::default();
        for line in &self.lines {
            if let Line::Entry {
                key,
                category,
                line,
            } = line
            {
                *verdict.counts.entry(*category).or_insert(0) += 1;
                if !sites.contains(key) {
                    verdict.stale.push((*line, key.clone()));
                }
            }
        }
        verdict.missing = sites
            .iter()
            .filter(|key| !self.raw.contains_key(*key))
            .cloned()
            .collect();
        verdict
    }

    /// The file without the lines that name no site. Every other line is
    /// written as it was read. No mode adds a line: adding one is a hand
    /// edit, on purpose.
    pub fn pruned(&self, sites: &BTreeSet<SiteKey>) -> String {
        let mut out = String::new();
        for line in &self.lines {
            match line {
                Line::Text(text) => out.push_str(text),
                Line::Entry { key, .. } if sites.contains(key) => {
                    out.push_str(self.raw.get(key).map_or("", String::as_str));
                }
                Line::Entry { .. } => continue,
            }
            out.push('\n');
        }
        out
    }
}

#[cfg(test)]
#[path = "allowlist_tests.rs"]
mod tests;
