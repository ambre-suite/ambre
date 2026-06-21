// ambre-trame/src/logic/packing.rs
// Style <-> packed-i64 conversions for the CRDT (`-ui.md` sec.5bis): alignment,
// number format, borders, the style mask, and number formatting.

use super::*;

pub(crate) fn pack_halign(a: HAlign) -> i64 {
    match a {
        HAlign::Auto => 0,
        HAlign::Left => 1,
        HAlign::Center => 2,
        HAlign::Right => 3,
    }
}
pub(crate) fn unpack_halign(v: i64) -> HAlign {
    match v {
        1 => HAlign::Left,
        2 => HAlign::Center,
        3 => HAlign::Right,
        _ => HAlign::Auto,
    }
}
// `Middle` is the default `VAlign`, so it MUST pack to 0: `persist_style` treats
// a zero style mask as "no style" and deletes the CRDT entry. If `Top` packed to
// 0, a Top-only style would vanish on the round-trip and revert to Middle.
pub(crate) fn pack_valign(a: VAlign) -> i64 {
    match a {
        VAlign::Middle => 0,
        VAlign::Top => 1,
        VAlign::Bottom => 2,
    }
}
pub(crate) fn unpack_valign(v: i64) -> VAlign {
    match v {
        1 => VAlign::Top,
        2 => VAlign::Bottom,
        _ => VAlign::Middle,
    }
}
pub(crate) fn pack_bordstyle(s: BorderStyle) -> i64 {
    match s {
        BorderStyle::Solid => 0,
        BorderStyle::Dashed => 1,
        BorderStyle::Dotted => 2,
        BorderStyle::Medium => 3,
        BorderStyle::Thick => 4,
        BorderStyle::Double => 5,
    }
}
pub(crate) fn unpack_bordstyle(v: i64) -> BorderStyle {
    match v {
        1 => BorderStyle::Dashed,
        2 => BorderStyle::Dotted,
        3 => BorderStyle::Medium,
        4 => BorderStyle::Thick,
        5 => BorderStyle::Double,
        _ => BorderStyle::Solid,
    }
}

/// Pack a number-format family discriminant (the [`CellFormat::Custom`]
/// pattern is stored out-of-band in `numfmt_{sheet}`).
pub(crate) fn pack_numfmt(f: &CellFormat) -> i64 {
    match f {
        CellFormat::General => 0,
        CellFormat::Number => 1,
        CellFormat::Currency => 2,
        CellFormat::Percent => 3,
        CellFormat::Scientific => 4,
        CellFormat::Fraction => 5,
        CellFormat::Date => 6,
        CellFormat::Time => 7,
        CellFormat::Text => 8,
        CellFormat::Custom(_) => 9,
    }
}
/// Inverse of [`pack_numfmt`]; `custom` supplies the pattern for the custom slot.
pub(crate) fn unpack_numfmt(v: i64, custom: Option<String>) -> CellFormat {
    match v {
        1 => CellFormat::Number,
        2 => CellFormat::Currency,
        3 => CellFormat::Percent,
        4 => CellFormat::Scientific,
        5 => CellFormat::Fraction,
        6 => CellFormat::Date,
        7 => CellFormat::Time,
        8 => CellFormat::Text,
        9 => CellFormat::Custom(custom.unwrap_or_default()),
        _ => CellFormat::General,
    }
}

/// Pack a [`CellStyle`]'s mask-resident fields (everything except colours and the
/// font family / custom pattern, which live in sibling maps) into one i64.
pub(crate) fn pack_style_mask(s: &CellStyle) -> i64 {
    let mut m = 0i64;
    if s.underline {
        m |= STYLE_UNDERLINE;
    }
    if s.strikethrough {
        m |= STYLE_STRIKE;
    }
    if s.wrap {
        m |= STYLE_WRAP;
    }
    m |= pack_halign(s.h_align) << STYLE_HALIGN_SHIFT;
    m |= pack_valign(s.v_align) << STYLE_VALIGN_SHIFT;
    pack_borders(&s.borders, &mut m);
    m |= pack_numfmt(&s.format) << STYLE_NUMFMT_SHIFT;
    if let Some(d) = s.decimals {
        m |= ((d as i64 + 1) << STYLE_DECIMALS_SHIFT) & STYLE_DECIMALS_MASK;
    }
    if let Some(sz) = s.size {
        m |= ((sz as i64).min(127) << STYLE_SIZE_SHIFT) & STYLE_SIZE_MASK;
    }
    m
}

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
    // `p.border` (the selection-level edit: preset + stroke style + colour) is
    // resolved into per-cell edges by `set_cell_style`, which has the selection
    // geometry; not handled here.
}

