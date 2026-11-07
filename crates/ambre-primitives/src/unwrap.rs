//! The unwrap report: where a workspace newtype gives up its primitive.
//!
//! The primitive gate reads signatures. A newtype still loses its value in
//! a body: `store(name.as_str())` hands bare text to a workspace `fn`, and
//! no type guards it there. The rule unwraps a newtype only at the foreign
//! call that takes the primitive, or inside the newtype's own impl and
//! module (`ai/ambre-rust-style.md`). An impl of a conversion trait whose
//! type argument is the newtype counts as its own. So does a named
//! conversion, `fn of_row(delta: RowDelta) -> Self`, for its parameter.
//!
//! The report reads syn trees, with no type information. What it tracks:
//!
//!   * a type is one name of one crate. A `use` of the file, the crate a
//!     path starts at, or a declaration of the current crate places a
//!     name. A glob of the workspace places it when one crate answers. A
//!     re-export follows the `pub use` items of its crate, or else any
//!     `use` of it. A type a signature names and nothing places is
//!     foreign. The owner of a call that nothing places goes by its name;
//!   * a newtype is a struct with one field over a primitive, a byte
//!     vector, a byte slice or a byte array. `PhantomData` is no field;
//!   * a generic struct is a newtype too when its one field is an
//!     associated type of a bounded parameter, as `Id<E: Kind>(E::Repr)`.
//!     An impl of the workspace must write a primitive or bytes for that
//!     associated type, matched by the last segment of the trait. Its
//!     accessor returns that associated type, or a borrow of it, never the
//!     marker;
//!   * a type a `define_*!` macro writes is a newtype too;
//!   * an accessor is a method of the newtype that takes `self` alone and
//!     returns the primitive or a borrow of it;
//!   * `len` and the `is_*` and `has_*` predicates are no accessors;
//!   * `to_string` is an accessor when the `Display` of the newtype reads a
//!     field. `.0` is one on a tuple field that is not private;
//!   * a hand-written `AsRef` or `Borrow` of a primitive adds its method;
//!   * `impl From<T> for String`, or any other primitive, makes `t.into()`
//!     and `String::from(t)` unwraps of `T`;
//!   * an unwrap is handed when a call takes it as an argument, or a
//!     method takes it as its receiver;
//!   * the unwrap travels through `&`, a paren, a `let` binding and the
//!     conversions and views of `PEELS`;
//!   * a parameter, a `let` annotation, a struct literal and `self` type a
//!     receiver;
//!   * so do a constructor such as `T::new` or `T::parse`, and a field of
//!     a struct of the workspace;
//!   * a `const`, a `static`, an array, a literal, a std constructor such as
//!     `Vec::new`, `format!` and `vec!` type a receiver too;
//!   * so do a method whose return std or a dependency fixes, as
//!     `to_token_stream` does, and a variant of a foreign `enum` in a pattern;
//!   * a field of an untyped base holds a newtype only when a workspace
//!     struct holds one under that field name. A generic field escapes this
//!     rule;
//!   * the return type of a workspace `fn` types its call, and `?` reads
//!     the `Ok` or `Some` type. An `Err(e)` pattern and the closure of
//!     `map_err` bind an error;
//!   * a type a `macro_rules!` body names as written is catalogued, as a
//!     transcriber that writes `struct Fields` names it.
//!
//! Each handed unwrap gets one verdict. A callee the workspace defines and
//! no foreign API names is a refusal. The tuple struct of a newtype is
//! such a callee, as `T::new` is. A method name both define is ambiguous.
//! On a receiver the report cannot type, an accessor that std shares or
//! many newtypes share is ambiguous too.
//!
//! A method counts as the workspace's only when the caller reaches its
//! crate through the manifests. A parameter typed `impl Into<T>`, with `T`
//! foreign, is a boundary: its argument goes to a foreign call.
//!
//! The report does not read a macro argument, a closure that returns the
//! unwrap, a struct literal field or a return value. The primitive gate
//! holds the last two at the signature. Nor does it read an `into`, an
//! `as_ref` or a `borrow` on a receiver it cannot type. A newtype a generic
//! `AsRef` bound takes escapes it too.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use proc_macro2::{Delimiter, TokenStream, TokenTree};
use quote::ToTokens;
use syn::spanned::Spanned;
use syn::visit::Visit;

use crate::allowlist::LineNo;
use crate::scan::{
    META, PRIMITIVES, Scan, Source, Tree, dependencies, fragments_of, item_attrs, macro_types,
    representation_slots, rule_bodies, test_only,
};

/// The allowlist header: a key, the number of sites it allows, then the
/// reason those sites keep their unwrap.
pub const HEADER: &str = "key\tcount\treason";

/// The accessor names a std type defines too. On a receiver the report
/// cannot type, the call may read a `String`, a `Cell` or a `Mutex`.
const SHARED_ACCESSORS: &[&str] = &[
    "as_str",
    "as_bytes",
    "get",
    "into_inner",
    "into_string",
    "into_bytes",
    "to_string",
    "to_vec",
    "as_slice",
    "as_ref",
    "bytes",
    "code",
    "encode",
    "id",
    "index",
    "key",
    "kind",
    "label",
    "name",
    "root",
    "value",
    "borrow",
];

/// The conversions and views an unwrap travels through unchanged.
const PEELS: &[&str] = &[
    "clone",
    "to_owned",
    "into",
    "to_string",
    "to_vec",
    "as_ref",
    "as_str",
    "as_bytes",
    "borrow",
];

/// The std types a constructor such as `Vec::new` types a receiver with.
const STD_TYPES: &[&str] = &[
    "Vec", "String", "BTreeMap", "BTreeSet", "HashMap", "HashSet", "VecDeque", "PathBuf",
];

/// The macros whose value has one std type.
const MACRO_TYPES: &[(&str, &str)] = &[("format", "String"), ("vec", "Vec")];

/// The methods whose return type std or a dependency fixes, with that
/// type: a receiver they build is no newtype.
const RETURNS: &[(&str, &str)] = &[
    ("to_string", "String"),
    ("to_vec", "Vec"),
    ("to_string_lossy", "Cow"),
    ("to_token_stream", "TokenStream"),
    ("into_token_stream", "TokenStream"),
    ("unraw", "Ident"),
    ("get_ident", "Ident"),
    ("display", "Display"),
    ("chars", "Chars"),
    ("trim", "str"),
    ("trim_start", "str"),
    ("trim_end", "str"),
    ("get_map", "LoroMap"),
    ("get_text", "LoroText"),
    ("get_list", "LoroList"),
    ("get_movable_list", "LoroMovableList"),
    ("oplog_vv", "VersionVector"),
    ("state_vv", "VersionVector"),
];

/// The methods of `RETURNS` that a trait of std fixes, so a workspace
/// method of the same name returns the same type.
const STD_RETURNS: &[&str] = &["to_string", "to_vec"];

/// The accessors the report reads on a typed receiver alone. Every type
/// has an `into`. An `Option` has an `as_ref` and a `RefCell` a `borrow`,
/// and neither is an unwrap.
const CONVERSIONS: &[&str] = &["into", "as_ref", "borrow"];

/// The type an error value takes: the name an `Err(..)` pattern or the
/// closure of `map_err` binds. No newtype is an error.
const ERROR: &str = "Error";

/// The most candidates a site names one by one; more print as `*`.
const FEW: usize = 3;

/// The method names std and the dependencies define. A workspace method of
/// the same name makes a call on an untyped receiver ambiguous.
const FOREIGN_METHODS: &[&str] = &[
    "abs",
    "add",
    "all",
    "and_then",
    "any",
    "append",
    "apply",
    "as_bytes",
    "as_mut",
    "as_ref",
    "as_slice",
    "as_str",
    "bind",
    "borrow",
    "borrow_mut",
    "call",
    "chain",
    "clamp",
    "clear",
    "clone",
    "close",
    "cmp",
    "collect",
    "contains",
    "contains_key",
    "count",
    "decode",
    "dedup",
    "deserialize",
    "div",
    "encode",
    "ends_with",
    "entry",
    "eq",
    "execute",
    "extend",
    "extend_from_slice",
    "fetch_all",
    "fetch_one",
    "fetch_optional",
    "filter",
    "find",
    "first",
    "flush",
    "fmt",
    "fold",
    "from",
    "get",
    "get_mut",
    "hash",
    "insert",
    "into",
    "into_bytes",
    "into_inner",
    "into_iter",
    "into_string",
    "is_empty",
    "iter",
    "iter_mut",
    "join",
    "last",
    "len",
    "load",
    "lock",
    "map",
    "map_err",
    "matches",
    "max",
    "min",
    "mul",
    "ne",
    "next",
    "ok_or",
    "ok_or_else",
    "open",
    "or_default",
    "or_insert",
    "or_insert_with",
    "parse",
    "partial_cmp",
    "peek",
    "pop",
    "position",
    "push",
    "push_back",
    "push_front",
    "push_str",
    "query",
    "read",
    "read_to_string",
    "recv",
    "remove",
    "replace",
    "reserve",
    "resize",
    "retain",
    "rev",
    "send",
    "serialize",
    "set",
    "skip",
    "sort",
    "sort_by",
    "sort_by_key",
    "spawn",
    "split",
    "split_at",
    "starts_with",
    "store",
    "strip_prefix",
    "strip_suffix",
    "sub",
    "sum",
    "swap",
    "take",
    "then",
    "to_owned",
    "to_string",
    "to_vec",
    "trim",
    "truncate",
    "try_into",
    "try_send",
    "unwrap_or",
    "unwrap_or_default",
    "unwrap_or_else",
    "update",
    "write",
    "write_all",
    "write_fmt",
    "write_str",
    "zip",
];

/// The method names a workspace method may share with a dependency. These
/// come from a primitive's arithmetic, Loro, the S3 and HTTP builders and
/// the drawing crates.
const DEPENDENCY_METHODS: &[&str] = &[
    "body",
    "bucket",
    "ceil",
    "checked_add",
    "checked_mul",
    "checked_sub",
    "content_type",
    "delete",
    "export",
    "fill",
    "floor",
    "header",
    "import",
    "key",
    "mark",
    "pow",
    "rotate",
    "round",
    "saturating_add",
    "saturating_sub",
    "scale",
    "splice",
    "sqrt",
    "stroke",
    "translate",
    "unmark",
    "unsigned_abs",
    "wrapping_add",
    "wrapping_sub",
];

/// Whether std or a dependency defines a method of this name.
fn foreign_method(name: &str) -> bool {
    FOREIGN_METHODS.contains(&name) || DEPENDENCY_METHODS.contains(&name)
}

/// The macro families that write a newtype.
fn writes_newtypes(name: &str) -> bool {
    name.starts_with("define_")
}

/// One site the report names.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Finding {
    pub krate: String,
    pub file: String,
    pub line: LineNo,
    /// The newtype, `A|B` for one of a few, `*` for one of many.
    pub newtype: String,
    pub accessor: String,
    /// The call the primitive is handed to: `f`, `a::f` or `.m`.
    pub callee: String,
    /// The module and item path of the enclosing `fn`.
    pub scope: String,
    pub why: Why,
}

/// What leaves a site ambiguous. A refusal carries `Callee`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Why {
    /// A workspace and a foreign method share the name of the callee.
    Callee,
    /// The accessor is a std name, on a receiver the report cannot type.
    Receiver,
}

