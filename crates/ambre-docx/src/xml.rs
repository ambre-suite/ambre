//! A small owned tree of one XML part, and the writer of one.
//!
//! The reader builds the tree from quick-xml's event stream, without
//! recursion, so a hostile nesting costs a refusal and never the stack. It
//! refuses a document type declaration. An entity it declares could expand
//! without bound or name a file outside the package, and Word refuses one
//! too. It resolves the five predefined entities and the character
//! references, and nothing else.
//!
//! Each name keeps its namespace, read through the prefix the part declares,
//! so a part that binds `w:` to another prefix reads the same. The
//! transitional and the strict namespaces of ISO/IEC 29500 read as one.

use std::collections::HashMap;
use std::rc::Rc;

use ambre_core::Payload;
use quick_xml::NsReader;

use crate::opc::PartBytes;
use quick_xml::events::Event;
use quick_xml::name::ResolveResult;

/// The deepest nesting of elements a part may hold. A Word table nests a
/// table in a cell at three levels per table, so 256 holds some 80 tables
/// deep, past what Word writes.
pub const MAX_DEPTH: usize = 256;

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
    /// The tree of a part holds more nodes or attribute bytes than
    /// `ambre_opc::TreeBudget` admits.
    #[error("a part builds a tree past its bound")]
    TooLarge,
}

impl From<ambre_opc::TreeTooLarge> for XmlError {
    fn from(_: ambre_opc::TreeTooLarge) -> Self {
        Self::TooLarge
    }
}

/// The namespaces the codec reads. A name in any other namespace reads as
/// [`Ns::Other`], and no reader matches it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ns {
    /// No namespace: an attribute without a prefix.
    None,
    /// WordprocessingML, `w:`.
    W,
    /// The relationship attributes of a part, `r:`.
    R,
    /// The Word drawing frame, `wp:`.
    Wp,
    /// DrawingML, `a:`.
    A,
    /// The DrawingML picture, `pic:`.
    Pic,
    /// Markup compatibility, `mc:`.
    Mc,
    /// The Word 2010 extensions, `w14:`.
    W14,
    /// The Word 2012 extensions, `w15:`.
    W15,
    /// The Word 2023 date extensions, `w16du:`.
    W16du,
    /// The relationships part of the package.
    PackageRelationships,
    /// The content types part of the package.
    ContentTypes,
    /// The core properties, `cp:`.
    CoreProperties,
    /// Dublin Core, `dc:`.
    DublinCore,
    /// Office Math, `m:`.
    Math,
    /// VML, `v:`.
    Vml,
    Other,
}