/// Pack a [`Borders`] into the style mask: a presence bit and a 3-bit stroke
/// style per edge, in the order top, right, bottom, left (colours live in the
/// per-edge `bordcolor{edge}_{sheet}` maps).
pub(crate) fn pack_borders(b: &Borders, m: &mut i64) {
    let edges = [b.top, b.right, b.bottom, b.left];
    for (i, edge) in edges.into_iter().enumerate() {
        if let Some(cb) = edge {
            *m |= 1 << (STYLE_BORDER_PRESENT_SHIFT + i as i64);
            *m |= pack_bordstyle(cb.style) << (STYLE_BORDER_STYLE_SHIFT + 3 * i as i64);
        }
    }
}

/// Unpack one edge (index `i` in top/right/bottom/left order) from the style
/// mask, pairing its stroke style with the `color` read from that edge's map.
pub(crate) fn unpack_border_edge(mask: i64, i: usize, color: Option<u32>) -> Option<CellBorder> {
    let present = mask & (1 << (STYLE_BORDER_PRESENT_SHIFT + i as i64)) != 0;
    present.then(|| CellBorder {
        style: unpack_bordstyle((mask >> (STYLE_BORDER_STYLE_SHIFT + 3 * i as i64)) & 0b111),
        color,
    })
}

/// Format a numeric `value` per a [`CellFormat`] and decimal count. Non-numeric
/// cells keep their text (handled by the caller); this is the numeric branch.
pub(crate) fn format_number(value: f64, fmt: &CellFormat, decimals: Option<u8>) -> String {
    let dec = |default: usize| decimals.map(|d| d as usize).unwrap_or(default);
    match fmt {
        CellFormat::Number => format!("{:.*}", dec(2), value),
        CellFormat::Currency => format!("{:.*}", dec(2), value) + " \u{20AC}",
        CellFormat::Percent => format!("{:.*}%", dec(0), value * 100.0),
        CellFormat::Scientific => format!("{:.*e}", dec(2), value),
        CellFormat::Fraction => {
            // Simple proper-fraction rendering over a /16 denominator (MVP).
            let whole = value.trunc() as i64;
            let frac = (value.fract().abs() * 16.0).round() as i64;
            if frac == 0 {
                whole.to_string()
            } else if whole == 0 {
                format!("{frac}/16")
            } else {
                format!("{whole} {frac}/16")
            }
        }
        // Date/Time MVP: render the day/second serial as an integer; a real
        // calendar conversion lands with the date engine (Phase 2).
        CellFormat::Date | CellFormat::Time => format!("{value}"),
        // General / Text / Custom leave the value untouched here.
        _ => format!("{value}"),
    }
}

/// Convert a 0-based column index to a spreadsheet column label (A, B, ..., Z,
/// AA, AB, ...).
///
/// col=0 -> "A", col=25 -> "Z", col=26 -> "AA", col=701 -> "ZZ".
pub(crate) fn col_to_alpha(mut col: u32) -> String {
    let mut result = String::new();
    loop {
        let rem = (col % 26) as u8;
        result.insert(0, (b'A' + rem) as char);
        if col < 26 {
            break;
        }
        col = col / 26 - 1;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_style_packs_to_zero_mask() {
        // The "mask == 0 means no style" contract in `persist_style` requires the
        // default of every mask-resident field to pack to 0.
        assert_eq!(pack_style_mask(&CellStyle::default()), 0);
    }

    #[test]
    fn top_valign_only_style_has_nonzero_mask_and_roundtrips() {
        // Regression: a Top-only style used to pack to 0 and get deleted from the
        // CRDT, reverting to Middle on reload.
        let style = CellStyle {
            v_align: VAlign::Top,
            ..Default::default()
        };
        let mask = pack_style_mask(&style);
        assert_ne!(mask, 0, "Top valign must survive the zero-mask cull");
        assert_eq!(
            unpack_valign((mask & STYLE_VALIGN_MASK) >> STYLE_VALIGN_SHIFT),
            VAlign::Top
        );
    }
}
