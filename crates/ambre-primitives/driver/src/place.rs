//! Where a site sits: its crate directory, its file, its module, its item path,
//! and the type text a key carries.
//!
//! A key names no line, so an edit above a site leaves it as it was. The
//! crate is the directory of the package cargo compiles, read from
//! `CARGO_MANIFEST_DIR`. The file is the path rustc read, from that directory.

use std::path::{Component, Path, PathBuf};

use rustc_hir as hir;
use rustc_hir::def::DefKind;
use rustc_hir::def_id::{DefId, LocalDefId};
use rustc_middle::ty::TyCtxt;
use rustc_span::Span;

/// The separator between two fields of a key.
pub const SEPARATOR: &str = " | ";

/// What one compile unit is: the package it belongs to, and where.
pub struct Unit {
    /// The directory of the package, as the workspace names it: `ambre-core`.
    pub krate: String,
    /// The absolute directory of the package.
    pub manifest: PathBuf,
    /// The directory rustc runs in: the workspace root, for a member.
    pub cwd: PathBuf,
}

impl Unit {
    /// The unit cargo compiles now. Outside cargo, as in the fixture tests,
    /// the current directory stands for the package.
    pub fn current() -> Self {
        let cwd = std::env::current_dir().unwrap_or_default();
        let manifest =
            std::env::var_os("CARGO_MANIFEST_DIR").map_or_else(|| cwd.clone(), PathBuf::from);
        let krate = manifest
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        Self {
            krate,
            manifest: lexical(&manifest),
            cwd,
        }
    }
}

/// A source position: the file from the package, the file from the workspace
/// root, and the line.
pub struct Position {
    /// The file from the package directory, as a key names it. `None` for a
    /// file outside the package.
    pub file: Option<String>,
    /// The absolute path, each `..` undone.
    pub path: PathBuf,
    /// The path from the workspace root, as a message shows it.
    pub shown: String,
    pub line: usize,
}

impl Position {
    /// `file:line`, from the workspace root.
    pub fn at(&self) -> String {
        format!("{}:{}", self.shown, self.line)
    }
}

/// The path with each `.` dropped and each `..` undone, as text and never
/// through the disk.
pub fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// Where `span` sits once each macro call is followed back to the source.
pub fn position(tcx: TyCtxt<'_>, unit: &Unit, span: Span) -> Position {
    let span = span.source_callsite();
    let lo = tcx.sess.source_map().lookup_char_pos(span.lo());
    let name = lo.file.name.prefer_local_unconditionally().to_string();
    let path = lexical(&unit.cwd.join(&name));
    let file = path
        .strip_prefix(&unit.manifest)
        .ok()
        .map(|rest| rest.to_string_lossy().replace('\\', "/"));
    let shown = path.strip_prefix(&unit.cwd).map_or_else(
        |_| name.clone(),
        |rest| rest.to_string_lossy().replace('\\', "/"),
    );
    Position {
        file,
        path,
        shown,
        line: lo.line,
    }
}

/// The text of a type or a path as a key writes it. A space stays only
/// between two words, or before a lifetime that follows a word.
pub fn normalise(text: &str) -> String {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in chars {
        if c.is_whitespace() {
            space = true;
            continue;
        }
        if space && out.chars().last().is_some_and(word) && (word(c) || c == '\'') {
            out.push(' ');
        }
        space = false;
        out.push(c);
    }
    out
}

/// The source text of `span`, normalised, or `None` when a macro wrote it.
pub fn written(tcx: TyCtxt<'_>, span: Span) -> Option<String> {
    if let Origin::Macro { .. } = origin(tcx, span) {
        return None;
    }
    tcx.sess
        .source_map()
        .span_to_snippet(span.source_callsite())
        .ok()
        .map(|text| normalise(&strip_comments(&text)))
        .filter(|text| !text.is_empty())
}

