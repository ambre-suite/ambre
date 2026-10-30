// ambre-trame/src/logic/styles.rs.rs
// Cell formatting: persist, reseed, set_cell_style
//
// Part of the `TrameState` impl, split out of the original logic.rs by concern.
// Submodule of `logic`, so it reaches the private fields/helpers of the parent
// module through the `use super::*` glob.

use super::*;
use serde::{Deserialize, Serialize};

/// A cell's whole visual style as one CRDT record. Bold/italic are not part of
/// [`CellStyle`]: the toolbar reports them separately, via their own sets. They
/// ride here alongside it, so the record is the *entire* style of a cell.
/// postcard-encoded into the single `cellstyle_{sheet}` register (see
/// [`SheetRoot::CellStyle`]). Storing the whole style in one LWW register, rather
/// than scattering it over ~11 per-attribute maps, is the convergence invariant
/// (`trame-schema`, `TRAME-016`). Two peers that concurrently restyle the same
/// cell race exactly once, so the merged style is always one writer's, never a
/// per-attribute blend.
#[derive(Serialize, Deserialize, Default)]
pub(super) struct StyleRecord {
    pub(super) bold: ambre_bus::Weight,
    pub(super) italic: ambre_bus::Slant,
    pub(super) style: CellStyle,
}

impl TrameState {
    /// Mirror a cell's whole style -- bold/italic plus its [`CellStyle`] -- into
    /// the single `cellstyle_{sheet}` register. A default (unstyled) cell is
    /// removed from the map so it never accumulates a dead entry. Does **not**
    /// commit or flush: every caller batches many cell writes (a style selection,
    /// a paste) and commits + flushes the whole group once. The batch is then a
    /// single undo item and a single broadcast delta. Committing here would split
    /// the batch into one undo item per cell. It would also, mid-batch, strip the
    /// intervening ops out of the broadcast delta (mirrors [`write_cell`]).
    pub(super) fn write_cell_style(&mut self, pos: CellPos) -> Option<()> {
        let record = StyleRecord {
            bold: ambre_bus::Weight::from_bool(self.bold.contains(&pos)),
            italic: ambre_bus::Slant::from_bool(self.italic.contains(&pos)),
            style: self.styles.get(&pos).cloned().unwrap_or_default(),
        };
        // Styling a cell pins an identity to it, so it must be materialized just
        // like an edited value. Beyond the cap there is nothing to persist.
        let (rl, cl) = self.materialize(pos).ok()?;
        let key = CellKey::of(rl, cl);
        let map = self
            .doc
            .get_map(SheetContainer::of(SheetRoot::CellStyle, &self.active_sheet).as_str());
        if record.bold == ambre_bus::Weight::Regular
            && record.italic == ambre_bus::Slant::Upright
            && record.style == CellStyle::default()
        {
            let _ = map.delete(key.as_str());
        } else {
            // postcard is deterministic, so both peers encode the same style to
            // the same bytes. The register is compared by CRDT metadata, not the
            // payload, so the encoding only has to round-trip.
            let bytes = postcard::to_allocvec(&record).unwrap_or_default();
            let _ = map.insert(key.as_str(), bytes);
        }
        Some(())
    }

    // -------------------------------------------------------------------------
    // Rich manual formatting (`trame-cell-styles`)
    // -------------------------------------------------------------------------

