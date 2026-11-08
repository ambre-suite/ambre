//! The records the driver writes, one file per compiled unit, read back into
//! the sets the judges compare with the allowlists.
//!
//! The driver writes sites and the facts of each site, never verdicts. A
//! record is one line of fields joined by tabs, its first field naming it.
//! The judge merges the records of every unit: a site two units compile, the
//! host and the wasm build of one file, is one site.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

/// One site, written as one line: the crate, the file, the module path, the
/// item path, the kind and the type, joined by [`SiteKey::SEPARATOR`].
///
/// The key carries no line number. An edit above a site leaves its key as it
/// was, so an allowlist line follows the site, not its position.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SiteKey(String);

impl SiteKey {
    /// The separator between two fields of a key. A normalised type keeps a
    /// space only between two words, so no field can hold this text.
    pub const SEPARATOR: &'static str = " | ";

    /// The number of fields a key carries.
    const FIELDS: usize = 6;

    /// Reads a key back from an allowlist line.
    ///
    /// Returns `None` for a text that does not split into six non-empty fields.
    pub fn parse(text: &str) -> Option<Self> {
        let fields: Vec<&str> = text.split(Self::SEPARATOR).collect();
        (fields.len() == Self::FIELDS && fields.iter().all(|field| !field.trim().is_empty()))
            .then(|| Self(text.to_owned()))
    }

    /// The kind the key names, its fifth field.
    pub fn kind(&self) -> &str {
        self.0.split(Self::SEPARATOR).nth(4).unwrap_or_default()
    }
}

impl fmt::Display for SiteKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Who reaches a site, as the `local` rule and the `bool` rule read it
/// (`ai/ambre-rust-style.md`). Every variant but `Private` is a site that is
/// not private.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Privacy {
    Private,
    /// `pub(crate)`, `pub(super)` or `pub(in path)`.
    Restricted,
    Public,
    /// A signature or a constant of a trait impl.
    TraitImpl,
    /// A field of a type of `ambre-bus`, which crosses a thread.
    BusType,
    /// A field of a type that derives `Serialize`, `Deserialize` or `FromRow`.
    Serialised,
}

impl Privacy {
    fn parse(word: &str) -> Option<Self> {
        Some(match word {
            "private" => Self::Private,
            "restricted" => Self::Restricted,
            "public" => Self::Public,
            "trait-impl" => Self::TraitImpl,
            "bus" => Self::BusType,
            "serialised" => Self::Serialised,
            _ => return None,
        })
    }
}

impl fmt::Display for Privacy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Private => "private",
            Self::Restricted => "pub(crate), pub(super) or pub(in path)",
            Self::Public => "pub",
            Self::TraitImpl => "in a trait impl",
            Self::BusType => "a field of a type of ambre-bus",
            Self::Serialised => "a field of a type that derives Serialize, Deserialize or FromRow",
        })
    }
}

/// How a type holds a `bool`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Other,
    Bool,
    /// A type that wraps a `bool` outside a callable signature.
    HoldsBool,
}

/// What a field is to the type that declares it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Any field but the one below, and every other kind of site.
    Member,
    /// The one field of a tuple variant with one field. The variant names the
    /// value, as `CellValue::Boolean(bool)` does, so that `bool` selects no
    /// mode (`ARCH-039`).
    VariantValue,
}

/// What a `newtype-inner` or a `trait` line would claim of a site, as the
/// driver read it from the code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Claim {
    Holds,
    /// Holds when a generic newtype of the workspace holds this associated
    /// type, `<trait path>::<name>`, as its one field.
    Representation(String),
    Fails(String),
}

impl Claim {
    fn parse(text: &str) -> Self {
        match text {
            "ok" => Self::Holds,
            _ => match text.strip_prefix("repr:") {
                Some(path) => Self::Representation(path.to_owned()),
                None => Self::Fails(text.to_owned()),
            },
        }
    }
}

/// One site and its facts.
#[derive(Clone, Debug)]
pub struct Site {
    pub privacy: Privacy,
    pub shape: Shape,
    pub role: Role,
    pub inner: Claim,
    pub trait_claim: Claim,
    /// The parameter a call reaches, for a parameter site.
    pub slot: Option<String>,
    /// Who wrote the site: `source`, or `macro:<name>` for a macro of the
    /// workspace.
    pub origin: String,
    /// Where the site sits, `file:line` from the workspace root.
    pub at: String,
}

