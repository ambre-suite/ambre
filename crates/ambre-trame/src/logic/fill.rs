// ambre-trame/src/logic/fill.rs
// Autofill / fill handle (`ambre-spec-trame-ui.md` sec.5.F).
//
// The UI drags the fill handle from the selection's bottom-right corner and
// emits `FillRange { src, dest, mode }`; `dest` contains `src` and extends it
// along a single axis (the handle locks to one direction). The Logic thread is
// the only holder of document data, so the fill -- including numeric series
// extrapolation, source tiling, relative-formula reindexing and style carry --
// happens here. Submodule of `logic`, so it reaches the parent's private
// fields/helpers through `use super::*`.

use super::*;
use crate::formula::shift::shift_formula;
use ambre_bus::{FillMode, GridRange};

impl TrameState {
    /// Autofill `src` into `dest` (`-ui.md` sec.5.F). `dest` contains `src`; the
    /// cells of `dest \ src` are filled by extrapolating a numeric series or by
    /// tiling/copying the source values per fill line (a column for a vertical
    /// fill, a row for a horizontal one). A copied formula's relative references
    /// reindex by its tile offset (every reference is relative -- AMBRE has no `$`
    /// syntax), and each source cell's style is carried verbatim. The whole fill
    /// is a single undo group. Returns a `DataReset` so the UI re-fetches the
    /// filled region, plus the CRDT delta.
    pub(super) fn fill_range(
        &mut self,
        sheet_id: &str,
        src: GridRange,
        dest: GridRange,
        mode: FillMode,
    ) -> TrameOutcome {
        // Range ops only ever target the active sheet (the UI activates it first).
        if sheet_id != self.active_sheet {
            return TrameOutcome::display_only(vec![]);
        }
        let (sr1, sc1, sr2, sc2) = normalize(src);
        let (dr1, dc1, dr2, dc2) = normalize(dest);
        // `dest` must contain `src`; otherwise the drag is malformed -> no-op.
        if dr1 > sr1 || dc1 > sc1 || dr2 < sr2 || dc2 < sc2 {
            return TrameOutcome::display_only(vec![]);
        }
        let rows_extended = dr1 < sr1 || dr2 > sr2;
        let cols_extended = dc1 < sc1 || dc2 > sc2;
        if !rows_extended && !cols_extended {
            // No extension: nothing to fill.
            return TrameOutcome::display_only(vec![]);
        }

        // One undo group for the whole fill (sealed by the next command's fallback
        // `group_end`), mirroring `paste_range`.
        let _ = self.undo_manager.group_start();

        // The fill handle locks to one axis; prefer the row axis when both look
        // extended (a diagonal drag), matching the handle's bottom-right anchor.
        if rows_extended {
            self.fill_vertical(sr1, sr2, sc1, sc2, dr1, dr2, mode);
        } else {
            self.fill_horizontal(sr1, sr2, sc1, sc2, dc1, dc2, mode);
        }

        self.finish_cell_writes()
    }

    /// Fill the rows of `[dr1..=dr2] \ [sr1..=sr2]` for every source column,
    /// independently per column (each column is one fill line along the row axis).
    #[allow(clippy::too_many_arguments)]
    fn fill_vertical(
        &mut self,
        sr1: u32,
        sr2: u32,
        sc1: u32,
        sc2: u32,
        dr1: u32,
        dr2: u32,
        mode: FillMode,
    ) {
        let span = (sr2 - sr1 + 1) as i64;
        for col in sc1..=sc2 {
            let sources: Vec<String> = (sr1..=sr2).map(|r| self.cell_raw(r, col)).collect();
            let series = series_params(mode, &sources);
            for row in dr1..=dr2 {
                if row >= sr1 && row <= sr2 {
                    continue; // inside the source block -- left untouched.
                }
                let idx = row as i64 - sr1 as i64; // signed: fill may go up or down.
                self.fill_one(row, col, idx, span, sr1, col, &sources, series);
            }
        }
    }

    /// Fill the columns of `[dc1..=dc2] \ [sc1..=sc2]` for every source row.
    #[allow(clippy::too_many_arguments)]
    fn fill_horizontal(
        &mut self,
        sr1: u32,
        sr2: u32,
        sc1: u32,
        sc2: u32,
        dc1: u32,
        dc2: u32,
        mode: FillMode,
    ) {
        let span = (sc2 - sc1 + 1) as i64;
        for row in sr1..=sr2 {
            let sources: Vec<String> = (sc1..=sc2).map(|c| self.cell_raw(row, c)).collect();
            let series = series_params(mode, &sources);
            for col in dc1..=dc2 {
                if col >= sc1 && col <= sc2 {
                    continue;
                }
                let idx = col as i64 - sc1 as i64;
                self.fill_one(row, col, idx, span, row, sc1, &sources, series);
            }
        }
    }

