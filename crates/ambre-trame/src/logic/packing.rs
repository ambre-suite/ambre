// ambre-trame/src/logic/packing.rs
// Style helpers for the Logic thread (`trame-cell-styles`): merging a toolbar
// `StylePatch` into a cell's `CellStyle`, and rendering a numeric value per its
// number format. The CRDT no longer packs style into an i64 bit-mask. A cell's
// whole style is one atomic `cellstyle_{sheet}` register (see `schema.rs` /
// `styles.rs`), so `CellStyle` is stored serde-encoded verbatim.

use super::*;
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
        s.decimals = Some(current.stepped(step));
    }
    // `p.border` (the selection-level edit: preset + stroke style + colour) is
    // resolved into per-cell edges by `set_cell_style`, which has the selection
    // geometry; not handled here.
}

/// Format a numeric `value` per a [`CellFormat`] and decimal count. Non-numeric
/// cells keep their text (handled by the caller); this is the numeric branch.
pub(crate) fn format_number(value: f64, fmt: &CellFormat, decimals: Option<Decimals>) -> String {
    let dec = |default: Decimals| decimals.unwrap_or(default).precision();
    match fmt {
        CellFormat::Number => format!("{:.*}", dec(Decimals::TWO), value),
        CellFormat::Currency => format!("{:.*}", dec(Decimals::TWO), value) + " \u{20AC}",
        CellFormat::Percent => format!("{:.*}%", dec(Decimals::ZERO), value * 100.0),
        CellFormat::Scientific => format!("{:.*e}", dec(Decimals::TWO), value),
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
        CellFormat::Date => super::number_format::format_date(Instant::<SheetDateTime>::new(value)),
        CellFormat::Time => super::number_format::format_time(Instant::<SheetDateTime>::new(value)),
        CellFormat::Custom(pattern) => super::number_format::format_custom(value, pattern),
        // General and Text leave the value untouched.
        CellFormat::General | CellFormat::Text => format!("{value}"),
    }
}

// Column-label conversion now lives in the crate-level `columns` module;
// re-exported so `packing::col_to_alpha` call sites keep compiling.
pub(crate) use crate::columns::{col_to_alpha, row_label};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fraction_format_keeps_the_sign_and_carries() {
        let frac = |v: f64| format_number(v, &CellFormat::Fraction, None);
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
