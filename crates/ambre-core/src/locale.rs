//! The document locale, the reader convention, and the forms a locale writes
//! numbers and dates in (`ambre-spec-locale.md`).
//!
//! A document carries one language tag, or none. A reader carries their own
//! tag and one convention of three. Every consumer resolves its locale
//! through [`resolve`]. It takes the request and the register alone, so no
//! resolution can read the machine (`LOCALE-012`).
//!
//! The forms are a table generated from CLDR, in [`table`]. A language the
//! table lacks formats in the neutral forms, as a document with no tag does
//! (`LOCALE-013`). No fixed language stands in for it.

use crate::bounded::{Bounded, Bounds, Refusal};
use crate::calendar::{Month, Year};
use crate::lang::{LangTag, NotALangTag, primary_subtag};
use crate::span::{Instant, SheetDay};
use crate::validated::Validated;

/// The locale a document or a reader names: one well-formed tag, or none.
///
/// `None` is a value, not a missing one: a consumer that resolves to it
/// formats in the neutral forms (`LOCALE-013`). The tag is kept as written.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct DocumentLocale(Option<Validated<LangTag>>);

/// A text that names no locale: it is no well-formed BCP 47 tag
/// (`LOCALE-010`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("a locale is a well-formed BCP 47 language tag")]
pub struct LocaleError;

impl From<NotALangTag> for LocaleError {
    fn from(_: NotALangTag) -> Self {
        Self
    }
}

impl DocumentLocale {
    /// The locale that names no language.
    pub const NONE: Self = Self(None);

    /// The locale a text spells.
    ///
    /// # Errors
    /// [`LocaleError`] for a text that is no well-formed tag, the empty text
    /// included (`LOCALE-010`). [`DocumentLocale::NONE`] is the one door to
    /// no language.
    pub fn parse(text: &str) -> Result<Self, LocaleError> {
        Ok(Self(Some(Validated::parse(text)?)))
    }

    /// The locale a stored register reads as. A malformed text is no
    /// language rather than a fixed one: a peer may have written any string.
    #[must_use]
    pub fn from_register(text: Option<&str>) -> Self {
        text.and_then(|text| Self::parse(text).ok())
            .unwrap_or(Self::NONE)
    }

    /// The tag, `None` when the locale names no language.
    #[must_use]
    pub const fn tag(&self) -> Option<&Validated<LangTag>> {
        self.0.as_ref()
    }

    /// The tag as written, `None` when the locale names no language.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        self.0.as_ref().map(Validated::as_str)
    }

    /// Whether the locale names no language.
    #[must_use]
    pub const fn is_none(&self) -> bool {
        self.0.is_none()
    }

    /// The forms this locale writes numbers and dates in.
    #[must_use]
    pub fn forms(&self) -> NumberForms {
        self.0
            .as_ref()
            .map_or(NumberForms::NEUTRAL, NumberForms::of)
    }
}

/// The locale a consumer formats in: the request's when it names one, then
/// the document's, then none (`LOCALE-011`, `LOCALE-013`).
///
/// The signature is the rule `LOCALE-012` rests on. It takes the request and
/// the register, and nothing that reads a machine's locale, clock zone or
/// region.
#[must_use]
pub fn resolve(requested: Option<&DocumentLocale>, document: &DocumentLocale) -> DocumentLocale {
    requested
        .filter(|locale| !locale.is_none())
        .unwrap_or(document)
        .clone()
}

/// Which locale a reader sees and types numbers and dates in. A closed set,
/// held per reader on their device, never in a document (`LOCALE-015`,
/// `LOCALE-017`).
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum NumberConvention {
    /// Shown and typed in the document's locale.
    Document,
    /// Shown in the document's locale, typed in the reader's own. The
    /// convention of a reader who picked none (`LOCALE-016`).
    #[default]
    TypeMine,
    /// Shown and typed in the reader's own locale.
    AllMine,
}

/// The reader's half: their own locale and their convention. Both live on
/// the reader's device and never in a document (`ambre-spec-locale.md`
/// section 3).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ReaderLocale {
    /// The locale the reader holds as their own.
    pub own: DocumentLocale,
    /// The convention the reader picked, [`NumberConvention::TypeMine`] by
    /// default.
    pub convention: NumberConvention,
}

