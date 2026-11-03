//! The newtype census of the gate (`ARCH-062`).
//!
//! A newtype of the workspace is an instance of a generic family of
//! `ambre-core`, or a hand-written exception with a reason
//! (`ambre-spec-newtypes.md`). The census reads, in each file the walk reaches
//! outside test code, every struct that:
//!
//!   * declares no generic parameter: a struct that does is a family, and an
//!     instance of it writes no struct;
//!   * holds exactly one field once each `PhantomData` marker is set apart;
//!   * holds that field as an integer, a float, a `String` or a `Vec<u8>`.
//!
//! Each such struct needs one line of the exceptions file, with the reason it
//! is no instance of a family. A struct no line names fails, and so does a line
//! that names no struct: the file only shrinks on its own.
//!
//! The census reads the type as the source writes it. An alias that names a
//! primitive hides it, and so does a type a macro writes: a reviewer holds
//! both.

use std::collections::{BTreeMap, BTreeSet};

use quote::ToTokens;
use syn::visit::Visit;

use crate::allowlist::LineNo;
use crate::scan::{normalise, test_only};

/// The exceptions file, relative to the workspace root.
pub const NEWTYPES: &str = "specs/1-Foundations/ambre-newtypes.tsv";

/// The header line, the first line of the file that is not a comment.
pub const HEADER: &str = "key\treason";

/// The separator between two fields of a key, as in the primitive allowlist.
const SEPARATOR: &str = " | ";

/// The number of fields a key carries: crate, file, module, struct, type.
const FIELDS: usize = 5;

/// The integer and float types a newtype may hold.
const NUMBERS: &[&str] = &[
    "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize", "f32",
    "f64",
];

/// The key of each hand-written newtype that `file` declares, in the crate
/// `krate`, at the crate-relative path `relative`, in `module`.
pub fn census(krate: &str, relative: &str, module: &str, file: &syn::File) -> Vec<String> {
    let mut visit = Census {
        krate,
        relative,
        module: module.split("::").map(str::to_owned).collect(),
        scope: Vec::new(),
        found: Vec::new(),
    };
    visit.visit_file(file);
    visit.found
}

struct Census<'a> {
    krate: &'a str,
    relative: &'a str,
    module: Vec<String>,
    /// The `fn` items around the next struct, the innermost last.
    scope: Vec<String>,
    found: Vec<String>,
}

impl<'ast> Visit<'ast> for Census<'_> {
    fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
        // A module in a file of its own is read when the walk reaches it.
        if test_only(&m.attrs) || m.content.is_none() {
            return;
        }
        self.module.push(m.ident.to_string());
        syn::visit::visit_item_mod(self, m);
        self.module.pop();
    }

    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        if test_only(&f.attrs) {
            return;
        }
        self.scope.push(f.sig.ident.to_string());
        syn::visit::visit_item_fn(self, f);
        self.scope.pop();
    }

    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        if !test_only(&i.attrs) {
            syn::visit::visit_item_impl(self, i);
        }
    }

    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        if test_only(&f.attrs) {
            return;
        }
        self.scope.push(f.sig.ident.to_string());
        syn::visit::visit_impl_item_fn(self, f);
        self.scope.pop();
    }

    fn visit_item_struct(&mut self, s: &'ast syn::ItemStruct) {
        if test_only(&s.attrs) || !s.generics.params.is_empty() {
            return;
        }
        let mut held = s.fields.iter().filter(|field| !phantom(&field.ty));
        let (Some(field), None) = (held.next(), held.next()) else {
            return;
        };
        if !primitive(&field.ty) {
            return;
        }
        let mut name = self.scope.clone();
        name.push(s.ident.to_string());
        self.found.push(
            [
                self.krate.to_owned(),
                self.relative.to_owned(),
                self.module.join("::"),
                name.join("::"),
                normalise(&field.ty.to_token_stream().to_string()),
            ]
            .join(SEPARATOR),
        );
    }
}

/// Whether `ty` is a `PhantomData` marker, by the last segment of its path.
fn phantom(ty: &syn::Type) -> bool {
    matches!(ty, syn::Type::Path(path)
        if path.path.segments.last().is_some_and(|last| last.ident == "PhantomData"))
}