impl Finding {
    /// The allowlist key: the line left out, so an edit above the site
    /// keeps it. Every site of one shape in one `fn` shares a key.
    pub fn key(&self) -> String {
        format!(
            "{} | {} | {} | {}::{} -> {}",
            self.krate, self.file, self.scope, self.newtype, self.accessor, self.callee
        )
    }
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} | {}:{} | {}::{} -> {}",
            self.krate, self.file, self.line, self.newtype, self.accessor, self.callee
        )
    }
}

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

/// What one report found.
#[derive(Debug, Default)]
pub struct Report {
    pub refusals: Vec<Finding>,
    pub ambiguous: Vec<Finding>,
    /// The unwraps handed to a foreign call.
    pub foreign: SiteCount,
    /// Every unwrap the report attributes to a newtype, handed or not.
    pub sites: SiteCount,
    /// The newtypes the report knows, by name.
    pub newtypes: SiteCount,
    /// Each file the report could not read again.
    pub errors: Vec<String>,
}

impl Report {
    /// The ambiguous sites whose receiver the report cannot type.
    pub fn ambiguous_receivers(&self) -> SiteCount {
        self.ambiguous_by(Why::Receiver)
    }

    /// The ambiguous sites whose callee name std or a dependency shares.
    pub fn ambiguous_callees(&self) -> SiteCount {
        self.ambiguous_by(Why::Callee)
    }

    fn ambiguous_by(&self, why: Why) -> SiteCount {
        SiteCount(
            self.ambiguous
                .iter()
                .filter(|finding| finding.why == why)
                .count(),
        )
    }
}

/// Where a newtype is declared.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Home {
    krate: String,
    /// The module path, as `crate::a::b`.
    module: String,
}

/// One invocation of a macro family.
struct Invocation {
    family: String,
    /// The types the invocation writes.
    types: Vec<String>,
    home: Home,
}

/// One newtype of the workspace, by name: a name two crates declare holds
/// both homes.
#[derive(Debug, Default)]
struct Newtype {
    /// Each crate and module that declares it.
    homes: BTreeSet<Home>,
    accessors: BTreeSet<String>,
    /// Whether `.0` reaches the value outside its module.
    tuple: bool,
    /// Each primitive a std conversion trait turns the newtype into, as
    /// `impl From<Name> for String` does.
    conversions: BTreeSet<String>,
    /// The representation of a generic newtype, `None` for any other.
    repr: Option<GenericRepr>,
}

/// What the workspace declares, read once before any body.
#[derive(Debug, Default)]
struct Catalog {
    newtypes: BTreeMap<Ty, Newtype>,
    /// The type of each field of each struct. A tuple field goes by its
    /// index.
    fields: BTreeMap<Ty, BTreeMap<String, Ty>>,
    /// Every type a field of each name holds, in any struct.
    field_types: BTreeMap<String, BTreeSet<Ty>>,
    /// The type of each `const` and `static`, keyed by its crate and name.
    consts: BTreeMap<Ty, Ty>,
    /// Every struct, enum, union, trait and alias, by name.
    types: BTreeSet<String>,
    /// The same types, with the crates that declare each.
    declared: Defs,
    /// Every `const` and `static`, with the crates that declare each.
    values: Defs,
    /// The roots each crate imports a name from, by crate and by name. A
    /// re-export is followed through them.
    crate_uses: BTreeMap<String, BTreeMap<String, Roots>>,
    /// Every free `fn`, by name.
    fns: Defs,
    /// Every method of an inherent impl, a workspace trait or a macro.
    methods: Defs,
    /// Every method of an impl of a foreign trait.
    trait_methods: Defs,
    /// Every method a workspace trait declares: an extension trait reaches
    /// a primitive receiver through one.
    declared_methods: Defs,
    /// The return type of each free `fn`, by name. A name with two
    /// returns holds none.
    returns: BTreeMap<String, Option<Ret>>,
    /// The return type of each associated `fn`, by owner and name.
    method_returns: BTreeMap<(Ty, String), Option<Ret>>,
    /// The parameters of each method typed `impl Into<T>`, by owner and
    /// name. Each holds its index, `self` left out, and the crate `T`
    /// comes from.
    boundaries: BTreeMap<(Ty, String), Vec<(usize, String)>>,
    /// The crates each crate reaches through its dependencies, itself
    /// included. A crate absent here reaches every crate.
    reach: BTreeMap<String, BTreeSet<String>>,
    /// Every name a `use` of the workspace brings in from a foreign crate:
    /// a re-export of `kurbo::Rect` is a foreign type.
    foreign_names: BTreeSet<String>,
    /// The crate names, as a path writes them.
    crates: BTreeSet<String>,
    /// Every module name of the workspace.
    modules: BTreeSet<String>,
}

impl Catalog {
    /// Files a type the crate `krate` declares.
    fn declare(&mut self, name: String, krate: &str) {
        self.declared.add(name.clone(), krate);
        self.types.insert(name);
    }

    /// The return the catalog knows for the free `fn` `name`.
    fn ret(&self, name: &str) -> Option<&Ret> {
        self.returns.get(name)?.as_ref()
    }

    /// The return the catalog knows for the associated `fn` `name` of
    /// `owner`.
    fn method_ret(&self, owner: &Ty, name: &str) -> Option<&Ret> {
        self.method_returns
            .get(&(owner.clone(), name.to_owned()))?
            .as_ref()
    }

    /// The type a path names in `scope`, `None` when the report cannot
    /// place it. A path that starts at a crate, or a name a `use` brings
    /// in, takes that crate. A name the current crate declares is its own.
    /// A glob of the workspace comes last: two crates through two globs
    /// leave the name unplaced.
    fn resolve(&self, scope: &Scope, segments: &[String], space: &Defs) -> Option<Ty> {
        let (first, name) = (segments.first()?, segments.last()?);
        if let [_, .., _] = segments {
            let root = scope.uses.names.get(first).unwrap_or(first);
            return self.rooted(&scope.krate, root, name, space, 0);
        }
        if let Some(root) = scope.uses.names.get(name) {
            return self.rooted(&scope.krate, root, name, space, 0);
        }
        if space.declares(name, &scope.krate) {
            return Some(Ty::of(&scope.krate, name));
        }
        let found: BTreeSet<Ty> = scope
            .uses
            .globs
            .iter()
            .filter(|root| self.workspace_root(root))
            .filter_map(|root| self.rooted(&scope.krate, root, name, space, 0))
            .collect();
        let mut found = found.into_iter();
        match (found.next(), found.next()) {
            (Some(one), None) => Some(one),
            _ => None,
        }
    }

    /// Whether a path root stays in the workspace.
    fn workspace_root(&self, root: &str) -> bool {
        self.local_root(root) || self.crates.contains(root)
    }

    /// Whether a path root stays in the current crate.
    fn local_root(&self, root: &str) -> bool {
        ["crate", "self", "super", "Self"].contains(&root) || self.modules.contains(root)
    }

    /// The type `name` that a path from `root` reaches, read from `krate`.
    /// A root outside the workspace is std or a dependency.
    fn rooted(
        &self,
        krate: &str,
        root: &str,
        name: &str,
        space: &Defs,
        depth: usize,
    ) -> Option<Ty> {
        if self.local_root(root) {
            self.exported(krate, name, space, depth)
        } else if self.crates.contains(root) {
            self.exported(&root.replace('_', "-"), name, space, depth)
        } else {
            Some(Ty::foreign(name))
        }
    }

    /// The type `name` the crate `krate` declares or re-exports. A
    /// re-export is a `pub use` of the crate, or else any `use` of it. Two
    /// roots that reach two types, or a chain longer than [`REEXPORTS`],
    /// leave the name unplaced.
    fn exported(&self, krate: &str, name: &str, space: &Defs, depth: usize) -> Option<Ty> {
        if space.declares(name, krate) {
            return Some(Ty::of(krate, name));
        }
        if depth >= REEXPORTS {
            return None;
        }
        let roots = self.crate_uses.get(krate)?.get(name)?;
        let roots = if roots.public.is_empty() {
            &roots.any
        } else {
            &roots.public
        };
        let found: BTreeSet<Option<Ty>> = roots
            .iter()
            .filter(|root| !self.local_root(root))
            .map(|root| self.rooted(krate, root, name, space, depth + 1))
            .collect();
        let mut found = found.into_iter();
        match (found.next(), found.next()) {
            (Some(one), None) => one,
            _ => None,
        }
    }
}

/// The longest chain of re-exports the report follows.
const REEXPORTS: usize = 4;

/// One type of one crate. A `const` goes by its crate and name the same
/// way.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Ty {
    /// The name, first: a map of types iterates in name order.
    name: String,
    /// The crate that declares it, `None` for std or a dependency.
    krate: Option<String>,
}

impl Ty {
    fn of(krate: &str, name: &str) -> Self {
        Self {
            name: name.to_owned(),
            krate: Some(krate.to_owned()),
        }
    }

    fn foreign(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            krate: None,
        }
    }

    fn is_workspace(&self) -> bool {
        self.krate.is_some()
    }
}

/// The roots a crate imports one name from.
#[derive(Debug, Default)]
struct Roots {
    /// The roots of a `pub use`: what the crate re-exports.
    public: BTreeSet<String>,
    /// The roots of any `use`.
    any: BTreeSet<String>,
}

/// What the `use` items of one file bring in.
#[derive(Clone, Debug, Default)]
struct Uses {
    /// The first segment of the path each `use` imports, by the name it
    /// brings in.
    names: BTreeMap<String, String>,
    /// The names a `pub use` brings in, with the first segment of each.
    public: BTreeMap<String, String>,
    /// The first segment of the path of each glob.
    globs: BTreeSet<String>,
}

/// Where a file resolves a name: its crate and its `use` items.
struct Scope {
    krate: String,
    uses: Uses,
}

/// Names the workspace defines, each with the crates that define it.
#[derive(Debug, Default)]
struct Defs(BTreeMap<String, BTreeSet<String>>);

impl Defs {
    fn add(&mut self, name: String, krate: &str) {
        self.0.entry(name).or_default().insert(krate.to_owned());
    }

    /// Whether the crate `krate` defines the name.
    fn declares(&self, name: &str, krate: &str) -> bool {
        self.0
            .get(name)
            .is_some_and(|crates| crates.contains(krate))
    }

    /// Whether a crate that `reach` holds defines the name. `None` reaches
    /// every crate.
    fn within(&self, name: &str, reach: Option<&BTreeSet<String>>) -> bool {
        self.0.get(name).is_some_and(|crates| {
            reach.is_none_or(|reach| crates.iter().any(|krate| reach.contains(krate)))
        })
    }
}

/// The return type of a `fn`, as the catalog reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Ret {
    /// The type, `Self` read as the owner.
    ty: Ty,
    /// The type `?` yields, for a `Result` or an `Option`.
    tried: Option<Ty>,
}

impl Ret {
    /// The return of a signature, each type placed by `place`.
    fn of(
        output: &syn::ReturnType,
        owner: Option<&Ty>,
        place: impl Fn(&[String]) -> Ty,
    ) -> Option<Self> {
        let syn::ReturnType::Type(_, ty) = output else {
            return None;
        };
        let own = |segments: Vec<String>| match owner {
            Some(owner) if segments == ["Self"] => owner.clone(),
            _ => place(&segments),
        };
        let shape = shape_path(ty)?;
        let tried = match &**ty {
            syn::Type::Path(p) if shape == ["Result"] || shape == ["Option"] => p
                .path
                .segments
                .last()
                .and_then(first_argument)
                .and_then(shape_path)
                .map(own),
            _ => None,
        };
        Some(Self {
            ty: own(shape),
            tried,
        })
    }
}