impl ReaderLocale {
    /// The locale this reader sees `document`'s numbers and dates in.
    #[must_use]
    pub fn shown<'a>(&'a self, document: &'a DocumentLocale) -> &'a DocumentLocale {
        match self.convention {
            NumberConvention::Document | NumberConvention::TypeMine => document,
            NumberConvention::AllMine => &self.own,
        }
    }

    /// The locale this reader types numbers and dates in, over `document`.
    #[must_use]
    pub fn typed<'a>(&'a self, document: &'a DocumentLocale) -> &'a DocumentLocale {
        match self.convention {
            NumberConvention::Document => document,
            NumberConvention::TypeMine | NumberConvention::AllMine => &self.own,
        }
    }

    /// Whether this reader sees `document` in a locale other than its own
    /// tag, which the view then says (`LOCALE-021`).
    #[must_use]
    pub fn shows_another(&self, document: &DocumentLocale) -> bool {
        self.shown(document) != document
    }
}

mod table;

/// The order of the three parts of a numeric date.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DateOrder {
    /// `2026-09-29`: the neutral order, which no reader misreads.
    YearMonthDay,
    /// `29/09/2026`.
    DayMonthYear,
    /// `9/29/2026`.
    MonthDayYear,
}

/// A day of a month, from 1 to 31, as a date prints it.
pub enum MonthDay {}

impl Bounds for MonthDay {
    type Repr = u8;
    const MIN: u8 = 1;
    const MAX: u8 = 31;
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("MonthDay");
    const REFUSAL: Refusal = Refusal::new("a day of a month lies in 1..=31, not ", "");
}

/// The names of the months and the days of the week in one language.
#[derive(Debug, PartialEq, Eq)]
pub struct CalendarNames {
    /// January first.
    months: [&'static str; 12],
    /// Saturday first: a sheet day serial modulo seven indexes it.
    days: [&'static str; 7],
}

impl CalendarNames {
    /// The full name of `month`.
    #[must_use]
    pub fn month(&self, month: Bounded<Month>) -> &'static str {
        self.months
            .get(usize::from(month.get()).saturating_sub(1))
            .copied()
            .unwrap_or_default()
    }

    /// The full name of the day `day` falls on: the serial zero is a
    /// Saturday.
    #[must_use]
    pub fn weekday(&self, day: Instant<SheetDay>) -> &'static str {
        let index = usize::try_from(day.get().rem_euclid(7)).unwrap_or_default();
        self.days.get(index).copied().unwrap_or_default()
    }
}

/// Whether a numeric date pads its day and month on two digits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Padding {
    /// `9/29/2026`.
    Bare,
    /// `29/09/2026`.
    TwoDigits,
}

/// How one locale writes a date in digits. The year is always written whole,
/// even where CLDR's short pattern cuts it to two digits: a cut year loses its
/// century.
#[derive(Debug, PartialEq, Eq)]
struct ShortDate {
    order: DateOrder,
    mark: char,
    padding: Padding,
}

/// One part of a date in words, as CLDR's long pattern spells it.
#[derive(Debug, PartialEq, Eq)]
enum LongPart {
    /// The day of the month, unpadded.
    Day,
    /// The full name of the month.
    Month,
    /// The year, whole.
    Year,
    /// Text written as it stands.
    Text(&'static str),
}

/// The text before and after the digits of an amount. A `\u{a4}` in either
/// stands for the currency symbol.
#[derive(Debug, PartialEq, Eq)]
struct Affix {
    prefix: &'static str,
    suffix: &'static str,
}

/// Where a currency symbol and a minus sign sit beside an amount.
#[derive(Debug, PartialEq, Eq)]
struct CurrencyForm {
    positive: Affix,
    negative: Affix,
}

/// What CLDR says one locale writes numbers and dates in. The records live
/// in the generated [`table`] alone.
#[derive(Debug, PartialEq, Eq)]
struct LocaleData {
    /// The tag, lowercased: a language, or a language and a region.
    tag: &'static str,
    /// The mark between the whole and the fraction.
    decimal: char,
    /// The mark between groups of three digits.
    group: char,
    /// Whether a whole of four digits groups (CLDR's minimum grouping).
    four_digits: Grouping,
    currency: CurrencyForm,
    short: ShortDate,
    long: &'static [LongPart],
    names: CalendarNames,
}

/// The CLDR currency placeholder.
const SYMBOL: &str = "\u{a4}";
/// A space a reader types where a locale groups with a no-break one: no
/// keyboard types the latter.
const NO_BREAK_SPACES: [char; 2] = ['\u{a0}', '\u{202f}'];
/// The typographic apostrophe a reader may type where a locale groups with
/// the ASCII one.
const TYPOGRAPHIC_APOSTROPHE: char = '\u{2019}';

/// The forms one locale writes numbers and dates in: a record of the CLDR
/// table, or the neutral forms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NumberForms(Option<&'static LocaleData>);

/// The digits of one number, split at the decimal mark: the shape both
/// [`NumberForms::localize`] and [`NumberForms::canonical`] work on.
struct Digits<'a> {
    sign: &'a str,
    whole: String,
    fraction: Option<&'a str>,
    exponent: &'a str,
}

