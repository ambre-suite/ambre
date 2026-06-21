// ambre-trame/src/logic/styles.rs.rs
// Cell formatting: persist, reseed, set_cell_style
//
// Part of the `TrameState` impl, split out of the original logic.rs by concern.
// Submodule of `logic`, so it reaches the private fields/helpers of the parent
// module through the `use super::*` glob.

use super::*;

impl TrameState {
    /// Mirror a cell's current bold/italic state into the CRDT `format` map and
    /// return the incremental delta to broadcast (and persist). A cell with no
    /// formatting is removed from the map so it does not accumulate dead entries.
    pub(super) fn persist_format(&mut self, row: u32, col: u32) -> Option<Vec<u8>> {
        let mut mask = 0i64;
        if self.bold.contains(&(row, col)) {
            mask |= FMT_BOLD;
        }
        if self.italic.contains(&(row, col)) {
            mask |= FMT_ITALIC;
        }
        // Formatting a cell pins an identity to it, so it must be materialized
        // just like an edited value. Beyond the cap there is nothing to persist.
        let (rl, cl) = self.materialize(row, col)?;
        let map = self.doc.get_map(format_map_name(&self.active_sheet));
        let key = cell_key(rl, cl);
        if mask == 0 {
            let _ = map.delete(&key);
        } else {
            let _ = map.insert(&key, mask);
        }
        self.doc.commit();
        self.flush_delta()
    }

    /// Rebuild the in-memory bold/italic sets from the active sheet's
    /// `format_{sheet}` map. Called after any import so a reload or a peer's
    /// formatting edit is reflected. Requires [`Self::index`] to be current
    /// (the keys are stable id pairs, mapped back to grid positions).
    pub(super) fn reseed_formatting(&mut self) {
        self.bold.clear();
        self.italic.clear();
        if let LoroValue::Map(fmts) = self
            .doc
            .get_map(format_map_name(&self.active_sheet))
            .get_value()
        {
            for (key, value) in fmts.iter() {
                let (Some((rl, cl)), LoroValue::I64(mask)) = (parse_cell_key(key), value) else {
                    continue;
                };
                let (Some(row), Some(col)) = (self.index.row_pos(&rl), self.index.col_pos(&cl))
                else {
                    continue; // references a deleted row/column
                };
                let cell = (row as u32, col as u32);
                if mask & FMT_BOLD != 0 {
                    self.bold.insert(cell);
                }
                if mask & FMT_ITALIC != 0 {
                    self.italic.insert(cell);
                }
            }
        }
    }

    // -------------------------------------------------------------------------
    // Rich manual formatting (`-ui.md` sec.5bis)
    // -------------------------------------------------------------------------

