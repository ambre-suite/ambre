//! The XML reader the tests open each part back with: a tree of elements
//! over `quick-xml`, the parser the codec already links.
//!
//! `cargo deny` keeps every other XML parser in the images module
//! (`deny.toml`, the `roxmltree` ban). So the tests read through `quick-xml`,
//! as the DOCX and xlsx codecs do. The reader refuses each defect a strict
//! parser refuses in a part the writer can produce:
//!
//! * an end tag that closes another element;
//! * an unbound prefix, and a repeated attribute;
//! * an unknown entity, and a character XML 1.0 excludes;
//! * a document with no root, or with two.

use quick_xml::NsReader;
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::{BytesRef, BytesStart, Event};
use quick_xml::name::ResolveResult;

/// One part, read whole into an arena of elements.
pub struct Document {
    elements: Vec<Element>,
}

struct Element {
    ns: Option<String>,
    local: String,
    attributes: Vec<Attribute>,
    children: Vec<usize>,
    /// The text before the first child element, if the element starts with
    /// text.
    text: Option<String>,
}

struct Attribute {
    ns: Option<String>,
    local: String,
    value: String,
}

/// One element of a [`Document`].
#[derive(Clone, Copy)]
pub struct Node<'d> {
    doc: &'d Document,
    index: usize,
}

/// An attribute name: a local name alone, or a namespace and a local name.
pub struct AttributeName<'n> {
    ns: Option<&'n str>,
    local: &'n str,
}

impl<'n> From<&'n str> for AttributeName<'n> {
    fn from(local: &'n str) -> Self {
        Self { ns: None, local }
    }
}

impl<'n> From<(&'n str, &'n str)> for AttributeName<'n> {
    fn from((ns, local): (&'n str, &'n str)) -> Self {
        Self {
            ns: Some(ns),
            local,
        }
    }
}

impl Document {
    /// The tree of `text`, or the reason it is no well-formed part.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut reader = NsReader::from_str(text);
        let mut elements: Vec<Element> = Vec::new();
        let mut open: Vec<usize> = Vec::new();
        let mut roots = 0_usize;
        loop {
            let (ns, event) = reader.read_resolved_event().map_err(|e| e.to_string())?;
            let ns = namespace(&ns)?;
            match event {
                Event::Start(start) => {
                    let index = add(
                        &mut elements,
                        &open,
                        &mut roots,
                        element(&reader, ns, &start)?,
                    );
                    open.push(index);
                }
                Event::Empty(start) => {
                    add(
                        &mut elements,
                        &open,
                        &mut roots,
                        element(&reader, ns, &start)?,
                    );
                }
                Event::End(_) => {
                    open.pop().ok_or("an end tag closes nothing")?;
                }
                Event::Text(t) => push_text(&mut elements, &open, &t.xml10_content())?,
                Event::CData(t) => push_text(&mut elements, &open, &t.xml10_content())?,
                Event::GeneralRef(r) => push_text(&mut elements, &open, &reference(&r)?)?,
                Event::DocType(_) => return Err("a part declares a document type".to_owned()),
                Event::Decl(_) | Event::PI(_) | Event::Comment(_) => {}
                Event::Eof => break,
            }
        }
        match (roots, open.is_empty()) {
            (1, true) => Ok(Self { elements }),
            (_, false) => Err("an element is never closed".to_owned()),
            _ => Err(format!("the part holds {roots} root elements")),
        }
    }

    /// The root element.
    pub fn root_element(&self) -> Node<'_> {
        Node {
            doc: self,
            index: 0,
        }
    }

    /// Every element, in document order.
    pub fn descendants(&self) -> impl Iterator<Item = Node<'_>> {
        (0..self.elements.len()).map(|index| Node { doc: self, index })
    }
}

impl<'d> Node<'d> {
    fn data(&self) -> &'d Element {
        &self.doc.elements[self.index]
    }

    /// Whether the element is `local` in the namespace `ns`.
    pub fn has_tag_name(&self, (ns, local): (&str, &str)) -> bool {
        let data = self.data();
        data.ns.as_deref() == Some(ns) && data.local == local
    }

    /// The local name of the element.
    pub fn name(&self) -> &'d str {
        &self.data().local
    }

    /// The value of the attribute `name`, if the element carries it.
    pub fn attribute<'n>(&self, name: impl Into<AttributeName<'n>>) -> Option<&'d str> {
        let name = name.into();
        self.data()
            .attributes
            .iter()
            .find(|a| a.ns.as_deref() == name.ns && a.local == name.local)
            .map(|a| a.value.as_str())
    }

    /// The text the element starts with, if any.
    pub fn text(&self) -> Option<&'d str> {
        self.data().text.as_deref()
    }

    /// The child elements, in order.
    pub fn children(&self) -> impl Iterator<Item = Node<'d>> {
        let doc = self.doc;
        self.data()
            .children
            .iter()
            .map(move |&index| Node { doc, index })
    }

    /// The element and every element inside it, in document order. The
    /// arena holds them in document order, so the subtree is a run of it.
    pub fn descendants(&self) -> impl Iterator<Item = Node<'d>> {
        let doc = self.doc;
        let end = self.subtree_end();
        (self.index..end).map(move |index| Node { doc, index })
    }

    fn subtree_end(&self) -> usize {
        self.children()
            .last()
            .map_or(self.index + 1, |last| last.subtree_end())
    }
}

