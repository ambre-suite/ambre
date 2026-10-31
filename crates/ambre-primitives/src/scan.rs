//! The scan: every site of the workspace whose type holds a primitive.
//!
//! The scan walks the module tree of each crate from its roots, the way rustc
//! does, rather than every file on disk. A module declared under a test gate is
//! then never opened, wherever its file sits. The walk records each file it
//! reads per crate root. The dep-info check holds that record to the files
//! rustc reads, since the gate cannot hold code it cannot place (`ARCH-033`).
//!
//! syn parses and never expands. An item that a macro writes, and the output
//! of a build script, are therefore outside the scan by construction. A
//! `mod` a macro holds fails the scan, since the walk could not follow it.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ffi::OsStr;
use std::fmt;
use std::path::{Component, Path, PathBuf};

use proc_macro2::{Delimiter, Group, TokenStream, TokenTree};
use quote::ToTokens;
use syn::ext::IdentExt;
use syn::parse::Parser;
use syn::visit::Visit;
use syn::visit_mut::VisitMut;

use crate::allowlist::LineNo;

/// The types the rule names: a bare one of these needs a newtype or a reason
/// (`ai/ambre-rust-style.md`).
pub(crate) const PRIMITIVES: &[&str] = &[
    "String", "str", "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128",
    "isize", "f32", "f64", "bool", "char", "Uuid",
];

/// The attribute macros that turn a `fn` into a test.
const TEST_MACROS: &[&str] = &["test", "wasm_bindgen_test"];

/// The crates of the toolchain. A closure that names only their types is
/// no evidence that a foreign signature dictates it.
const TOOLCHAIN: &[&str] = &["std", "core", "alloc"];

/// The first segments of a path that stay inside the crate that writes it.
const LOCAL_ROOTS: &[&str] = &["crate", "self", "super", "Self"];

/// The owning wrappers a dereference target exposes its element through:
/// a `Box<str>` hands out a `str` as surely as a `str` target does.
const WRAPPERS: &[&str] = &["Vec", "Box", "Rc", "Arc", "Cow"];

/// The prefix of each word the gate writes in place of a source name. It
/// replaces a metavariable once the body of a `macro_rules!` is read as
/// Rust, and a parameter of an alias while its target resolves. No source
/// identifier starts with it.
pub(crate) const META: &str = "__ambre_meta_";

/// How deep one alias may lead to another. A cycle fails to compile, so the
/// bound keeps a broken tree from looping. Reaching it fails the scan, since
/// the type past it stays unread.
const ALIAS_DEPTH: Hops = Hops(32);

/// How many aliases one type may open, nested ones included. An alias that
/// names another twice doubles the walk at each level, so the bound keeps
/// the walk finite. Reaching it fails the scan.
const ALIAS_EXPANSIONS: AliasCount = AliasCount(4096);

/// How many `use` items and modules one path may lead through before the
/// lookup stops. A loop of imports fails to compile, so reaching it fails
/// the scan.
const RESOLVE_HOPS: Hops = Hops(16);

/// How many `use` items, modules or aliases one walk has led through,
/// checked against [`RESOLVE_HOPS`] or [`ALIAS_DEPTH`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Hops(usize);

impl Hops {
    /// One step further.
    fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

impl fmt::Display for Hops {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// How many aliases one type has opened, checked against
/// [`ALIAS_EXPANSIONS`]. Each opening takes the next count, so the count
/// also names the opening.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct AliasCount(usize);

impl AliasCount {
    /// The count once one more alias opens.
    fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

impl fmt::Display for AliasCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

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
    /// An associated type an impl writes: a marker hands a generic newtype
    /// its representation there.
    Assoc,
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
            Self::Assoc => "assoc",
        }
    }
}

/// The source tree the scan reads. The disk in the gate, a map in the tests.
pub trait Tree {
    /// The text of one file, or `None` when the file does not exist.
    fn read(&self, path: &Path) -> Option<String>;

    /// Every file under `dir`, at any depth, sorted. The build output of the
    /// workspace is left out.
    fn files(&self, dir: &Path) -> Vec<PathBuf>;

    /// The path with every symbolic link followed and every `..` undone: the
    /// file rustc opens. A path that does not resolve keeps its lexical form.
    fn canonical(&self, path: &Path) -> PathBuf;
}

/// The tree on disk, under one workspace root.
pub struct Disk {
    /// The directories the listing leaves out, under the root.
    skipped: Vec<PathBuf>,
}

/// The directories of the workspace root that a listing leaves out. They
/// are the history of git, and the build output of the workspace and of
/// each nested workspace. A directory of the same name elsewhere is listed,
/// since no build writes there.
const SKIPPED: &[&str] = &[".git", "target"];

/// The nested workspaces under the root, which build apart, as the fuzz
/// targets do under `cargo-fuzz`. The orphan check leaves each out, and a
/// listing leaves out its build output.
pub(crate) const NESTED_WORKSPACES: &[&str] = &["fuzz"];

impl Disk {
    /// The tree on disk under the workspace root `root`.
    pub fn at(root: &Path) -> Self {
        let nested = NESTED_WORKSPACES
            .iter()
            .map(|dir| root.join(dir).join("target"));
        Self {
            skipped: SKIPPED
                .iter()
                .map(|dir| root.join(dir))
                .chain(nested)
                .collect(),
        }
    }
}

impl Tree for Disk {
    fn read(&self, path: &Path) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }

    // A link to a directory is not followed, so a loop of links cannot hold
    // the listing. The files under the root are listed where they sit.
    fn files(&self, dir: &Path) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let mut pending = vec![dir.to_path_buf()];
        while let Some(next) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(&next) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                if kind.is_dir() {
                    if !self.skipped.contains(&path) {
                        pending.push(path);
                    }
                } else if !(kind.is_symlink() && path.is_dir()) {
                    found.push(path);
                }
            }
        }
        found.sort();
        found
    }

    fn canonical(&self, path: &Path) -> PathBuf {
        canonical(path)
    }
}

/// The path with every symbolic link followed and every `..` undone, or its
/// lexical form when it does not resolve.
pub(crate) fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| lexical(path))
}

/// Whether a directory of this name is build output. A crate built alone
/// writes its own `target/`, so a `#[path]` into one fails the scan.
fn is_build_output(name: &OsStr) -> bool {
    name == "target"
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
    /// Each name of a secret reader or of the `zeroize` crate outside the
    /// code the secret checks admit (`ARCH-045`, `ARCH-046`).
    pub secrets: Vec<String>,
    /// Each crate root the walk starts from, and every file it reads from
    /// that root outside test code, both canonical. The dep-info check holds
    /// it to the files rustc reads (`ARCH-033`).
    pub walked: BTreeMap<PathBuf, BTreeSet<PathBuf>>,
    /// Each crate root the walk starts from, and every file that a walked
    /// file names as data outside test code, both canonical. Data is the
    /// target of an `include_str!` or an `include_bytes!` that a string
    /// literal names, or a file of the directory a `sqlx::migrate!` names. The dep-info check
    /// admits these reads, and no other file the walk does not read
    /// (`ARCH-033`).
    pub data: BTreeMap<PathBuf, BTreeSet<PathBuf>>,
    /// Every call of the workspace, tests included, that passes a `bool`
    /// literal (`ARCH-040`).
    literals: Vec<BoolCall>,
    /// Where each site sits, for the categories the gate refutes from the
    /// code (`ARCH-036`).
    places: BTreeMap<SiteKey, Place>,
    declared: Declared,
    /// Every file the walk reads outside test code, once each, in the
    /// order it reads them. The unwrap report reads them again.
    pub sources: Vec<Source>,
}

/// One file the walk reads outside test code. It names the crate, the path
/// in the crate, the module and the path the tree reads it at.
#[derive(Clone, Debug)]
pub struct Source {
    pub krate: String,
    pub file: String,
    /// The module path, as `crate::a::b`.
    pub module: String,
    pub path: PathBuf,
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
    /// The struct that declares the field, or the enum of its variant.
    /// `None` for a field of a union, and for every other kind.
    owner: Option<String>,
    /// The impl whose signature or constant the site is. `None` outside an
    /// impl, and in a body nested inside one.
    host: Option<Host>,
    form: TypeForm,
    /// The first segment of each path of the type with two or more.
    roots: BTreeSet<String>,
    /// Who can reach the site (`ARCH-038`).
    privacy: Privacy,
    /// Whether the type is a `bool`, or holds one outside a callable's
    /// signature. Read from the type once each alias is resolved.
    shape: Shape,
    /// Each primitive the type names once its aliases are resolved.
    primitives: BTreeSet<String>,
    /// The `fn` and the position of a parameter. `None` for every other
    /// kind.
    slot: Option<Slot>,
    /// What a field is to its owner.
    role: FieldRole,
}

/// What a field is to the type that declares it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum FieldRole {
    /// A field of a struct or a union, a named field of a variant, or one of
    /// two or more fields of a tuple variant. Every other kind of site too.
    #[default]
    Member,
    /// The one field of a tuple variant. The variant names the value, as
    /// `CellValue::Boolean(bool)` does, so that `bool` selects no mode.
    VariantValue,
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
    /// The name of the parameter, the name a component prop writes.
    param: String,
    kind: FnKind,
}

/// Whether a `fn` is a Leptos component, which `view!` calls by props.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FnKind {
    Plain,
    /// A `fn` under `#[component]`: `<Name prop=value/>` writes its
    /// parameters by name.
    Component,
}

/// How a call names its `fn`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CallForm {
    /// `g(..)` or `Type::g(..)`: a method's receiver, if any, is the first
    /// argument.
    Path,
    /// `x.g(..)`: the receiver sits before the dot.
    Method,
    /// `<Name prop=true/>` inside `view!`: the prop names the parameter.
    Prop,
}

/// One `bool` literal passed to a call, anywhere in a crate, tests included.
#[derive(Clone, Debug)]
struct BoolCall {
    /// The crate directory of the call.
    krate: String,
    /// The file of the call, from the workspace root.
    file: String,
    line: LineNo,
    /// The last segment of the called path, the method name, or the
    /// component a `view!` tag names.
    name: String,
    position: ArgPosition,
    form: CallForm,
    /// The prop a `view!` tag writes, `None` for a call.
    prop: Option<String>,
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

/// What a type declaration is, as a newtype claim reads it. A newtype and a
/// fieldless enum have a codec of their own; the one field of a tuple
/// variant is the representation of its variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Decl {
    /// A struct with exactly one field, or a type a `define_*!` macro
    /// writes.
    Newtype,
    /// A struct with no field, or with two or more.
    Struct,
    /// An enum whose every variant is a unit variant.
    FieldlessEnum,
    /// An enum with a variant that holds a field.
    DataEnum,
}

/// What part of a type's own representation a `newtype-inner` site claims.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Inside {
    /// A field of a struct, or a field of a variant that is not the one
    /// field of a tuple variant.
    Field,
    /// The one field of a tuple variant.
    VariantValue,
    /// A parameter of a constructor, or the return of a method.
    Codec,
}

impl Decl {
    /// Whether a site of `inside` is the own representation of this type.
    fn holds(self, inside: Inside) -> bool {
        matches!(
            (self, inside),
            (Self::Newtype, Inside::Field | Inside::Codec)
                | (Self::FieldlessEnum, Inside::Codec)
                | (Self::DataEnum, Inside::VariantValue)
        )
    }

    /// Why a site of `inside` is not the own representation of `name`.
    fn refusal(self, name: &str, inside: Inside) -> String {
        match (self, inside) {
            (Self::Struct, _) => format!("{name} is a struct with no field, or with two or more"),
            (Self::DataEnum, Inside::Codec) => format!(
                "{name} is an enum that carries data, not a struct with one field or a \
                 fieldless enum"
            ),
            (Self::DataEnum, _) => format!(
                "a field of a variant of {name} with a named field or two or more fields, not \
                 the one field of a tuple variant"
            ),
            (Self::FieldlessEnum, _) => format!("{name} is a fieldless enum, which holds no field"),
            (Self::Newtype, _) => format!("{name} is a struct, not an enum with a tuple variant"),
        }
    }
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
    /// The crate directory.
    krate: String,
    /// The module path, where an alias in the target resolves.
    module: String,
    /// The file, relative to the crate directory.
    file: String,
    /// The implementing type, as the impl writes it.
    ty: String,
    /// The last segment of the implementing type, the name `DerefMut` is
    /// matched by.
    self_ty: String,
    /// Each type parameter of the impl, with the text of its bounds, the
    /// `where` clause included.
    bounds: BTreeMap<String, String>,
    of: DerefTrait,
    /// The `Target` of a `Deref`, normalised. `None` for a `DerefMut`,
    /// whose target its `Deref` states, and for a `Deref` that writes none.
    target: Option<String>,
}

/// What a dereference target hands out, as `ARCH-037` reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Exposure {
    /// A primitive, bare or through a slice, an array, a reference or a
    /// [`WRAPPERS`] type.
    Primitive(String),
    /// A type parameter of the impl, which an instance may make a
    /// primitive.
    Generic(String),
    /// A target the gate cannot read, which fails closed: why.
    Unsure(String),
    /// Any other type.
    Opaque,
}

/// The impl item that holds a site.
#[derive(Clone, Debug)]
struct Host {
    /// The last segment of the implementing type, without its generics.
    self_ty: String,
    /// The implementing type as the impl writes it, normalised. A primitive
    /// it names is the workspace's choice, as a generic argument is.
    self_full: String,
    /// The trait the impl implements, `None` for an inherent impl.
    of_trait: Option<TraitPath>,
    /// `Without` for a constant.
    receiver: Receiver,
    /// `Other` for a constant.
    output: Output,
    /// Whether the impl sits in a `fn` body, where a local item may shadow
    /// the trait the module declares.
    in_body: bool,
    /// Whether a segment of the trait path carries arguments, as
    /// `Kind<u8>`, or the path opens with `::`. No generic newtype reads
    /// such a trait ([`representation_slots`]).
    trait_unplain: bool,
}

/// The trait of an impl, as the impl writes its path once a `use ... as`
/// rename of the file is undone.
#[derive(Clone, Debug)]
struct TraitPath {
    /// The last segment, without its generics.
    name: String,
    /// The first segment, when the path has two or more.
    root: Option<String>,
    /// The type arguments the impl writes on the last segment, as text. The
    /// workspace chose each one, not the trait.
    args: Vec<String>,
    /// Every segment, without generics, once a rename is undone.
    segments: Vec<String>,
}

/// One `type` alias, as a use of its name resolves it.
#[derive(Clone, Debug)]
struct Alias {
    /// The module path that declares it.
    module: String,
    /// Its generic parameters, in order.
    params: Vec<Param>,
    /// The aliased type, normalised.
    target: String,
}

/// One generic parameter of an alias.
#[derive(Clone, Debug)]
struct Param {
    name: String,
    kind: ParamKind,
    /// The default a type parameter writes, normalised. A use that writes
    /// no argument for the parameter takes it.
    default: Option<String>,
}

/// What a generic parameter of an alias stands for. An argument of a use
/// takes the place of a parameter of the same kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ParamKind {
    Lifetime,
    Type,
    Const,
}

/// Whether a `use` gives the item it imports a new name with `as`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImportName {
    /// `use a::B;` brings `B` in as `B`.
    Kept,
    /// `use a::B as C;` brings `B` in as `C`.
    Renamed,
}

/// A crate directory, a module and the name of an item it declares.
type Home = (String, String, String);

/// A [`Home`] and the name of one of its associated types.
type Home4 = (String, String, String, String);

/// One name a `use` item brings into a module.
#[derive(Clone, Debug)]
struct Import {
    /// The path the `use` writes, read from the module that holds it.
    path: Vec<String>,
    name: ImportName,
}

/// What the last segment of a path names, as the alias lookup reads it.
enum Named {
    /// A workspace alias, with the crate directory that declares it.
    Alias(String, Alias),
    /// An item of a foreign crate, by its whole path once every `use` of the
    /// workspace on the way is undone. A `Flag` that a `use ... as` renames
    /// reads as `std::primitive::bool`.
    Foreign(Vec<String>),
    /// Anything else: a struct, an enum, a trait, a module, a generic
    /// parameter.
    Other,
}

/// Whether a lookup of one segment searches the glob `use` items of its
/// module once the module declares and imports no such name.
#[derive(Clone, Copy)]
enum Globs {
    /// The first segment of a path: a crate name comes before a glob.
    Skip,
    /// A later segment, or a name a glob brings in.
    Follow,
}

/// What one segment of a module path leads to.
enum Step {
    /// A module of the workspace: the crate directory and the module path.
    Module(String, String),
    /// A place outside the workspace's modules: a foreign crate, a type
    /// whose associated item the path names, or a generic parameter.
    Outside,
    /// Nothing the module declares or imports.
    Absent,
}

/// Where the glob `use` items of a module lead one name.
#[derive(Clone, Debug)]
enum Through {
    /// The first module of the workspace, in breadth order, that declares
    /// or imports the name: the crate directory and the module path.
    Module(String, String),
    /// No module of the workspace, while a glob of a foreign crate may
    /// bring the name in.
    Foreign,
    /// No glob brings it in.
    Absent,
}

/// What the workspace declares, across every crate the scan reads.
#[derive(Debug, Default)]
struct Declared {
    /// The crate names, as a path writes them.
    crates: BTreeSet<String>,
    /// The name of every trait, per crate directory.
    traits: BTreeMap<String, BTreeSet<String>>,
    /// The packages each crate directory depends on in its manifest, dev and
    /// build ones included. A renamed dependency counts under its package.
    deps: BTreeMap<String, BTreeSet<String>>,
    /// Per crate directory, each dependency the manifest renames: the name a
    /// path writes, then the package.
    renamed: BTreeMap<String, BTreeMap<String, String>>,
    /// Per crate directory, the first segment of each path a `pub use`
    /// re-exports.
    reexports: BTreeMap<String, BTreeSet<String>>,
    /// Per crate directory, the crates whose `pub` items its code can
    /// call: its dependencies, and the crates they re-export. Written once
    /// every crate is read.
    reach: BTreeMap<String, BTreeSet<String>>,
    /// Every struct and enum, per crate directory, then per name. A name
    /// declared twice in one crate keeps both.
    types: BTreeMap<String, BTreeMap<String, Vec<Declaration>>>,
    /// Each associated type a generic newtype holds as its one field, as
    /// the struct writes it. That is the crate directory, the module, the
    /// trait path and the name. Placed once every crate is read.
    held: Vec<(String, String, Vec<String>, String)>,
    /// Each associated type of [`Declared::held`] whose trait places in the
    /// workspace. That is the crate directory and module of the trait, its
    /// name, and the name of the associated type.
    representations: BTreeSet<Home4>,
    /// Every `type` alias, per crate directory, then per name.
    aliases: BTreeMap<String, BTreeMap<String, Vec<Alias>>>,
    /// The name of every alias, in any crate: a type that names none needs
    /// no resolution.
    alias_names: BTreeSet<String>,
    /// Per crate directory, the path of every module, `crate` included.
    modules: BTreeMap<String, BTreeSet<String>>,
    /// Per crate directory, every struct, enum, union and trait, and every
    /// type a `define_*!` macro writes, as `module::Name`: a name that is no
    /// alias.
    items: BTreeMap<String, BTreeSet<String>>,
    /// Per crate directory, then per module, each name a `use` brings in.
    imports: BTreeMap<String, BTreeMap<String, BTreeMap<String, Import>>>,
    /// Per crate directory, then per module, the path of each glob `use`.
    globs: BTreeMap<String, BTreeMap<String, Vec<Vec<String>>>>,
    /// The new name of every `use ... as`, in any crate: a type that writes
    /// one may hide an alias or a primitive under it.
    renames: BTreeSet<String>,
    /// Per crate directory, each `use ... as` that is not private: the new
    /// name, then the last segment of each path it renames. A call of the
    /// new name, from any module, calls that `fn`.
    exported: BTreeMap<String, BTreeMap<String, BTreeSet<String>>>,
    /// What each glob search found, per crate directory, module path and
    /// name. `None` while the search runs, so a search that leads back to
    /// itself is caught. Two lookups that share a step read it once.
    searched: RefCell<BTreeMap<(String, String, String), Option<Through>>>,
}

