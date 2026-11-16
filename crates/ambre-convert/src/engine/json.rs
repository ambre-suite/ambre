//! A small JSON reader for the board reader of the boards module.
//!
//! The module holds a ceiling of raw bytes (`ambre-spec-budgets.md section
//! 5`). A general JSON crate costs it about 24 KB, for a reader that reads a
//! tree and never writes one. This reader parses RFC 8259 JSON into a tree
//! that borrows each string without an escape. It refuses a value nested past
//! [`MAX_NESTING`], so a hostile file never grows the module's stack.

use std::borrow::Cow;

use ambre_client_core::FileBytes;
use ambre_core::Payload;

/// The deepest array or object the reader enters, the bound `serde_json`
/// holds by default.
const MAX_NESTING: usize = 128;

/// One JSON value. A number is kept as the `f64` RFC 8259 reads it as.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Json<'a> {
    Null,
    Bool(bool),
    Number(f64),
    Text(Cow<'a, str>),
    Array(Vec<Json<'a>>),
    Object(Vec<(Cow<'a, str>, Json<'a>)>),
}

impl<'a> Json<'a> {
    /// The value of `key` in an object; `None` for a missing key or a value
    /// that is no object. The first of two equal keys wins.
    pub(super) fn get(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub(super) fn as_str(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            _ => None,
        }
    }

    pub(super) fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub(super) fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub(super) fn as_array(&self) -> Option<&Vec<Self>> {
        match self {
            Self::Array(items) => Some(items),
            _ => None,
        }
    }

    pub(super) fn is_array(&self) -> bool {
        matches!(self, Self::Array(_))
    }
}

/// Why a text is no JSON the reader takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum JsonRefusal {
    /// The bytes break the grammar, or are not UTF-8.
    Malformed,
    /// A value nests past [`MAX_NESTING`].
    TooDeep,
}

/// The value the file `data` holds, whole: trailing text other than white space is
/// refused.
///
/// # Errors
///
/// [`JsonRefusal`] for text that is no JSON, or nests too deep.
pub(super) fn parse(data: &Payload<FileBytes>) -> Result<Json<'_>, JsonRefusal> {
    let text = std::str::from_utf8(data.as_bytes()).map_err(|_| JsonRefusal::Malformed)?;
    let mut reader = Reader { text, at: 0 };
    let value = reader.value(0)?;
    reader.space();
    if reader.at == text.len() {
        Ok(value)
    } else {
        Err(JsonRefusal::Malformed)
    }
}

struct Reader<'a> {
    text: &'a str,
    /// The byte offset of the next character to read.
    at: usize,
}

