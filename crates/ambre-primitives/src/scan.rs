//! The scan: every site of the workspace whose type holds a primitive.
//!
//! The scan walks the module tree of each crate from its roots, the way rustc
//! does, rather than every file on disk. A module declared under a test gate is
//! then never opened, wherever its file sits. A file that no `mod` reaches is
//! reported instead of read: the gate cannot hold code it cannot place
//! (`ARCH-033`).
//!
//! syn parses and never expands. An item that a macro writes, and the output
//! of a build script, are therefore outside the scan by construction.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use quote::ToTokens;
use syn::visit::Visit;

use crate::allowlist::LineNo;

/// The types the rule names: a bare one of these needs a newtype or a reason
/// (`ai/ambre-rust-style.md`).
const PRIMITIVES: &[&str] = &[
    "String", "str", "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128",
    "isize", "f32", "f64", "bool", "char", "Uuid",
];

/// The Cargo features that only a test build turns on. A `cfg` that needs one
/// gates test support, like a `cfg(test)`.
const TEST_FEATURES: &[&str] = &["testing", "test-helpers"];

/// The attribute macros that turn a `fn` into a test.
const TEST_MACROS: &[&str] = &["test", "wasm_bindgen_test"];

/// The crates of the toolchain. A closure that names only their types is
/// no evidence that a foreign signature dictates it.
const TOOLCHAIN: &[&str] = &["std", "core", "alloc"];

/// The first segments of a path that stay inside the crate that writes it.
const LOCAL_ROOTS: &[&str] = &["crate", "self", "super", "Self"];

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
}

impl fmt::Display for SiteKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a site declares, the fifth field of its key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SiteKind {
    Field,
    Param,
    Ret,
    Const,
    Static,
    Alias,
}

impl SiteKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Field => "field",
            Self::Param => "param",
            Self::Ret => "ret",
            Self::Const => "const",
            Self::Static => "static",
            Self::Alias => "alias",
        }
    }
}

/// The source tree the scan reads. The disk in the gate, a map in the tests.
pub trait Tree {
    /// The text of one file, or `None` when the file does not exist.
    fn read(&self, path: &Path) -> Option<String>;

    /// Every file under `dir`, at any depth, sorted. A `target/` directory is
    /// build output and is left out.
    fn files(&self, dir: &Path) -> Vec<PathBuf>;
}

/// The tree on disk.
pub struct Disk;

impl Tree for Disk {
    fn read(&self, path: &Path) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }

    fn files(&self, dir: &Path) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let mut pending = vec![dir.to_path_buf()];
        while let Some(next) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(&next) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if path.file_name().is_some_and(|name| name != "target") {
                        pending.push(path);
                    }
                } else {
                    found.push(path);
                }
            }
        }
        found.sort();
        found
    }
}

/// What one scan found: the sites, and what kept it from reading the tree
/// whole.
#[derive(Debug, Default)]
pub struct Scan {
    pub sites: BTreeSet<SiteKey>,
    pub errors: Vec<String>,
    /// Each `Deref` or `DerefMut` of a workspace type to a primitive, in
    /// words a reviewer reads (`ARCH-037`). No line of the allowlist admits
    /// one.
    pub derefs: Vec<String>,
    /// Every call of the workspace, tests included, that passes a `bool`
    /// literal (`ARCH-040`).
    literals: Vec<BoolCall>,
    /// Where each site sits, for the categories the gate refutes from the
    /// code (`ARCH-036`).
    places: BTreeMap<SiteKey, Place>,
    declared: Declared,
}

/// Where one site sits: what a `newtype-inner` or a `trait` line claims of
/// it, read from the code and never from the note.
#[derive(Clone, Debug)]
struct Place {
    /// The crate directory, as the key names it.
    krate: String,
    /// The module path, as the key names it.
    module: String,
    kind: SiteKind,
    /// The struct that declares the field. `None` for a field of an enum
    /// variant or of a union, and for every other kind.
    owner: Option<String>,
    /// The impl whose signature or constant the site is. `None` outside an
    /// impl, and in a body nested inside one.
    host: Option<Host>,
    form: TypeForm,
    /// The first segment of each path of the type with two or more.
    roots: BTreeSet<String>,
    /// Who can reach the site (`ARCH-038`).
    privacy: Privacy,
    /// Whether the type is a `bool`, or holds one.
    shape: Shape,
    /// The `fn` and the position of a parameter. `None` for every other
    /// kind.
    slot: Option<Slot>,
}

/// Who can reach a site, as the `local` rule and the `bool` rule read it
/// (`ai/ambre-rust-style.md`). Every variant but `Private` is a site that is
/// not private.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Privacy {
    /// No `pub`, on a type of no wire.
    #[default]
    Private,
    /// `pub(crate)`, `pub(super)` or `pub(in path)`.
    Restricted,
    /// `pub`.
    Public,
    /// A signature or a constant of a trait impl: the trait reaches it
    /// wherever the type goes.
    TraitImpl,
    /// A field of a type of `ambre-bus`, which crosses a thread.
    BusType,
    /// A field of a type that derives `Serialize`, `Deserialize` or
    /// `FromRow`, which crosses a wire or a store.
    Serialised,
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Shape {
    /// Any type without a `bool`.
    #[default]
    Other,
    /// A bare `bool`.
    Bool,
    /// A type that wraps a `bool`: `Option<bool>`, `Cell<bool>`.
    HoldsBool,
}

/// The position of an argument in a call, or of a parameter in a signature,
/// `self` left out. The first is 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ArgPosition(usize);

/// Where a parameter sits: its `fn`, and its position there.
#[derive(Clone, Debug)]
struct Slot {
    /// The name of the `fn`, the name a call writes.
    name: String,
    position: ArgPosition,
    receiver: Receiver,
}

/// How a call names its `fn`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CallForm {
    /// `g(..)` or `Type::g(..)`: a method's receiver, if any, is the first
    /// argument.
    Path,
    /// `x.g(..)`: the receiver sits before the dot.
    Method,
}

/// One `bool` literal passed to a call, anywhere in a crate, tests included.
#[derive(Clone, Debug)]
struct BoolCall {
    /// The crate directory of the call.
    krate: String,
    /// The file of the call, from the workspace root.
    file: String,
    line: LineNo,
    /// The last segment of the called path, or the method name.
    name: String,
    position: ArgPosition,
    form: CallForm,
}

/// Whether a type is called or held.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum TypeForm {
    /// Any type that is not a closure or a function pointer.
    #[default]
    Value,
    /// A closure bound or a function pointer.
    Callable,
}

/// Whether an associated `fn` takes `self`, by value or by reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Receiver {
    /// No `self`: a constructor, or a constant.
    Without,
    /// A method.
    With,
}