/// The index of each parameter, `self` left out, typed `impl Into<T>`,
/// with the crate `T` comes from. A path names that crate, or else a `use`
/// of the file does. A `T` with no crate to name, as `String`, is left out.
fn into_params(sig: &syn::Signature, imports: &BTreeMap<String, String>) -> Vec<(usize, String)> {
    sig.inputs
        .iter()
        .filter_map(|input| match input {
            syn::FnArg::Typed(t) => Some(&*t.ty),
            syn::FnArg::Receiver(_) => None,
        })
        .enumerate()
        .filter_map(|(index, ty)| {
            let syn::Type::ImplTrait(bounds) = ty else {
                return None;
            };
            bounds.bounds.iter().find_map(|bound| {
                let syn::TypeParamBound::Trait(t) = bound else {
                    return None;
                };
                let last = t.path.segments.last()?;
                if last.ident != "Into" {
                    return None;
                }
                let syn::Type::Path(target) = first_argument(last)? else {
                    return None;
                };
                let segments = &target.path.segments;
                let first = segments.first()?.ident.to_string();
                let root = if segments.len() > 1 || target.path.leading_colon.is_some() {
                    first
                } else {
                    imports.get(&first)?.clone()
                };
                Some((index, root))
            })
        })
        .collect()
}

/// A method signature, as the catalog reads it.
struct Method {
    name: String,
    /// Takes `self` and nothing else.
    self_only: bool,
    takes_self: bool,
    /// Returns a primitive, a borrow of one included.
    returns_inner: bool,
    /// Returns an associated type of a parameter of a generic impl, by name.
    returns_repr: Option<String>,
    /// Returns a `bool`: a predicate, never the representation.
    returns_bool: bool,
}

impl Method {
    fn is_accessor(&self) -> bool {
        self.is_accessor_of(None)
    }

    /// Whether the method is an accessor of a newtype whose generic
    /// representation is `repr`. Only the field's own associated type
    /// counts: the marker is no representation.
    fn is_accessor_of(&self, repr: Option<&GenericRepr>) -> bool {
        let returns = self.returns_inner
            || match (repr, &self.returns_repr) {
                (Some(repr), Some(name)) => repr.assocs.contains(name),
                _ => false,
            };
        self.self_only
            && returns
            && !self.returns_bool
            && self.name != "len"
            && !self.name.starts_with("is_")
            && !self.name.starts_with("has_")
    }
}

/// One impl block, as the catalog reads it.
struct Impl {
    owner: Ty,
    /// The crate that writes the impl.
    krate: String,
    /// The last segment of the trait, `None` for an inherent impl.
    of_trait: Option<String>,
    /// The first type argument of the trait: `T` in `From<T>`.
    argument: Option<Argument>,
    /// The name of the implementing type when it is a primitive, as in
    /// `impl From<Name> for String`.
    inner: Option<String>,
    methods: Vec<Method>,
    /// A `Display` whose `fmt` reads a field of `self`.
    shows_content: bool,
}

/// The first type argument of a trait, as the catalog reads it.
enum Argument {
    /// A primitive, a borrow or a slice of one: `str` in `AsRef<str>`. The
    /// name is `None` for a slice or an array.
    Inner(Option<String>),
    /// Any other type: `Name` in `From<Name>`.
    Named(Ty),
}

impl Argument {
    fn of(ty: &syn::Type, place: impl Fn(&[String]) -> Ty) -> Option<Self> {
        if inner_like(ty) {
            return Some(Self::Inner(type_name(ty)));
        }
        type_path(ty).map(|path| Self::Named(place(&path)))
    }
}

/// What the first pass gathers before the newtypes are known.
#[derive(Default)]
struct Raw {
    catalog: Catalog,
    impls: Vec<Impl>,
    traits: BTreeSet<String>,
    /// Each macro by name: its accessors.
    macros: BTreeMap<String, BTreeSet<String>>,
    invocations: Vec<Invocation>,
    /// Each generic struct that is a newtype once an impl writes a
    /// primitive for one of its slots.
    generic: Vec<GenericCandidate>,
    /// Each trait, by its last segment, and associated type an impl of the
    /// workspace writes as a primitive or bytes.
    primitive_reprs: BTreeSet<(String, String)>,
}

/// Whether a type is a primitive, or a borrow, a box or a slice of one.
fn inner_like(ty: &syn::Type) -> bool {
    match ty {
        syn::Type::Reference(r) => inner_like(&r.elem),
        syn::Type::Paren(p) => inner_like(&p.elem),
        syn::Type::Group(g) => inner_like(&g.elem),
        syn::Type::Array(a) => is_named(&a.elem, "u8"),
        syn::Type::Slice(s) => is_named(&s.elem, "u8"),
        syn::Type::Path(p) if p.qself.is_none() => {
            let Some(last) = p.path.segments.last() else {
                return false;
            };
            let name = last.ident.to_string();
            if PRIMITIVES.contains(&name.as_str()) {
                return true;
            }
            (name == "Vec" || name == "Box") && first_argument(last).is_some_and(inner_like)
        }
        _ => false,
    }
}

/// Whether a type is the bare path `name`.
fn is_named(ty: &syn::Type, name: &str) -> bool {
    matches!(ty, syn::Type::Path(p) if p.qself.is_none() && p.path.is_ident(name))
}

/// The first type argument of a path segment.
fn first_argument(segment: &syn::PathSegment) -> Option<&syn::Type> {
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    args.args.iter().find_map(|arg| match arg {
        syn::GenericArgument::Type(ty) => Some(ty),
        _ => None,
    })
}

/// The name of a type for the receiver it types: the last segment, through
/// a borrow, a `Box`, an `Arc` or an `Rc`.
fn type_name(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Reference(r) => type_name(&r.elem),
        syn::Type::Paren(p) => type_name(&p.elem),
        syn::Type::Group(g) => type_name(&g.elem),
        syn::Type::Path(p) if p.qself.is_none() => {
            let last = p.path.segments.last()?;
            let name = last.ident.to_string();
            if ["Box", "Arc", "Rc"].contains(&name.as_str()) {
                first_argument(last).and_then(type_name)
            } else {
                Some(name)
            }
        }
        _ => None,
    }
}

/// The path of a type for the receiver it types, as [`type_name`] reads
/// it: through a borrow, a `Box`, an `Arc` or an `Rc`.
fn type_path(ty: &syn::Type) -> Option<Vec<String>> {
    match ty {
        syn::Type::Reference(r) => type_path(&r.elem),
        syn::Type::Paren(p) => type_path(&p.elem),
        syn::Type::Group(g) => type_path(&g.elem),
        syn::Type::Path(p) if p.qself.is_none() => {
            let last = p.path.segments.last()?;
            if ["Box", "Arc", "Rc"].contains(&last.ident.to_string().as_str()) {
                first_argument(last).and_then(type_path)
            } else {
                Some(segments_of(&p.path))
            }
        }
        _ => None,
    }
}

/// The path of [`type_path`], with `[]` for a slice or an array and `()`
/// for a tuple: none of them is a newtype.
fn shape_path(ty: &syn::Type) -> Option<Vec<String>> {
    match ty {
        syn::Type::Reference(r) => shape_path(&r.elem),
        syn::Type::Paren(p) => shape_path(&p.elem),
        syn::Type::Group(g) => shape_path(&g.elem),
        syn::Type::Slice(_) | syn::Type::Array(_) => Some(vec!["[]".to_owned()]),
        syn::Type::Tuple(_) => Some(vec!["()".to_owned()]),
        other => type_path(other),
    }
}

/// The idents of a path, in order.
fn segments_of(path: &syn::Path) -> Vec<String> {
    path.segments.iter().map(|s| s.ident.to_string()).collect()
}

/// The expression under a borrow, a paren or a group.
fn peel(expr: &syn::Expr) -> &syn::Expr {
    match expr {
        syn::Expr::Reference(r) => peel(&r.expr),
        syn::Expr::Paren(p) => peel(&p.expr),
        syn::Expr::Group(g) => peel(&g.expr),
        other => other,
    }
}

/// The name of a field: its ident, or the index of a tuple field.
fn member_name(member: &syn::Member) -> String {
    match member {
        syn::Member::Named(ident) => ident.to_string(),
        syn::Member::Unnamed(index) => index.index.to_string(),
    }
}