impl<'a> Reader<'a> {
    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.at).copied()
    }

    fn space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    /// Step over `word` when it comes next.
    fn eat(&mut self, word: &str) -> Result<(), JsonRefusal> {
        let rest = self.text.get(self.at..).unwrap_or_default();
        if rest.starts_with(word) {
            self.at += word.len();
            Ok(())
        } else {
            Err(JsonRefusal::Malformed)
        }
    }

    /// The value that starts after any white space. `nesting` counts the
    /// arrays and objects around it.
    fn value(&mut self, nesting: usize) -> Result<Json<'a>, JsonRefusal> {
        self.space();
        match self.peek() {
            Some(b'n') => self.eat("null").map(|()| Json::Null),
            Some(b't') => self.eat("true").map(|()| Json::Bool(true)),
            Some(b'f') => self.eat("false").map(|()| Json::Bool(false)),
            Some(b'"') => self.string().map(Json::Text),
            Some(b'[') => self.array(nesting + 1),
            Some(b'{') => self.object(nesting + 1),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(JsonRefusal::Malformed),
        }
    }

    fn array(&mut self, nesting: usize) -> Result<Json<'a>, JsonRefusal> {
        if nesting > MAX_NESTING {
            return Err(JsonRefusal::TooDeep);
        }
        self.at += 1;
        let mut items = Vec::new();
        self.space();
        if self.peek() == Some(b']') {
            self.at += 1;
            return Ok(Json::Array(items));
        }
        loop {
            items.push(self.value(nesting)?);
            self.space();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b']') => {
                    self.at += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(JsonRefusal::Malformed),
            }
        }
    }

    fn object(&mut self, nesting: usize) -> Result<Json<'a>, JsonRefusal> {
        if nesting > MAX_NESTING {
            return Err(JsonRefusal::TooDeep);
        }
        self.at += 1;
        let mut fields = Vec::new();
        self.space();
        if self.peek() == Some(b'}') {
            self.at += 1;
            return Ok(Json::Object(fields));
        }
        loop {
            self.space();
            if self.peek() != Some(b'"') {
                return Err(JsonRefusal::Malformed);
            }
            let key = self.string()?;
            self.space();
            self.eat(":")?;
            fields.push((key, self.value(nesting)?));
            self.space();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b'}') => {
                    self.at += 1;
                    return Ok(Json::Object(fields));
                }
                _ => return Err(JsonRefusal::Malformed),
            }
        }
    }

    /// A number, read by the standard parser once its characters are taken.
    /// The grammar of RFC 8259 is a subset of what `f64::from_str` reads,
    /// save a leading `+`, `inf` and `nan`, which the scan never takes.
    fn number(&mut self) -> Result<Json<'a>, JsonRefusal> {
        let start = self.at;
        while matches!(
            self.peek(),
            Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
        ) {
            self.at += 1;
        }
        self.text
            .get(start..self.at)
            .filter(|n| !n.starts_with('+'))
            .and_then(|n| n.parse().ok())
            .map(Json::Number)
            .ok_or(JsonRefusal::Malformed)
    }

    /// A string, borrowed when it holds no escape.
    fn string(&mut self) -> Result<Cow<'a, str>, JsonRefusal> {
        self.at += 1;
        let start = self.at;
        let mut owned: Option<String> = None;
        loop {
            let Some(c) = self.text.get(self.at..).and_then(|r| r.chars().next()) else {
                return Err(JsonRefusal::Malformed);
            };
            match c {
                '"' => {
                    let end = self.at;
                    self.at += 1;
                    return Ok(match owned {
                        Some(text) => Cow::Owned(text),
                        None => Cow::Borrowed(self.text.get(start..end).unwrap_or_default()),
                    });
                }
                '\\' => {
                    let text = owned.get_or_insert_with(|| {
                        self.text.get(start..self.at).unwrap_or_default().to_owned()
                    });
                    self.at += 1;
                    let escaped = match self.peek() {
                        Some(b'"') => '"',
                        Some(b'\\') => '\\',
                        Some(b'/') => '/',
                        Some(b'b') => '\u{8}',
                        Some(b'f') => '\u{c}',
                        Some(b'n') => '\n',
                        Some(b'r') => '\r',
                        Some(b't') => '\t',
                        Some(b'u') => {
                            self.at += 1;
                            let ch = self.unicode()?;
                            text.push(ch);
                            continue;
                        }
                        _ => return Err(JsonRefusal::Malformed),
                    };
                    text.push(escaped);
                    self.at += 1;
                }
                c if u32::from(c) < 0x20 => return Err(JsonRefusal::Malformed),
                c => {
                    if let Some(text) = owned.as_mut() {
                        text.push(c);
                    }
                    self.at += c.len_utf8();
                }
            }
        }
    }

    /// The character of a `\u` escape, its four digits next, a surrogate
    /// pair joined. A lone surrogate reads as the replacement character.
    fn unicode(&mut self) -> Result<char, JsonRefusal> {
        let high = self.hex4()?;
        if !(0xD800..0xDC00).contains(&high) {
            return Ok(char::from_u32(high).unwrap_or(char::REPLACEMENT_CHARACTER));
        }
        if self
            .text
            .get(self.at..)
            .is_some_and(|r| r.starts_with("\\u"))
        {
            self.at += 2;
            let low = self.hex4()?;
            if (0xDC00..0xE000).contains(&low) {
                let joined = 0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00);
                return Ok(char::from_u32(joined).unwrap_or(char::REPLACEMENT_CHARACTER));
            }
        }
        Ok(char::REPLACEMENT_CHARACTER)
    }

    fn hex4(&mut self) -> Result<u32, JsonRefusal> {
        let digits = self
            .text
            .get(self.at..self.at + 4)
            .ok_or(JsonRefusal::Malformed)?;
        let value = u32::from_str_radix(digits, 16).map_err(|_| JsonRefusal::Malformed)?;
        self.at += 4;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(bytes: &[u8]) -> Result<Json<'static>, JsonRefusal> {
        let data: &'static Payload<FileBytes> = Box::leak(Box::new(Payload::new(bytes.to_vec())));
        super::parse(data)
    }

    #[test]
    fn a_document_reads_into_its_tree() {
        let value = parse(br#" {"a": [1, -2.5e1, true, null], "b": "x\"\u00e9\ud83d\ude00"} "#)
            .expect("json");
        assert_eq!(
            value.get("a").and_then(Json::as_array).map(Vec::len),
            Some(4)
        );
        assert_eq!(
            value
                .get("a")
                .and_then(Json::as_array)
                .and_then(|a| a.get(1))
                .and_then(Json::as_f64),
            Some(-25.0)
        );
        assert_eq!(
            value.get("b").and_then(Json::as_str),
            Some("x\"\u{e9}\u{1F600}")
        );
    }

    #[test]
    fn text_off_the_grammar_is_refused() {
        for bad in [
            &b"{"[..],
            b"[1,]",
            b"{\"a\" 1}",
            b"+1",
            b"\"\x01\"",
            b"nul",
            b"1 2",
            b"\xff",
        ] {
            assert_eq!(parse(bad), Err(JsonRefusal::Malformed), "{bad:?}");
        }
    }

    #[test]
    fn a_value_nested_past_the_bound_is_refused() {
        let deep = "[".repeat(MAX_NESTING + 1) + &"]".repeat(MAX_NESTING + 1);
        assert_eq!(parse(deep.as_bytes()), Err(JsonRefusal::TooDeep));
        let fits = "[".repeat(MAX_NESTING) + &"]".repeat(MAX_NESTING);
        assert!(parse(fits.as_bytes()).is_ok());
    }
}