/// What an associated `fn` returns, as a newtype claim reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Output {
    /// Anything else, or nothing.
    Other,
    /// The implementing type: `Self`, `Option<Self>` or `Result<Self, _>`,
    /// or the same with the type's own name.
    Itself,
}

/// What a type declaration is, as a newtype claim reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Decl {
    /// A struct with exactly one field, or a type a `define_*!` macro
    /// writes.
    Newtype,
    /// A struct with no field, or with two or more.
    Struct,
    Enum,
}

/// Which crates a newtype claim reads to find the type of an impl.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reach {
    /// The crate of the site alone.
    OwnCrate,
    /// The crate of the site, then the other crates of the workspace: the
    /// impl implements a trait of its own crate (the orphan rule).
    TraitCrate,
}

/// One struct or enum, with the module that declares it.
#[derive(Clone, Debug)]
struct Declaration {
    module: String,
    decl: Decl,
}

/// Which of the two dereference traits an impl implements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DerefTrait {
    Deref,
    DerefMut,
}

impl fmt::Display for DerefTrait {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Deref => "Deref",
            Self::DerefMut => "DerefMut",
        })
    }
}

/// One impl of `Deref` or `DerefMut`, before its target is resolved.
#[derive(Clone, Debug)]
struct DerefImpl {
    /// The file, relative to the crate directory.
    file: String,
    /// The implementing type, as the impl writes it.
    ty: String,
    /// The last segment of the implementing type, the name `DerefMut` is
    /// matched by.
    self_ty: String,
    of: DerefTrait,
    /// The `Target` of a `Deref`, normalised. `None` for a `DerefMut`,
    /// whose target its `Deref` states.
    target: Option<String>,
}

/// The impl item that holds a site.
#[derive(Clone, Debug)]
struct Host {
    /// The last segment of the implementing type, without its generics.
    self_ty: String,
    /// The trait the impl implements, `None` for an inherent impl.
    of_trait: Option<TraitPath>,
    /// `Without` for a constant.
    receiver: Receiver,
    /// `Other` for a constant.
    output: Output,
}

/// The trait of an impl, as the impl writes its path.
#[derive(Clone, Debug)]
struct TraitPath {
    /// The last segment, without its generics.
    name: String,
    /// The first segment, when the path has two or more.
    root: Option<String>,
}

/// What the workspace declares, across every crate the scan reads.
#[derive(Debug, Default)]
struct Declared {
    /// The crate names, as a path writes them.
    crates: BTreeSet<String>,
    /// The name of every trait, per crate directory.
    traits: BTreeMap<String, BTreeSet<String>>,
    /// The dependencies each crate directory names in its manifest, dev and
    /// build ones included.
    deps: BTreeMap<String, BTreeSet<String>>,
    /// Every struct and enum, per crate directory, then per name. A name
    /// declared twice in one crate keeps both.
    types: BTreeMap<String, BTreeMap<String, Vec<Declaration>>>,
}

impl Declared {
    /// Whether a path that starts with `root` names an item of the
    /// workspace.
    fn owns_root(&self, root: &str) -> bool {
        LOCAL_ROOTS.contains(&root) || self.crates.contains(root)
    }

    /// Whether the workspace declares the trait. A path of one segment is
    /// looked up by name. A foreign trait with the name of a workspace trait
    /// then counts as the workspace's.
    fn owns_trait(&self, path: &TraitPath) -> bool {
        match &path.root {
            Some(root) => self.owns_root(root),
            None => self.traits.values().any(|names| names.contains(&path.name)),
        }
    }

    /// Where a newtype claim looks for the type of `host`. The orphan rule
    /// puts the impl of a trait in the crate that declares the trait. That
    /// impl may name a type of another crate.
    fn reach(&self, site: &Place, host: &Host) -> Reach {
        let Some(path) = &host.of_trait else {
            return Reach::OwnCrate;
        };
        let here = match &path.root {
            Some(root) => {
                LOCAL_ROOTS.contains(&root.as_str()) || *root == site.krate.replace('-', "_")
            }
            None => self
                .traits
                .get(&site.krate)
                .is_some_and(|names| names.contains(&path.name)),
        };
        if here {
            Reach::TraitCrate
        } else {
            Reach::OwnCrate
        }
    }

    /// Whether `name` is a struct with exactly one field, in the crate of
    /// the site. The declaration in the module of the site wins; with none
    /// there, every declaration of the crate must be one.
    ///
    /// Under [`Reach::TraitCrate`], a type the crate of the site lacks is
    /// read in the crates it depends on. Every declaration there must be a
    /// newtype.
    fn single_field(&self, site: &Place, name: &str, reach: Reach) -> Result<(), String> {
        let (krate, module) = (&site.krate, &site.module);
        let found = self.types.get(krate).and_then(|names| names.get(name));
        if found.is_none() && reach == Reach::TraitCrate {
            // The newtype comes from a crate this one depends on. A type of
            // the same name in any other crate is not the one the impl names.
            let deps = self.deps.get(krate);
            let elsewhere: Vec<Decl> = self
                .types
                .iter()
                .filter(|(other, _)| *other != krate)
                .filter(|(other, _)| deps.is_some_and(|deps| deps.contains(*other)))
                .filter_map(|(_, names)| names.get(name))
                .flatten()
                .map(|declaration| declaration.decl)
                .collect();
            return match elsewhere.as_slice() {
                [] => Err(format!(
                    "{name} is no type of a crate {krate} depends on: an alias or a foreign type"
                )),
                decls if decls.iter().all(|decl| *decl == Decl::Newtype) => Ok(()),
                _ => Err(format!(
                    "{name} of another crate is not a struct with exactly one field"
                )),
            };
        }
        let here: Option<Vec<Decl>> = found.map(|all| {
            let local: Vec<Decl> = all
                .iter()
                .filter(|declaration| declaration.module == *module)
                .map(|declaration| declaration.decl)
                .collect();
            if local.is_empty() {
                all.iter().map(|declaration| declaration.decl).collect()
            } else {
                local
            }
        });
        match here {
            None if self.types.values().any(|names| names.contains_key(name)) => Err(format!(
                "{name} is a type of another crate, so this impl is not the newtype's own"
            )),
            None => Err(format!(
                "{name} is no type of {krate}: an alias or a foreign type"
            )),
            Some(decls) if decls.iter().all(|decl| *decl == Decl::Newtype) => Ok(()),
            Some(decls) if decls.contains(&Decl::Enum) => {
                Err(format!("{name} is an enum, not a struct"))
            }
            Some(_) => Err(format!(
                "{name} is a struct with no field, or with two or more"
            )),
        }
    }
}

