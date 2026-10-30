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
//!   * a newtype is a struct with one field over a primitive, a byte
//!     vector, a byte slice or a byte array. `PhantomData` is no field;
//!   * a type a `define_*!` or `fixed_bytes!` macro writes is a newtype too;
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
//!   * a type a `macro_rules!` body names as written is catalogued, as
//!     `loro_fields!` writes `Fields`.
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
    META, PRIMITIVES, Scan, Source, Tree, fragments_of, item_attrs, macro_types, rule_bodies,
    test_only,
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
    name.starts_with("define_") || name == "fixed_bytes"
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
pub struct Tally(usize);

impl Tally {
    /// A tally of `n` sites.
    pub const fn new(n: usize) -> Self {
        Self(n)
    }

    /// The tally one site later.
    const fn incremented(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

impl fmt::Display for Tally {
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
    pub foreign: Tally,
    /// Every unwrap the report attributes to a newtype, handed or not.
    pub sites: Tally,
    /// The newtypes the report knows, by name.
    pub newtypes: Tally,
    /// Each file the report could not read again.
    pub errors: Vec<String>,
}

impl Report {
    /// The ambiguous sites whose receiver the report cannot type.
    pub fn ambiguous_receivers(&self) -> Tally {
        self.ambiguous_by(Why::Receiver)
    }

    /// The ambiguous sites whose callee name std or a dependency shares.
    pub fn ambiguous_callees(&self) -> Tally {
        self.ambiguous_by(Why::Callee)
    }

    fn ambiguous_by(&self, why: Why) -> Tally {
        Tally(
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
}

/// What the workspace declares, read once before any body.
#[derive(Debug, Default)]
struct Catalog {
    newtypes: BTreeMap<String, Newtype>,
    /// The type of each field of each struct, by struct name. A tuple
    /// field goes by its index.
    fields: BTreeMap<String, BTreeMap<String, String>>,
    /// Every type a field of each name holds, in any struct.
    field_types: BTreeMap<String, BTreeSet<String>>,
    /// The type of each `const` and `static`, by name.
    consts: BTreeMap<String, String>,
    /// Every struct, enum, union, trait and alias, by name.
    types: BTreeSet<String>,
    /// The same types, with the crates that declare each.
    declared: Defs,
    /// Every name a `use` brings in from a foreign crate, with the crates
    /// whose files write that `use`.
    reexports: Defs,
    /// Every free `fn`, by name.
    fns: Defs,
    /// Every method of an inherent impl, a workspace trait or a macro.
    methods: Defs,
    /// Every method of an impl of a foreign trait.
    trait_methods: Defs,
    /// Every method a workspace trait declares: an extension trait reaches
    /// a primitive receiver through one.
    declared_methods: Defs,
    /// The return type of each `fn`, as `name` for a free one and
    /// `Owner::name` for an associated one. A name with two returns holds
    /// none.
    returns: BTreeMap<String, Option<Ret>>,
    /// The parameters of each `Owner::method` typed `impl Into<T>`. Each
    /// holds its index, `self` left out, and the name of `T`.
    boundaries: BTreeMap<String, Vec<(usize, String)>>,
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

    /// Files the return of the `fn` `key`. Two different returns under one
    /// key cancel each other.
    fn returned(&mut self, key: String, ret: Option<Ret>) {
        let Some(ret) = ret else {
            return;
        };
        self.returns
            .entry(key)
            .and_modify(|known| {
                if known.as_ref() != Some(&ret) {
                    *known = None;
                }
            })
            .or_insert_with(|| Some(ret.clone()));
    }

    /// The return the catalog knows for `key`.
    fn ret(&self, key: &str) -> Option<&Ret> {
        self.returns.get(key)?.as_ref()
    }
}

/// Names the workspace defines, each with the crates that define it.
#[derive(Debug, Default)]
struct Defs(BTreeMap<String, BTreeSet<String>>);

impl Defs {
    fn add(&mut self, name: String, krate: &str) {
        self.0.entry(name).or_default().insert(krate.to_owned());
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
    /// The type name, `Self` read as the owner.
    ty: String,
    /// The type `?` yields, for a `Result` or an `Option`.
    tried: Option<String>,
}

impl Ret {
    fn of(output: &syn::ReturnType, owner: Option<&str>) -> Option<Self> {
        let syn::ReturnType::Type(_, ty) = output else {
            return None;
        };
        let own = |name: String| match owner {
            Some(owner) if name == "Self" => owner.to_owned(),
            _ => name,
        };
        let ty_name = own(shape_name(ty)?);
        let tried = match &**ty {
            syn::Type::Path(p) if ty_name == "Result" || ty_name == "Option" => p
                .path
                .segments
                .last()
                .and_then(first_argument)
                .and_then(shape_name)
                .map(own),
            _ => None,
        };
        Some(Self { ty: ty_name, tried })
    }
}

/// The index of each parameter, `self` left out, typed `impl Into<T>`,
/// with the name of `T`.
fn into_params(sig: &syn::Signature) -> Vec<(usize, String)> {
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
                Some((index, type_name(first_argument(last)?)?))
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
    /// Returns a `bool`: a predicate, never the representation.
    returns_bool: bool,
}

impl Method {
    fn is_accessor(&self) -> bool {
        self.self_only
            && self.returns_inner
            && !self.returns_bool
            && self.name != "len"
            && !self.name.starts_with("is_")
            && !self.name.starts_with("has_")
    }
}

/// One impl block, as the catalog reads it.
struct Impl {
    owner: String,
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
    /// Any other type, by name: `Name` in `From<Name>`.
    Named(String),
}

impl Argument {
    fn of(ty: &syn::Type) -> Option<Self> {
        if inner_like(ty) {
            return Some(Self::Inner(type_name(ty)));
        }
        type_name(ty).map(Self::Named)
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

/// The type name of [`type_name`], with `[]` for a slice or an array and
/// `()` for a tuple: none of them is a newtype.
fn shape_name(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Reference(r) => shape_name(&r.elem),
        syn::Type::Paren(p) => shape_name(&p.elem),
        syn::Type::Group(g) => shape_name(&g.elem),
        syn::Type::Slice(_) | syn::Type::Array(_) => Some("[]".to_owned()),
        syn::Type::Tuple(_) => Some("()".to_owned()),
        other => type_name(other),
    }
}

/// The signature of an impl method, as the catalog reads it.
fn method_of(sig: &syn::Signature) -> Method {
    let takes_self = matches!(sig.inputs.first(), Some(syn::FnArg::Receiver(_)));
    let (returns_inner, returns_bool) = match &sig.output {
        syn::ReturnType::Type(_, ty) => (inner_like(ty), is_named(ty, "bool")),
        syn::ReturnType::Default => (false, false),
    };
    Method {
        name: sig.ident.to_string(),
        self_only: takes_self && sig.inputs.len() == 1,
        takes_self,
        returns_inner,
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
    krate: String,
    module: Vec<String>,
    raw: &'r mut Raw,
}

impl CatalogVisit<'_> {
    fn home(&self) -> Home {
        Home {
            krate: self.krate.clone(),
            module: self.module.join("::"),
        }
    }
}

impl<'ast> Visit<'ast> for CatalogVisit<'_> {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        if test_only(item_attrs(item)) {
            return;
        }
        let catalog = &mut self.raw.catalog;
        match item {
            syn::Item::Mod(m) => {
                let Some((_, items)) = &m.content else {
                    return;
                };
                catalog.modules.insert(m.ident.to_string());
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
                catalog.declare(name.clone(), &self.krate);
                let fields: Vec<&syn::Field> = s
                    .fields
                    .iter()
                    .filter(|field| type_name(&field.ty).is_none_or(|ty| ty != "PhantomData"))
                    .collect();
                let named: BTreeMap<String, String> = s
                    .fields
                    .iter()
                    .enumerate()
                    .filter_map(|(index, field)| {
                        let name = field
                            .ident
                            .as_ref()
                            .map_or_else(|| index.to_string(), ToString::to_string);
                        Some((name, shape_name(&field.ty)?))
                    })
                    .collect();
                for (field, ty) in &named {
                    catalog
                        .field_types
                        .entry(field.clone())
                        .or_default()
                        .insert(ty.clone());
                }
                catalog
                    .fields
                    .entry(name.clone())
                    .or_default()
                    .extend(named);
                if let [field] = fields.as_slice()
                    && inner_like(&field.ty)
                {
                    let home = self.home();
                    let newtype = self.raw.catalog.newtypes.entry(name).or_default();
                    newtype.homes.insert(home);
                    newtype.tuple |=
                        field.ident.is_none() && !matches!(field.vis, syn::Visibility::Inherited);
                }
            }
            syn::Item::Enum(e) => {
                catalog.declare(e.ident.to_string(), &self.krate);
            }
            syn::Item::Union(u) => {
                catalog.declare(u.ident.to_string(), &self.krate);
            }
            syn::Item::Type(t) => {
                catalog.declare(t.ident.to_string(), &self.krate);
            }
            syn::Item::Trait(t) => {
                catalog.declare(t.ident.to_string(), &self.krate);
                self.raw.traits.insert(t.ident.to_string());
                for item in &t.items {
                    if let syn::TraitItem::Fn(f) = item
                        && matches!(f.sig.inputs.first(), Some(syn::FnArg::Receiver(_)))
                    {
                        catalog.methods.add(f.sig.ident.to_string(), &self.krate);
                        catalog
                            .declared_methods
                            .add(f.sig.ident.to_string(), &self.krate);
                    }
                }
            }
            syn::Item::Fn(f) => {
                catalog.fns.add(f.sig.ident.to_string(), &self.krate);
                catalog.returned(f.sig.ident.to_string(), Ret::of(&f.sig.output, None));
            }
            syn::Item::Const(c) => {
                if let Some(ty) = shape_name(&c.ty) {
                    catalog.consts.insert(c.ident.to_string(), ty);
                }
            }
            syn::Item::Static(s) => {
                if let Some(ty) = shape_name(&s.ty) {
                    catalog.consts.insert(s.ident.to_string(), ty);
                }
            }
            syn::Item::Impl(i) => {
                let Some(owner) = type_name(&i.self_ty).filter(|owner| !owner.starts_with(META))
                else {
                    return;
                };
                for item in &i.items {
                    if let syn::ImplItem::Fn(f) = item
                        && !test_only(&f.attrs)
                    {
                        let key = format!("{owner}::{}", f.sig.ident);
                        catalog.returned(key.clone(), Ret::of(&f.sig.output, Some(&owner)));
                        let into = into_params(&f.sig);
                        if i.trait_.is_none() && !into.is_empty() {
                            catalog.boundaries.insert(key, into);
                        }
                    }
                }
                let last = i
                    .trait_
                    .as_ref()
                    .and_then(|(_, path, _)| path.segments.last());
                let of_trait = last.map(|last| last.ident.to_string());
                let argument = last.and_then(first_argument).and_then(Argument::of);
                let inner = inner_like(&i.self_ty).then(|| owner.clone());
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
                            Some(method_of(&f.sig))
                        }
                        _ => None,
                    })
                    .collect();
                self.raw.impls.push(Impl {
                    owner,
                    krate: self.krate.clone(),
                    of_trait,
                    argument,
                    inner,
                    methods,
                    shows_content,
                });
            }
            syn::Item::Macro(m) => {
                let Some(name) = m.mac.path.segments.last().map(|s| s.ident.to_string()) else {
                    return;
                };
                if name == "macro_rules" {
                    if let Some(ident) = &m.ident {
                        let mut methods = BTreeSet::new();
                        let accessors = macro_methods(m.mac.tokens.clone(), &mut methods);
                        for method in methods {
                            catalog.methods.add(method, &self.krate);
                        }
                        self.raw.macros.insert(ident.to_string(), accessors);
                    }
                    // A type a transcriber names, as `loro_fields!` writes
                    // `Fields`, is catalogued as written. A type it names by
                    // a metavariable is not.
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
                    for ty in &types {
                        catalog.declare(ty.clone(), &self.krate);
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

impl Raw {
    /// The catalog, once every newtype is known: the accessors of each, and
    /// the methods of each impl sorted by who owns the trait.
    fn close(mut self) -> Catalog {
        let mut catalog = self.catalog;
        for invocation in self.invocations {
            let accessors = self
                .macros
                .get(&invocation.family)
                .cloned()
                .unwrap_or_default();
            for ty in invocation.types {
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
                newtype.accessors.extend(
                    imp.methods
                        .iter()
                        .filter(|method| method.is_accessor())
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
    ty: Option<String>,
    unwrap: Option<Unwrap>,
}

/// The second pass over one file.
struct BodyVisit<'c, 'r> {
    catalog: &'c Catalog,
    krate: String,
    file: String,
    module: Vec<String>,
    /// The item path inside the module: an impl owner, then a `fn`.
    scope: Vec<String>,
    /// The owner of each enclosing impl, the innermost last.
    owners: Vec<String>,
    /// For each enclosing impl, the newtypes it may unwrap: its owner and
    /// the type a conversion trait names.
    exempt: Vec<BTreeSet<String>>,
    /// The type of the parameter of the next closure, as `map_err` gives
    /// its closure the error.
    closure_hint: Option<String>,
    env: Vec<BTreeMap<String, Binding>>,
    /// The first segment of the path each `use` of the file imports, by
    /// the name it brings in.
    imports: BTreeMap<String, String>,
    report: &'r mut Report,
}

impl BodyVisit<'_, '_> {
    fn module(&self) -> String {
        self.module.join("::")
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
    fn hint(&self, ty: &syn::Type) -> Option<String> {
        let name = type_name(ty)?;
        if name == "Self" {
            return self.owners.last().cloned();
        }
        Some(name)
    }

    /// The type name of an expression, where the syntax states it.
    fn infer(&self, expr: &syn::Expr) -> Option<String> {
        match expr {
            syn::Expr::Path(p) if p.qself.is_none() => {
                let ident = p.path.get_ident()?;
                if ident == "self" {
                    return self.owners.last().cloned();
                }
                let name = ident.to_string();
                match self.lookup(&name) {
                    Some(binding) => binding.ty.clone(),
                    None => self.catalog.consts.get(&name).cloned(),
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
            syn::Expr::Array(_) => Some("[]".to_owned()),
            syn::Expr::Lit(_) => Some("literal".to_owned()),
            syn::Expr::Macro(m) => {
                let name = m.mac.path.segments.last()?.ident.to_string();
                MACRO_TYPES
                    .iter()
                    .find(|(mac, _)| *mac == name)
                    .map(|(_, ty)| (*ty).to_owned())
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
                    .map(|(_, ty)| (*ty).to_owned())
            }
            syn::Expr::Reference(r) => self.infer(&r.expr),
            syn::Expr::Paren(p) => self.infer(&p.expr),
            syn::Expr::Group(g) => self.infer(&g.expr),
            syn::Expr::Unary(u) if matches!(u.op, syn::UnOp::Deref(_)) => self.infer(&u.expr),
            syn::Expr::Try(t) => match self.ret_of(&t.expr) {
                Some(ret) => ret.tried.clone(),
                None => self.infer(&t.expr),
            },
            syn::Expr::Struct(s) => {
                let name = s.path.segments.last()?.ident.to_string();
                if name == "Self" {
                    return self.owners.last().cloned();
                }
                Some(name)
            }
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
                if let Some(ret) = self.ret_of(expr) {
                    return Some(ret.ty.clone());
                }
                let [.., owner, ctor] = segments.as_slice() else {
                    return None;
                };
                // An associated `fn` of a foreign type, as
                // `uuid::Uuid::new_v4`, returns no workspace newtype.
                if owner.starts_with(|c: char| c.is_ascii_uppercase())
                    && self.foreign_path(&segments)
                {
                    return Some(owner.clone());
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
                if owner == "Self" {
                    return self.owners.last().cloned();
                }
                // A constructor of a type a foreign crate exports builds
                // that foreign type, as `RwSignal::new` does.
                let foreign = self
                    .imports
                    .get(owner)
                    .is_some_and(|root| !self.workspace_root(root));
                (STD_TYPES.contains(&owner.as_str())
                    || self.catalog.types.contains(owner)
                    || foreign)
                    .then(|| owner.clone())
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
                    [name] if self.lookup(name).is_none() => self.catalog.ret(name),
                    [.., owner, name] if owner.starts_with(|c: char| c.is_ascii_uppercase()) => {
                        let owner = match owner.as_str() {
                            "Self" => self.owners.last()?,
                            _ => owner,
                        };
                        self.catalog.ret(&format!("{owner}::{name}"))
                    }
                    [.., name] if !self.foreign_path(&segments) => self.catalog.ret(name),
                    _ => None,
                }
            }
            syn::Expr::MethodCall(m) => {
                let owner = self.infer(&m.receiver)?;
                self.catalog.ret(&format!("{owner}::{}", m.method))
            }
            _ => None,
        }
    }

    /// The newtype a named conversion takes. Such a conversion is an
    /// associated `fn` that returns `Self` or the impl owner. Its one
    /// parameter is a workspace newtype, by value or by reference.
    fn named_conversion(&self, sig: &syn::Signature) -> Option<String> {
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
        type_name(&input.ty).filter(|name| self.catalog.newtypes.contains_key(name))
    }

    /// The crates the current crate reaches, `None` for all of them.
    fn reach(&self) -> Option<&BTreeSet<String>> {
        self.catalog.reach.get(&self.krate)
    }

    /// Whether the site may unwrap the newtype: the current module declares
    /// it, or an enclosing impl owns or converts it.
    fn exempts(&self, name: &str) -> bool {
        let here = Home {
            krate: self.krate.clone(),
            module: self.module(),
        };
        let home = self
            .catalog
            .newtypes
            .get(name)
            .is_some_and(|newtype| newtype.homes.contains(&here));
        home || self.exempt.iter().any(|names| names.contains(name))
    }

    /// The unwrap an accessor call on `receiver` makes, if any.
    fn attribute(&self, receiver: &syn::Expr, accessor: &str) -> Option<Unwrap> {
        if self.unwrap_of(receiver).is_some() {
            return None;
        }
        let candidates: Vec<&String> = self
            .catalog
            .newtypes
            .iter()
            .filter(|(_, newtype)| newtype.accessors.contains(accessor))
            .map(|(name, _)| name)
            .collect();
        if candidates.is_empty() {
            return None;
        }
        if let Some(ty) = self.infer(receiver) {
            let name = candidates.into_iter().find(|name| **name == ty)?;
            if self.exempts(name) {
                return None;
            }
            return Some(Unwrap {
                newtype: name.clone(),
                accessor: accessor.to_owned(),
                receiver: Receiver::Known,
            });
        }
        if CONVERSIONS.contains(&accessor) {
            return None;
        }
        // Take a field of a base the report cannot type. It holds a newtype
        // only when a workspace struct has a field of that name and type. A
        // field of a syn or proc-macro2 value holds none.
        let candidates: Vec<&String> = match peel(receiver) {
            syn::Expr::Field(f) => {
                let held = self.catalog.field_types.get(&member_name(&f.member));
                candidates
                    .into_iter()
                    .filter(|name| held.is_some_and(|types| types.contains(*name)))
                    .collect()
            }
            _ => candidates,
        };
        if candidates.is_empty() {
            return None;
        }
        let exempt = candidates.iter().filter(|name| self.exempts(name)).count();
        if exempt == candidates.len() {
            return None;
        }
        let newtype = match candidates.as_slice() {
            [one] => (*one).clone(),
            few if few.len() <= FEW => few.iter().map(|n| n.as_str()).collect::<Vec<_>>().join("|"),
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
        let (name, newtype) = self.catalog.newtypes.get_key_value(&ty)?;
        if !newtype.tuple || self.exempts(name) {
            return None;
        }
        Some(Unwrap {
            newtype: name.clone(),
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
        let (name, newtype) = self.catalog.newtypes.get_key_value(&ty)?;
        if !newtype.conversions.contains(&target.ident.to_string()) || self.exempts(name) {
            return None;
        }
        Some(Unwrap {
            newtype: name.clone(),
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
        ["crate", "self", "super", "Self"].contains(&root)
            || self.catalog.crates.contains(root)
            || self.catalog.modules.contains(root)
    }

    /// Whether a path is an import from a foreign crate, or starts at one.
    fn foreign_path(&self, segments: &[String]) -> bool {
        let Some(first) = segments.first() else {
            return false;
        };
        match self.imports.get(first) {
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
                self.type_callee(owner, &segments)
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
        let name = match segments.last() {
            Some(last) if last == "Self" => self.owners.last(),
            last => last,
        };
        match name {
            Some(name) if !variant && self.catalog.newtypes.contains_key(name) => {
                self.type_callee(name, segments)
            }
            _ => Callee::Constructor,
        }
    }

    /// The crate a path takes its type `owner` from. That is the crate the
    /// path starts at, or the crate a `use` imports it from. Otherwise it
    /// is the current crate.
    fn owner_crate(&self, owner: &str, segments: &[String]) -> Option<String> {
        let first = segments.first()?;
        let root = match self.imports.get(owner) {
            Some(root) => root,
            None if first != owner => first,
            None => return Some(self.krate.clone()),
        };
        if ["crate", "self", "super"].contains(&root.as_str()) {
            return Some(self.krate.clone());
        }
        self.catalog
            .crates
            .contains(root)
            .then(|| root.replace('_', "-"))
    }

    /// How the report reads an associated `fn` of the type `owner`. A
    /// foreign type the workspace re-exports is foreign. When the path or a
    /// `use` names the crate, that crate decides. Otherwise, a name both a
    /// workspace type and such a re-export carry is ambiguous.
    fn type_callee(&self, owner: &str, segments: &[String]) -> Callee {
        if let Some(krate) = self.owner_crate(owner, segments) {
            let only = Some(BTreeSet::from([krate]));
            if self.catalog.declared.within(owner, only.as_ref()) {
                return Callee::Workspace;
            }
            if self.catalog.reexports.within(owner, only.as_ref()) {
                return Callee::Foreign;
            }
        }
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
            return if workspace && self.catalog.types.contains(&ty) {
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
            krate: self.krate.clone(),
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
    within: Option<String>,
    names: Vec<(String, Option<String>)>,
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
        let outer = self.within.clone();
        if segments == ["Err"] {
            self.within = Some(ERROR.to_owned());
        } else if self.visit.foreign_path(&segments) {
            self.within = Some(segments.join("::"));
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
            self.within = Some(segments.join("::"));
        }
        for field in &p.fields {
            self.visit_pat(&field.pat);
        }
        self.within = outer;
    }
}

/// Every name the `use` items of a file bring in, mapped to the first
/// segment of the path.
fn imports(items: &[syn::Item], found: &mut BTreeMap<String, String>) {
    fn tree(t: &syn::UseTree, root: Option<&str>, found: &mut BTreeMap<String, String>) {
        match t {
            syn::UseTree::Path(p) => {
                let segment = p.ident.to_string();
                tree_inner(&p.tree, root.unwrap_or(&segment), found);
            }
            syn::UseTree::Name(n) => {
                let name = n.ident.to_string();
                let root = root.unwrap_or(&name).to_owned();
                found.insert(name, root);
            }
            syn::UseTree::Rename(r) => {
                let name = r.ident.to_string();
                found.insert(r.rename.to_string(), root.unwrap_or(&name).to_owned());
            }
            syn::UseTree::Glob(_) => {}
            syn::UseTree::Group(g) => {
                for item in &g.items {
                    tree(item, root, found);
                }
            }
        }
    }
    fn tree_inner(t: &syn::UseTree, root: &str, found: &mut BTreeMap<String, String>) {
        tree(t, Some(root), found);
    }
    for item in items {
        match item {
            syn::Item::Use(u) if !test_only(&u.attrs) => tree(&u.tree, None, found),
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
        let Some(owner) = type_name(&i.self_ty) else {
            return;
        };
        // The own impl of a newtype, and a conversion of it, unwrap that
        // newtype freely, and no other.
        let converted = i
            .trait_
            .as_ref()
            .and_then(|(_, path, _)| path.segments.last())
            .and_then(first_argument)
            .and_then(type_name);
        let exempt: BTreeSet<String> = [Some(owner.clone()), converted]
            .into_iter()
            .flatten()
            .filter(|name| self.catalog.newtypes.contains_key(name))
            .collect();
        self.exempt.push(exempt);
        self.owners.push(owner.clone());
        self.scope.push(owner);
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
                    ty: Some("char".to_owned()),
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
                    let line = LineNo(arg.span().start().line);
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
        // `insert` of `loro_fields!` is.
        let boundaries: Vec<usize> = self
            .infer(&m.receiver)
            .and_then(|owner| self.catalog.boundaries.get(&format!("{owner}::{method}")))
            .map(|params| {
                params
                    .iter()
                    .filter(|(_, target)| !self.catalog.types.contains(target))
                    .map(|(index, _)| *index)
                    .collect()
            })
            .unwrap_or_default();
        for (index, arg) in m.args.iter().enumerate() {
            if let Some(unwrap) = self.unwrap_of(arg) {
                let line = LineNo(arg.span().start().line);
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
            let line = LineNo(m.method.span().start().line);
            self.record(unwrap, callee, format!(".{method}"), line);
        }
        self.visit_expr(&m.receiver);
        for arg in &m.args {
            // The closure `map_err` takes reads the error.
            if method == "map_err" && matches!(arg, syn::Expr::Closure(_)) {
                self.closure_hint = Some(ERROR.to_owned());
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
/// the callee. A crate whose manifest, or a dependency's, does not parse
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
            .and_then(|text| text.parse::<toml::Table>().ok())
            .map(|manifest| dependencies(&manifest));
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

/// The dependency names of a manifest, its target tables included.
fn dependencies(manifest: &toml::Table) -> BTreeSet<String> {
    let keys = |table: Option<&toml::Value>| -> Vec<String> {
        table
            .and_then(toml::Value::as_table)
            .map(|deps| deps.keys().cloned().collect())
            .unwrap_or_default()
    };
    let mut names: BTreeSet<String> = keys(manifest.get("dependencies")).into_iter().collect();
    if let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) {
        for target in targets.values() {
            names.extend(keys(target.get("dependencies")));
        }
    }
    names
}

/// The report over every file the scan read outside test code.
pub fn report(tree: &impl Tree, scan: &Scan) -> Report {
    let mut report = Report::default();
    let mut files: Vec<(&Source, syn::File)> = Vec::new();
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
        let mut visit = CatalogVisit {
            krate: source.krate.clone(),
            module: source.module.split("::").map(str::to_owned).collect(),
            raw: &mut raw,
        };
        for item in &file.items {
            visit.visit_item(item);
        }
        files.push((source, file));
    }
    raw.catalog.modules.remove("crate");
    let mut catalog = raw.close();
    let mut foreign = BTreeSet::new();
    let mut reexports = Defs::default();
    for (source, file) in &files {
        let mut uses = BTreeMap::new();
        imports(&file.items, &mut uses);
        for (name, _) in uses.into_iter().filter(|(_, root)| {
            !["crate", "self", "super"].contains(&root.as_str())
                && !catalog.crates.contains(root)
                && !catalog.modules.contains(root)
        }) {
            reexports.add(name.clone(), &source.krate);
            foreign.insert(name);
        }
    }
    catalog.foreign_names = foreign;
    catalog.reexports = reexports;
    catalog.reach = reach(tree, &files);
    report.newtypes = Tally::new(catalog.newtypes.len());
    for (source, file) in &files {
        let mut found = BTreeMap::new();
        imports(&file.items, &mut found);
        let mut visit = BodyVisit {
            catalog: &catalog,
            krate: source.krate.clone(),
            file: source.file.clone(),
            module: source.module.split("::").map(str::to_owned).collect(),
            scope: Vec::new(),
            owners: Vec::new(),
            exempt: Vec::new(),
            closure_hint: None,
            env: vec![BTreeMap::new()],
            imports: found,
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

/// One line of the unwrap allowlist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub line: LineNo,
    /// The sites the line allows under its key.
    pub count: Tally,
}

/// The allowlist: each key, with its line and its count.
pub fn parse_allowlist(text: &str) -> Result<BTreeMap<String, Row>, Vec<String>> {
    let mut rows = BTreeMap::new();
    let mut errors = Vec::new();
    let mut header = false;
    for (index, line) in text.lines().enumerate() {
        let number = LineNo(index + 1);
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
            Ok(count) if count > 0 => Tally::new(count),
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
    pub unlisted: Tally,
    /// The lines whose key has more sites than the count allows.
    pub over: Tally,
    /// The lines whose key has fewer sites than the count, none included.
    pub stale: Tally,
    /// One message per line that is over or stale.
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
    let mut sites: BTreeMap<String, Tally> = BTreeMap::new();
    for refusal in &report.refusals {
        let tally = sites.entry(refusal.key()).or_default();
        *tally = tally.incremented();
    }
    let mut judgement = Judgement {
        outcome: Outcome::Pass,
        unlisted: Tally::default(),
        over: Tally::default(),
        stale: Tally::default(),
        problems: Vec::new(),
    };
    for (key, found) in &sites {
        match rows.get(key) {
            None => judgement.unlisted = judgement.unlisted.incremented(),
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
        && judgement.unlisted == Tally::default()
        && judgement.over == Tally::default()
        && judgement.stale == Tally::default();
    if strictness == Strictness::Strict && !clean {
        judgement.outcome = Outcome::Fail;
    }
    Ok(judgement)
}

#[cfg(test)]
#[path = "unwrap_tests.rs"]
mod tests;
