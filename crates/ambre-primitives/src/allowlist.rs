//! The allowlist: one line per site that keeps its primitive, with the reason
//! it may.
//!
//! The file only shrinks on its own. A site the scan finds and no line names
//! fails the gate, and so does a line that names no site (`ARCH-033`,
//! `ARCH-034`). Adding a line is a hand edit, held by a reviewer (`ARCH-035`).
//! Two categories claim a shape the code shows, so the gate refutes them
//! from the site itself (`ARCH-036`). Three refusals read the privacy of the
//! site, whatever the category:
//!
//!   1. a `local` line on a site that is not private (`ARCH-038`);
//!   2. a `bool` field that is not private, outside `foreign-api` (`ARCH-039`);
//!   3. a `bool` parameter that is not private and that a caller writes as a
//!      literal, outside `trait`, whose foreign signature fixes the type
//!      (`ARCH-040`).
//!
//! One refusal reads the key alone: a `math` line on a name that ends on a
//! unit (`ARCH-084`, `units.rs`).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::records::{Claim, Privacy, Records, Role, Shape, SiteKey};

/// The header line, the first line of the file that is not a comment.
pub const HEADER: &str = "key\tcategory\tnote";

/// Why a site may keep its primitive. A closed set: a line naming another
/// word fails the gate. No word means "to type later", so a value still to
/// type has no line, and the gate fails until it is typed (`ARCH-033`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    /// Free text with no rule: a label, a message, user content.
    FreeText,
    /// The call into a foreign API that takes the primitive.
    ForeignApi,
    /// The own representation of a type: the field of a single-field
    /// struct, or the one field of a tuple variant of any enum. For a newtype
    /// or a fieldless enum, also a parameter of an associated `fn` that
    /// returns the type, or the return of its method.
    NewtypeInner,
    /// A signature that a foreign trait dictates, or a closure type that a
    /// foreign signature requires.
    Trait,
    /// A truly generic helper over the primitive: a codec, a hash, a parser.
    Generic,
    /// Arithmetic local to an algorithm, which never crosses a boundary.
    Math,
    /// A value that stays inside one function or one private struct, under
    /// the four conditions of `ai/ambre-rust-style.md`.
    Local,
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
        Self::Local,
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
            Self::Local => "local",
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct LineNo(usize);

impl LineNo {
    /// The line numbered `number`, counted from one as an editor shows it.
    pub fn new(number: usize) -> Self {
        Self(number)
    }
}

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

/// What a refused line breaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Defect {
    /// A `newtype-inner` or a `trait` line whose site has another shape
    /// (`ARCH-036`).
    Shape,
    /// A `local` line on a site that is not private (`ARCH-038`).
    LocalSite,
    /// A `bool` field that is not private, off `foreign-api` (`ARCH-039`).
    BoolField,
    /// A `bool` parameter that is not private, written as a literal by a
    /// caller (`ARCH-040`).
    BoolLiteral,
    /// A `math` line on a name whose last segment is a unit (`ARCH-084`).
    UnitName,
}

impl fmt::Display for Defect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Shape => "shape",
            Self::LocalSite => "local-site",
            Self::BoolField => "bool-field",
            Self::BoolLiteral => "bool-literal",
            Self::UnitName => "unit-name",
        })
    }
}

/// A line the gate refuses from its site.
#[derive(Debug)]
pub struct Refuted {
    pub line: LineNo,
    pub key: SiteKey,
    pub category: Category,
    pub defect: Defect,
    /// What the site is instead, from the code.
    pub reason: String,
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
            let line = LineNo::new(i + 1);
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