/// Push `element` under the innermost open element, or as a root, and
/// answer its index.
fn add(elements: &mut Vec<Element>, open: &[usize], roots: &mut usize, element: Element) -> usize {
    let index = elements.len();
    elements.push(element);
    match open.last() {
        Some(&parent) => elements[parent].children.push(index),
        None => *roots += 1,
    }
    index
}

fn namespace(ns: &ResolveResult<'_>) -> Result<Option<String>, String> {
    match ns {
        ResolveResult::Unbound => Ok(None),
        ResolveResult::Bound(ns) => Ok(Some(ns.as_ref().to_owned())),
        ResolveResult::Unknown(prefix) => Err(format!("the prefix {prefix} is unbound")),
    }
}

fn element(
    reader: &NsReader<&[u8]>,
    ns: Option<String>,
    start: &BytesStart<'_>,
) -> Result<Element, String> {
    let mut attributes: Vec<Attribute> = Vec::new();
    for attribute in start.attributes() {
        let attribute = attribute.map_err(|e| e.to_string())?;
        let key = attribute.key;
        // A namespace declaration binds a prefix, and names no attribute.
        if key.as_namespace_binding().is_some() {
            continue;
        }
        let (resolved, local) = reader.resolver().resolve_attribute(key);
        let value = attribute
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .map_err(|e| e.to_string())?;
        check_chars(&value)?;
        attributes.push(Attribute {
            ns: namespace(&resolved)?,
            local: local.as_ref().to_owned(),
            value: value.into_owned(),
        });
    }
    let local = start.local_name().as_ref().to_owned();
    Ok(Element {
        ns,
        local,
        attributes,
        children: Vec::new(),
        text: None,
    })
}

fn reference(r: &BytesRef<'_>) -> Result<String, String> {
    if let Some(c) = r.resolve_char_ref().map_err(|e| e.to_string())? {
        return Ok(c.to_string());
    }
    let name = r.xml10_content();
    resolve_predefined_entity(&name)
        .map(str::to_owned)
        .ok_or_else(|| format!("the entity {name} is unknown"))
}

/// Text inside the innermost open element. Text before the first child is
/// the element's own text. Text after a child is mixed content, which no
/// part of the writer holds, so the reader checks it and keeps none.
fn push_text(elements: &mut [Element], open: &[usize], text: &str) -> Result<(), String> {
    check_chars(text)?;
    let Some(&current) = open.last() else {
        return if text.trim().is_empty() {
            Ok(())
        } else {
            Err("text lies outside the root element".to_owned())
        };
    };
    let element = &mut elements[current];
    if element.children.is_empty() {
        element.text.get_or_insert_with(String::new).push_str(text);
    }
    Ok(())
}

/// Refuse a character outside the `Char` production of XML 1.0.
fn check_chars(text: &str) -> Result<(), String> {
    match text.chars().find(|&c| {
        !matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}')
            && c < '\u{10000}'
    }) {
        Some(c) => Err(format!("the character {:#x} is no XML", u32::from(c))),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::Document;

    /// A reader that accepts every part proves nothing: each defect a strict
    /// parser refuses is refused here too.
    #[test]
    fn a_malformed_part_is_refused() {
        for bad in [
            "<a><b></a></b>",
            "<x:a/>",
            "<a k=\"1\" k=\"2\"/>",
            "<a>&nope;</a>",
            "<a>\u{1}</a>",
            "<a/><b/>",
            "<a>",
            "",
        ] {
            assert!(Document::parse(bad).is_err(), "{bad:?} parsed");
        }
    }

    #[test]
    fn a_part_reads_back_its_names_attributes_and_text() {
        let doc = Document::parse(
            "<p:r xmlns:p=\"urn:p\" xmlns:q=\"urn:q\" q:id=\"7\" plain=\"&amp;\">\
             <p:t>a&lt;b</p:t><p:e/><p:t>c</p:t></p:r>",
        )
        .expect("parses");
        let root = doc.root_element();
        assert!(root.has_tag_name(("urn:p", "r")));
        assert_eq!(root.attribute(("urn:q", "id")), Some("7"));
        assert_eq!(root.attribute("plain"), Some("&"));
        assert_eq!(root.attribute("id"), None);
        let names: Vec<&str> = root.children().map(|n| n.name()).collect();
        assert_eq!(names, ["t", "e", "t"]);
        let texts: Vec<Option<&str>> = doc
            .descendants()
            .filter(|n| n.has_tag_name(("urn:p", "t")))
            .map(|n| n.text())
            .collect();
        assert_eq!(texts, [Some("a<b"), Some("c")]);
        assert_eq!(root.descendants().count(), 4);
    }
}
