// ambre-trame/src/logic/find.rs Find & Replace (`trame-find-replace`).
//
// The DOM `FindReplace` chrome panel (Leptos, in `trame_chrome`) emits two
// parameterless-origin commands. The Logic thread -- the sole holder of document
// data -- does the scanning and the rewrite here. `FindNext` is read-only and
// resolves its search origin from the tracked active cell (like the filter/sort
// intents). `ReplaceAll` mutates every match in a single undo group. Submodule of
// `logic`, so it reaches the parent's private fields/helpers through `use
// super::*`.

use super::*;
use crate::formula::types::CellId;

impl TrameState {
    /// Scan the active sheet for cells whose **displayed** value matches `query`
    /// and reply with a [`TrameEvent::FindResult`] (`trame-find-replace`).
    /// Read-only: no CRDT
    /// delta. `matches` lists every hit in row-major order; `active` is the index
    /// of the first hit strictly after the tracked active cell, wrapping to the
    /// top. The UI moves the active cell onto that hit, so a repeated `FindNext`
    /// advances naturally. An empty `query` clears the highlight (no matches).
    pub(super) fn find_next(
        &mut self,
        query: String,
        match_case: bool,
        whole_cell: bool,
    ) -> TrameOutcome {
        if query.is_empty() {
            return TrameOutcome::display_only(vec![TrameEvent::FindResult {
                matches: Vec::new(),
                active: None,
            }]);
        }
        let needle = fold(&query, match_case);
        let (rows, cols) = (self.index.row_len() as u32, self.index.col_len() as u32);
        let mut matches: Vec<(u32, u32)> = Vec::new();
        for row in 0..rows {
            for col in 0..cols {
                // The displayed value, format included (`TRAME-UI-078`).
                let text = self.displayed_text(row, col);
                if !text.is_empty() && cell_matches(&text, &needle, match_case, whole_cell) {
                    matches.push((row, col));
                }
            }
        }
        // Pick the first hit strictly after the active cell (row-major), else wrap
        // to the first hit overall.
        let (ar, ac) = self.active;
        let active = if matches.is_empty() {
            None
        } else {
            let after = matches.iter().position(|&(r, c)| (r, c) > (ar, ac));
            Some(after.unwrap_or(0))
        };
        TrameOutcome::display_only(vec![TrameEvent::FindResult { matches, active }])
    }

    /// Replace every match of `query` with `replacement` across the active sheet
    /// (or all sheets when `all_sheets`) in one undo group (`trame-find-replace`).
    /// A cell matches on its **displayed** value (`whole_cell` = exact equality,
    /// else substring; `match_case` toggles folding). The rewrite edits the
    /// **raw** cell text: the whole cell for `whole_cell`, else each occurrence of
    /// `query` in the raw string. Cells whose raw text is a formula (`=...`) are
    /// skipped unless `in_formulas`. Replies with [`TrameEvent::ReplaceDone`] and
    /// a [`TrameEvent::DataReset`] so the UI re-fetches.
    pub(super) fn replace_all(
        &mut self,
        query: String,
        replacement: String,
        match_case: bool,
        whole_cell: bool,
        in_formulas: bool,
        all_sheets: bool,
    ) -> TrameOutcome {
        if query.is_empty() {
            return TrameOutcome::display_only(vec![TrameEvent::ReplaceDone { replaced: 0 }]);
        }
        let needle = fold(&query, match_case);

        // One undo group for the whole replace (sealed by the next command's
        // fallback `group_end`), mirroring `fill_range`.
        let _ = self.undo_manager.group_start();

        // Active sheet first, through the regular write path (engine-aware).
        let mut replaced = self.replace_in_active(
            &needle,
            &query,
            &replacement,
            match_case,
            whole_cell,
            in_formulas,
        );

        // Other sheets: edit their cells map directly, then reseed the engine so
        // their formula results recompute. The active write path only tracks the
        // active sheet's engine state.
        let mut touched_other = false;
        if all_sheets {
            let others: Vec<String> = self
                .all_sheet_ids()
                .into_iter()
                .filter(|id| *id != self.active_sheet)
                .collect();
            for sheet_id in others {
                let n = self.replace_in_other(
                    &sheet_id,
                    &needle,
                    &query,
                    &replacement,
                    match_case,
                    whole_cell,
                    in_formulas,
                );
                if n > 0 {
                    touched_other = true;
                    replaced += n;
                }
            }
        }
        if touched_other {
            // Direct map writes bypassed the engine; rebuild it from the CRDT so
            // every sheet's formula results are correct before the flush.
            self.reseed_engine();
        }

        let mut outcome = self.finish_cell_writes();
        outcome.display.push(TrameEvent::ReplaceDone { replaced });
        outcome
    }