    /// Write one filled cell at `(row, col)`. `idx` is its signed position along
    /// the fill axis relative to the source origin (`0` = first source cell);
    /// `span` is the source line length; `sources` are the source line's raw
    /// strings. With a detected `series` the cell gets the extrapolated number;
    /// otherwise the source value at the tiled position is copied (formulas
    /// reindexed by the tile offset). The tiled source cell -- at `(src_row,
    /// src_col)` resolved below -- supplies the carried style in both cases.
    #[allow(clippy::too_many_arguments)]
    fn fill_one(
        &mut self,
        row: u32,
        col: u32,
        idx: i64,
        span: i64,
        line_row: u32,
        line_col: u32,
        sources: &[String],
        series: Option<(f64, f64)>,
    ) {
        // Tiled source index: wrap into `[0, span)` so a fill longer than the
        // source repeats the pattern (and an upward fill stays in range).
        let k = idx.rem_euclid(span) as usize;
        // The tiled source cell's grid position, for the style carry and the
        // copy-mode formula reindex offset.
        let (src_row, src_col) = if line_col == col {
            (line_row + k as u32, col) // vertical fill: source varies by row.
        } else {
            (row, line_col + k as u32) // horizontal fill: source varies by col.
        };
        let _ = self.materialize(row, col);

        let text = if let Some((first, step)) = series {
            format_number(first + idx as f64 * step)
        } else {
            let raw = &sources[k];
            if raw.starts_with('=') {
                shift_formula(
                    raw,
                    row as i64 - src_row as i64,
                    col as i64 - src_col as i64,
                )
            } else {
                raw.clone()
            }
        };
        self.write_cell(row, col, &text);

        // Carry the tiled source cell's formatting (bold/italic + rich style).
        let cell = ClipboardCell {
            bold: self.bold.contains(&(src_row, src_col)),
            italic: self.italic.contains(&(src_row, src_col)),
            style: self
                .styles
                .get(&(src_row, src_col))
                .cloned()
                .unwrap_or_default(),
            ..Default::default()
        };
        self.apply_pasted_style(row, col, &cell);
    }
}

/// Normalize a [`GridRange`] (corners in any order) to `(r1, c1, r2, c2)` with
/// `start <= end`.
fn normalize(g: GridRange) -> (u32, u32, u32, u32) {
    (
        g.row.min(g.row_end),
        g.col.min(g.col_end),
        g.row.max(g.row_end),
        g.col.max(g.col_end),
    )
}

/// Decide whether a fill line extrapolates a numeric arithmetic series, and with
/// what `(first, step)`. `first` corresponds to the source origin (index 0).
///
/// - [`FillMode::Copy`]: never a series.
/// - All-numeric source with >= 2 values forming an arithmetic progression: that
///   progression (this is the [`FillMode::Auto`] trigger).
/// - [`FillMode::Series`] with a single numeric source: step `1` (Excel/Sheets
///   "Fill Series" on a lone number).
/// - Anything else: `None` (the line is copied).
fn series_params(mode: FillMode, sources: &[String]) -> Option<(f64, f64)> {
    if mode == FillMode::Copy {
        return None;
    }
    // Every source cell must be a plain numeric literal (a formula or text breaks
    // the series and the line falls back to copy/reindex).
    let nums: Vec<f64> = sources
        .iter()
        .map(|s| parse_num(s))
        .collect::<Option<_>>()?;
    match nums.as_slice() {
        [] => None,
        [only] => (mode == FillMode::Series).then_some((*only, 1.0)),
        [first, second, rest @ ..] => {
            let step = second - first;
            let arithmetic = rest
                .iter()
                .zip(nums.iter().skip(1))
                .all(|(v, prev)| (v - prev - step).abs() < 1e-9);
            arithmetic.then_some((*first, step))
        }
    }
}

/// Parse a plain numeric literal, or `None` for empty / non-numeric / formula
/// cells (which cannot participate in a numeric series).
fn parse_num(raw: &str) -> Option<f64> {
    let t = raw.trim();
    if t.is_empty() || t.starts_with('=') {
        return None;
    }
    t.parse::<f64>().ok()
}

/// Render an extrapolated series value as cell text: an integral value drops the
/// decimals (`3.0 -> "3"`), otherwise Rust's shortest round-trip form is used.
fn format_number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}