/// The name of the associated type of a parameter of the impl that `ty`
/// returns, through a borrow: `Repr` for `E::Repr` or `<E as Kind>::Repr`.
fn representation_return(ty: &syn::Type, params: &BTreeSet<String>) -> Option<String> {
    match ty {
        syn::Type::Reference(r) => representation_return(&r.elem, params),
        syn::Type::Paren(p) => representation_return(&p.elem, params),
        syn::Type::Group(g) => representation_return(&g.elem, params),
        syn::Type::Path(p) => {
            let is_param = |ident: &syn::Ident| params.contains(&ident.to_string());
            let segments: Vec<&syn::PathSegment> = p.path.segments.iter().collect();
            match (&p.qself, segments.as_slice()) {
                (Some(qself), [.., assoc]) if assoc.arguments.is_none() => {
                    matches!(&*qself.ty, syn::Type::Path(q)
                        if q.qself.is_none() && q.path.get_ident().is_some_and(is_param))
                    .then(|| assoc.ident.to_string())
                }
                (None, [param, assoc])
                    if param.arguments.is_none()
                        && assoc.arguments.is_none()
                        && is_param(&param.ident) =>
                {
                    Some(assoc.ident.to_string())
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// The type parameters `generics` declares.
fn type_params(generics: &syn::Generics) -> BTreeSet<String> {
    generics
        .type_params()
        .map(|param| param.ident.to_string())
        .collect()
}

/// The representation of a generic newtype, as its accessors return it.
#[derive(Clone, Debug, Default)]
struct GenericRepr {
    /// The name of each associated type the field holds: `Repr` in
    /// `Id<E: Kind>(E::Repr)`.
    assocs: BTreeSet<String>,
}

/// A generic struct whose one field is an associated type of a marker
/// ([`representation_slots`]), before the impls are known.
struct GenericCandidate {
    ty: Ty,
    home: Home,
    tuple: bool,
    /// Each trait, by its last segment, and associated type the field reads.
    slots: BTreeSet<(String, String)>,
    repr: GenericRepr,
}

/// The signature of an impl method, as the catalog reads it.
fn method_of(sig: &syn::Signature, params: &BTreeSet<String>) -> Method {
    let takes_self = matches!(sig.inputs.first(), Some(syn::FnArg::Receiver(_)));
    let (returns_inner, returns_repr, returns_bool) = match &sig.output {
        syn::ReturnType::Type(_, ty) => (
            inner_like(ty),
            representation_return(ty, params),
            is_named(ty, "bool"),
        ),
        syn::ReturnType::Default => (false, None, false),
    };
    Method {
        name: sig.ident.to_string(),
        self_only: takes_self && sig.inputs.len() == 1,
        takes_self,
        returns_inner,
        returns_repr,
        returns_bool,
    }
}

/// Whether the tokens hold `self . <field>` other than a call.
fn reads_field(tokens: TokenStream) -> bool {
    let trees: Vec<TokenTree> = tokens.into_iter().collect();
    trees.iter().enumerate().any(|(i, tree)| match tree {
        TokenTree::Ident(ident) if ident == "self" => {
            matches!(trees.get(i + 1), Some(TokenTree::Punct(p)) if p.as_char() == '.')
                && match trees.get(i + 2) {
                    Some(TokenTree::Literal(_)) => true,
                    Some(TokenTree::Ident(_)) => !matches!(
                        trees.get(i + 3),
                        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis
                    ),
                    _ => false,
                }
        }
        TokenTree::Group(group) => reads_field(group.stream()),
        _ => false,
    })
}

/// Whether the tokens name `len`: a `Display` that prints a length shows no
/// content.
fn names_len(tokens: TokenStream) -> bool {
    tokens.into_iter().any(|tree| match tree {
        TokenTree::Ident(ident) => ident == "len",
        TokenTree::Group(group) => names_len(group.stream()),
        _ => false,
    })
}

/// Every method a `macro_rules!` body writes, and its accessors. A return
/// type is a primitive when it is one by name, or a metavariable of the
/// fragment `ty`.
fn macro_methods(tokens: TokenStream, methods: &mut BTreeSet<String>) -> BTreeSet<String> {
    let mut accessors = BTreeSet::new();
    let trees: Vec<TokenTree> = tokens.into_iter().collect();
    for window in trees.windows(4) {
        if let [
            TokenTree::Group(matcher),
            TokenTree::Punct(eq),
            TokenTree::Punct(gt),
            TokenTree::Group(body),
        ] = window
            && eq.as_char() == '='
            && gt.as_char() == '>'
        {
            let mut fragments = BTreeMap::new();
            fragments_of(matcher.stream(), &mut fragments);
            body_methods(body.stream(), &fragments, methods, &mut accessors);
        }
    }
    accessors
}

/// The methods of one transcriber, at any depth.
fn body_methods(
    stream: TokenStream,
    fragments: &BTreeMap<String, String>,
    methods: &mut BTreeSet<String>,
    accessors: &mut BTreeSet<String>,
) {
    let trees: Vec<TokenTree> = stream.into_iter().collect();
    for (i, tree) in trees.iter().enumerate() {
        if let TokenTree::Group(group) = tree {
            body_methods(group.stream(), fragments, methods, accessors);
            continue;
        }
        let (TokenTree::Ident(word), Some(TokenTree::Ident(name)), Some(TokenTree::Group(params))) =
            (tree, trees.get(i + 1), trees.get(i + 2))
        else {
            continue;
        };
        if word != "fn" || params.delimiter() != Delimiter::Parenthesis {
            continue;
        }
        let params: Vec<TokenTree> = params.stream().into_iter().collect();
        let takes_self = params
            .iter()
            .take_while(|tree| !matches!(tree, TokenTree::Punct(p) if p.as_char() == ','))
            .any(|tree| matches!(tree, TokenTree::Ident(ident) if ident == "self"));
        if !takes_self {
            continue;
        }
        let name = name.to_string();
        methods.insert(name.clone());
        let more = params
            .iter()
            .position(|tree| matches!(tree, TokenTree::Punct(p) if p.as_char() == ','))
            .is_some_and(|comma| comma + 1 < params.len());
        let ret: Vec<&TokenTree> = trees
            .iter()
            .skip(i + 3)
            .skip_while(|tree| matches!(tree, TokenTree::Punct(p) if p.as_char() == '-' || p.as_char() == '>'))
            .take_while(|tree| match tree {
                TokenTree::Group(g) => g.delimiter() != Delimiter::Brace,
                TokenTree::Ident(ident) => ident != "where",
                TokenTree::Punct(p) => p.as_char() != ';',
                TokenTree::Literal(_) => true,
            })
            .collect();
        let arrow = matches!(trees.get(i + 3), Some(TokenTree::Punct(p)) if p.as_char() == '-');
        let returns_bool = matches!(
            ret.as_slice(),
            [TokenTree::Ident(ident)] if ident == "bool"
        );
        let method = Method {
            name,
            self_only: !more,
            takes_self,
            returns_inner: arrow && returns_inner(&ret, fragments),
            returns_repr: None,
            returns_bool,
        };
        if method.is_accessor() {
            accessors.insert(method.name);
        }
    }
}

/// Whether the tokens of a return type name a primitive, a borrow and a
/// lifetime set aside.
fn returns_inner(ret: &[&TokenTree], fragments: &BTreeMap<String, String>) -> bool {
    let mut core: Vec<&TokenTree> = Vec::new();
    let mut lifetime = false;
    for tree in ret {
        match tree {
            TokenTree::Punct(p) if p.as_char() == '&' && core.is_empty() => {}
            TokenTree::Punct(p) if p.as_char() == '\'' && core.is_empty() => lifetime = true,
            TokenTree::Ident(ident) if core.is_empty() && (lifetime || ident == "mut") => {
                lifetime = false;
            }
            _ => core.push(tree),
        }
    }
    match core.as_slice() {
        [TokenTree::Ident(ident)] => PRIMITIVES.contains(&ident.to_string().as_str()),
        [TokenTree::Punct(dollar), TokenTree::Ident(meta)] if dollar.as_char() == '$' => {
            fragments.get(&meta.to_string()).is_some_and(|f| f == "ty")
        }
        [TokenTree::Group(group)] if group.delimiter() == Delimiter::Bracket => group
            .stream()
            .into_iter()
            .next()
            .is_some_and(|first| matches!(first, TokenTree::Ident(ident) if ident == "u8")),
        _ => {
            let text: String = core.iter().map(ToString::to_string).collect();
            text == "Vec<u8>"
        }
    }
}

/// The first pass over one file.
struct CatalogVisit<'r> {
    phase: Phase,
    module: Vec<String>,
    scope: Scope,
    raw: &'r mut Raw,
}

/// The two passes over the items of a file. The second resolves a type
/// through what the first declared in every crate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Declare,
    Catalog,
}

impl CatalogVisit<'_> {
    fn home(&self) -> Home {
        Home {
            krate: self.scope.krate.clone(),
            module: self.module.join("::"),
        }
    }

    /// The type a path names in this file. A name the report cannot place
    /// reads as foreign: no workspace type answers it.
    fn place(&self, segments: &[String]) -> Ty {
        place(&self.raw.catalog, &self.scope, segments)
    }
}

/// The type a path names in `scope`, foreign when the catalog cannot place
/// it.
fn place(catalog: &Catalog, scope: &Scope, segments: &[String]) -> Ty {
    catalog
        .resolve(scope, segments, &catalog.declared)
        .unwrap_or_else(|| Ty::foreign(segments.last().map_or("", String::as_str)))
}

impl<'ast> Visit<'ast> for CatalogVisit<'_> {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        if test_only(item_attrs(item)) {
            return;
        }
        let declare = self.phase == Phase::Declare;
        let krate = self.scope.krate.clone();
        match item {
            syn::Item::Mod(m) => {
                let Some((_, items)) = &m.content else {
                    return;
                };
                if declare {
                    self.raw.catalog.modules.insert(m.ident.to_string());
                }
                self.module.push(m.ident.to_string());
                for item in items {
                    self.visit_item(item);
                }
                self.module.pop();
            }
            syn::Item::Struct(s) => {
                let name = s.ident.to_string();
                if name.starts_with(META) {
                    return;
                }
                if declare {
                    self.raw.catalog.declare(name, &krate);
                    return;
                }
                let ty = Ty::of(&krate, &name);
                let fields: Vec<&syn::Field> = s
                    .fields
                    .iter()
                    .filter(|field| type_name(&field.ty).is_none_or(|ty| ty != "PhantomData"))
                    .collect();
                let named: BTreeMap<String, Ty> = s
                    .fields
                    .iter()
                    .enumerate()
                    .filter_map(|(index, field)| {
                        let name = field
                            .ident
                            .as_ref()
                            .map_or_else(|| index.to_string(), ToString::to_string);
                        Some((name, self.place(&shape_path(&field.ty)?)))
                    })
                    .collect();
                let catalog = &mut self.raw.catalog;
                for (field, ty) in &named {
                    catalog
                        .field_types
                        .entry(field.clone())
                        .or_default()
                        .insert(ty.clone());
                }
                catalog.fields.entry(ty.clone()).or_default().extend(named);
                if let [field] = fields.as_slice() {
                    let tuple =
                        field.ident.is_none() && !matches!(field.vis, syn::Visibility::Inherited);
                    let slots = representation_slots(&s.generics, &field.ty);
                    if inner_like(&field.ty) {
                        let home = self.home();
                        let newtype = self.raw.catalog.newtypes.entry(ty).or_default();
                        newtype.homes.insert(home);
                        newtype.tuple |= tuple;
                    } else if !slots.is_empty() {
                        // A newtype only once an impl writes a primitive for
                        // one of its slots: `Handle<K: Kind>(K::Repr)` over a
                        // `Config` representation holds no primitive.
                        let assocs = slots.iter().map(|(_, assoc)| assoc.clone()).collect();
                        let slots = slots
                            .into_iter()
                            .filter_map(|(path, assoc)| Some((path.last()?.clone(), assoc)))
                            .collect();
                        let home = self.home();
                        self.raw.generic.push(GenericCandidate {
                            ty,
                            home,
                            tuple,
                            slots,
                            repr: GenericRepr { assocs },
                        });
                    }
                }
            }
            // A name a transcriber writes as a metavariable names no item.
            syn::Item::Enum(syn::ItemEnum { ident, .. })
            | syn::Item::Union(syn::ItemUnion { ident, .. })
            | syn::Item::Type(syn::ItemType { ident, .. })
            | syn::Item::Trait(syn::ItemTrait { ident, .. })
            | syn::Item::Fn(syn::ItemFn {
                sig: syn::Signature { ident, .. },
                ..
            })
            | syn::Item::Const(syn::ItemConst { ident, .. })
            | syn::Item::Static(syn::ItemStatic { ident, .. })
                if ident.to_string().starts_with(META) => {}
            syn::Item::Enum(syn::ItemEnum { ident, .. })
            | syn::Item::Union(syn::ItemUnion { ident, .. })
            | syn::Item::Type(syn::ItemType { ident, .. })
                if declare =>
            {
                self.raw.catalog.declare(ident.to_string(), &krate);
            }
            syn::Item::Trait(t) if declare => {
                self.raw.catalog.declare(t.ident.to_string(), &krate);
            }
            syn::Item::Trait(t) => {
                self.raw.traits.insert(t.ident.to_string());
                let catalog = &mut self.raw.catalog;
                for item in &t.items {
                    if let syn::TraitItem::Fn(f) = item
                        && matches!(f.sig.inputs.first(), Some(syn::FnArg::Receiver(_)))
                    {
                        catalog.methods.add(f.sig.ident.to_string(), &krate);
                        catalog
                            .declared_methods
                            .add(f.sig.ident.to_string(), &krate);
                    }
                }
            }
            syn::Item::Fn(f) if !declare => {
                let ret = Ret::of(&f.sig.output, None, |path| self.place(path));
                let catalog = &mut self.raw.catalog;
                catalog.fns.add(f.sig.ident.to_string(), &krate);
                if let Some(ret) = ret {
                    returned(&mut catalog.returns, f.sig.ident.to_string(), ret);
                }
            }
            syn::Item::Const(syn::ItemConst { ident, ty, .. })
            | syn::Item::Static(syn::ItemStatic { ident, ty, .. }) => {
                if declare {
                    self.raw.catalog.values.add(ident.to_string(), &krate);
                } else if let Some(path) = shape_path(ty) {
                    let ty = self.place(&path);
                    self.raw
                        .catalog
                        .consts
                        .insert(Ty::of(&krate, &ident.to_string()), ty);
                }
            }
            syn::Item::Impl(i) if !declare => self.visit_impl(i),
            syn::Item::Macro(m) => {
                let Some(name) = m.mac.path.segments.last().map(|s| s.ident.to_string()) else {
                    return;
                };
                if name == "macro_rules" {
                    if let Some(ident) = &m.ident
                        && !declare
                    {
                        let mut methods = BTreeSet::new();
                        let accessors = macro_methods(m.mac.tokens.clone(), &mut methods);
                        for method in methods {
                            self.raw.catalog.methods.add(method, &krate);
                        }
                        self.raw.macros.insert(ident.to_string(), accessors);
                    }
                    // A type a transcriber names as written, as in
                    // `struct Fields`, is catalogued. A type it names by a
                    // metavariable is not.
                    for body in rule_bodies(m.mac.tokens.clone()) {
                        if let Ok(file) = syn::parse2::<syn::File>(body) {
                            for item in &file.items {
                                self.visit_item(item);
                            }
                        }
                    }
                } else if writes_newtypes(&name) {
                    let mut types = macro_types(&m.mac);
                    types.retain(|ty| !ty.starts_with(META));
                    if declare {
                        for ty in types {
                            self.raw.catalog.declare(ty, &krate);
                        }
                        return;
                    }
                    let home = self.home();
                    self.raw.invocations.push(Invocation {
                        family: name,
                        types,
                        home,
                    });
                }
            }
            _ => {}
        }
    }
}

