//! A small owned tree of one XML part of an OpenDocument package, and the
//! writer of one.
//!
//! The reader builds the tree from quick-xml's event stream, without
//! recursion, so a hostile nesting costs a refusal and never the stack. It
//! refuses a document type declaration: an entity it declares could expand
//! without bound or name a file outside the package. It resolves the five
//! predefined entities and the character references, and nothing else.
//!
//! Each name keeps its namespace, read through the prefix the part
//! declares. ODF collapses white space in a paragraph, so the tree keeps
//! every text node under an element of the text namespace, and the reader
//! collapses it (`ODF 1.3 part 3, section 6.1.2`).

use std::collections::HashMap;
use std::rc::Rc;

use ambre_core::{Payload, PayloadKind};
use quick_xml::NsReader;
use quick_xml::events::Event;
use quick_xml::name::ResolveResult;

/// The deepest nesting of elements a part may hold, as the Word reader
/// bounds it. A table in a cell costs three levels, a list in a list item
/// two.
pub const MAX_DEPTH: usize = 256;

/// The bytes of one part of a package, inflated. `Payload<PartBytes>`.
pub enum PartBytes {}

impl PayloadKind for PartBytes {
    const NAME: &'static str = "OdfPartBytes";
}

/// Why a part does not read as XML. No variant quotes the part: its bytes
/// are the user's content.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum XmlError {
    #[error("a part is not well-formed XML")]
    Malformed,
    #[error("a part declares a document type, which the reader refuses")]
    DocumentType,
    #[error("a part nests elements deeper than {MAX_DEPTH} levels")]
    TooDeep,
}

/// The namespaces the codec reads. A name in any other namespace reads as
/// [`Ns::Other`], and no reader matches it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ns {
    /// No namespace: an attribute without a prefix.
    None,
    Office,
    Text,
    Table,
    Style,
    /// The formatting properties ODF borrows from XSL-FO, `fo:`.
    Fo,
    Draw,
    Svg,
    XLink,
    DublinCore,
    Meta,
    /// LibreOffice's extensions, `loext:`: the state and the parent of a
    /// comment.
    LoExt,
    /// The package manifest, `manifest:`.
    Manifest,
    /// The attributes XML itself defines, `xml:`.
    Xml,
    Other,
}

impl Ns {
    /// The namespace a URI names.
    fn of(uri: &[u8]) -> Self {
        match uri {
            b"urn:oasis:names:tc:opendocument:xmlns:office:1.0" => Self::Office,
            b"urn:oasis:names:tc:opendocument:xmlns:text:1.0" => Self::Text,
            b"urn:oasis:names:tc:opendocument:xmlns:table:1.0" => Self::Table,
            b"urn:oasis:names:tc:opendocument:xmlns:style:1.0" => Self::Style,
            b"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" => Self::Fo,
            b"urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" => Self::Draw,
            b"urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" => Self::Svg,
            b"http://www.w3.org/1999/xlink" => Self::XLink,
            b"http://purl.org/dc/elements/1.1/" => Self::DublinCore,
            b"urn:oasis:names:tc:opendocument:xmlns:meta:1.0" => Self::Meta,
            b"urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0" => Self::LoExt,
            b"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" => Self::Manifest,
            b"http://www.w3.org/XML/1998/namespace" => Self::Xml,
            _ => Self::Other,
        }
    }

    fn resolved(result: &ResolveResult<'_>) -> Self {
        match result {
            ResolveResult::Bound(ns) => Self::of(ns.as_ref().as_bytes()),
            ResolveResult::Unbound => Self::None,
            ResolveResult::Unknown(_) => Self::Other,
        }
    }
}

/// One element: its name, its attributes and its children.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Element {
    pub ns: Ns,
    /// The local name, without its prefix. A part repeats a few names many
    /// times, so each name is held once per part and shared.
    name: Rc<str>,
    attrs: Vec<Attr>,
    pub children: Vec<Node>,
    /// The element binds a namespace prefix of its own.
    binds: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Attr {
    ns: Ns,
    name: Rc<str>,
    value: String,
}

/// A child of an element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    Element(Element),
    Text(String),
}

impl Element {
    /// The local name, without its prefix.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether the element is `name` in `ns`.
    #[must_use]
    pub fn is(&self, ns: Ns, name: &str) -> bool {
        self.ns == ns && *self.name == *name
    }

