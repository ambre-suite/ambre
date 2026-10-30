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
use crate::formula::text::fold_char;
use crate::formula::types::CellId;
use ambre_bus::app::find::{FindText, MatchCount, MatchIndex};

pub use ambre_bus::app::trame::{CaseMatch, FindOptions, MatchExtent, SearchScope, SheetScope};

/// The query folded by `case`, the form every comparison takes. The find text
/// is never empty: an empty field sends no query, so it never reaches a scan.
fn fold_needle(query: &FindText, case: CaseMatch) -> FoldedNeedle {
    FoldedNeedle {
        text: fold(query.as_str(), case),
        case,
    }
}

/// The text a replace writes in place of each match, as the user typed it.
/// Any text, the empty one included, is a replacement.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct Replacement(String);

/// The length alone: the text is document content.
impl std::fmt::Debug for Replacement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Replacement({} chars)", self.0.chars().count())
    }
}

impl Replacement {
    /// The replacement `text`.
    pub(super) fn new(text: String) -> Self {
        Self(text)
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

/// A query folded by the case it is compared under. Only
/// [`fold_needle`] makes one, so no comparison meets an unfolded query.
struct FoldedNeedle {
    text: String,
    case: CaseMatch,
}

impl TrameState {
    /// Scan the active sheet for cells whose **displayed** value matches `query`
    /// and reply with a [`TrameEvent::FindResult`] (`trame-find-replace`).
    /// Read-only: the receiver is shared, so the search cannot write the
    /// document (`FIND-001`). `matches` lists every hit of the sheet in
    /// row-major order. `active` is the index of the first hit strictly after
    /// the tracked active cell, wrapping to the top. The UI moves the active
    /// cell onto that hit, so a repeated `FindNext` advances naturally. An
    /// empty query clears the highlight: see [`Self::find_nothing`].
    pub(super) fn find_next(&self, query: &FindText, options: FindOptions) -> TrameOutcome {
        let needle = fold_needle(query, options.case);
        let mut matches: Vec<CellPos> = Vec::new();
        for at in self
            .index
            .used_range()
            .into_iter()
            .flat_map(CellRect::cells)
        {
            // The displayed value, format included (`TRAME-UI-078`).
            let text = self.displayed_text(at);
            if !text.is_empty() && cell_matches(&text, &needle, options.extent) {
                matches.push(at);
            }
        }
        // Pick the first hit strictly after the active cell (row-major), else wrap
        // to the first hit overall.
        let active = if matches.is_empty() {
            None
        } else {
            let after = matches.iter().position(|&at| at > self.active);
            Some(MatchIndex::from_usize(after.unwrap_or(0)))
        };
        TrameOutcome::display_only(vec![TrameEvent::FindResult { matches, active }])
    }

    /// The reply to an empty find: no matches, so the highlight clears.
    pub(super) fn find_nothing() -> TrameOutcome {
        TrameOutcome::display_only(vec![TrameEvent::FindResult {
            matches: Vec::new(),
            active: None,
        }])
    }

    /// The reply to an empty replace: nothing replaced.
    pub(super) fn replace_nothing() -> TrameOutcome {
        TrameOutcome::display_only(vec![TrameEvent::ReplaceDone {
            replaced: MatchCount::ZERO,
        }])
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
        query: &FindText,
        replacement: &Replacement,
        options: FindOptions,
    ) -> TrameOutcome {
        let needle = fold_needle(query, options.case);

        // One undo group for the whole replace (sealed by the next command's
        // fallback `group_end`), mirroring `fill_range`.
        let _ = self.undo_manager.group_start();

        // Active sheet first, through the regular write path (engine-aware).
        let mut replaced = self.replace_in_active(&needle, replacement, options);

        // Other sheets: edit their cells map directly, then reseed the engine so
        // their formula results recompute. The active write path only tracks the
        // active sheet's engine state.
        let mut touched_other = false;
        if options.sheets == SheetScope::All {
            let others: Vec<SheetId> = self
                .all_sheet_ids()
                .into_iter()
                .filter(|id| *id != self.active_sheet)
                .collect();
            for sheet_id in others {
                let n = self.replace_in_other(&sheet_id, &needle, replacement, options);
                if !n.is_zero() {
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
        query: &FindText,
        replacement: &Replacement,
        options: FindOptions,
    ) -> TrameOutcome {
        let needle = fold_needle(query, options.case);
        let at = self.active;

        let _ = self.undo_manager.group_start();
        let replaced = if self.replace_active_cell(at, &needle, replacement, options) {
            MatchCount::ONE
        } else {
            MatchCount::ZERO
        };

        // Recompute the match set so the UI can advance to (and highlight) the
        // next hit. After a successful replace the active cell no longer matches,
        // so `find_next` naturally lands on the following one.
        let next = self.find_next(query, options);

        if replaced.is_zero() {
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
        needle: &FoldedNeedle,
        replacement: &Replacement,
        options: FindOptions,
    ) -> MatchCount {
        let mut replaced = MatchCount::ZERO;
        for at in self
            .index
            .used_range()
            .into_iter()
            .flat_map(CellRect::cells)
        {
            if self.replace_active_cell(at, needle, replacement, options) {
                replaced += MatchCount::ONE;
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
        needle: &FoldedNeedle,
        replacement: &Replacement,
        options: FindOptions,
    ) -> bool {
        let stored = self.cell_raw(at);
        if stored.is_empty() {
            return false;
        }
        let is_formula = stored.is_formula();
        // A formula is searched and rewritten in its positional source, the
        // text the user reads (`TRAME-228`); the write binds it again.
        let raw = if is_formula {
            let active = self.active_sheet.clone();
            self.formula_source(&stored, &active, &self.sheet_lines())
        } else {
            FormulaSource::new(stored.into_string())
        };
        if is_formula && options.scope == SearchScope::Values {
            return false;
        }
        let haystack = if is_formula {
            raw.as_str().to_owned()
        } else {
            self.displayed_text(at)
        };
        if !cell_matches(&haystack, needle, options.extent) {
            return false;
        }
        let new_raw = rewrite(&raw, needle, replacement, options.extent);
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
        sheet_id: &SheetId,
        needle: &FoldedNeedle,
        replacement: &Replacement,
        options: FindOptions,
    ) -> MatchCount {
        let Some(&idx) = self.engine_sheet_by_id.get(sheet_id) else {
            return MatchCount::ZERO;
        };
        let index = self.build_sheet_index(sheet_id);
        let map = self
            .doc
            .get_map(SheetContainer::of(SheetRoot::Cells, sheet_id).as_str());
        let mut edits: Vec<((Uid, Uid), Option<StoredFormula>)> = Vec::new();
        let mut refusals = Vec::new();
        let lines = self.sheet_lines();
        for at in index.used_range().into_iter().flat_map(CellRect::cells) {
            let Some((rl, cl)) = index.resolve(at) else {
                continue;
            };
            let raw = match map
                .get(CellKey::of(rl, cl).as_str())
                .map(|v| v.get_deep_value())
            {
                Some(LoroValue::String(s)) => StoredFormula::new(s.to_string()),
                _ => continue,
            };
            let is_formula = raw.is_formula();
            if is_formula && options.scope == SearchScope::Values {
                continue;
            }
            let raw = if is_formula {
                self.formula_source(&raw, sheet_id, &lines)
            } else {
                FormulaSource::new(raw.into_string())
            };
            // Formula cells (in the formulas scope) match on their raw formula
            // text; plain cells match on the engine's displayed value.
            let haystack = if is_formula {
                raw.as_str().to_owned()
            } else {
                let cell = CellId::bound(idx, rl, cl);
                let text = if self.engine.is_formula_cell(cell) {
                    self.engine.evaluated_text(cell).unwrap_or_default()
                } else {
                    raw.as_str().to_owned()
                };
                // The displayed value, format included (`TRAME-UI-078`).
                match self.stored_style(sheet_id, rl, cl) {
                    Some(style) => self.format_display(at, text, &style),
                    None => text,
                }
            };
            if !cell_matches(&haystack, needle, options.extent) {
                continue;
            }
            let new_raw = rewrite(&raw, needle, replacement, options.extent);
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
        let count = MatchCount::from_len(edits.len());
        for ((rl, cl), stored) in edits {
            match stored {
                Some(text) => {
                    let _ = map.insert(CellKey::of(rl, cl).as_str(), text.as_str());
                }
                None => {
                    let _ = map.delete(CellKey::of(rl, cl).as_str());
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
        CaseMatch::Insensitive => s.chars().flat_map(fold_char).collect(),
    }
}

/// Whether `haystack` matches `needle`, folded by the needle's case: exact
/// equality for a whole-cell match, else substring containment.
///
/// An insensitive match reads the cell through [`strip_ci_match`], as a
/// replace does. A needle that ends inside the fold of one char matches for
/// neither. So a find meets no cell a replace leaves untouched.
fn cell_matches(haystack: &str, needle: &FoldedNeedle, extent: MatchExtent) -> bool {
    match (needle.case, extent) {
        (CaseMatch::Sensitive, MatchExtent::WholeCell) => haystack == needle.text,
        (CaseMatch::Sensitive, MatchExtent::Substring) => haystack.contains(needle.text.as_str()),
        (CaseMatch::Insensitive, MatchExtent::WholeCell) => {
            strip_ci_match(haystack, needle) == Some("")
        }
        (CaseMatch::Insensitive, MatchExtent::Substring) => {
            haystack.char_indices().any(|(at, _)| {
                haystack
                    .get(at..)
                    .and_then(|rest| strip_ci_match(rest, needle))
                    .is_some()
            })
        }
    }
}

/// Produce the rewritten raw text for a matched cell: the whole-cell case swaps
/// the entire content for `replacement`. The substring case replaces every
/// occurrence of the needle, case-insensitively unless its case is sensitive.
fn rewrite(
    raw: &FormulaSource,
    needle: &FoldedNeedle,
    replacement: &Replacement,
    extent: MatchExtent,
) -> FormulaSource {
    let raw = raw.as_str();
    FormulaSource::new(match (extent, needle.case) {
        (MatchExtent::WholeCell, _) => replacement.as_str().to_owned(),
        (MatchExtent::Substring, CaseMatch::Sensitive) => {
            raw.replace(needle.text.as_str(), replacement.as_str())
        }
        (MatchExtent::Substring, CaseMatch::Insensitive) => replace_ci(raw, needle, replacement),
    })
}

/// What follows a case-insensitive match of the folded `needle` anchored at
/// the start of `hay`. `None` when `hay` does not start with it.
///
/// Folding is compared character by character rather than on a pre-folded
/// copy of `hay`, because `to_lowercase` is not length-preserving. `I` (U+0130)
/// grows from 2 bytes to 3, the Kelvin sign `K` (U+212A) shrinks from 3 to 1. An
/// offset found in the folded text therefore does not address the original.
fn strip_ci_match<'a>(hay: &'a str, needle: &FoldedNeedle) -> Option<&'a str> {
    let mut needle = needle.text.chars();
    let mut pending = needle.next();
    for (offset, ch) in hay.char_indices() {
        if pending.is_none() {
            return hay.get(offset..);
        }
        // One source char can lowercase to several; all of them must line up, so
        // a needle ending mid-expansion is not a match.
        for lower in fold_char(ch) {
            match pending {
                Some(want) if want == lower => pending = needle.next(),
                _ => return None,
            }
        }
    }
    pending.is_none().then_some("")
}

/// Case-insensitive substring replace: replace every occurrence of `needle`
/// in `raw` (compared case-folded) with `replacement`, preserving the
/// surrounding text verbatim. A needle comes from a [`FindText`], so it is
/// never empty.
fn replace_ci(raw: &str, needle: &FoldedNeedle, replacement: &Replacement) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    let mut chars = rest.chars();
    while let Some(ch) = chars.next() {
        // A non-empty needle always consumes at least one char, so `rest` shrinks
        // on every iteration whichever branch is taken.
        match strip_ci_match(rest, needle) {
            Some(after) if after.len() < rest.len() => {
                out.push_str(replacement.as_str());
                rest = after;
            }
            _ => {
                out.push(ch);
                rest = chars.as_str();
            }
        }
        chars = rest.chars();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{
        CaseMatch, FindText, FoldedNeedle, FormulaSource, MatchExtent, Replacement, fold_needle,
        replace_ci, rewrite,
    };

    /// `query` folded by `case`.
    fn needle(query: &str, case: CaseMatch) -> FoldedNeedle {
        let query = FindText::new(query.to_owned()).expect("a test query is not empty");
        fold_needle(&query, case)
    }

    /// `raw` with every case-insensitive match of `query` replaced by `with`.
    fn ci(raw: &str, query: &str, with: &str) -> String {
        replace_ci(
            raw,
            &needle(query, CaseMatch::Insensitive),
            &Replacement::new(with.to_owned()),
        )
    }

    #[test]
    fn case_insensitive_replace_survives_a_non_length_preserving_lowercase() {
        // `to_lowercase` grows U+0130 (2 bytes -> 3) and shrinks the Kelvin sign
        // U+212A (3 -> 1). Offsets taken from the folded copy used to index the
        // original string, which panicked the Logic worker. That was out of bounds
        // on the first and off a char boundary on the second, and it killed the
        // session.
        assert_eq!(ci("\u{0130}x", "x", "Z"), "\u{0130}Z");
        assert_eq!(ci("\u{212A}elvin", "elvin", "Z"), "\u{212A}Z");
        // The match may also start on the offending char itself.
        assert_eq!(ci("a\u{212A}b", "k", "Z"), "aZb");
    }

    /// A capital sigma lowercases to a final sigma at the end of a word in a
    /// whole string, and to a plain sigma alone. A replace must meet every
    /// match a find meets, so both fold a sigma to one letter.
    #[test]
    fn case_insensitive_replace_meets_what_find_meets_over_a_final_sigma() {
        assert_eq!(ci("\u{39F}\u{3A3}", "\u{39F}\u{3A3}", "Z"), "Z");
        assert_eq!(ci("\u{3BF}\u{3C2}", "\u{39F}\u{3A3}", "Z"), "Z");
        assert_eq!(ci("\u{39F}\u{3A3}", "\u{3BF}\u{3C3}", "Z"), "Z");
        let hit = |hay: &str, query: &str| {
            super::cell_matches(
                hay,
                &needle(query, CaseMatch::Insensitive),
                MatchExtent::Substring,
            )
        };
        assert!(hit("\u{3BF}\u{3C3}", "\u{39F}\u{3A3}"));
        assert!(hit("\u{39F}\u{3A3}", "\u{3BF}\u{3C3}"));
    }

    /// A capital I with a dot folds to two chars. A needle that ends between
    /// them is no match for a replace, so a find must not meet it either.
    #[test]
    fn a_find_meets_no_cell_a_replace_leaves_untouched() {
        let hit = |hay: &str, query: &str, extent| {
            super::cell_matches(hay, &needle(query, CaseMatch::Insensitive), extent)
        };
        assert!(!hit("\u{130}", "i", MatchExtent::Substring));
        assert_eq!(ci("\u{130}", "i", "Z"), "\u{130}");
        assert!(hit("a\u{130}b", "\u{130}B", MatchExtent::Substring));
        assert!(hit("\u{130}", "\u{130}", MatchExtent::WholeCell));
        assert!(!hit("ab", "a", MatchExtent::WholeCell));
    }

    #[test]
    fn case_insensitive_replace_handles_the_ordinary_cases() {
        assert_eq!(ci("Hello hello", "HELLO", "hi"), "hi hi");
        assert_eq!(ci("aaa", "aa", "b"), "ba");
        assert_eq!(ci("abc", "z", "y"), "abc");
        assert_eq!(ci("abc", "abc", ""), "");
        // A replacement containing the query must not be rescanned.
        assert_eq!(ci("a", "a", "aa"), "aa");
    }

    /// A replacement is document content, so its Debug form is its length.
    #[test]
    fn a_replacement_prints_its_length_alone() {
        let printed = format!("{:?}", Replacement::new("secret".to_owned()));
        assert_eq!(printed, "Replacement(6 chars)");
    }

    #[test]
    fn whole_cell_rewrite_swaps_the_entire_content() {
        let y = Replacement::new("y".to_owned());
        let raw = |text: &str| FormulaSource::new(text.to_owned());
        let x = needle("x", CaseMatch::Insensitive);
        let whole = rewrite(&raw("=A1+1"), &x, &y, MatchExtent::WholeCell);
        assert_eq!(whole.as_str(), "y");
        let x = needle("x", CaseMatch::Sensitive);
        let part = rewrite(&raw("Xx"), &x, &y, MatchExtent::Substring);
        assert_eq!(part.as_str(), "Xy");
    }
}