/// The module above `module`. The crate root has none, and stays itself.
fn parent(module: &str) -> String {
    module
        .rsplit_once("::")
        .map_or_else(|| module.to_owned(), |(above, _)| above.to_owned())
}

impl Declared {
    /// Whether a path that starts with `root` names an item of the
    /// workspace. A dependency the manifest renames counts under its new
    /// name.
    fn owns_root(&self, root: &str) -> bool {
        LOCAL_ROOTS.contains(&root)
            || self.crates.contains(root)
            || self.renamed.values().any(|names| {
                names
                    .get(root)
                    .is_some_and(|package| self.crates.contains(&package.replace('-', "_")))
            })
    }

    /// The crate directory a path root names from `krate`: the crate itself
    /// for a local root, a workspace crate, or a renamed one. `None` for a
    /// foreign crate.
    fn crate_of(&self, krate: &str, root: &str) -> Option<String> {
        if LOCAL_ROOTS.contains(&root) {
            return Some(krate.to_owned());
        }
        if let Some(package) = self.renamed.get(krate).and_then(|names| names.get(root)) {
            return Some(package.clone());
        }
        self.types
            .keys()
            .chain(self.deps.keys())
            .find(|dir| dir.replace('-', "_") == root)
            .cloned()
    }

    /// Whether `root` names a crate outside the workspace from `krate`: one
    /// of the toolchain, or a dependency its manifest declares.
    fn foreign_crate(&self, krate: &str, root: &str) -> bool {
        TOOLCHAIN.contains(&root)
            || root == "proc_macro"
            || self
                .deps
                .get(krate)
                .is_some_and(|deps| deps.iter().any(|dep| dep.replace('-', "_") == root))
            || self
                .renamed
                .get(krate)
                .is_some_and(|names| names.contains_key(root))
    }

    /// Writes [`Declared::reach`]: the dependencies of each crate, then
    /// every crate a reached crate re-exports, until none is added.
    fn close_reach(&mut self) {
        let mut reach = BTreeMap::new();
        for krate in self.deps.keys() {
            let mut found: BTreeSet<String> = BTreeSet::new();
            let mut pending: Vec<String> = self
                .deps
                .get(krate)
                .into_iter()
                .flatten()
                .cloned()
                .collect();
            while let Some(next) = pending.pop() {
                if !found.insert(next.clone()) {
                    continue;
                }
                let roots = self.reexports.get(&next).into_iter().flatten();
                pending.extend(roots.filter_map(|root| self.crate_of(&next, root)));
            }
            found.remove(krate);
            reach.insert(krate.clone(), found);
        }
        self.reach = reach;
    }

    /// The name `name` that a `use` of `module` brings in, if one does.
    fn import(&self, krate: &str, module: &str, name: &str) -> Option<&Import> {
        self.imports.get(krate)?.get(module)?.get(name)
    }

    /// The path of each glob `use` of `module`.
    fn globs_of(&self, krate: &str, module: &str) -> &[Vec<String>] {
        self.globs
            .get(krate)
            .and_then(|modules| modules.get(module))
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// Whether `krate` declares the module `path`.
    fn has_module(&self, krate: &str, path: &str) -> bool {
        self.modules
            .get(krate)
            .is_some_and(|modules| modules.contains(path))
    }

    /// Whether `module` of `krate` declares a type of that name that is no
    /// alias.
    fn has_item(&self, krate: &str, module: &str, name: &str) -> bool {
        self.items
            .get(krate)
            .is_some_and(|items| items.contains(&format!("{module}::{name}")))
    }

    /// The alias `module` of `krate` declares under `name`, if any.
    fn alias_at(&self, krate: &str, module: &str, name: &str) -> Option<&Alias> {
        self.aliases
            .get(krate)?
            .get(name)?
            .iter()
            .find(|alias| alias.module == module)
    }

    /// Where the segments `prefix` of a path lead from `module` of `krate`.
    /// They are read in rustc's order: `crate`, `self` and `super`, a child
    /// module, a `use`, a crate, then a glob `use`. `Ok(None)` when they
    /// leave the workspace's modules.
    ///
    /// # Errors
    ///
    /// A segment the gate cannot place, or imports that loop.
    fn place(
        &self,
        krate: &str,
        module: &str,
        prefix: &[String],
        hops: Hops,
    ) -> Result<Option<(String, String)>, String> {
        if hops > RESOLVE_HOPS {
            return Err(format!(
                "the path {} leads through more than {RESOLVE_HOPS} imports",
                prefix.join("::")
            ));
        }
        let Some((first, rest)) = prefix.split_first() else {
            return Ok(Some((krate.to_owned(), module.to_owned())));
        };
        let start = match first.as_str() {
            "crate" => Step::Module(krate.to_owned(), "crate".to_owned()),
            "self" => Step::Module(krate.to_owned(), module.to_owned()),
            "super" => Step::Module(krate.to_owned(), parent(module)),
            "Self" => Step::Outside,
            _ => match self.child(krate, module, first, hops, Globs::Skip)? {
                Step::Absent => match self.crate_of(krate, first) {
                    Some(dir) => Step::Module(dir, "crate".to_owned()),
                    None if self.foreign_crate(krate, first) => Step::Outside,
                    None => match self.glob_child(krate, module, first, hops)? {
                        Step::Absent if first.starts_with(|c: char| c.is_ascii_uppercase()) => {
                            Step::Outside
                        }
                        Step::Absent => {
                            return Err(format!(
                                "{first} is no module, import or crate that {module} of {krate} \
                                 declares"
                            ));
                        }
                        step => step,
                    },
                },
                step => step,
            },
        };
        let Step::Module(mut at_crate, mut at) = start else {
            return Ok(None);
        };
        for segment in rest {
            if segment == "super" {
                at = parent(&at);
                continue;
            }
            match self.child(&at_crate, &at, segment, hops, Globs::Follow)? {
                Step::Module(next_crate, next) => (at_crate, at) = (next_crate, next),
                Step::Outside => return Ok(None),
                Step::Absent => {
                    return Err(format!(
                        "{at} of {at_crate} declares or imports no module {segment}"
                    ));
                }
            }
        }
        Ok(Some((at_crate, at)))
    }

    /// Where the segment `name` leads from `module` of `krate`. It leads to
    /// a child module or to a module a `use` imports. Under
    /// [`Globs::Follow`] it may lead to one a glob `use` brings in. A type
    /// of that name, or a foreign glob, leads outside.
    fn child(
        &self,
        krate: &str,
        module: &str,
        name: &str,
        hops: Hops,
        globs: Globs,
    ) -> Result<Step, String> {
        let path = format!("{module}::{name}");
        if self.has_module(krate, &path) {
            return Ok(Step::Module(krate.to_owned(), path));
        }
        // `use serde;` names the crate, not the import itself.
        if let Some(import) = self.import(krate, module, name)
            && import.path.first().map(String::as_str) != Some(name)
        {
            return Ok(
                match self.place(krate, module, &import.path, hops.next())? {
                    Some((dir, found)) => Step::Module(dir, found),
                    None => Step::Outside,
                },
            );
        }
        if self.has_item(krate, module, name) || self.alias_at(krate, module, name).is_some() {
            return Ok(Step::Outside);
        }
        match globs {
            Globs::Follow => self.glob_child(krate, module, name, hops),
            Globs::Skip => Ok(Step::Absent),
        }
    }

    /// Where the segment `name` leads through the glob `use` items of
    /// `module` of `krate` (see [`Declared::through_globs`]).
    ///
    /// # Errors
    ///
    /// A glob the gate cannot place, or a search that leads back to itself.
    fn glob_child(
        &self,
        krate: &str,
        module: &str,
        name: &str,
        hops: Hops,
    ) -> Result<Step, String> {
        Ok(match self.through_globs(krate, module, name, hops)? {
            Through::Module(dir, found) => {
                match self.child(&dir, &found, name, hops.next(), Globs::Skip)? {
                    // `use serde;` in the module the glob reaches.
                    Step::Absent => match self.crate_of(&dir, name) {
                        Some(named) => Step::Module(named, "crate".to_owned()),
                        None => Step::Outside,
                    },
                    step => step,
                }
            }
            Through::Foreign => Step::Outside,
            Through::Absent => Step::Absent,
        })
    }

    /// Whether `module` of `krate` declares or imports `name`, glob `use`
    /// items apart.
    fn declares(&self, krate: &str, module: &str, name: &str) -> bool {
        self.alias_at(krate, module, name).is_some()
            || self.has_item(krate, module, name)
            || self.has_module(krate, &format!("{module}::{name}"))
            || self.import(krate, module, name).is_some()
    }

    /// Where the glob `use` items of `module` of `krate` lead `name`: to the
    /// first module, in breadth order, that declares or imports it. A glob
    /// whose path starts with `name` brings in what `name` holds, never
    /// `name`, and is passed over. The answer is kept, so each search runs
    /// once per scan.
    ///
    /// # Errors
    ///
    /// A glob the gate cannot place, or a search that leads back to itself.
    /// The name stays unread, so the lookup fails closed.
    fn through_globs(
        &self,
        krate: &str,
        module: &str,
        name: &str,
        hops: Hops,
    ) -> Result<Through, String> {
        let key = (krate.to_owned(), module.to_owned(), name.to_owned());
        let known = self.searched.borrow().get(&key).cloned();
        match known {
            Some(Some(found)) => return Ok(found),
            Some(None) => {
                return Err(format!(
                    "the search of the glob use items of {module} of {krate} for {name} leads \
                     back to itself"
                ));
            }
            None => {}
        }
        self.searched.borrow_mut().insert(key.clone(), None);
        let found = self.search_globs(krate, module, name, hops);
        let mut searched = self.searched.borrow_mut();
        match &found {
            Ok(through) => searched.insert(key, Some(through.clone())),
            Err(_) => searched.remove(&key),
        };
        found
    }

    /// The breadth-first walk of [`Declared::through_globs`]. Each module
    /// is read once, so two modules that glob each other end it.
    fn search_globs(
        &self,
        krate: &str,
        module: &str,
        name: &str,
        hops: Hops,
    ) -> Result<Through, String> {
        let start = (krate.to_owned(), module.to_owned());
        let mut seen = BTreeSet::from([start.clone()]);
        let mut pending = VecDeque::from([start]);
        let mut foreign = false;
        while let Some((at_crate, at)) = pending.pop_front() {
            for glob in self.globs_of(&at_crate, &at) {
                if glob.first().map(String::as_str) == Some(name) {
                    continue;
                }
                match self.place(&at_crate, &at, glob, hops.next())? {
                    Some((dir, found)) => {
                        if self.declares(&dir, &found, name) {
                            return Ok(Through::Module(dir, found));
                        }
                        if seen.insert((dir.clone(), found.clone())) {
                            pending.push_back((dir, found));
                        }
                    }
                    None => foreign = true,
                }
            }
        }
        Ok(if foreign {
            Through::Foreign
        } else {
            Through::Absent
        })
    }

    /// What `name` names in `module` of `krate`. It is an alias or a type
    /// the module declares, what a `use` of it imports, or what a glob
    /// `use` brings in. `Ok(None)` when none does.
    fn member(
        &self,
        krate: &str,
        module: &str,
        name: &str,
        hops: Hops,
    ) -> Result<Option<Named>, String> {
        if let Some(named) = self.direct(krate, module, name, hops)? {
            return Ok(Some(named));
        }
        match self.through_globs(krate, module, name, hops)? {
            Through::Module(dir, found) => self.direct(&dir, &found, name, hops.next()),
            Through::Foreign => Ok(Some(Named::Other)),
            Through::Absent => Ok(None),
        }
    }

    /// What `name` names in `module` of `krate`, glob `use` items apart.
    fn direct(
        &self,
        krate: &str,
        module: &str,
        name: &str,
        hops: Hops,
    ) -> Result<Option<Named>, String> {
        if let Some(alias) = self.alias_at(krate, module, name) {
            return Ok(Some(Named::Alias(krate.to_owned(), alias.clone())));
        }
        if self.has_item(krate, module, name)
            || self.has_module(krate, &format!("{module}::{name}"))
        {
            return Ok(Some(Named::Other));
        }
        let Some(import) = self.import(krate, module, name) else {
            return Ok(None);
        };
        if import.path.len() == 1 && import.path.first().map(String::as_str) == Some(name) {
            return Ok(Some(Named::Other));
        }
        self.lookup(krate, module, &import.path, hops.next())
            .map(Some)
    }

    /// What the path `segments`, written in `module` of `krate`, names.
    /// A bare name that no item, `use` or glob of the module brings in is
    /// a generic parameter or an item of the prelude.
    ///
    /// # Errors
    ///
    /// A qualified path whose module the gate places, and that names an
    /// alias name the module neither declares nor imports. A segment it
    /// cannot place. The alias the path names stays unread, so the check
    /// fails closed.
    fn lookup(
        &self,
        krate: &str,
        module: &str,
        segments: &[String],
        hops: Hops,
    ) -> Result<Named, String> {
        if hops > RESOLVE_HOPS {
            return Err(format!(
                "the path {} leads through more than {RESOLVE_HOPS} imports",
                segments.join("::")
            ));
        }
        let Some((name, prefix)) = segments.split_last() else {
            return Ok(Named::Other);
        };
        if prefix.is_empty() {
            return Ok(self
                .member(krate, module, name, hops)?
                .unwrap_or(Named::Other));
        }
        let Some((dir, found)) = self.place(krate, module, prefix, hops)? else {
            return self.outside(krate, module, segments, hops);
        };
        match self.member(&dir, &found, name, hops)? {
            Some(named) => Ok(named),
            None if self.alias_names.contains(name) => Err(format!(
                "{} names an alias name, and {found} of {dir} declares or imports no {name}",
                segments.join("::")
            )),
            None => Ok(Named::Other),
        }
    }

    /// What a path of two or more segments names once its module leaves the
    /// workspace's modules. A first segment that a `use` of `module`
    /// imports reads as the path it imports, so `use std::primitive as p`
    /// makes `p::bool` read `std::primitive::bool`. A first segment that
    /// names a foreign crate makes the path [`Named::Foreign`].
    ///
    /// # Errors
    ///
    /// Imports that loop.
    fn outside(
        &self,
        krate: &str,
        module: &str,
        segments: &[String],
        hops: Hops,
    ) -> Result<Named, String> {
        let Some((first, rest)) = segments.split_first() else {
            return Ok(Named::Other);
        };
        if let Some(import) = self.import(krate, module, first)
            && import.path.first() != Some(first)
        {
            let full: Vec<String> = import.path.iter().chain(rest).cloned().collect();
            return self.lookup(krate, module, &full, hops.next());
        }
        let foreign = !LOCAL_ROOTS.contains(&first.as_str())
            && !self.has_module(krate, &format!("{module}::{first}"))
            && self.crate_of(krate, first).is_none()
            && self.foreign_crate(krate, first);
        Ok(if foreign {
            Named::Foreign(segments.to_vec())
        } else {
            Named::Other
        })
    }

    /// `ty` with every workspace alias it names replaced by its target, and
    /// each `use ... as` rename undone, as seen from `module` of `krate`.
    /// A one-segment path that `masked` holds names a generic parameter, or
    /// an item of an enclosing block, and stays as written.
    ///
    /// # Errors
    ///
    /// Why the gate cannot read the type whole: an alias path it cannot
    /// place, a chain past [`ALIAS_DEPTH`], or more than
    /// [`ALIAS_EXPANSIONS`] aliases opened.
    fn resolve(
        &self,
        ty: &syn::Type,
        krate: &str,
        module: &str,
        masked: &BTreeSet<String>,
    ) -> Result<syn::Type, String> {
        let mut resolved = ty.clone();
        if self.names_alias(ty) {
            let mut expansion = Expansion::default();
            Resolver {
                declared: self,
                krate: krate.to_owned(),
                module: module.to_owned(),
                masked: masked.clone(),
                depth: Hops::default(),
                expansion: &mut expansion,
            }
            .visit_type_mut(&mut resolved);
            if let Some(why) = expansion.failure {
                return Err(why);
            }
        }
        Ok(resolved)
    }

    /// Whether a type writes a word that some alias of the workspace is
    /// named, or that some `use ... as` gives as a new name.
    fn names_alias(&self, ty: &syn::Type) -> bool {
        normalise(&ty.to_token_stream().to_string())
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .any(|word| self.alias_names.contains(word) || self.renames.contains(word))
    }

    /// The primitives the workspace chose in the impl `host`: each one the
    /// generic arguments of its trait name, and each one its implementing
    /// type names. Aliases resolve from `module` of `krate`.
    ///
    /// # Errors
    ///
    /// Why an alias of those types stays unread.
    fn chosen(
        &self,
        path: &TraitPath,
        host: &Host,
        krate: &str,
        module: &str,
    ) -> Result<BTreeSet<String>, String> {
        let mut chosen = BTreeSet::new();
        for written in path.args.iter().chain(std::iter::once(&host.self_full)) {
            if let Ok(ty) = syn::parse_str::<syn::Type>(written) {
                chosen.extend(primitives_of(&self.resolve(
                    &ty,
                    krate,
                    module,
                    &BTreeSet::new(),
                )?));
            }
        }
        Ok(chosen)
    }

    /// The crate directory, the module and the name of the type or trait
    /// the path `segments` names, written in `module` of `krate`. The name
    /// is the one the item declares, so a `use ... as` rename on the way is
    /// undone. The path follows the module's items, its `use` items and its
    /// globs, as [`Declared::lookup`] does. `None` when the path leaves the
    /// workspace, names no item, or cannot be placed. A caller that grants
    /// an exemption on the answer then fails closed.
    fn type_home(
        &self,
        krate: &str,
        module: &str,
        segments: &[String],
        hops: Hops,
    ) -> Option<Home> {
        if hops > RESOLVE_HOPS {
            return None;
        }
        let (name, prefix) = segments.split_last()?;
        if !prefix.is_empty() {
            let (dir, found) = self.place(krate, module, prefix, hops).ok()??;
            return self.type_home(&dir, &found, std::slice::from_ref(name), hops.next());
        }
        if self.has_item(krate, module, name) {
            return Some((krate.to_owned(), module.to_owned(), name.clone()));
        }
        if let Some(import) = self.import(krate, module, name) {
            if import.path.len() == 1 {
                return None;
            }
            return self.type_home(krate, module, &import.path, hops.next());
        }
        match self.through_globs(krate, module, name, hops).ok()? {
            Through::Module(dir, found) => {
                self.type_home(&dir, &found, std::slice::from_ref(name), hops.next())
            }
            Through::Foreign | Through::Absent => None,
        }
    }

    /// Places the trait of each associated type a generic newtype holds
    /// ([`Declared::held`]). A trait that leaves the workspace places
    /// nowhere, and no impl can hold its associated type. Called once every
    /// crate is read.
    fn place_representations(&mut self) {
        let placed: BTreeSet<Home4> = self
            .held
            .iter()
            .filter_map(|(krate, module, path, name)| {
                let (dir, found, tr) = self.type_home(krate, module, path, Hops::default())?;
                Some((dir, found, tr, name.clone()))
            })
            .collect();
        self.representations = placed;
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

    /// Whether a site of `inside` is the own representation of `name`, a
    /// type of the crate of the site ([`Decl::holds`]). The declaration in
    /// the module of the site wins; with none there, every declaration of
    /// the crate must hold.
    ///
    /// Under [`Reach::TraitCrate`], a type the crate of the site lacks is
    /// read in the crates it depends on. Every declaration there must hold.
    fn own_representation(
        &self,
        site: &Place,
        name: &str,
        reach: Reach,
        inside: Inside,
    ) -> Result<(), String> {
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
                decls => match decls.iter().find(|decl| !decl.holds(inside)) {
                    None => Ok(()),
                    Some(decl) => Err(format!(
                        "{name} of another crate: {}",
                        decl.refusal(name, inside)
                    )),
                },
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
            Some(decls) => match decls.iter().find(|decl| !decl.holds(inside)) {
                None => Ok(()),
                Some(decl) => Err(decl.refusal(name, inside)),
            },
        }
    }
}

/// How far one resolution has gone, shared by every alias it opens.
#[derive(Debug, Default)]
struct Expansion {
    /// The aliases opened so far, bounded by [`ALIAS_EXPANSIONS`].
    opened: AliasCount,
    /// Why the resolution stopped, if it did.
    failure: Option<String>,
}

/// Replaces each workspace alias a type names with its target, the type
/// rustc reads, and each path a `use ... as` renames with the path it
/// names.
struct Resolver<'d, 'e> {
    declared: &'d Declared,
    /// The crate directory the type is written in.
    krate: String,
    /// The module path the type is written in.
    module: String,
    /// The one-segment names that stand for no item of the module. Each is
    /// a generic parameter of the enclosing item or alias, or an item of an
    /// enclosing block, and none resolves here.
    masked: BTreeSet<String>,
    /// How many aliases led here, bounded by [`ALIAS_DEPTH`].
    depth: Hops,
    expansion: &'e mut Expansion,
}

impl Resolver<'_, '_> {
    /// What the type path `ty` reads as, resolved whole. That is the target
    /// of the alias it names, or the foreign path it names under a
    /// `use ... as`. It is also the path a `use ... as` of its first segment
    /// renames. `Ok(None)` for any other type.
    fn open(&mut self, ty: &syn::Type) -> Result<Option<syn::Type>, String> {
        let syn::Type::Path(path) = ty else {
            return Ok(None);
        };
        if path.qself.is_some() {
            return Ok(None);
        }
        let segments: Vec<String> = path
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect();
        let (Some(first), Some(last)) = (segments.first(), path.path.segments.last()) else {
            return Ok(None);
        };
        let name = last.ident.to_string();
        if segments.len() == 1 && self.masked.contains(&name) {
            return Ok(None);
        }
        let renamed =
            self.declared.renames.contains(first) || self.declared.renames.contains(&name);
        if !renamed && !self.declared.alias_names.contains(&name) {
            return Ok(None);
        }
        match self
            .declared
            .lookup(&self.krate, &self.module, &segments, Hops::default())?
        {
            Named::Alias(home, alias) => {
                self.expand(&home, &alias, &name, &last.arguments).map(Some)
            }
            Named::Foreign(full) => {
                let mut opened = foreign_path(&full, &last.arguments)?;
                syn::visit_mut::visit_type_mut(self, &mut opened);
                Ok(Some(opened))
            }
            Named::Other => match self.declared.import(&self.krate, &self.module, first) {
                Some(import) if import.name == ImportName::Renamed => {
                    if self.depth >= ALIAS_DEPTH {
                        return Err(format!(
                            "the renames lead through more than {ALIAS_DEPTH} paths"
                        ));
                    }
                    let mut full = unrenamed(path, &import.path)?;
                    self.depth = self.depth.next();
                    self.visit_type_mut(&mut full);
                    self.depth = Hops(self.depth.0.saturating_sub(1));
                    Ok(Some(full))
                }
                _ => Ok(None),
            },
        }
    }

