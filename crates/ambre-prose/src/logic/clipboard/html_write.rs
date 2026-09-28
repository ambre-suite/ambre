//! The `text/html` half of a copy, which a word processor, a mail client and
//! a web editor read with the formatting kept.
//!
//! The markup is inert. It holds the elements a block and a mark stand for,
//! and one inline style for a colour, a size and a family. It holds no
//! script, no event handler and no class a page would style.

use std::fmt::Write as _;

use ambre_bus::app::prose::OrderedStyle;

use super::{ClipBlock, ClipKind, ClipRun, ClipValue};

/// The flag marks, in the order their elements nest, each with its element.
const ELEMENTS: [(&str, &str); 7] = [
    ("b", "strong"),
    ("i", "em"),
    ("u", "u"),
    ("s", "s"),
    ("sup", "sup"),
    ("sub", "sub"),
    ("c", "code"),
];

/// The list element a block sits in, spelled as its opening tag.
fn list_open(kind: &ClipKind) -> Option<String> {
    match kind {
        ClipKind::Bullet | ClipKind::Todo(_) => Some("<ul>".to_owned()),
        ClipKind::Ordered(code) => Some(
            match OrderedStyle::from_code(*code) {
                Some(OrderedStyle::LowerAlpha) => "<ol type=\"a\">",
                Some(OrderedStyle::UpperAlpha) => "<ol type=\"A\">",
                Some(OrderedStyle::LowerRoman) => "<ol type=\"i\">",
                Some(OrderedStyle::UpperRoman) => "<ol type=\"I\">",
                _ => "<ol>",
            }
            .to_owned(),
        ),
        _ => None,
    }
}

/// Write `blocks` as an HTML fragment. Consecutive list items of one kind
/// share one list element.
pub(super) fn write(blocks: &[ClipBlock]) -> String {
    let mut html = String::from("<meta charset=\"utf-8\">");
    let mut list: Option<String> = None;
    for block in blocks {
        let wanted = list_open(&block.kind);
        if wanted != list {
            close_list(&mut html, list.as_deref());
            if let Some(open) = &wanted {
                html.push_str(open);
            }
            list = wanted;
        }
        match &block.kind {
            ClipKind::Paragraph => element(&mut html, "p", &block.runs),
            ClipKind::Heading(level) => {
                element(
                    &mut html,
                    &format!("h{}", (*level).clamp(1, 6)),
                    &block.runs,
                );
            }
            ClipKind::Bullet | ClipKind::Ordered(_) => element(&mut html, "li", &block.runs),
            ClipKind::Todo(done) => {
                html.push_str("<li><input type=\"checkbox\" disabled");
                if *done {
                    html.push_str(" checked");
                }
                html.push('>');
                runs(&mut html, &block.runs);
                html.push_str("</li>");
            }
            ClipKind::Code(lang) => {
                html.push_str("<pre><code");
                if !lang.is_empty() {
                    let _ = write!(html, " class=\"language-{}\"", escape(lang));
                }
                html.push('>');
                for run in &block.runs {
                    html.push_str(&escape(&run.text));
                }
                html.push_str("</code></pre>");
            }
        }
    }
    close_list(&mut html, list.as_deref());
    html
}

fn close_list(html: &mut String, open: Option<&str>) {
    match open {
        Some(tag) if tag.starts_with("<ol") => html.push_str("</ol>"),
        Some(_) => html.push_str("</ul>"),
        None => {}
    }
}

fn element(html: &mut String, tag: &str, block: &[ClipRun]) {
    let _ = write!(html, "<{tag}>");
    runs(html, block);
    let _ = write!(html, "</{tag}>");
}

fn value<'a>(run: &'a ClipRun, key: &str) -> Option<&'a str> {
    run.marks.iter().find_map(|(k, v)| match v {
        ClipValue::Text(text) if k == key => Some(text.as_str()),
        _ => None,
    })
}

fn has(run: &ClipRun, key: &str) -> bool {
    run.marks.iter().any(|(k, _)| k == key)
}

/// Each run in the elements its marks stand for. A run of no text still
/// writes nothing, so an empty paragraph stays one.
fn runs(html: &mut String, block: &[ClipRun]) {
    for run in block {
        if run.text.is_empty() {
            continue;
        }
        let mut closing = Vec::new();
        if let Some(href) = value(run, "a") {
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
        if let Some(color) = value(run, "fg") {
            let _ = write!(style, "color:{color};");
        }
        if let Some(color) = value(run, "bg") {
            let _ = write!(style, "background-color:{color};");
        }
        if let Some(size) = value(run, "fs") {
            let _ = write!(style, "font-size:{size}pt;");
        }
        if let Some(family) = value(run, "ff") {
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