    /// The value of the attribute `name` in `ns`.
    #[must_use]
    pub fn attr(&self, ns: Ns, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|attr| attr.ns == ns && *attr.name == *name)
            .map(|attr| attr.value.as_str())
    }

    /// The child elements, in order.
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|node| match node {
            Node::Element(element) => Some(element),
            Node::Text(_) => None,
        })
    }

    /// The first child element `name` in `ns`.
    #[must_use]
    pub fn child(&self, ns: Ns, name: &str) -> Option<&Element> {
        self.elements().find(|element| element.is(ns, name))
    }

    /// The text of every text node under the element, in order.
    #[must_use]
    pub fn text(&self) -> String {
        let mut out = String::new();
        let mut stack: Vec<&Node> = self.children.iter().rev().collect();
        while let Some(node) = stack.pop() {
            match node {
                Node::Text(text) => out.push_str(text),
                Node::Element(element) => stack.extend(element.children.iter().rev()),
            }
        }
        out
    }
}

/// Whether white space under `parent` may be content. Text under the text
/// namespace collapses in the reader; text under the metadata elements is
/// a value. White space anywhere else is layout of the XML.
fn keeps_whitespace(parent: &Element) -> bool {
    matches!(parent.ns, Ns::Text | Ns::DublinCore | Ns::Meta)
}

/// The tree of one part.
///
/// # Errors
///
/// [`XmlError::Malformed`] for bytes that are not well-formed UTF-8 XML,
/// [`XmlError::DocumentType`] for a part that declares a document type, and
/// [`XmlError::TooDeep`] past [`MAX_DEPTH`].
pub fn parse(part: &Payload<PartBytes>) -> Result<Element, XmlError> {
    let mut reader = NsReader::from_reader(part.as_bytes());
    reader.config_mut().check_end_names = true;
    let mut buf = Vec::new();
    let mut names = Names::default();
    // The open elements, the innermost last. The root's parent is a
    // sentinel that collects the root.
    let mut stack: Vec<Element> = vec![Element {
        ns: Ns::Other,
        name: Rc::from(""),
        attrs: Vec::new(),
        children: Vec::new(),
        binds: false,
    }];
    loop {
        buf.clear();
        let (resolved, event) = reader
            .read_resolved_event_into(&mut buf)
            .map_err(|_| XmlError::Malformed)?;
        let ns = Ns::resolved(&resolved);
        match event {
            Event::Start(start) => {
                if stack.len() > MAX_DEPTH {
                    return Err(XmlError::TooDeep);
                }
                stack.push(open(&reader, &mut names, ns, &start)?);
            }
            Event::Empty(start) => {
                if stack.len() > MAX_DEPTH {
                    return Err(XmlError::TooDeep);
                }
                let element = open(&reader, &mut names, ns, &start)?;
                if element.binds {
                    names.prefixes.clear();
                }
                push_child(&mut stack, Node::Element(element))?;
            }
            Event::End(_) => {
                let element = stack.pop().ok_or(XmlError::Malformed)?;
                if element.binds {
                    names.prefixes.clear();
                }
                push_child(&mut stack, Node::Element(element))?;
            }
            Event::Text(text) => push_text(&mut stack, &text.xml10_content())?,
            Event::CData(data) => push_text(&mut stack, &data.xml10_content())?,
            Event::GeneralRef(reference) => {
                let resolved = match reference.resolve_char_ref() {
                    Ok(Some(ch)) => ch,
                    Ok(None) => {
                        predefined_entity(&reference.xml10_content()).ok_or(XmlError::Malformed)?
                    }
                    Err(_) => return Err(XmlError::Malformed),
                };
                let mut utf8 = [0_u8; 4];
                push_text(&mut stack, resolved.encode_utf8(&mut utf8))?;
            }
            Event::DocType(_) => return Err(XmlError::DocumentType),
            Event::Decl(_) | Event::PI(_) | Event::Comment(_) => {}
            Event::Eof => break,
        }
    }
    let mut sentinel = stack.pop().ok_or(XmlError::Malformed)?;
    if !stack.is_empty() {
        return Err(XmlError::Malformed);
    }
    sentinel
        .children
        .drain(..)
        .find_map(|node| match node {
            Node::Element(element) => Some(element),
            Node::Text(_) => None,
        })
        .ok_or(XmlError::Malformed)
}

