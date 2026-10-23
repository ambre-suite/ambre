//! The text a band shows on one page: its block's text, each field replaced by
//! its value (`features/ambre-spec-prose-headers-footers.md`).
//!
//! A field is resolved from the page it is drawn on (`PROSE-UI-047`), the
//! document's last-modified stamp and its language. A reader's clock is never
//! read, so every reader resolves one date (`PROSE-FEAT-066`). Nothing here
//! writes to the document.

use loro::{LoroDoc, LoroValue, TextDelta, TreeID};

use crate::doc;
use crate::events::DateFieldFormat;

/// The milliseconds of one day.
const DAY_MS: i64 = 86_400_000;

/// The text of `block` as drawn on the zero-based `page` of `page_count`.
#[must_use]
pub fn resolve(document: &LoroDoc, block: &TreeID, page: u32, page_count: u32) -> String {
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
                .map(|(key, value)| (key.to_string(), value.clone()))
        });
        let Some((key, value)) = field else {
            out.push_str(&insert);
            continue;
        };
        let shown = field_value(document, &key, &value, page, page_count);
        // One value per field character: a span may carry several fields that
        // share a key and a value.
        for _ in insert.chars() {
            out.push_str(&shown);
        }
    }
    out
}

/// What one field shows on one page.
fn field_value(document: &LoroDoc, key: &str, value: &LoroValue, page: u32, count: u32) -> String {
    match key {
        "f:pn" => page.saturating_add(1).to_string(),
        "f:pc" => count.to_string(),
        "f:ti" => doc::read_title(document),
        "f:dt" => {
            // The word `field_mark` stores. Any other value draws the ISO
            // date, as an unknown format always has.
            let format = match value {
                LoroValue::String(word) if word.as_str() == "short" => DateFieldFormat::Short,
                LoroValue::String(word) if word.as_str() == "long" => DateFieldFormat::Long,
                _ => DateFieldFormat::Iso,
            };
            doc::read_updated_at(document)
                .map(|ms| format_date(ms, format, &doc::read_lang(document)))
                .unwrap_or_default()
        }
        _ => String::new(),
    }
}

/// A calendar date in UTC: year, month from one, day from one.
///
/// UTC and not the reader's zone, so two readers in two zones draw one date
/// (`PROSE-FEAT-066`). The civil-from-days algorithm of H. Hinnant.
#[must_use]
pub fn civil_date(ms: i64) -> (i64, u32, u32) {
    let days = ms.div_euclid(DAY_MS) + 719_468;
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
    (
        year,
        u32::try_from(month).unwrap_or(1),
        u32::try_from(day).unwrap_or(1),
    )
}

/// The twelve month names of the languages that have a long and a short form
/// here. Any other language draws the calendar form for all three formats.
fn months(lang: &str) -> Option<[&'static str; 12]> {
    Some(match lang {
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
/// (`PROSE-FEAT-282`). The language is read from the tag's primary subtag.
#[must_use]
pub fn format_date(ms: i64, format: DateFieldFormat, lang: &str) -> String {
    let (year, month, day) = civil_date(ms);
    let iso = format!("{year:04}-{month:02}-{day:02}");
    let primary = lang.split(['-', '_']).next().unwrap_or("");
    let primary = primary.to_ascii_lowercase();
    let Some(names) = months(&primary) else {
        return iso;
    };
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
    const MARCH_FIFTH: i64 = 1_772_712_000_000;

    #[test]
    fn a_stamp_reads_as_its_utc_calendar_date() {
        assert_eq!(civil_date(0), (1970, 1, 1));
        assert_eq!(civil_date(MARCH_FIFTH), (2026, 3, 5));
        assert_eq!(civil_date(-1), (1969, 12, 31));
        assert_eq!(civil_date(951_782_400_000), (2000, 2, 29));
    }

    #[test]
    fn the_date_follows_the_document_language() {
        assert_eq!(
            format_date(MARCH_FIFTH, DateFieldFormat::Long, "en"),
            "March 5, 2026"
        );
        assert_eq!(
            format_date(MARCH_FIFTH, DateFieldFormat::Long, "fr-CA"),
            "5 mars 2026"
        );
        assert_eq!(
            format_date(MARCH_FIFTH, DateFieldFormat::Long, "de"),
            "5. M\u{e4}rz 2026"
        );
        assert_eq!(
            format_date(MARCH_FIFTH, DateFieldFormat::Short, "en-US"),
            "3/5/2026"
        );
        assert_eq!(
            format_date(MARCH_FIFTH, DateFieldFormat::Short, "fr"),
            "05/03/2026"
        );
        assert_eq!(
            format_date(MARCH_FIFTH, DateFieldFormat::Iso, "fr"),
            "2026-03-05"
        );
    }

    #[test]
    fn a_language_without_names_draws_the_calendar_form() {
        assert_eq!(
            format_date(MARCH_FIFTH, DateFieldFormat::Long, "ja"),
            "2026-03-05"
        );
        assert_eq!(
            format_date(MARCH_FIFTH, DateFieldFormat::Short, ""),
            "2026-03-05"
        );
    }
}
