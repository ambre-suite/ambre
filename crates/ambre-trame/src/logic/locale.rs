// ambre-trame/src/logic/locale.rs
// The workbook's locale and the reader's half (`ambre-spec-locale.md`).
//
// The register is one key of `meta`. A cell stores its number, its date and
// its formula in no locale (`LOCALE-014`). A view or an export applies the
// forms, and a typed text has them undone.

use super::{TrameOutcome, TrameState};
use crate::CellPos;
use crate::message::{
    CellFormat, CellText, DateParts, DayOfMonth, DisplayForm, SourceForm, TrameEvent,
};
use ambre_core::crdt::CrdtName as _;
use ambre_core::schema::MetaKey;
use ambre_core::{DocumentLocale, Grouping, NumberForms, ReaderLocale};
use loro::LoroValue;

impl TrameState {
    /// The workbook's locale, [`DocumentLocale::NONE`] when the register is
    /// absent or holds no well-formed tag.
    #[must_use]
    pub fn document_locale(&self) -> DocumentLocale {
        let value = self
            .doc
            .get_map(ambre_core::crdt::FoundationRoot::Meta.as_str())
            .get(MetaKey::Locale.as_str())
            .and_then(|value| value.into_value().ok());
        match value {
            Some(LoroValue::String(text)) => DocumentLocale::from_register(Some(text.as_str())),
            _ => DocumentLocale::NONE,
        }
    }

    /// The reader's own locale and convention.
    #[must_use]
    pub fn reader_locale(&self) -> &ReaderLocale {
        &self.reader
    }

    /// The forms this reader sees numbers and dates in.
    pub(super) fn shown_forms(&self) -> NumberForms {
        self.reader.shown(&self.document_locale()).forms()
    }

    /// The forms this reader types numbers and dates in.
    pub(super) fn typed_forms(&self) -> NumberForms {
        self.reader.typed(&self.document_locale()).forms()
    }

    /// The forms an export writes in: the workbook's, whoever runs it
    /// (`LOCALE-020`). No export of this build names a locale of its own, so
    /// the request half of the order is none (`LOCALE-011`).
    pub(super) fn export_forms(&self) -> NumberForms {
        ambre_core::resolve(None, &self.document_locale()).forms()
    }

    /// The event that tells the view the workbook's locale and the reader's
    /// half it is read under.
    pub(super) fn locale_state_event(&self) -> TrameEvent {
        TrameEvent::LocaleState {
            document: self.document_locale(),
            reader: self.reader.clone(),
        }
    }

    /// Write `locale` into the register: one delta, and no cell moves
    /// (`LOCALE-009`). The same locale writes nothing.
    pub(super) fn set_document_locale(&mut self, locale: &DocumentLocale) -> TrameOutcome {
        if *locale == self.document_locale() {
            return TrameOutcome::display_only(vec![]);
        }
        let meta = self
            .doc
            .get_map(ambre_core::crdt::FoundationRoot::Meta.as_str());
        let _ = match locale.as_str() {
            Some(tag) => meta.insert(MetaKey::Locale.as_str(), tag),
            None => meta.delete(MetaKey::Locale.as_str()),
        };
        self.doc.commit();
        let update = self.flush_delta();
        TrameOutcome {
            display: vec![self.locale_state_event(), TrameEvent::DataReset],
            update,
            ephemeral: None,
        }
    }

    /// Hold the reader's half, with no event: a state built before any
    /// viewport is served. A live view takes
    /// [`TrameCommand::SetReaderLocale`](crate::TrameCommand::SetReaderLocale).
    pub fn adopt_reader_locale(&mut self, reader: ReaderLocale) {
        self.reader = reader;
    }

    /// Hold the reader's half and repaint: nothing is written
    /// (`LOCALE-017`).
    pub(super) fn set_reader_locale(&mut self, reader: ReaderLocale) -> TrameOutcome {
        if reader == self.reader {
            return TrameOutcome::display_only(vec![]);
        }
        self.reader = reader;
        TrameOutcome::display_only(vec![self.locale_state_event(), TrameEvent::DataReset])
    }