impl NumberForms {
    /// The forms of no locale: a decimal point, no group, the ISO date. They
    /// are what Trame drew before the register existed, so a document with
    /// no tag reads as it did.
    pub const NEUTRAL: Self = Self(None);

    /// The forms `tag` names: its language and region when the table holds
    /// them, else its language alone, else the neutral forms.
    #[must_use]
    pub fn of(tag: &Validated<LangTag>) -> Self {
        let language = primary_subtag(tag);
        let regional = region_subtag(tag).map(|region| format!("{language}-{region}"));
        let find = |key: &str| table::LOCALES.iter().find(|data| data.tag == key);
        Self(
            regional
                .as_deref()
                .and_then(find)
                .or_else(|| find(&language)),
        )
    }

    /// The order of a numeric date.
    #[must_use]
    pub fn date_order(&self) -> DateOrder {
        self.0
            .map_or(DateOrder::YearMonthDay, |data| data.short.order)
    }

    /// The month and day names, `None` for the neutral forms.
    #[must_use]
    pub fn names(&self) -> Option<&'static CalendarNames> {
        self.0.map(|data| &data.names)
    }

    /// Whether these are the neutral forms.
    #[must_use]
    pub const fn is_neutral(&self) -> bool {
        self.0.is_none()
    }

    /// A canonical number text in these forms: the decimal mark swapped, and
    /// the whole part grouped when `grouping` asks and the forms group it.
    ///
    /// A text that is no canonical number is handed back unchanged, so a word
    /// or an error value never takes a mark.
    #[must_use]
    pub fn localize(&self, canonical: &str, grouping: Grouping) -> String {
        let Some(digits) = split_canonical(canonical) else {
            return canonical.to_owned();
        };
        let Some(data) = self.0 else {
            return canonical.to_owned();
        };
        let groups = grouping == Grouping::Grouped
            && (digits.whole.len() > 4
                || (digits.whole.len() == 4 && data.four_digits == Grouping::Grouped));
        let whole = if groups {
            group_digits(&digits.whole, data.group)
        } else {
            digits.whole
        };
        let mut out = String::with_capacity(canonical.len() + 4);
        out.push_str(digits.sign);
        out.push_str(&whole);
        if let Some(fraction) = digits.fraction {
            out.push(data.decimal);
            out.push_str(fraction);
        }
        out.push_str(digits.exponent);
        out
    }

    /// The canonical text of a number typed in these forms: no group, a
    /// decimal point, so the store holds no locale (`LOCALE-014`). `None`
    /// when `typed` is no
    /// number in these forms.
    ///
    /// The forms are read first, so a German `1.000` is a thousand. The
    /// canonical form is read next, so a text a paste wrote stays the number
    /// it was. A group mark is read only between groups of three digits, so
    /// `3,5` is a number in French and no number in English.
    #[must_use]
    pub fn canonical(&self, typed: &str) -> Option<String> {
        let typed = typed.trim();
        self.in_forms(typed)
            .or_else(|| split_canonical(typed).map(|digits| digits.join()))
    }

    /// The canonical text of `typed` read in these forms alone.
    fn in_forms(&self, typed: &str) -> Option<String> {
        let data = self.0?;
        let parts = Parts::of(typed, data.decimal);
        let digits = Digits {
            sign: parts.sign,
            whole: ungroup(data, parts.whole)?,
            fraction: parts.fraction,
            exponent: parts.exponent,
        };
        let text = digits.join();
        split_canonical(&text).is_some().then_some(text)
    }

    /// An amount, already in these forms, beside a currency symbol.
    #[must_use]
    pub fn currency(&self, amount: &str, symbol: &str) -> String {
        let Some(data) = self.0 else {
            // The neutral forms keep what Trame drew before the register
            // existed, so a workbook with no tag reads as it did.
            return format!("{amount} {symbol}");
        };
        let (affix, digits) = match amount.strip_prefix('-') {
            Some(digits) => (&data.currency.negative, digits),
            None => (&data.currency.positive, amount),
        };
        format!(
            "{}{digits}{}",
            affix.prefix.replace(SYMBOL, symbol),
            affix.suffix.replace(SYMBOL, symbol)
        )
    }

    /// A calendar date as these forms write it in digits.
    #[must_use]
    pub fn numeric_date(
        &self,
        year: Year,
        month: Bounded<Month>,
        day: Bounded<MonthDay>,
    ) -> String {
        let (order, mark, padding) = self
            .0
            .map_or((DateOrder::YearMonthDay, '-', Padding::TwoDigits), |data| {
                (data.short.order, data.short.mark, data.short.padding)
            });
        let (m, d) = (month.get(), day.get());
        let (m, d) = match padding {
            Padding::TwoDigits => (format!("{m:02}"), format!("{d:02}")),
            Padding::Bare => (m.to_string(), d.to_string()),
        };
        let y = format!("{:04}", year.get());
        match order {
            DateOrder::YearMonthDay => format!("{y}{mark}{m}{mark}{d}"),
            DateOrder::DayMonthYear => format!("{d}{mark}{m}{mark}{y}"),
            DateOrder::MonthDayYear => format!("{m}{mark}{d}{mark}{y}"),
        }
    }

    /// A calendar date in words, which no reader misreads (`LOCALE-022`).
    /// The neutral forms write the ISO date, which no reader misreads either.
    #[must_use]
    pub fn long_date(&self, year: Year, month: Bounded<Month>, day: Bounded<MonthDay>) -> String {
        let Some(data) = self.0 else {
            return Self::NEUTRAL.numeric_date(year, month, day);
        };
        data.long
            .iter()
            .map(|part| match part {
                LongPart::Day => day.get().to_string(),
                LongPart::Month => data.names.month(month).to_owned(),
                LongPart::Year => year.get().to_string(),
                LongPart::Text(text) => (*text).to_owned(),
            })
            .collect()
    }

    /// The numeric date a reader typed in these forms, `None` when `typed`
    /// is no date. The year takes four digits, the day and the month one or
    /// two, in the order of the forms, behind one mark.
    #[must_use]
    pub fn typed_date(&self, typed: &str) -> Option<TypedDate> {
        let typed = typed.trim();
        let mark = typed.chars().find(|c| matches!(c, '/' | '.' | '-'))?;
        let parts: Vec<&str> = typed.split(mark).collect();
        let [a, b, c] = parts.as_slice() else {
            return None;
        };
        let order = self.date_order();
        let (year, month, day) = match order {
            DateOrder::YearMonthDay => (a, b, c),
            DateOrder::DayMonthYear => (c, b, a),
            DateOrder::MonthDayYear => (c, a, b),
        };
        let digits = |text: &str, widths: std::ops::RangeInclusive<usize>| {
            (widths.contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit()))
                .then(|| text.parse::<i64>().ok())
                .flatten()
        };
        let (year, month, day) = (
            digits(year, 4..=4)?,
            digits(month, 1..=2)?,
            digits(day, 1..=2)?,
        );
        let month = Bounded::<Month>::numbered(month)?;
        let day = u8::try_from(day).ok().and_then(Bounded::<MonthDay>::new)?;
        // A day and a month that both read as a month could be swapped by a
        // reader of the other order (`LOCALE-022`).
        let ambiguity = if order != DateOrder::YearMonthDay && day.get() <= 12 {
            DateAmbiguity::Swappable
        } else {
            DateAmbiguity::Plain
        };
        Some(TypedDate {
            year: Year::new(year),
            month,
            day,
            ambiguity,
        })
    }
}