    /// Apply a [`StylePatch`] to every cell of the tracked selection, persist the
    /// merged styles in the CRDT, and report the active cell's resulting style so
    /// the toolbar reflects the selection. Bold/italic fold into their dedicated
    /// sets (and the `format_{sheet}` map); the rest into the cell's [`CellStyle`]
    /// (the `style_{sheet}` / colour / font containers).
    pub(super) fn set_cell_style(&mut self, patch: StylePatch) -> TrameOutcome {
        let (r1, c1, r2, c2) = self.selection;
        for row in r1..=r2 {
            for col in c1..=c2 {
                let cell = (row, col);
                // Cells covered by a merge are swallowed by their master: they
                // render blank, so styling them is pointless and -- for `All`
                // borders -- actively wrong (it would draw interior edges inside
                // the merged block). The master carries the whole span's style.
                if self.covered.contains(&cell) {
                    continue;
                }
                // Bold/italic live in their own sets + the format map.
                if let Some(b) = patch.bold {
                    if b {
                        self.bold.insert(cell);
                    } else {
                        self.bold.remove(&cell);
                    }
                }
                if let Some(i) = patch.italic {
                    if i {
                        self.italic.insert(cell);
                    } else {
                        self.italic.remove(&cell);
                    }
                }
                if patch.bold.is_some() || patch.italic.is_some() {
                    let _ = self.persist_format(row, col);
                }
                // Everything else lives in the rich-style containers.
                let mut style = self.styles.get(&cell).cloned().unwrap_or_default();
                apply_patch_to_style(&mut style, &patch);
                // Resolve the selection-level border edit into this cell's edges.
                // Merge-aware: a master's perimeter follows its full span, and
                // interior edges fall between adjacent selected cells. Every
                // preset is additive (touches only its own edges) except `None`,
                // which clears all four.
                if let Some(edit) = patch.border {
                    let (br, bc) = self.merges.get(&cell).copied().unwrap_or((row, col));
                    // Perimeter membership within the selection rectangle.
                    let top_p = row == r1;
                    let bottom_p = br == r2;
                    let left_p = col == c1;
                    let right_p = bc == c2;
                    // Interior membership (edge shared with an adjacent cell).
                    let top_i = row > r1;
                    let bottom_i = br < r2;
                    let left_i = col > c1;
                    let right_i = bc < c2;
                    // Targeted edges in [top, right, bottom, left] order.
                    let targeted = match edit.preset {
                        BorderPreset::None => [true; 4],
                        BorderPreset::All => [true; 4],
                        BorderPreset::Outline => [top_p, right_p, bottom_p, left_p],
                        BorderPreset::Inner => [top_i, right_i, bottom_i, left_i],
                        BorderPreset::InnerHorizontal => [top_i, false, bottom_i, false],
                        BorderPreset::InnerVertical => [false, right_i, false, left_i],
                        BorderPreset::Top => [top_p, false, false, false],
                        BorderPreset::Bottom => [false, false, bottom_p, false],
                        BorderPreset::Left => [false, false, false, left_p],
                        BorderPreset::Right => [false, right_p, false, false],
                    };
                    let new_edge = (edit.preset != BorderPreset::None).then_some(CellBorder {
                        style: edit.style,
                        color: edit.color,
                    });
                    let slots = [
                        &mut style.borders.top,
                        &mut style.borders.right,
                        &mut style.borders.bottom,
                        &mut style.borders.left,
                    ];
                    for (slot, &hit) in slots.into_iter().zip(targeted.iter()) {
                        if hit {
                            *slot = new_edge;
                        }
                    }
                }
                if style == CellStyle::default() {
                    self.styles.remove(&cell);
                } else {
                    self.styles.insert(cell, style.clone());
                }
                let _ = self.persist_style(row, col, &style);
            }
        }
        let update = self.flush_delta();
        // The active cell's resolved state drives the toolbar; DataReset re-fetches
        // the restyled viewport (number-format reformat, colours, borders).
        TrameOutcome {
            display: vec![self.style_state_event(), TrameEvent::DataReset],
            update,
            ephemeral: None,
        }
    }

    /// The active cell's resolved formatting, for the toolbar controls.
    pub(super) fn style_state_event(&self) -> TrameEvent {
        TrameEvent::StyleApplied {
            style: self.styles.get(&self.active).cloned().unwrap_or_default(),
            bold: self.bold.contains(&self.active),
            italic: self.italic.contains(&self.active),
            merged: self.merges.contains_key(&self.active) || self.covered.contains(&self.active),
            // Same numeric heuristic the renderer uses to resolve `HAlign::Auto`
            // (see `render::content::cell_alignment`): a value that parses as a
            // number aligns right, everything else left.
            numeric: self
                .cell_text(self.active.0, self.active.1)
                .parse::<f64>()
                .is_ok(),
        }
    }

    /// Mirror a cell's [`CellStyle`] into the CRDT style containers and return the
    /// incremental delta. A default style removes every entry so the maps never
    /// accumulate dead keys. Commits but does not flush (the caller batches the
    /// flush over the whole selection).
    pub(super) fn persist_style(&mut self, row: u32, col: u32, style: &CellStyle) -> Option<()> {
        let (rl, cl) = self.materialize(row, col)?;
        let key = cell_key(rl, cl);
        let sheet = self.active_sheet.clone();

        let mask = pack_style_mask(style);
        let map = self.doc.get_map(style_map_name(&sheet));
        if mask == 0 {
            let _ = map.delete(&key);
        } else {
            let _ = map.insert(&key, mask);
        }

        let put_color = |doc: &LoroDoc, name: String, c: Option<u32>| {
            let m = doc.get_map(name);
            match c {
                Some(c) => {
                    let _ = m.insert(&key, c as i64);
                }
                None => {
                    let _ = m.delete(&key);
                }
            }
        };
        put_color(&self.doc, fgcolor_map_name(&sheet), style.fg);
        put_color(&self.doc, bgcolor_map_name(&sheet), style.bg);
        // One colour map per edge (top, right, bottom, left); each present edge
        // stores its own colour, absent edges delete their entry.
        let edges = [
            style.borders.top,
            style.borders.right,
            style.borders.bottom,
            style.borders.left,
        ];
        for (i, edge) in edges.into_iter().enumerate() {
            put_color(
                &self.doc,
                bordcolor_map_name(&sheet, i),
                edge.and_then(|e| e.color),
            );
        }

        let font_map = self.doc.get_map(font_map_name(&sheet));
        match &style.font {
            Some(f) => {
                let _ = font_map.insert(&key, f.as_str());
            }
            None => {
                let _ = font_map.delete(&key);
            }
        }

        let numfmt_map = self.doc.get_map(numfmt_map_name(&sheet));
        match &style.format {
            CellFormat::Custom(pat) => {
                let _ = numfmt_map.insert(&key, pat.as_str());
            }
            _ => {
                let _ = numfmt_map.delete(&key);
            }
        }

        self.doc.commit();
        Some(())
    }