    /// What a typed `text` stores, read in the reader's forms.
    ///
    /// * A number becomes the canonical one (`TRAME-237`, `TRAME-238`).
    /// * A numeric date becomes its day serial (`LOCALE-014`).
    /// * A formula takes the canonical separators (`LOCALE-018`).
    /// * Any other text is kept as typed.
    ///
    /// A reader with no locale types the canonical forms, which need no
    /// reading.
    pub(super) fn typed_source(&self, text: CellText<SourceForm>) -> Typed {
        let forms = self.typed_forms();
        let value = |text| Typed {
            text,
            kind: TypedKind::Value,
        };
        if forms.is_neutral() {
            return value(text);
        }
        if text.is_formula() {
            return value(CellText::new(canonical_formula(text.as_str(), &forms)));
        }
        if let Some(number) = forms.canonical(text.as_str()) {
            return value(CellText::new(number));
        }
        match forms.typed_date(text.as_str()).and_then(day_serial) {
            Some(serial) => Typed {
                text: CellText::new(serial.get().to_string()),
                kind: TypedKind::Date,
            },
            None => value(text),
        }
    }

    /// Show the date a reader typed at `at` as a date. A `General` cell takes
    /// the `Date` format, and a cell with a format of its own keeps it. The
    /// caller commits.
    pub(super) fn format_typed_date(&mut self, at: CellPos) {
        let mut style = self.styles.get(&at).cloned().unwrap_or_default();
        if style.format != CellFormat::General {
            return;
        }
        style.format = CellFormat::Date;
        self.styles.insert(at, style);
        let _ = self.write_cell_style(at);
    }

    /// The edit-bar text of a stored `source`, in the forms this reader types
    /// in (`LOCALE-023`). A commit of it back stores `source` again.
    pub(super) fn edit_bar_text(&self, source: CellText<DisplayForm>) -> CellText<DisplayForm> {
        let forms = self.typed_forms();
        if forms.is_neutral() {
            return source;
        }
        CellText::new(if source.as_str().starts_with('=') {
            localized_formula(source.as_str(), &forms)
        } else {
            forms.localize(source.as_str(), Grouping::Ungrouped)
        })
    }
}

/// What a typed text stores, and whether it was a date.
pub(super) struct Typed {
    /// The text the cell stores.
    pub(super) text: CellText<SourceForm>,
    /// Whether the reader typed a date, which then shows as one.
    pub(super) kind: TypedKind,
}

/// Whether a typed text was a date.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum TypedKind {
    /// A number, a formula or a text.
    Value,
    /// A numeric date, stored as its day serial.
    Date,
}

/// The day serial of a typed date. A day past the end of its month gives
/// `None`: `31/02/2026` is no date, rather than the 3rd of March.
fn day_serial(date: ambre_core::TypedDate) -> Option<ambre_core::Instant<ambre_core::SheetDay>> {
    let parts = DateParts {
        year: date.year,
        month: date.month,
        day: DayOfMonth::new(i64::from(date.day.get())),
    };
    let serial = parts.serial();
    (DateParts::of_day(serial) == parts).then_some(serial)
}

/// Whether `forms` writes a formula with a semicolon between arguments: the
/// forms whose decimal mark is the comma.
fn semicolon_arguments(forms: &NumberForms) -> bool {
    forms.localize("0.5", Grouping::Ungrouped) == "0,5"
}

/// The scan state of a formula text, outside or inside a quoted run.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Quote {
    /// Outside any quote.
    None,
    /// Inside a `"` string literal.
    Text,
    /// Inside a `'` quoted sheet name.
    Sheet,
}

/// The number literal being scanned, and whether its decimal mark is spent.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Literal {
    /// No number literal is open.
    Closed,
    /// A literal is open, before its decimal mark.
    Whole,
    /// A literal is open, past its decimal mark or its exponent.
    Spent,
}

/// `canonical` with the separators of `forms`: `=SUM(1.5,2)` reads
/// `=SUM(1,5;2)` in French. Text in quotes is kept verbatim.
fn localized_formula(canonical: &str, forms: &NumberForms) -> String {
    if !semicolon_arguments(forms) {
        return canonical.to_owned();
    }
    translate(canonical, |c, literal, next_is_digit| match c {
        '.' if literal == Literal::Whole && next_is_digit => (',', Literal::Spent),
        ',' => (';', Literal::Closed),
        _ => (c, Literal::Closed),
    })
}