    /// Replace the single match at the active cell, then reply with a fresh
    /// `FindResult` advancing to the following match (`trame-find-replace`). The
    /// active cell is
    /// rewritten only if it matches `query`; either way the UI moves to the next
    /// match. One undo group.
    pub(super) fn replace_one(
        &mut self,
        query: String,
        replacement: String,
        match_case: bool,
        whole_cell: bool,
        in_formulas: bool,
    ) -> TrameOutcome {
        if query.is_empty() {
            return TrameOutcome::display_only(vec![TrameEvent::ReplaceDone { replaced: 0 }]);
        }
        let needle = fold(&query, match_case);
        let (r, c) = self.active;

        let _ = self.undo_manager.group_start();
        let replaced = u32::from(self.replace_active_cell(
            r,
            c,
            &needle,
            &query,
            &replacement,
            match_case,
            whole_cell,
            in_formulas,
        ));

        // Recompute the match set so the UI can advance to (and highlight) the
        // next hit. After a successful replace the active cell no longer matches,
        // so `find_next` naturally lands on the following one.
        let next = self.find_next(query, match_case, whole_cell);

        if replaced == 0 {
            // Nothing changed: just forward the advance (no commit needed).
            let mut display = next.display;
            display.push(TrameEvent::ReplaceDone { replaced });
            return TrameOutcome::display_only(display);
        }
        let mut outcome = self.finish_cell_writes();
        outcome.display.extend(next.display);
        outcome.display.push(TrameEvent::ReplaceDone { replaced });
        outcome
    }

    /// Replace on the active sheet via the engine-aware [`Self::write_cell`] path.
    /// Returns the number of cells actually rewritten.
    #[allow(clippy::too_many_arguments)]
    fn replace_in_active(
        &mut self,
        needle: &str,
        query: &str,
        replacement: &str,
        match_case: bool,
        whole_cell: bool,
        in_formulas: bool,
    ) -> u32 {
        let (rows, cols) = (self.index.row_len() as u32, self.index.col_len() as u32);
        let mut replaced = 0;
        for row in 0..rows {
            for col in 0..cols {
                if self.replace_active_cell(
                    row,
                    col,
                    needle,
                    query,
                    replacement,
                    match_case,
                    whole_cell,
                    in_formulas,
                ) {
                    replaced += 1;
                }
            }
        }
        replaced
    }

    /// Test and rewrite a single active-sheet cell. A formula cell is eligible
    /// only with `in_formulas`, and then it matches/rewrites on its raw formula
    /// text ("search within formulas"). A plain cell matches on its displayed
    /// value. Returns whether the cell was actually rewritten.
    #[allow(clippy::too_many_arguments)]
    fn replace_active_cell(
        &mut self,
        row: u32,
        col: u32,
        needle: &str,
        query: &str,
        replacement: &str,
        match_case: bool,
        whole_cell: bool,
        in_formulas: bool,
    ) -> bool {
        let raw = self.cell_raw(row, col);
        if raw.is_empty() {
            return false;
        }
        let is_formula = raw.starts_with('=');
        // A formula is searched and rewritten in its positional source, the
        // text the user reads (`TRAME-228`); the write binds it again.
        let raw = if is_formula {
            let active = self.active_sheet.clone();
            self.formula_source(&raw, &active, &self.sheet_lines())
        } else {
            raw
        };
        if is_formula && !in_formulas {
            return false;
        }
        let haystack = if is_formula {
            raw.clone()
        } else {
            self.displayed_text(row, col)
        };
        if !cell_matches(&haystack, needle, match_case, whole_cell) {
            return false;
        }
        let new_raw = rewrite(&raw, query, replacement, match_case, whole_cell);
        if new_raw == raw {
            return false;
        }
        self.write_cell(row, col, &new_raw);
        true
    }