/// The character one of the five predefined entities names.
fn predefined_entity(name: &str) -> Option<char> {
    match name {
        "lt" => Some('<'),
        "gt" => Some('>'),
        "amp" => Some('&'),
        "apos" => Some('\''),
        "quot" => Some('"'),
        _ => None,
    }
}

/// The names met so far in one part, each held once, and the namespace
/// each attribute prefix resolved to in the scope open now.
#[derive(Default)]
struct Names {
    names: HashMap<Box<str>, Rc<str>>,
    prefixes: HashMap<Box<str>, Ns>,
}

impl Names {
    fn get(&mut self, text: &str) -> Rc<str> {
        if let Some(name) = self.names.get(text) {
            return Rc::clone(name);
        }
        let name: Rc<str> = Rc::from(text);
        self.names.insert(text.into(), Rc::clone(&name));
        name
    }

    /// The namespace of an attribute. A part nearly always binds its
    /// prefixes on its root, so a prefix resolves once. A binding anywhere
    /// else clears the cache, when the element opens and when it closes.
    fn attribute_ns(&mut self, reader: &NsReader<&[u8]>, key: quick_xml::name::QName<'_>) -> Ns {
        let Some(prefix) = key.prefix() else {
            return Ns::None;
        };
        if let Some(ns) = self.prefixes.get(prefix.as_ref()) {
            return *ns;
        }
        let ns = Ns::resolved(&reader.resolver().resolve_attribute(key).0);
        self.prefixes.insert(prefix.as_ref().into(), ns);
        ns
    }
}

fn open(
    reader: &NsReader<&[u8]>,
    names: &mut Names,
    ns: Ns,
    start: &quick_xml::events::BytesStart<'_>,
) -> Result<Element, XmlError> {
    let name = names.get(start.local_name().as_ref());
    let binds = start
        .attributes()
        .flatten()
        .any(|attr| attr.key.as_namespace_binding().is_some());
    if binds {
        names.prefixes.clear();
    }
    let mut attrs = Vec::new();
    for attr in start.attributes() {
        let attr = attr.map_err(|_| XmlError::Malformed)?;
        // A namespace declaration is no attribute of the element.
        if attr.key.as_namespace_binding().is_some() {
            continue;
        }
        let attr_ns = names.attribute_ns(reader, attr.key);
        let value = attr
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .map_err(|_| XmlError::Malformed)?
            .into_owned();
        attrs.push(Attr {
            ns: attr_ns,
            name: names.get(attr.key.local_name().as_ref()),
            value,
        });
    }
    Ok(Element {
        ns,
        name,
        attrs,
        children: Vec::new(),
        binds,
    })
}

fn push_child(stack: &mut [Element], node: Node) -> Result<(), XmlError> {
    stack
        .last_mut()
        .ok_or(XmlError::Malformed)?
        .children
        .push(node);
    Ok(())
}

fn push_text(stack: &mut [Element], text: &str) -> Result<(), XmlError> {
    let parent = stack.last_mut().ok_or(XmlError::Malformed)?;
    if text.trim().is_empty() && !keeps_whitespace(parent) {
        return Ok(());
    }
    // Text split by an entity joins the text node before it.
    if let Some(Node::Text(previous)) = parent.children.last_mut() {
        previous.push_str(text);
    } else {
        parent.children.push(Node::Text(text.to_owned()));
    }
    Ok(())
}

/// A writer of one XML part, into a string.
///
/// Every text and attribute value passes through [`escape`], which drops a
/// character XML 1.0 cannot carry, so the part stays well-formed whatever
/// the model holds.
pub struct XmlWriter {
    out: String,
}

impl XmlWriter {
    /// A part that opens on the XML declaration.
    #[must_use]
    pub fn new() -> Self {
        Self {
            out: String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"),
        }
    }

    /// A piece of a part, with no declaration: the writer joins pieces
    /// into a part.
    #[must_use]
    pub fn fragment() -> Self {
        Self { out: String::new() }
    }

    /// Append `xml`, a piece another writer wrote, as it is.
    pub fn raw(&mut self, xml: &str) {
        self.out.push_str(xml);
    }

    /// The text written so far.
    #[must_use]
    pub fn into_string(self) -> String {
        self.out
    }