impl Scan {
    /// Checks the claim of a `newtype-inner` line (`ai/ambre-rust-style.md`).
    /// The site sits in a struct with exactly one field. It is that field, a
    /// parameter of a `fn` that returns the struct, or a method's return.
    ///
    /// A key the scan did not find passes: the stale check reports it.
    ///
    /// # Errors
    ///
    /// Why the site has another shape, in words a reviewer reads beside the
    /// key.
    pub fn newtype_inner(&self, key: &SiteKey) -> Result<(), String> {
        let Some(place) = self.places.get(key) else {
            return Ok(());
        };
        match place.kind {
            SiteKind::Field => match &place.owner {
                Some(owner) => self.declared.single_field(place, owner, Reach::OwnCrate),
                None => Err("a field of an enum variant or of a union, not of a struct".to_owned()),
            },
            SiteKind::Param | SiteKind::Ret => {
                let Some(host) = &place.host else {
                    return Err("a signature outside an impl, or nested in a body".to_owned());
                };
                let reach = self.declared.reach(place, host);
                self.declared.single_field(place, &host.self_ty, reach)?;
                match place.kind {
                    SiteKind::Param if host.output == Output::Itself => Ok(()),
                    SiteKind::Param => Err(format!(
                        "a parameter of a fn of {} that returns neither Self, Option<Self> \
                         nor Result<Self, _>",
                        host.self_ty
                    )),
                    _ if host.receiver == Receiver::With => Ok(()),
                    _ => Err(format!(
                        "the return of a fn of {} that takes no self",
                        host.self_ty
                    )),
                }
            }
            kind => Err(format!(
                "a {} is never the inside of a newtype",
                kind.as_str()
            )),
        }
    }

    /// Checks the claim of a `trait` line. The site is a signature or a
    /// constant of an impl of a trait the workspace does not declare. Or
    /// its type is a closure or a function pointer that names a type of a
    /// foreign crate, outside the toolchain.
    ///
    /// A key the scan did not find passes: the stale check reports it.
    ///
    /// # Errors
    ///
    /// Why no foreign signature dictates the site.
    pub fn foreign_signature(&self, key: &SiteKey) -> Result<(), String> {
        let Some(place) = self.places.get(key) else {
            return Ok(());
        };
        if let Some(path) = place.host.as_ref().and_then(|host| host.of_trait.as_ref()) {
            return if self.declared.owns_trait(path) {
                Err(format!(
                    "the trait {} is declared in the workspace, so its signature takes types",
                    path.name
                ))
            } else {
                Ok(())
            };
        }
        let foreign = place
            .roots
            .iter()
            .any(|root| !self.declared.owns_root(root) && !TOOLCHAIN.contains(&root.as_str()));
        match (place.form, foreign) {
            (TypeForm::Callable, true) => Ok(()),
            (TypeForm::Callable, false) => {
                Err("a closure type that names no type of a foreign crate".to_owned())
            }
            (TypeForm::Value, _) => {
                Err("neither in an impl of a foreign trait nor a closure type".to_owned())
            }
        }
    }
}

impl Scan {
    /// Checks that a `local` line names a private site (`ARCH-038`).
    ///
    /// A key the scan did not find passes: the stale check reports it.
    ///
    /// # Errors
    ///
    /// Who else reaches the site.
    pub fn local_site(&self, key: &SiteKey) -> Result<(), String> {
        match self.places.get(key).map(|place| place.privacy) {
            None | Some(Privacy::Private) => Ok(()),
            Some(privacy) => Err(format!(
                "the site is not private, it is {privacy}: a local line names a private site \
                 only (ARCH-038)"
            )),
        }
    }

    /// Checks that a field holding a `bool` is private (`ARCH-039`). The
    /// caller skips a `foreign-api` line, the one category that admits a
    /// field a foreign schema fixes.
    ///
    /// # Errors
    ///
    /// Who else reaches the field.
    pub fn bool_field(&self, key: &SiteKey) -> Result<(), String> {
        let Some(place) = self.places.get(key) else {
            return Ok(());
        };
        if place.kind != SiteKind::Field
            || place.shape == Shape::Other
            || place.privacy == Privacy::Private
        {
            return Ok(());
        }
        Err(format!(
            "a bool field that is {}: it is an enum, define_flag! writes one, unless the field \
             mirrors a schema the workspace does not control, under foreign-api (ARCH-039)",
            place.privacy
        ))
    }

    /// Checks that no caller writes a `bool` parameter that is not private
    /// as a literal (`ARCH-040`). A call matches a parameter by the name of
    /// its `fn` and by position. The call sits in the crate of the site, or,
    /// for a `pub` site, in a crate that depends on it. `listed` holds the
    /// keys of the allowlist, the other sites a call may match.
    ///
    /// # Errors
    ///
    /// The first literal and the count of the others. It also counts the
    /// calls that match another site too. A name cannot tell those apart,
    /// so each match is refused.
    pub fn bool_literal(&self, key: &SiteKey, listed: &BTreeSet<SiteKey>) -> Result<(), String> {
        let Some(place) = self.places.get(key) else {
            return Ok(());
        };
        if place.shape != Shape::Bool || place.privacy == Privacy::Private {
            return Ok(());
        }
        let hits: Vec<&BoolCall> = self
            .literals
            .iter()
            .filter(|call| self.writes(place, call))
            .collect();
        let Some(first) = hits.first() else {
            return Ok(());
        };
        let others: Vec<&Place> = listed
            .iter()
            .filter(|other| *other != key)
            .filter_map(|other| self.places.get(other))
            .filter(|other| other.shape == Shape::Bool && other.slot.is_some())
            .collect();
        let shared = hits
            .iter()
            .filter(|call| others.iter().any(|other| self.writes(other, call)))
            .count();
        let more = match hits.len() {
            1 => String::new(),
            n => format!(" and {} more", n - 1),
        };
        let collision = match shared {
            0 => String::new(),
            n => format!("; {n} of these calls match another site too, so each match is refused"),
        };
        Err(format!(
            "a caller writes this bool parameter as a literal at {}:{}{more}, and the site is {}: \
             it is an enum (ARCH-040){collision}",
            first.file, first.line, place.privacy
        ))
    }

    /// Whether `call` passes its literal to the parameter at `place`.
    fn writes(&self, place: &Place, call: &BoolCall) -> bool {
        let Some(slot) = &place.slot else {
            return false;
        };
        let reached = call.krate == place.krate
            || (matches!(place.privacy, Privacy::Public | Privacy::TraitImpl)
                && self
                    .declared
                    .deps
                    .get(&call.krate)
                    .is_some_and(|deps| deps.contains(&place.krate)));
        let (ArgPosition(at), ArgPosition(param)) = (call.position, slot.position);
        let aligned = match (call.form, slot.receiver) {
            (CallForm::Method, Receiver::With) | (CallForm::Path, Receiver::Without) => at == param,
            (CallForm::Path, Receiver::With) => at == param + 1,
            (CallForm::Method, Receiver::Without) => false,
        };
        reached && aligned && call.name == slot.name
    }
}