/// The digits of a whole part typed with group marks, the marks dropped.
/// `None` when a mark sits anywhere but between groups of three.
fn ungroup(data: &LocaleData, whole: &str) -> Option<String> {
    let groups: Vec<&str> = whole.split(|c| is_group_mark(data, c)).collect();
    let well_grouped = match groups.split_first() {
        Some((first, rest)) if !rest.is_empty() => {
            (1..=3).contains(&first.len()) && rest.iter().all(|group| group.len() == 3)
        }
        _ => true,
    };
    let digits: String = groups.concat();
    (well_grouped && digits.bytes().all(|b| b.is_ascii_digit())).then_some(digits)
}

/// Whether a reader typing in `data`'s forms means `c` as a group mark.
fn is_group_mark(data: &LocaleData, c: char) -> bool {
    if NO_BREAK_SPACES.contains(&data.group) {
        c == ' ' || NO_BREAK_SPACES.contains(&c)
    } else if data.group == '\'' {
        matches!(c, '\'' | TYPOGRAPHIC_APOSTROPHE)
    } else {
        c == data.group
    }
}

/// Whether a number groups the digits of its whole part.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grouping {
    /// `1234.5`.
    Ungrouped,
    /// `1,234.5`, when the forms group.
    Grouped,
}

/// Whether a typed numeric date could read as another one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateAmbiguity {
    /// The day lies past twelve, or the order is the year's first.
    Plain,
    /// The day and the month could swap: `03/04/2026`.
    Swappable,
}

