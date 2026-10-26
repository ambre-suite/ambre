//! The secret checks of the gate (`ARCH-045`, `ARCH-046`, `ARCH-048`,
//! `ARCH-049`).
//!
//! Every secret of the workspace sits in an `ambre_core::Secret`. Its value
//! is read in `ambre-crypto`, in the Secret module, and in the secrets module
//! of the relay alone. These checks hold that, on every file the scan
//! reaches, outside test code:
//!
//!   * a reader, `expose`, `expose_mut` or `clone_secret`, is named only in
//!     `ambre-crypto`, in the Secret module (`ARCH-045`), and in the
//!     `src/secrets/` module of `ambre-server` (`ARCH-048`);
//!   * the `zeroize` crate is named only in the Secret module, so no type
//!     wipes itself beside the wrapper (`ARCH-046`);
//!   * a sink that lends a secret to one foreign call is named only in the
//!     file that makes that call (`ARCH-049`);
//!   * `include!` is refused, since the gate reads no file it includes. The
//!     one form admitted is the output of the crate's build script,
//!     `include!(concat!(env!("OUT_DIR"), ..))`: the script is scanned, its
//!     output is not (`ARCH-047`).
//!
//! The gate reads names, not types: syn resolves no receiver. So a method of
//! another type that bears a reader's name is refused too, called or
//! declared, and fails closed.
//! Inside a macro, the gate reads every identifier of the token stream, and
//! fails closed the same way.

use std::collections::BTreeMap;

use proc_macro2::{Ident, TokenStream, TokenTree};
use quote::ToTokens;
use syn::spanned::Spanned;
use syn::visit::Visit;

use crate::scan::test_only;

/// The names that read or copy the value of a `Secret`.
const READERS: &[&str] = &["expose", "expose_mut", "clone_secret"];

/// The names the `zeroize` crate gives: the crate, its traits and its
/// wrapper.
const WIPERS: &[&str] = &["zeroize", "Zeroize", "ZeroizeOnDrop", "Zeroizing"];

/// The crate whose code reads the value of a secret.
const CRYPTO: &str = "ambre-crypto";

/// The crate and the file of the Secret module, which every check admits.
const SECRET_MODULE: (&str, &str) = ("ambre-core", "src/secret.rs");

/// The crate and the directory of the relay's secrets module, the one place
/// the relay reads a secret (`ARCH-048`).
const RELAY_SECRETS: (&str, &str) = ("ambre-server", "src/secrets/");

/// The sinks that lend a secret to one foreign call, each with the one file
/// that makes the call (`ARCH-049`).
const SINKS: &[(&str, (&str, &str))] = &[
    ("present_bearer", ("ambre-sync", "src/http.rs")),
    ("present_link", ("ambre-sync", "src/http.rs")),
    ("present_state", ("ambre-app", "src/signin.rs")),
];

/// The names of the sinks.
const SINK_NAMES: &[&str] = &["present_bearer", "present_link", "present_state"];

/// One check, and the rule that states it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Check {
    /// A reader named outside `ambre-crypto` and the Secret module.
    Reader,
    /// The `zeroize` crate named outside the Secret module.
    Wiper,
    /// A sink named outside the file that makes its call.
    Sink,
}

impl Check {
    /// The names the check refuses.
    fn names(self) -> &'static [&'static str] {
        match self {
            Self::Reader => READERS,
            Self::Wiper => WIPERS,
            Self::Sink => SINK_NAMES,
        }
    }

    /// Why the name is refused, in words a reviewer reads.
    fn reason(self) -> &'static str {
        match self {
            Self::Reader => {
                "a secret is read only in ambre-crypto and the Secret module (ARCH-045), and in \
                 the relay's src/secrets/ (ARCH-048): move the call there, or give the newtype a \
                 typed method there"
            }
            Self::Sink => {
                "a sink lends a secret to one foreign call, and is named only in the file that \
                 makes that call (ARCH-049): call it there, or write a typed sink for this call"
            }
            Self::Wiper => {
                "only the Secret module names zeroize (ARCH-046): hold the value in an \
                 ambre_core::Secret"
            }
        }
    }
}

/// The checks that apply to one file of one crate. `relative` is the path of
/// the file inside its crate, with `/` separators.
fn checks(krate: &str, relative: &str) -> Vec<Check> {
    if (krate, relative) == SECRET_MODULE {
        Vec::new()
    } else if krate == CRYPTO {
        vec![Check::Wiper]
    } else if krate == RELAY_SECRETS.0 && relative.starts_with(RELAY_SECRETS.1) {
        vec![Check::Wiper, Check::Sink]
    } else {
        vec![Check::Reader, Check::Wiper, Check::Sink]
    }
}

/// Whether `name` is a sink this file makes the call of.
fn admits_sink(name: &str, krate: &str, relative: &str) -> bool {
    SINKS
        .iter()
        .any(|(sink, file)| *sink == name && *file == (krate, relative))
}

/// Whether the tokens of an `include!` name the output of the build script:
/// `concat!(env!("OUT_DIR"), ..)`.
fn includes_build_output(tokens: &TokenStream) -> bool {
    let text: String = tokens
        .to_string()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    text.starts_with("concat!(env!(\"OUT_DIR\"),")
}

/// Every refusal of the two checks in one file, test code left out. `file`
/// names the file in each refusal.
pub(crate) fn refusals(krate: &str, relative: &str, file: &str, tree: &syn::File) -> Vec<String> {
    let mut seek = Seek {
        checks: checks(krate, relative),
        krate: krate.to_owned(),
        relative: relative.to_owned(),
        file: file.to_owned(),
        found: Vec::new(),
    };
    if !seek.checks.is_empty() && !test_only(&tree.attrs) {
        seek.visit_file(tree);
    }
    seek.found
}

