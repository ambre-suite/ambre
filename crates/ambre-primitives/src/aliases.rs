//! The bare-alias check of the gate (`ambre-spec-newtypes.md` section 3).
//!
//! A type alias hides the family it names, so no alias stands for a family
//! instance alone. The check reads every module-level `type` alias of the
//! workspace, test code included. It refuses one whose target is a family of
//! `ambre-core` once references, parentheses and other aliases are set apart.
//! A compound target holds a family in a tuple, a container or a closure.
//! Such a target is out of reach of the check: `clippy::type_complexity`
//! decides whether its alias stays, and a reviewer holds the decision.
//!
//! The set of families is the `Family` column of section 2 of the spec. The
//! check refuses a name of that column that `ambre-core` declares as no
//! generic struct, so the table and the source cannot drift apart. The
//! source alone cannot give the set: `Redacted<T>` bounds no marker, while
//! `Secret<T>` and `Issuer<K>` are generic and are no family.
//!
//! A name resolves by the `use` items and the items of its file, then by
//! the aliases and the imports of its crate. A glob can bring in a name of a
//! family. The check then counts it as that family, unless its crate declares
//! an item of that name: the check fails closed. An alias a macro writes,
//! and an alias in a `fn` body, are out of its reach.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use quote::ToTokens;

use crate::scan::{ALIAS_DEPTH, Hops, Tree, normalise};

/// The spec whose section 2 names the families, relative to the workspace
/// root.
pub const SPEC: &str = "specs/1-Foundations/ambre-spec-newtypes.md";

/// The directories the check walks, relative to the workspace root.
const WALKED: &[&str] = &["crates", "fuzz"];

/// The heading of the families table.
const FAMILIES: &str = "## 2. THE FAMILIES";

/// The crate that declares the families, as a directory and as a path root.
const CORE: &str = "ambre-core";
const CORE_ROOT: &str = "ambre_core";

/// The first segments of a path that start in the crate that writes it.
const LOCAL_ROOTS: &[&str] = &["crate", "self", "super"];

/// One file the check reads: the crate directory name, the path a message
/// shows, and the text.
pub struct Source {
    pub krate: String,
    pub shown: String,
    pub text: String,
}

/// Every refusal of the check over the workspace under `root`.
pub fn check(tree: &impl Tree, root: &Path) -> Vec<String> {
    let mut sources = Vec::new();
    for dir in WALKED {
        let base = root.join(dir);
        for path in tree.files(&base) {
            if path.extension().is_none_or(|ext| ext != "rs") {
                continue;
            }
            let Some(text) = tree.read(&path) else {
                continue;
            };
            let inside = path.strip_prefix(&base).unwrap_or(&path);
            let krate = if *dir == "crates" {
                inside
                    .components()
                    .next()
                    .map(|first| first.as_os_str().to_string_lossy().into_owned())
                    .unwrap_or_default()
            } else {
                (*dir).to_owned()
            };
            let shown = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            sources.push(Source { krate, shown, text });
        }
    }
    let spec = tree.read(&root.join(SPEC));
    judge(spec.as_deref(), &sources)
}

/// The refusals over `sources`, with the families read from `spec`. `spec`
/// is `None` when the file cannot be read.
pub fn judge(spec: Option<&str>, sources: &[Source]) -> Vec<String> {
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
    let mut crates: BTreeMap<String, Crate> = BTreeMap::new();
    for source in sources {
        let file = match syn::parse_file(&source.text) {
            Ok(file) => file,
            Err(e) => {
                problems.push(format!("{}: does not parse: {e}", source.shown));
                continue;
            }
        };
        crates
            .entry(source.krate.clone())
            .or_default()
            .read(&source.shown, &file);
    }
    let generic = crates
        .get(CORE)
        .map(|core| core.generic.clone())
        .unwrap_or_default();
    for family in &families {
        if !generic.contains(family) {
            problems.push(format!(
                "{SPEC}: the families table names {family}, and {CORE} declares no generic struct \
                 of that name. Correct the table"
            ));
        }
    }
    let resolver = Resolver {
        crates: &crates,
        families: &families,
    };
    for (krate, declared) in &crates {
        for alias in &declared.aliases {
            if let Some(found) = resolver.alias(krate, alias, Hops::default()) {
                problems.push(refusal(alias, &found));
            }
        }
    }
    problems
}

