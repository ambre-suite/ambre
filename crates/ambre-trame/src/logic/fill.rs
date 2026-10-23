// ambre-trame/src/logic/fill.rs Autofill / fill handle (`trame-autofill`).
//
// The UI drags the fill handle from the selection's bottom-right corner and
// emits `FillRange { src, dest, mode }`. `dest` contains `src` and extends it
// along a single axis (the handle locks to one direction). The Logic thread is
// the only holder of document data, so the fill happens here. That covers
// numeric series extrapolation, source tiling, relative-formula reindexing and
// style carry. Submodule of `logic`, so it reaches the parent's private
// fields/helpers through `use super::*`.

use super::*;
use crate::formula::shift::shift_formula;
use ambre_bus::FillMode;

/// A cell's or a tile's place in its fill line's series, counted from the
/// source origin. Signed: a fill runs either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Term(pub(super) i64);

impl Term {
    /// The item of `items` this term copies, cycling through them.
    fn pick<T>(self, items: &[T]) -> Option<&T> {
        let len = i64::try_from(items.len()).ok().filter(|&n| n > 0)?;
        let at = usize::try_from(self.0.rem_euclid(len)).ok()?;
        items.get(at)
    }
}

/// One tile of a merged fill along one axis (`TRAME-UI-134`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Tile<P: Line> {
    /// The tile's signed block offset from the source.
    pub(super) term: Term,
    /// The tile's first line.
    pub(super) first: P,
    /// How many lines the tile covers, cut at the drag edge.
    pub(super) len: P::Count,
}

impl TrameState {
    /// Autofill `src` into `dest` (`trame-autofill`). `dest` contains `src`. The
    /// cells of `dest \ src` are filled by extrapolating a numeric series, or by
    /// tiling/copying the source values per fill line. A fill line is a column
    /// for a vertical fill, a row for a horizontal one.
    ///
    /// A copied formula's relative references reindex by its tile offset; every
    /// reference is relative, as AMBRE has no `$` syntax. Each source cell's
    /// style is carried verbatim. The whole fill is a single undo group. Returns
    /// a `DataReset` so the UI re-fetches the filled region, plus the CRDT delta.
    pub(super) fn fill_range(
        &mut self,
        sheet_id: &SheetId,
        src: CellRect,
        dest: CellRect,
        mode: FillMode,
    ) -> TrameOutcome {
        // Range ops only ever target the active sheet (the UI activates it first).
        if *sheet_id != self.active_sheet {
            return TrameOutcome::display_only(vec![]);
        }
        // `dest` must contain `src`; otherwise the drag is malformed -> no-op.
        if !dest.contains(src.top_left) || !dest.contains(src.bottom_right) {
            return TrameOutcome::display_only(vec![]);
        }
        let rows_extended = dest.rows() != src.rows();
        let cols_extended = dest.cols() != src.cols();
        if !rows_extended && !cols_extended {
            // No extension: nothing to fill.
            return TrameOutcome::display_only(vec![]);
        }

        // One undo group for the whole fill (sealed by the next command's fallback
        // `group_end`), mirroring `paste_range`.
        let _ = self.undo_manager.group_start();
        // Materialize every destination cell up front (`TRAME-UI-073`). Each
        // `materialize` is its own excluded-origin commit, and one between two
        // value writes splits the fill into one undo item per cell. Done first,
        // the value writes land in one trailing commit, as `paste_range` does.
        for at in dest.cells().filter(|&at| !src.contains(at)) {
            let _ = self.materialize(at);
        }

        // A source tiled by equal merged blocks (one block or a uniform grid of
        // them) fills block by block, not cell by cell. Every tile is re-merged
        // (Excel behaviour, `trame-autofill`).
        if let Some(block) = self.merged_source_block(src) {
            return self.fill_merged(src, dest, mode, block);
        }

        // The fill handle locks to one axis; prefer the row axis when both look
        // extended (a diagonal drag), matching the handle's bottom-right anchor.
        if rows_extended {
            self.fill_vertical(src, dest, mode);
        } else {
            self.fill_horizontal(src, dest, mode);
        }

        self.finish_cell_writes()
    }

    /// Return the block extent when `src` is exactly tiled by equal merged
    /// blocks (`trame-autofill`). That means one merge, or a uniform grid of
    /// identically sized merges, each with its master at the tile corner. `None`
    /// for a plain selection or a mix of sizes, which then falls back to the
    /// per-cell fill.
    fn merged_source_block(&self, src: CellRect) -> Option<MergeSpan> {
        let end = self.merges.get(&src.top_left).copied()?;
        let block = CellRect::spanning(src.top_left, end).extent();
        let (bh, bw) = (block.rows.get(), block.cols.get());
        if !src.row_count().get().is_multiple_of(bh) || !src.col_count().get().is_multiple_of(bw) {
            return None;
        }
        for top in src.rows().into_iter().step_by(bh as usize) {
            for left in src.cols().into_iter().step_by(bw as usize) {
                let master = CellPos::new(top, left);
                if self.merges.get(&master).copied() != Some(block.last_from(master)) {
                    return None;
                }
            }
        }
        Some(block)
    }

