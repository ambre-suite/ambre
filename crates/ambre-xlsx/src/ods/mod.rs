//! The OpenDocument spreadsheet codec: an `.ods` file read into the same
//! [`Workbook`](crate::Workbook) an xlsx file reads into, and written back
//! (`2-Trame/features/ambre-spec-trame-import-export.md`, section 3).
//!
//! The codec shares the model, the report and the formula translation of
//! the xlsx codec. A spreadsheet read from either format reaches Trame
//! through one map, so the two cannot drift. Each format
//! difference sits here: the package (`ambre_opc::odf`), the formula
//! dialect ([`formula`]), the number styles ([`number`]) and the cell
//! styles ([`styles`]).

mod formula;
mod number;
mod read;
mod styles;
mod write;

pub(crate) use read::read;
pub(crate) use write::write;

use ambre_core::{Doc, Px};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use crate::xml::{self, XmlError};

/// The deepest nesting a tree may hold. A style or a settings part nests
/// below ten levels; a deeper one is hostile.
const MAX_DEPTH: usize = 64;

/// One element of a part read whole: its local name, its attributes by local
/// name, its children and the text directly inside it. A number style keeps a
/// literal in its text, so the tree keeps it too.
#[derive(Debug, Clone, Default)]
pub(crate) struct Node {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<Node>,
    text: String,
}

impl Node {
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn attr(&self, local: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == local)
            .map(|(_, v)| v.as_str())
    }

    pub(crate) fn child(&self, local: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == local)
    }

    pub(crate) fn children(&self) -> &[Node] {
        &self.children
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    /// Every element of the tree, this one first, depth first.
    pub(crate) fn walk(&self) -> Vec<&Node> {
        let mut out = vec![self];
        let mut at = 0;
        while let Some(node) = out.get(at) {
            let children: Vec<&Node> = node.children.iter().collect();
            out.extend(children);
            at += 1;
        }
        out
    }
}

/// The root element of the part `bytes`.
pub(crate) fn tree(bytes: &[u8]) -> Result<Node, XmlError> {
    let mut reader = Reader::from_str(xml::utf8(bytes)?);
    loop {
        match reader.read_event()? {
            Event::Start(e) => return subtree(&mut reader, &e),
            Event::Empty(e) => return node_of(&e),
            Event::Eof => return Err(XmlError::new("the part holds no root element")),
            _ => {}
        }
    }
}

/// The element `start` opens, read up to its end from `reader`.
pub(crate) fn subtree(
    reader: &mut Reader<&[u8]>,
    start: &BytesStart<'_>,
) -> Result<Node, XmlError> {
    let mut stack = vec![node_of(start)?];
    loop {
        match reader.read_event()? {
            Event::Start(e) => {
                if stack.len() >= MAX_DEPTH {
                    return Err(XmlError::new("the part nests too deep"));
                }
                stack.push(node_of(&e)?);
            }
            Event::Empty(e) => {
                let node = node_of(&e)?;
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(node);
                }
            }
            Event::End(_) => {
                let Some(done) = stack.pop() else {
                    return Err(XmlError::new("an end tag closes nothing"));
                };
                match stack.last_mut() {
                    Some(parent) => parent.children.push(done),
                    None => return Ok(done),
                }
            }
            Event::Text(t) => {
                if let Some(node) = stack.last_mut() {
                    node.text.push_str(&t.xml10_content());
                }
            }
            Event::CData(t) => {
                if let Some(node) = stack.last_mut() {
                    node.text.push_str(&t.xml10_content());
                }
            }
            Event::GeneralRef(r) => {
                if let Some(node) = stack.last_mut() {
                    xml::push_reference(&mut node.text, &r)?;
                }
            }
            Event::Eof => return Err(XmlError::new("the part ends inside an element")),
            Event::Comment(_) | Event::Decl(_) | Event::PI(_) | Event::DocType(_) => {}
        }
    }
}

fn node_of(e: &BytesStart<'_>) -> Result<Node, XmlError> {
    Ok(Node {
        name: xml::local_name(e),
        attrs: xml::attributes(e)?,
        children: Vec::new(),
        text: String::new(),
    })
}

/// Append `text` escaped for an attribute value or element text. A white
/// space character other than the space leaves as a character reference.
/// Neither attribute normalization nor the white space rule of a paragraph
/// collapses one. A character XML 1.0 cannot carry is dropped.
pub(crate) fn push_escaped(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'.. => out.push(ch),
            _ => {}
        }
    }
}

/// `text` escaped, as [`push_escaped`] writes it.
pub(crate) fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    push_escaped(&mut out, text);
    out
}

/// The length `text` spells, such as `0.889in` or `2.258cm`, in pixels at
/// 96 dots per inch.
pub(crate) fn length_px(text: &str) -> Option<Px<Doc>> {
    let text = text.trim();
    let at = text.find(|c: char| c.is_ascii_alphabetic())?;
    let (number, unit) = text.split_at(at);
    let value: f64 = number.trim().parse().ok().filter(|v: &f64| v.is_finite())?;
    let per_unit = match unit {
        "in" => 96.0,
        "cm" => 96.0 / 2.54,
        "mm" => 96.0 / 25.4,
        "pt" => 96.0 / 72.0,
        "pc" => 16.0,
        "px" => 1.0,
        _ => return None,
    };
    Some(Px::new((value * per_unit) as f32))
}

/// The length of `px` pixels as a written part spells it, in points. A
/// whole pixel is a point count of two decimals at most, read back exact.
pub(crate) fn length_pt(px: Px<Doc>) -> String {
    format!("{}pt", f64::from(px.get()) * 0.75)
}