/// A calendar date a reader typed in digits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TypedDate {
    /// The year.
    pub year: Year,
    /// The month.
    pub month: Bounded<Month>,
    /// The day of the month. A day past the month's last is no check of
    /// this type: the consumer counts the date.
    pub day: Bounded<MonthDay>,
    /// Whether a reader of another order could read it as another date.
    pub ambiguity: DateAmbiguity,
}

impl Digits<'_> {
    /// The canonical text of the number.
    fn join(&self) -> String {
        let mut out = String::from(self.sign);
        out.push_str(&self.whole);
        if let Some(fraction) = self.fraction {
            out.push('.');
            out.push_str(fraction);
        }
        out.push_str(self.exponent);
        out
    }
}

/// A number text cut at its sign, its decimal mark and its exponent, each
/// part as written: nothing is checked yet.
struct Parts<'a> {
    sign: &'a str,
    whole: &'a str,
    fraction: Option<&'a str>,
    exponent: &'a str,
}

impl<'a> Parts<'a> {
    /// `text` cut at its leading sign, at the first `decimal`, and at its
    /// `e`.
    fn of(text: &'a str, decimal: char) -> Self {
        let (sign, rest) = if let Some(rest) = text.strip_prefix('-') {
            ("-", rest)
        } else if let Some(rest) = text.strip_prefix('+') {
            ("+", rest)
        } else {
            ("", text)
        };
        let (mantissa, exponent) = rest
            .find(['e', 'E'])
            .map_or((rest, ""), |at| rest.split_at(at));
        let (whole, fraction) = match mantissa.split_once(decimal) {
            Some((whole, fraction)) => (whole, Some(fraction)),
            None => (mantissa, None),
        };
        Self {
            sign,
            whole,
            fraction,
            exponent,
        }
    }
}

/// The parts of a canonical number text, `None` for any other text.
fn split_canonical(text: &str) -> Option<Digits<'_>> {
    let Parts {
        sign,
        whole,
        fraction,
        exponent,
    } = Parts::of(text, '.');
    let digits = |part: &str| part.bytes().all(|b| b.is_ascii_digit());
    let exponent_digits = exponent
        .get(1..)
        .map(|rest| rest.strip_prefix(['+', '-']).unwrap_or(rest));
    let exponent_ok =
        exponent.is_empty() || exponent_digits.is_some_and(|rest| !rest.is_empty() && digits(rest));
    let some_digit = !whole.is_empty() || fraction.is_some_and(|f| !f.is_empty());
    (digits(whole) && fraction.is_none_or(digits) && exponent_ok && some_digit).then(|| Digits {
        sign,
        whole: whole.to_owned(),
        fraction,
        exponent,
    })
}