    /// Autofill a merged source (one block or a uniform grid of `block`-sized
    /// blocks) over `dest` (`trame-autofill`). Tiles the blocks along the drag
    /// axis and re-merges every tile to the block span. Each lane perpendicular
    /// to the drag extrapolates independently from its own source blocks: a
    /// numeric series progresses, otherwise the source blocks are copied
    /// cyclically. A copied formula's relative refs reindex by the tile offset.
    /// Tiles align on the source, and a tile the drag edge cuts is written to
    /// that edge (`TRAME-UI-134`); source tiles are skipped.
    fn fill_merged(
        &mut self,
        src: CellRect,
        dest: CellRect,
        mode: FillMode,
        block: MergeSpan,
    ) -> TrameOutcome {
        let (bh, bw) = (block.rows.get() as usize, block.cols.get() as usize);
        if dest.rows() != src.rows() {
            // Each block column is a lane; fill it from its own stack of masters.
            for left in src.cols().into_iter().step_by(bw) {
                let masters: Vec<CellPos> = src
                    .rows()
                    .into_iter()
                    .step_by(bh)
                    .map(|top| CellPos::new(top, left))
                    .collect();
                let sources: Vec<String> = masters.iter().map(|&m| self.cell_source(m)).collect();
                let series = series_params(mode, &sources);
                let tiles = merged_tiles(
                    src.top_left.row,
                    src.bottom_right.row,
                    dest.top_left.row,
                    dest.bottom_right.row,
                    block.rows,
                );
                for tile in tiles {
                    let master = CellPos::new(tile.first, left);
                    let extent = MergeSpan::new(tile.len, block.cols);
                    let rect = CellRect::spanning(master, extent.last_from(master));
                    let picked = (tile.term.pick(&masters), tile.term.pick(&sources));
                    if let (Some(&src_master), Some(src_raw)) = picked {
                        self.fill_merged_tile(rect, tile.term, src_raw, src_master, series);
                    }
                }
            }
        } else {
            // Each block row is a lane; fill it from its own row of masters.
            for top in src.rows().into_iter().step_by(bh) {
                let masters: Vec<CellPos> = src
                    .cols()
                    .into_iter()
                    .step_by(bw)
                    .map(|left| CellPos::new(top, left))
                    .collect();
                let sources: Vec<String> = masters.iter().map(|&m| self.cell_source(m)).collect();
                let series = series_params(mode, &sources);
                let tiles = merged_tiles(
                    src.top_left.col,
                    src.bottom_right.col,
                    dest.top_left.col,
                    dest.bottom_right.col,
                    block.cols,
                );
                for tile in tiles {
                    let master = CellPos::new(top, tile.first);
                    let extent = MergeSpan::new(block.rows, tile.len);
                    let rect = CellRect::spanning(master, extent.last_from(master));
                    let picked = (tile.term.pick(&masters), tile.term.pick(&sources));
                    if let (Some(&src_master), Some(src_raw)) = picked {
                        self.fill_merged_tile(rect, tile.term, src_raw, src_master, series);
                    }
                }
            }
        }
        // Rebuild the derived merge caches over the new entries and let masked /
        // unmasked cells rejoin the engine before the standard write tail flushes.
        self.rebuild_merges();
        self.reseed_engine();
        self.finish_cell_writes()
    }

    /// Write and merge one autofill `tile`. `term` is the tile's signed block
    /// offset from the source along the drag axis (`0` = first source block).
    /// `src_raw` / `src_master` are the source block this tile copies (cyclic
    /// for a multi-block source). The covered cells are cleared so the merge is
    /// accepted. The master gets the series/copied value and the source block's
    /// style, and a merge entry is recorded for the tile.
    fn fill_merged_tile(
        &mut self,
        tile: CellRect,
        term: Term,
        src_raw: &str,
        src_master: CellPos,
        series: Option<(f64, f64)>,
    ) {
        let master = tile.top_left;
        let text = if let Some((first, step)) = series {
            format_number(first + term.0 as f64 * step)
        } else if src_raw.starts_with('=') {
            shift_formula(
                src_raw,
                master.row - src_master.row,
                master.col - src_master.col,
            )
        } else {
            src_raw.to_string()
        };
        // Clear the covered cells first: merge_cells/rebuild only accept an empty
        // block, and a previous value there would otherwise be masked.
        for at in tile.cells().filter(|&at| at != master) {
            self.write_cell(at, "");
        }
        self.write_cell(master, &text);
        // Carry the source master's formatting onto the tile master.
        self.carry_style(src_master, master);
        // A tile cut down to one cell by the drag edge is a plain cell.
        if !tile.is_single_cell() {
            self.insert_merge_entry(tile);
        }
    }