    /// The target of `alias`, declared in `home` and named `name`, for a
    /// use that writes `arguments`.
    ///
    /// Each argument resolves where the use sits. It pairs with the next
    /// parameter of its kind: a lifetime with none, a const with a const, a
    /// type with a type. A type parameter with no argument takes its
    /// default, which resolves where the alias sits. The target resolves
    /// there too, with each type parameter held by a word of its own frame.
    /// Each word then takes the value of its parameter. Two frames
    /// never swap their values, even when their parameters share a name.
    fn expand(
        &mut self,
        home: &str,
        alias: &Alias,
        name: &str,
        arguments: &syn::PathArguments,
    ) -> Result<syn::Type, String> {
        let frame = self.expansion.opened.next();
        self.expansion.opened = frame;
        if frame > ALIAS_EXPANSIONS {
            return Err(format!(
                "the type opens more than {ALIAS_EXPANSIONS} aliases, so the gate cannot read it \
                 whole"
            ));
        }
        if self.depth >= ALIAS_DEPTH {
            return Err(format!(
                "the alias {name} leads through more than {ALIAS_DEPTH} aliases, so the gate \
                 cannot read the type it names"
            ));
        }
        let mut given: VecDeque<syn::GenericArgument> = match arguments {
            syn::PathArguments::AngleBracketed(args) => args
                .args
                .iter()
                .filter(|arg| !matches!(arg, syn::GenericArgument::Lifetime(_)))
                .cloned()
                .collect(),
            _ => VecDeque::new(),
        };
        let mut consts = BTreeSet::new();
        let mut stand_ins: BTreeMap<String, syn::Type> = BTreeMap::new();
        let mut bound: BTreeMap<String, syn::Type> = BTreeMap::new();
        for (i, param) in alias.params.iter().enumerate() {
            match param.kind {
                ParamKind::Lifetime => {}
                ParamKind::Const => {
                    given.pop_front();
                    consts.insert(param.name.clone());
                }
                ParamKind::Type => {
                    let word = format!("{META}{frame}_{i}");
                    let value = match given.pop_front() {
                        Some(syn::GenericArgument::Type(mut ty)) => {
                            self.visit_type_mut(&mut ty);
                            if let Some(why) = self.expansion.failure.take() {
                                return Err(why);
                            }
                            ty
                        }
                        Some(_) => syn::Type::Infer(syn::TypeInfer {
                            underscore_token: syn::token::Underscore::default(),
                        }),
                        None => match &param.default {
                            Some(default) => {
                                let mut ty = syn::parse_str::<syn::Type>(default).map_err(|e| {
                                    format!(
                                        "the default of {} of the alias {name} does not parse: {e}",
                                        param.name
                                    )
                                })?;
                                Substitute(&stand_ins).visit_type_mut(&mut ty);
                                self.at_home(home, alias, &consts, &mut ty)?;
                                Substitute(&bound).visit_type_mut(&mut ty);
                                ty
                            }
                            None => syn::Type::Infer(syn::TypeInfer {
                                underscore_token: syn::token::Underscore::default(),
                            }),
                        },
                    };
                    let stand_in = syn::parse_str::<syn::Type>(&word)
                        .map_err(|e| format!("the word {word} does not parse: {e}"))?;
                    stand_ins.insert(param.name.clone(), stand_in);
                    bound.insert(word, value);
                }
            }
        }
        let mut target = syn::parse_str::<syn::Type>(&alias.target)
            .map_err(|e| format!("the target of the alias {name} does not parse: {e}"))?;
        Substitute(&stand_ins).visit_type_mut(&mut target);
        self.at_home(home, alias, &consts, &mut target)?;
        Substitute(&bound).visit_type_mut(&mut target);
        Ok(target)
    }

    /// Resolves `ty` where `alias` sits, in the crate directory `home`, one
    /// alias deeper. `consts` names its const parameters.
    fn at_home(
        &mut self,
        home: &str,
        alias: &Alias,
        consts: &BTreeSet<String>,
        ty: &mut syn::Type,
    ) -> Result<(), String> {
        Resolver {
            declared: self.declared,
            krate: home.to_owned(),
            module: alias.module.clone(),
            masked: consts.clone(),
            depth: self.depth.next(),
            expansion: &mut *self.expansion,
        }
        .visit_type_mut(ty);
        match self.expansion.failure.take() {
            Some(why) => Err(why),
            None => Ok(()),
        }
    }
}

/// The foreign path `full` as a type, with the generics `arguments` its
/// use writes on the last segment. A primitive the toolchain names, as
/// `std::primitive::bool` does, reads bare.
fn foreign_path(full: &[String], arguments: &syn::PathArguments) -> Result<syn::Type, String> {
    let bare = full
        .first()
        .is_some_and(|root| TOOLCHAIN.contains(&root.as_str()))
        && full
            .last()
            .is_some_and(|last| PRIMITIVES.contains(&last.as_str()));
    let mut text = if bare {
        full.last().cloned().unwrap_or_default()
    } else {
        full.join("::")
    };
    text.push_str(&arguments.to_token_stream().to_string());
    syn::parse_str::<syn::Type>(&text)
        .map_err(|e| format!("the path {text} a use ... as names does not parse: {e}"))
}

/// The type path `path` once its first segment reads as `full`, the path a
/// `use ... as` renames. A primitive the toolchain names, as
/// `use std::primitive::u32 as Raw` does, reads bare.
fn unrenamed(path: &syn::TypePath, full: &[String]) -> Result<syn::Type, String> {
    let bare = path.path.segments.len() == 1
        && full
            .first()
            .is_some_and(|root| TOOLCHAIN.contains(&root.as_str()))
        && full
            .last()
            .is_some_and(|last| PRIMITIVES.contains(&last.as_str()));
    let mut text = if bare {
        full.last().cloned().unwrap_or_default()
    } else {
        full.join("::")
    };
    for (i, segment) in path.path.segments.iter().enumerate() {
        if i > 0 {
            text.push_str("::");
            text.push_str(&segment.ident.to_string());
        }
        text.push_str(&segment.arguments.to_token_stream().to_string());
    }
    syn::parse_str::<syn::Type>(&text)
        .map_err(|e| format!("the path {text} a use ... as renames does not parse: {e}"))
}

impl VisitMut for Resolver<'_, '_> {
    fn visit_type_mut(&mut self, ty: &mut syn::Type) {
        if self.expansion.failure.is_some() {
            return;
        }
        match self.open(ty) {
            Ok(Some(opened)) => *ty = opened,
            Ok(None) => syn::visit_mut::visit_type_mut(self, ty),
            Err(why) => self.expansion.failure = Some(why),
        }
    }
}

/// Replaces each one-segment type path the map names with its value. A
/// parameter of an alias takes the word of its frame, and a word takes the
/// value of its parameter.
struct Substitute<'m>(&'m BTreeMap<String, syn::Type>);

impl VisitMut for Substitute<'_> {
    fn visit_type_mut(&mut self, ty: &mut syn::Type) {
        if let syn::Type::Path(path) = ty
            && path.qself.is_none()
            && let Some(ident) = path.path.get_ident()
            && let Some(arg) = self.0.get(&ident.to_string())
        {
            *ty = arg.clone();
            return;
        }
        syn::visit_mut::visit_type_mut(self, ty);
    }
}

/// Each primitive a type names, as a whole word.
fn primitives_of(ty: &syn::Type) -> BTreeSet<String> {
    normalise(&ty.to_token_stream().to_string())
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|word| PRIMITIVES.contains(word))
        .map(str::to_owned)
        .collect()
}

/// How a type holds a `bool`. A `bool` in the signature of a closure or of
/// a function pointer is no value the type holds: `Box<dyn Fn(bool)>` is a
/// callable.
fn shape_of(ty: &syn::Type) -> Shape {
    let mut bare = ty;
    while let syn::Type::Paren(syn::TypeParen { elem, .. })
    | syn::Type::Group(syn::TypeGroup { elem, .. }) = bare
    {
        bare = elem;
    }
    if let syn::Type::Path(path) = bare
        && path.qself.is_none()
        && path.path.is_ident("bool")
    {
        return Shape::Bool;
    }
    let mut seek = BoolSeek(Shape::Other);
    seek.visit_type(ty);
    seek.0
}

/// Finds a `bool` a type holds outside every callable signature.
struct BoolSeek(Shape);

impl<'ast> Visit<'ast> for BoolSeek {
    fn visit_type_bare_fn(&mut self, _: &'ast syn::TypeBareFn) {}

    fn visit_path_segment(&mut self, segment: &'ast syn::PathSegment) {
        let callable = matches!(segment.arguments, syn::PathArguments::Parenthesized(_))
            && ["Fn", "FnMut", "FnOnce"]
                .iter()
                .any(|name| segment.ident == name);
        if !callable {
            syn::visit::visit_path_segment(self, segment);
        }
    }

    fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
        if path.qself.is_none() && path.path.is_ident("bool") {
            self.0 = Shape::HoldsBool;
        }
        syn::visit::visit_type_path(self, path);
    }
}

impl Scan {
    /// Checks the claim of a `newtype-inner` line (`ai/ambre-rust-style.md`):
    /// the site is the own representation of its type. It is one of four:
    ///
    /// 1. the field of a struct with exactly one field, or the one field of
    ///    a tuple variant;
    /// 2. a parameter of a `fn` that returns a struct with one field or a
    ///    fieldless enum, as `Self`, `Option<Self>` or `Result<Self, _>`;
    /// 3. the return of a method of such a type;
    /// 4. an associated type an impl of a workspace trait writes, that a
    ///    generic newtype of the workspace holds as its one field.
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
            SiteKind::Field => {
                let inside = match place.role {
                    FieldRole::VariantValue => Inside::VariantValue,
                    FieldRole::Member => Inside::Field,
                };
                match &place.owner {
                    Some(owner) => {
                        self.declared
                            .own_representation(place, owner, Reach::OwnCrate, inside)
                    }
                    None => Err("a field of a union, not of a struct or an enum".to_owned()),
                }
            }
            SiteKind::Param | SiteKind::Ret => {
                let Some(host) = &place.host else {
                    return Err("a signature outside an impl, or nested in a body".to_owned());
                };
                let reach = self.declared.reach(place, host);
                self.declared
                    .own_representation(place, &host.self_ty, reach, Inside::Codec)?;
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
            SiteKind::Assoc => {
                let Some(host) = place.host.as_ref() else {
                    return Err("an associated type outside an impl of a trait".to_owned());
                };
                let Some(path) = host.of_trait.as_ref() else {
                    return Err("an associated type outside an impl of a trait".to_owned());
                };
                if host.in_body {
                    return Err(
                        "an impl in a fn body, whose trait a local item may shadow".to_owned()
                    );
                }
                if host.trait_unplain {
                    return Err(format!(
                        "the trait {} carries generic arguments or a leading `::`, and no \
                         generic newtype reads such a trait",
                        path.name
                    ));
                }
                let Some((dir, found, trait_name)) = self.declared.type_home(
                    &place.krate,
                    &place.module,
                    &path.segments,
                    Hops::default(),
                ) else {
                    return Err(format!(
                        "the trait {} does not place on a trait of the workspace, so no \
                         generic newtype of the workspace reads its associated type",
                        path.name
                    ));
                };
                // The item of an associated type is `<Type as Trait>::Name`,
                // with a `#n` suffix when two `cfg` twins share the key.
                let name = key
                    .0
                    .split(SiteKey::SEPARATOR)
                    .nth(3)
                    .and_then(|item| item.rsplit_once("::"))
                    .map_or("", |(_, name)| name);
                // Only a `#` and digits are the suffix: `r#Repr` is a name.
                let name = name
                    .rsplit_once('#')
                    .filter(|(_, n)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
                    .map_or(name, |(bare, _)| bare);
                if self.declared.representations.contains(&(
                    dir,
                    found,
                    trait_name,
                    name.to_owned(),
                )) {
                    Ok(())
                } else {
                    Err(format!(
                        "no generic newtype of the workspace holds {}::{name} as its one field",
                        path.name
                    ))
                }
            }
            kind => Err(format!(
                "a {} is never the inside of a newtype",
                kind.as_str()
            )),
        }
    }

