//! The `text/html` half of a copy, which a word processor, a mail client and
//! a web editor read with the formatting kept.
//!
//! The markup is inert. It holds the elements a block and a mark stand for,
//! and one inline style for a colour, a size and a family. It holds no
//! script, no event handler and no class a page would style.

use std::fmt::Write as _;

use ambre_bus::app::prose::OrderedStyle;

use super::{ClipBlock, ClipKind, ClipRun, ClipValue};
use crate::mark::MarkRoot;

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
            ClipKind::Paragraph => element(&mut html, "p", &block.runs),
            ClipKind::Heading(level) => {
                element(&mut html, &format!("h{level}"), &block.runs);
            }
            ClipKind::Bullet | ClipKind::Ordered(_) => element(&mut html, "li", &block.runs),
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

fn element(html: &mut String, tag: &str, block: &[ClipRun]) {
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
        if let Some(href) = value(run, MarkRoot::Link) {
            let _ = write!(html, "<a href=\"{}\">", escape(href));
            closing.push("a");
        }
        for (key, tag) in ELEMENTS {
            if has(run, key) {
                let _ = write!(html, "<{tag}>");
                closing.push(tag);
            }
        }
        let mut style = String::new();
        if let Some(color) = value(run, MarkRoot::Ink) {
            let _ = write!(style, "color:{color};");
        }
        if let Some(color) = value(run, MarkRoot::Highlight) {
            let _ = write!(style, "background-color:{color};");
        }
        if let Some(size) = value(run, MarkRoot::FontSize) {
            let _ = write!(style, "font-size:{size}pt;");
        }
        if let Some(family) = value(run, MarkRoot::FontFamily) {
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
