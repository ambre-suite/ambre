//! The XML of the two parts the package layer owns: the content types part
//! and a relationships part. Both are a root holding a flat list of empty
//! elements, so the reader answers that list and builds no tree.
//!
//! The reader refuses a document type declaration. An entity it declares
//! could expand without bound or name a file outside the package. Every
//! value is unescaped through the five predefined entities and the
//! character references alone.

use ambre_core::Payload;
use quick_xml::Reader;
use quick_xml::events::Event;

use crate::PartBytes;
use crate::tree::TreeBudget;

/// Why a part does not read. The package layer names the part in its own
/// error, so this carries nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XmlError;

/// One element of a part: its local name and its attributes, prefixes
/// dropped. A namespace declaration is no attribute.
pub struct Entry {
    pub name: String,
    attrs: Vec<(String, String)>,
}

impl Entry {
    /// The value of the attribute `name`, if the element carries one.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// Every element of `bytes` below the root, in document order. The reader
/// keeps no stack, so a nesting costs nothing.
///
/// # Errors
///
/// [`XmlError`] for bytes that are not well-formed UTF-8 XML, for a
/// document type declaration, and for a list past the bound of a tree.
pub fn elements(bytes: &[u8]) -> Result<Vec<Entry>, XmlError> {
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().check_end_names = true;
    let mut buf = Vec::new();
    let mut out = Vec::new();
    let mut budget = TreeBudget::default();
    let mut depth = 0_usize;
    loop {
        buf.clear();
        match reader.read_event_into(&mut buf).map_err(|_| XmlError)? {
            Event::Start(start) => {
                if depth > 0 {
                    out.push(entry(&start, &mut budget)?);
                }
                depth += 1;
            }
            Event::Empty(start) if depth > 0 => out.push(entry(&start, &mut budget)?),
            Event::End(_) => depth = depth.checked_sub(1).ok_or(XmlError)?,
            Event::DocType(_) => return Err(XmlError),
            Event::Eof => break,
            _ => {}
        }
    }
    if depth == 0 { Ok(out) } else { Err(XmlError) }
}

/// One entry, charged to `budget` before it is built: a flat list of
/// empty elements costs a record each, as a tree does.
fn entry(
    start: &quick_xml::events::BytesStart<'_>,
    budget: &mut TreeBudget,
) -> Result<Entry, XmlError> {
    budget.node().map_err(|_| XmlError)?;
    let name = start.local_name().as_ref().to_owned();
    let mut attrs = Vec::new();
    for attr in start.attributes() {
        let attr = attr.map_err(|_| XmlError)?;
        if attr.key.as_namespace_binding().is_some() {
            continue;
        }
        let key = attr.key.local_name().as_ref().to_owned();
        let value = attr
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .map_err(|_| XmlError)?
            .into_owned();
        budget
            .attr(ambre_core::Count::of(value.len()))
            .map_err(|_| XmlError)?;
        attrs.push((key, value));
    }
    Ok(Entry { name, attrs })
}

/// A writer of one of the two parts, into a string. Every attribute value
/// passes through [`escape`], so the part stays well-formed whatever text a
/// codec hands in.
pub struct XmlWriter {
    out: String,
}

impl XmlWriter {
    /// A part that opens on the XML declaration Office writes.
    pub fn new() -> Self {
        Self {
            out: String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n"),
        }
    }

    pub fn open(&mut self, tag: &str, attrs: &[(&str, &str)]) {
        self.start(tag, attrs);
        self.out.push('>');
    }

    pub fn empty(&mut self, tag: &str, attrs: &[(&str, &str)]) {
        self.start(tag, attrs);
        self.out.push_str("/>");
    }

    pub fn close(&mut self, tag: &str) {
        self.out.push_str("</");
        self.out.push_str(tag);
        self.out.push('>');
    }

    /// The bytes of the part.
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

/// Append `text` escaped for an attribute value. A character XML 1.0 cannot
/// carry is dropped.
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
            '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'.. => out.push(ch),
            _ => {}
        }
    }
}