    /// Checks the claim of a `trait` line. The site is a signature or a
    /// constant of an impl of a trait the workspace does not declare. Its
    /// primitive is no generic argument the impl writes, and not in the
    /// type the impl implements the trait for. Or its type is a
    /// closure or a function pointer that names a type of a foreign crate,
    /// outside the toolchain.
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
        if place.kind == SiteKind::Assoc {
            // A trait declares the associated type and leaves its value to
            // the impl: the workspace chose the primitive.
            return Err(
                "an associated type, whose value the impl chooses, not the trait".to_owned(),
            );
        }
        if let Some(host) = &place.host
            && let Some(path) = &host.of_trait
        {
            if self.declared.owns_trait(path) {
                return Err(format!(
                    "the trait {} is declared in the workspace, so its signature takes types",
                    path.name
                ));
            }
            // `impl Add<u32> for Px` and `impl Add<Px> for u32`: the trait
            // takes any argument and any type, and the workspace picked the
            // primitive.
            let chosen = self
                .declared
                .chosen(path, host, &place.krate, &place.module)
                .map_err(|why| {
                    format!(
                        "the gate cannot read the types the impl chooses, and fails closed: {why}"
                    )
                })?;
            let shared: Vec<&str> = place
                .primitives
                .intersection(&chosen)
                .map(String::as_str)
                .collect();
            return if shared.is_empty() {
                Ok(())
            } else {
                Err(format!(
                    "the impl writes {} as a generic argument of {} or as the type it implements \
                     for, so the workspace chose the primitive, not the trait: the site takes a type",
                    shared.join(", "),
                    path.name
                ))
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
    /// field a foreign schema fixes. The one `bool` of a tuple variant is
    /// the value its variant names, and passes.
    ///
    /// # Errors
    ///
    /// Who else reaches the field.
    pub fn bool_field(&self, key: &SiteKey) -> Result<(), String> {
        let Some(place) = self.places.get(key) else {
            return Ok(());
        };
        let value = place.role == FieldRole::VariantValue && place.shape == Shape::Bool;
        if place.kind != SiteKind::Field
            || place.shape == Shape::Other
            || place.privacy == Privacy::Private
            || value
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
    /// its `fn` and by position; a `view!` prop matches a parameter of a
    /// component by name. A wrapped `bool` takes `Some(true)`. The call sits
    /// in the crate of the site. For a `pub` site, it may sit in a crate
    /// that reaches it: a dependency, or a crate a dependency re-exports. `listed`
    /// holds the keys of the allowlist, the other sites a call may match.
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
        if place.shape == Shape::Other || place.privacy == Privacy::Private {
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
            .filter(|other| other.shape != Shape::Other && other.slot.is_some())
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
                    .reach
                    .get(&call.krate)
                    .is_some_and(|reach| reach.contains(&place.krate)));
        let (ArgPosition(at), ArgPosition(param)) = (call.position, slot.position);
        let aligned = match (call.form, slot.receiver) {
            (CallForm::Prop, _) => {
                slot.kind == FnKind::Component && call.prop.as_deref() == Some(slot.param.as_str())
            }
            (CallForm::Method, Receiver::With) | (CallForm::Path, Receiver::Without) => at == param,
            (CallForm::Path, Receiver::With) => at == param + 1,
            (CallForm::Method, Receiver::Without) => false,
        };
        reached && aligned && self.names(place, call, &slot.name)
    }

    /// Whether the name `call` writes names the `fn` `name` of `place`. It
    /// may name it as itself, or under a `pub use ... as`. That rename sits
    /// in the crate of the `fn`, of the caller, or of a crate it reaches.
    fn names(&self, place: &Place, call: &BoolCall, name: &str) -> bool {
        if call.name == name {
            return true;
        }
        let reach = self.declared.reach.get(&call.krate);
        [&place.krate, &call.krate]
            .into_iter()
            .chain(reach.into_iter().flatten())
            .filter_map(|krate| self.declared.exported.get(krate))
            .filter_map(|renames| renames.get(&call.name))
            .any(|old| old.contains(name))
    }
}

/// What a manifest says of its dependencies.
#[derive(Debug, Default)]
pub(crate) struct Manifest {
    /// The package of each dependency, in every table whose name ends with
    /// `dependencies`, and in every `[...dependencies.name]` table.
    pub(crate) packages: BTreeSet<String>,
    /// Each dependency whose key differs from its `package`: the key as a
    /// path writes it, then the package.
    renamed: BTreeMap<String, String>,
}

/// The `package = "..."` of a manifest line, if it writes one.
fn package_of(line: &str) -> Option<String> {
    let at = line.find("package")?;
    let rest = line.get(at + "package".len()..)?.trim_start();
    let rest = rest.strip_prefix('=')?.trim_start().strip_prefix('"')?;
    rest.split_once('"').map(|(name, _)| name.to_owned())
}

/// Reads the dependencies of a manifest, a renamed one under its package.
pub(crate) fn dependencies(manifest: &str) -> Manifest {
    let mut found = Manifest::default();
    let mut table = String::new();
    // The key of a `[...dependencies.name]` table, whose `package` line may
    // come later.
    let mut open: Option<String> = None;
    fn add(found: &mut Manifest, key: &str, package: Option<String>) {
        let package = package.unwrap_or_else(|| key.to_owned());
        if package != key {
            found.renamed.insert(key.replace('-', "_"), package.clone());
        }
        found.packages.insert(package);
    }
    for line in manifest.lines().map(str::trim) {
        if let Some(header) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            table = header.trim().to_owned();
            open = table
                .split_once("dependencies.")
                .map(|(_, name)| name.trim_matches('"').to_owned());
            if let Some(key) = &open {
                add(&mut found, key, None);
            }
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        if let Some(key) = &open {
            if line.starts_with("package")
                && let Some(package) = package_of(line)
            {
                found.packages.remove(key);
                add(&mut found, key, Some(package));
            }
            continue;
        }
        if !table.ends_with("dependencies") {
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
                add(&mut found, name, package_of(line));
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
    /// Each `use ... as` of the file, per module. A call of the new name
    /// calls the last segment of the path it renames.
    renames: Renames,
    /// The module path the walk is in, from the file.
    module: Vec<String>,
}

/// The line of the `bool` literal an argument writes. The literal may sit
/// bare, in parentheses, or as the one expression of a block, an `unsafe`
/// block included. It may sit behind a `&` or a `!`, or as the one argument
/// of `Some`, the literal of an `Option<bool>`. It may be an element of a
/// tuple, of an array or of a `vec![..]` list, or the value an array
/// repeats. It may be the value a branch of an `if` or a `match` gives.
fn literal_of(arg: &syn::Expr) -> Option<LineNo> {
    match arg {
        syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Bool(literal),
            ..
        }) => Some(LineNo(literal.span.start().line)),
        syn::Expr::Paren(inner) => literal_of(&inner.expr),
        syn::Expr::Group(inner) => literal_of(&inner.expr),
        syn::Expr::Reference(inner) => literal_of(&inner.expr),
        syn::Expr::Unary(syn::ExprUnary {
            op: syn::UnOp::Not(_),
            expr,
            ..
        }) => literal_of(expr),
        syn::Expr::Block(block) => block_literal(&block.block),
        syn::Expr::Unsafe(block) => block_literal(&block.block),
        syn::Expr::If(branch) => block_literal(&branch.then_branch).or_else(|| {
            branch
                .else_branch
                .as_ref()
                .and_then(|(_, other)| literal_of(other))
        }),
        syn::Expr::Match(arms) => arms.arms.iter().find_map(|arm| literal_of(&arm.body)),
        syn::Expr::Tuple(tuple) => tuple.elems.iter().find_map(literal_of),
        syn::Expr::Array(array) => array.elems.iter().find_map(literal_of),
        syn::Expr::Repeat(repeat) => literal_of(&repeat.expr),
        syn::Expr::Macro(mac)
            if mac
                .mac
                .path
                .segments
                .last()
                .is_some_and(|last| last.ident == "vec") =>
        {
            mac.mac
                .parse_body_with(
                    syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated,
                )
                .ok()?
                .iter()
                .find_map(literal_of)
        }
        syn::Expr::Call(call) => match (&*call.func, call.args.first()) {
            (syn::Expr::Path(path), Some(only))
                if call.args.len() == 1 && path.path.is_ident("Some") =>
            {
                literal_of(only)
            }
            _ => None,
        },
        _ => None,
    }
}

/// The line of the `bool` literal a block of one expression gives.
fn block_literal(block: &syn::Block) -> Option<LineNo> {
    match block.stmts.as_slice() {
        [syn::Stmt::Expr(only, None)] => literal_of(only),
        _ => None,
    }
}

/// The line of the `bool` literal a `view!` prop gives. The value is the
/// first one to three tokens after its `=`, the longest that parses.
/// `checked=Some(true)`, `checked=!false` and `checked=(true)` each write
/// one, as `checked=true` and `checked={true}` do.
fn prop_literal(tokens: &[TokenTree]) -> Option<LineNo> {
    (1..=3).rev().find_map(|count| {
        let taken: TokenStream = tokens.iter().take(count).cloned().collect();
        if tokens.len() < count {
            return None;
        }
        syn::parse2::<syn::Expr>(taken)
            .ok()
            .and_then(|expr| literal_of(&expr))
    })
}

/// Whether a token is the punctuation `c`.
fn is_punct(token: Option<&TokenTree>, c: char) -> bool {
    matches!(token, Some(TokenTree::Punct(punct)) if punct.as_char() == c)
}

/// What a macro's tokens are, for the literal scan.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tokens {
    /// Any macro: a call is a name before a parenthesised group.
    Plain,
    /// The body of `view!`, where `<Tag prop=true/>` writes a prop too.
    View,
}

impl LiteralVisit {
    /// The `fn` a call of the one-segment path `name` calls, once a
    /// `use ... as` rename of the file is undone: `use demo::ctrl as c`
    /// makes `c` call `ctrl`.
    fn called(&self, name: &syn::Ident) -> syn::Ident {
        self.renames
            .get(&self.module)
            .and_then(|renames| renames.get(&name.to_string()))
            .and_then(|full| full.last())
            .and_then(|last| syn::parse_str::<syn::Ident>(last).ok())
            .unwrap_or_else(|| name.clone())
    }

    fn record<'e>(
        &mut self,
        form: CallForm,
        name: &syn::Ident,
        args: impl IntoIterator<Item = &'e syn::Expr>,
    ) {
        for (i, arg) in args.into_iter().enumerate() {
            if let Some(line) = literal_of(arg) {
                self.calls.push(BoolCall {
                    krate: self.krate.clone(),
                    file: self.file.clone(),
                    line,
                    name: name.to_string(),
                    position: ArgPosition(i),
                    form,
                    prop: None,
                });
            }
        }
    }

    /// Reads the tokens of a macro that do not parse as a list of
    /// expressions, `vec![x; n]` and `tokio::select!` among them. A name
    /// before a parenthesised group that parses as arguments is a call. In
    /// `view!`, `prop=true` after a `<Tag` writes the prop of a component.
    fn tokens(&mut self, stream: TokenStream, mode: Tokens) {
        let tokens: Vec<TokenTree> = stream.into_iter().collect();
        let mut tag: Option<String> = None;
        let mut taken: BTreeSet<usize> = BTreeSet::new();
        for (i, token) in tokens.iter().enumerate() {
            if taken.contains(&i) {
                continue;
            }
            let next = tokens.get(i + 1);
            match token {
                TokenTree::Ident(name) => {
                    if let Some(TokenTree::Group(group)) = next
                        && group.delimiter() == Delimiter::Parenthesis
                        && let Ok(args) = syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated
                            .parse2(group.stream())
                    {
                        let after_path = i > 0 && is_punct(tokens.get(i - 1), ':');
                        let (form, called) = if i > 0 && is_punct(tokens.get(i - 1), '.') {
                            (CallForm::Method, name.clone())
                        } else if after_path {
                            (CallForm::Path, name.clone())
                        } else {
                            (CallForm::Path, self.called(name))
                        };
                        self.record(form, &called, &args);
                        for arg in &args {
                            self.visit_expr(arg);
                        }
                        taken.insert(i + 1);
                        continue;
                    }
                    let after_colon = i > 0 && is_punct(tokens.get(i - 1), ':');
                    if mode == Tokens::View
                        && !after_colon
                        && is_punct(next, '=')
                        && !is_punct(tokens.get(i + 2), '=')
                        && let Some(component) = &tag
                        && let Some(line) = prop_literal(tokens.get(i + 2..).unwrap_or_default())
                    {
                        self.calls.push(BoolCall {
                            krate: self.krate.clone(),
                            file: self.file.clone(),
                            line,
                            name: component.clone(),
                            position: ArgPosition(0),
                            form: CallForm::Prop,
                            prop: Some(name.to_string()),
                        });
                    }
                }
                TokenTree::Punct(punct) if punct.as_char() == '<' && mode == Tokens::View => {
                    // `<a::Comp`: the tag is the last word of the path. A
                    // `</` closes a tag and opens none.
                    let mut at = i + 1;
                    tag = None;
                    while let Some(TokenTree::Ident(word)) = tokens.get(at) {
                        tag = Some(word.to_string());
                        let more =
                            is_punct(tokens.get(at + 1), ':') && is_punct(tokens.get(at + 2), ':');
                        if !more {
                            break;
                        }
                        at += 3;
                    }
                }
                TokenTree::Group(group) => self.tokens(group.stream(), mode),
                _ => {}
            }
        }
    }
}

impl<'ast> Visit<'ast> for LiteralVisit {
    fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
        self.module.push(m.ident.to_string());
        syn::visit::visit_item_mod(self, m);
        self.module.pop();
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = &*call.func
            && let Some(last) = path.path.segments.last()
        {
            let called = if path.path.segments.len() == 1 && path.qself.is_none() {
                self.called(&last.ident)
            } else {
                last.ident.clone()
            };
            self.record(CallForm::Path, &called, &call.args);
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        self.record(CallForm::Method, &call.method, &call.args);
        syn::visit::visit_expr_method_call(self, call);
    }

    // syn leaves a macro's tokens unparsed. The arguments of `assert!`,
    // `assert_eq!`, `vec!` and `format!` parse as expressions, and a test
    // writes many of its calls there. Any other body is read as tokens,
    // once each metavariable of a `macro_rules!` reads as a word, so that
    // `f($x, true)` parses as a call.
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        let view = mac
            .path
            .segments
            .last()
            .is_some_and(|last| last.ident == "view");
        if view {
            self.tokens(mac.tokens.clone(), Tokens::View);
            return;
        }
        match mac.parse_body_with(
            syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated,
        ) {
            Ok(args) => {
                for arg in &args {
                    self.visit_expr(arg);
                }
            }
            Err(_) => self.tokens(unmeta(mac.tokens.clone(), &BTreeMap::new()), Tokens::Plain),
        }
    }
}

/// Scans every crate under `crates`, a directory holding one directory per
/// crate.
pub fn scan(tree: &impl Tree, crates: &Path) -> Scan {
    let mut result = Scan::default();
    let mut found = Found::default();
    let manifests = tree
        .files(crates)
        .into_iter()
        .filter(|path| path.file_name().is_some_and(|name| name == "Cargo.toml"))
        .filter(|path| path.parent().and_then(Path::parent) == Some(crates));
    for manifest in manifests {
        let Some(dir) = manifest.parent() else {
            continue;
        };
        scan_crate(tree, dir, &mut result, &mut found);
    }
    // An alias of one crate may name a type of another. A call may reach a
    // crate through a re-export. Both are read once every crate is.
    result.declared.close_reach();
    result.declared.place_representations();
    let mut keys = Vec::new();
    for raw in found.keys {
        if let Some(key) = settle(&result.declared, raw, &mut result.errors) {
            keys.push(key);
        }
    }
    for (key, place) in dedup(keys) {
        result.sites.insert(key.clone());
        result.places.insert(key, place);
    }
    result.derefs = refused_derefs(&result.declared, &found.derefs);
    result.derefs.append(&mut found.unread);
    // A file two roots declare is walked once per root, and a refusal of
    // one of its modules would print once per root.
    let mut seen = BTreeSet::new();
    result.errors.retain(|error| seen.insert(error.clone()));
    result
}

/// What the walk of every crate gathers before an alias resolves: each
/// type a site writes, and each dereference impl.
#[derive(Default)]
struct Found {
    keys: Vec<RawKey>,
    derefs: Vec<DerefImpl>,
    /// The refusal of each `macro_rules!` body that writes a dereference
    /// impl and does not parse.
    unread: Vec<String>,
}

/// The key of a site once its type is resolved. `None` for a type that
/// holds no primitive, alias or not. A type the gate cannot resolve whole
/// keeps its written form and adds an error to `errors`: the scan fails
/// closed.
fn settle(declared: &Declared, raw: RawKey, errors: &mut Vec<String>) -> Option<RawKey> {
    let RawKey {
        head,
        item,
        tail,
        ty,
        mut place,
        shadow,
    } = raw;
    let sep = SiteKey::SEPARATOR;
    let read = match shadowed(declared, &ty, &shadow) {
        Some(why) => Err(why),
        None => declared.resolve(&ty, &place.krate, &place.module, &shadow.masked),
    };
    let resolved = match read {
        Ok(resolved) => resolved,
        Err(why) => {
            errors.push(format!(
                "{head}{sep}{item}{sep}{tail}: {why}, so the gate fails closed"
            ));
            ty.clone()
        }
    };
    if !holds_primitive(&normalise(&resolved.to_token_stream().to_string())) {
        return None;
    }
    let mut facts = TypeFacts::default();
    facts.visit_type(&resolved);
    place.form = facts.form;
    place.roots = facts.roots;
    place.shape = shape_of(&resolved);
    place.primitives = primitives_of(&resolved);
    Some(RawKey {
        head,
        item,
        tail,
        ty,
        place,
        shadow,
    })
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
pub(crate) fn macro_types(mac: &syn::Macro) -> Vec<String> {
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
    placement: Placement,
    /// The canonical crate root whose module tree holds the file.
    root: PathBuf,
}

/// How a `mod` declaration places the file of its module.
enum Placement {
    /// By the module name: `x.rs`, then `x/mod.rs`, inside the crate.
    Named,
    /// By a plain `#[path = "..."]`. The walk checks the file against the
    /// crate before it reads it.
    Path,
    /// By a `path` the gate cannot follow, so the scan fails closed.
    Unplaced(Unplaced),
}

/// Where the `path` attributes of a module declaration put its file.
enum ModPath {
    /// No `path` attribute.
    Named,
    /// One plain `#[path = "..."]`, relative to the declaring file.
    Given(PathBuf),
    /// A `path` the gate cannot follow.
    Unplaced(Unplaced),
}

/// Why the gate cannot follow the `path` of a module. rustc compiles the
/// file the attribute names, and the gate would read another one or none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Unplaced {
    /// A `path` inside a `cfg_attr`, at any depth. Which build takes it is
    /// a `cfg`, and the gate evaluates none here.
    CfgAttr,
    /// A `path` attribute that is not one string literal.
    NotLiteral,
    /// A `#[path]` on an inline module, which moves the files of the
    /// modules it declares.
    Inline,
    /// A file outside the directory of the crate.
    Outside,
    /// A file that is not `.rs`.
    NotRust,
    /// A file under a build output directory, which the walk leaves out.
    BuildOutput,
    /// A `mod` inside a block, as in a `fn` body. The gate keys items on
    /// their module path, so its items would merge with those of a module
    /// of the same path outside the block.
    InBody,
}

impl fmt::Display for Unplaced {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::CfgAttr => "a `path` inside a `cfg_attr`, which the gate does not evaluate",
            Self::NotLiteral => "a `path` attribute that is not one string literal",
            Self::Inline => "a `#[path]` on an inline module, which moves the files of its modules",
            Self::Outside => "a `#[path]` to a file outside the directory of the crate",
            Self::NotRust => "a `#[path]` to a file that is not `.rs`",
            Self::BuildOutput => "a `#[path]` to a file under a `target/` directory",
            Self::InBody => "a `mod` inside a block, whose items the gate cannot tell apart",
        })
    }
}

/// Why the file a `#[path]` names cannot be read as the module. `None`
/// when the file is `.rs`, in the crate directory `dir`, and under no
/// directory the walk leaves out. The comparison is lexical: `..` is undone
/// on the text of the path.
fn misplaced(dir: &Path, file: &Path) -> Option<Unplaced> {
    let file = lexical(file);
    let Ok(inside) = file.strip_prefix(lexical(dir)) else {
        return Some(Unplaced::Outside);
    };
    let skipped = inside
        .parent()
        .is_some_and(|parent| parent.iter().any(is_build_output));
    if skipped {
        return Some(Unplaced::BuildOutput);
    }
    if file.extension().is_none_or(|ext| ext != "rs") {
        return Some(Unplaced::NotRust);
    }
    None
}

/// The path with each `.` dropped and each `..` undone on the text, without
/// a read of the disk.
pub(crate) fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match out.components().next_back() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                _ => out.push(".."),
            },
            other => out.push(other),
        }
    }
    out
}