/// The dependencies a manifest names, in every table whose name ends with
/// `dependencies`, and in every `[...dependencies.name]` table.
fn dependencies(manifest: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut table = String::new();
    for line in manifest.lines().map(str::trim) {
        if let Some(header) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            table = header.trim().to_owned();
            if let Some((_, name)) = table.split_once("dependencies.") {
                found.insert(name.trim_matches('"').to_owned());
            }
            continue;
        }
        if !table.ends_with("dependencies") || line.starts_with('#') {
            continue;
        }
        if let Some((key, _)) = line.split_once('=') {
            let name = key
                .split('.')
                .next()
                .unwrap_or(key)
                .trim()
                .trim_matches('"');
            if !name.is_empty() {
                found.insert(name.to_owned());
            }
        }
    }
    found
}

/// Who can reach an item of visibility `vis`. `pub(self)` is private.
fn privacy(vis: &syn::Visibility) -> Privacy {
    match vis {
        syn::Visibility::Public(_) => Privacy::Public,
        syn::Visibility::Restricted(restricted) if restricted.path.is_ident("self") => {
            Privacy::Private
        }
        syn::Visibility::Restricted(_) => Privacy::Restricted,
        syn::Visibility::Inherited => Privacy::Private,
    }
}

/// `Serialised` for a type that derives `Serialize`, `Deserialize` or
/// `FromRow`, under `derive` or `cfg_attr`; `Private` otherwise.
fn derived(attrs: &[syn::Attribute]) -> Privacy {
    let wire = attrs
        .iter()
        .filter(|attr| attr.path().is_ident("derive") || attr.path().is_ident("cfg_attr"))
        .any(|attr| {
            attr.meta
                .to_token_stream()
                .to_string()
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .any(|word| ["Serialize", "Deserialize", "FromRow"].contains(&word))
        });
    if wire {
        Privacy::Serialised
    } else {
        Privacy::Private
    }
}

/// The scan of one file for the `bool` literals its calls pass, macro
/// arguments that parse as expressions included.
struct LiteralVisit {
    /// The crate directory.
    krate: String,
    /// The file, from the workspace root.
    file: String,
    calls: Vec<BoolCall>,
}

impl LiteralVisit {
    fn record<'e>(
        &mut self,
        form: CallForm,
        name: &syn::Ident,
        args: impl IntoIterator<Item = &'e syn::Expr>,
    ) {
        for (i, arg) in args.into_iter().enumerate() {
            if let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Bool(literal),
                ..
            }) = arg
            {
                self.calls.push(BoolCall {
                    krate: self.krate.clone(),
                    file: self.file.clone(),
                    line: LineNo(literal.span.start().line),
                    name: name.to_string(),
                    position: ArgPosition(i),
                    form,
                });
            }
        }
    }
}

impl<'ast> Visit<'ast> for LiteralVisit {
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = &*call.func
            && let Some(last) = path.path.segments.last()
        {
            self.record(CallForm::Path, &last.ident, &call.args);
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        self.record(CallForm::Method, &call.method, &call.args);
        syn::visit::visit_expr_method_call(self, call);
    }

    // syn leaves a macro's tokens unparsed. The arguments of `assert!`,
    // `assert_eq!`, `vec!` and `format!` parse as expressions, and a test
    // writes many of its calls there.
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if let Ok(args) = mac.parse_body_with(
            syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated,
        ) {
            for arg in &args {
                self.visit_expr(arg);
            }
        }
    }
}

/// Scans every crate under `crates`, a directory holding one directory per
/// crate.
pub fn scan(tree: &impl Tree, crates: &Path) -> Scan {
    let mut result = Scan::default();
    let manifests = tree
        .files(crates)
        .into_iter()
        .filter(|path| path.file_name().is_some_and(|name| name == "Cargo.toml"))
        .filter(|path| path.parent().and_then(Path::parent) == Some(crates));
    for manifest in manifests {
        let Some(dir) = manifest.parent() else {
            continue;
        };
        scan_crate(tree, dir, &mut result);
    }
    result
}

/// The facts a type gives a [`Place`]: its form, and the roots of its
/// paths.
#[derive(Default)]
struct TypeFacts {
    form: TypeForm,
    roots: BTreeSet<String>,
}

impl<'ast> Visit<'ast> for TypeFacts {
    fn visit_type_bare_fn(&mut self, f: &'ast syn::TypeBareFn) {
        self.form = TypeForm::Callable;
        syn::visit::visit_type_bare_fn(self, f);
    }

    fn visit_path_segment(&mut self, segment: &'ast syn::PathSegment) {
        if matches!(segment.arguments, syn::PathArguments::Parenthesized(_))
            && ["Fn", "FnMut", "FnOnce"]
                .iter()
                .any(|name| segment.ident == name)
        {
            self.form = TypeForm::Callable;
        }
        syn::visit::visit_path_segment(self, segment);
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        if path.segments.len() >= 2
            && let Some(first) = path.segments.first()
        {
            self.roots.insert(first.ident.to_string());
        }
        syn::visit::visit_path(self, path);
    }
}

/// The last segment of a type path, without its generics, or the whole type
/// as text for any other type.
fn type_name(ty: &syn::Type) -> String {
    match ty {
        syn::Type::Path(path) => path
            .path
            .segments
            .last()
            .map_or_else(String::new, |last| last.ident.to_string()),
        other => normalise(&other.to_token_stream().to_string()),
    }
}

/// What a return type is. `Itself` is `Self` or `owner`, bare or as the
/// first argument of an `Option` or a `Result`.
fn output(output: &syn::ReturnType, owner: &str) -> Output {
    let syn::ReturnType::Type(_, ty) = output else {
        return Output::Other;
    };
    if is_self(ty, owner) || wrapped(ty).is_some_and(|inner| is_self(inner, owner)) {
        Output::Itself
    } else {
        Output::Other
    }
}

/// Whether a type is `Self` or `owner`, bare.
fn is_self(ty: &syn::Type, owner: &str) -> bool {
    let syn::Type::Path(path) = ty else {
        return false;
    };
    let Some(last) = path.path.segments.last() else {
        return false;
    };
    path.qself.is_none()
        && ((last.ident == "Self" && path.path.segments.len() == 1) || last.ident == owner)
}

