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

/// Whether a search tells upper case from lower case.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CaseMatch {
    /// Both sides are lowercased before they are compared.
    #[default]
    Insensitive,
    Sensitive,
}

/// How much of a cell a match must cover.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MatchExtent {
    /// The query may sit anywhere in the cell, and each occurrence is replaced.
    #[default]
    Substring,
    /// The query must equal the cell, and a replace swaps the whole content.
    WholeCell,
}

/// Which text of a formula cell a replace may read and rewrite.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SearchScope {
    /// Formula cells are skipped, and a plain cell matches on its value.
    #[default]
    Values,
    /// A formula cell also matches, on its positional source text.
    Formulas,
}

/// Which sheets a replace-all walks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SheetScope {
    #[default]
    Active,
    All,
}

/// The options of a find or a replace (`trame-find-replace`). A find reads
/// only `case` and `extent`, and only a replace-all reads `sheets`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FindOptions {
    pub case: CaseMatch,
    pub extent: MatchExtent,
    pub scope: SearchScope,
    pub sheets: SheetScope,
}

impl TrameState {
    /// Scan the active sheet for cells whose **displayed** value matches `query`
    /// and reply with a [`TrameEvent::FindResult`] (`trame-find-replace`).
    /// Read-only: the receiver is shared, so the search cannot write the
    /// document (`FIND-001`). `matches` lists every hit of the sheet in
    /// row-major order. `active` is the index of the first hit strictly after
    /// the tracked active cell, wrapping to the top. The UI moves the active
    /// cell onto that hit, so a repeated `FindNext` advances naturally. An
    /// empty `query` clears the highlight (no matches).
    pub(super) fn find_next(&self, query: String, options: FindOptions) -> TrameOutcome {
        if query.is_empty() {
            return TrameOutcome::display_only(vec![TrameEvent::FindResult {
                matches: Vec::new(),
                active: None,
            }]);
        }
        let needle = fold(&query, options.case);
        let mut matches: Vec<CellPos> = Vec::new();
        for at in self
            .index
            .used_range()
            .into_iter()
            .flat_map(CellRect::cells)
        {
            // The displayed value, format included (`TRAME-UI-078`).
            let text = self.displayed_text(at);
            if !text.is_empty() && cell_matches(&text, &needle, options) {
                matches.push(at);
            }
        }
        // Pick the first hit strictly after the active cell (row-major), else wrap
        // to the first hit overall.
        let active = if matches.is_empty() {
            None
        } else {
            let after = matches.iter().position(|&at| at > self.active);
            Some(after.unwrap_or(0))
        };
        TrameOutcome::display_only(vec![TrameEvent::FindResult { matches, active }])
    }