/// Scans one crate from its roots: the library, the binaries and the build
/// script.
fn scan_crate(tree: &impl Tree, dir: &Path, result: &mut Scan, found: &mut Found) {
    let name = dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    result.declared.crates.insert(name.replace('-', "_"));
    result
        .declared
        .modules
        .entry(name.clone())
        .or_default()
        .insert("crate".to_owned());
    if let Some(manifest) = tree.read(&dir.join("Cargo.toml")) {
        let Manifest { packages, renamed } = dependencies(&manifest);
        result
            .secrets
            .append(&mut crate::secrets::renamed_wipers(&name, &renamed));
        result.declared.deps.insert(name.clone(), packages);
        result.declared.renamed.insert(name.clone(), renamed);
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
            placement: Placement::Named,
            root: tree.canonical(path),
        })
        .collect();
    // rustc compiles a module tree per root, so a file two roots declare is
    // walked once per root. Its sites are read once.
    let mut scanned = BTreeSet::new();
    let mut walked = BTreeSet::new();
    let canonical_dir = tree.canonical(dir);
    while let Some(next) = pending.pop() {
        // rustc compiles the file a `path` names. A file the gate cannot
        // show to be that one, or cannot list, fails the scan rather than
        // let the gate read another (`ARCH-033`, `ARCH-045`).
        let refusal = match next.placement {
            Placement::Named => None,
            Placement::Path => next.candidates.iter().find_map(|file| misplaced(dir, file)),
            Placement::Unplaced(why) => Some(why),
        };
        if let Some(why) = refusal {
            let names: Vec<String> = next
                .candidates
                .iter()
                .map(|path| path.display().to_string())
                .collect();
            result.errors.push(format!(
                "crates/{name}: the module {} is placed by {why}{}, so the gate cannot show that \
                 it reads the file rustc compiles, and fails closed",
                next.module.join("::"),
                if names.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", names.join(", "))
                },
            ));
            continue;
        }
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
        if !walked.insert((next.root.clone(), path.clone())) {
            continue;
        }
        // rustc reads a file under `#![cfg(test)]` too, before the `cfg`
        // drops its items, so the walk records it as read.
        let canonical = tree.canonical(&path);
        result
            .walked
            .entry(next.root.clone())
            .or_default()
            .insert(canonical.clone());
        let first = scanned.insert(path.clone());
        let file = match syn::parse_file(&text) {
            Ok(file) => file,
            Err(e) => {
                if first {
                    result
                        .errors
                        .push(format!("{}: syn cannot parse it: {e}", path.display()));
                }
                continue;
            }
        };
        // A file under `#![cfg(test)]` is test code, and so is every module
        // it declares.
        if test_only(&file.attrs) {
            continue;
        }
        // rustc resolves the literal of an `include_str!` beside the file
        // that calls it, and `sqlx` resolves a `migrate!` under the crate
        // directory. A path built by a macro is never recorded, so the
        // dep-info check refuses the file it reads (`ARCH-033`).
        let beside = path.parent().unwrap_or(dir);
        let data = result.data.entry(next.root.clone()).or_default();
        for read in data_reads(&file) {
            match read {
                DataRead::File(name) => {
                    data.insert(tree.canonical(&beside.join(name)));
                }
                DataRead::Dir(name) => {
                    let listed = lexical(&dir.join(name));
                    data.extend(
                        tree.files(&listed)
                            .into_iter()
                            .filter(|file| file.parent().map(lexical).as_ref() == Some(&listed))
                            .map(|file| tree.canonical(&file)),
                    );
                }
            }
        }
        // The file as the text names it, `..` undone, and the file the disk
        // holds, links followed. An exemption of the secret checks holds
        // only when both forms sit in its place (`ARCH-045`, `ARCH-048`).
        let lexical_path = lexical(&path);
        let relative = lexical_path
            .strip_prefix(lexical(dir))
            .unwrap_or(&lexical_path);
        let held = canonical
            .strip_prefix(&canonical_dir)
            .ok()
            .map(|inside| inside.to_string_lossy().replace('\\', "/"));
        let root_file = next.module.len() == 1;
        let module = next.module.join("::");
        let mut visit = FileVisit::new(&name, relative, &path, next.module, &next.placement);
        visit.uses(&file);
        visit.visit_file(&file);
        pending.extend(visit.children.drain(..).map(|child| Pending {
            root: next.root.clone(),
            ..child
        }));
        if !first {
            continue;
        }
        let file_name = relative.to_string_lossy().replace('\\', "/");
        let shown = format!("crates/{name}/{file_name}");
        result.sources.push(Source {
            krate: name.clone(),
            file: file_name.clone(),
            module,
            path: path.clone(),
        });
        result.errors.append(&mut macro_mods(&shown, &file));
        if root_file && !FEATURE_ROOTS.contains(&(name.as_str(), file_name.as_str())) {
            result.errors.append(&mut features(&shown, &file));
        }
        result.secrets.append(&mut crate::secrets::refusals(
            &name,
            &[Some(file_name.as_str()), held.as_deref()],
            &shown,
            &file,
        ));
        found.keys.append(&mut visit.keys);
        found.derefs.append(&mut visit.derefs);
        found.unread.append(&mut visit.unread);
        let declared = &mut result.declared;
        declared
            .modules
            .entry(name.clone())
            .or_default()
            .append(&mut visit.modules);
        declared
            .items
            .entry(name.clone())
            .or_default()
            .append(&mut visit.items);
        for (module, new_name, import) in visit.imports.drain(..) {
            if import.name == ImportName::Renamed {
                declared.renames.insert(new_name.clone());
            }
            declared
                .imports
                .entry(name.clone())
                .or_default()
                .entry(module)
                .or_default()
                .insert(new_name, import);
        }
        for (module, glob) in visit.globs.drain(..) {
            declared
                .globs
                .entry(name.clone())
                .or_default()
                .entry(module)
                .or_default()
                .push(glob);
        }
        result
            .declared
            .reexports
            .entry(name.clone())
            .or_default()
            .append(&mut visit.reexports);
        for (new_name, old_name) in visit.exported.drain(..) {
            result
                .declared
                .exported
                .entry(name.clone())
                .or_default()
                .entry(new_name)
                .or_default()
                .insert(old_name);
        }
        for (alias, declared) in visit.aliases {
            result.declared.alias_names.insert(alias.clone());
            result
                .declared
                .aliases
                .entry(name.clone())
                .or_default()
                .entry(alias)
                .or_default()
                .push(declared);
        }
        result
            .declared
            .traits
            .entry(name.clone())
            .or_default()
            .append(&mut visit.traits);
        for (module, path, assoc) in visit.representations {
            result
                .declared
                .held
                .push((name.clone(), module, path, assoc));
        }
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
    }
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
            if !scanned.contains(path) {
                result
                    .errors
                    .push(format!("{}: syn cannot parse it", path.display()));
            }
            continue;
        };
        let relative = path.strip_prefix(dir).unwrap_or(path);
        let mut seek = UseSeek::default();
        seek.visit_file(&file);
        let mut visit = LiteralVisit {
            krate: name.clone(),
            file: format!(
                "crates/{name}/{}",
                relative.to_string_lossy().replace('\\', "/")
            ),
            calls: Vec::new(),
            renames: seek.renames,
            module: Vec::new(),
        };
        visit.visit_file(&file);
        result.literals.append(&mut visit.calls);
    }
}

/// A key before the duplicates are numbered: the fields before the item
/// path, the item path, and the fields after it.
struct RawKey {
    head: String,
    item: String,
    tail: String,
    /// The type as the site writes it, before an alias resolves.
    ty: syn::Type,
    place: Place,
    shadow: Shadow,
}

/// What the items and blocks around a site declare. The gate reads aliases
/// and `use` items by module. A name a block declares either stands for no
/// alias of the module or stops the scan.
#[derive(Clone, Debug, Default)]
struct Shadow {
    /// The generic parameters of the enclosing items, and the names the
    /// enclosing blocks declare or import: none names an item of the
    /// module.
    masked: BTreeSet<String>,
    /// The `type` aliases the enclosing blocks declare.
    aliases: BTreeSet<String>,
    /// The names the `use` items of the enclosing blocks bring in, and
    /// [`GLOB`] for a glob.
    uses: BTreeSet<String>,
    /// What each name of `uses` imports, the innermost block's last. An
    /// `extern crate` counts: its path is the crate.
    imports: BTreeMap<String, Import>,
}

/// The word that stands for a glob `use` among the names a block brings in.
const GLOB: &str = "*";

/// The names one block declares as statements.
#[derive(Debug, Default)]
struct BlockNames {
    /// Its `type` aliases.
    aliases: BTreeSet<String>,
    /// The names its `use` and `extern crate` items bring in, and [`GLOB`]
    /// for a glob.
    uses: BTreeSet<String>,
    /// What each name of `uses` imports, and whether the item renames it.
    imports: BTreeMap<String, Import>,
    /// Every other item it declares.
    items: BTreeSet<String>,
}

impl BlockNames {
    /// The names the statements of `block` declare.
    fn of(block: &syn::Block) -> Self {
        let mut names = Self::default();
        for stmt in &block.stmts {
            let syn::Stmt::Item(item) = stmt else {
                continue;
            };
            let ident = match item {
                syn::Item::Type(t) => {
                    names.aliases.insert(t.ident.to_string());
                    continue;
                }
                syn::Item::Use(u) => {
                    let mut found = Vec::new();
                    let mut globs = Vec::new();
                    use_entries(&u.tree, &[], &mut found, &mut globs);
                    for (name, import) in found {
                        names.uses.insert(name.clone());
                        names.imports.insert(name, import);
                    }
                    if !globs.is_empty() {
                        names.uses.insert(GLOB.to_owned());
                    }
                    continue;
                }
                syn::Item::ExternCrate(e) => {
                    let (name, how) = match &e.rename {
                        Some((_, rename)) => (rename, ImportName::Renamed),
                        None => (&e.ident, ImportName::Kept),
                    };
                    names.uses.insert(name.to_string());
                    let import = Import {
                        path: vec![e.ident.to_string()],
                        name: how,
                    };
                    names.imports.insert(name.to_string(), import);
                    continue;
                }
                syn::Item::Struct(i) => &i.ident,
                syn::Item::Enum(i) => &i.ident,
                syn::Item::Union(i) => &i.ident,
                syn::Item::Trait(i) => &i.ident,
                syn::Item::TraitAlias(i) => &i.ident,
                syn::Item::Mod(i) => &i.ident,
                syn::Item::Const(i) => &i.ident,
                syn::Item::Static(i) => &i.ident,
                syn::Item::Fn(i) => &i.sig.ident,
                _ => continue,
            };
            names.items.insert(ident.to_string());
        }
        names
    }
}

/// The segments of `path`, without their generic arguments.
fn idents(path: &syn::Path) -> Vec<String> {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect()
}

/// The associated types a generic newtype holds as its one field. Each is
/// a trait path, as the struct writes it, and a name. The field holds
/// `<P as Trait>::Name`, or `P::Name` with `P: Trait`. Under `P::Name`,
/// each trait that bounds `P` counts. An impl writes `Name` only for the
/// trait that declares it, and a foreign bound never places in the
/// workspace.
///
/// A trait written with generic arguments counts for none: two impls of
/// `Trait<A>` and `Trait<B>` give one type two representations. A trait
/// path with a leading `::` counts for none either. It names an extern
/// crate, which the gate's module lookup confuses with a child module.
pub(crate) fn representation_slots(
    generics: &syn::Generics,
    ty: &syn::Type,
) -> BTreeSet<(Vec<String>, String)> {
    let syn::Type::Path(path) = ty else {
        return BTreeSet::new();
    };
    let plain =
        |segments: &[&syn::PathSegment]| segments.iter().all(|segment| segment.arguments.is_none());
    if let Some(qself) = &path.qself {
        // `<P as Trait>::Name`: the segments before `qself.position` name
        // the trait, the rest name the associated type.
        let is_param = matches!(&*qself.ty, syn::Type::Path(p)
        if p.qself.is_none()
            && p.path.get_ident().is_some_and(|ident| {
                generics.type_params().any(|param| param.ident == *ident)
            }));
        let segments: Vec<&syn::PathSegment> = path.path.segments.iter().collect();
        return match (is_param, segments.split_at_checked(qself.position)) {
            (true, Some((trait_path, [name])))
                if path.path.leading_colon.is_none()
                    && !trait_path.is_empty()
                    && plain(trait_path)
                    && name.arguments.is_none() =>
            {
                BTreeSet::from([(
                    trait_path
                        .iter()
                        .map(|segment| segment.ident.to_string())
                        .collect(),
                    name.ident.to_string(),
                )])
            }
            _ => BTreeSet::new(),
        };
    }
    let segments: Vec<&syn::PathSegment> = path.path.segments.iter().collect();
    match segments.as_slice() {
        [param, name] if param.arguments.is_none() && name.arguments.is_none() => {
            let Some(declared) = generics
                .type_params()
                .find(|candidate| candidate.ident == param.ident)
            else {
                return BTreeSet::new();
            };
            let mut bounds: Vec<&syn::TypeParamBound> = declared.bounds.iter().collect();
            if let Some(clause) = &generics.where_clause {
                for predicate in &clause.predicates {
                    if let syn::WherePredicate::Type(bound) = predicate
                        && matches!(&bound.bounded_ty, syn::Type::Path(p)
                            if p.qself.is_none() && p.path.is_ident(&param.ident))
                    {
                        bounds.extend(bound.bounds.iter());
                    }
                }
            }
            bounds
                .into_iter()
                .filter_map(|bound| match bound {
                    syn::TypeParamBound::Trait(t)
                        if t.path.leading_colon.is_none()
                            && plain(&t.path.segments.iter().collect::<Vec<_>>()) =>
                    {
                        Some(idents(&t.path))
                    }
                    _ => None,
                })
                .map(|bound| (bound, name.ident.to_string()))
                .collect()
        }
        _ => BTreeSet::new(),
    }
}

/// The type and const parameters `generics` declares.
fn generic_names(generics: &syn::Generics) -> BTreeSet<String> {
    generics
        .params
        .iter()
        .filter_map(|param| match param {
            syn::GenericParam::Type(t) => Some(t.ident.to_string()),
            syn::GenericParam::Const(c) => Some(c.ident.to_string()),
            syn::GenericParam::Lifetime(_) => None,
        })
        .collect()
}

/// Why the gate cannot read `ty` under `shadow`: it names an alias, or a
/// path a `use` brings in, that a block declares. The gate reads both by
/// module only. `None` when the module's reading holds.
fn shadowed(declared: &Declared, ty: &syn::Type, shadow: &Shadow) -> Option<String> {
    let mut paths = PathSeek::default();
    paths.visit_type(ty);
    paths.found.iter().find_map(|segments| {
        let (first, last) = (segments.first()?, segments.last()?);
        let opens =
            |name: &str| declared.alias_names.contains(name) || declared.renames.contains(name);
        if shadow.aliases.contains(first) {
            return Some(format!(
                "it names {first}, an alias a block declares, and the gate reads aliases by \
                 module only"
            ));
        }
        // The module reading masks a name a block imports. That holds only
        // for an import kept under its own name that ends in no alias, no
        // rename and no primitive.
        if let Some(import) = shadow.imports.get(first) {
            let end = import.path.last().map_or("", String::as_str);
            if import.name == ImportName::Renamed || opens(end) || PRIMITIVES.contains(&end) {
                return Some(format!(
                    "it names {first}, which a use or extern crate item of a block brings in \
                     for {}, and the gate reads use items by module only",
                    import.path.join("::")
                ));
            }
        }
        if shadow.uses.contains(first) && (opens(first) || opens(last)) {
            return Some(format!(
                "it names {}, which a use item of a block brings in, and the gate reads use \
                 items by module only",
                segments.join("::")
            ));
        }
        if segments.len() == 1 && shadow.uses.contains(GLOB) && opens(first) {
            return Some(format!(
                "it names {first}, which a glob use item of a block may bring in, and the gate \
                 reads use items by module only"
            ));
        }
        None
    })
}

/// The segments of every type path a type writes.
#[derive(Default)]
struct PathSeek {
    found: Vec<Vec<String>>,
}

impl<'ast> Visit<'ast> for PathSeek {
    fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
        if path.qself.is_none() {
            self.found.push(
                path.path
                    .segments
                    .iter()
                    .map(|segment| segment.ident.to_string())
                    .collect(),
            );
        }
        syn::visit::visit_type_path(self, path);
    }
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