/// The message for an alias that stands for a family alone.
fn refusal(alias: &Alias, found: &Found) -> String {
    let through = if found.through.is_empty() {
        String::new()
    } else {
        format!(", through the alias {}", found.through.join(", then "))
    };
    format!(
        "{}: the alias {} stands for the family {} alone{through}. Remove the alias and write \
         the family in full at each use, as {} (ambre-spec-newtypes.md section 3)",
        alias.shown, alias.name, found.family, alias.target
    )
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

/// One module-level alias.
struct Alias {
    /// The file that declares it, as a message shows it.
    shown: String,
    name: String,
    /// The names of its type parameters, which stand for no family.
    params: BTreeSet<String>,
    ty: syn::Type,
    /// The target as text, for the message.
    target: String,
    /// The index of its file in [`Crate::files`].
    file: usize,
}

/// What one file says about the names it writes.
#[derive(Default)]
struct Names {
    /// Each name a `use` brings in, then the path it imports.
    imports: BTreeMap<String, Vec<String>>,
    /// Each struct, enum, union, trait and alias the file declares.
    items: BTreeSet<String>,
}

/// What one crate declares, across its files.
#[derive(Default)]
struct Crate {
    files: Vec<Names>,
    aliases: Vec<Alias>,
    /// Every struct, enum, union and trait name, in any file.
    items: BTreeSet<String>,
    /// Each name a `use` brings in, in any file, then the path it imports.
    /// A path of another module of the crate, or of another crate, reaches
    /// the name through one of them.
    imports: BTreeMap<String, Vec<String>>,
    /// Every struct that declares a type parameter.
    generic: BTreeSet<String>,
}

impl Crate {
    fn read(&mut self, shown: &str, file: &syn::File) {
        let index = self.files.len();
        let mut names = Names::default();
        self.items(shown, index, &file.items, &mut names);
        self.files.push(names);
    }

    /// Records the items of one module, and of each inline module in it.
    /// A `fn` body, an impl and a trait hold no module-level alias.
    fn items(&mut self, shown: &str, file: usize, items: &[syn::Item], names: &mut Names) {
        for item in items {
            match item {
                syn::Item::Type(t) => {
                    names.items.insert(t.ident.to_string());
                    let params = t
                        .generics
                        .type_params()
                        .map(|param| param.ident.to_string())
                        .collect();
                    self.aliases.push(Alias {
                        shown: shown.to_owned(),
                        name: t.ident.to_string(),
                        params,
                        ty: (*t.ty).clone(),
                        target: normalise(&t.ty.to_token_stream().to_string()),
                        file,
                    });
                }
                syn::Item::Struct(s) => {
                    if s.generics.type_params().next().is_some() {
                        self.generic.insert(s.ident.to_string());
                    }
                    self.declare(&s.ident, names);
                }
                syn::Item::Enum(e) => self.declare(&e.ident, names),
                syn::Item::Union(u) => self.declare(&u.ident, names),
                syn::Item::Trait(t) => self.declare(&t.ident, names),
                syn::Item::Use(u) => {
                    let mut found = Vec::new();
                    uses(&u.tree, &mut Vec::new(), &mut found);
                    for (name, path) in found {
                        self.imports.insert(name.clone(), path.clone());
                        names.imports.insert(name, path);
                    }
                }
                syn::Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content {
                        self.items(shown, file, inner, names);
                    }
                }
                _ => {}
            }
        }
    }

    fn declare(&mut self, ident: &syn::Ident, names: &mut Names) {
        names.items.insert(ident.to_string());
        self.items.insert(ident.to_string());
    }
}

/// Each name a `use` tree brings in, with the path it imports. A glob brings
/// in no name the check can list.
fn uses(tree: &syn::UseTree, prefix: &mut Vec<String>, found: &mut Vec<(String, Vec<String>)>) {
    match tree {
        syn::UseTree::Path(p) => {
            prefix.push(p.ident.to_string());
            uses(&p.tree, prefix, found);
            prefix.pop();
        }
        syn::UseTree::Name(n) => {
            let mut path = prefix.clone();
            path.push(n.ident.to_string());
            found.push((n.ident.to_string(), path));
        }
        syn::UseTree::Rename(r) => {
            let mut path = prefix.clone();
            path.push(r.ident.to_string());
            found.push((r.rename.to_string(), path));
        }
        syn::UseTree::Group(g) => {
            for item in &g.items {
                uses(item, prefix, found);
            }
        }
        syn::UseTree::Glob(_) => {}
    }
}

/// The family an alias stands for, and the aliases on the way.
struct Found {
    family: String,
    through: Vec<String>,
}

struct Resolver<'a> {
    crates: &'a BTreeMap<String, Crate>,
    families: &'a BTreeSet<String>,
}