/// Files the return of the `fn` `key`. Two different returns under one key
/// cancel each other.
fn returned<K: Ord>(returns: &mut BTreeMap<K, Option<Ret>>, key: K, ret: Ret) {
    returns
        .entry(key)
        .and_modify(|known| {
            if known.as_ref() != Some(&ret) {
                *known = None;
            }
        })
        .or_insert_with(|| Some(ret.clone()));
}

impl CatalogVisit<'_> {
    /// Catalogues one impl block: the returns and boundaries of its
    /// methods, and what the impl tells of its owner.
    fn visit_impl(&mut self, i: &syn::ItemImpl) {
        if !test_only(&i.attrs)
            && let Some((_, trait_path, _)) = &i.trait_
            && let Some(trait_name) = trait_path.segments.last()
        {
            for item in &i.items {
                if let syn::ImplItem::Type(t) = item
                    && !test_only(&t.attrs)
                    && inner_like(&t.ty)
                {
                    self.raw
                        .primitive_reprs
                        .insert((trait_name.ident.to_string(), t.ident.to_string()));
                }
            }
        }
        let Some(owner) = type_path(&i.self_ty)
            .filter(|path| path.last().is_some_and(|last| !last.starts_with(META)))
            .map(|path| self.place(&path))
        else {
            return;
        };
        for item in &i.items {
            if let syn::ImplItem::Fn(f) = item
                && !test_only(&f.attrs)
            {
                let key = (owner.clone(), f.sig.ident.to_string());
                let ret = Ret::of(&f.sig.output, Some(&owner), |path| self.place(path));
                let into = into_params(&f.sig, &self.scope.uses.names);
                let catalog = &mut self.raw.catalog;
                if i.trait_.is_none() && !into.is_empty() {
                    catalog.boundaries.insert(key.clone(), into);
                }
                if let Some(ret) = ret {
                    returned(&mut catalog.method_returns, key, ret);
                }
            }
        }
        let last = i
            .trait_
            .as_ref()
            .and_then(|(_, path, _)| path.segments.last());
        let of_trait = last.map(|last| last.ident.to_string());
        let argument = last
            .and_then(first_argument)
            .and_then(|ty| Argument::of(ty, |path| self.place(path)));
        let inner = inner_like(&i.self_ty).then(|| owner.name.clone());
        // A method generic over its own parameters returns no
        // representation: only the impl's parameters count.
        let params = type_params(&i.generics);
        let mut shows_content = false;
        let methods = i
            .items
            .iter()
            .filter_map(|item| match item {
                syn::ImplItem::Fn(f) if !test_only(&f.attrs) => {
                    if of_trait.as_deref() == Some("Display") && f.sig.ident == "fmt" {
                        let body = f.block.to_token_stream();
                        shows_content = reads_field(body.clone()) && !names_len(body);
                    }
                    Some(method_of(&f.sig, &params))
                }
                _ => None,
            })
            .collect();
        self.raw.impls.push(Impl {
            owner,
            krate: self.scope.krate.clone(),
            of_trait,
            argument,
            inner,
            methods,
            shows_content,
        });
    }
}

impl Raw {
    /// The catalog, once every newtype is known: the accessors of each, and
    /// the methods of each impl sorted by who owns the trait.
    fn close(mut self) -> Catalog {
        let mut catalog = self.catalog;
        for candidate in self.generic.drain(..) {
            if !candidate
                .slots
                .iter()
                .any(|slot| self.primitive_reprs.contains(slot))
            {
                continue;
            }
            let newtype = catalog.newtypes.entry(candidate.ty).or_default();
            newtype.homes.insert(candidate.home);
            newtype.tuple |= candidate.tuple;
            let repr = newtype.repr.get_or_insert_with(GenericRepr::default);
            repr.assocs.extend(candidate.repr.assocs);
        }
        for invocation in self.invocations {
            let accessors = self
                .macros
                .get(&invocation.family)
                .cloned()
                .unwrap_or_default();
            for ty in invocation.types {
                let ty = Ty::of(&invocation.home.krate, &ty);
                let newtype = catalog.newtypes.entry(ty).or_default();
                newtype.homes.insert(invocation.home.clone());
                newtype.accessors.extend(accessors.iter().cloned());
            }
        }
        for imp in self.impls.drain(..) {
            // An inherent impl, or an impl of a workspace trait.
            let own = imp
                .of_trait
                .as_ref()
                .is_none_or(|name| self.traits.contains(name));
            for method in &imp.methods {
                if !method.takes_self {
                    continue;
                }
                if own {
                    catalog.methods.add(method.name.clone(), &imp.krate);
                } else {
                    catalog.trait_methods.add(method.name.clone(), &imp.krate);
                }
            }
            // `impl From<Name> for String`: `name.into()` unwraps `Name`.
            if imp.of_trait.as_deref() == Some("From")
                && let (Some(inner), Some(Argument::Named(source))) = (&imp.inner, &imp.argument)
                && let Some(newtype) = catalog.newtypes.get_mut(source)
            {
                newtype.conversions.insert(inner.clone());
                newtype.accessors.insert("into".to_owned());
            }
            let Some(newtype) = catalog.newtypes.get_mut(&imp.owner) else {
                continue;
            };
            match (imp.of_trait.as_deref(), &imp.argument) {
                (Some("Into"), Some(Argument::Inner(target))) => {
                    newtype.conversions.extend(target.clone());
                    newtype.accessors.insert("into".to_owned());
                }
                (Some("AsRef" | "Borrow"), Some(Argument::Inner(_))) => {
                    newtype.accessors.extend(
                        imp.methods
                            .iter()
                            .filter(|method| method.self_only)
                            .map(|method| method.name.clone()),
                    );
                }
                _ => {}
            }
            if imp.of_trait.is_none() {
                let repr = newtype.repr.clone();
                newtype.accessors.extend(
                    imp.methods
                        .iter()
                        .filter(|method| method.is_accessor_of(repr.as_ref()))
                        .map(|method| method.name.clone()),
                );
            }
            if imp.shows_content {
                newtype.accessors.insert("to_string".to_owned());
            }
        }
        catalog
    }
}

/// How the report reads a callee.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Callee {
    Workspace,
    Foreign,
    Ambiguous,
    /// A tuple struct or a variant: it takes the value, and calls nothing.
    Constructor,
}

/// How sure the report is of the newtype of an unwrap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Receiver {
    /// The receiver is typed, or the accessor name is the workspace's own.
    Known,
    /// A shared accessor on a receiver the report cannot type.
    Unknown,
}

/// One unwrap, before the call it is handed to.
#[derive(Clone, Debug)]
struct Unwrap {
    newtype: String,
    accessor: String,
    receiver: Receiver,
}

/// What a name in scope is known to hold.
#[derive(Clone, Debug, Default)]
struct Binding {
    ty: Option<Ty>,
    unwrap: Option<Unwrap>,
}

/// The second pass over one file.
struct BodyVisit<'c, 'r> {
    catalog: &'c Catalog,
    scope_of: Scope,
    file: String,
    module: Vec<String>,
    /// The item path inside the module: an impl owner, then a `fn`.
    scope: Vec<String>,
    /// The owner of each enclosing impl, the innermost last.
    owners: Vec<Ty>,
    /// For each enclosing impl, the newtypes it may unwrap: its owner and
    /// the type a conversion trait names.
    exempt: Vec<BTreeSet<Ty>>,
    /// The type of the parameter of the next closure, as `map_err` gives
    /// its closure the error.
    closure_hint: Option<Ty>,
    env: Vec<BTreeMap<String, Binding>>,
    report: &'r mut Report,
}

impl BodyVisit<'_, '_> {
    fn module(&self) -> String {
        self.module.join("::")
    }

    /// The crate of the file.
    fn krate(&self) -> &str {
        &self.scope_of.krate
    }

    /// The first segment of the path a `use` of the file imports `name`
    /// from.
    fn import(&self, name: &str) -> Option<&String> {
        self.scope_of.uses.names.get(name)
    }

    /// The type a path names in the file, `Self` read as the impl owner.
    /// A name the report cannot place reads as foreign.
    fn place(&self, segments: &[String]) -> Ty {
        match segments {
            [only] if only == "Self" => self
                .owners
                .last()
                .cloned()
                .unwrap_or_else(|| Ty::foreign(only)),
            _ => place(self.catalog, &self.scope_of, segments),
        }
    }

    /// The type a path names in the file, `None` when the report cannot
    /// place it.
    fn resolve(&self, segments: &[String]) -> Option<Ty> {
        match segments {
            [only] if only == "Self" => self.owners.last().cloned(),
            _ => self
                .catalog
                .resolve(&self.scope_of, segments, &self.catalog.declared),
        }
    }

    /// The error type a bare `Err(e)` or a `map_err` closure binds.
    fn error(&self) -> Ty {
        self.place(&[ERROR.to_owned()])
    }

    fn lookup(&self, name: &str) -> Option<&Binding> {
        self.env.iter().rev().find_map(|scope| scope.get(name))
    }

    fn bind(&mut self, name: String, binding: Binding) {
        if let Some(scope) = self.env.last_mut() {
            scope.insert(name, binding);
        }
    }

    /// Binds every name a pattern brings in. A lone name takes what the
    /// annotation or the initialiser says of it; any other name is unknown.
    fn bind_pattern(&mut self, pat: &syn::Pat, init: Option<&syn::Expr>) {
        let (name, ty) = match pat {
            syn::Pat::Ident(p) if p.subpat.is_none() => (Some(p.ident.to_string()), None),
            syn::Pat::Type(t) => match &*t.pat {
                syn::Pat::Ident(p) if p.subpat.is_none() => {
                    (Some(p.ident.to_string()), Some(&*t.ty))
                }
                _ => (None, None),
            },
            _ => (None, None),
        };
        if let Some(name) = name {
            let ty = match ty {
                Some(ty) => self.hint(ty),
                None => init.and_then(|init| self.infer(init)),
            };
            let unwrap = init.and_then(|init| self.unwrap_of(init));
            self.bind(name, Binding { ty, unwrap });
            return;
        }
        let mut names = PatNames {
            visit: self,
            within: None,
            names: Vec::new(),
        };
        names.visit_pat(pat);
        for (name, ty) in names.names {
            self.bind(name, Binding { ty, unwrap: None });
        }
    }