impl Ns {
    /// The namespace a URI names. Transitional and strict read as one.
    fn of(uri: &[u8]) -> Self {
        match uri {
            b"http://schemas.openxmlformats.org/wordprocessingml/2006/main"
            | b"http://purl.oclc.org/ooxml/wordprocessingml/main" => Self::W,
            b"http://schemas.openxmlformats.org/officeDocument/2006/relationships"
            | b"http://purl.oclc.org/ooxml/officeDocument/relationships" => Self::R,
            b"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
            | b"http://purl.oclc.org/ooxml/drawingml/wordprocessingDrawing" => Self::Wp,
            b"http://schemas.openxmlformats.org/drawingml/2006/main"
            | b"http://purl.oclc.org/ooxml/drawingml/main" => Self::A,
            b"http://schemas.openxmlformats.org/drawingml/2006/picture"
            | b"http://purl.oclc.org/ooxml/drawingml/picture" => Self::Pic,
            b"http://schemas.openxmlformats.org/markup-compatibility/2006" => Self::Mc,
            b"http://schemas.microsoft.com/office/word/2010/wordml" => Self::W14,
            b"http://schemas.microsoft.com/office/word/2012/wordml" => Self::W15,
            b"http://schemas.microsoft.com/office/word/2023/wordml/word16du" => Self::W16du,
            b"http://schemas.openxmlformats.org/package/2006/relationships" => {
                Self::PackageRelationships
            }
            b"http://schemas.openxmlformats.org/package/2006/content-types" => Self::ContentTypes,
            b"http://schemas.openxmlformats.org/package/2006/metadata/core-properties" => {
                Self::CoreProperties
            }
            b"http://purl.org/dc/elements/1.1/" => Self::DublinCore,
            b"http://schemas.openxmlformats.org/officeDocument/2006/math"
            | b"http://purl.oclc.org/ooxml/officeDocument/math" => Self::Math,
            b"urn:schemas-microsoft-com:vml" => Self::Vml,
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

    /// The value of the WordprocessingML attribute `name`. A part written
    /// by an older tool may spell it without its prefix, so that form reads
    /// too.
    #[must_use]
    pub fn w_attr(&self, name: &str) -> Option<&str> {
        self.attr(Ns::W, name).or_else(|| self.attr(Ns::None, name))
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

/// Whether text under an element of this local name is content. Word keeps
/// text in a run's text element alone, and the core properties keep theirs
/// in their own elements. Whitespace between any other elements is layout
/// of the XML and is dropped.
fn keeps_whitespace(parent: &Element) -> bool {
    matches!(&*parent.name, "t" | "delText" | "instrText")
        || matches!(parent.ns, Ns::DublinCore | Ns::CoreProperties)
}

/// The tree of one part.
///
/// # Errors
///
/// * [`XmlError::Malformed`] for bytes that are not well-formed UTF-8 XML;
/// * [`XmlError::DocumentType`] for a part that declares a document type;
/// * [`XmlError::TooDeep`] past [`MAX_DEPTH`];
/// * [`XmlError::TooLarge`] for a tree past the node or attribute bound of
///   `ambre_opc::TreeBudget`.
pub fn parse(part: &Payload<PartBytes>) -> Result<Element, XmlError> {
    let mut reader = NsReader::from_reader(part.as_bytes());
    reader.config_mut().check_end_names = true;
    let mut buf = Vec::new();
    // The open elements, the innermost last. The root's parent is a
    // sentinel that collects the root.
    let mut names = Names::default();
    let mut budget = ambre_opc::TreeBudget::default();
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
                stack.push(open(&reader, &mut names, &mut budget, ns, &start)?);
            }
            Event::Empty(start) => {
                if stack.len() > MAX_DEPTH {
                    return Err(XmlError::TooDeep);
                }
                let element = open(&reader, &mut names, &mut budget, ns, &start)?;
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
            Event::Text(text) => {
                let text = text.xml10_content();
                push_text(&mut stack, &mut budget, &normalize_eols(&text))?;
            }
            Event::CData(data) => {
                let text = data.xml10_content();
                push_text(&mut stack, &mut budget, &normalize_eols(&text))?;
            }
            Event::GeneralRef(reference) => {
                let resolved = match reference.resolve_char_ref() {
                    Ok(Some(ch)) => ch,
                    Ok(None) => {
                        let name = reference.xml10_content();
                        predefined_entity(&name).ok_or(XmlError::Malformed)?
                    }
                    Err(_) => return Err(XmlError::Malformed),
                };
                let mut utf8 = [0_u8; 4];
                push_text(&mut stack, &mut budget, resolved.encode_utf8(&mut utf8))?;
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

/// The text with each line end read as one line feed, as XML 1.0 reads it.
/// Text with no carriage return, nearly all of it, is borrowed as is.
fn normalize_eols(text: &str) -> std::borrow::Cow<'_, str> {
    if text.contains('\r') {
        std::borrow::Cow::Owned(text.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        std::borrow::Cow::Borrowed(text)
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

/// Whether an attribute is a revision session id, `w:rsid*`: Word writes
/// one on most paragraphs and runs, and no reader reads it.
fn is_session_id(ns: Ns, local: &str) -> bool {
    ns == Ns::W && local.starts_with("rsid")
}

/// The element `start` opens, charged to `budget` with each attribute it
/// keeps before the tree holds it.
fn open(
    reader: &NsReader<&[u8]>,
    names: &mut Names,
    budget: &mut ambre_opc::TreeBudget,
    ns: Ns,
    start: &quick_xml::events::BytesStart<'_>,
) -> Result<Element, XmlError> {
    budget.node()?;
    let name = names.get(start.local_name().as_ref());
    let mut attrs = Vec::new();
    let binds = start
        .attributes()
        .flatten()
        .any(|attr| attr.key.as_namespace_binding().is_some());
    if binds {
        names.prefixes.clear();
    }
    for attr in start.attributes() {
        let attr = attr.map_err(|_| XmlError::Malformed)?;
        // A namespace declaration is no attribute of the element.
        if attr.key.as_namespace_binding().is_some() {
            continue;
        }
        let attr_ns = names.attribute_ns(reader, attr.key);
        let local = attr.key.local_name();
        if is_session_id(attr_ns, local.as_ref()) {
            continue;
        }
        let value = attr
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .map_err(|_| XmlError::Malformed)?
            .into_owned();
        budget.attr(ambre_core::Count::of(value.len()))?;
        attrs.push(Attr {
            ns: attr_ns,
            name: names.get(local.as_ref()),
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

fn push_text(
    stack: &mut [Element],
    budget: &mut ambre_opc::TreeBudget,
    text: &str,
) -> Result<(), XmlError> {
    let parent = stack.last_mut().ok_or(XmlError::Malformed)?;
    if text.trim().is_empty() && !keeps_whitespace(parent) {
        return Ok(());
    }
    // Text split by an entity joins the text node before it.
    if let Some(Node::Text(previous)) = parent.children.last_mut() {
        previous.push_str(text);
    } else {
        budget.node()?;
        parent.children.push(Node::Text(text.to_owned()));
    }
    Ok(())
}

/// A writer of one XML part, into a string.
///
/// Every text and attribute value passes through [`escape`], which drops a
/// character XML 1.0 cannot carry, so the part stays well-formed whatever
/// the model holds.
#[derive(Default)]
pub struct XmlWriter {
    out: String,
}

impl XmlWriter {
    /// A part that opens on the XML declaration Word writes.
    #[must_use]
    pub fn new() -> Self {
        Self {
            out: String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n"),
        }
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
#[must_use]
pub const fn is_xml_char(ch: char) -> bool {
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
        let part = br#"<x:document xmlns:x="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><x:body x:val="1"/></x:document>"#;
        let root = parse_bytes(part).expect("reads");
        assert!(root.is(Ns::W, "document"));
        let body = root.child(Ns::W, "body").expect("body");
        assert_eq!(body.w_attr("val"), Some("1"));
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

    /// `W72`: a part of empty elements under the part bound, past the node
    /// bound, is refused before its tree holds them all.
    #[test]
    fn a_part_past_the_node_bound_is_refused() {
        let past = usize::try_from(ambre_opc::MAX_TREE_NODES.get()).expect("fits");
        let part = format!("<r>{}</r>", "<a/>".repeat(past));
        assert_eq!(parse_bytes(part.as_bytes()), Err(XmlError::TooLarge));
    }

    /// `W72`: short attributes meet the attribute bound by their records.
    #[test]
    fn a_part_past_the_attribute_bound_is_refused() {
        let records = ambre_opc::MAX_ATTR_BYTES.get() / ambre_opc::tree::ATTR_RECORD.get();
        let elements = usize::try_from(records / 8).expect("fits") + 1;
        let part = format!(
            "<r>{}</r>",
            "<a b='' c='' d='' e='' f='' g='' h='' i=''/>".repeat(elements)
        );
        assert_eq!(parse_bytes(part.as_bytes()), Err(XmlError::TooLarge));
    }

    #[test]
    fn entities_and_references_join_the_text() {
        let root = parse_bytes(b"<t>a &amp; b &#x263A;&lt;</t>").expect("reads");
        assert_eq!(root.text(), "a & b \u{263A}<");
    }

    #[test]
    fn whitespace_is_content_inside_a_run_text_alone() {
        let root = parse_bytes(b"<p> <t> </t> </p>").expect("reads");
        assert_eq!(root.children.len(), 1);
        assert_eq!(root.text(), " ");
    }

    #[test]
    fn the_writer_escapes_and_drops_what_xml_cannot_carry() {
        let mut writer = XmlWriter::new();
        writer.leaf("t", &[("v", "\"<&\t")], "a\u{1}<b>&");
        let bytes = writer.finish();
        let root = parse(&bytes).expect("reads back");
        assert_eq!(root.text(), "a<b>&");
        assert_eq!(root.attr(Ns::None, "v"), Some("\"<&\t"));
    }
}