/// The first type argument of an `Option` or a `Result`.
fn wrapped(ty: &syn::Type) -> Option<&syn::Type> {
    let syn::Type::Path(path) = ty else {
        return None;
    };
    let last = path.path.segments.last()?;
    if !(last.ident == "Option" || last.ident == "Result") {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &last.arguments else {
        return None;
    };
    args.args.iter().find_map(|arg| match arg {
        syn::GenericArgument::Type(inner) => Some(inner),
        _ => None,
    })
}

/// The types a `define_*!` invocation writes: each name after a `struct`,
/// or, with no `struct`, each top-level word in `UpperCamelCase`. A macro
/// that writes an `enum` writes no newtype.
fn macro_types(mac: &syn::Macro) -> Vec<String> {
    let words = mac
        .parse_body_with(|input: syn::parse::ParseStream<'_>| {
            input.step(|cursor| {
                let mut words = Vec::new();
                let mut rest = *cursor;
                while let Some((_, next)) = rest.token_tree() {
                    if let Some((ident, _)) = rest.ident() {
                        words.push(ident.to_string());
                    }
                    rest = next;
                }
                Ok((words, rest))
            })
        })
        .unwrap_or_default();
    if words.iter().any(|word| word == "enum") {
        return Vec::new();
    }
    if words.iter().any(|word| word == "struct") {
        return words
            .windows(2)
            .filter(|pair| pair.first().is_some_and(|word| word == "struct"))
            .filter_map(|pair| pair.get(1).cloned())
            .collect();
    }
    words
        .into_iter()
        .filter(|word| {
            word.starts_with(|c: char| c.is_ascii_uppercase())
                && word.chars().any(|c| c.is_ascii_lowercase())
        })
        .collect()
}

/// A file the walk still has to read, and the module it holds. A `mod x;`
/// names two candidates, `x.rs` and `x/mod.rs`, and the first that exists is
/// the file.
struct Pending {
    candidates: Vec<PathBuf>,
    module: Vec<String>,
}

/// Scans one crate from its roots: the library, the binaries and the build
/// script.
fn scan_crate(tree: &impl Tree, dir: &Path, result: &mut Scan) {
    let name = dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    result.declared.crates.insert(name.replace('-', "_"));
    if let Some(manifest) = tree.read(&dir.join("Cargo.toml")) {
        result
            .declared
            .deps
            .insert(name.clone(), dependencies(&manifest));
    }
    let all = tree.files(dir);
    let src = dir.join("src");
    let roots = all.iter().filter(|path| {
        let is_rs = path.extension().is_some_and(|ext| ext == "rs");
        **path == src.join("lib.rs")
            || **path == src.join("main.rs")
            || **path == dir.join("build.rs")
            || (is_rs && path.parent() == Some(src.join("bin").as_path()))
    });
    let mut pending: Vec<Pending> = roots
        .map(|path| Pending {
            candidates: vec![path.clone()],
            module: vec!["crate".to_owned()],
        })
        .collect();
    let mut reached = BTreeSet::new();
    let mut skipped = Skipped::default();
    let mut keys: Vec<RawKey> = Vec::new();
    let mut derefs: Vec<DerefImpl> = Vec::new();
    while let Some(next) = pending.pop() {
        let Some((path, text)) = next
            .candidates
            .iter()
            .find_map(|path| tree.read(path).map(|text| (path.clone(), text)))
        else {
            let names: Vec<String> = next
                .candidates
                .iter()
                .map(|path| path.display().to_string())
                .collect();
            result.errors.push(format!(
                "{}: declared by a `mod`, and missing",
                names.join(" or ")
            ));
            continue;
        };
        if !reached.insert(path.clone()) {
            continue;
        }
        let file = match syn::parse_file(&text) {
            Ok(file) => file,
            Err(e) => {
                result
                    .errors
                    .push(format!("{}: syn cannot parse it: {e}", path.display()));
                continue;
            }
        };
        if test_only(&file.attrs) {
            continue;
        }
        let relative = path.strip_prefix(dir).unwrap_or(&path);
        let mut visit = FileVisit::new(&name, relative, &path, next.module);
        visit.visit_file(&file);
        keys.append(&mut visit.keys);
        derefs.append(&mut visit.derefs);
        result
            .declared
            .traits
            .entry(name.clone())
            .or_default()
            .append(&mut visit.traits);
        for (owner, decl) in visit.types {
            result
                .declared
                .types
                .entry(name.clone())
                .or_default()
                .entry(owner)
                .or_default()
                .push(decl);
        }
        pending.extend(visit.children);
        skipped.files.append(&mut visit.skipped.files);
        skipped.dirs.append(&mut visit.skipped.dirs);
    }
    result.derefs.extend(refused_derefs(&name, &derefs));
    // Tests are callers too: every file of the crate is read for literals,
    // whether a `mod` reaches it or not.
    for path in all
        .iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
    {
        let Some(text) = tree.read(path) else {
            continue;
        };
        let Ok(file) = syn::parse_file(&text) else {
            if !reached.contains(path) {
                result
                    .errors
                    .push(format!("{}: syn cannot parse it", path.display()));
            }
            continue;
        };
        let relative = path.strip_prefix(dir).unwrap_or(path);
        let mut visit = LiteralVisit {
            krate: name.clone(),
            file: format!(
                "crates/{name}/{}",
                relative.to_string_lossy().replace('\\', "/")
            ),
            calls: Vec::new(),
        };
        visit.visit_file(&file);
        result.literals.append(&mut visit.calls);
    }
    for (key, place) in dedup(keys) {
        result.sites.insert(key.clone());
        result.places.insert(key, place);
    }
    for path in all.iter().filter(|path| path.starts_with(&src)) {
        let is_rs = path.extension().is_some_and(|ext| ext == "rs");
        if is_rs && !reached.contains(path) && !skipped.covers(path) && !is_test_file(path) {
            result.errors.push(format!(
                "{}: no `mod` of the crate reaches it, so the gate cannot place it",
                path.display()
            ));
        }
    }
}

/// The files and the directories a test gate keeps out of the scan.
#[derive(Default)]
struct Skipped {
    files: Vec<PathBuf>,
    dirs: Vec<PathBuf>,
}

impl Skipped {
    fn covers(&self, path: &Path) -> bool {
        self.files.iter().any(|file| file == path)
            || self.dirs.iter().any(|dir| path.starts_with(dir))
    }
}

/// A file named after the tests it holds, kept out whatever declares it.
fn is_test_file(path: &Path) -> bool {
    path.file_stem()
        .is_some_and(|stem| stem.to_string_lossy().ends_with("_tests"))
}

/// A key before the duplicates are numbered: the fields before the item
/// path, the item path, and the fields after it.
struct RawKey {
    head: String,
    item: String,
    tail: String,
    place: Place,
}