/// Everything the driver wrote, over every unit.
#[derive(Debug, Default)]
pub struct Records {
    pub sites: BTreeMap<SiteKey, Site>,
    /// The associated types the generic newtypes of the workspace hold.
    pub representations: BTreeSet<String>,
    /// Each `bool` literal: the parameter slot it reaches, and where.
    pub literals: Vec<(String, String)>,
    /// The key of each struct the census reads, with where it sits.
    pub newtypes: BTreeMap<String, String>,
    /// Each alias whose target is an instance of a generic struct of
    /// `ambre-core`: the struct, the alias and where it sits.
    pub aliases: BTreeSet<(String, String, String)>,
    /// The name of each generic struct of `ambre-core`.
    pub generics: BTreeSet<String>,
    /// Each unwrap site: its key, and where it sits.
    pub unwraps: BTreeSet<(String, String)>,
    /// Each refusal the code alone settles: the check, then the message.
    pub refusals: BTreeSet<(String, String)>,
    /// Each type, alias or trait the protocol, the worker or the relay
    /// declares: the package, the name, `public` or `private`, and where.
    pub decls: BTreeSet<(String, String, String, String)>,
    /// Each file a compile read.
    pub files: BTreeSet<PathBuf>,
    /// Each record the judge could not read.
    pub errors: Vec<String>,
}

impl Records {
    /// Reads every record file in `dir`.
    pub fn read(dir: &Path) -> Self {
        let mut records = Self::default();
        let Ok(entries) = std::fs::read_dir(dir) else {
            records
                .errors
                .push(format!("{}: no record of the driver", dir.display()));
            return records;
        };
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
            match std::fs::read_to_string(&path) {
                Ok(text) => records.add(&text),
                Err(e) => records.errors.push(format!("{}: {e}", path.display())),
            }
        }
        records
    }

    /// Adds the records of one unit.
    pub fn add(&mut self, text: &str) {
        for line in text.lines() {
            let fields: Vec<&str> = line.split('\t').collect();
            match fields.as_slice() {
                ["unit", ..] => {}
                [
                    "site",
                    key,
                    privacy,
                    shape,
                    role,
                    inner,
                    trait_claim,
                    slot,
                    origin,
                    at,
                ] => {
                    let Some(key) = SiteKey::parse(key) else {
                        self.errors
                            .push(format!("a site key of other than six fields: {key}"));
                        continue;
                    };
                    let Some(privacy) = Privacy::parse(privacy) else {
                        self.errors.push(format!("{key}: no privacy {privacy:?}"));
                        continue;
                    };
                    let shape = match *shape {
                        "bool" => Shape::Bool,
                        "holds-bool" => Shape::HoldsBool,
                        _ => Shape::Other,
                    };
                    self.sites.entry(key).or_insert(Site {
                        privacy,
                        shape,
                        role: if *role == "variant-value" {
                            Role::VariantValue
                        } else {
                            Role::Member
                        },
                        inner: Claim::parse(inner),
                        trait_claim: Claim::parse(trait_claim),
                        slot: (*slot != "-").then(|| (*slot).to_owned()),
                        origin: (*origin).to_owned(),
                        at: (*at).to_owned(),
                    });
                }
                ["repr", path] => {
                    self.representations.insert((*path).to_owned());
                }
                ["lit", slot, at] => self.literals.push(((*slot).to_owned(), (*at).to_owned())),
                ["newtype", key, at] => {
                    self.newtypes.insert((*key).to_owned(), (*at).to_owned());
                }
                ["alias", family, name, at] => {
                    self.aliases.insert((
                        (*family).to_owned(),
                        (*name).to_owned(),
                        (*at).to_owned(),
                    ));
                }
                ["generic", name] => {
                    self.generics.insert((*name).to_owned());
                }
                ["unwrap", key, at] => {
                    self.unwraps.insert(((*key).to_owned(), (*at).to_owned()));
                }
                ["refuse", check, message] => {
                    self.refusals
                        .insert(((*check).to_owned(), (*message).to_owned()));
                }
                ["decl", krate, name, public, at] => {
                    self.decls.insert((
                        (*krate).to_owned(),
                        (*name).to_owned(),
                        (*public).to_owned(),
                        (*at).to_owned(),
                    ));
                }
                ["file", path] => {
                    self.files.insert(PathBuf::from(path));
                }
                _ => self
                    .errors
                    .push(format!("a record the judge cannot read: {line}")),
            }
        }
    }

    /// The keys of every site.
    pub fn keys(&self) -> BTreeSet<SiteKey> {
        self.sites.keys().cloned().collect()
    }
}

#[cfg(test)]
#[path = "records_tests.rs"]
mod tests;