/// `digits` with `mark` between each group of three, counted from the right.
fn group_digits(digits: &str, mark: char) -> String {
    let len = digits.len();
    let mut out = String::with_capacity(len + len / 3 * mark.len_utf8());
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (len - index).is_multiple_of(3) {
            out.push(mark);
        }
        out.push(digit);
    }
    out
}

/// The region subtag of `tag`, lowercased: `ca` for `fr-CA`. A script
/// subtag of four letters is skipped, and a subtag past it is no region.
fn region_subtag(tag: &Validated<LangTag>) -> Option<String> {
    tag.as_str()
        .split('-')
        .skip(1)
        .find(|subtag| subtag.len() != 4)
        .filter(|subtag| {
            (subtag.len() == 2 && subtag.bytes().all(|b| b.is_ascii_alphabetic()))
                || (subtag.len() == 3 && subtag.bytes().all(|b| b.is_ascii_digit()))
        })
        .map(str::to_ascii_lowercase)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn locale(text: &str) -> DocumentLocale {
        DocumentLocale::parse(text).unwrap()
    }

    fn day(d: u8) -> Bounded<MonthDay> {
        Bounded::new(d).unwrap()
    }

    fn month(m: i64) -> Bounded<Month> {
        Bounded::<Month>::numbered(m).unwrap()
    }

    /// `LOCALE-002`, `LOCALE-010`: a well-formed tag parses as written, and
    /// any other text is refused, the empty one included.
    #[test]
    fn a_locale_is_a_well_formed_tag_or_refused() {
        assert_eq!(locale("fr-CA").as_str(), Some("fr-CA"));
        for text in ["", "fr_FR", "french!", "-", "en--us"] {
            assert_eq!(DocumentLocale::parse(text), Err(LocaleError), "{text:?}");
        }
        assert_eq!(
            DocumentLocale::from_register(Some("fr_FR")),
            DocumentLocale::NONE
        );
        assert_eq!(DocumentLocale::from_register(None), DocumentLocale::NONE);
        assert_eq!(DocumentLocale::from_register(Some("de")), locale("de"));
    }

    /// `LOCALE-011`, `LOCALE-013`: the request wins, then the document, then
    /// none, carried as none.
    #[test]
    fn resolution_takes_the_request_then_the_document_then_none() {
        let (fr, en) = (locale("fr"), locale("en"));
        assert_eq!(resolve(Some(&en), &fr), en);
        assert_eq!(resolve(None, &fr), fr);
        assert_eq!(resolve(Some(&DocumentLocale::NONE), &fr), fr);
        assert_eq!(resolve(None, &DocumentLocale::NONE), DocumentLocale::NONE);
        assert!(resolve(None, &DocumentLocale::NONE).forms().is_neutral());
    }

    /// `LOCALE-015`, `LOCALE-016`: three conventions, and a reader who picked
    /// none sees the document's locale and types in their own.
    #[test]
    fn each_convention_picks_the_locale_of_each_half() {
        let (doc, own) = (locale("en"), locale("fr"));
        let reader = |convention| ReaderLocale {
            own: own.clone(),
            convention,
        };
        let halves = |convention| {
            let r = reader(convention);
            (r.shown(&doc).clone(), r.typed(&doc).clone())
        };
        assert_eq!(
            halves(NumberConvention::Document),
            (doc.clone(), doc.clone())
        );
        assert_eq!(
            halves(NumberConvention::TypeMine),
            (doc.clone(), own.clone())
        );
        assert_eq!(
            halves(NumberConvention::AllMine),
            (own.clone(), own.clone())
        );
        assert_eq!(
            ReaderLocale::default().convention,
            NumberConvention::TypeMine
        );
        assert_eq!(
            halves(ReaderLocale::default().convention),
            halves(NumberConvention::TypeMine)
        );
    }

    /// `LOCALE-021`: only a reader shown another locale than the document's
    /// is told so.
    #[test]
    fn a_reader_shown_another_locale_is_told() {
        let doc = locale("en");
        let reader = |own: &str, convention| ReaderLocale {
            own: locale(own),
            convention,
        };
        assert!(reader("fr", NumberConvention::AllMine).shows_another(&doc));
        assert!(!reader("en", NumberConvention::AllMine).shows_another(&doc));
        assert!(!reader("fr", NumberConvention::TypeMine).shows_another(&doc));
        assert!(!reader("fr", NumberConvention::Document).shows_another(&doc));
    }

    #[test]
    fn each_locale_writes_its_own_separators() {
        let shown = |tag: &str| {
            locale(tag)
                .forms()
                .localize("-1234567.5", Grouping::Grouped)
        };
        assert_eq!(shown("en"), "-1,234,567.5");
        assert_eq!(shown("fr"), "-1\u{202f}234\u{202f}567,5");
        assert_eq!(shown("de"), "-1.234.567,5");
        assert_eq!(shown("de-CH"), "-1'234'567.5");
        // Spanish groups no whole of four digits (CLDR's minimum grouping).
        assert_eq!(
            locale("es").forms().localize("1234.5", Grouping::Grouped),
            "1234,5"
        );
        assert_eq!(
            locale("es").forms().localize("12345.5", Grouping::Grouped),
            "12.345,5"
        );
        // A region the table lacks takes its language's forms.
        assert_eq!(shown("fr-BE"), shown("fr"));
        assert_eq!(shown("sw"), "-1234567.5");
        assert_eq!(
            DocumentLocale::NONE
                .forms()
                .localize("-1234567.5", Grouping::Grouped),
            "-1234567.5"
        );
        assert_eq!(
            locale("fr").forms().localize("3.5", Grouping::Ungrouped),
            "3,5"
        );
        assert_eq!(
            locale("fr").forms().localize("1.5e10", Grouping::Grouped),
            "1,5e10"
        );
        assert_eq!(
            locale("fr").forms().localize("#DIV/0!", Grouping::Grouped),
            "#DIV/0!"
        );
    }

    /// `LOCALE-014`: a number typed in a locale's forms becomes
    /// the canonical text, and a text that is no number there stays none.
    #[test]
    fn a_typed_number_reads_back_canonical() {
        let fr = locale("fr").forms();
        assert_eq!(fr.canonical("3,5").as_deref(), Some("3.5"));
        assert_eq!(fr.canonical("1 234,5").as_deref(), Some("1234.5"));
        assert_eq!(fr.canonical("-1\u{202f}234").as_deref(), Some("-1234"));
        assert_eq!(fr.canonical("3.5").as_deref(), Some("3.5"));
        assert_eq!(fr.canonical("12 34"), None);
        let en = locale("en").forms();
        assert_eq!(en.canonical("1,234.5").as_deref(), Some("1234.5"));
        assert_eq!(en.canonical("3,5"), None);
        let de = locale("de").forms();
        assert_eq!(de.canonical("1.234,5").as_deref(), Some("1234.5"));
        assert_eq!(de.canonical("2,5e3").as_deref(), Some("2.5e3"));
        assert_eq!(de.canonical("abc"), None);
        let swiss = locale("de-CH").forms();
        assert_eq!(swiss.canonical("1'234.5").as_deref(), Some("1234.5"));
        assert_eq!(swiss.canonical("1\u{2019}234.5").as_deref(), Some("1234.5"));
        assert_eq!(NumberForms::NEUTRAL.canonical("3,5"), None);
        assert_eq!(
            NumberForms::NEUTRAL.canonical(" 42 ").as_deref(),
            Some("42")
        );
    }

    #[test]
    fn a_localized_number_reads_back_as_the_canonical_one() {
        for tag in [
            "en", "en-GB", "fr", "fr-CA", "fr-CH", "de", "de-AT", "de-CH", "es", "it", "it-CH",
            "pt", "nl",
        ] {
            let forms = locale(tag).forms();
            for canonical in ["0", "3.5", "-1234567.25", "1000", "12.0001"] {
                for grouping in [Grouping::Grouped, Grouping::Ungrouped] {
                    let shown = forms.localize(canonical, grouping);
                    assert_eq!(
                        forms.canonical(&shown).as_deref(),
                        Some(canonical),
                        "{tag} {shown:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_currency_sits_where_its_locale_puts_it() {
        let money = |tag: &str, amount: &str| {
            let forms = locale(tag).forms();
            forms.currency(&forms.localize(amount, Grouping::Grouped), "\u{20ac}")
        };
        assert_eq!(money("fr", "1234.50"), "1\u{202f}234,50\u{a0}\u{20ac}");
        assert_eq!(money("en", "-1234.50"), "-\u{20ac}1,234.50");
        assert_eq!(money("nl", "1234.50"), "\u{20ac}\u{a0}1.234,50");
        assert_eq!(money("nl", "-1234.50"), "\u{20ac}\u{a0}-1.234,50");
        assert_eq!(money("de-CH", "-1234.50"), "\u{20ac}-1'234.50");
        assert_eq!(money("pt", "1234.50"), "\u{20ac}\u{a0}1.234,50");
        assert_eq!(
            NumberForms::NEUTRAL.currency("1234.50", "\u{20ac}"),
            "1234.50 \u{20ac}"
        );
    }

    #[test]
    fn each_locale_orders_a_date() {
        let date = |tag: &str| {
            locale(tag)
                .forms()
                .numeric_date(Year::new(2026), month(9), day(5))
        };
        assert_eq!(date("en"), "9/5/2026");
        assert_eq!(date("en-GB"), "05/09/2026");
        assert_eq!(date("fr"), "05/09/2026");
        assert_eq!(date("de"), "05.09.2026");
        assert_eq!(date("fr-CA"), "2026-09-05");
        assert_eq!(date("fr-CH"), "05.09.2026");
        assert_eq!(date("es"), "5/9/2026");
        assert_eq!(date("nl"), "05-09-2026");
        assert_eq!(date("sw"), "2026-09-05");
        let long = |tag: &str| {
            locale(tag)
                .forms()
                .long_date(Year::new(2026), month(3), day(4))
        };
        assert_eq!(long("en"), "March 4, 2026");
        assert_eq!(long("fr"), "4 mars 2026");
        assert_eq!(long("de"), "4. M\u{e4}rz 2026");
        assert_eq!(long("es"), "4 de marzo de 2026");
        assert_eq!(long("pt"), "4 de mar\u{e7}o de 2026");
        assert_eq!(long("sw"), "2026-03-04");
    }

    /// `LOCALE-022`: a typed date whose day and month could swap says so.
    #[test]
    fn a_typed_date_whose_parts_could_swap_is_flagged() {
        let fr = locale("fr").forms();
        let swappable = fr.typed_date("03/04/2026").unwrap();
        assert_eq!(swappable.month, month(4));
        assert_eq!(swappable.day, day(3));
        assert_eq!(swappable.ambiguity, DateAmbiguity::Swappable);
        assert_eq!(
            fr.typed_date("13/04/2026").unwrap().ambiguity,
            DateAmbiguity::Plain
        );
        let en = locale("en").forms();
        assert_eq!(en.typed_date("3/4/2026").unwrap().month, month(3));
        assert_eq!(en.typed_date("13/4/2026"), None);
        assert_eq!(
            NumberForms::NEUTRAL
                .typed_date("2026-03-04")
                .unwrap()
                .ambiguity,
            DateAmbiguity::Plain
        );
        assert_eq!(fr.typed_date("3,5"), None);
        // A cut year, a mixed mark and a decimal number are no date.
        assert_eq!(fr.typed_date("03/04/26"), None);
        assert_eq!(fr.typed_date("03/04.2026"), None);
        assert_eq!(locale("de").forms().typed_date("1.5"), None);
    }

    #[test]
    fn names_follow_the_language() {
        let names = locale("fr").forms().names().unwrap();
        assert_eq!(names.month(month(8)), "ao\u{fb}t");
        assert_eq!(names.weekday(Instant::<SheetDay>::new(1)), "dimanche");
        assert!(NumberForms::NEUTRAL.names().is_none());
    }

    /// The register travels as its bare tag, and a malformed one is refused
    /// at the decode.
    #[test]
    fn the_locale_travels_as_its_tag() {
        assert_eq!(serde_json::to_string(&locale("fr")).unwrap(), "\"fr\"");
        assert_eq!(
            serde_json::to_string(&DocumentLocale::NONE).unwrap(),
            "null"
        );
        assert!(serde_json::from_str::<DocumentLocale>("\"fr_FR\"").is_err());
    }
}