/// Whether `ty` is an integer, a float, a `String` or a `Vec<u8>`, by the last
/// segment of its path.
fn primitive(ty: &syn::Type) -> bool {
    let syn::Type::Path(path) = ty else {
        return false;
    };
    if path.qself.is_some() {
        return false;
    }
    let Some(last) = path.path.segments.last() else {
        return false;
    };
    match &last.arguments {
        syn::PathArguments::None => {
            last.ident == "String" || NUMBERS.iter().any(|number| last.ident == number)
        }
        syn::PathArguments::AngleBracketed(args) => {
            last.ident == "Vec"
                && args.args.len() == 1
                && matches!(args.args.first(), Some(syn::GenericArgument::Type(syn::Type::Path(inner)))
                    if inner.qself.is_none() && inner.path.is_ident("u8"))
        }
        syn::PathArguments::Parenthesized(_) => false,
    }
}

/// The defects of the exceptions file against the census. A line may break
/// the format, a newtype may have no line, and a line may name no newtype.
/// `text` is `None` when the file cannot be read.
pub fn judge(text: Option<&str>, found: &BTreeSet<String>) -> Vec<String> {
    let Some(text) = text else {
        return vec![format!("cannot read {NEWTYPES}")];
    };
    let (listed, mut problems) = parse(text);
    for key in found.difference(&listed.keys().cloned().collect()) {
        problems.push(format!(
            "a hand-written newtype no line of {NEWTYPES} names. Make it an instance of a \
             generic family of ambre-core, or add the line by hand with the reason it is none \
             (ambre-spec-newtypes.md)\n  {key}"
        ));
    }
    for (key, line) in &listed {
        if !found.contains(key) {
            problems.push(format!(
                "{NEWTYPES}:{line} names no hand-written newtype. Remove it with `--prune`\n  {key}"
            ));
        }
    }
    problems
}

/// The file with each line that names no newtype removed, and nothing else.
pub fn pruned(text: &str, found: &BTreeSet<String>) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let stale = line
            .split_once('\t')
            .is_some_and(|(key, _)| key.split(SEPARATOR).count() == FIELDS && !found.contains(key));
        if !stale {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// The lines of the file by key, with their number, and the defects of its
/// format. The format breaks on five defects:
///
/// * a missing header;
/// * a line of other than two fields;
/// * a key that does not split into five fields;
/// * an empty reason;
/// * a key that two lines carry.
fn parse(text: &str) -> (BTreeMap<String, LineNo>, Vec<String>) {
    let mut listed = BTreeMap::new();
    let mut problems = Vec::new();
    let mut header = false;
    for (i, line) in text.lines().enumerate() {
        let number = LineNo(i + 1);
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        if !header {
            if line != HEADER {
                problems.push(format!(
                    "{NEWTYPES}:{number}: the header must read {HEADER:?}"
                ));
            }
            header = true;
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        let [key, reason] = fields.as_slice() else {
            problems.push(format!(
                "{NEWTYPES}:{number}: {} fields, want 2: key, reason",
                fields.len()
            ));
            continue;
        };
        let parts: Vec<&str> = key.split(SEPARATOR).collect();
        if parts.len() != FIELDS || parts.iter().any(|part| part.trim().is_empty()) {
            problems.push(format!(
                "{NEWTYPES}:{number}: the key does not split into five fields on {SEPARATOR:?}"
            ));
            continue;
        }
        if reason.trim().is_empty() {
            problems.push(format!(
                "{NEWTYPES}:{number}: the reason is empty -- say why no generic family fits"
            ));
            continue;
        }
        if listed.insert((*key).to_owned(), number).is_some() {
            problems.push(format!(
                "{NEWTYPES}:{number}: a second line for the key {key}"
            ));
        }
    }
    if !header {
        problems.push(format!(
            "{NEWTYPES}: the file has no header line {HEADER:?}"
        ));
    }
    (listed, problems)
}

#[cfg(test)]
#[path = "newtypes_tests.rs"]
mod tests;
