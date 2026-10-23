//! The text a band shows on one page: its block's text, each field replaced by
//! its value (`features/ambre-spec-prose-headers-footers.md`).
//!
//! A field is resolved from the page it is drawn on (`PROSE-UI-047`), the
//! document's last-modified stamp and its language. A reader's clock is never
//! read, so every reader resolves one date (`PROSE-FEAT-066`). Nothing here
//! writes to the document.

use ambre_core::{LangTag, Millis, Timestamp};
use loro::{LoroDoc, LoroValue, TextDelta, TreeID};

use crate::doc;
use crate::events::{DateFieldFormat, HeaderField, PageCount, PageIndex};

/// The span of one day.
const DAY: Millis = Millis(86_400_000);

/// The text of `block` as drawn on `page` of `page_count`.
#[must_use]
pub fn resolve(
    document: &LoroDoc,
    block: &TreeID,
    page: PageIndex,
    page_count: PageCount,
) -> String {
    let text = doc::get_block_text(document, block);
    let mut out = String::new();
    for span in text.to_delta() {
        let TextDelta::Insert { insert, attributes } = span else {
            continue;
        };
        let field = attributes.as_ref().and_then(|attributes| {
            attributes
                .iter()
                .find(|(key, _)| key.starts_with("f:"))
                .map(|(key, value)| field_of(key, value))
        });
        let Some(field) = field else {
            out.push_str(&insert);
            continue;
        };
        // A field of a kind this build does not know shows nothing.
        let shown = field
            .map(|field| field_value(document, field, page, page_count))
            .unwrap_or_default();
        // One value per field character: a span may carry several fields that
        // share a key and a value.
        for _ in insert.chars() {
            out.push_str(&shown);
        }
    }
    out
}

/// The mark key and the stored value of a field. The date stores its format
/// and nothing else (`PROSE-FEAT-162`). [`field_of`] reads them back.
#[must_use]
pub const fn field_mark(field: HeaderField) -> (&'static str, &'static str) {
    match field {
        HeaderField::PageNumber => ("f:pn", ""),
        HeaderField::PageCount => ("f:pc", ""),
        HeaderField::Title => ("f:ti", ""),
        HeaderField::Date(DateFieldFormat::Short) => ("f:dt", "short"),
        HeaderField::Date(DateFieldFormat::Long) => ("f:dt", "long"),
        HeaderField::Date(DateFieldFormat::Iso) => ("f:dt", "iso"),
    }
}

/// The field a mark carries, or `None` for a mark that is not a field. A date
/// whose stored format is not one [`field_mark`] writes draws the ISO date,
/// as an unknown format always has.
#[must_use]
pub fn field_of(key: &str, value: &LoroValue) -> Option<HeaderField> {
    match key {
        "f:pn" => Some(HeaderField::PageNumber),
        "f:pc" => Some(HeaderField::PageCount),
        "f:ti" => Some(HeaderField::Title),
        "f:dt" => Some(HeaderField::Date(match value {
            LoroValue::String(word) if word.as_str() == "short" => DateFieldFormat::Short,
            LoroValue::String(word) if word.as_str() == "long" => DateFieldFormat::Long,
            _ => DateFieldFormat::Iso,
        })),
        _ => None,
    }
}

/// What one field shows on one page.
fn field_value(
    document: &LoroDoc,
    field: HeaderField,
    page: PageIndex,
    count: PageCount,
) -> String {
    match field {
        HeaderField::PageNumber => page.number().to_string(),
        HeaderField::PageCount => count.to_string(),
        HeaderField::Title => doc::read_title(document),
        HeaderField::Date(format) => doc::read_updated_at(document)
            .map(|at| format_date(at, format, doc::read_lang(document).as_ref()))
            .unwrap_or_default(),
    }
}

/// A calendar date in UTC: a year, a month from one and a day from one.
///
/// Only [`civil_date`] builds one, so the month is in `1 ..= 12` and the day
/// in `1 ..= 31`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CivilDate {
    year: i64,
    month: u32,
    day: u32,
}

impl CivilDate {
    /// The year of the common era.
    #[must_use]
    pub const fn year(self) -> i64 {
        self.year
    }

    /// The month, from one.
    #[must_use]
    pub const fn month(self) -> u32 {
        self.month
    }

    /// The day of the month, from one.
    #[must_use]
    pub const fn day(self) -> u32 {
        self.day
    }
}