    /// The name of a written type, `Self` read as the impl owner.
    fn hint(&self, ty: &syn::Type) -> Option<Ty> {
        type_path(ty).map(|path| self.place(&path))
    }

    /// The type name of an expression, where the syntax states it.
    fn infer(&self, expr: &syn::Expr) -> Option<Ty> {
        match expr {
            syn::Expr::Path(p) if p.qself.is_none() => {
                let ident = p.path.get_ident()?;
                if ident == "self" {
                    return self.owners.last().cloned();
                }
                let name = ident.to_string();
                match self.lookup(&name) {
                    Some(binding) => binding.ty.clone(),
                    None => {
                        let segments = [name];
                        let item = self.catalog.resolve(
                            &self.scope_of,
                            &segments,
                            &self.catalog.values,
                        )?;
                        self.catalog.consts.get(&item).cloned()
                    }
                }
            }
            syn::Expr::Field(f) => {
                let owner = self.infer(&f.base)?;
                self.catalog
                    .fields
                    .get(&owner)?
                    .get(&member_name(&f.member))
                    .cloned()
            }
            syn::Expr::Array(_) => Some(Ty::foreign("[]")),
            syn::Expr::Lit(_) => Some(Ty::foreign("literal")),
            syn::Expr::Macro(m) => {
                let name = m.mac.path.segments.last()?.ident.to_string();
                MACRO_TYPES
                    .iter()
                    .find(|(mac, _)| *mac == name)
                    .map(|(_, ty)| Ty::foreign(ty))
            }
            syn::Expr::MethodCall(m) => {
                let method = m.method.to_string();
                match method.as_str() {
                    "clone" | "to_owned" => return self.infer(&m.receiver),
                    // `unwrap_or` yields its argument's type.
                    "unwrap_or" => return m.args.first().and_then(|arg| self.infer(arg)),
                    _ => {}
                }
                if let Some(ret) = self.ret_of(expr) {
                    return Some(ret.ty.clone());
                }
                let fixed = STD_RETURNS.contains(&method.as_str())
                    || !self.catalog.methods.within(&method, self.reach());
                RETURNS
                    .iter()
                    .find(|(name, _)| *name == method)
                    .filter(|_| fixed)
                    .map(|(_, ty)| Ty::foreign(ty))
            }
            syn::Expr::Reference(r) => self.infer(&r.expr),
            syn::Expr::Paren(p) => self.infer(&p.expr),
            syn::Expr::Group(g) => self.infer(&g.expr),
            syn::Expr::Unary(u) if matches!(u.op, syn::UnOp::Deref(_)) => self.infer(&u.expr),
            syn::Expr::Try(t) => match self.ret_of(&t.expr) {
                Some(ret) => ret.tried.clone(),
                None => self.infer(&t.expr),
            },
            syn::Expr::Struct(s) => Some(self.place(&segments_of(&s.path))),
            syn::Expr::Call(c) => {
                let syn::Expr::Path(p) = &*c.func else {
                    return None;
                };
                let segments = segments_of(&p.path);
                if let Some(ret) = self.ret_of(expr) {
                    return Some(ret.ty.clone());
                }
                let [path @ .., owner, ctor] = segments.as_slice() else {
                    return None;
                };
                // An associated `fn` of a foreign type, as
                // `uuid::Uuid::new_v4`, returns no workspace newtype.
                if owner.starts_with(|c: char| c.is_ascii_uppercase())
                    && self.foreign_path(&segments)
                {
                    return Some(Ty::foreign(owner));
                }
                let builds = ctor == "new"
                    || ctor.starts_with("from")
                    || ctor.starts_with("try_from")
                    || ctor == "parse"
                    || ctor == "default"
                    || ctor == "with_capacity";
                if !builds {
                    return None;
                }
                if STD_TYPES.contains(&owner.as_str()) && path.is_empty() {
                    return Some(Ty::foreign(owner));
                }
                // A constructor of a type a foreign crate exports builds
                // that foreign type, as `RwSignal::new` does.
                let owner_path: Vec<String> = path.iter().chain([owner]).cloned().collect();
                self.resolve(&owner_path)
            }
            _ => None,
        }
    }

    /// The return the catalog knows for a call or a method call.
    fn ret_of(&self, expr: &syn::Expr) -> Option<&Ret> {
        match expr {
            syn::Expr::Call(c) => {
                let syn::Expr::Path(p) = &*c.func else {
                    return None;
                };
                let segments: Vec<String> = p
                    .path
                    .segments
                    .iter()
                    .map(|s| s.ident.to_string())
                    .collect();
                match segments.as_slice() {
                    // A `use` of a foreign `fn`, as `serde_json::from_str`,
                    // shadows a workspace `fn` of the same name.
                    [name] if self.lookup(name).is_none() && !self.foreign_path(&segments) => {
                        self.catalog.ret(name)
                    }
                    [path @ .., owner, name]
                        if owner.starts_with(|c: char| c.is_ascii_uppercase()) =>
                    {
                        let owner_path: Vec<String> = path.iter().chain([owner]).cloned().collect();
                        self.catalog.method_ret(&self.resolve(&owner_path)?, name)
                    }
                    [.., name] if !self.foreign_path(&segments) => self.catalog.ret(name),
                    _ => None,
                }
            }
            syn::Expr::MethodCall(m) => {
                let owner = self.infer(&m.receiver)?;
                self.catalog.method_ret(&owner, &m.method.to_string())
            }
            _ => None,
        }
    }

    /// The newtype a named conversion takes. Such a conversion is an
    /// associated `fn` that returns `Self` or the impl owner. Its one
    /// parameter is a workspace newtype, by value or by reference.
    fn named_conversion(&self, sig: &syn::Signature) -> Option<Ty> {
        let mut inputs = sig.inputs.iter();
        let (Some(syn::FnArg::Typed(input)), None) = (inputs.next(), inputs.next()) else {
            return None;
        };
        let syn::ReturnType::Type(_, ret) = &sig.output else {
            return None;
        };
        let returned = self.hint(ret)?;
        if self.owners.last() != Some(&returned) {
            return None;
        }
        self.hint(&input.ty)
            .filter(|ty| self.catalog.newtypes.contains_key(ty))
    }

    /// The crates the current crate reaches, `None` for all of them.
    fn reach(&self) -> Option<&BTreeSet<String>> {
        self.catalog.reach.get(self.krate())
    }

    /// Whether the site may unwrap the newtype: the current module declares
    /// it, or an enclosing impl owns or converts it.
    fn exempts(&self, ty: &Ty) -> bool {
        let here = Home {
            krate: self.krate().to_owned(),
            module: self.module(),
        };
        let home = self
            .catalog
            .newtypes
            .get(ty)
            .is_some_and(|newtype| newtype.homes.contains(&here));
        home || self.exempt.iter().any(|types| types.contains(ty))
    }

    /// The unwrap an accessor call on `receiver` makes, if any.
    fn attribute(&self, receiver: &syn::Expr, accessor: &str) -> Option<Unwrap> {
        if self.unwrap_of(receiver).is_some() {
            return None;
        }
        let candidates: Vec<&Ty> = self
            .catalog
            .newtypes
            .iter()
            .filter(|(_, newtype)| newtype.accessors.contains(accessor))
            .map(|(ty, _)| ty)
            .collect();
        if candidates.is_empty() {
            return None;
        }
        if let Some(ty) = self.infer(receiver) {
            let ty = candidates.into_iter().find(|candidate| **candidate == ty)?;
            if self.exempts(ty) {
                return None;
            }
            return Some(Unwrap {
                newtype: ty.name.clone(),
                accessor: accessor.to_owned(),
                receiver: Receiver::Known,
            });
        }
        // A field keeps the check below: a struct of the workspace may hold
        // a newtype with its own `as_ref` there.
        if CONVERSIONS.contains(&accessor) && !matches!(peel(receiver), syn::Expr::Field(_)) {
            return None;
        }
        // Take a field of a base the report cannot type. It holds a newtype
        // only when a workspace struct has a field of that name and type. A
        // field of a syn or proc-macro2 value holds none.
        let candidates: Vec<&Ty> = match peel(receiver) {
            syn::Expr::Field(f) => {
                let held = self.catalog.field_types.get(&member_name(&f.member));
                candidates
                    .into_iter()
                    .filter(|ty| held.is_some_and(|types| types.contains(*ty)))
                    .collect()
            }
            _ => candidates,
        };
        if candidates.is_empty() {
            return None;
        }
        let exempt = candidates.iter().filter(|ty| self.exempts(ty)).count();
        if exempt == candidates.len() {
            return None;
        }
        // The key names a newtype by its name: two crates that declare one
        // name show it once.
        let names: BTreeSet<&str> = candidates.iter().map(|ty| ty.name.as_str()).collect();
        let newtype = match names.len() {
            few if few <= FEW => names.into_iter().collect::<Vec<_>>().join("|"),
            _ => "*".to_owned(),
        };
        // An exempt candidate, a std name or a name many newtypes share
        // leaves the receiver open.
        let shared = SHARED_ACCESSORS.contains(&accessor) || foreign_method(accessor);
        let receiver = if exempt > 0 || candidates.len() > FEW || shared {
            Receiver::Unknown
        } else {
            Receiver::Known
        };
        Some(Unwrap {
            newtype,
            accessor: accessor.to_owned(),
            receiver,
        })
    }

    /// The `.0` of a typed newtype receiver, outside its module.
    fn tuple_field(&self, field: &syn::ExprField) -> Option<Unwrap> {
        if !matches!(&field.member, syn::Member::Unnamed(index) if index.index == 0) {
            return None;
        }
        let ty = self.infer(&field.base)?;
        let newtype = self.catalog.newtypes.get(&ty)?;
        if !newtype.tuple || self.exempts(&ty) {
            return None;
        }
        Some(Unwrap {
            newtype: ty.name,
            accessor: "0".to_owned(),
            receiver: Receiver::Known,
        })
    }

    /// The `String::from(name)` of a newtype that converts into the
    /// primitive the path names.
    fn conversion(&self, call: &syn::ExprCall) -> Option<Unwrap> {
        let syn::Expr::Path(path) = &*call.func else {
            return None;
        };
        let mut segments = path.path.segments.iter().rev();
        let (Some(from), Some(target)) = (segments.next(), segments.next()) else {
            return None;
        };
        let mut args = call.args.iter();
        let (Some(arg), None) = (args.next(), args.next()) else {
            return None;
        };
        if from.ident != "from" {
            return None;
        }
        let ty = self.infer(arg)?;
        let newtype = self.catalog.newtypes.get(&ty)?;
        if !newtype.conversions.contains(&target.ident.to_string()) || self.exempts(&ty) {
            return None;
        }
        Some(Unwrap {
            newtype: ty.name,
            accessor: "from".to_owned(),
            receiver: Receiver::Known,
        })
    }

    /// The unwrap an expression hands on, through the conversions and the
    /// bindings that keep it.
    fn unwrap_of(&self, expr: &syn::Expr) -> Option<Unwrap> {
        match expr {
            syn::Expr::Reference(r) => self.unwrap_of(&r.expr),
            syn::Expr::Paren(p) => self.unwrap_of(&p.expr),
            syn::Expr::Group(g) => self.unwrap_of(&g.expr),
            syn::Expr::MethodCall(m) if m.args.is_empty() => {
                let method = m.method.to_string();
                if PEELS.contains(&method.as_str())
                    && let Some(unwrap) = self.unwrap_of(&m.receiver)
                {
                    return Some(unwrap);
                }
                self.attribute(&m.receiver, &method)
            }
            syn::Expr::Field(f) => self.tuple_field(f),
            syn::Expr::Call(c) => self.conversion(c),
            syn::Expr::Path(p) if p.qself.is_none() => {
                let ident = p.path.get_ident()?;
                self.lookup(&ident.to_string())?.unwrap.clone()
            }
            _ => None,
        }
    }

