// ambre-trame/src/ui/locale.rs
// What the chrome says about the locale a reader sees and types in
// (`ambre-spec-locale.md`). Pure functions, so the host tests read them; the
// chrome only mounts their text.

use ambre_core::{DateAmbiguity, DocumentLocale, ReaderLocale};

/// The workbook's locale and the reader's half, as the last
/// [`TrameEvent::LocaleState`](crate::TrameEvent::LocaleState) named them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LocaleView {
    /// The workbook's locale.
    pub document: DocumentLocale,
    /// The reader's own locale and convention.
    pub reader: ReaderLocale,
}

/// The tag a reader reads for `locale`, or the words for none.
fn spelled(locale: &DocumentLocale) -> &str {
    locale.as_str().unwrap_or("no language")
}

/// The notice a view shows when it formats in a locale other than the
/// workbook's (`LOCALE-021`), `None` when the two agree. The words are the
/// interface's, never the workbook's (`LOCALE-003`).
#[must_use]
pub fn locale_notice(view: &LocaleView) -> Option<String> {
    view.reader.shows_another(&view.document).then(|| {
        format!(
            "Numbers shown in {}; the document is in {}",
            spelled(view.reader.shown(&view.document)),
            spelled(&view.document)
        )
    })
}

/// The long form of a numeric date being typed whose day and month could
/// swap, in the reader's own language (`LOCALE-022`). `None` for any other
/// text, so the edit bar shows nothing.
#[must_use]
pub fn date_preview(typed: &str, view: &LocaleView) -> Option<String> {
    let date = view
        .reader
        .typed(&view.document)
        .forms()
        .typed_date(typed)?;
    if date.ambiguity != DateAmbiguity::Swappable {
        return None;
    }
    let own = view.reader.own.forms();
    Some(own.long_date(date.year, date.month, date.day))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_core::NumberConvention;

    fn view(document: &str, own: &str, convention: NumberConvention) -> LocaleView {
        LocaleView {
            document: DocumentLocale::parse(document).unwrap(),
            reader: ReaderLocale {
                own: DocumentLocale::parse(own).unwrap(),
                convention,
            },
        }
    }

    /// `LOCALE-021`: only a view in another locale than the workbook's says
    /// so, and it names both.
    #[test]
    fn a_view_in_another_locale_says_so() {
        assert_eq!(
            locale_notice(&view("en", "fr", NumberConvention::AllMine)).as_deref(),
            Some("Numbers shown in fr; the document is in en")
        );
        assert_eq!(
            locale_notice(&view("en", "fr", NumberConvention::TypeMine)),
            None
        );
        assert_eq!(
            locale_notice(&view("fr", "fr", NumberConvention::AllMine)),
            None
        );
    }

    /// `LOCALE-022`: a date whose day and month could swap shows its long
    /// form in the reader's language; any other text shows nothing.
    #[test]
    fn a_swappable_date_shows_its_long_form_before_the_commit() {
        let french = view("en", "fr", NumberConvention::TypeMine);
        assert_eq!(
            date_preview("03/04/2026", &french).as_deref(),
            Some("3 avril 2026")
        );
        assert_eq!(date_preview("13/04/2026", &french), None);
        assert_eq!(date_preview("3,5", &french), None);
        // Under the document's convention, the order is the workbook's, and
        // the words stay the reader's.
        let typed_in_english = view("en", "fr", NumberConvention::Document);
        assert_eq!(
            date_preview("03/04/2026", &typed_in_english).as_deref(),
            Some("4 mars 2026")
        );
    }
}
