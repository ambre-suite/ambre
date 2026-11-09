//! A small XML writer over `quick-xml`, shaped for the parts of a package.
//!
//! Every text and every attribute value goes through the escaping of
//! `quick-xml`, so a body the user typed never breaks the markup it lands in.
//! A part is written start to end in one pass. The writer keeps the first
//! failure and answers it at the end, so the code that writes a part reads as
//! the tree it writes.

use std::borrow::Cow;

use quick_xml::Writer;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};

/// The writer of one XML part.
pub(crate) struct Xml {
    writer: Writer<Vec<u8>>,
    failure: Option<std::io::Error>,
}

impl Xml {
    /// A part that opens on the standalone UTF-8 declaration every OOXML part
    /// carries.
    pub(crate) fn part() -> Self {
        let mut xml = Self {
            writer: Writer::new(Vec::new()),
            failure: None,
        };
        xml.emit(Event::Decl(BytesDecl::new(
            "1.0",
            Some("UTF-8"),
            Some("yes"),
        )));
        xml.emit(Event::Text(BytesText::from_escaped("\n")));
        xml
    }

    /// Open the element `name` with `attributes`, in their order.
    pub(crate) fn open(&mut self, name: &str, attributes: &[(&str, &str)]) {
        let start = element(name, attributes);
        self.emit(Event::Start(start));
    }

    /// Close the element `name`.
    pub(crate) fn close(&mut self, name: &str) {
        self.emit(Event::End(BytesEnd::new(name)));
    }

    /// Write the element `name` with `attributes` and no content.
    pub(crate) fn leaf(&mut self, name: &str, attributes: &[(&str, &str)]) {
        let start = element(name, attributes);
        self.emit(Event::Empty(start));
    }

    /// Write `text`, escaped, without the characters XML cannot hold.
    pub(crate) fn text(&mut self, text: &str) {
        self.emit(Event::Text(BytesText::new(&xml_chars(text))));
    }

    /// The bytes of the part.
    ///
    /// # Errors
    ///
    /// The first failure of the writer, on any event.
    pub(crate) fn finish(self) -> Result<Vec<u8>, std::io::Error> {
        match self.failure {
            Some(e) => Err(e),
            None => Ok(self.writer.into_inner()),
        }
    }

    fn emit(&mut self, event: Event<'_>) {
        if self.failure.is_some() {
            return;
        }
        if let Err(e) = self.writer.write_event(event) {
            self.failure = Some(e);
        }
    }
}

/// The start of the element `name`, each attribute value without the
/// characters XML cannot hold.
fn element<'a>(name: &'a str, attributes: &[(&str, &str)]) -> BytesStart<'a> {
    let mut start = BytesStart::new(name);
    for (key, value) in attributes {
        start.push_attribute((*key, xml_chars(value).as_ref()));
    }
    start
}

/// `text` without the characters XML 1.0 forbids. Those are the control
/// characters but the tab and the line feed, and the two non-characters. The
/// carriage return goes too, since a parser turns it into a line feed. A text
/// the user typed may hold any of them, and one makes the whole part unreadable.
pub(crate) fn xml_chars(text: &str) -> Cow<'_, str> {
    let allowed = |c: char| {
        matches!(c, '\t' | '\n' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}')
            || c >= '\u{10000}'
    };
    if text.chars().all(allowed) {
        Cow::Borrowed(text)
    } else {
        Cow::Owned(text.chars().filter(|c| allowed(*c)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_character_xml_cannot_hold_is_left_out() {
        assert_eq!(xml_chars("a\u{1}b\rc\u{FFFE}d\te"), "abcd\te");
        assert!(matches!(xml_chars("plain"), Cow::Borrowed("plain")));
    }

    #[test]
    fn a_text_with_markup_characters_writes_a_part_that_parses() {
        let mut x = Xml::part();
        x.open("a", &[("name", "\"<&>\u{2}")]);
        x.text("<b> & \u{7}");
        x.close("a");
        let bytes = x.finish().expect("writes");
        let text = String::from_utf8(bytes).expect("UTF-8");
        let doc = roxmltree::Document::parse(&text).expect("parses");
        assert_eq!(doc.root_element().attribute("name"), Some("\"<&>"));
        assert_eq!(doc.root_element().text(), Some("<b> & "));
    }
}
