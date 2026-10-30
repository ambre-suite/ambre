//! The `text/html` half of a copy, which a word processor, a mail client and
//! a web editor read with the formatting kept.
//!
//! The markup is inert. It holds the elements a block and a mark stand for,
//! and one inline style for a colour, a size and a family. It holds no
//! script, no event handler and no class a page would style.

use std::fmt::Write as _;

use ambre_bus::app::prose::{HeadingLevel, OrderedStyle};
use ambre_core::Rgba;
use html5ever::{LocalName, local_name};

use super::{ClipBlock, ClipKind, ClipRun, ClipValue, safe_family, safe_link};
use crate::mark::{self, MarkRoot};

/// The flag marks, in the order their elements nest, each with its element.
const ELEMENTS: [(MarkRoot, &str); 7] = [
    (MarkRoot::Bold, "strong"),
    (MarkRoot::Italic, "em"),
    (MarkRoot::Underline, "u"),
    (MarkRoot::Strike, "s"),
    (MarkRoot::Superscript, "sup"),
    (MarkRoot::Subscript, "sub"),
    (MarkRoot::Code, "code"),
];

/// The list element a block sits in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ListElement {
    Unordered,
    Ordered(OrderedStyle),
}

impl ListElement {
    /// The element `kind` sits in, `None` for a block outside a list.
    fn of(kind: &ClipKind) -> Option<Self> {
        match kind {
            ClipKind::Bullet | ClipKind::Todo(_) => Some(Self::Unordered),
            ClipKind::Ordered(style) => Some(Self::Ordered(*style)),
            _ => None,
        }
    }

    /// The opening tag.
    fn open(self) -> &'static str {
        match self {
            Self::Unordered => "<ul>",
            Self::Ordered(OrderedStyle::LowerAlpha) => "<ol type=\"a\">",
            Self::Ordered(OrderedStyle::UpperAlpha) => "<ol type=\"A\">",
            Self::Ordered(OrderedStyle::LowerRoman) => "<ol type=\"i\">",
            Self::Ordered(OrderedStyle::UpperRoman) => "<ol type=\"I\">",
            Self::Ordered(OrderedStyle::Decimal) => "<ol>",
        }
    }

    /// The closing tag.
    fn close(self) -> &'static str {
        match self {
            Self::Unordered => "</ul>",
            Self::Ordered(_) => "</ol>",
        }
    }
}

/// Write `blocks` as an HTML fragment. Consecutive list items of one kind
/// share one list element.
pub(super) fn write(blocks: &[ClipBlock]) -> String {
    let mut html = String::from("<meta charset=\"utf-8\">");
    let mut list: Option<ListElement> = None;
    for block in blocks {
        let wanted = ListElement::of(&block.kind);
        if wanted != list {
            if let Some(open) = list {
                html.push_str(open.close());
            }
            if let Some(wanted) = wanted {
                html.push_str(wanted.open());
            }
            list = wanted;
        }
        match &block.kind {
            ClipKind::Paragraph => element(&mut html, &local_name!("p"), &block.runs),
            ClipKind::Heading(level) => {
                let tag = match level {
                    HeadingLevel::H1 => local_name!("h1"),
                    HeadingLevel::H2 => local_name!("h2"),
                    HeadingLevel::H3 => local_name!("h3"),
                    HeadingLevel::H4 => local_name!("h4"),
                    HeadingLevel::H5 => local_name!("h5"),
                    HeadingLevel::H6 => local_name!("h6"),
                };
                element(&mut html, &tag, &block.runs);
            }
            ClipKind::Bullet | ClipKind::Ordered(_) => {
                element(&mut html, &local_name!("li"), &block.runs);
            }
            ClipKind::Todo(done) => {
                html.push_str("<li><input type=\"checkbox\" disabled");
                if done.is_done() {
                    html.push_str(" checked");
                }
                html.push('>');
                runs(&mut html, &block.runs);
                html.push_str("</li>");
            }
            ClipKind::Code(lang) => {
                html.push_str("<pre><code");
                if let Some(lang) = lang {
                    let _ = write!(html, " class=\"language-{}\"", escape(lang.as_str()));
                }
                html.push('>');
                for run in &block.runs {
                    html.push_str(&escape(&run.text));
                }
                html.push_str("</code></pre>");
            }
        }
    }
    if let Some(open) = list {
        html.push_str(open.close());
    }
    html
}

fn element(html: &mut String, tag: &LocalName, block: &[ClipRun]) {
    let _ = write!(html, "<{tag}>");
    runs(html, block);
    let _ = write!(html, "</{tag}>");
}

fn value(run: &ClipRun, key: MarkRoot) -> Option<&str> {
    run.marks.iter().find_map(|(k, v)| match v {
        ClipValue::Text(text) if k.as_str() == key.as_str() => Some(text.as_str()),
        _ => None,
    })
}

/// The colour under `key`, `None` for one the grammar refuses.
fn color(run: &ClipRun, key: MarkRoot) -> Option<Rgba> {
    mark::parse_color(value(run, key)?).ok()
}

fn has(run: &ClipRun, key: MarkRoot) -> bool {
    run.marks.iter().any(|(k, _)| k.as_str() == key.as_str())
}

/// Each run in the elements its marks stand for. A run of no text still
/// writes nothing, so an empty paragraph stays one.
fn runs(html: &mut String, block: &[ClipRun]) {
    for run in block {
        if run.text.is_empty() {
            continue;
        }
        let mut closing = Vec::new();
        // A peer writes a mark value straight into the document, unchecked.
        // A target a paste would refuse keeps its text alone (`PROSE-UI-235`).
        if let Some(href) = value(run, MarkRoot::Link).filter(|href| safe_link(href)) {
            let _ = write!(html, "<a href=\"{}\">", escape(href));
            closing.push("a");
        }
        for (key, tag) in ELEMENTS {
            if has(run, key) {
                let _ = write!(html, "<{tag}>");
                closing.push(tag);
            }
        }
        // A colour and a size are written in their canonical spelling, never
        // as stored, so a peer's value cannot carry a declaration of its own.
        let mut style = String::new();
        if let Some(color) = color(run, MarkRoot::Ink) {
            let _ = write!(style, "color:{};", mark::canonical_color(color));
        }
        if let Some(color) = color(run, MarkRoot::Highlight) {
            let _ = write!(style, "background-color:{};", mark::canonical_color(color));
        }
        if let Some(size) = value(run, MarkRoot::FontSize)
            .and_then(|size| mark::parse_font_size(size).ok())
            .and_then(|size| mark::canonical_number(size.points()))
        {
            let _ = write!(style, "font-size:{size}pt;");
        }
        // A family passes the paste's own check, then loses what would close
        // the quoted name.
        if let Some(family) = value(run, MarkRoot::FontFamily).filter(|name| safe_family(name)) {
            let family: String = family
                .chars()
                .filter(|c| !matches!(c, '\'' | '"' | ';' | '<' | '>' | '\\'))
                .collect();
            let _ = write!(style, "font-family:'{family}';");
        }
        if !style.is_empty() {
            let _ = write!(html, "<span style=\"{}\">", escape(&style));
            closing.push("span");
        }
        html.push_str(&escape(&run.text));
        for tag in closing.into_iter().rev() {
            let _ = write!(html, "</{tag}>");
        }
    }
}

/// Text and attribute values with the characters HTML reads as markup
/// escaped.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}
