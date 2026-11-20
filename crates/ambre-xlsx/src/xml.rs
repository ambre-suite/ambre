//! The XML reading the parts share: a small tree for the parts that stay
//! small, and the text and attribute decoding of the streamed parts.
//!
//! Every name compares by its local part, so a file that writes `x:sheet`
//! under a prefix reads like one that writes `sheet`.

use std::borrow::Cow;

use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::{BytesRef, BytesStart, Event};
use quick_xml::{Reader, XmlVersion};

use ambre_opc::TreeBudget;

/// The deepest nesting a tree part may hold. The parts the tree reads nest
/// below ten levels; a deeper one is hostile.
const MAX_DEPTH: usize = 64;

/// Why an XML part did not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlError(String);

impl XmlError {
    pub(crate) fn new(reason: impl Into<String>) -> Self {
        Self(reason.into())
    }

    /// The reason, in the parser's words.
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.0
    }
}

impl From<quick_xml::Error> for XmlError {
    fn from(e: quick_xml::Error) -> Self {
        Self(e.to_string())
    }
}

impl From<ambre_opc::TreeTooLarge> for XmlError {
    fn from(e: ambre_opc::TreeTooLarge) -> Self {
        Self(e.to_string())
    }
}

impl From<quick_xml::events::attributes::AttrError> for XmlError {
    fn from(e: quick_xml::events::attributes::AttrError) -> Self {
        Self(e.to_string())
    }
}

/// One element of a small part: its attributes and its child elements. No
/// part the tree reads keeps a value in element text.
#[derive(Debug, Clone, Default)]
pub(crate) struct Element {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<Element>,
}

impl Element {
    /// The local name of the element.
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    /// The value of the attribute whose local name is `local`.
    pub(crate) fn attr(&self, local: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == local)
            .map(|(_, v)| v.as_str())
    }

    /// The first child named `local`.
    pub(crate) fn child(&self, local: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.name == local)
    }

    /// Every child named `local`, in document order.
    pub(crate) fn children<'a>(&'a self, local: &'a str) -> impl Iterator<Item = &'a Element> {
        self.children.iter().filter(move |c| c.name == local)
    }

    /// Every child, in document order.
    pub(crate) fn all_children(&self) -> &[Element] {
        &self.children
    }
}

