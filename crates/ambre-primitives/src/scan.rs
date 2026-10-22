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
        pending.extend(visit.children);
        skipped.files.append(&mut visit.skipped.files);
        skipped.dirs.append(&mut visit.skipped.dirs);
    }
    result.sites.extend(dedup(keys));
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
}

/// Numbers the second and later sites that share one key, `#2` and on, so two
/// `cfg` twins of one `fn` keep a key each.
fn dedup(keys: Vec<RawKey>) -> Vec<SiteKey> {
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    keys.into_iter()
        .map(|raw| {
            let sep = SiteKey::SEPARATOR;
            let base = format!("{}{sep}{}{sep}{}", raw.head, raw.item, raw.tail);
            let count = seen.entry(base.clone()).or_insert(0);
            *count += 1;
            if *count == 1 {
                SiteKey(base)
            } else {
                SiteKey(format!(
                    "{}{sep}{}#{count}{sep}{}",
                    raw.head, raw.item, raw.tail
                ))
            }
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
        let ty = normalise(&ty.to_token_stream().to_string());
        if !holds_primitive(&ty) {
            return;
        }
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
        });
    }

    fn signature(&mut self, sig: &syn::Signature) {
        let name = sig.ident.to_string();
        for input in &sig.inputs {
            if let syn::FnArg::Typed(arg) = input {
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
        }
        if let syn::ReturnType::Type(_, ty) = &sig.output {
            self.emit(self.item(&format!("{name}()")), SiteKind::Ret, ty);
        }
    }

    fn fields<'f>(&mut self, owner: &str, fields: impl IntoIterator<Item = &'f syn::Field>) {
        for (i, field) in fields.into_iter().enumerate() {
            if test_only(&field.attrs) {
                continue;
            }
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
        self.signature(&f.sig);
        self.body(f.sig.ident.to_string(), &f.block);
    }

    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        if test_only(&i.attrs) {
            return;
        }
        let ty = normalise(&i.self_ty.to_token_stream().to_string());
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
                    self.signature(&f.sig);
                    self.body(f.sig.ident.to_string(), &f.block);
                }
                syn::ImplItem::Const(c) if !test_only(&c.attrs) => {
                    self.emit(self.item(&c.ident.to_string()), SiteKind::Const, &c.ty);
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
        self.scope.push(t.ident.to_string());
        for item in &t.items {
            if let syn::TraitItem::Fn(f) = item {
                if test_only(&f.attrs) {
                    continue;
                }
                self.signature(&f.sig);
                if let Some(block) = &f.default {
                    self.body(f.sig.ident.to_string(), block);
                }
            }
        }
        self.scope.pop();
    }

    fn visit_item_struct(&mut self, s: &'ast syn::ItemStruct) {
        if !test_only(&s.attrs) {
            self.fields(&s.ident.to_string(), &s.fields);
        }
    }

    fn visit_item_union(&mut self, u: &'ast syn::ItemUnion) {
        if !test_only(&u.attrs) {
            self.fields(&u.ident.to_string(), &u.fields.named);
        }
    }

    fn visit_item_enum(&mut self, e: &'ast syn::ItemEnum) {
        if test_only(&e.attrs) {
            return;
        }
        for variant in e
            .variants
            .iter()
            .filter(|variant| !test_only(&variant.attrs))
        {
            self.fields(&format!("{}::{}", e.ident, variant.ident), &variant.fields);
        }
    }

    fn visit_item_const(&mut self, c: &'ast syn::ItemConst) {
        if !test_only(&c.attrs) {
            self.emit(self.item(&c.ident.to_string()), SiteKind::Const, &c.ty);
        }
    }

    fn visit_item_static(&mut self, s: &'ast syn::ItemStatic) {
        if !test_only(&s.attrs) {
            self.emit(self.item(&s.ident.to_string()), SiteKind::Static, &s.ty);
        }
    }

    fn visit_item_type(&mut self, t: &'ast syn::ItemType) {
        if !test_only(&t.attrs) {
            self.emit(self.item(&t.ident.to_string()), SiteKind::Alias, &t.ty);
        }
    }

    // A macro's tokens are not items until it expands, and the scan expands
    // nothing: what a macro writes is out of reach by construction.
    fn visit_item_macro(&mut self, _: &'ast syn::ItemMacro) {}

    // A foreign block declares the foreign API itself.
    fn visit_item_foreign_mod(&mut self, _: &'ast syn::ItemForeignMod) {}
}

#[cfg(test)]
#[path = "scan_tests.rs"]
mod tests;