/// Numbers the second and later sites that share one key, `#2` and on, so two
/// `cfg` twins of one `fn` keep a key each.
fn dedup(keys: Vec<RawKey>) -> Vec<(SiteKey, Place)> {
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    keys.into_iter()
        .map(|raw| {
            let sep = SiteKey::SEPARATOR;
            let base = format!("{}{sep}{}{sep}{}", raw.head, raw.item, raw.tail);
            let count = seen.entry(base.clone()).or_insert(0);
            *count += 1;
            let key = if *count == 1 {
                SiteKey(base)
            } else {
                SiteKey(format!(
                    "{}{sep}{}#{count}{sep}{}",
                    raw.head, raw.item, raw.tail
                ))
            };
            (key, raw.place)
        })
        .collect()
}

/// Three-valued logic over a `cfg` predicate, with every test switch off and
/// every other switch unknown.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tri {
    True,
    False,
    Unknown,
}

impl Tri {
    fn not(self) -> Self {
        match self {
            Self::True => Self::False,
            Self::False => Self::True,
            Self::Unknown => Self::Unknown,
        }
    }
}

/// Evaluates a `cfg` predicate in a build that is not a test.
fn evaluate(meta: &syn::Meta) -> Tri {
    match meta {
        syn::Meta::Path(path) if path.is_ident("test") => Tri::False,
        syn::Meta::NameValue(pair) if pair.path.is_ident("feature") => match &pair.value {
            syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(name),
                ..
            }) if TEST_FEATURES.contains(&name.value().as_str()) => Tri::False,
            _ => Tri::Unknown,
        },
        syn::Meta::List(list) => {
            let Ok(args) = list.parse_args_with(
                syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
            ) else {
                return Tri::Unknown;
            };
            let values = args.iter().map(evaluate);
            if list.path.is_ident("all") {
                values.fold(Tri::True, |acc, value| match (acc, value) {
                    (Tri::False, _) | (_, Tri::False) => Tri::False,
                    (Tri::True, Tri::True) => Tri::True,
                    _ => Tri::Unknown,
                })
            } else if list.path.is_ident("any") {
                values.fold(Tri::False, |acc, value| match (acc, value) {
                    (Tri::True, _) | (_, Tri::True) => Tri::True,
                    (Tri::False, Tri::False) => Tri::False,
                    _ => Tri::Unknown,
                })
            } else if list.path.is_ident("not") {
                let mut values = values;
                match (values.next(), values.next()) {
                    (Some(value), None) => value.not(),
                    _ => Tri::Unknown,
                }
            } else {
                Tri::Unknown
            }
        }
        _ => Tri::Unknown,
    }
}

/// Whether the attributes keep an item out of every build but a test:
/// a `cfg` that no other build satisfies, or a test macro.
fn test_only(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        let path = attr.path();
        if path.is_ident("cfg") {
            return attr
                .parse_args::<syn::Meta>()
                .is_ok_and(|meta| evaluate(&meta) == Tri::False);
        }
        path.segments
            .last()
            .is_some_and(|last| TEST_MACROS.iter().any(|name| last.ident == name))
    })
}

/// The `#[path = "..."]` of a module declaration, if it carries one.
fn path_attr(attrs: &[syn::Attribute]) -> Option<String> {
    attrs.iter().find_map(|attr| match &attr.meta {
        syn::Meta::NameValue(pair) if pair.path.is_ident("path") => match &pair.value {
            syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(path),
                ..
            }) => Some(path.value()),
            _ => None,
        },
        _ => None,
    })
}

/// A token stream as one line: a space stays only between two words, so
/// `& 'static str` reads `&'static str` and `Vec < u8 >` reads `Vec<u8>`.
fn normalise(tokens: &str) -> String {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let chars: Vec<char> = tokens.chars().collect();
    let mut out = String::with_capacity(tokens.len());
    for (i, c) in chars.iter().enumerate() {
        if *c != ' ' {
            out.push(*c);
            continue;
        }
        let before = out.chars().last().is_some_and(word);
        let after = chars
            .get(i + 1)
            .is_some_and(|next| word(*next) || *next == '\'');
        if before && after {
            out.push(' ');
        }
    }
    out
}

/// The primitive a dereference target exposes: the target itself, or the
/// element of a slice, an array or a `Vec` of it. `None` for any other
/// target.
fn primitive_target(target: &str) -> Option<&str> {
    let inner = target
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .map(|rest| rest.split(';').next().unwrap_or(rest))
        .or_else(|| {
            target
                .strip_prefix("Vec<")
                .and_then(|rest| rest.strip_suffix('>'))
        })
        .unwrap_or(target);
    PRIMITIVES.contains(&inner).then_some(inner)
}

/// The refusal of each impl of `derefs` whose target is a primitive. A
/// `DerefMut` takes the target of the `Deref` of the same type in the crate.
fn refused_derefs(krate: &str, derefs: &[DerefImpl]) -> Vec<String> {
    derefs
        .iter()
        .filter_map(|deref| {
            let target = match &deref.target {
                Some(target) => target.as_str(),
                None => derefs
                    .iter()
                    .filter(|other| other.of == DerefTrait::Deref && other.self_ty == deref.self_ty)
                    .find_map(|other| other.target.as_deref())?,
            };
            let primitive = primitive_target(target)?;
            Some(format!(
                "{krate} | {}: {} of {} to {target} exposes the primitive {primitive}. A newtype \
                 never dereferences to its primitive: name an accessor for the one foreign call \
                 that needs it (ARCH-037)",
                deref.file, deref.of, deref.ty
            ))
        })
        .collect()
}

/// Whether a normalised type names one of [`PRIMITIVES`] as a whole word.
fn holds_primitive(ty: &str) -> bool {
    ty.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|word| PRIMITIVES.contains(&word))
}

/// The scan of one file: the sites it declares, and the module files it
/// names.
struct FileVisit<'a> {
    krate: &'a str,
    file: String,
    module: Vec<String>,
    /// The item path above the next site: an impl, a trait, an enclosing `fn`.
    scope: Vec<String>,
    /// Where a `mod x;` of the current module resolves.
    dir: PathBuf,
    /// Where a `#[path]` of the current file resolves, or `None` inside an
    /// inline module, where it resolves against `dir`.
    file_dir: Option<PathBuf>,
    keys: Vec<RawKey>,
    children: Vec<Pending>,
    skipped: Skipped,
    /// The impl item whose signature the visit reads, `None` elsewhere.
    host: Option<Host>,
    /// The struct whose fields the visit reads, `None` elsewhere.
    owner: Option<String>,
    /// Every trait the file declares.
    traits: BTreeSet<String>,
    /// Every struct and enum the file declares, by name.
    types: Vec<(String, Declaration)>,
    /// Every impl of `Deref` or `DerefMut` the file writes.
    derefs: Vec<DerefImpl>,
    /// Who reaches the next site.
    privacy: Privacy,
    /// The `fn` and position of the next site, when it is a parameter.
    slot: Option<Slot>,
}