/// A dependency on the `zeroize` crate under another key, which no name of
/// the source would show. The gate reads names, so it refuses one in every
/// crate, the Secret module's included (`ARCH-046`).
pub(crate) fn renamed_wipers(krate: &str, renamed: &BTreeMap<String, String>) -> Vec<String> {
    renamed
        .iter()
        .filter(|(_, package)| package.as_str() == "zeroize")
        .map(|(key, _)| {
            format!(
                "crates/{krate}/Cargo.toml: `{key}` renames the zeroize crate; {}",
                Check::Wiper.reason()
            )
        })
        .collect()
}

/// The walk of one file.
struct Seek {
    checks: Vec<Check>,
    krate: String,
    relative: String,
    file: String,
    found: Vec<String>,
}

impl Seek {
    /// Refuses `ident` when a check of this file names it.
    fn name(&mut self, ident: &Ident) {
        let text = ident.to_string();
        let text = text.trim_start_matches("r#");
        for check in &self.checks {
            if *check == Check::Sink && admits_sink(text, &self.krate, &self.relative) {
                continue;
            }
            if check.names().contains(&text) {
                self.found.push(format!(
                    "{}:{}: `{text}` is named here; {}",
                    self.file,
                    ident.span().start().line,
                    check.reason()
                ));
            }
        }
    }

    /// Refuses every identifier of `tokens` a check names, at any depth. A
    /// macro is read this way, since syn parses no macro body.
    fn tokens(&mut self, tokens: TokenStream) {
        for token in tokens {
            match token {
                TokenTree::Ident(ident) => self.name(&ident),
                TokenTree::Group(group) => self.tokens(group.stream()),
                TokenTree::Punct(_) | TokenTree::Literal(_) => {}
            }
        }
    }
}

/// The attributes of an item, for the test gate.
fn item_attrs(item: &syn::Item) -> &[syn::Attribute] {
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

/// The attributes of an impl item, for the test gate.
fn impl_item_attrs(item: &syn::ImplItem) -> &[syn::Attribute] {
    match item {
        syn::ImplItem::Const(i) => &i.attrs,
        syn::ImplItem::Fn(i) => &i.attrs,
        syn::ImplItem::Type(i) => &i.attrs,
        syn::ImplItem::Macro(i) => &i.attrs,
        _ => &[],
    }
}

/// The attributes of a trait item, for the test gate.
fn trait_item_attrs(item: &syn::TraitItem) -> &[syn::Attribute] {
    match item {
        syn::TraitItem::Const(i) => &i.attrs,
        syn::TraitItem::Fn(i) => &i.attrs,
        syn::TraitItem::Type(i) => &i.attrs,
        syn::TraitItem::Macro(i) => &i.attrs,
        _ => &[],
    }
}

impl<'ast> Visit<'ast> for Seek {
    // Test code is no build of the client or the relay.
    fn visit_item(&mut self, item: &'ast syn::Item) {
        if !test_only(item_attrs(item)) {
            syn::visit::visit_item(self, item);
        }
    }

    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        if !test_only(impl_item_attrs(item)) {
            syn::visit::visit_impl_item(self, item);
        }
    }

    fn visit_trait_item(&mut self, item: &'ast syn::TraitItem) {
        if !test_only(trait_item_attrs(item)) {
            syn::visit::visit_trait_item(self, item);
        }
    }

    // A method of that name declared outside: a second door to the value.
    fn visit_signature(&mut self, sig: &'ast syn::Signature) {
        self.name(&sig.ident);
        syn::visit::visit_signature(self, sig);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        self.name(&call.method);
        syn::visit::visit_expr_method_call(self, call);
    }

    // A path in an expression, a type, a bound or an impl: `Secret::expose`,
    // `zeroize::Zeroizing<T>`, `impl Zeroize for T`.
    fn visit_path_segment(&mut self, segment: &'ast syn::PathSegment) {
        self.name(&segment.ident);
        syn::visit::visit_path_segment(self, segment);
    }

    fn visit_use_path(&mut self, path: &'ast syn::UsePath) {
        self.name(&path.ident);
        syn::visit::visit_use_path(self, path);
    }

    fn visit_use_name(&mut self, name: &'ast syn::UseName) {
        self.name(&name.ident);
    }

    fn visit_use_rename(&mut self, rename: &'ast syn::UseRename) {
        self.name(&rename.ident);
        self.name(&rename.rename);
    }

    fn visit_item_extern_crate(&mut self, item: &'ast syn::ItemExternCrate) {
        self.name(&item.ident);
        if let Some((_, rename)) = &item.rename {
            self.name(rename);
        }
    }

    // A derive list, `#[zeroize(skip)]`, a `serialize_with` path: an
    // attribute is tokens, read whole.
    fn visit_attribute(&mut self, attr: &'ast syn::Attribute) {
        self.tokens(attr.meta.to_token_stream());
    }

    // An included file is out of the scan's reach, so the call fails
    // closed. The one admitted form is the build script's output.
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if mac.path.is_ident("include") && !includes_build_output(&mac.tokens) {
            self.found.push(format!(
                "{}:{}: `include!` is named here; the gate reads no file it includes, so it \
                 cannot show that the file reads no secret (ARCH-047): declare a `mod`, or \
                 include the build script's output through `concat!(env!(\"OUT_DIR\"), ..)`",
                self.file,
                mac.path.span().start().line,
            ));
        }
        self.tokens(mac.to_token_stream());
    }
}

#[cfg(test)]
#[path = "secrets_tests.rs"]
mod tests;