impl std::fmt::Display for CivilDate {
    /// The ISO 8601 calendar form: `2026-03-05`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

/// The calendar date in UTC of the instant `at`.
///
/// UTC and not the reader's zone, so two readers in two zones draw one date
/// (`PROSE-FEAT-066`). The civil-from-days algorithm of H. Hinnant.
#[must_use]
pub fn civil_date(at: Timestamp) -> CivilDate {
    let day_ms = i64::try_from(DAY.0).unwrap_or(i64::MAX);
    let days = at.to_i64_saturating().div_euclid(day_ms) + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    CivilDate {
        year,
        month: u32::try_from(month).unwrap_or(1),
        day: u32::try_from(day).unwrap_or(1),
    }
}

/// The twelve month names of the languages that have a long and a short form
/// here. Any other language draws the calendar form for all three formats.
fn months(lang: &LangTag) -> Option<[&'static str; 12]> {
    Some(match lang.primary().as_str() {
        "en" => [
            "January",
            "February",
            "March",
            "April",
            "May",
            "June",
            "July",
            "August",
            "September",
            "October",
            "November",
            "December",
        ],
        "fr" => [
            "janvier",
            "f\u{e9}vrier",
            "mars",
            "avril",
            "mai",
            "juin",
            "juillet",
            "ao\u{fb}t",
            "septembre",
            "octobre",
            "novembre",
            "d\u{e9}cembre",
        ],
        "de" => [
            "Januar",
            "Februar",
            "M\u{e4}rz",
            "April",
            "Mai",
            "Juni",
            "Juli",
            "August",
            "September",
            "Oktober",
            "November",
            "Dezember",
        ],
        "es" => [
            "enero",
            "febrero",
            "marzo",
            "abril",
            "mayo",
            "junio",
            "julio",
            "agosto",
            "septiembre",
            "octubre",
            "noviembre",
            "diciembre",
        ],
        "it" => [
            "gennaio",
            "febbraio",
            "marzo",
            "aprile",
            "maggio",
            "giugno",
            "luglio",
            "agosto",
            "settembre",
            "ottobre",
            "novembre",
            "dicembre",
        ],
        "pt" => [
            "janeiro",
            "fevereiro",
            "mar\u{e7}o",
            "abril",
            "maio",
            "junho",
            "julho",
            "agosto",
            "setembro",
            "outubro",
            "novembro",
            "dezembro",
        ],
        "nl" => [
            "januari",
            "februari",
            "maart",
            "april",
            "mei",
            "juni",
            "juli",
            "augustus",
            "september",
            "oktober",
            "november",
            "december",
        ],
        _ => return None,
    })
}

/// A stamp drawn in one of the three formats, in the document's language
/// (`PROSE-FEAT-282`). The language is read from the tag's primary subtag; a
/// document with no language draws the calendar form.
#[must_use]
pub fn format_date(at: Timestamp, format: DateFieldFormat, lang: Option<&LangTag>) -> String {
    let date = civil_date(at);
    let (year, month, day) = (date.year, date.month, date.day);
    let iso = date.to_string();
    let Some((lang, names)) = lang.and_then(|lang| Some((lang, months(lang)?))) else {
        return iso;
    };
    let primary = lang.primary();
    let name = names
        .get(usize::try_from(month).unwrap_or(1).saturating_sub(1))
        .copied()
        .unwrap_or("");
    match (format, primary.as_str()) {
        (DateFieldFormat::Short, "en") => format!("{month}/{day}/{year}"),
        (DateFieldFormat::Short, "de") => format!("{day:02}.{month:02}.{year}"),
        (DateFieldFormat::Short, "nl") => format!("{day:02}-{month:02}-{year}"),
        (DateFieldFormat::Short, _) => format!("{day:02}/{month:02}/{year}"),
        (DateFieldFormat::Long, "en") => format!("{name} {day}, {year}"),
        (DateFieldFormat::Long, "de") => format!("{day}. {name} {year}"),
        (DateFieldFormat::Long, "es" | "pt") => format!("{day} de {name} de {year}"),
        (DateFieldFormat::Long, _) => format!("{day} {name} {year}"),
        (DateFieldFormat::Iso, _) => iso,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-03-05T12:00:00Z.
    const MARCH_FIFTH: Timestamp = Timestamp(1_772_712_000_000);

    fn ymd(at: Timestamp) -> (i64, u32, u32) {
        let date = civil_date(at);
        (date.year(), date.month(), date.day())
    }

    #[test]
    fn every_field_reads_back_from_the_mark_it_writes() {
        let fields = [
            HeaderField::PageNumber,
            HeaderField::PageCount,
            HeaderField::Title,
            HeaderField::Date(DateFieldFormat::Short),
            HeaderField::Date(DateFieldFormat::Long),
            HeaderField::Date(DateFieldFormat::Iso),
        ];
        for field in fields {
            let (key, value) = field_mark(field);
            assert_eq!(field_of(key, &LoroValue::from(value)), Some(field));
        }
        assert_eq!(field_of("b", &LoroValue::from("")), None);
    }

    fn tag(text: &str) -> Option<LangTag> {
        LangTag::parse(text)
    }

    #[test]
    fn a_stamp_reads_as_its_utc_calendar_date() {
        assert_eq!(ymd(Timestamp(0)), (1970, 1, 1));
        assert_eq!(ymd(MARCH_FIFTH), (2026, 3, 5));
        assert_eq!(ymd(Timestamp(86_399_999)), (1970, 1, 1));
        assert_eq!(ymd(Timestamp(951_782_400_000)), (2000, 2, 29));
        assert_eq!(civil_date(MARCH_FIFTH).to_string(), "2026-03-05");
    }

    #[test]
    fn the_date_follows_the_document_language() {
        assert_eq!(
            format_date(MARCH_FIFTH, DateFieldFormat::Long, tag("en").as_ref()),
            "March 5, 2026"
        );
        assert_eq!(
            format_date(MARCH_FIFTH, DateFieldFormat::Long, tag("fr-CA").as_ref()),
            "5 mars 2026"
        );
        assert_eq!(
            format_date(MARCH_FIFTH, DateFieldFormat::Long, tag("de").as_ref()),
            "5. M\u{e4}rz 2026"
        );
        assert_eq!(
            format_date(MARCH_FIFTH, DateFieldFormat::Short, tag("en-US").as_ref()),
            "3/5/2026"
        );
        assert_eq!(
            format_date(MARCH_FIFTH, DateFieldFormat::Short, tag("fr").as_ref()),
            "05/03/2026"
        );
        assert_eq!(
            format_date(MARCH_FIFTH, DateFieldFormat::Iso, tag("fr").as_ref()),
            "2026-03-05"
        );
    }

    #[test]
    fn a_language_without_names_draws_the_calendar_form() {
        assert_eq!(
            format_date(MARCH_FIFTH, DateFieldFormat::Long, tag("ja").as_ref()),
            "2026-03-05"
        );
        assert_eq!(
            format_date(MARCH_FIFTH, DateFieldFormat::Short, None),
            "2026-03-05"
        );
    }
}
