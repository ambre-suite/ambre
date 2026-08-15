// ambre-trame/src/logic/fixtures.rs
// Trame's half of the fixture loader surface (`ambre-spec-fixtures.md` §2.B).
//
// `shape_of` and `canonical_dump` are declared in `ambre-logic` over `DocState`;
// their per-app bodies live here, next to the schema they read, for the same
// reason §6 puts generators next to their engine: a schema change and the code
// that decodes it move in one commit. `ambre-logic` only dispatches.
//
// Fixture code (I4): the whole module sits behind the non-default `fixtures`
// feature and is unreachable from any client build. The determinism seam
// (`TrameState::with_peer`) is *not* here -- it is engine API, documented in
// `ambre-spec-trame-logic.md` §2 and ungated.
//
// Part of the `TrameState` impl, split out by concern like `sync` and `docfile`.

use std::collections::BTreeMap;

use ambre_core::{FixtureShape, ShapeBody};

use super::*;

impl TrameState {
    /// The coarse shape of this workbook, as the manifest records it.
    ///
    /// Counted from the CRDT rather than declared by the generator: §4 checks
    /// the manifest against a recomputed shape, and a generator declaring its own
    /// would assert only that it agrees with itself.
    ///
    /// `formulas` counts *stored* formula sources (a leading `=`), not the
    /// engine's bytecode cache: the cache is a derived, local structure, and the
    /// manifest describes the document.
    pub fn fixture_shape(&self) -> FixtureShape {
        let mut cells = 0u32;
        let mut formulas = 0u32;
        let sheet_ids = self.fixture_sheet_ids();
        for sheet in &sheet_ids {
            if let LoroValue::Map(map) = self.doc.get_map(cells_map_name(sheet)).get_value() {
                for value in map.values() {
                    cells = cells.saturating_add(1);
                    if matches!(value, LoroValue::String(s) if s.starts_with('=')) {
                        formulas = formulas.saturating_add(1);
                    }
                }
            }
        }
        FixtureShape {
            peers: self.fixture_peer_count(),
            body: ShapeBody::Trame {
                cells,
                sheets: u16::try_from(sheet_ids.len()).unwrap_or(u16::MAX),
                formulas,
            },
        }
    }

    /// The canonical semantic form of this workbook: every user-visible value, in
    /// a deterministic order, as text.
    ///
    /// **Not** the CRDT -- two workbooks agreeing on content still differ in op
    /// history, peer ids and commit timestamps, so no byte or state comparison
    /// can express "same document by a different route" (§6). This is what the
    /// frozen belt of §4 compares, and text means a failure prints a readable
    /// diff rather than a length mismatch.
    ///
    /// **Formula cells carry both representations** -- the stored source and the
    /// evaluated result. Dumping only the source would let an evaluation
    /// regression pass unnoticed; dumping only the result would hide a change in
    /// what is persisted.
    pub fn canonical_dump(&self) -> String {
        let mut out = String::from("trame\n");
        for sheet in self.fixture_sheet_ids() {
            let name = self
                .sheet_id_to_name
                .get(&sheet)
                .cloned()
                .unwrap_or_else(|| sheet.clone());
            out.push_str(&format!("sheet {sheet} {name}\n"));
            // One index per sheet: `self.index` tracks only the active sheet, and
            // a dump quantifies over all of them.
            let index = SheetIndex::build(
                &self.doc.get_movable_list(rows_list_name(&sheet)),
                &self.doc.get_movable_list(cols_list_name(&sheet)),
            );
            let sheet_u16 = self.sheet_id_to_u16.get(&sheet).copied();
            // Keyed by grid position, not by stored key: the key carries peer ids
            // and mint counters, which are exactly what two documents "agreeing by
            // a different route" are allowed to differ in. `BTreeMap` sorts the
            // rows/columns for free, making the order deterministic.
            let mut by_pos: BTreeMap<(usize, usize), String> = BTreeMap::new();
            if let LoroValue::Map(map) = self.doc.get_map(cells_map_name(&sheet)).get_value() {
                for (key, value) in map.iter() {
                    let LoroValue::String(raw) = value else {
                        continue;
                    };
                    let raw = raw.as_str();
                    let Some((rl, cl)) = parse_cell_key(key) else {
                        continue;
                    };
                    // A cell whose row or column was deleted has no position and
                    // no user-visible existence; it is not part of the content.
                    let (Some(row), Some(col)) = (index.row_pos(&rl), index.col_pos(&cl)) else {
                        continue;
                    };
                    let label = format!(
                        "{}{}",
                        crate::columns::col_to_alpha(col as u32),
                        row.saturating_add(1)
                    );
                    let line = match sheet_u16 {
                        // Both representations for a formula cell, one for a
                        // literal. `#N/A` marks a formula the engine has not
                        // evaluated -- a real difference, not one to paper over.
                        Some(idx) if raw.starts_with('=') => {
                            let cell = CellId::bound(idx, rl, cl);
                            let text = self
                                .engine
                                .evaluated_text(cell)
                                .unwrap_or_else(|| "<unevaluated>".to_owned());
                            format!("  {label} raw={raw} text={text}\n")
                        }
                        _ => format!("  {label} {raw}\n"),
                    };
                    by_pos.insert((row, col), line);
                }
            }
            for line in by_pos.values() {
                out.push_str(line);
            }
        }
        out
    }

    /// Sheet ids in a deterministic order: the tab order first, then any sheet
    /// the order list does not mention, sorted by id.
    ///
    /// Not `sheet_order_ids()`: that one is a *view* concern and pins the active
    /// sheet first, so the same workbook would dump differently depending on
    /// which tab happened to be open.
    fn fixture_sheet_ids(&self) -> Vec<String> {
        let mut listed: Vec<String> = match self.doc.get_movable_list(SHEET_ORDER).get_value() {
            LoroValue::List(items) => items
                .iter()
                .filter_map(|v| match v {
                    LoroValue::String(s) => Some(s.to_string()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        let mut rest: Vec<String> = match self.doc.get_map(SHEETS_MAP).get_value() {
            LoroValue::Map(sheets) => sheets
                .keys()
                .filter(|id| !listed.contains(id))
                .cloned()
                .collect(),
            _ => Vec::new(),
        };
        rest.sort();
        listed.extend(rest);
        // A workbook always has at least the implicit default sheet, even before
        // anything registers it in the `sheets` map.
        if listed.is_empty() {
            listed.push(DEFAULT_SHEET.to_owned());
        }
        listed
    }

    /// Number of distinct peers whose ops are present (1 for a solo document).
    /// `oplog_vv()` derefs to a map keyed by `PeerID`; the codebase already calls
    /// it on both engines' sync paths.
    fn fixture_peer_count(&self) -> u8 {
        u8::try_from(self.doc.oplog_vv().len()).unwrap_or(u8::MAX)
    }
}