/// Evaluates a `cfg` predicate in a build that is not a test. Only `test`
/// is known false there. A feature, even one named for tests, can be turned
/// on in a release build, so it is unknown, and its code is scanned.
fn evaluate(meta: &syn::Meta) -> Tri {
    match meta {
        syn::Meta::Path(path) if path.is_ident("test") => Tri::False,
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
pub(crate) fn test_only(attrs: &[syn::Attribute]) -> bool {
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

/// The macros that read a file as data, beside the file that calls them.
const DATA_MACROS: &[&str] = &["include_str", "include_bytes"];

/// The `sqlx` macro that embeds each file of a directory, its crate, and the
/// directory it reads when the call names none.
const MIGRATE: &str = "migrate";
pub(crate) const SQLX: &str = "sqlx";
const MIGRATIONS: &str = "migrations";

/// A file the source names as data rather than as a module.
#[derive(Debug, PartialEq, Eq)]
enum DataRead {
    /// The literal of an `include_str!` or an `include_bytes!`, relative to
    /// the directory of the file that calls it.
    File(String),
    /// The directory of a `migrate!`, relative to the crate directory.
    Dir(String),
}

/// Whether a bare `migrate!` in a module names the `sqlx` macro: a `use` of
/// that module imports `sqlx::migrate` under its own name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Migrate {
    /// No `use` of the module imports it, so a bare `migrate!` is some other
    /// macro, and names no data.
    Foreign,
    /// A `use` of the module imports it.
    Imported,
}

/// Every data read a file names outside test code. The tokens are read
/// whole, so a call inside another macro or inside an attribute counts. A
/// call whose argument is no string literal, such as a `concat!` or a
/// metavariable, names nothing the walk can resolve, and is left out.
fn data_reads(file: &syn::File) -> Vec<DataRead> {
    let mut found = Vec::new();
    let migrate = imports_migrate(&file.items);
    for attr in &file.attrs {
        seek_data(attr.to_token_stream(), migrate, &mut found);
    }
    data_items(&file.items, migrate, &mut found);
    found
}

/// Whether a `use` among `items`, outside test code, imports `sqlx::migrate`
/// under its own name. A glob, a rename and a `use` inside a block import
/// nothing here, so a `migrate!` they would name fails closed.
fn imports_migrate(items: &[syn::Item]) -> Migrate {
    let imported = items.iter().any(|item| {
        let syn::Item::Use(u) = item else {
            return false;
        };
        if test_only(&u.attrs) {
            return false;
        }
        let mut names = Vec::new();
        let mut globs = Vec::new();
        use_entries(&u.tree, &[], &mut names, &mut globs);
        names.iter().any(|(name, import)| {
            name == MIGRATE && import.name == ImportName::Kept && import.path == [SQLX, MIGRATE]
        })
    });
    if imported {
        Migrate::Imported
    } else {
        Migrate::Foreign
    }
}

/// The data reads of `items`, an inline module's items included, a test
/// item left out. `migrate` says whether the module of `items` imports the
/// `sqlx` macro.
fn data_items(items: &[syn::Item], migrate: Migrate, found: &mut Vec<DataRead>) {
    for item in items {
        if test_only(item_attrs(item)) {
            continue;
        }
        match item {
            syn::Item::Mod(m) => {
                for attr in &m.attrs {
                    seek_data(attr.to_token_stream(), migrate, found);
                }
                if let Some((_, inner)) = &m.content {
                    data_items(inner, imports_migrate(inner), found);
                }
            }
            other => seek_data(other.to_token_stream(), migrate, found),
        }
    }
}

/// The outer attributes of an item.
pub(crate) fn item_attrs(item: &syn::Item) -> &[syn::Attribute] {
    match item {
        syn::Item::Const(i) => &i.attrs,
        syn::Item::Enum(i) => &i.attrs,
        syn::Item::ExternCrate(i) => &i.attrs,
        syn::Item::Fn(i) => &i.attrs,
        syn::Item::ForeignMod(i) => &i.attrs,
        syn::Item::Impl(i) => &i.attrs,
        syn::Item::Macro(i) => &i.attrs,
        syn::Item::Mod(i) => &i.attrs,
        syn::Item::Static(i) => &i.attrs,
        syn::Item::Struct(i) => &i.attrs,
        syn::Item::Trait(i) => &i.attrs,
        syn::Item::TraitAlias(i) => &i.attrs,
        syn::Item::Type(i) => &i.attrs,
        syn::Item::Union(i) => &i.attrs,
        syn::Item::Use(i) => &i.attrs,
        _ => &[],
    }
}

/// Whether the call named at `at` of `trees` is the `sqlx` macro. That is
/// `sqlx::migrate!`, `::sqlx::migrate!`, or a bare `migrate!` in a module
/// that imports it. Any other path names another macro.
fn names_migrate(trees: &[TokenTree], at: usize, migrate: Migrate) -> bool {
    let back = |n: usize| at.checked_sub(n).and_then(|i| trees.get(i));
    let colon = |token: Option<&TokenTree>| is_punct(token, ':');
    if colon(back(1)) {
        let crate_name = matches!(back(3), Some(TokenTree::Ident(ident)) if ident.unraw() == SQLX);
        // `::sqlx` is the crate. `x::sqlx` and `<T>::sqlx` name a module.
        let rooted = !colon(back(4))
            || (colon(back(5))
                && !matches!(back(6), Some(TokenTree::Ident(_)))
                && !is_punct(back(6), '>'));
        return colon(back(2)) && crate_name && rooted;
    }
    migrate == Migrate::Imported
}

/// The data reads of a token stream, at any depth of groups. A call is a
/// name, a `!` and a group. An `include_str!` or an `include_bytes!` counts
/// behind any path, and a `migrate!` counts as the `sqlx` macro alone.
fn seek_data(tokens: TokenStream, migrate: Migrate, found: &mut Vec<DataRead>) {
    let trees: Vec<TokenTree> = tokens.into_iter().collect();
    for (at, tree) in trees.iter().enumerate() {
        if let TokenTree::Group(group) = tree {
            seek_data(group.stream(), migrate, found);
            continue;
        }
        let TokenTree::Ident(ident) = tree else {
            continue;
        };
        let (Some(TokenTree::Punct(bang)), Some(TokenTree::Group(args))) =
            (trees.get(at + 1), trees.get(at + 2))
        else {
            continue;
        };
        if bang.as_char() != '!' {
            continue;
        }
        let name = ident.unraw().to_string();
        let literal = literal_argument(args.stream());
        if DATA_MACROS.contains(&name.as_str()) {
            if let Some(Some(literal)) = literal {
                found.push(DataRead::File(literal));
            }
        } else if name == MIGRATE && names_migrate(&trees, at, migrate) {
            match literal {
                Some(Some(literal)) => found.push(DataRead::Dir(literal)),
                Some(None) => found.push(DataRead::Dir(MIGRATIONS.to_owned())),
                None => {}
            }
        }
    }
}

/// The one string literal of a call's arguments, a trailing comma allowed.
/// `Some(None)` when the call has no argument, `None` for any other shape.
fn literal_argument(tokens: TokenStream) -> Option<Option<String>> {
    if tokens.is_empty() {
        return Some(None);
    }
    let parser = |input: syn::parse::ParseStream| {
        let literal: syn::LitStr = input.parse()?;
        let _: Option<syn::Token![,]> = input.parse()?;
        Ok(literal.value())
    };
    parser.parse2(tokens).ok().map(Some)
}

/// Whether the attribute is the one-segment `name`, raw or not: rustc reads
/// `#[r#path]` as `#[path]`.
fn named(attr: &syn::Attribute, name: &str) -> bool {
    attr.path()
        .get_ident()
        .is_some_and(|ident| ident.unraw() == name)
}

/// Where the `path` attributes of a module declaration put its file. A
/// `path` inside a `cfg_attr`, at any depth, and a `path` that is not one
/// string literal, are unplaced: the gate fails closed on both.
fn mod_path(attrs: &[syn::Attribute]) -> ModPath {
    for attr in attrs {
        if named(attr, "cfg_attr") {
            let mut streams = match &attr.meta {
                syn::Meta::List(list) => vec![list.tokens.clone()],
                _ => Vec::new(),
            };
            while let Some(stream) = streams.pop() {
                for tree in stream {
                    match tree {
                        TokenTree::Ident(ident) if ident.unraw() == "path" => {
                            return ModPath::Unplaced(Unplaced::CfgAttr);
                        }
                        TokenTree::Group(group) => streams.push(group.stream()),
                        _ => {}
                    }
                }
            }
        }
    }
    let Some(attr) = attrs.iter().find(|attr| named(attr, "path")) else {
        return ModPath::Named;
    };
    match &attr.meta {
        syn::Meta::NameValue(syn::MetaNameValue {
            value:
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(path),
                    ..
                }),
            ..
        }) => ModPath::Given(PathBuf::from(path.value())),
        _ => ModPath::Unplaced(Unplaced::NotLiteral),
    }
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

/// What a dereference target hands out (`ARCH-037`). A reference, a slice,
/// an array at any depth and a [`WRAPPERS`] type expose their element. A
/// path is read by its last segment, so `std::primitive::u32` is a `u32`.
/// `generics` holds the type parameters of the impl.
fn exposure(ty: &syn::Type, generics: &BTreeMap<String, String>) -> Exposure {
    match ty {
        syn::Type::Paren(syn::TypeParen { elem, .. })
        | syn::Type::Group(syn::TypeGroup { elem, .. })
        | syn::Type::Reference(syn::TypeReference { elem, .. })
        | syn::Type::Slice(syn::TypeSlice { elem, .. })
        | syn::Type::Array(syn::TypeArray { elem, .. })
        | syn::Type::Ptr(syn::TypePtr { elem, .. }) => exposure(elem, generics),
        syn::Type::Tuple(tuple) => {
            let parts: Vec<Exposure> = tuple
                .elems
                .iter()
                .map(|elem| exposure(elem, generics))
                .collect();
            parts
                .iter()
                .find(|part| matches!(part, Exposure::Primitive(_)))
                .or_else(|| parts.iter().find(|part| !matches!(part, Exposure::Opaque)))
                .cloned()
                .unwrap_or(Exposure::Opaque)
        }
        syn::Type::Path(path) => {
            if path.qself.is_some() {
                return Exposure::Unsure("a qualified path".to_owned());
            }
            let Some(last) = path.path.segments.last() else {
                return Exposure::Unsure("an empty path".to_owned());
            };
            let name = last.ident.to_string();
            if name.starts_with(META) {
                return Exposure::Unsure("a metavariable of a macro_rules! body".to_owned());
            }
            // `T::Assoc` reads as `<T as Trait>::Assoc`, which an instance
            // decides.
            if let Some(first) = path.path.segments.first()
                && path.path.segments.len() >= 2
                && generics.contains_key(&first.ident.to_string())
            {
                return Exposure::Unsure(format!(
                    "an associated type of the type parameter {}",
                    first.ident
                ));
            }
            // `Self::Inner` names an associated type of the impl's own type,
            // which the gate does not read.
            if path.path.segments.len() >= 2
                && path
                    .path
                    .segments
                    .first()
                    .is_some_and(|first| first.ident == "Self")
            {
                return Exposure::Unsure("an associated type of Self".to_owned());
            }
            if path.path.segments.len() == 1 && generics.contains_key(&name) {
                return Exposure::Generic(name);
            }
            if PRIMITIVES.contains(&name.as_str()) {
                return Exposure::Primitive(name);
            }
            if WRAPPERS.contains(&name.as_str()) {
                let syn::PathArguments::AngleBracketed(args) = &last.arguments else {
                    return Exposure::Unsure(format!("a {name} with no type argument"));
                };
                return args
                    .args
                    .iter()
                    .find_map(|arg| match arg {
                        syn::GenericArgument::Type(inner) => Some(exposure(inner, generics)),
                        _ => None,
                    })
                    .unwrap_or_else(|| {
                        Exposure::Unsure(format!("a {name} with no type argument"))
                    });
            }
            Exposure::Opaque
        }
        syn::Type::BareFn(_)
        | syn::Type::TraitObject(_)
        | syn::Type::ImplTrait(_)
        | syn::Type::Never(_) => Exposure::Opaque,
        _ => Exposure::Unsure("a type the gate cannot read without expansion".to_owned()),
    }
}

/// Why one dereference impl exposes a primitive, or `None` when it does
/// not. `derefs` holds every impl, where a `DerefMut` finds its `Deref`.
///
/// A generic target fails closed. A bound that names a primitive, as
/// `T: AsRef<str>` does, refuses it by design. So does a bare `T`, whose
/// instances rustc resolves and the tokens do not show.
fn deref_refusal(declared: &Declared, deref: &DerefImpl, derefs: &[DerefImpl]) -> Option<String> {
    let source = match &deref.target {
        Some(_) => Some(deref.clone()),
        None if deref.of == DerefTrait::DerefMut => derefs
            .iter()
            .find(|other| {
                other.of == DerefTrait::Deref
                    && other.krate == deref.krate
                    && other.module == deref.module
                    && other.self_ty == deref.self_ty
            })
            .cloned(),
        None => None,
    };
    let unsure = |why: &str| {
        format!(
            "{} | {}: {} of {}: {why}, so the gate cannot show that it exposes no primitive and \
             fails closed. A newtype never dereferences to its primitive (ARCH-037)",
            deref.krate, deref.file, deref.of, deref.ty
        )
    };
    let Some(source) = source else {
        return Some(unsure(match deref.of {
            DerefTrait::Deref => "the impl writes no Target",
            DerefTrait::DerefMut => "no Deref of the type is in its module",
        }));
    };
    let target_text = source.target.clone().unwrap_or_default();
    let Ok(target) = syn::parse_str::<syn::Type>(&target_text) else {
        return Some(unsure("the Target does not parse as a type"));
    };
    let params: BTreeSet<String> = source.bounds.keys().cloned().collect();
    let target = match declared.resolve(&target, &source.krate, &source.module, &params) {
        Ok(target) => target,
        Err(why) => {
            return Some(unsure(&format!(
                "its Target {target_text} stays unread: {why}"
            )));
        }
    };
    let exposes = |primitive: &str, how: String| {
        format!(
            "{} | {}: {} of {} to {target_text}{how} exposes the primitive {primitive}. A newtype \
             never dereferences to its primitive: name an accessor for the one foreign call that \
             needs it (ARCH-037)",
            deref.krate, deref.file, deref.of, deref.ty
        )
    };
    match exposure(&target, &source.bounds) {
        Exposure::Opaque => None,
        Exposure::Primitive(primitive) => Some(exposes(&primitive, String::new())),
        Exposure::Unsure(why) => Some(unsure(&format!("its Target {target_text} is {why}"))),
        Exposure::Generic(param) => {
            let bound = source.bounds.get(&param).cloned().unwrap_or_default();
            if let Some(primitive) = bound
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .find(|word| PRIMITIVES.contains(word))
            {
                return Some(exposes(
                    primitive,
                    format!(", whose bound {bound} names {primitive},"),
                ));
            }
            Some(unsure(&format!(
                "its Target is the type parameter {param}, which an instance may make a primitive"
            )))
        }
    }
}

/// The refusal of each impl of `derefs` whose target exposes a primitive,
/// or that the gate cannot read. A `DerefMut` takes the target of the
/// `Deref` of the same type name in its module.
fn refused_derefs(declared: &Declared, derefs: &[DerefImpl]) -> Vec<String> {
    derefs
        .iter()
        .filter_map(|deref| deref_refusal(declared, deref, derefs))
        .collect()
}

/// Whether a normalised type names one of [`PRIMITIVES`] as a whole word.
fn holds_primitive(ty: &str) -> bool {
    ty.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|word| PRIMITIVES.contains(&word))
}

/// Each `use ... as` of a file, per module path: the new name, then the
/// segments of the path it renames. A rename holds in the module that
/// writes it, and in no other.
type Renames = BTreeMap<Vec<String>, BTreeMap<String, Vec<String>>>;

/// The `use` items of one file: what each `as` renames, and what each
/// `pub use` re-exports.
#[derive(Default)]
struct UseSeek {
    /// The module path the walk is in.
    module: Vec<String>,
    renames: Renames,
    reexports: BTreeSet<String>,
    /// Each `use ... as` that is not private: the new name, then the last
    /// segment of the path it renames.
    exported: Vec<(String, String)>,
}

impl UseSeek {
    /// Walks one use tree under the segments `prefix`.
    fn tree(&mut self, tree: &syn::UseTree, prefix: &[String], public: Privacy) {
        match tree {
            syn::UseTree::Path(path) => {
                let ident = path.ident.to_string();
                if prefix.is_empty() && public == Privacy::Public {
                    self.reexports.insert(ident.clone());
                }
                let mut next = prefix.to_vec();
                next.push(ident);
                self.tree(&path.tree, &next, public);
            }
            syn::UseTree::Rename(rename) => {
                let ident = rename.ident.to_string();
                if prefix.is_empty() && public == Privacy::Public {
                    self.reexports.insert(ident.clone());
                }
                if public != Privacy::Private {
                    self.exported
                        .push((rename.rename.to_string(), ident.clone()));
                }
                let mut full = prefix.to_vec();
                full.push(ident);
                self.renames
                    .entry(self.module.clone())
                    .or_default()
                    .insert(rename.rename.to_string(), full);
            }
            syn::UseTree::Name(name) => {
                if prefix.is_empty() && public == Privacy::Public {
                    self.reexports.insert(name.ident.to_string());
                }
            }
            syn::UseTree::Group(group) => {
                for item in &group.items {
                    self.tree(item, prefix, public);
                }
            }
            syn::UseTree::Glob(_) => {}
        }
    }
}

impl<'ast> Visit<'ast> for UseSeek {
    fn visit_item_use(&mut self, u: &'ast syn::ItemUse) {
        self.tree(&u.tree, &[], privacy(&u.vis));
    }

    fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
        self.module.push(m.ident.to_string());
        syn::visit::visit_item_mod(self, m);
        self.module.pop();
    }
}

