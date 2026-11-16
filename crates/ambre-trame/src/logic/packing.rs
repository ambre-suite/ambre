// ambre-trame/src/logic/packing.rs
// Style helpers for the Logic thread (`trame-cell-styles`): merging a toolbar
// `StylePatch` into a cell's `CellStyle`, and rendering a numeric value per its
// number format. The CRDT no longer packs style into an i64 bit-mask. A cell's
// whole style is one atomic `cellstyle_{sheet}` register (see `schema.rs` /
// `styles.rs`), so `CellStyle` is stored serde-encoded verbatim.

use super::*;
use ambre_client_core::{Grouping, NumberForms};
use ambre_core::{Instant, SheetDateTime};

/// Merge a [`StylePatch`] into a [`CellStyle`] in place: each `Some` field of the
/// patch overwrites the style; `None` leaves it. Bold/italic are not part of
/// [`CellStyle`] (they live in their own sets), so they are ignored here.
pub(crate) fn apply_patch_to_style(s: &mut CellStyle, p: &StylePatch) {
    if let Some(u) = p.underline {
        s.underline = u;
    }
    if let Some(st) = p.strikethrough {
        s.strikethrough = st;
    }
    if let Some(a) = p.h_align {
        s.h_align = a;
    }
    if let Some(a) = p.v_align {
        s.v_align = a;
    }
    if let Some(w) = p.wrap {
        s.wrap = w;
    }
    if let Some(fg) = p.fg {
        s.fg = fg;
    }
    if let Some(bg) = p.bg {
        s.bg = bg;
    }
    if let Some(font) = &p.font {
        s.font = font.clone();
    }
    if let Some(size) = p.size {
        s.size = size;
    }
    if let Some(f) = &p.format {
        s.format = f.clone();
    }
    if let Some(d) = p.decimals {
        s.decimals = d;
    }
    if let Some(step) = p.decimals_step
        && let Some(default) = s.format.default_decimals()
    {
        let current = s.decimals.unwrap_or(default);
        s.decimals = Some(step.apply_to(current));
    }
    // `p.border` (the selection-level edit: preset + stroke style + colour) is
    // resolved into per-cell edges by `set_cell_style`, which has the selection
    // geometry; not handled here.
}

/// Format a numeric `value` per a [`CellFormat`] and decimal count. Non-numeric
/// cells keep their text (handled by the caller); this is the numeric branch.
pub(crate) fn format_number(
    value: f64,
    fmt: &CellFormat,
    decimals: Option<Bounded<Decimals>>,
    forms: &NumberForms,
) -> String {
    // Only the four numeric families read the count, and each has a default.
    let places = decimals
        .or_else(|| fmt.default_decimals())
        .map_or(0, |count| usize::from(count.get()));
    // Each family writes the canonical digits, and the forms place the marks
    // (`TRAME-110`). The neutral forms leave them as written.
    let marked = |text: String, grouping| forms.localize(&text, grouping);
    match fmt {
        CellFormat::Number => marked(format!("{value:.places$}"), Grouping::Ungrouped),
        // The symbol is the format's, and the locale places it: a locale
        // change never turns one currency into another (`LOCALE-009`).
        CellFormat::Currency => forms.currency(
            &marked(format!("{value:.places$}"), Grouping::Grouped),
            "\u{20AC}",
        ),
        CellFormat::Percent => {
            marked(format!("{:.places$}", value * 100.0), Grouping::Ungrouped) + "%"
        }
        CellFormat::Scientific => marked(format!("{value:.places$e}"), Grouping::Ungrouped),
        CellFormat::Fraction => {
            // Simple proper-fraction rendering over a /16 denominator (MVP).
            // Round the whole value to sixteenths in one step. A fraction that
            // rounds up to 16/16 then carries into the integer part, instead of
            // being printed as such.
            let sixteenths = (value * 16.0).round() as i64;
            let whole = sixteenths / 16;
            let frac = (sixteenths % 16).abs();
            if frac == 0 {
                whole.to_string()
            } else if whole == 0 {
                // A zero integer part cannot carry the sign of a negative value,
                // so the fraction has to: "-8/16", not "8/16".
                let sign = if sixteenths < 0 { "-" } else { "" };
                format!("{sign}{frac}/16")
            } else {
                format!("{whole} {frac}/16")
            }
        }
        CellFormat::Date => {
            super::number_format::format_date(Instant::<SheetDateTime>::new(value), forms)
        }
        CellFormat::Time => super::number_format::format_time(Instant::<SheetDateTime>::new(value)),
        CellFormat::Custom(pattern) => super::number_format::format_custom(value, pattern, forms),
        // General and Text leave the value untouched.
        CellFormat::General | CellFormat::Text => format!("{value}"),
    }
}

// Column-label conversion lives in the crate-level `columns` module. The
// tests of this module read it through here.
#[cfg(test)]
pub(crate) use crate::columns::col_to_alpha;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fraction_format_keeps_the_sign_and_carries() {
        let frac = |v: f64| format_number(v, &CellFormat::Fraction, None, &NumberForms::NEUTRAL);
        // A value below 1 has a zero integer part, which cannot carry the sign.
        assert_eq!(frac(-0.5), "-8/16");
        assert_eq!(frac(0.5), "8/16");
        // Rounding to the nearest sixteenth must carry, not print "16/16".
        assert_eq!(frac(0.97), "1");
        assert_eq!(frac(-0.97), "-1");
        assert_eq!(frac(-1.5), "-1 8/16");
        assert_eq!(frac(2.0), "2");
    }
}