    /// Apply a [`StylePatch`] to every cell of the tracked selection, then persist
    /// the merged styles in the CRDT. Reports the active cell's resulting style so
    /// the toolbar reflects the selection. Bold/italic fold into their dedicated
    /// sets (and the `format_{sheet}` map); the rest into the cell's [`CellStyle`]
    /// (the `style_{sheet}` / colour / font containers).
    pub(super) fn set_cell_style(&mut self, patch: StylePatch) -> TrameOutcome {
        // A gesture that changes no attribute writes nothing: no line, no undo
        // item, no delta (`TRAME-189`).
        if patch == StylePatch::default() {
            return TrameOutcome::display_only(vec![]);
        }
        // A size outside the cell range is refused whole, as a spreadsheet
        // refuses a typed one (`CANVAS-088`).
        if let Some(Some(size)) = patch.size
            && !(ambre_bus::app::trame::CELL_FONT_SIZE_MIN
                ..=ambre_bus::app::trame::CELL_FONT_SIZE_MAX)
                .contains(&size)
        {
            return TrameOutcome::display_only(vec![TrameEvent::Error {
                message: "a cell font size lies between 1 and 409 points".to_owned(),
            }]);
        }
        // Bounding box for the border perimeter, and the deduplicated cell set for
        // the writes (`TRAME-UI-129`): every rectangle the user selected.
        let bounds = self.selection_bounds();
        let cells = self.selected_cells();
        // One undo group + one commit for the whole selection. The restyle is then
        // a single undo step and a single broadcast delta (mirrors `fill_range` /
        // `SetCellsBatch`). Pre-materialize every targeted cell up front so the
        // excluded-origin `materialize` commits happen first. Interleaving them
        // between the style writes would split the batch into one undo item per
        // cell. The style writes below never commit (`write_cell_style`), so they
        // collapse into the single trailing commit. The group is sealed by the
        // next command's fallback `group_end`.
        let _ = self.undo_manager.group_start();
        // A cell with no line to key its style by is refused whole, never
        // drawn with a style the document does not hold (`TRAME-193`). It lies
        // past the materialization cap, or the session is out of fresh uids.
        let mut refused: HashMap<CellPos, Unmaterialized> = HashMap::new();
        for &cell in &cells {
            if self.covered.contains(&cell) {
                continue;
            }
            if let Err(why) = self.materialize(cell) {
                refused.insert(cell, why);
            }
        }
        for &cell in &cells {
            // Cells covered by a merge are swallowed by their master: they
            // render blank, so styling them is pointless. For `All` borders
            // it is actively wrong, since it would draw interior edges
            // inside the merged block. The master carries the whole span's
            // style.
            if self.covered.contains(&cell) || refused.contains_key(&cell) {
                continue;
            }
            // Bold/italic live in their own in-memory sets; they are folded
            // back into the single style record by `persist_cell_style` below.
            if let Some(b) = patch.bold {
                if b == Weight::Bold {
                    self.bold.insert(cell);
                } else {
                    self.bold.remove(&cell);
                }
            }
            if let Some(i) = patch.italic {
                if i == Slant::Italic {
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
                let end = self.merges.get(&cell).copied().unwrap_or(cell);
                let (first, last) = (bounds.top_left, bounds.bottom_right);
                // Perimeter membership within the selection rectangle.
                let top_p = cell.row == first.row;
                let bottom_p = end.row == last.row;
                let left_p = cell.col == first.col;
                let right_p = end.col == last.col;
                // Interior membership (edge shared with an adjacent cell).
                let top_i = cell.row > first.row;
                let bottom_i = end.row < last.row;
                let left_i = cell.col > first.col;
                let right_i = end.col < last.col;
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
            let _ = self.write_cell_style(cell);
        }
        // Single trailing commit for every style write in the selection -> one
        // undo item, one delta.
        self.doc.commit();
        let update = self.flush_delta();
        // The active cell's resolved state drives the toolbar; DataReset re-fetches
        // the restyled viewport (number-format reformat, colours, borders).
        let mut display = vec![self.style_state_event(), TrameEvent::DataReset];
        let beyond_cap = refused
            .values()
            .filter(|why| **why == Unmaterialized::BeyondCap)
            .count();
        if beyond_cap > 0 {
            display.push(TrameEvent::Error {
                message: format!(
                    "{beyond_cap} cell(s) lie beyond the sheet's {}-line limit and were not formatted",
                    MATERIALIZE_ROWS.get()
                ),
            });
        }
        for why in [Unmaterialized::IdsExhausted, Unmaterialized::Unwritten] {
            if refused.values().any(|w| *w == why) {
                display.push(why.notice());
            }
        }
        TrameOutcome {
            display,
            update,
            ephemeral: None,
        }
    }

    /// Whether the cell at `at` computes to a number, before its format
    /// renders it. The one test behind `HAlign::Auto`, for the grid and the
    /// toolbar alike (`TRAME-UI-093`, `TRAME-UI-094`, `TRAME-186`).
    pub(super) fn cell_is_numeric(&self, at: CellPos) -> bool {
        matches!(
            self.engine.evaluated_value(self.active_cell_id(at)),
            Some(CellValue::Number(n)) if n.is_finite()
        )
    }

    /// The master of the merged block covering `cell`, or `cell` itself when no
    /// block covers it.
    fn merge_master(&self, cell: CellPos) -> CellPos {
        if !self.covered.contains(&cell) {
            return cell;
        }
        self.merges
            .iter()
            .find(|&(&master, &end)| CellRect::spanning(master, end).contains(cell))
            .map_or(cell, |(&master, _)| master)
    }

    /// The active cell's resolved formatting, for the toolbar controls. Inside a
    /// merged block that is the block's style, the one the user sees, which its
    /// master carries (`TRAME-188`).
    pub(super) fn style_state_event(&self) -> TrameEvent {
        let shown = self.merge_master(self.active);
        TrameEvent::StyleApplied {
            style: self.styles.get(&shown).cloned().unwrap_or_default(),
            bold: Weight::from_bool(self.bold.contains(&shown)),
            italic: Slant::from_bool(self.italic.contains(&shown)),
            merged: Merge::from_bool(
                self.merges.contains_key(&self.active) || self.covered.contains(&self.active),
            ),
            // The test the viewport pack hands the renderer (`TRAME-UI-094`).
            numeric: CellValueKind::from_bool(self.cell_is_numeric(shown)),
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
            .get_map(SheetContainer::of(SheetRoot::CellStyle, &self.active_sheet).as_str())
            .get_value()
        else {
            return;
        };
        for (key, value) in entries.iter() {
            let Some((rl, cl)) = parse_cell_key(key) else {
                // A key no build writes is said, never read in silence
                // (`TRAME-248`).
                self.note_unreadable_key();
                continue;
            };
            let LoroValue::Binary(bytes) = value else {
                continue;
            };
            let Some(cell) = self.index.pos_of(&rl, &cl) else {
                continue; // references a deleted row/column
            };
            let Ok(record) = postcard::from_bytes::<StyleRecord>(bytes) else {
                // Skip rather than crash the reseed, but say so. A cell that
                // silently loses its formatting is a workbook that looks intact
                // and is not (`SCHEMA-003`).
                self.note_undecodable(bytes);
                continue;
            };
            if record.bold == Weight::Bold {
                self.bold.insert(cell);
            }
            if record.italic == Slant::Italic {
                self.italic.insert(cell);
            }
            // A default `CellStyle` is not tracked in `styles` (absence == default);
            // the bold/italic sets above still hold the rest of the record.
            if record.style != CellStyle::default() {
                self.styles.insert(cell, record.style);
            }
        }
    }

    /// The text a cell of the active sheet shows: its computed text through its
    /// number format. Find, replace and export read this, so a cell reads the
    /// same wherever it is shown as text (`TRAME-108`).
    pub(super) fn displayed_text(&self, at: CellPos) -> String {
        let text = self.cell_text(at);
        match self.styles.get(&at) {
            Some(style) => self.format_display(at, text, style),
            None => text,
        }
    }

    /// The style `cellstyle_{sheet}` stores for a cell of any sheet, or `None`
    /// for the default. An undecodable record reads as the default here: the
    /// reseed of that sheet is what reports it (`SCHEMA-003`).
    pub(super) fn stored_style(&self, sheet_id: &SheetId, rl: Uid, cl: Uid) -> Option<CellStyle> {
        match self
            .doc
            .get_map(SheetContainer::of(SheetRoot::CellStyle, sheet_id).as_str())
            .get(CellKey::of(rl, cl).as_str())
            .map(|v| v.get_deep_value())
        {
            Some(LoroValue::Binary(bytes)) => postcard::from_bytes::<StyleRecord>(&bytes)
                .ok()
                .map(|record| record.style),
            _ => None,
        }
    }

    /// Apply a cell's number format to its computed display `text`. A `General` /
    /// `Text` cell or a non-numeric value renders verbatim
    /// (`trame-number-formats`).
    pub(super) fn format_display(&self, _at: CellPos, text: String, style: &CellStyle) -> String {
        match style.format {
            CellFormat::General | CellFormat::Text => text,
            ref fmt => match crate::formula::types::finite_number(&text) {
                Some(v) => format_number(v, fmt, style.decimals),
                None => text,
            },
        }
    }
}