    /// Fill the rows of `dest` outside `src` for every source column,
    /// independently per column (each column is one fill line along the row axis).
    fn fill_vertical(&mut self, src: CellRect, dest: CellRect, mode: FillMode) {
        let origin = src.top_left.row;
        for col in src.cols() {
            let sources: Vec<String> = src
                .rows()
                .into_iter()
                .map(|r| self.cell_source(CellPos::new(r, col)))
                .collect();
            let series = series_params(mode, &sources);
            for row in dest.rows() {
                if src.rows().contains(row) {
                    continue; // inside the source block -- left untouched.
                }
                // Signed: fill may go up or down.
                let term = Term((row - origin).get());
                let k = term.0.rem_euclid(sources.len().max(1) as i64) as u32;
                let from = CellPos::new(origin + RowCount::new(k), col);
                self.fill_one(CellPos::new(row, col), from, term, &sources, series);
            }
        }
    }

    /// Fill the columns of `dest` outside `src` for every source row.
    fn fill_horizontal(&mut self, src: CellRect, dest: CellRect, mode: FillMode) {
        let origin = src.top_left.col;
        for row in src.rows() {
            let sources: Vec<String> = src
                .cols()
                .into_iter()
                .map(|c| self.cell_source(CellPos::new(row, c)))
                .collect();
            let series = series_params(mode, &sources);
            for col in dest.cols() {
                if src.cols().contains(col) {
                    continue;
                }
                let term = Term((col - origin).get());
                let k = term.0.rem_euclid(sources.len().max(1) as i64) as u32;
                let from = CellPos::new(row, origin + ColCount::new(k));
                self.fill_one(CellPos::new(row, col), from, term, &sources, series);
            }
        }
    }

    /// Write one filled cell at `at`. `term` is its signed position along the
    /// fill axis relative to the source origin (`0` = first source cell).
    /// `sources` are the source line's raw strings. With a detected `series`
    /// the cell gets the extrapolated number. Otherwise the source value at the
    /// tiled position is copied (formulas reindexed by the tile offset). The
    /// tiled source cell `from` supplies the carried style in both cases.
    fn fill_one(
        &mut self,
        at: CellPos,
        from: CellPos,
        term: Term,
        sources: &[String],
        series: Option<(f64, f64)>,
    ) {
        let _ = self.materialize(at);

        let text = if let Some((first, step)) = series {
            format_number(first + term.0 as f64 * step)
        } else {
            // Tiled source index: wrap into the source line so a fill longer
            // than the source repeats the pattern (an upward fill stays in range).
            let k = term.0.rem_euclid(sources.len().max(1) as i64) as usize;
            let raw = sources.get(k).map(String::as_str).unwrap_or_default();
            if raw.starts_with('=') {
                shift_formula(raw, at.row - from.row, at.col - from.col)
            } else {
                raw.to_owned()
            }
        };
        self.write_cell(at, &text);
        self.carry_style(from, at);
    }

    /// Carry the formatting of `from` (bold/italic + rich style) onto `to`.
    fn carry_style(&mut self, from: CellPos, to: CellPos) {
        let cell = ClipboardCell {
            bold: self.bold.contains(&from),
            italic: self.italic.contains(&from),
            style: self.styles.get(&from).cloned().unwrap_or_default(),
            ..Default::default()
        };
        self.apply_pasted_style(to, &cell);
    }
}

/// The tiles a merged fill writes along one axis (`TRAME-UI-134`), in order.
/// The source runs `s1..=s2`, the drag `d1..=d2`, in blocks of `block` lines.
///
/// Tiles sit on the source's own block grid, so an upward fill never lands on
/// the source. A tile the destination edge cuts is kept, cut to that edge, so
/// every line between the source and the edge is written. Source tiles are
/// left out.
pub(super) fn merged_tiles<P: Line>(s1: P, s2: P, d1: P, d2: P, block: P::Count) -> Vec<Tile<P>> {
    let mut tiles = Vec::new();
    // Before the source, walking back from it one block at a time.
    // A tile that would start before the axis is cut at its first line.
    let mut term = -1;
    let mut next = s1;
    while let Some(last) = next.before() {
        if last < d1 {
            break;
        }
        let first = next.back(block);
        let lo = first.unwrap_or(P::FIRST).max(d1);
        tiles.push(Tile {
            term: Term(term),
            first: lo,
            len: last.count_from(lo) + P::ONE,
        });
        let Some(first) = first else { break };
        next = first;
        term -= 1;
    }
    tiles.reverse();
    // From the source on, walking forward; the source's own tiles are skipped.
    let mut term = 0;
    let mut first = s1;
    while first <= d2 {
        if first > s2 {
            let last = first.forward(block).before().unwrap_or(first).min(d2);
            tiles.push(Tile {
                term: Term(term),
                first,
                len: last.count_from(first) + P::ONE,
            });
        }
        let next = first.forward(block);
        if next <= first {
            break;
        }
        first = next;
        term += 1;
    }
    tiles
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