    /// Replace on a non-active sheet by mutating its cells map directly (no engine
    /// update -- the caller reseeds afterwards). Matching uses the engine's cached
    /// evaluation for that sheet, consistent with the active path.
    #[allow(clippy::too_many_arguments)]
    fn replace_in_other(
        &mut self,
        sheet_id: &str,
        needle: &str,
        query: &str,
        replacement: &str,
        match_case: bool,
        whole_cell: bool,
        in_formulas: bool,
    ) -> u32 {
        let Some(&idx) = self.sheet_id_to_u16.get(sheet_id) else {
            return 0;
        };
        let index = self.build_sheet_index(sheet_id);
        let (rows, cols) = (index.row_len(), index.col_len());
        let map = self.doc.get_map(cells_map_name(sheet_id));
        let mut edits: Vec<((Uid, Uid), Option<String>)> = Vec::new();
        let mut refusals = Vec::new();
        let lines = self.sheet_lines();
        for row in 0..rows {
            for col in 0..cols {
                let Some((rl, cl)) = index.resolve(row, col) else {
                    continue;
                };
                let raw = match map.get(&cell_key(rl, cl)).map(|v| v.get_deep_value()) {
                    Some(LoroValue::String(s)) => s.to_string(),
                    _ => continue,
                };
                let is_formula = raw.starts_with('=');
                if is_formula && !in_formulas {
                    continue;
                }
                let raw = if is_formula {
                    self.formula_source(&raw, sheet_id, &lines)
                } else {
                    raw
                };
                // Formula cells (with `in_formulas`) match on their raw formula
                // text; plain cells match on the engine's displayed value.
                let haystack = if is_formula {
                    raw.clone()
                } else {
                    let cell = CellId::bound(idx, rl, cl);
                    let text = if self.engine.is_formula_cell(cell) {
                        self.engine.evaluated_text(cell).unwrap_or_default()
                    } else {
                        raw.clone()
                    };
                    // The displayed value, format included (`TRAME-UI-078`).
                    match self.stored_style(sheet_id, rl, cl) {
                        Some(style) => self.format_display(0, 0, text, &style),
                        None => text,
                    }
                };
                if !cell_matches(&haystack, needle, match_case, whole_cell) {
                    continue;
                }
                let new_raw = rewrite(&raw, query, replacement, match_case, whole_cell);
                if new_raw != raw {
                    let stored = if new_raw.is_empty() {
                        None
                    } else {
                        match self.canonicalize_input(&new_raw, sheet_id, &lines) {
                            Ok(stored) => Some(stored),
                            Err(sheet) => {
                                refusals.push(WriteRefusal::UnknownSheet(sheet));
                                continue;
                            }
                        }
                    };
                    edits.push(((rl, cl), stored));
                }
            }
        }
        drop(lines);
        self.write_refusals.extend(refusals);
        let count = edits.len() as u32;
        for ((rl, cl), stored) in edits {
            match stored {
                Some(text) => {
                    let _ = map.insert(&cell_key(rl, cl), text.as_str());
                }
                None => {
                    let _ = map.delete(&cell_key(rl, cl));
                }
            }
        }
        count
    }
}

/// Case-fold `s` for comparison: lowercased when `match_case` is off, verbatim
/// otherwise.
fn fold(s: &str, match_case: bool) -> String {
    if match_case {
        s.to_string()
    } else {
        s.to_lowercase()
    }
}

/// Whether `haystack` matches the already-folded `needle`: exact equality for a
/// whole-cell match, else substring containment.
fn cell_matches(haystack: &str, needle: &str, match_case: bool, whole_cell: bool) -> bool {
    let hay = fold(haystack, match_case);
    if whole_cell {
        hay == needle
    } else {
        hay.contains(needle)
    }
}

