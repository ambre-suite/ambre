//! A language tag, as a document names the language its content is in.

use crate::{Rule, Validated};

/// The rule of a well-formed BCP 47 language tag: `en`, `fr-CA`,
/// `zh-Hant-TW`. A `Validated<LangTag>` holds one.
///
/// The check is RFC 5646's shape, not its registry. Subtags hold one to eight
/// ASCII letters and digits and are joined by `-`. The first holds two to
/// eight letters, or is the `x` or `i` prefix. A tag that passes may still
/// name no language anybody speaks; a text that fails is no tag at all. The
/// rule keeps the tag as written.
pub enum LangTag {}

/// A text that is no well-formed language tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("a language tag has the shape of RFC 5646")]
pub struct NotALangTag;

impl Rule for LangTag {
    type Error = NotALangTag;

    const LOGGED_AS: Option<&'static str> = Some("LangTag");

    fn admit(text: String) -> Result<String, NotALangTag> {
        if is_lang_tag(&text) {
            Ok(text)
        } else {
            Err(NotALangTag)
        }
    }
}

/// Whether `text` has the shape of RFC 5646.
fn is_lang_tag(text: &str) -> bool {
    let mut subtags = text.split('-');
    let Some(primary) = subtags.next() else {
        return false;
    };
    let prefix = primary.eq_ignore_ascii_case("x") || primary.eq_ignore_ascii_case("i");
    let language =
        (2..=8).contains(&primary.len()) && primary.bytes().all(|byte| byte.is_ascii_alphabetic());
    if !(language || prefix) {
        return false;
    }
    let mut rest = 0_usize;
    for subtag in subtags {
        if !(1..=8).contains(&subtag.len())
            || !subtag.bytes().all(|byte| byte.is_ascii_alphanumeric())
        {
            return false;
        }
        rest += 1;
    }
    // A bare `x` or `i` is a prefix with nothing after it.
    !(prefix && rest == 0)
}

/// The primary language subtag of `tag`, lowercased: `fr` for `fr-CA`.
#[must_use]
pub fn primary_subtag(tag: &Validated<LangTag>) -> String {
    tag.as_str()
        .split('-')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// The document's locale: one well-formed tag, or none when the document
/// names no language (`ambre-spec-locale.md` section 3).
///
/// `None` is a value, not a missing one: a consumer that resolves to it
/// formats without a locale. The only ways in are [`DocumentLocale::parse`],
/// a tag already validated, and [`DocumentLocale::read`], so no malformed
/// text is ever held (`LOCALE-002`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct DocumentLocale(Option<Validated<LangTag>>);

impl DocumentLocale {
    /// The locale a text spells.
    ///
    /// # Errors
    ///
    /// [`NotALangTag`] for a text that is no well-formed tag, the empty text
    /// included: a caller that means "no language" names [`Self::none`]
    /// (`LOCALE-010`).
    pub fn parse(text: &str) -> Result<Self, NotALangTag> {
        Validated::<LangTag>::parse(text).map(|tag| Self(Some(tag)))
    }

    /// The locale of a document that names no language.
    #[must_use]
    pub const fn none() -> Self {
        Self(None)
    }

    /// The locale a stored register holds. A peer may have written any text
    /// there, and a malformed one reads as no language rather than as a fixed
    /// one (`LOCALE-002`, `LOCALE-013`).
    #[must_use]
    pub fn read(stored: Option<&str>) -> Self {
        Self(stored.and_then(|text| Validated::<LangTag>::parse(text).ok()))
    }

    /// The tag, or `None` when the document names no language.
    #[must_use]
    pub fn tag(&self) -> Option<&Validated<LangTag>> {
        self.0.as_ref()
    }
}

impl From<Validated<LangTag>> for DocumentLocale {
    fn from(tag: Validated<LangTag>) -> Self {
        Self(Some(tag))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_well_formed_tag_parses_as_written() {
        for text in [
            "en",
            "fr-CA",
            "zh-Hant-TW",
            "de-CH-1996",
            "x-private",
            "EN-us",
        ] {
            assert_eq!(
                Validated::<LangTag>::parse(text).map(Validated::into_string),
                Ok(text.to_owned())
            );
        }
    }

    #[test]
    fn a_malformed_text_is_no_tag() {
        for text in [
            "",
            "e",
            "fr_CA",
            "fr-",
            "-fr",
            "en--us",
            "x",
            "123",
            "en-toolongsubtag",
            "fr CA",
        ] {
            assert_eq!(
                Validated::<LangTag>::parse(text),
                Err(NotALangTag),
                "{text:?}"
            );
        }
    }

    #[test]
    fn the_primary_subtag_is_read_lowercased() {
        assert_eq!(
            Validated::<LangTag>::parse("FR-ca").map(|tag| primary_subtag(&tag)),
            Ok("fr".to_owned())
        );
    }

    /// `Debug` prints the tag as the hand-written type did, and
    /// the serde form is the bare string.
    #[test]
    fn a_tag_prints_and_travels_as_its_text() {
        let tag = Validated::<LangTag>::parse("fr-CA").unwrap();
        assert_eq!(format!("{tag:?}"), "LangTag(\"fr-CA\")");
        assert_eq!(serde_json::to_string(&tag).unwrap(), "\"fr-CA\"");
        assert!(serde_json::from_str::<Validated<LangTag>>("\"fr_CA\"").is_err());
    }
    #[test]
    fn a_malformed_locale_is_refused() {
        for text in ["", "fr_CA", "fr-", "x"] {
            assert_eq!(DocumentLocale::parse(text), Err(NotALangTag), "{text:?}");
        }
        assert_eq!(
            DocumentLocale::parse("fr-CA")
                .map(|locale| locale.tag().map(|t| t.as_str().to_owned())),
            Ok(Some("fr-CA".to_owned()))
        );
    }

    #[test]
    fn a_malformed_stored_locale_reads_as_none() {
        assert_eq!(DocumentLocale::read(Some("fr_CA")), DocumentLocale::none());
        assert_eq!(DocumentLocale::read(None), DocumentLocale::none());
        assert_eq!(
            DocumentLocale::read(Some("de-CH")),
            DocumentLocale::parse("de-CH").unwrap()
        );
    }
}