impl Resolver<'_> {
    /// The family `alias` of `krate` stands for alone, if any.
    fn alias(&self, krate: &str, alias: &Alias, hops: Hops) -> Option<Found> {
        if hops > ALIAS_DEPTH {
            return None;
        }
        self.ty(krate, alias, &alias.ty, hops)
    }

    /// The family `ty` names alone, as `alias` of `krate` writes it.
    fn ty(&self, krate: &str, alias: &Alias, ty: &syn::Type, hops: Hops) -> Option<Found> {
        match ty {
            syn::Type::Reference(r) => self.ty(krate, alias, &r.elem, hops),
            syn::Type::Paren(p) => self.ty(krate, alias, &p.elem, hops),
            syn::Type::Group(g) => self.ty(krate, alias, &g.elem, hops),
            syn::Type::Path(path) if path.qself.is_none() => {
                let segments: Vec<String> = path
                    .path
                    .segments
                    .iter()
                    .map(|segment| segment.ident.to_string())
                    .collect();
                self.path(krate, alias, &segments, hops)
            }
            _ => None,
        }
    }

    /// The family a path names, as `alias` of `krate` writes it.
    fn path(&self, krate: &str, alias: &Alias, segments: &[String], hops: Hops) -> Option<Found> {
        let (first, last) = (segments.first()?, segments.last()?);
        let names = &self.crates.get(krate)?.files.get(alias.file)?;
        if segments.len() == 1 {
            if alias.params.contains(first) {
                return None;
            }
            if let Some(imported) = names.imports.get(first) {
                return self.imported(krate, imported, hops);
            }
            if names.items.contains(first) {
                // An item of the file: an alias of the file, or no family.
                let local = self
                    .crates
                    .get(krate)?
                    .aliases
                    .iter()
                    .find(|other| other.file == alias.file && other.name == *first)?;
                return self.through(krate, local, hops);
            }
            // A glob, or a module above the file, brings the name in.
            return self
                .named(krate, first, hops)
                .or_else(|| self.glob(krate, first));
        }
        self.imported(krate, segments, hops).or_else(|| {
            // A path that starts at an import of the file: `units::Count`.
            let imported = names.imports.get(first)?;
            let mut whole = imported.clone();
            whole.extend(segments.iter().skip(1).cloned());
            if whole.last() == Some(last) {
                self.imported(krate, &whole, hops)
            } else {
                None
            }
        })
    }

    /// The family a whole path names from `krate`: a path of `ambre-core`, a
    /// path of the crate itself, or a path of another workspace crate.
    fn imported(&self, krate: &str, path: &[String], hops: Hops) -> Option<Found> {
        let (first, last) = (path.first()?, path.last()?);
        if path.len() < 2 {
            return None;
        }
        let home = if LOCAL_ROOTS.contains(&first.as_str()) {
            krate.to_owned()
        } else {
            first.replace('_', "-")
        };
        if home == CORE {
            return self.families.contains(last).then(|| Found {
                family: last.clone(),
                through: Vec::new(),
            });
        }
        self.crates.get(&home)?;
        self.named(&home, last, hops)
    }

    /// The family `next`, an alias of `home`, stands for alone, with `next`
    /// first on the way.
    fn through(&self, home: &str, next: &Alias, hops: Hops) -> Option<Found> {
        let mut found = self.alias(home, next, hops.next())?;
        found.through.insert(0, next.name.clone());
        Some(found)
    }

    /// The family `name` names in `home`: an alias of the crate, or a family
    /// some `use` of the crate imports.
    fn named(&self, home: &str, name: &str, hops: Hops) -> Option<Found> {
        let declared = self.crates.get(home)?;
        if let Some(next) = declared.aliases.iter().find(|alias| alias.name == name) {
            return self.through(home, next, hops);
        }
        let imported = declared.imports.get(name)?;
        if imported.first().is_some_and(|first| first == CORE_ROOT) {
            return self.imported(home, imported, hops.next());
        }
        None
    }

    /// The family a glob brings in under `name`: one the crate does not
    /// shadow with an item of its own.
    fn glob(&self, krate: &str, name: &str) -> Option<Found> {
        let declared = self.crates.get(krate)?;
        let shadowed = krate != CORE && declared.items.contains(name);
        (self.families.contains(name) && !shadowed).then(|| Found {
            family: name.to_owned(),
            through: Vec::new(),
        })
    }
}

#[cfg(test)]
#[path = "aliases_tests.rs"]
mod tests;
