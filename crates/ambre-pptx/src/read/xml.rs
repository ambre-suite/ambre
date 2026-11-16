//! A small owned tree of one XML part of a PowerPoint package, or of an
//! OpenDocument presentation package.
//!
//! The reader builds the tree from the event stream of `quick-xml`, without
//! recursion, so a hostile nesting costs a refusal and never the stack. It
//! refuses a document type declaration: an entity it declares could expand
//! without bound or name a file outside the package. It resolves the five
//! predefined entities and the character references, and nothing else.
//!
//! Each name keeps its namespace, read through the prefix the part declares.
//! The tree keeps white space under `a:t`, where a run spells it. It also
//! keeps it under an OpenDocument paragraph or span: a space between two
//! spans is content. Any other white space is layout of the XML.

use std::collections::HashMap;
use std::rc::Rc;

use ambre_core::Payload;
use quick_xml::NsReader;
use quick_xml::events::Event;
use quick_xml::name::ResolveResult;

use crate::opc::PartBytes;

/// The deepest nesting of elements a part may hold, as the Word reader
/// bounds it. A group in a group costs two levels.
pub const MAX_DEPTH: usize = 256;

/// Why a part does not read as XML. No variant quotes the part: its bytes
/// are the user's content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XmlError {
    Malformed,
    DocumentType,
    TooDeep,
}

/// The namespaces the reader follows. A name in any other namespace reads as
/// [`Ns::Other`], and no reader matches it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ns {
    /// No namespace: an attribute without a prefix.
    None,
    /// PresentationML, `p:`, under the transitional or the strict URI.
    P,
    /// DrawingML, `a:`.
    A,
    /// The relationship id an element cites, `r:`.
    R,
    /// Markup compatibility, `mc:`: the alternate content of a newer writer.
    Mc,
    /// DrawingML charts, `c:`: the chart part a graphic frame cites.
    C,
    /// The OpenDocument office namespace, `office:`.
    Office,
    /// The OpenDocument styles, `style:`.
    Style,
    /// The OpenDocument text, `text:`.
    Text,
    /// The OpenDocument drawing, `draw:`.
    Draw,
    /// The XSL-FO compatible properties, `fo:`.
    Fo,
    /// The SVG compatible attributes, `svg:`.
    Svg,
    /// The OpenDocument presentation, `presentation:`.
    Presentation,
    /// The OpenDocument table, `table:`.
    Table,
    /// The OpenDocument animation, `anim:`.
    Anim,
    /// The SMIL compatible attributes, `smil:`.
    Smil,
    /// The link a picture names its part by, `xlink:`.
    Xlink,
    /// The XML namespace itself, `xml:`: an `xml:id`.
    Xml,
    Other,
}

impl Ns {
    /// The namespace a URI names.
    fn of(uri: &str) -> Self {
        match uri {
            "http://schemas.openxmlformats.org/presentationml/2006/main"
            | "http://purl.oclc.org/ooxml/presentationml/main" => Self::P,
            "http://schemas.openxmlformats.org/drawingml/2006/main"
            | "http://purl.oclc.org/ooxml/drawingml/main" => Self::A,
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
            | "http://purl.oclc.org/ooxml/officeDocument/relationships" => Self::R,
            "http://schemas.openxmlformats.org/markup-compatibility/2006" => Self::Mc,
            "http://schemas.openxmlformats.org/drawingml/2006/chart"
            | "http://purl.oclc.org/ooxml/drawingml/chart" => Self::C,
            "urn:oasis:names:tc:opendocument:xmlns:office:1.0" => Self::Office,
            "urn:oasis:names:tc:opendocument:xmlns:style:1.0" => Self::Style,
            "urn:oasis:names:tc:opendocument:xmlns:text:1.0" => Self::Text,
            "urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" => Self::Draw,
            "urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" => Self::Fo,
            "urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" => Self::Svg,
            "urn:oasis:names:tc:opendocument:xmlns:presentation:1.0" => Self::Presentation,
            "urn:oasis:names:tc:opendocument:xmlns:table:1.0" => Self::Table,
            "urn:oasis:names:tc:opendocument:xmlns:animation:1.0" => Self::Anim,
            "urn:oasis:names:tc:opendocument:xmlns:smil-compatible:1.0" => Self::Smil,
            "http://www.w3.org/1999/xlink" => Self::Xlink,
            "http://www.w3.org/XML/1998/namespace" => Self::Xml,
            _ => Self::Other,
        }
    }

    fn resolved(result: &ResolveResult<'_>) -> Self {
        match result {
            ResolveResult::Bound(ns) => Self::of(ns.as_ref()),
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

    /// The value of the attribute `name` with no prefix.
    #[must_use]
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attr_in(Ns::None, name)
    }

    /// The value of the attribute `name` in `ns`.
    #[must_use]
    pub fn attr_in(&self, ns: Ns, name: &str) -> Option<&str> {
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

    /// The element the path of names reaches, each a child of the one
    /// before, every name in `ns`.
    #[must_use]
    pub fn path(&self, ns: Ns, names: &[&str]) -> Option<&Element> {
        names
            .iter()
            .try_fold(self, |element, name| element.child(ns, name))
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

    /// Every element under this one, at any depth, that is `name` in `ns`,
    /// in document order.
    #[must_use]
    pub fn descendants(&self, ns: Ns, name: &str) -> Vec<&Element> {
        let mut found = Vec::new();
        let mut stack: Vec<&Element> = self.elements().collect();
        stack.reverse();
        while let Some(element) = stack.pop() {
            if element.is(ns, name) {
                found.push(element);
            }
            stack.extend(element.elements().collect::<Vec<_>>().into_iter().rev());
        }
        found
    }

    /// Whether any element under this one, at any depth, is `name` in `ns`.
    #[must_use]
    pub fn holds(&self, ns: Ns, name: &str) -> bool {
        let mut stack: Vec<&Element> = self.elements().collect();
        while let Some(element) = stack.pop() {
            if element.is(ns, name) {
                return true;
            }
            stack.extend(element.elements());
        }
        false
    }
}

/// Whether white space under `parent` may be content: the text of a run,
/// or the text of an OpenDocument paragraph, heading, span or link.
fn keeps_whitespace(parent: &Element) -> bool {
    parent.is(Ns::A, "t")
        || (parent.ns == Ns::Text && matches!(parent.name(), "p" | "h" | "span" | "a"))
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