    /// Replace every match of `query` with `replacement` across the active sheet
    /// (or every sheet for [`SheetScope::All`]) in one undo group
    /// (`trame-find-replace`). A cell matches on its **displayed** value, by the
    /// extent and case of `options`. The rewrite edits the **raw** cell text: the
    /// whole cell for [`MatchExtent::WholeCell`], else each occurrence of `query`
    /// in the raw string. Cells whose raw text is a formula (`=...`) are skipped
    /// unless the scope is [`SearchScope::Formulas`]. Replies with [`TrameEvent::ReplaceDone`] and
    /// a [`TrameEvent::DataReset`] so the UI re-fetches.
    pub(super) fn replace_all(
        &mut self,
        query: String,
        replacement: String,
        options: FindOptions,
    ) -> TrameOutcome {
        if query.is_empty() {
            return TrameOutcome::display_only(vec![TrameEvent::ReplaceDone { replaced: 0 }]);
        }
        let needle = fold(&query, options.case);

        // One undo group for the whole replace (sealed by the next command's
        // fallback `group_end`), mirroring `fill_range`.
        let _ = self.undo_manager.group_start();

        // Active sheet first, through the regular write path (engine-aware).
        let mut replaced = self.replace_in_active(&needle, &query, &replacement, options);

        // Other sheets: edit their cells map directly, then reseed the engine so
        // their formula results recompute. The active write path only tracks the
        // active sheet's engine state.
        let mut touched_other = false;
        if options.sheets == SheetScope::All {
            let others: Vec<String> = self
                .all_sheet_ids()
                .into_iter()
                .filter(|id| *id != self.active_sheet)
                .collect();
            for sheet_id in others {
                let n = self.replace_in_other(&sheet_id, &needle, &query, &replacement, options);
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
        options: FindOptions,
    ) -> TrameOutcome {
        if query.is_empty() {
            return TrameOutcome::display_only(vec![TrameEvent::ReplaceDone { replaced: 0 }]);
        }
        let needle = fold(&query, options.case);
        let at = self.active;

        let _ = self.undo_manager.group_start();
        let replaced =
            u32::from(self.replace_active_cell(at, &needle, &query, &replacement, options));

        // Recompute the match set so the UI can advance to (and highlight) the
        // next hit. After a successful replace the active cell no longer matches,
        // so `find_next` naturally lands on the following one.
        let next = self.find_next(query, options);

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
    fn replace_in_active(
        &mut self,
        needle: &str,
        query: &str,
        replacement: &str,
        options: FindOptions,
    ) -> u32 {
        let mut replaced = 0;
        for at in self
            .index
            .used_range()
            .into_iter()
            .flat_map(CellRect::cells)
        {
            if self.replace_active_cell(at, needle, query, replacement, options) {
                replaced += 1;
            }
        }
        replaced
    }

    /// Test and rewrite a single active-sheet cell. A formula cell is eligible
    /// only in the [`SearchScope::Formulas`] scope, and then it matches/rewrites on its raw formula
    /// text ("search within formulas"). A plain cell matches on its displayed
    /// value. Returns whether the cell was actually rewritten.
    fn replace_active_cell(
        &mut self,
        at: CellPos,
        needle: &str,
        query: &str,
        replacement: &str,
        options: FindOptions,
    ) -> bool {
        let raw = self.cell_raw(at);
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
        if is_formula && options.scope == SearchScope::Values {
            return false;
        }
        let haystack = if is_formula {
            raw.clone()
        } else {
            self.displayed_text(at)
        };
        if !cell_matches(&haystack, needle, options) {
            return false;
        }
        let new_raw = rewrite(&raw, query, replacement, options);
        if new_raw == raw {
            return false;
        }
        self.write_cell(at, &new_raw);
        true
    }

    /// Replace on a non-active sheet by mutating its cells map directly (no engine
    /// update -- the caller reseeds afterwards). Matching uses the engine's cached
    /// evaluation for that sheet, consistent with the active path.
    fn replace_in_other(
        &mut self,
        sheet_id: &str,
        needle: &str,
        query: &str,
        replacement: &str,
        options: FindOptions,
    ) -> u32 {
        let Some(&idx) = self.engine_sheet_by_id.get(sheet_id) else {
            return 0;
        };
        let index = self.build_sheet_index(sheet_id);
        let map = self.doc.get_map(cells_map_name(sheet_id));
        let mut edits: Vec<((Uid, Uid), Option<String>)> = Vec::new();
        let mut refusals = Vec::new();
        let lines = self.sheet_lines();
        for at in index.used_range().into_iter().flat_map(CellRect::cells) {
            let Some((rl, cl)) = index.resolve(at) else {
                continue;
            };
            let raw = match map.get(&cell_key(rl, cl)).map(|v| v.get_deep_value()) {
                Some(LoroValue::String(s)) => s.to_string(),
                _ => continue,
            };
            let is_formula = raw.starts_with('=');
            if is_formula && options.scope == SearchScope::Values {
                continue;
            }
            let raw = if is_formula {
                self.formula_source(&raw, sheet_id, &lines)
            } else {
                raw
            };
            // Formula cells (in the formulas scope) match on their raw formula
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
                    Some(style) => self.format_display(at, text, &style),
                    None => text,
                }
            };
            if !cell_matches(&haystack, needle, options) {
                continue;
            }
            let new_raw = rewrite(&raw, query, replacement, options);
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

/// Case-fold `s` for comparison: lowercased for an insensitive match, verbatim
/// otherwise.
fn fold(s: &str, case: CaseMatch) -> String {
    match case {
        CaseMatch::Sensitive => s.to_string(),
        CaseMatch::Insensitive => s.to_lowercase(),
    }
}

/// Whether `haystack` matches the already-folded `needle`: exact equality for a
/// whole-cell match, else substring containment.
fn cell_matches(haystack: &str, needle: &str, options: FindOptions) -> bool {
    let hay = fold(haystack, options.case);
    match options.extent {
        MatchExtent::WholeCell => hay == needle,
        MatchExtent::Substring => hay.contains(needle),
    }
}

/// Produce the rewritten raw text for a matched cell: the whole-cell case swaps
/// the entire content for `replacement`. The substring case replaces every
/// occurrence of `query`, case-insensitively unless the case is sensitive.
fn rewrite(raw: &str, query: &str, replacement: &str, options: FindOptions) -> String {
    if options.extent == MatchExtent::WholeCell {
        return replacement.to_string();
    }
    match options.case {
        CaseMatch::Sensitive => raw.replace(query, replacement),
        CaseMatch::Insensitive => replace_ci(raw, query, replacement),
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
    while let Some(ch) = rest.chars().next() {
        // A non-empty needle always consumes at least one char, so `rest` shrinks
        // on every iteration whichever branch is taken.
        match ci_match_len(rest, &needle) {
            Some(len) if len > 0 => {
                out.push_str(replacement);
                rest = &rest[len..];
            }
            _ => {
                out.push(ch);
                rest = &rest[ch.len_utf8()..];
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{CaseMatch, FindOptions, MatchExtent, replace_ci, rewrite};

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
        let whole = FindOptions {
            extent: MatchExtent::WholeCell,
            ..FindOptions::default()
        };
        let sensitive = FindOptions {
            case: CaseMatch::Sensitive,
            ..FindOptions::default()
        };
        assert_eq!(rewrite("=A1+1", "x", "y", whole), "y");
        assert_eq!(rewrite("Xx", "x", "y", sensitive), "Xy");
    }
}