impl<'a> FileVisit<'a> {
    fn new(krate: &'a str, relative: &Path, path: &Path, module: Vec<String>) -> Self {
        let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
        // A crate root and a `mod.rs` hold their children beside them; any
        // other module file holds them in the directory of its own name.
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        let mod_rs = matches!(
            name.as_deref(),
            Some("mod.rs" | "lib.rs" | "main.rs" | "build.rs")
        ) || parent.ends_with("src/bin");
        let dir = if mod_rs {
            parent.clone()
        } else {
            path.with_extension("")
        };
        Self {
            krate,
            file: relative.to_string_lossy().replace('\\', "/"),
            module,
            scope: Vec::new(),
            dir,
            file_dir: Some(parent),
            keys: Vec::new(),
            children: Vec::new(),
            skipped: Skipped::default(),
            host: None,
            owner: None,
            traits: BTreeSet::new(),
            types: Vec::new(),
            derefs: Vec::new(),
            privacy: Privacy::Private,
            slot: None,
        }
    }

    fn item(&self, name: &str) -> String {
        if self.scope.is_empty() {
            name.to_owned()
        } else {
            format!("{}::{name}", self.scope.join("::"))
        }
    }

    fn emit(&mut self, item: String, kind: SiteKind, ty: &syn::Type) {
        let text = normalise(&ty.to_token_stream().to_string());
        if !holds_primitive(&text) {
            return;
        }
        let mut facts = TypeFacts::default();
        facts.visit_type(ty);
        let place = Place {
            krate: self.krate.to_owned(),
            module: self.module.join("::"),
            kind,
            owner: self.owner.clone(),
            host: self.host.clone(),
            form: facts.form,
            roots: facts.roots,
            privacy: self.privacy,
            shape: if text == "bool" {
                Shape::Bool
            } else if text
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .any(|word| word == "bool")
            {
                Shape::HoldsBool
            } else {
                Shape::Other
            },
            slot: self.slot.clone(),
        };
        let ty = text;
        let sep = SiteKey::SEPARATOR;
        self.keys.push(RawKey {
            head: format!(
                "{}{sep}{}{sep}{}",
                self.krate,
                self.file,
                self.module.join("::")
            ),
            item,
            tail: format!("{}{sep}{ty}", kind.as_str()),
            place,
        });
    }

    fn signature(&mut self, sig: &syn::Signature, privacy: Privacy) {
        let name = sig.ident.to_string();
        let receiver = if sig.receiver().is_some() {
            Receiver::With
        } else {
            Receiver::Without
        };
        self.privacy = privacy;
        let typed = sig.inputs.iter().filter_map(|input| match input {
            syn::FnArg::Typed(arg) => Some(arg),
            syn::FnArg::Receiver(_) => None,
        });
        for (i, arg) in typed.enumerate() {
            self.slot = Some(Slot {
                name: name.clone(),
                position: ArgPosition(i),
                receiver,
            });
            let param = match &*arg.pat {
                syn::Pat::Ident(ident) => ident.ident.to_string(),
                other => normalise(&other.to_token_stream().to_string()),
            };
            self.emit(
                self.item(&format!("{name}({param})")),
                SiteKind::Param,
                &arg.ty,
            );
        }
        self.slot = None;
        if let syn::ReturnType::Type(_, ty) = &sig.output {
            self.emit(self.item(&format!("{name}()")), SiteKind::Ret, ty);
        }
    }

    /// The fields of `owner`. `base` is the privacy of an enum, which its
    /// variant fields share; `holder` is what the owner derives.
    fn fields<'f>(
        &mut self,
        owner: &str,
        fields: impl IntoIterator<Item = &'f syn::Field>,
        base: Privacy,
        holder: Privacy,
    ) {
        let holder = if self.krate == "ambre-bus" {
            Privacy::BusType
        } else {
            holder
        };
        for (i, field) in fields.into_iter().enumerate() {
            if test_only(&field.attrs) {
                continue;
            }
            self.privacy = [privacy(&field.vis), base, holder]
                .into_iter()
                .find(|reach| *reach != Privacy::Private)
                .unwrap_or_default();
            let name = field
                .ident
                .as_ref()
                .map_or_else(|| i.to_string(), ToString::to_string);
            self.emit(
                self.item(&format!("{owner}.{name}")),
                SiteKind::Field,
                &field.ty,
            );
        }
    }

    /// Scans a `fn` body for the items it declares, under the `fn`'s name.
    fn body(&mut self, name: String, block: &syn::Block) {
        self.scope.push(name);
        self.visit_block(block);
        self.scope.pop();
    }

    fn declare(&mut self, name: String, decl: Decl) {
        let module = self.module.join("::");
        self.types.push((name, Declaration { module, decl }));
    }

    /// Keeps a test module's file, and every file under its directory, out of
    /// the scan and out of the unreached report.
    fn skip_module(&mut self, m: &syn::ItemMod) {
        let name = m.ident.to_string();
        if m.content.is_none() {
            if let Some(path) = path_attr(&m.attrs) {
                let base = self.file_dir.clone().unwrap_or_else(|| self.dir.clone());
                let file = base.join(path);
                self.skipped.dirs.push(file.with_extension(""));
                self.skipped.files.push(file);
            }
            self.skipped.files.push(self.dir.join(format!("{name}.rs")));
        }
        self.skipped.dirs.push(self.dir.join(&name));
    }
}

