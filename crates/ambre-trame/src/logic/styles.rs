// ambre-trame/src/logic/styles.rs.rs
// Cell formatting: persist, reseed, set_cell_style
//
// Part of the `TrameState` impl, split out of the original logic.rs by concern.
// Submodule of `logic`, so it reaches the private fields/helpers of the parent
// module through the `use super::*` glob.

use super::*;
use serde::{Deserialize, Serialize};

/// A cell's whole visual style as one CRDT record. Bold/italic are not part of
/// [`CellStyle`] (the toolbar reports them separately, via their own sets), so
/// they ride here alongside it: the record is the *entire* style of a cell.
/// postcard-encoded into the single `cellstyle_{sheet}` register (see
/// [`cellstyle_map_name`]). Storing the whole style in one LWW register -- rather
/// than scattering it over ~11 per-attribute maps -- is the convergence
/// invariant: two peers that concurrently restyle the same cell race exactly
/// once, so the merged style is always one writer's, never a per-attribute blend
/// (`trame-schema`, `TRAME-016`).
#[derive(Serialize, Deserialize, Default)]
struct StyleRecord {
    bold: bool,
    italic: bool,
    style: CellStyle,
}

impl TrameState {
    /// Mirror a cell's whole style -- bold/italic plus its [`CellStyle`] -- into
    /// the single `cellstyle_{sheet}` register. A default (unstyled) cell is
    /// removed from the map so it never accumulates a dead entry. Does **not**
    /// commit or flush: every caller batches many cell writes (a style selection,
    /// a paste) and commits + flushes the whole group once, so the batch is a
    /// single undo item and a single broadcast delta. Committing here would both
    /// split the batch into one undo item per cell and, mid-batch, strip the
    /// intervening ops out of the broadcast delta (mirrors [`write_cell`]).
    pub(super) fn write_cell_style(&mut self, row: u32, col: u32) -> Option<()> {
        let pos = (row, col);
        let record = StyleRecord {
            bold: self.bold.contains(&pos),
            italic: self.italic.contains(&pos),
            style: self.styles.get(&pos).cloned().unwrap_or_default(),
        };
        // Styling a cell pins an identity to it, so it must be materialized just
        // like an edited value. Beyond the cap there is nothing to persist.
        let (rl, cl) = self.materialize(row, col)?;
        let key = cell_key(rl, cl);
        let map = self.doc.get_map(cellstyle_map_name(&self.active_sheet));
        if !record.bold && !record.italic && record.style == CellStyle::default() {
            let _ = map.delete(&key);
        } else {
            // postcard is deterministic, so both peers encode the same style to
            // the same bytes; the register is compared by CRDT metadata, not the
            // payload, so the encoding only has to round-trip.
            let bytes = postcard::to_allocvec(&record).unwrap_or_default();
            let _ = map.insert(&key, bytes);
        }
        Some(())
    }

    // -------------------------------------------------------------------------
    // Rich manual formatting (`trame-cell-styles`)
    // -------------------------------------------------------------------------

    /// Apply a [`StylePatch`] to every cell of the tracked selection, persist the
    /// merged styles in the CRDT, and report the active cell's resulting style so
    /// the toolbar reflects the selection. Bold/italic fold into their dedicated
    /// sets (and the `format_{sheet}` map); the rest into the cell's [`CellStyle`]
    /// (the `style_{sheet}` / colour / font containers).
    pub(super) fn set_cell_style(&mut self, patch: StylePatch) -> TrameOutcome {
        let (r1, c1, r2, c2) = self.selection;
        // One undo group + one commit for the whole selection, so the restyle is
        // a single undo step and a single broadcast delta (mirrors `fill_range` /
        // `SetCellsBatch`). Pre-materialize every targeted cell up front so the
        // excluded-origin `materialize` commits happen first; interleaving them
        // between the style writes would split the batch into one undo item per
        // cell. The style writes below never commit (`write_cell_style`), so they
        // collapse into the single trailing commit, and the group is sealed by the
        // next command's fallback `group_end`.
        let _ = self.undo_manager.group_start();
        for row in r1..=r2 {
            for col in c1..=c2 {
                if !self.covered.contains(&(row, col)) {
                    let _ = self.materialize(row, col);
                }
            }
        }
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
                // Bold/italic live in their own in-memory sets; they are folded
                // back into the single style record by `persist_cell_style` below.
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
                // Everything else lives in the cell's `CellStyle`.
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
                let _ = self.write_cell_style(row, col);
            }
        }
        // Single trailing commit for every style write in the selection -> one
        // undo item, one delta.
        self.doc.commit();
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

    /// Rebuild the in-memory bold/italic sets and [`Self::styles`] map from the
    /// active sheet's single `cellstyle_{sheet}` register. Called after any import
    /// so a reload or a peer's formatting edit is reflected. Requires
    /// [`Self::index`] to be current (the keys are stable id pairs, mapped back to
    /// grid positions).
    pub(super) fn reseed_styles(&mut self) {
        self.bold.clear();
        self.italic.clear();
        self.styles.clear();

        let LoroValue::Map(entries) = self
            .doc
            .get_map(cellstyle_map_name(&self.active_sheet))
            .get_value()
        else {
            return;
        };
        for (key, value) in entries.iter() {
            let (Some((rl, cl)), LoroValue::Binary(bytes)) = (parse_cell_key(key), value) else {
                continue;
            };
            let (Some(row), Some(col)) = (self.index.row_pos(&rl), self.index.col_pos(&cl)) else {
                continue; // references a deleted row/column
            };
            let Ok(record) = postcard::from_bytes::<StyleRecord>(bytes) else {
                continue; // unreadable payload: skip rather than crash the reseed
            };
            let cell = (row as u32, col as u32);
            if record.bold {
                self.bold.insert(cell);
            }
            if record.italic {
                self.italic.insert(cell);
            }
            // A default `CellStyle` is not tracked in `styles` (absence == default);
            // the bold/italic sets above still hold the rest of the record.
            if record.style != CellStyle::default() {
                self.styles.insert(cell, record.style);
            }
        }
    }

    /// Apply a cell's number format to its computed display `text`. A `General` /
    /// `Text` cell or a non-numeric value renders verbatim
    /// (`trame-number-formats`).
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
