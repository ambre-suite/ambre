//! A language tag, as a document names the language its content is in.

/// A well-formed BCP 47 language tag: `en`, `fr-CA`, `zh-Hant-TW`.
///
/// The check is RFC 5646's shape, not its registry: subtags of one to eight
/// ASCII letters and digits joined by `-`, the first of two to eight letters,
/// or the `x` and `i` prefixes. A tag that passes may still name no language
/// anybody speaks; a text that fails is no tag at all.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LangTag(String);

impl LangTag {
    /// The tag `text` spells, or `None` when it is not a well-formed tag.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let mut subtags = text.split('-');
        let primary = subtags.next()?;
        let prefix = primary.eq_ignore_ascii_case("x") || primary.eq_ignore_ascii_case("i");
        let language = (2..=8).contains(&primary.len())
            && primary.bytes().all(|byte| byte.is_ascii_alphabetic());
        if !(language || prefix) {
            return None;
        }
        let mut rest = 0_usize;
        for subtag in subtags {
            if !(1..=8).contains(&subtag.len())
                || !subtag.bytes().all(|byte| byte.is_ascii_alphanumeric())
            {
                return None;
            }
            rest += 1;
        }
        // A bare `x` or `i` is a prefix with nothing after it.
        if prefix && rest == 0 {
            return None;
        }
        Some(Self(text.to_owned()))
    }

    /// The tag as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The primary language subtag, lowercased: `fr` for `fr-CA`.
    #[must_use]
    pub fn primary(&self) -> String {
        self.0
            .split('-')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase()
    }
}

impl std::fmt::Display for LangTag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
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
            assert_eq!(LangTag::parse(text).map(|tag| tag.0), Some(text.to_owned()));
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
            assert_eq!(LangTag::parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn the_primary_subtag_is_read_lowercased() {
        assert_eq!(
            LangTag::parse("FR-ca").map(|tag| tag.primary()),
            Some("fr".to_owned())
        );
    }
}