/// Produce the rewritten raw text for a matched cell: the whole-cell case swaps
/// the entire content for `replacement`. The substring case replaces every
/// occurrence of `query` (case-insensitively when `match_case` is off).
fn rewrite(
    raw: &str,
    query: &str,
    replacement: &str,
    match_case: bool,
    whole_cell: bool,
) -> String {
    if whole_cell {
        return replacement.to_string();
    }
    if match_case {
        raw.replace(query, replacement)
    } else {
        replace_ci(raw, query, replacement)
    }
}

/// Byte length, measured in `hay`, of a case-insensitive match of `needle_lower`
/// anchored at the start of `hay`. `None` when `hay` does not start with it.
///
/// Lowercasing is compared character by character rather than on a pre-folded
/// copy of `hay`, because `to_lowercase` is not length-preserving. `I` (U+0130)
/// grows from 2 bytes to 3, the Kelvin sign `K` (U+212A) shrinks from 3 to 1. An
/// offset found in the folded text therefore does not address the original.
fn ci_match_len(hay: &str, needle_lower: &str) -> Option<usize> {
    let mut needle = needle_lower.chars();
    let mut pending = needle.next();
    for (offset, ch) in hay.char_indices() {
        if pending.is_none() {
            return Some(offset);
        }
        // One source char can lowercase to several; all of them must line up, so
        // a needle ending mid-expansion is not a match.
        for lower in ch.to_lowercase() {
            match pending {
                Some(want) if want == lower => pending = needle.next(),
                _ => return None,
            }
        }
    }
    pending.is_none().then_some(hay.len())
}

/// Case-insensitive substring replace: replace every occurrence of `query` in
/// `raw` (compared case-folded) with `replacement`, preserving the surrounding
/// text verbatim. `query` is assumed non-empty.
fn replace_ci(raw: &str, query: &str, replacement: &str) -> String {
    let needle = query.to_lowercase();
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while !rest.is_empty() {
        // A non-empty needle always consumes at least one char, so `rest` shrinks
        // on every iteration whichever branch is taken.
        match ci_match_len(rest, &needle) {
            Some(len) if len > 0 => {
                out.push_str(replacement);
                rest = &rest[len..];
            }
            _ => {
                let ch = rest.chars().next().expect("rest is not empty");
                out.push(ch);
                rest = &rest[ch.len_utf8()..];
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{replace_ci, rewrite};

    #[test]
    fn case_insensitive_replace_survives_a_non_length_preserving_lowercase() {
        // `to_lowercase` grows U+0130 (2 bytes -> 3) and shrinks the Kelvin sign
        // U+212A (3 -> 1). Offsets taken from the folded copy used to index the
        // original string, which panicked the Logic worker. That was out of bounds
        // on the first and off a char boundary on the second, and it killed the
        // session.
        assert_eq!(replace_ci("\u{0130}x", "x", "Z"), "\u{0130}Z");
        assert_eq!(replace_ci("\u{212A}elvin", "elvin", "Z"), "\u{212A}Z");
        // The match may also start on the offending char itself.
        assert_eq!(replace_ci("a\u{212A}b", "k", "Z"), "aZb");
    }

    #[test]
    fn case_insensitive_replace_handles_the_ordinary_cases() {
        assert_eq!(replace_ci("Hello hello", "HELLO", "hi"), "hi hi");
        assert_eq!(replace_ci("aaa", "aa", "b"), "ba");
        assert_eq!(replace_ci("abc", "z", "y"), "abc");
        assert_eq!(replace_ci("abc", "abc", ""), "");
        // A replacement containing the query must not be rescanned.
        assert_eq!(replace_ci("a", "a", "aa"), "aa");
    }

    #[test]
    fn whole_cell_rewrite_swaps_the_entire_content() {
        assert_eq!(rewrite("=A1+1", "x", "y", false, true), "y");
        assert_eq!(rewrite("Xx", "x", "y", true, false), "Xy");
    }
}