/// `typed` in the canonical separators: the inverse of
/// [`localized_formula`]. A canonical text typed as it stands is kept,
/// since a comma after a decimal point ends the literal.
fn canonical_formula(typed: &str, forms: &NumberForms) -> String {
    if !semicolon_arguments(forms) {
        return typed.to_owned();
    }
    translate(typed, |c, literal, next_is_digit| match c {
        ',' if literal == Literal::Whole && next_is_digit => ('.', Literal::Spent),
        '.' if literal == Literal::Whole => ('.', Literal::Spent),
        ';' => (',', Literal::Closed),
        _ => (c, Literal::Closed),
    })
}

/// Rewrite each character of `formula` outside quotes through `swap`. It
/// sees the state of the number literal and whether a digit follows. It
/// answers the character and the state after it.
fn translate(formula: &str, swap: impl Fn(char, Literal, bool) -> (char, Literal)) -> String {
    let chars: Vec<char> = formula.chars().collect();
    let mut out = String::with_capacity(formula.len());
    let mut quote = Quote::None;
    let mut literal = Literal::Closed;
    // A letter, a digit, `_` or `$` before a digit makes it part of a
    // reference or a name, `A1`, never a literal.
    let mut after_word = false;
    for (at, &c) in chars.iter().enumerate() {
        let next_is_digit = chars.get(at + 1).is_some_and(char::is_ascii_digit);
        match quote {
            Quote::Text => {
                if c == '"' {
                    quote = Quote::None;
                }
                out.push(c);
                continue;
            }
            Quote::Sheet => {
                if c == '\'' {
                    quote = Quote::None;
                }
                out.push(c);
                continue;
            }
            Quote::None => {}
        }
        match c {
            '"' => {
                quote = Quote::Text;
                literal = Literal::Closed;
                after_word = false;
                out.push(c);
            }
            '\'' => {
                quote = Quote::Sheet;
                literal = Literal::Closed;
                after_word = false;
                out.push(c);
            }
            d if d.is_ascii_digit() => {
                if literal == Literal::Closed && !after_word {
                    literal = Literal::Whole;
                }
                out.push(d);
            }
            'e' | 'E' if literal != Literal::Closed => {
                literal = Literal::Spent;
                out.push(c);
            }
            '+' | '-' if literal == Literal::Spent && out.ends_with(['e', 'E']) => {
                out.push(c);
            }
            _ => {
                let (swapped, next) = swap(c, literal, next_is_digit);
                out.push(swapped);
                after_word = c.is_alphanumeric() || matches!(c, '_' | '$');
                literal = if after_word { Literal::Closed } else { next };
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forms(tag: &str) -> NumberForms {
        DocumentLocale::parse(tag).unwrap().forms()
    }

    /// `LOCALE-018`, `LOCALE-023`: a French reader reads and types a formula
    /// with a decimal comma and semicolons, and the canonical text comes back.
    #[test]
    fn a_formula_takes_the_readers_separators_both_ways() {
        let fr = forms("fr");
        for (canonical, french) in [
            ("=SUM(1.5,2)", "=SUM(1,5;2)"),
            ("=IF(A1>0.25,\"a,b.c\",B2)", "=IF(A1>0,25;\"a,b.c\";B2)"),
            ("='My, sheet'!A1+1.5e3", "='My, sheet'!A1+1,5e3"),
            ("=ROUND(A1,2)", "=ROUND(A1;2)"),
            ("=SUM(A1:B2,C3)", "=SUM(A1:B2;C3)"),
            ("=1.5", "=1,5"),
        ] {
            assert_eq!(localized_formula(canonical, &fr), french, "{canonical}");
            assert_eq!(canonical_formula(french, &fr), canonical, "{french}");
        }
        // A canonical text a French reader types as it stands is kept.
        assert_eq!(canonical_formula("=SUM(1.5,2)", &fr), "=SUM(1.5,2)");
        // A locale with a decimal point keeps the canonical separators.
        assert_eq!(
            localized_formula("=SUM(1.5,2)", &forms("en")),
            "=SUM(1.5,2)"
        );
        assert_eq!(
            localized_formula("=SUM(1.5,2)", &forms("de-CH")),
            "=SUM(1.5,2)"
        );
    }
}