/// The trait of an impl: `path` as written, `segments` once a rename is
/// undone.
fn trait_path(path: &syn::Path, segments: &[String]) -> TraitPath {
    let args = match path.segments.last().map(|last| &last.arguments) {
        Some(syn::PathArguments::AngleBracketed(args)) => args
            .args
            .iter()
            .filter_map(|arg| match arg {
                syn::GenericArgument::Type(ty) => {
                    Some(normalise(&ty.to_token_stream().to_string()))
                }
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    TraitPath {
        name: segments.last().cloned().unwrap_or_default(),
        root: (segments.len() >= 2)
            .then(|| segments.first().cloned())
            .flatten(),
        args,
        segments: segments.to_vec(),
    }
}

/// The dereference impl `i` writes, `None` for an impl of any other trait.
fn deref_impl(
    i: &syn::ItemImpl,
    of_trait: Option<&TraitPath>,
    krate: &str,
    module: &str,
    file: &str,
) -> Option<DerefImpl> {
    let of = match of_trait.map(|path| path.name.as_str()) {
        Some("Deref") => DerefTrait::Deref,
        Some("DerefMut") => DerefTrait::DerefMut,
        _ => return None,
    };
    let target = i.items.iter().find_map(|item| match item {
        syn::ImplItem::Type(t) if t.ident == "Target" => {
            Some(normalise(&t.ty.to_token_stream().to_string()))
        }
        _ => None,
    });
    let mut bounds: BTreeMap<String, String> = BTreeMap::new();
    for param in i.generics.type_params() {
        let text = param
            .bounds
            .iter()
            .map(|bound| normalise(&bound.to_token_stream().to_string()))
            .collect::<Vec<_>>()
            .join("+");
        bounds.insert(param.ident.to_string(), text);
    }
    for predicate in i
        .generics
        .where_clause
        .iter()
        .flat_map(|clause| &clause.predicates)
    {
        if let syn::WherePredicate::Type(predicate) = predicate
            && let syn::Type::Path(path) = &predicate.bounded_ty
            && let Some(ident) = path.path.get_ident()
            && let Some(bound) = bounds.get_mut(&ident.to_string())
        {
            let text = predicate
                .bounds
                .iter()
                .map(|bound| normalise(&bound.to_token_stream().to_string()))
                .collect::<Vec<_>>()
                .join("+");
            bound.push('+');
            bound.push_str(&text);
        }
    }
    Some(DerefImpl {
        krate: krate.to_owned(),
        module: module.to_owned(),
        file: file.to_owned(),
        ty: normalise(&i.self_ty.to_token_stream().to_string()),
        self_ty: type_name(&i.self_ty),
        bounds,
        of,
        target: (of == DerefTrait::Deref).then_some(target).flatten(),
    })
}

/// The body of each rule of a `macro_rules!`, the group after each `=>`,
/// with its metavariables made Rust that syn parses (see [`unmeta`]).
pub(crate) fn rule_bodies(tokens: TokenStream) -> Vec<TokenStream> {
    let tokens: Vec<TokenTree> = tokens.into_iter().collect();
    tokens
        .windows(4)
        .filter_map(|window| match window {
            [
                TokenTree::Group(matcher),
                TokenTree::Punct(eq),
                TokenTree::Punct(gt),
                TokenTree::Group(body),
            ] if eq.as_char() == '=' && gt.as_char() == '>' => {
                let mut fragments = BTreeMap::new();
                fragments_of(matcher.stream(), &mut fragments);
                Some(unmeta(body.stream(), &fragments))
            }
            _ => None,
        })
        .collect()
}

/// The fragment each metavariable of a matcher declares: `$v:vis` maps
/// `v` to `vis`.
pub(crate) fn fragments_of(matcher: TokenStream, fragments: &mut BTreeMap<String, String>) {
    let tokens: Vec<TokenTree> = matcher.into_iter().collect();
    for (i, token) in tokens.iter().enumerate() {
        match token {
            TokenTree::Punct(dollar) if dollar.as_char() == '$' => {
                if let (
                    Some(TokenTree::Ident(name)),
                    Some(TokenTree::Punct(colon)),
                    Some(TokenTree::Ident(fragment)),
                ) = (tokens.get(i + 1), tokens.get(i + 2), tokens.get(i + 3))
                    && colon.as_char() == ':'
                {
                    fragments.insert(name.to_string(), fragment.to_string());
                }
            }
            TokenTree::Group(group) => fragments_of(group.stream(), fragments),
            _ => {}
        }
    }
}

/// What a metavariable `$name` of the fragment `fragment` reads as, so the
/// body parses. A `vis`, an `item` or a `stmt` reads as nothing. A
/// `lifetime` reads as a lifetime, a `block` as an empty block, a
/// `literal` as `0`. Any other reads as the word `__ambre_meta_name`, a
/// name, a type, an expression or a pattern alike.
fn metavariable(name: &proc_macro2::Ident, fragment: Option<&str>) -> Vec<TokenTree> {
    let word = || proc_macro2::Ident::new(&format!("{META}{name}"), name.span());
    match fragment {
        Some("vis" | "item" | "stmt") => Vec::new(),
        Some("lifetime") => vec![
            TokenTree::Punct(proc_macro2::Punct::new('\'', proc_macro2::Spacing::Joint)),
            TokenTree::Ident(word()),
        ],
        Some("block") => vec![TokenTree::Group(Group::new(
            Delimiter::Brace,
            TokenStream::new(),
        ))],
        Some("literal") => vec![TokenTree::Literal(proc_macro2::Literal::u8_unsuffixed(0))],
        _ => vec![TokenTree::Ident(word())],
    }
}

/// A `macro_rules!` body as Rust. A `$x` reads as [`metavariable`] says
/// for the fragment `fragments` gives it. `$crate` reads `crate`, and a
/// repetition `$( .. ) sep op` reads its contents once.
fn unmeta(stream: TokenStream, fragments: &BTreeMap<String, String>) -> TokenStream {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    let mut out: Vec<TokenTree> = Vec::new();
    let mut i = 0;
    while let Some(token) = tokens.get(i) {
        i += 1;
        match token {
            TokenTree::Punct(dollar) if dollar.as_char() == '$' => match tokens.get(i) {
                Some(TokenTree::Ident(name)) => {
                    i += 1;
                    if name == "crate" {
                        out.push(TokenTree::Ident(proc_macro2::Ident::new(
                            "crate",
                            name.span(),
                        )));
                    } else {
                        let fragment = fragments.get(&name.to_string()).map(String::as_str);
                        out.extend(metavariable(name, fragment));
                    }
                }
                Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Parenthesis => {
                    i += 1;
                    out.extend(unmeta(group.stream(), fragments));
                    // The separator, if any, then the operator.
                    let op = |token: Option<&TokenTree>| matches!(token, Some(TokenTree::Punct(p)) if matches!(p.as_char(), '*' | '+' | '?'));
                    if op(tokens.get(i)) {
                        i += 1;
                    } else if op(tokens.get(i + 1)) {
                        i += 2;
                    }
                }
                _ => out.push(token.clone()),
            },
            TokenTree::Group(group) => {
                let mut inner = Group::new(group.delimiter(), unmeta(group.stream(), fragments));
                inner.set_span(group.span());
                out.push(TokenTree::Group(inner));
            }
            other => out.push(other.clone()),
        }
    }
    out.into_iter().collect()
}

/// Whether a token stream writes `impl ... Deref` or `impl ... DerefMut`
/// before the body of the impl, at any depth.
fn writes_deref(stream: TokenStream) -> bool {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    tokens.iter().enumerate().any(|(i, token)| match token {
        TokenTree::Ident(word) if word == "impl" => tokens
            .iter()
            .skip(i + 1)
            .take_while(|next| {
                !matches!(next, TokenTree::Group(group) if group.delimiter() == Delimiter::Brace)
                    && !is_punct(Some(next), ';')
            })
            .any(|next| matches!(next, TokenTree::Ident(name) if name == "Deref" || name == "DerefMut")),
        TokenTree::Group(group) => writes_deref(group.stream()),
        _ => false,
    })
}

/// The impls a parsed `macro_rules!` body writes at any depth: in a
/// constant, a `fn` body or a module. It also keeps the rules of each
/// `macro_rules!` the body declares.
#[derive(Default)]
struct MacroItems {
    impls: Vec<syn::ItemImpl>,
    rules: Vec<TokenStream>,
    /// The path of each other macro the body calls whose tokens write a
    /// dereference impl.
    opaque: Vec<String>,
}

impl<'ast> Visit<'ast> for MacroItems {
    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        self.impls.push(i.clone());
        syn::visit::visit_item_impl(self, i);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if mac.path.is_ident("macro_rules") {
            self.rules.push(mac.tokens.clone());
        } else if writes_deref(mac.tokens.clone()) {
            self.opaque
                .push(normalise(&mac.path.to_token_stream().to_string()));
        }
    }
}

/// The crate roots that may enable an unstable feature, as the crate
/// directory and the root file inside it. None does today.
const FEATURE_ROOTS: &[(&str, &str)] = &[];

/// Each `#![feature(..)]` of a crate root, as one refusal, in `file`
/// (`ARCH-033`). An unstable feature enables syntax that the pinned syn may
/// not parse. A data file that holds such syntax would then pass the parse
/// guard of the dep-info check, and a nightly build could compile it. A
/// `feature` inside a `cfg_attr`, at any depth, fails too: the gate
/// evaluates no `cfg_attr`.
fn features(file: &str, tree: &syn::File) -> Vec<String> {
    let mut found = Vec::new();
    for attr in &tree.attrs {
        let held = named(attr, "feature")
            || (named(attr, "cfg_attr") && holds_ident(attr.meta.to_token_stream(), "feature"));
        if held {
            found.push(format!(
                "{file}:{}: a crate root enables an unstable feature, whose syntax the gate may \
                 not parse, so the scan fails closed (ARCH-033): remove it, or name the root in \
                 FEATURE_ROOTS with the reason",
                attr.pound_token.span.start().line
            ));
        }
    }
    found
}

/// Whether `tokens` hold the identifier `name`, raw or not, at any depth.
fn holds_ident(tokens: TokenStream, name: &str) -> bool {
    tokens.into_iter().any(|tree| match tree {
        TokenTree::Ident(ident) => ident.unraw() == name,
        TokenTree::Group(group) => holds_ident(group.stream(), name),
        _ => false,
    })
}

/// Each `mod` that the tokens of a macro hold in `file`, outside test code,
/// as one refusal per keyword (`ARCH-033`).
///
/// The walk follows the `mod` items the source writes, and expands no
/// macro. The dep-info check refuses a file rustc reads that the walk does
/// not, unless a walked file names it as data. The dep-info does not tell a
/// module from data. So a `mod` a macro writes could compile a data file as
/// code, and no check would read it. The gate refuses the keyword, raw or
/// not, wherever a macro holds it. That is a `macro_rules!` body, called or
/// not, the tokens of any macro call, and the tokens of an attribute.
///
/// One form passes: `mod name { .. }` or `mod $name { .. }`, an inline
/// module, in the literal tokens of a `macro_rules!` transcriber. Those
/// tokens reach rustc in the order they are written, so the module reads no
/// file. A child `mod` it holds is a keyword of the same tokens, read the
/// same way, so a `#[path]` child still fails. The tokens of a macro call
/// pass to a macro that may reorder them: `mod hid {}` can come out as
/// `mod hid;`. So an inline module fails there, in a matcher, and in a call
/// a transcriber writes.
fn macro_mods(file: &str, tree: &syn::File) -> Vec<String> {
    let mut seek = MacroMods {
        file: file.to_owned(),
        found: Vec::new(),
    };
    if !test_only(&tree.attrs) {
        seek.visit_file(tree);
    }
    seek.found
}

/// The walk of one file for the `mod` keywords its macros hold.
struct MacroMods {
    file: String,
    found: Vec<String>,
}

/// Where a `mod` keyword sits in the tokens of a macro, which decides
/// whether an inline module passes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ModReach {
    /// The literal tokens of a `macro_rules!` transcriber, outside a macro
    /// call they write. rustc reads them in the order they are written, so
    /// an inline module passes.
    Transcriber,
    /// Every other token: a matcher, the tokens of a macro call and those
    /// of an attribute. A macro may reorder them, so every `mod` fails.
    Reordered,
}

impl MacroMods {
    /// Refuses each `mod` keyword of `tokens` that is not an inline module
    /// of a transcriber, at any depth of groups.
    fn tokens(&mut self, tokens: TokenStream, reach: ModReach) {
        let trees: Vec<TokenTree> = tokens.into_iter().collect();
        for (at, tree) in trees.iter().enumerate() {
            match tree {
                TokenTree::Ident(ident)
                    if ident.unraw() == "mod"
                        && !(reach == ModReach::Transcriber
                            && inline_module(trees.get(at + 1..).unwrap_or_default())) =>
                {
                    self.found.push(format!(
                        "{}:{}: a macro holds the keyword `mod` outside an inline module of a \
                         macro_rules! transcriber. The walk follows no module a macro \
                         declares, and the dep-info check cannot tell its file from data, so \
                         the scan fails closed (ARCH-033): declare the module in the source",
                        self.file,
                        ident.span().start().line
                    ));
                }
                TokenTree::Group(group) => {
                    let inner = if called(trees.get(..at).unwrap_or_default()) {
                        ModReach::Reordered
                    } else {
                        reach
                    };
                    self.tokens(group.stream(), inner);
                }
                _ => {}
            }
        }
    }

    /// Reads the rules of a `macro_rules!`: each group after `=>` is a
    /// transcriber, and every other token is a matcher or a separator.
    fn rules(&mut self, tokens: TokenStream) {
        let trees: Vec<TokenTree> = tokens.into_iter().collect();
        for (at, tree) in trees.iter().enumerate() {
            let arrow = at
                .checked_sub(2)
                .and_then(|start| trees.get(start..at))
                .is_some_and(|before| {
                    matches!(before, [TokenTree::Punct(eq), TokenTree::Punct(gt)]
                        if eq.as_char() == '=' && gt.as_char() == '>')
                });
            match tree {
                TokenTree::Group(group) if arrow => {
                    self.tokens(group.stream(), ModReach::Transcriber);
                }
                other => self.tokens(other.clone().into(), ModReach::Reordered),
            }
        }
    }
}

/// Whether the group after `before` may be the tokens of a macro call. That
/// is `!` then the group, or `!`, a name and the group, as `macro_rules! m {
/// .. }` writes. A negation in parentheses matches too, which only fails closed.
///
/// A metavariable before the group may also make a call, directly or with
/// one name between. When the caller passes `k !`, `$m $b { .. }` and
/// `$($m)* { .. }` both expand to `k ! { .. }`. So that group reads as a call too. The body
/// of `mod $name { .. }` is the one exception: rustc parses a module there,
/// whatever name the caller passes.
fn called(before: &[TokenTree]) -> bool {
    let bang = |tree: &TokenTree| matches!(tree, TokenTree::Punct(p) if p.as_char() == '!');
    let named = match before {
        [rest @ .., TokenTree::Ident(_)] => Some(rest),
        _ => None,
    };
    match before {
        [.., last] if bang(last) => true,
        [.., mark, TokenTree::Ident(_)] if bang(mark) => true,
        [
            ..,
            TokenTree::Ident(keyword),
            TokenTree::Punct(dollar),
            TokenTree::Ident(_),
        ] if keyword.unraw() == "mod" && dollar.as_char() == '$' => false,
        _ => ends_on_metavariable(before) || named.is_some_and(ends_on_metavariable),
    }
}

/// Whether `before` ends on a metavariable of a transcriber: `$x`, or a
/// repetition `$( .. )` with its operator and an optional separator.
fn ends_on_metavariable(before: &[TokenTree]) -> bool {
    let dollar = |tree: &TokenTree| matches!(tree, TokenTree::Punct(p) if p.as_char() == '$');
    let operator = |tree: &TokenTree| matches!(tree, TokenTree::Punct(p) if matches!(p.as_char(), '*' | '+' | '?'));
    let repetition = |tree: &TokenTree| matches!(tree, TokenTree::Group(g) if g.delimiter() == Delimiter::Parenthesis);
    match before {
        [.., mark, TokenTree::Ident(_)] => dollar(mark),
        [.., mark, group, op] if operator(op) && repetition(group) => dollar(mark),
        [.., mark, group, _, op] if operator(op) && repetition(group) => dollar(mark),
        _ => false,
    }
}

/// Whether the tokens after a `mod` keyword are a name and a brace group:
/// `name { .. }` or `$name { .. }`.
fn inline_module(after: &[TokenTree]) -> bool {
    let body = match after {
        [TokenTree::Ident(_), body, ..] => body,
        [TokenTree::Punct(dollar), TokenTree::Ident(_), body, ..] if dollar.as_char() == '$' => {
            body
        }
        _ => return false,
    };
    matches!(body, TokenTree::Group(group) if group.delimiter() == Delimiter::Brace)
}

impl<'ast> Visit<'ast> for MacroMods {
    // Test code is no build that ships.
    fn visit_item(&mut self, item: &'ast syn::Item) {
        if !test_only(item_attrs(item)) {
            syn::visit::visit_item(self, item);
        }
    }

    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        if !test_only(crate::secrets::impl_item_attrs(item)) {
            syn::visit::visit_impl_item(self, item);
        }
    }

    fn visit_trait_item(&mut self, item: &'ast syn::TraitItem) {
        if !test_only(crate::secrets::trait_item_attrs(item)) {
            syn::visit::visit_trait_item(self, item);
        }
    }

    // An attribute's tokens reach a macro: `#[doc = m!(..)]` does, and so
    // does a `cfg_attr` that writes one.
    fn visit_attribute(&mut self, attr: &'ast syn::Attribute) {
        self.tokens(attr.meta.to_token_stream(), ModReach::Reordered);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if mac.path.is_ident("macro_rules") {
            self.rules(mac.tokens.clone());
        } else {
            self.tokens(mac.tokens.clone(), ModReach::Reordered);
        }
    }
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
    /// The modules the file declares, each still to read. The walk sets
    /// their crate root.
    children: Vec<Pending>,
    /// The impl item whose signature the visit reads, `None` elsewhere.
    host: Option<Host>,
    /// The struct or the enum whose fields the visit reads, `None`
    /// elsewhere.
    owner: Option<String>,
    /// Every trait the file declares.
    traits: BTreeSet<String>,
    /// Every struct and enum the file declares, by name.
    types: Vec<(String, Declaration)>,
    /// Each associated type a generic newtype of the file holds as its one
    /// field: its module, the trait path and the name
    /// ([`representation_slots`]).
    representations: Vec<(String, Vec<String>, String)>,
    /// Every impl of `Deref` or `DerefMut` the file writes.
    derefs: Vec<DerefImpl>,
    /// Who reaches the next site.
    privacy: Privacy,
    /// The `fn` and position of the next site, when it is a parameter.
    slot: Option<Slot>,
    /// What the next field is to its owner.
    role: FieldRole,
    /// Every `type` alias the file declares, by name.
    aliases: Vec<(String, Alias)>,
    /// The first segment of each path a `pub use` of the file re-exports.
    reexports: BTreeSet<String>,
    /// Each `use ... as` of the file, per module.
    renames: Renames,
    /// Each `use ... as` of the file that is not private: the new name,
    /// then the last segment of the path it renames.
    exported: Vec<(String, String)>,
    /// The generic parameters of each enclosing item, the innermost last.
    generics: Vec<BTreeSet<String>>,
    /// The names each enclosing block declares, the innermost last.
    blocks: Vec<BlockNames>,
    /// The path of every module the file declares.
    modules: BTreeSet<String>,
    /// Every struct, enum, union and trait the file declares, and every
    /// type a `define_*!` macro writes, as `module::Name`.
    items: BTreeSet<String>,
    /// Each name a `use` of the file brings into a module: the module, the
    /// name, and what it imports.
    imports: Vec<(String, String, Import)>,
    /// The path of each glob `use` of the file, with its module.
    globs: Vec<(String, Vec<String>)>,
    /// The refusal of each `macro_rules!` body that writes a dereference
    /// impl and does not parse.
    unread: Vec<String>,
}

/// Each name a use tree brings in under the segments `prefix`, with the
/// path it imports, and the path of each glob. `use x::{self}` brings in
/// `x`, and `use x as _` brings in no name.
fn use_entries(
    tree: &syn::UseTree,
    prefix: &[String],
    names: &mut Vec<(String, Import)>,
    globs: &mut Vec<Vec<String>>,
) {
    let joined = |ident: &syn::Ident| {
        let mut path = prefix.to_vec();
        if ident != "self" {
            path.push(ident.to_string());
        }
        path
    };
    match tree {
        syn::UseTree::Path(path) => {
            let mut next = prefix.to_vec();
            next.push(path.ident.to_string());
            use_entries(&path.tree, &next, names, globs);
        }
        syn::UseTree::Name(name) => {
            let path = joined(&name.ident);
            if let Some(last) = path.last().cloned() {
                let import = Import {
                    path,
                    name: ImportName::Kept,
                };
                names.push((last, import));
            }
        }
        syn::UseTree::Rename(rename) => {
            if rename.rename != "_" {
                let import = Import {
                    path: joined(&rename.ident),
                    name: ImportName::Renamed,
                };
                names.push((rename.rename.to_string(), import));
            }
        }
        syn::UseTree::Glob(_) => globs.push(prefix.to_vec()),
        syn::UseTree::Group(group) => {
            for item in &group.items {
                use_entries(item, prefix, names, globs);
            }
        }
    }
}