    /// The lines the gate refuses from their site. Only `newtype-inner` and
    /// `trait` claim a shape the code shows (`ARCH-036`); the other
    /// categories claim a meaning, which a reviewer holds (`ARCH-035`). The
    /// privacy refusals hold for every category (`ARCH-038` to `ARCH-040`),
    /// with two exceptions. A `foreign-api` line takes no `bool` field
    /// refusal. A `trait` line takes no literal refusal.
    pub fn refuted(&self, records: &Records) -> Vec<Refuted> {
        let mut found = Vec::new();
        for line in &self.lines {
            let Line::Entry {
                key,
                category,
                line,
            } = line
            else {
                continue;
            };
            // The name alone refutes it, so a stale line takes it too.
            if *category == Category::Math
                && let Some(unit) = crate::units::unit_of(key.item())
            {
                found.push(Refuted {
                    line: *line,
                    key: key.clone(),
                    category: *category,
                    defect: Defect::UnitName,
                    reason: format!(
                        "the name ends on the unit {unit}: a unit quantity is never math, it \
                         takes a type of ambre-core (ARCH-084)"
                    ),
                });
            }
            // A key the driver did not find passes: the stale check reports it.
            let Some(site) = records.sites.get(key) else {
                continue;
            };
            let shape = match category {
                Category::NewtypeInner => claim(&site.inner, records),
                Category::Trait => claim(&site.trait_claim, records),
                _ => Ok(()),
            };
            let local = match (category, site.privacy) {
                (Category::Local, Privacy::Private) => Ok(()),
                (Category::Local, privacy) => Err(format!(
                    "the site is not private, it is {privacy}: a local line names a private site \
                     only (ARCH-038)"
                )),
                _ => Ok(()),
            };
            // A field that mirrors a schema the workspace does not control
            // keeps its bool under foreign-api, converted at that boundary.
            let field = match category {
                Category::ForeignApi => Ok(()),
                _ => bool_field(key, site),
            };
            // A foreign signature fixes the type of a `trait` site, so a
            // literal its callers write cannot be typed there.
            let literal = match category {
                Category::Trait => Ok(()),
                _ => bool_literal(site, records),
            };
            let claims = [
                (Defect::Shape, shape),
                (Defect::LocalSite, local),
                (Defect::BoolField, field),
                (Defect::BoolLiteral, literal),
            ];
            found.extend(claims.into_iter().filter_map(|(defect, claim)| {
                claim.err().map(|reason| Refuted {
                    line: *line,
                    key: key.clone(),
                    category: *category,
                    defect,
                    reason,
                })
            }));
        }
        found
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

/// Whether a shape claim holds. An associated type holds when a generic
/// newtype of the workspace holds it as its one field (`ARCH-036`).
fn claim(claim: &Claim, records: &Records) -> Result<(), String> {
    match claim {
        Claim::Holds => Ok(()),
        Claim::Representation(path) if records.representations.contains(path) => Ok(()),
        Claim::Representation(path) => Err(format!(
            "no generic newtype of the workspace holds {path} as its one field"
        )),
        Claim::Fails(reason) => Err(reason.clone()),
    }
}

/// Checks that a field holding a `bool` is private (`ARCH-039`). The one
/// `bool` of a tuple variant is the value its variant names, and passes.
fn bool_field(key: &SiteKey, site: &crate::records::Site) -> Result<(), String> {
    let value = site.role == Role::VariantValue && site.shape == Shape::Bool;
    if key.kind() != "field"
        || site.shape == Shape::Other
        || site.privacy == Privacy::Private
        || value
    {
        return Ok(());
    }
    Err(format!(
        "a bool field that is {}: it is an enum, define_flag! writes one, unless the field \
         mirrors a schema the workspace does not control, under foreign-api (ARCH-039)",
        site.privacy
    ))
}

/// Checks that no call of the workspace writes a `bool` parameter that is not
/// private as a literal (`ARCH-040`). The driver resolves each call to its
/// `fn`, so a literal reaches exactly one parameter, or a trait item and the
/// impl it resolves to.
fn bool_literal(site: &crate::records::Site, records: &Records) -> Result<(), String> {
    let Some(slot) = &site.slot else {
        return Ok(());
    };
    if site.shape == Shape::Other || site.privacy == Privacy::Private {
        return Ok(());
    }
    let hits: Vec<&String> = records
        .literals
        .iter()
        .filter(|(at, _)| at == slot)
        .map(|(_, at)| at)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let Some(first) = hits.first() else {
        return Ok(());
    };
    let more = match hits.len() {
        1 => String::new(),
        n => format!(" and {} more", n - 1),
    };
    Err(format!(
        "a caller writes this bool parameter as a literal at {first}{more}, and the site is {}: \
         it is an enum (ARCH-040)",
        site.privacy
    ))
}

#[cfg(test)]
#[path = "allowlist_tests.rs"]
mod tests;