impl<'ast> Visit<'ast> for FileVisit<'_> {
    fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
        let name = m.ident.to_string();
        if test_only(&m.attrs) || name.ends_with("_tests") {
            self.skip_module(m);
            return;
        }
        let mut module = self.module.clone();
        module.push(name.clone());
        match &m.content {
            Some((_, items)) => {
                let saved = (
                    self.dir.clone(),
                    self.file_dir.take(),
                    std::mem::take(&mut self.scope),
                );
                self.dir = self.dir.join(&name);
                self.module.push(name);
                for item in items {
                    self.visit_item(item);
                }
                self.module.pop();
                (self.dir, self.file_dir, self.scope) = saved;
            }
            None => {
                let candidates = match path_attr(&m.attrs) {
                    Some(path) => {
                        vec![
                            self.file_dir
                                .clone()
                                .unwrap_or_else(|| self.dir.clone())
                                .join(path),
                        ]
                    }
                    None => {
                        vec![
                            self.dir.join(format!("{name}.rs")),
                            self.dir.join(&name).join("mod.rs"),
                        ]
                    }
                };
                self.children.push(Pending { candidates, module });
            }
        }
    }

    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        if test_only(&f.attrs) {
            return;
        }
        self.signature(&f.sig, privacy(&f.vis));
        self.body(f.sig.ident.to_string(), &f.block);
    }

    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        if test_only(&i.attrs) {
            return;
        }
        let ty = normalise(&i.self_ty.to_token_stream().to_string());
        let self_ty = type_name(&i.self_ty);
        let of_trait = i.trait_.as_ref().map(|(_, path, _)| TraitPath {
            name: path
                .segments
                .last()
                .map_or_else(String::new, |last| last.ident.to_string()),
            root: (path.segments.len() >= 2)
                .then(|| path.segments.first().map(|first| first.ident.to_string()))
                .flatten(),
        });
        let host = |receiver: Receiver, output: Output| Host {
            self_ty: self_ty.clone(),
            of_trait: of_trait.clone(),
            receiver,
            output,
        };
        let deref = match of_trait.as_ref().map(|path| path.name.as_str()) {
            Some("Deref") => Some(DerefTrait::Deref),
            Some("DerefMut") => Some(DerefTrait::DerefMut),
            _ => None,
        };
        if let Some(of) = deref {
            let target = i.items.iter().find_map(|item| match item {
                syn::ImplItem::Type(t) if t.ident == "Target" => {
                    Some(normalise(&t.ty.to_token_stream().to_string()))
                }
                _ => None,
            });
            self.derefs.push(DerefImpl {
                file: self.file.clone(),
                ty: ty.clone(),
                self_ty: self_ty.clone(),
                of,
                target: (of == DerefTrait::Deref).then_some(target).flatten(),
            });
        }
        let label = match &i.trait_ {
            Some((_, path, _)) => format!(
                "<{ty} as {}>",
                normalise(&path.to_token_stream().to_string())
            ),
            None => ty,
        };
        self.scope.push(label);
        for item in &i.items {
            match item {
                syn::ImplItem::Fn(f) if !test_only(&f.attrs) => {
                    let receiver = if f.sig.receiver().is_some() {
                        Receiver::With
                    } else {
                        Receiver::Without
                    };
                    self.host = Some(host(receiver, output(&f.sig.output, &self_ty)));
                    let reach = if of_trait.is_some() {
                        Privacy::TraitImpl
                    } else {
                        privacy(&f.vis)
                    };
                    self.signature(&f.sig, reach);
                    self.host = None;
                    self.body(f.sig.ident.to_string(), &f.block);
                }
                syn::ImplItem::Const(c) if !test_only(&c.attrs) => {
                    self.host = Some(host(Receiver::Without, Output::Other));
                    self.privacy = if of_trait.is_some() {
                        Privacy::TraitImpl
                    } else {
                        privacy(&c.vis)
                    };
                    self.emit(self.item(&c.ident.to_string()), SiteKind::Const, &c.ty);
                    self.host = None;
                }
                _ => {}
            }
        }
        self.scope.pop();
    }

    fn visit_item_trait(&mut self, t: &'ast syn::ItemTrait) {
        if test_only(&t.attrs) {
            return;
        }
        self.traits.insert(t.ident.to_string());
        self.scope.push(t.ident.to_string());
        for item in &t.items {
            if let syn::TraitItem::Fn(f) = item {
                if test_only(&f.attrs) {
                    continue;
                }
                self.signature(&f.sig, privacy(&t.vis));
                if let Some(block) = &f.default {
                    self.body(f.sig.ident.to_string(), block);
                }
            }
        }
        self.scope.pop();
    }

    fn visit_item_struct(&mut self, s: &'ast syn::ItemStruct) {
        if !test_only(&s.attrs) {
            let name = s.ident.to_string();
            // A `PhantomData` marker carries a type, never a value: it is no
            // field in the count that makes a newtype.
            let counted = s
                .fields
                .iter()
                .filter(|field| {
                    !matches!(&field.ty, syn::Type::Path(path)
                        if path.path.segments.last().is_some_and(|last| last.ident == "PhantomData"))
                })
                .count();
            let decl = if counted == 1 {
                Decl::Newtype
            } else {
                Decl::Struct
            };
            self.declare(name.clone(), decl);
            self.owner = Some(name.clone());
            self.fields(&name, &s.fields, Privacy::Private, derived(&s.attrs));
            self.owner = None;
        }
    }

    fn visit_item_union(&mut self, u: &'ast syn::ItemUnion) {
        if !test_only(&u.attrs) {
            self.fields(
                &u.ident.to_string(),
                &u.fields.named,
                Privacy::Private,
                derived(&u.attrs),
            );
        }
    }

    fn visit_item_enum(&mut self, e: &'ast syn::ItemEnum) {
        if test_only(&e.attrs) {
            return;
        }
        self.declare(e.ident.to_string(), Decl::Enum);
        for variant in e
            .variants
            .iter()
            .filter(|variant| !test_only(&variant.attrs))
        {
            self.fields(
                &format!("{}::{}", e.ident, variant.ident),
                &variant.fields,
                privacy(&e.vis),
                derived(&e.attrs),
            );
        }
    }

    fn visit_item_const(&mut self, c: &'ast syn::ItemConst) {
        if !test_only(&c.attrs) {
            self.privacy = privacy(&c.vis);
            self.emit(self.item(&c.ident.to_string()), SiteKind::Const, &c.ty);
        }
    }

    fn visit_item_static(&mut self, s: &'ast syn::ItemStatic) {
        if !test_only(&s.attrs) {
            self.privacy = privacy(&s.vis);
            self.emit(self.item(&s.ident.to_string()), SiteKind::Static, &s.ty);
        }
    }

    fn visit_item_type(&mut self, t: &'ast syn::ItemType) {
        if !test_only(&t.attrs) {
            self.privacy = privacy(&t.vis);
            self.emit(self.item(&t.ident.to_string()), SiteKind::Alias, &t.ty);
        }
    }

    // A macro's tokens are not items until it expands, and the scan expands
    // nothing: what a macro writes is out of reach by construction. A
    // `define_*!` macro of the workspace writes newtypes, so the scan notes
    // their names for the `newtype-inner` check.
    fn visit_item_macro(&mut self, m: &'ast syn::ItemMacro) {
        if test_only(&m.attrs) {
            return;
        }
        let defines = m
            .mac
            .path
            .segments
            .last()
            .is_some_and(|last| last.ident.to_string().starts_with("define_"));
        if defines {
            for name in macro_types(&m.mac) {
                self.declare(name, Decl::Newtype);
            }
        }
    }

    // A foreign block declares the foreign API itself.
    fn visit_item_foreign_mod(&mut self, _: &'ast syn::ItemForeignMod) {}
}

#[cfg(test)]
#[path = "scan_tests.rs"]
mod tests;