    /// Open `tag` with `attrs`.
    pub fn open(&mut self, tag: &str, attrs: &[(&str, &str)]) {
        self.start(tag, attrs);
        self.out.push('>');
    }

    /// Write `tag` with `attrs` and no content.
    pub fn empty(&mut self, tag: &str, attrs: &[(&str, &str)]) {
        self.start(tag, attrs);
        self.out.push_str("/>");
    }

    /// Close `tag`.
    pub fn close(&mut self, tag: &str) {
        self.out.push_str("</");
        self.out.push_str(tag);
        self.out.push('>');
    }

    /// Write `text` as the content of the open element.
    pub fn text(&mut self, text: &str) {
        escape(&mut self.out, text);
    }

    /// Write `tag` holding `text` alone.
    pub fn leaf(&mut self, tag: &str, attrs: &[(&str, &str)], text: &str) {
        self.open(tag, attrs);
        self.text(text);
        self.close(tag);
    }

    /// The bytes of the part.
    #[must_use]
    pub fn finish(self) -> Payload<PartBytes> {
        Payload::new(self.out.into_bytes())
    }

    fn start(&mut self, tag: &str, attrs: &[(&str, &str)]) {
        self.out.push('<');
        self.out.push_str(tag);
        for (name, value) in attrs {
            self.out.push(' ');
            self.out.push_str(name);
            self.out.push_str("=\"");
            escape(&mut self.out, value);
            self.out.push('"');
        }
    }
}

impl Default for XmlWriter {
    fn default() -> Self {
        Self::new()
    }
}

/// Append `text` escaped for a text node and an attribute value alike.
/// A character XML 1.0 cannot carry is dropped. A tab, a line feed and a
/// carriage return are written as references, so an attribute value keeps
/// them through the reader's normalization.
fn escape(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            ch if is_xml_char(ch) => out.push(ch),
            _ => {}
        }
    }
}

/// Whether XML 1.0 can carry `ch`: its `Char` production.
const fn is_xml_char(ch: char) -> bool {
    matches!(ch, '\u{9}' | '\u{A}' | '\u{D}' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_bytes(bytes: &[u8]) -> Result<Element, XmlError> {
        parse(&Payload::new(bytes.to_vec()))
    }

    #[test]
    fn a_prefix_reads_as_its_namespace() {
        let part = br#"<o:a xmlns:o="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><o:p o:style-name="x"/></o:a>"#;
        let root = parse_bytes(part).expect("reads");
        assert!(root.is(Ns::Text, "a"));
        let p = root.child(Ns::Text, "p").expect("p");
        assert_eq!(p.attr(Ns::Text, "style-name"), Some("x"));
    }

    #[test]
    fn a_document_type_is_refused() {
        let part = br#"<?xml version="1.0"?><!DOCTYPE x [<!ENTITY e SYSTEM "file:///etc/passwd">]><x>&e;</x>"#;
        assert_eq!(parse_bytes(part), Err(XmlError::DocumentType));
    }

    #[test]
    fn an_undeclared_entity_is_refused() {
        assert_eq!(parse_bytes(b"<x>&e;</x>"), Err(XmlError::Malformed));
    }

    #[test]
    fn a_deep_nesting_is_refused_without_recursion() {
        let depth = 100_000;
        let part = format!("{}{}", "<a>".repeat(depth), "</a>".repeat(depth));
        assert_eq!(parse_bytes(part.as_bytes()), Err(XmlError::TooDeep));
    }

    #[test]
    fn white_space_is_kept_under_the_text_namespace_alone() {
        let part =
            br#"<x><t:p xmlns:t="urn:oasis:names:tc:opendocument:xmlns:text:1.0"> </t:p> </x>"#;
        let root = parse_bytes(part).expect("reads");
        assert_eq!(root.children.len(), 1);
        assert_eq!(root.text(), " ");
    }

    #[test]
    fn the_writer_escapes_and_drops_what_xml_cannot_carry() {
        let mut writer = XmlWriter::new();
        writer.leaf("t", &[("v", "\"<&\t")], "a\u{1}<b>&");
        let root = parse(&writer.finish()).expect("reads back");
        assert_eq!(root.text(), "a<b>&");
        assert_eq!(root.attr(Ns::None, "v"), Some("\"<&\t"));
    }
}