    /// Rebuild the in-memory [`Self::styles`] map from the active sheet's style
    /// containers. Called on import alongside [`Self::reseed_formatting`].
    pub(super) fn reseed_styles(&mut self) {
        self.styles.clear();
        let sheet = self.active_sheet.clone();

        // Read the sibling colour / font / custom-pattern maps once, keyed by the
        // raw cell-key string (the mask map drives which cells exist).
        let read_i64_map = |doc: &LoroDoc, name: String| -> HashMap<String, i64> {
            let mut out = HashMap::new();
            if let LoroValue::Map(m) = doc.get_map(name).get_value() {
                for (k, v) in m.iter() {
                    if let LoroValue::I64(n) = v {
                        out.insert(k.to_string(), *n);
                    }
                }
            }
            out
        };
        let read_str_map = |doc: &LoroDoc, name: String| -> HashMap<String, String> {
            let mut out = HashMap::new();
            if let LoroValue::Map(m) = doc.get_map(name).get_value() {
                for (k, v) in m.iter() {
                    if let LoroValue::String(s) = v {
                        out.insert(k.to_string(), s.to_string());
                    }
                }
            }
            out
        };
        let fg = read_i64_map(&self.doc, fgcolor_map_name(&sheet));
        let bg = read_i64_map(&self.doc, bgcolor_map_name(&sheet));
        let bordc: [HashMap<String, i64>; BORDER_EDGE_ORDER] =
            std::array::from_fn(|e| read_i64_map(&self.doc, bordcolor_map_name(&sheet, e)));
        let fonts = read_str_map(&self.doc, font_map_name(&sheet));
        let customs = read_str_map(&self.doc, numfmt_map_name(&sheet));

        if let LoroValue::Map(masks) = self.doc.get_map(style_map_name(&sheet)).get_value() {
            for (key, value) in masks.iter() {
                let (Some((rl, cl)), LoroValue::I64(mask)) = (parse_cell_key(key), value) else {
                    continue;
                };
                let (Some(row), Some(col)) = (self.index.row_pos(&rl), self.index.col_pos(&cl))
                else {
                    continue; // references a deleted row/column
                };
                let key_s = key.as_str();
                let decimals_raw = (mask & STYLE_DECIMALS_MASK) >> STYLE_DECIMALS_SHIFT;
                let size_raw = (mask & STYLE_SIZE_MASK) >> STYLE_SIZE_SHIFT;
                let style = CellStyle {
                    underline: mask & STYLE_UNDERLINE != 0,
                    strikethrough: mask & STYLE_STRIKE != 0,
                    wrap: mask & STYLE_WRAP != 0,
                    h_align: unpack_halign((mask & STYLE_HALIGN_MASK) >> STYLE_HALIGN_SHIFT),
                    v_align: unpack_valign((mask & STYLE_VALIGN_MASK) >> STYLE_VALIGN_SHIFT),
                    borders: {
                        let edge = |i: usize| {
                            unpack_border_edge(*mask, i, bordc[i].get(key_s).map(|&c| c as u32))
                        };
                        Borders {
                            top: edge(0),
                            right: edge(1),
                            bottom: edge(2),
                            left: edge(3),
                        }
                    },
                    format: unpack_numfmt(
                        (mask & STYLE_NUMFMT_MASK) >> STYLE_NUMFMT_SHIFT,
                        customs.get(key_s).cloned(),
                    ),
                    decimals: (decimals_raw != 0).then(|| (decimals_raw - 1) as u8),
                    size: (size_raw != 0).then_some(size_raw as u16),
                    fg: fg.get(key_s).map(|&c| c as u32),
                    bg: bg.get(key_s).map(|&c| c as u32),
                    font: fonts.get(key_s).cloned(),
                };
                self.styles.insert((row as u32, col as u32), style);
            }
        }
    }

    /// Apply a cell's number format to its computed display `text`. A `General` /
    /// `Text` cell or a non-numeric value renders verbatim (`-ui.md` sec.5bis).
    pub(super) fn format_display(
        &self,
        _row: u32,
        _col: u32,
        text: String,
        style: &CellStyle,
    ) -> String {
        match style.format {
            CellFormat::General | CellFormat::Text | CellFormat::Custom(_) => text,
            ref fmt => match text.parse::<f64>() {
                Ok(v) => format_number(v, fmt, style.decimals),
                Err(_) => text,
            },
        }
    }
}