    /// Whether the first segment of a path stays in the workspace.
    fn workspace_root(&self, root: &str) -> bool {
        self.catalog.workspace_root(root)
    }

    /// Whether a path is an import from a foreign crate, or starts at one.
    fn foreign_path(&self, segments: &[String]) -> bool {
        let Some(first) = segments.first() else {
            return false;
        };
        match self.import(first) {
            Some(root) => !self.workspace_root(root),
            None => {
                segments.len() > 1
                    && !self.workspace_root(first)
                    && !self.catalog.types.contains(first)
            }
        }
    }

    /// How the report reads the callee of a call through a path.
    fn path_callee(&self, path: &syn::ExprPath) -> Callee {
        if path.qself.is_some() {
            return Callee::Foreign;
        }
        let segments: Vec<String> = path
            .path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect();
        let (Some(first), Some(last)) = (segments.first(), segments.last()) else {
            return Callee::Foreign;
        };
        let foreign = self.foreign_path(&segments);
        if last.starts_with(|c: char| c.is_ascii_uppercase()) {
            return if foreign {
                Callee::Constructor
            } else {
                self.constructor(&segments)
            };
        }
        if foreign {
            return Callee::Foreign;
        }
        let owner = segments
            .len()
            .checked_sub(2)
            .and_then(|at| segments.get(at));
        match owner {
            Some(owner) if owner == "Self" => Callee::Workspace,
            Some(owner) if owner.starts_with(|c: char| c.is_ascii_uppercase()) => {
                let owner_path = segments.split_last().map_or(&[][..], |(_, path)| path);
                self.type_callee(owner_path)
            }
            Some(_) => Callee::Workspace,
            None if self.lookup(first).is_none()
                && self.catalog.fns.within(first, self.reach()) =>
            {
                Callee::Workspace
            }
            None => Callee::Foreign,
        }
    }

    /// How the report reads a call through a capitalised path. The tuple
    /// struct of a workspace newtype builds it as `T::new` does, so it is
    /// a workspace callee. A variant, or any other tuple struct, calls
    /// nothing.
    fn constructor(&self, segments: &[String]) -> Callee {
        let variant = segments
            .len()
            .checked_sub(2)
            .and_then(|at| segments.get(at))
            .is_some_and(|owner| owner.starts_with(|c: char| c.is_ascii_uppercase()));
        let newtype = self
            .resolve(segments)
            .is_some_and(|ty| self.catalog.newtypes.contains_key(&ty));
        match segments.last() {
            Some(_) if !variant && newtype => Callee::Workspace,
            _ => Callee::Constructor,
        }
    }

    /// How the report reads an associated `fn` of the type that
    /// `owner_path` names. The type the path resolves to decides. A name
    /// the report cannot place falls back on its name. A name that both a
    /// workspace type and a foreign re-export carry is ambiguous.
    fn type_callee(&self, owner_path: &[String]) -> Callee {
        if let Some(ty) = self.resolve(owner_path) {
            return if ty.is_workspace() {
                Callee::Workspace
            } else {
                Callee::Foreign
            };
        }
        let Some(owner) = owner_path.last() else {
            return Callee::Foreign;
        };
        let declared = self.catalog.types.contains(owner);
        let reexported = self.catalog.foreign_names.contains(owner);
        match (declared, reexported) {
            (true, true) => Callee::Ambiguous,
            (true, false) => Callee::Workspace,
            (false, _) => Callee::Foreign,
        }
    }

    /// How the report reads a method on a receiver.
    fn method_callee(&self, receiver: &syn::Expr, method: &str) -> Callee {
        let workspace = self.catalog.methods.within(method, self.reach());
        if let Some(ty) = self.infer(receiver) {
            return if workspace && ty.is_workspace() {
                Callee::Workspace
            } else {
                Callee::Foreign
            };
        }
        if !workspace {
            Callee::Foreign
        } else if foreign_method(method) || self.catalog.trait_methods.within(method, self.reach())
        {
            Callee::Ambiguous
        } else {
            Callee::Workspace
        }
    }

    /// Files the verdict on one handed unwrap.
    fn record(&mut self, unwrap: Unwrap, callee: Callee, shown: String, line: LineNo) {
        let why = match (callee, unwrap.receiver) {
            (Callee::Foreign, _) => {
                self.report.foreign = self.report.foreign.incremented();
                return;
            }
            (Callee::Constructor, _) => return,
            (Callee::Ambiguous, _) => Some(Why::Callee),
            (Callee::Workspace, Receiver::Unknown) => Some(Why::Receiver),
            (Callee::Workspace, Receiver::Known) => None,
        };
        let finding = Finding {
            krate: self.krate().to_owned(),
            file: self.file.clone(),
            line,
            newtype: unwrap.newtype,
            accessor: unwrap.accessor,
            callee: shown,
            scope: [self.module(), self.scope.join("::")]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join("::"),
            why: why.unwrap_or(Why::Callee),
        };
        match why {
            None => self.report.refusals.push(finding),
            Some(_) => self.report.ambiguous.push(finding),
        }
    }

    /// Visits a `fn` body with a fresh scope that its parameters open.
    fn visit_fn(&mut self, name: &syn::Ident, sig: &syn::Signature, block: &syn::Block) {
        let outer = std::mem::replace(&mut self.env, vec![BTreeMap::new()]);
        self.scope.push(name.to_string());
        for input in &sig.inputs {
            let syn::FnArg::Typed(t) = input else {
                continue;
            };
            if let syn::Pat::Ident(p) = &*t.pat {
                let ty = self.hint(&t.ty);
                self.bind(p.ident.to_string(), Binding { ty, unwrap: None });
            } else {
                self.bind_pattern(&t.pat, None);
            }
        }
        self.visit_block(block);
        self.scope.pop();
        self.env = outer;
    }
}

/// Every name a pattern binds. A name a variant of a foreign `enum` binds,
/// as `TokenTree::Ident(ident)` does, holds a foreign value: the variant
/// path types it.
struct PatNames<'v, 'c, 'r> {
    visit: &'v BodyVisit<'c, 'r>,
    /// The foreign variant the current field belongs to.
    within: Option<Ty>,
    names: Vec<(String, Option<Ty>)>,
}

impl<'ast> Visit<'ast> for PatNames<'_, '_, '_> {
    fn visit_pat_ident(&mut self, p: &'ast syn::PatIdent) {
        self.names.push((p.ident.to_string(), self.within.clone()));
        syn::visit::visit_pat_ident(self, p);
    }

    fn visit_pat_tuple_struct(&mut self, p: &'ast syn::PatTupleStruct) {
        let segments: Vec<String> = p
            .path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect();
        // `Err(e)` binds an error, but `Err(AppError::Conflict(id))` binds
        // what the variant holds: only a direct name is the error.
        if segments == ["Err"] {
            for elem in &p.elems {
                match elem {
                    syn::Pat::Ident(i) if i.subpat.is_none() => self
                        .names
                        .push((i.ident.to_string(), Some(self.visit.error()))),
                    other => self.visit_pat(other),
                }
            }
            return;
        }
        let outer = self.within.clone();
        if self.visit.foreign_path(&segments) {
            self.within = Some(Ty::foreign(&segments.join("::")));
        }
        for elem in &p.elems {
            self.visit_pat(elem);
        }
        self.within = outer;
    }

    fn visit_pat_struct(&mut self, p: &'ast syn::PatStruct) {
        let segments: Vec<String> = p
            .path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect();
        let outer = self.within.clone();
        if self.visit.foreign_path(&segments) {
            self.within = Some(Ty::foreign(&segments.join("::")));
        }
        for field in &p.fields {
            self.visit_pat(&field.pat);
        }
        self.within = outer;
    }
}

/// Every name the `use` items of a file bring in, mapped to the first
/// segment of the path, and the first segment of each glob.
fn imports(items: &[syn::Item], found: &mut Uses) {
    fn tree(t: &syn::UseTree, root: Option<&str>, public: bool, found: &mut Uses) {
        let mut bring = |name: String, root: String| {
            if public {
                found.public.insert(name.clone(), root.clone());
            }
            found.names.insert(name, root);
        };
        match t {
            syn::UseTree::Path(p) => {
                let segment = p.ident.to_string();
                tree(&p.tree, Some(root.unwrap_or(&segment)), public, found);
            }
            syn::UseTree::Name(n) => {
                let name = n.ident.to_string();
                let root = root.unwrap_or(&name).to_owned();
                bring(name, root);
            }
            syn::UseTree::Rename(r) => {
                let name = r.ident.to_string();
                bring(r.rename.to_string(), root.unwrap_or(&name).to_owned());
            }
            syn::UseTree::Glob(_) => {
                if let Some(root) = root {
                    found.globs.insert(root.to_owned());
                }
            }
            syn::UseTree::Group(g) => {
                for item in &g.items {
                    tree(item, root, public, found);
                }
            }
        }
    }
    for item in items {
        match item {
            syn::Item::Use(u) if !test_only(&u.attrs) => {
                let public = !matches!(u.vis, syn::Visibility::Inherited);
                tree(&u.tree, None, public, found);
            }
            syn::Item::Mod(m) if !test_only(&m.attrs) => {
                if let Some((_, items)) = &m.content {
                    imports(items, found);
                }
            }
            _ => {}
        }
    }
}