/// The text without its `//` and `/* */` comments, which a key never holds.
fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('/') {
        let (head, tail) = rest.split_at(at);
        out.push_str(head);
        if let Some(after) = tail.strip_prefix("//") {
            rest = after.find('\n').map_or("", |end| &after[end..]);
        } else if let Some(after) = tail.strip_prefix("/*") {
            out.push(' ');
            rest = after.find("*/").map_or("", |end| &after[end + 2..]);
        } else {
            out.push('/');
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    out
}

/// The module path of the item `def`, as `crate::a::b`.
pub fn module_path(tcx: TyCtxt<'_>, def: LocalDefId) -> String {
    let module = tcx.parent_module_from_def_id(def);
    let mut path = vec!["crate".to_owned()];
    path.extend(
        tcx.def_path(module.to_def_id())
            .data
            .iter()
            .filter_map(|data| data.data.get_opt_name())
            .map(|name| name.to_string()),
    );
    path.join("::")
}

/// A path that names `def` the same way from every crate: the crate name,
/// then each named segment of its definition path.
pub fn stable_path(tcx: TyCtxt<'_>, def: DefId) -> String {
    let mut path = vec![tcx.crate_name(def.krate).to_string()];
    path.extend(
        tcx.def_path(def)
            .data
            .iter()
            .map(|data| match data.data.get_opt_name() {
                Some(name) => name.to_string(),
                None => format!("{{{:?}#{}}}", data.data, data.disambiguator)
                    .replace("{Impl#", "{impl#"),
            }),
    );
    path.join("::")
}

/// The label of an impl in an item path: `Type`, or `<Type as Trait>`.
pub fn impl_label(tcx: TyCtxt<'_>, def: LocalDefId) -> String {
    let hir::Node::Item(item) = tcx.hir_node_by_def_id(def) else {
        return "_".to_owned();
    };
    let hir::ItemKind::Impl(imp) = item.kind else {
        return "_".to_owned();
    };
    let self_ty = written(tcx, imp.self_ty.span).unwrap_or_else(|| {
        short(
            &tcx.type_of(def)
                .instantiate_identity()
                .skip_normalization()
                .to_string(),
        )
    });
    match imp.of_trait {
        Some(header) => {
            let path = written(tcx, header.trait_ref.path.span).unwrap_or_else(|| {
                tcx.impl_opt_trait_ref(def)
                    .map(|t| normalise(&tcx.def_path_str(t.skip_binder().def_id)))
                    .unwrap_or_default()
            });
            format!("<{self_ty} as {path}>")
        }
        None => self_ty,
    }
}

/// The item path above `def`, from its module down: each enclosing impl,
/// trait, `fn`, constant, static and type, joined by `::`.
pub fn scope(tcx: TyCtxt<'_>, def: LocalDefId) -> Vec<String> {
    let module = tcx.parent_module_from_def_id(def).to_local_def_id();
    let mut labels = Vec::new();
    let mut at = tcx.local_parent(def);
    while at != module {
        match tcx.def_kind(at) {
            DefKind::Impl { .. } => labels.push(impl_label(tcx, at)),
            DefKind::Fn
            | DefKind::AssocFn
            | DefKind::Const
            | DefKind::AssocConst
            | DefKind::Static { .. }
            | DefKind::Trait
            | DefKind::Struct
            | DefKind::Enum
            | DefKind::Union
            | DefKind::Variant
            | DefKind::Mod => {
                if let Some(name) = tcx.opt_item_name(at.to_def_id()) {
                    labels.push(name.to_string());
                }
            }
            _ => {}
        }
        if at == tcx.local_parent(at) {
            break;
        }
        at = tcx.local_parent(at);
    }
    labels.reverse();
    labels
}

/// The item path of a site: its scope, then `leaf`.
pub fn item(tcx: TyCtxt<'_>, def: LocalDefId, leaf: &str) -> String {
    let mut path = scope(tcx, def);
    path.push(leaf.to_owned());
    path.join("::")
}

/// The visibility a declaration writes, read from its text: the gate reads
/// `pub(crate)` at a crate root as restricted, as the source says.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Written {
    Private,
    Restricted,
    Public,
}

/// The visibility the text at `span` writes.
pub fn written_vis(tcx: TyCtxt<'_>, span: Span) -> Written {
    if span.is_empty() {
        return Written::Private;
    }
    if let Origin::Macro {
        workspace: false, ..
    } = origin(tcx, span)
    {
        // A foreign macro wrote the word: read what it wrote back from the
        // tokens it took, which the call site spans.
        let text = tcx
            .sess
            .source_map()
            .span_to_snippet(span)
            .unwrap_or_default();
        return match text.trim() {
            "" => Written::Private,
            "pub" => Written::Public,
            t if t.starts_with("pub(") || t.starts_with("pub (") => Written::Restricted,
            _ => Written::Public,
        };
    }
    let text = tcx
        .sess
        .source_map()
        .span_to_snippet(span)
        .unwrap_or_default();
    let text = normalise(&text);
    if text.is_empty() {
        Written::Private
    } else if text == "pub" {
        Written::Public
    } else {
        Written::Restricted
    }
}

/// Who wrote a span: the source, or a macro, by name, and whether a crate of
/// the workspace defines that macro.
pub enum Origin {
    Source,
    Macro { name: String, workspace: bool },
}

impl Origin {
    /// The word a record writes: `source`, `macro:name` or `foreign:name`.
    pub fn word(&self) -> String {
        match self {
            Self::Source => "source".to_owned(),
            Self::Macro {
                name,
                workspace: true,
            } => format!("macro:{name}"),
            Self::Macro {
                name,
                workspace: false,
            } => format!("foreign:{name}"),
        }
    }
}

/// Who wrote `span`: the outermost macro call it comes from, if any.
pub fn origin(tcx: TyCtxt<'_>, span: Span) -> Origin {
    if !span.from_expansion() {
        return Origin::Source;
    }
    // The innermost macro wrote the token. A desugaring, `async` or `?`,
    // writes what its call site wrote.
    let mut data = span.ctxt().outer_expn_data();
    let name = loop {
        match data.kind {
            rustc_span::ExpnKind::Macro(_, name) => break name,
            _ if data.call_site.from_expansion() => {
                data = data.call_site.ctxt().outer_expn_data();
            }
            _ => return Origin::Source,
        }
    };
    let name = name.to_string();
    let workspace = data
        .macro_def_id
        .is_some_and(|def| crate::types::is_workspace(tcx, def));
    Origin::Macro { name, workspace }
}

/// The text rustc prints for a type a macro wrote, each module path dropped:
/// `rest::KeySetup` reads `KeySetup`, as the source would write it.
pub fn short(text: &str) -> String {
    let text = normalise(text);
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_alphanumeric() || c == '_' {
            word.push(c);
            continue;
        }
        let path = c == ':' && chars.peek() == Some(&':');
        let module = word
            .chars()
            .next()
            .is_some_and(|f| f.is_lowercase() || f == '_');
        if path && module {
            chars.next();
            word.clear();
            continue;
        }
        out.push_str(&word);
        word.clear();
        out.push(c);
    }
    out.push_str(&word);
    out
}