impl<'a> FileVisit<'a> {
    /// The visit of the file at `path`, which `placement` placed. rustc gives
    /// a file a `#[path]` loads the directory of a `mod.rs`: its own modules
    /// sit beside it.
    fn new(
        krate: &'a str,
        relative: &Path,
        path: &Path,
        module: Vec<String>,
        placement: &Placement,
    ) -> Self {
        let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
        // A crate root, a `mod.rs` and a file a `#[path]` loads hold their
        // children beside them. Any other module file holds them in the
        // directory of its own name.
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        let mod_rs = matches!(placement, Placement::Path)
            || matches!(
                name.as_deref(),
                Some("mod.rs" | "lib.rs" | "main.rs" | "build.rs")
            )
            || parent.ends_with("src/bin");
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
            host: None,
            owner: None,
            traits: BTreeSet::new(),
            types: Vec::new(),
            representations: Vec::new(),
            derefs: Vec::new(),
            privacy: Privacy::Private,
            slot: None,
            role: FieldRole::Member,
            aliases: Vec::new(),
            reexports: BTreeSet::new(),
            renames: BTreeMap::new(),
            exported: Vec::new(),
            generics: Vec::new(),
            blocks: Vec::new(),
            modules: BTreeSet::new(),
            items: BTreeSet::new(),
            imports: Vec::new(),
            globs: Vec::new(),
            unread: Vec::new(),
        }
    }

    /// Reads the `use` items of a file, inline modules included, before the
    /// visit: a rename applies above the `use` that writes it too.
    fn uses(&mut self, file: &syn::File) {
        let mut seek = UseSeek {
            module: self.module.clone(),
            ..UseSeek::default()
        };
        seek.visit_file(file);
        self.renames = seek.renames;
        self.reexports = seek.reexports;
        self.exported = seek.exported;
    }

    /// The segments of a trait path once a `use ... as` rename of the file
    /// is undone: `use ambre_core::Tree as T` makes `T` read
    /// `ambre_core::Tree`.
    fn unrename(&self, path: &syn::Path) -> Vec<String> {
        let segments: Vec<String> = path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect();
        match segments.split_first() {
            Some((first, rest)) => match self
                .renames
                .get(&self.module)
                .and_then(|renames| renames.get(first))
            {
                Some(full) => full.iter().chain(rest).cloned().collect(),
                None => segments,
            },
            None => segments,
        }
    }

    fn item(&self, name: &str) -> String {
        if self.scope.is_empty() {
            name.to_owned()
        } else {
            format!("{}::{name}", self.scope.join("::"))
        }
    }

    /// Records the type of a site. Whether it holds a primitive is read
    /// once every alias of the workspace is known, after the walk.
    fn emit(&mut self, item: String, kind: SiteKind, ty: &syn::Type) {
        let text = normalise(&ty.to_token_stream().to_string());
        let place = Place {
            krate: self.krate.to_owned(),
            module: self.module.join("::"),
            kind,
            owner: self.owner.clone(),
            host: self.host.clone(),
            form: TypeForm::Value,
            roots: BTreeSet::new(),
            privacy: self.privacy,
            shape: Shape::Other,
            primitives: BTreeSet::new(),
            slot: self.slot.clone(),
            role: self.role,
        };
        let sep = SiteKey::SEPARATOR;
        self.keys.push(RawKey {
            head: format!(
                "{}{sep}{}{sep}{}",
                self.krate,
                self.file,
                self.module.join("::")
            ),
            item,
            tail: format!("{}{sep}{text}", kind.as_str()),
            ty: ty.clone(),
            place,
            shadow: self.shadow(),
        });
    }

    /// What the enclosing items and blocks declare, for the next site.
    fn shadow(&self) -> Shadow {
        let mut shadow = Shadow::default();
        for names in &self.generics {
            shadow.masked.extend(names.iter().cloned());
        }
        for block in &self.blocks {
            shadow.masked.extend(block.items.iter().cloned());
            shadow
                .masked
                .extend(block.uses.iter().filter(|name| *name != GLOB).cloned());
            shadow.aliases.extend(block.aliases.iter().cloned());
            shadow.uses.extend(block.uses.iter().cloned());
            shadow.imports.extend(
                block
                    .imports
                    .iter()
                    .map(|(name, import)| (name.clone(), import.clone())),
            );
        }
        shadow
    }

    /// Visits what `visit` reads with the generic parameters of `generics`
    /// in scope.
    fn within(&mut self, generics: &syn::Generics, visit: impl FnOnce(&mut Self)) {
        self.generics.push(generic_names(generics));
        visit(self);
        self.generics.pop();
    }

    fn signature(&mut self, sig: &syn::Signature, privacy: Privacy, kind: FnKind) {
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
            let param = match &*arg.pat {
                syn::Pat::Ident(ident) => ident.ident.to_string(),
                other => normalise(&other.to_token_stream().to_string()),
            };
            self.slot = Some(Slot {
                name: name.clone(),
                position: ArgPosition(i),
                receiver,
                param: param.clone(),
                kind,
            });
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
        self.items.insert(format!("{module}::{name}"));
        self.types.push((name, Declaration { module, decl }));
    }

    /// Scans the initialiser of a constant or a static for the items it
    /// declares, `const _: () = { impl .. };` among them, under its name.
    fn initialiser(&mut self, name: String, expr: &syn::Expr) {
        self.scope.push(name);
        self.visit_expr(expr);
        self.scope.pop();
    }

    /// Reads each rule body of a `macro_rules!` for the dereference impls
    /// it writes, at any depth, nested `macro_rules!` included. A body that
    /// parses neither as items nor as statements, and that writes
    /// `impl ... Deref`, fails closed.
    fn macro_rules(&mut self, tokens: TokenStream) {
        let module = self.module.join("::");
        for body in rule_bodies(tokens) {
            let mut walk = MacroItems::default();
            if let Ok(file) = syn::parse2::<syn::File>(body.clone()) {
                walk.visit_file(&file);
            } else if let Ok(stmts) = syn::Block::parse_within.parse2(body.clone()) {
                for stmt in &stmts {
                    walk.visit_stmt(stmt);
                }
            } else {
                if writes_deref(body) {
                    self.unread.push(format!(
                        "{} | {}: a macro_rules! body in {module} writes an impl of Deref or \
                         DerefMut and does not parse as Rust, so the gate cannot show that it \
                         exposes no primitive and fails closed. A newtype never dereferences to \
                         its primitive (ARCH-037)",
                        self.krate, self.file
                    ));
                }
                continue;
            }
            for i in &walk.impls {
                let of_trait: Option<TraitPath> = i
                    .trait_
                    .as_ref()
                    .map(|(_, path, _)| trait_path(path, &self.unrename(path)));
                if let Some(deref) =
                    deref_impl(i, of_trait.as_ref(), self.krate, &module, &self.file)
                {
                    self.derefs.push(deref);
                }
            }
            for path in &walk.opaque {
                self.unread.push(self.opaque_refusal(path, &module));
            }
            for rules in walk.rules {
                self.macro_rules(rules);
            }
        }
    }

    /// Fails closed on a call of the macro `mac` whose tokens write a
    /// dereference impl. The gate expands no macro, so it cannot read the
    /// impl the call writes.
    fn opaque_deref(&mut self, mac: &syn::Macro) {
        if writes_deref(mac.tokens.clone()) {
            let path = normalise(&mac.path.to_token_stream().to_string());
            let refusal = self.opaque_refusal(&path, &self.module.join("::"));
            self.unread.push(refusal);
        }
    }

    /// The refusal of a call of the macro `path` in `module` that writes a
    /// dereference impl.
    fn opaque_refusal(&self, path: &str, module: &str) -> String {
        format!(
            "{} | {}: a call of {path}! in {module} writes an impl of Deref or DerefMut, and \
             the gate expands no macro, so it cannot show that the impl exposes no primitive \
             and fails closed. A newtype never dereferences to its primitive (ARCH-037)",
            self.krate, self.file
        )
    }
}

impl<'ast> Visit<'ast> for FileVisit<'_> {
    fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
        let name = m.ident.to_string();
        // A module is test code by its `cfg` alone, never by its name: a
        // `mod x_tests;` with no test `cfg` is built and scanned.
        let placed = mod_path(&m.attrs);
        let mut module = self.module.clone();
        module.push(name.clone());
        if test_only(&m.attrs) {
            // A test module is never read, wherever its file sits. A `path`
            // the gate cannot follow still fails: no `cfg` is read here.
            if let ModPath::Unplaced(why) = placed {
                self.children.push(Pending {
                    candidates: Vec::new(),
                    module,
                    placement: Placement::Unplaced(why),
                    root: PathBuf::new(),
                });
            }
            return;
        }
        if !self.blocks.is_empty() {
            // Its items are left unread: the scan fails on it.
            self.children.push(Pending {
                candidates: Vec::new(),
                module,
                placement: Placement::Unplaced(Unplaced::InBody),
                root: PathBuf::new(),
            });
            return;
        }
        self.modules.insert(module.join("::"));
        match &m.content {
            Some((_, items)) => {
                let why = match placed {
                    ModPath::Named => None,
                    ModPath::Given(_) => Some(Unplaced::Inline),
                    ModPath::Unplaced(why) => Some(why),
                };
                if let Some(why) = why {
                    self.children.push(Pending {
                        candidates: Vec::new(),
                        module,
                        placement: Placement::Unplaced(why),
                        root: PathBuf::new(),
                    });
                }
                let saved = (
                    self.dir.clone(),
                    self.file_dir.take(),
                    std::mem::take(&mut self.scope),
                    std::mem::take(&mut self.generics),
                    std::mem::take(&mut self.blocks),
                );
                self.dir = self.dir.join(&name);
                self.module.push(name);
                for item in items {
                    self.visit_item(item);
                }
                self.module.pop();
                (
                    self.dir,
                    self.file_dir,
                    self.scope,
                    self.generics,
                    self.blocks,
                ) = saved;
            }
            None => {
                let (candidates, placement) = match placed {
                    ModPath::Given(path) => (
                        vec![
                            self.file_dir
                                .clone()
                                .unwrap_or_else(|| self.dir.clone())
                                .join(path),
                        ],
                        Placement::Path,
                    ),
                    ModPath::Named => (
                        vec![
                            self.dir.join(format!("{name}.rs")),
                            self.dir.join(&name).join("mod.rs"),
                        ],
                        Placement::Named,
                    ),
                    ModPath::Unplaced(why) => (Vec::new(), Placement::Unplaced(why)),
                };
                self.children.push(Pending {
                    candidates,
                    module,
                    placement,
                    root: PathBuf::new(),
                });
            }
        }
    }

    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        if test_only(&f.attrs) {
            return;
        }
        let component = f.attrs.iter().any(|attr| {
            attr.path()
                .segments
                .last()
                .is_some_and(|last| last.ident == "component")
        });
        let kind = if component {
            FnKind::Component
        } else {
            FnKind::Plain
        };
        self.within(&f.sig.generics, |visit| {
            visit.signature(&f.sig, privacy(&f.vis), kind);
            visit.body(f.sig.ident.to_string(), &f.block);
        });
    }

    fn visit_block(&mut self, b: &'ast syn::Block) {
        self.blocks.push(BlockNames::of(b));
        syn::visit::visit_block(self, b);
        self.blocks.pop();
    }

    // A macro called inside a body or an expression writes what the gate
    // does not expand. One that writes a dereference impl fails closed.
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if mac.path.is_ident("macro_rules") {
            self.macro_rules(mac.tokens.clone());
            return;
        }
        self.opaque_deref(mac);
    }

    // `extern crate x as y;` names the crate `x` as `y`.
    fn visit_item_extern_crate(&mut self, e: &'ast syn::ItemExternCrate) {
        if test_only(&e.attrs) || !self.scope.is_empty() {
            return;
        }
        if let Some((_, rename)) = &e.rename {
            self.imports.push((
                self.module.join("::"),
                rename.to_string(),
                Import {
                    path: vec![e.ident.to_string()],
                    name: ImportName::Renamed,
                },
            ));
        }
    }

    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        if test_only(&i.attrs) {
            return;
        }
        let ty = normalise(&i.self_ty.to_token_stream().to_string());
        let self_ty = type_name(&i.self_ty);
        let self_full = ty.clone();
        let of_trait: Option<TraitPath> = i
            .trait_
            .as_ref()
            .map(|(_, path, _)| trait_path(path, &self.unrename(path)));
        let in_body = !self.blocks.is_empty();
        let trait_unplain = i.trait_.as_ref().is_some_and(|(_, path, _)| {
            path.leading_colon.is_some()
                || path
                    .segments
                    .iter()
                    .any(|segment| !segment.arguments.is_none())
        });
        let host = |receiver: Receiver, output: Output| Host {
            self_ty: self_ty.clone(),
            self_full: self_full.clone(),
            of_trait: of_trait.clone(),
            receiver,
            output,
            in_body,
            trait_unplain,
        };
        let module = self.module.join("::");
        if let Some(deref) = deref_impl(i, of_trait.as_ref(), self.krate, &module, &self.file) {
            self.derefs.push(deref);
        }
        let label = match &i.trait_ {
            Some((_, path, _)) => format!(
                "<{ty} as {}>",
                normalise(&path.to_token_stream().to_string())
            ),
            None => ty,
        };
        self.scope.push(label);
        self.generics.push(generic_names(&i.generics));
        for item in &i.items {
            match item {
                syn::ImplItem::Fn(f) if !test_only(&f.attrs) => {
                    self.generics.push(generic_names(&f.sig.generics));
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
                    self.signature(&f.sig, reach, FnKind::Plain);
                    self.host = None;
                    self.body(f.sig.ident.to_string(), &f.block);
                    self.generics.pop();
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
                    self.initialiser(c.ident.to_string(), &c.expr);
                }
                // Only a trait impl declares an associated type on stable
                // Rust, so the site is as public as the trait.
                syn::ImplItem::Type(t) if !test_only(&t.attrs) => {
                    self.generics.push(generic_names(&t.generics));
                    self.host = Some(host(Receiver::Without, Output::Other));
                    self.privacy = Privacy::TraitImpl;
                    self.emit(self.item(&t.ident.to_string()), SiteKind::Assoc, &t.ty);
                    self.host = None;
                    self.generics.pop();
                }
                _ => {}
            }
        }
        self.generics.pop();
        self.scope.pop();
    }

    fn visit_item_trait(&mut self, t: &'ast syn::ItemTrait) {
        if test_only(&t.attrs) {
            return;
        }
        self.traits.insert(t.ident.to_string());
        self.items
            .insert(format!("{}::{}", self.module.join("::"), t.ident));
        self.scope.push(t.ident.to_string());
        self.generics.push(generic_names(&t.generics));
        for item in &t.items {
            if let syn::TraitItem::Fn(f) = item {
                if test_only(&f.attrs) {
                    continue;
                }
                self.within(&f.sig.generics, |visit| {
                    visit.signature(&f.sig, privacy(&t.vis), FnKind::Plain);
                    if let Some(block) = &f.default {
                        visit.body(f.sig.ident.to_string(), block);
                    }
                });
            }
        }
        self.generics.pop();
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
            // A struct a `fn` body declares is out of reach of every other
            // item, so it grants no exemption.
            if decl == Decl::Newtype
                && self.blocks.is_empty()
                && let Some(field) = s.fields.iter().find(|field| {
                    !matches!(&field.ty, syn::Type::Path(path)
                        if path.path.segments.last().is_some_and(|last| last.ident == "PhantomData"))
                })
            {
                let module = self.module.join("::");
                for (path, assoc) in representation_slots(&s.generics, &field.ty) {
                    self.representations.push((module.clone(), path, assoc));
                }
            }
            self.declare(name.clone(), decl);
            self.owner = Some(name.clone());
            self.within(&s.generics, |visit| {
                visit.fields(&name, &s.fields, Privacy::Private, derived(&s.attrs));
            });
            self.owner = None;
        }
    }

    fn visit_item_union(&mut self, u: &'ast syn::ItemUnion) {
        if !test_only(&u.attrs) {
            self.items
                .insert(format!("{}::{}", self.module.join("::"), u.ident));
            self.within(&u.generics, |visit| {
                visit.fields(
                    &u.ident.to_string(),
                    &u.fields.named,
                    Privacy::Private,
                    derived(&u.attrs),
                );
            });
        }
    }

    fn visit_item_enum(&mut self, e: &'ast syn::ItemEnum) {
        if test_only(&e.attrs) {
            return;
        }
        // A variant behind a test `cfg` is out of every build the gate reads,
        // so it holds no field that makes the enum carry data. A variant
        // written `V()` or `V {}` holds none either.
        let carries = e
            .variants
            .iter()
            .filter(|variant| !test_only(&variant.attrs))
            .any(|variant| !variant.fields.is_empty());
        let decl = if carries {
            Decl::DataEnum
        } else {
            Decl::FieldlessEnum
        };
        self.declare(e.ident.to_string(), decl);
        self.generics.push(generic_names(&e.generics));
        for variant in e
            .variants
            .iter()
            .filter(|variant| !test_only(&variant.attrs))
        {
            if let Some((_, discriminant)) = &variant.discriminant {
                self.initialiser(format!("{}::{}", e.ident, variant.ident), discriminant);
            }
            self.role = match &variant.fields {
                syn::Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                    FieldRole::VariantValue
                }
                _ => FieldRole::Member,
            };
            self.owner = Some(e.ident.to_string());
            self.fields(
                &format!("{}::{}", e.ident, variant.ident),
                &variant.fields,
                privacy(&e.vis),
                derived(&e.attrs),
            );
            self.owner = None;
            self.role = FieldRole::Member;
        }
        self.generics.pop();
    }

    fn visit_item_const(&mut self, c: &'ast syn::ItemConst) {
        if !test_only(&c.attrs) {
            self.privacy = privacy(&c.vis);
            self.emit(self.item(&c.ident.to_string()), SiteKind::Const, &c.ty);
            self.initialiser(c.ident.to_string(), &c.expr);
        }
    }

    fn visit_item_static(&mut self, s: &'ast syn::ItemStatic) {
        if !test_only(&s.attrs) {
            self.privacy = privacy(&s.vis);
            self.emit(self.item(&s.ident.to_string()), SiteKind::Static, &s.ty);
            self.initialiser(s.ident.to_string(), &s.expr);
        }
    }

    fn visit_item_type(&mut self, t: &'ast syn::ItemType) {
        if !test_only(&t.attrs) {
            self.privacy = privacy(&t.vis);
            self.within(&t.generics, |visit| {
                visit.emit(visit.item(&t.ident.to_string()), SiteKind::Alias, &t.ty);
            });
            // An alias nested in a `fn` body is out of reach of the module:
            // a site that names it fails closed.
            if self.scope.is_empty() {
                let params = t
                    .generics
                    .params
                    .iter()
                    .map(|param| match param {
                        syn::GenericParam::Lifetime(l) => Param {
                            name: l.lifetime.ident.to_string(),
                            kind: ParamKind::Lifetime,
                            default: None,
                        },
                        syn::GenericParam::Type(t) => Param {
                            name: t.ident.to_string(),
                            kind: ParamKind::Type,
                            default: t
                                .default
                                .as_ref()
                                .map(|ty| normalise(&ty.to_token_stream().to_string())),
                        },
                        syn::GenericParam::Const(c) => Param {
                            name: c.ident.to_string(),
                            kind: ParamKind::Const,
                            default: None,
                        },
                    })
                    .collect();
                self.aliases.push((
                    t.ident.to_string(),
                    Alias {
                        module: self.module.join("::"),
                        params,
                        target: normalise(&t.ty.to_token_stream().to_string()),
                    },
                ));
            }
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
        if m.mac.path.is_ident("macro_rules") {
            self.macro_rules(m.mac.tokens.clone());
        } else if !defines {
            self.opaque_deref(&m.mac);
        }
    }

    // A `use` inside a `fn` body reaches that body alone, so only a `use`
    // of a module is recorded here. `BlockNames` reads the others.
    fn visit_item_use(&mut self, u: &'ast syn::ItemUse) {
        if test_only(&u.attrs) || !self.scope.is_empty() {
            return;
        }
        let module = self.module.join("::");
        let mut names = Vec::new();
        let mut globs = Vec::new();
        use_entries(&u.tree, &[], &mut names, &mut globs);
        for (name, import) in names {
            self.imports.push((module.clone(), name, import));
        }
        for glob in globs {
            self.globs.push((module.clone(), glob));
        }
    }

    // A foreign block declares the foreign API itself.
    fn visit_item_foreign_mod(&mut self, _: &'ast syn::ItemForeignMod) {}
}

#[cfg(test)]
#[path = "scan_tests.rs"]
mod tests;