impl<'ast> Visit<'ast> for BodyVisit<'_, '_> {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        if test_only(item_attrs(item)) {
            return;
        }
        syn::visit::visit_item(self, item);
    }

    fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
        let Some((_, items)) = &m.content else {
            return;
        };
        self.module.push(m.ident.to_string());
        let scope = std::mem::take(&mut self.scope);
        for item in items {
            self.visit_item(item);
        }
        self.scope = scope;
        self.module.pop();
    }

    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        let Some(owner) = self.hint(&i.self_ty) else {
            return;
        };
        // The own impl of a newtype, and a conversion of it, unwrap that
        // newtype freely, and no other.
        let converted = i
            .trait_
            .as_ref()
            .and_then(|(_, path, _)| path.segments.last())
            .and_then(first_argument)
            .and_then(|ty| self.hint(ty));
        let exempt: BTreeSet<Ty> = [Some(owner.clone()), converted]
            .into_iter()
            .flatten()
            .filter(|ty| self.catalog.newtypes.contains_key(ty))
            .collect();
        self.exempt.push(exempt);
        self.scope.push(owner.name.clone());
        self.owners.push(owner);
        for item in &i.items {
            // A named conversion, `fn of_row(delta: RowDelta) -> Self`,
            // unwraps its one newtype freely, as `From<RowDelta>` does.
            let named = match item {
                syn::ImplItem::Fn(f) if i.trait_.is_none() => self.named_conversion(&f.sig),
                _ => None,
            };
            self.exempt.push(named.into_iter().collect());
            self.visit_impl_item(item);
            self.exempt.pop();
        }
        self.scope.pop();
        self.owners.pop();
        self.exempt.pop();
    }

    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        self.visit_fn(&f.sig.ident, &f.sig, &f.block);
    }

    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        if test_only(&f.attrs) {
            return;
        }
        self.visit_fn(&f.sig.ident, &f.sig, &f.block);
    }

    fn visit_trait_item_fn(&mut self, f: &'ast syn::TraitItemFn) {
        if let Some(block) = &f.default {
            self.visit_fn(&f.sig.ident, &f.sig, block);
        }
    }

    fn visit_block(&mut self, block: &'ast syn::Block) {
        self.env.push(BTreeMap::new());
        syn::visit::visit_block(self, block);
        self.env.pop();
    }

    fn visit_local(&mut self, local: &'ast syn::Local) {
        let init = local.init.as_ref();
        if let Some(init) = init {
            self.visit_expr(&init.expr);
            if let Some((_, diverge)) = &init.diverge {
                self.visit_expr(diverge);
            }
        }
        self.bind_pattern(&local.pat, init.map(|init| &*init.expr));
    }

    fn visit_expr_closure(&mut self, c: &'ast syn::ExprClosure) {
        let hint = self.closure_hint.take();
        self.env.push(BTreeMap::new());
        for input in &c.inputs {
            match (input, &hint) {
                (syn::Pat::Ident(p), Some(ty)) => self.bind(
                    p.ident.to_string(),
                    Binding {
                        ty: Some(ty.clone()),
                        unwrap: None,
                    },
                ),
                _ => self.bind_pattern(input, None),
            }
        }
        self.visit_expr(&c.body);
        self.env.pop();
    }

    fn visit_arm(&mut self, arm: &'ast syn::Arm) {
        self.env.push(BTreeMap::new());
        self.bind_pattern(&arm.pat, None);
        if let Some((_, guard)) = &arm.guard {
            self.visit_expr(guard);
        }
        self.visit_expr(&arm.body);
        self.env.pop();
    }

    fn visit_expr_for_loop(&mut self, f: &'ast syn::ExprForLoop) {
        self.visit_expr(&f.expr);
        self.env.push(BTreeMap::new());
        // A loop over `chars()` binds a `char`.
        match (&*f.pat, &*f.expr) {
            (syn::Pat::Ident(p), syn::Expr::MethodCall(m)) if m.method == "chars" => self.bind(
                p.ident.to_string(),
                Binding {
                    ty: Some(Ty::foreign("char")),
                    unwrap: None,
                },
            ),
            _ => self.bind_pattern(&f.pat, None),
        }
        self.visit_block(&f.body);
        self.env.pop();
    }

    fn visit_expr_let(&mut self, l: &'ast syn::ExprLet) {
        self.visit_expr(&l.expr);
        self.bind_pattern(&l.pat, None);
    }

    fn visit_expr_call(&mut self, c: &'ast syn::ExprCall) {
        if self.conversion(c).is_some() {
            self.report.sites = self.report.sites.incremented();
        }
        if let syn::Expr::Path(path) = &*c.func {
            let callee = self.path_callee(path);
            let shown: Vec<String> = path
                .path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect();
            for arg in &c.args {
                if let Some(unwrap) = self.unwrap_of(arg) {
                    let line = LineNo::new(arg.span().start().line);
                    self.record(unwrap, callee, shown.join("::"), line);
                }
            }
        }
        syn::visit::visit_expr_call(self, c);
    }

    fn visit_expr_method_call(&mut self, m: &'ast syn::ExprMethodCall) {
        let method = m.method.to_string();
        if m.args.is_empty() && self.attribute(&m.receiver, &method).is_some() {
            self.report.sites = self.report.sites.incremented();
        }
        let callee = self.method_callee(&m.receiver, &method);
        // A parameter typed `impl Into<T>`, with `T` foreign, hands its
        // argument to the foreign type at once: a boundary, as the
        // `insert` of `ambre_crdt::Fields` is.
        let boundaries: Vec<usize> = self
            .infer(&m.receiver)
            .and_then(|owner| self.catalog.boundaries.get(&(owner, method.clone())))
            .map(|params| {
                params
                    .iter()
                    .filter(|(_, root)| {
                        !self.workspace_root(root)
                            && !["std", "core", "alloc"].contains(&root.as_str())
                    })
                    .map(|(index, _)| *index)
                    .collect()
            })
            .unwrap_or_default();
        for (index, arg) in m.args.iter().enumerate() {
            if let Some(unwrap) = self.unwrap_of(arg) {
                let line = LineNo::new(arg.span().start().line);
                let callee = if boundaries.contains(&index) {
                    Callee::Foreign
                } else {
                    callee
                };
                self.record(unwrap, callee, format!(".{method}"), line);
            }
        }
        // A method on the unwrapped value: the primitive's own method is
        // foreign, and the method of a workspace extension trait is not.
        let accessor = m.args.is_empty()
            && self
                .catalog
                .newtypes
                .values()
                .any(|newtype| newtype.accessors.contains(&method));
        if !PEELS.contains(&method.as_str())
            && !accessor
            && let Some(unwrap) = self.unwrap_of(&m.receiver)
        {
            let own = self.catalog.declared_methods.within(&method, self.reach())
                && !foreign_method(&method);
            let callee = if own {
                Callee::Workspace
            } else {
                Callee::Foreign
            };
            let line = LineNo::new(m.method.span().start().line);
            self.record(unwrap, callee, format!(".{method}"), line);
        }
        self.visit_expr(&m.receiver);
        for arg in &m.args {
            // The closure `map_err` takes reads the error.
            if method == "map_err" && matches!(arg, syn::Expr::Closure(_)) {
                self.closure_hint = Some(self.error());
            }
            self.visit_expr(arg);
        }
    }

    fn visit_expr_field(&mut self, f: &'ast syn::ExprField) {
        if self.tuple_field(f).is_some() {
            self.report.sites = self.report.sites.incremented();
        }
        syn::visit::visit_expr_field(self, f);
    }
}

/// The crates each crate reaches through the dependencies its manifest
/// names, itself included. A method of a crate it does not reach cannot be
/// the callee. A crate whose manifest, or a dependency's, cannot be read
/// is left out, so it reaches every crate.
fn reach(tree: &impl Tree, files: &[(&Source, syn::File)]) -> BTreeMap<String, BTreeSet<String>> {
    let mut direct: BTreeMap<String, Option<BTreeSet<String>>> = BTreeMap::new();
    for (source, _) in files {
        if direct.contains_key(&source.krate) {
            continue;
        }
        let depth = std::path::Path::new(&source.file).components().count();
        let deps = source
            .path
            .ancestors()
            .nth(depth)
            .and_then(|dir| tree.read(&dir.join("Cargo.toml")))
            .map(|manifest| dependencies(&manifest).packages);
        direct.insert(source.krate.clone(), deps);
    }
    let mut reach = BTreeMap::new();
    for krate in direct.keys() {
        let mut seen = BTreeSet::from([krate.clone()]);
        let mut next = vec![krate.clone()];
        let mut whole = true;
        while let Some(at) = next.pop() {
            match direct.get(&at) {
                Some(Some(deps)) => {
                    for dep in deps.iter().filter(|dep| direct.contains_key(*dep)) {
                        if seen.insert(dep.clone()) {
                            next.push(dep.clone());
                        }
                    }
                }
                _ => whole = false,
            }
        }
        if whole {
            reach.insert(krate.clone(), seen);
        }
    }
    reach
}

/// The report over every file the scan read outside test code.
pub fn report(tree: &impl Tree, scan: &Scan) -> Report {
    let mut report = Report::default();
    let mut files: Vec<(&Source, syn::File)> = Vec::new();
    let mut uses: Vec<Uses> = Vec::new();
    let mut raw = Raw::default();
    for source in &scan.sources {
        let parsed = tree
            .read(&source.path)
            .and_then(|text| syn::parse_file(&text).ok());
        let Some(file) = parsed else {
            report.errors.push(format!(
                "crates/{}/{}: cannot read it again",
                source.krate, source.file
            ));
            continue;
        };
        raw.catalog.crates.insert(source.krate.replace('-', "_"));
        raw.catalog
            .modules
            .extend(source.module.split("::").map(str::to_owned));
        let mut found = Uses::default();
        imports(&file.items, &mut found);
        let crate_uses = raw
            .catalog
            .crate_uses
            .entry(source.krate.clone())
            .or_default();
        for (name, root) in &found.names {
            let roots = crate_uses.entry(name.clone()).or_default();
            roots.any.insert(root.clone());
            if let Some(root) = found.public.get(name) {
                roots.public.insert(root.clone());
            }
        }
        visit_items(&mut raw, Phase::Declare, source, &found, &file);
        files.push((source, file));
        uses.push(found);
    }
    raw.catalog.modules.remove("crate");
    raw.catalog.reach = reach(tree, &files);
    for ((source, file), found) in files.iter().zip(&uses) {
        visit_items(&mut raw, Phase::Catalog, source, found, file);
    }
    let mut catalog = raw.close();
    catalog.foreign_names = uses
        .iter()
        .flat_map(|found| &found.names)
        .filter(|(_, root)| !catalog.workspace_root(root))
        .map(|(name, _)| name.clone())
        .collect();
    report.newtypes = SiteCount::new(catalog.newtypes.len());
    for ((source, file), found) in files.iter().zip(uses) {
        let mut visit = BodyVisit {
            catalog: &catalog,
            scope_of: Scope {
                krate: source.krate.clone(),
                uses: found,
            },
            file: source.file.clone(),
            module: source.module.split("::").map(str::to_owned).collect(),
            scope: Vec::new(),
            owners: Vec::new(),
            exempt: Vec::new(),
            closure_hint: None,
            env: vec![BTreeMap::new()],
            report: &mut report,
        };
        for item in &file.items {
            visit.visit_item(item);
        }
    }
    report.refusals.sort();
    report.ambiguous.sort();
    report
}

/// One pass of the catalog over the items of one file.
fn visit_items(raw: &mut Raw, phase: Phase, source: &Source, uses: &Uses, file: &syn::File) {
    let mut visit = CatalogVisit {
        phase,
        module: source.module.split("::").map(str::to_owned).collect(),
        scope: Scope {
            krate: source.krate.clone(),
            uses: uses.clone(),
        },
        raw,
    };
    for item in &file.items {
        visit.visit_item(item);
    }
}

/// One line of the unwrap allowlist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub line: LineNo,
    /// The sites the line allows under its key.
    pub count: SiteCount,
}

/// The allowlist: each key, with its line and its count.
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

/// Whether the unwrap report fails on a refusal.
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

/// Compares the report with the allowlist text, `None` when the file is
/// missing. A missing or malformed allowlist is an error in both modes;
/// each message of the error names one defect.
pub fn judge(
    report: &Report,
    allowlist: Option<&str>,
    strictness: Strictness,
) -> Result<Judgement, Vec<String>> {
    let Some(text) = allowlist else {
        return Err(vec!["cannot read it".to_owned()]);
    };
    let rows = parse_allowlist(text)?;
    let mut sites: BTreeMap<String, SiteCount> = BTreeMap::new();
    for refusal in &report.refusals {
        let count = sites.entry(refusal.key()).or_default();
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
        match rows.get(key) {
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
        let found = sites.get(key).copied().unwrap_or_default();
        if found < row.count {
            judgement.stale = judgement.stale.incremented();
            judgement.problems.push(format!(
                "line {} allows {} sites, the report finds {found}\n  {key}",
                row.line, row.count
            ));
        }
    }
    let clean = report.errors.is_empty()
        && judgement.unlisted == SiteCount::default()
        && judgement.over == SiteCount::default()
        && judgement.stale == SiteCount::default();
    if strictness == Strictness::Strict && !clean {
        judgement.outcome = Outcome::Fail;
    }
    Ok(judgement)
}

#[cfg(test)]
#[path = "unwrap_tests.rs"]
mod tests;