/// The root element of `bytes`.
///
/// # Errors
///
/// The part refuses when it is one of these:
/// * malformed, or without a root;
/// * a part that declares a document type;
/// * nested past [`MAX_DEPTH`];
/// * a tree past the node or attribute bound of [`TreeBudget`].
pub(crate) fn parse_tree(bytes: &[u8]) -> Result<Element, XmlError> {
    let mut reader = Reader::from_str(utf8(bytes)?);
    let mut stack: Vec<Element> = Vec::new();
    let mut budget = TreeBudget::default();
    loop {
        match reader.read_event()? {
            Event::Start(e) => {
                if stack.len() >= MAX_DEPTH {
                    return Err(XmlError::new("the part nests too deep"));
                }
                stack.push(element_of(&e, &mut budget)?);
            }
            Event::Empty(e) => {
                let element = element_of(&e, &mut budget)?;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(element),
                    None => return Ok(element),
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
            Event::Eof => return Err(XmlError::new("the part holds no root element")),
            Event::DocType(_) => return Err(document_type()),
            Event::Text(_)
            | Event::CData(_)
            | Event::GeneralRef(_)
            | Event::Comment(_)
            | Event::Decl(_)
            | Event::PI(_) => {}
        }
    }
}

/// The refusal of a part that declares a document type. An entity it
/// declares could expand without bound or name a file outside the package,
/// so the four other codecs refuse one too (`W307`).
pub(crate) fn document_type() -> XmlError {
    XmlError::new("the part declares a document type")
}

/// The element `e` opens, charged to `budget` as one node and each
/// attribute by its value before the tree holds it.
fn element_of(e: &BytesStart<'_>, budget: &mut TreeBudget) -> Result<Element, XmlError> {
    budget.node()?;
    let attrs = attributes(e)?;
    for (_, value) in &attrs {
        budget.attr(ambre_core::Count::of(value.len()))?;
    }
    Ok(Element {
        name: local_name(e),
        attrs,
        children: Vec::new(),
    })
}

/// The text of a part. Every part an OOXML producer writes is UTF-8.
///
/// # Errors
///
/// A part that is no UTF-8.
pub(crate) fn utf8(bytes: &[u8]) -> Result<&str, XmlError> {
    let text = std::str::from_utf8(bytes).map_err(|e| XmlError::new(e.to_string()))?;
    Ok(text.strip_prefix('\u{feff}').unwrap_or(text))
}

/// The local name of a start tag.
pub(crate) fn local_name(e: &BytesStart<'_>) -> String {
    e.local_name().as_ref().to_owned()
}

/// Every attribute of a start tag, by local name, its value unescaped.
///
/// # Errors
///
/// A malformed attribute or an unknown entity in a value.
pub(crate) fn attributes(e: &BytesStart<'_>) -> Result<Vec<(String, String)>, XmlError> {
    e.attributes()
        .map(|a| {
            let a = a?;
            let key = a.key.local_name().as_ref().to_owned();
            let value = a.normalized_value(XmlVersion::Explicit1_0)?.into_owned();
            Ok((key, value))
        })
        .collect()
}

/// The value of the attribute `local` of a start tag.
///
/// # Errors
///
/// A malformed attribute or an unknown entity in a value.
pub(crate) fn attr(e: &BytesStart<'_>, local: &str) -> Result<Option<String>, XmlError> {
    for a in e.attributes() {
        let a = a?;
        if a.key.local_name().as_ref() == local {
            return Ok(Some(
                a.normalized_value(XmlVersion::Explicit1_0)?.into_owned(),
            ));
        }
    }
    Ok(None)
}

/// Append the character or the entity `r` stands for to `out`.
///
/// # Errors
///
/// An entity XML does not predefine, or a character reference to no
/// character.
pub(crate) fn push_reference(out: &mut String, r: &BytesRef<'_>) -> Result<(), XmlError> {
    if let Some(c) = r.resolve_char_ref()? {
        out.push(c);
        return Ok(());
    }
    let name: &str = r;
    let text = resolve_predefined_entity(name)
        .ok_or_else(|| XmlError::new("the part names an unknown entity"))?;
    out.push_str(text);
    Ok(())
}

/// The text of a string item with its `_xHHHH_` escapes decoded (ECMA-376
/// part 1, 22.9.2.19). The format escapes a character XML cannot carry, a
/// carriage return most often, as its code in hexadecimal.
pub(crate) fn decode_ooxml_escapes(text: &str) -> Cow<'_, str> {
    if !text.contains("_x") {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("_x") {
        let (before, tail) = rest.split_at(at);
        out.push_str(before);
        let decoded = tail
            .get(2..6)
            .filter(|hex| hex.bytes().all(|b| b.is_ascii_hexdigit()))
            .filter(|_| tail.get(6..7) == Some("_"))
            .and_then(|hex| u32::from_str_radix(hex, 16).ok())
            .and_then(char::from_u32);
        match decoded {
            Some(c) => {
                out.push(c);
                rest = tail.get(7..).unwrap_or_default();
            }
            None => {
                out.push_str("_x");
                rest = tail.get(2..).unwrap_or_default();
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// `text` escaped for an element or an attribute of a written part. The XML
/// characters take their escapes, and a control character its `_xHHHH_`
/// code. A literal `_x` that would read as an escape takes a `_x005F_`.
pub(crate) fn escape(text: &str) -> Cow<'_, str> {
    let needs_ooxml = text
        .chars()
        .any(|c| (c.is_control() && !matches!(c, '\t' | '\n')) || c == '_');
    if !needs_ooxml {
        return quick_xml::escape::escape(text);
    }
    let mut out = String::with_capacity(text.len() + 8);
    for (at, c) in text.char_indices() {
        match c {
            '_' if is_escape_at(text.get(at..).unwrap_or_default()) => out.push_str("_x005F_"),
            c if c.is_control() && !matches!(c, '\t' | '\n') => {
                out.push_str(&format!("_x{:04X}_", u32::from(c)));
            }
            c => out.push(c),
        }
    }
    Cow::Owned(quick_xml::escape::escape(&out).into_owned())
}

/// Whether `text` opens with a `_xHHHH_` escape.
fn is_escape_at(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() >= 7
        && bytes.starts_with(b"_x")
        && bytes
            .get(2..6)
            .is_some_and(|h| h.iter().all(u8::is_ascii_hexdigit))
        && bytes.get(6) == Some(&b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_escaped_carriage_return_decodes_and_a_literal_escape_round_trips() {
        assert_eq!(decode_ooxml_escapes("a_x000D_b"), "a\rb");
        assert_eq!(decode_ooxml_escapes("a_x00"), "a_x00");
        for text in ["a\rb", "_x0041_", "plain & <tag>", "x_y", "_x00D_"] {
            assert_eq!(
                decode_ooxml_escapes(&quick_xml_unescape(&escape(text))),
                text
            );
        }
    }

    fn quick_xml_unescape(text: &str) -> String {
        quick_xml::escape::unescape(text)
            .map(Cow::into_owned)
            .unwrap_or_default()
    }

    #[test]
    fn a_tree_reads_prefixed_names_by_their_local_part() {
        let tree = parse_tree(b"<x:a xmlns:x='u' x:k='v &amp; w'><x:b>t</x:b><c/></x:a>");
        let Ok(tree) = tree else {
            panic!("the part reads");
        };
        assert_eq!(tree.name(), "a");
        assert_eq!(tree.attr("k"), Some("v & w"));
        assert!(tree.child("b").is_some());
        assert!(tree.child("c").is_some());
    }

    #[test]
    fn a_part_nested_too_deep_is_refused() {
        let deep = "<a>".repeat(100) + &"</a>".repeat(100);
        assert!(parse_tree(deep.as_bytes()).is_err());
    }

    /// `W72`: a part of empty elements past the node bound of a tree is
    /// refused, though it reads under the bound of a part.
    #[test]
    fn a_part_past_the_node_bound_is_refused() {
        let past = usize::try_from(ambre_opc::MAX_TREE_NODES.get()).expect("fits");
        let wide = format!("<a>{}</a>", "<b/>".repeat(past));
        assert!(parse_tree(wide.as_bytes()).is_err());
        let under = format!("<a>{}</a>", "<b/>".repeat(past - 1));
        assert!(parse_tree(under.as_bytes()).is_ok());
    }

    /// `W307`: a part that declares a document type is refused, as the four
    /// other codecs refuse it.
    #[test]
    fn a_part_that_declares_a_document_type_is_refused() {
        let part = b"<?xml version='1.0'?><!DOCTYPE a [<!ENTITY e 'x'>]><a/>";
        assert_eq!(parse_tree(part).err(), Some(document_type()));
    }
}
