//! The schema reading belt: what each stored value means, frozen per
//! `SCHEMA_VERSION` (`SCHEMA-017`).
//!
//! The shape belt freezes the keys and the types. A width stored in pixels and
//! read in points keeps both, so the shape stays green while every older
//! workbook is read wrong. This belt freezes the other half: one line per
//! stored value, carrying the value as stored and what this build reads it as.
//!
//! **Two columns, two drifts.** A line reads `path = stored => reading`. A
//! second workbook that opened the first one's snapshot takes the reading,
//! through the readers the view uses. A reader that moves its
//! unit moves the reading. A writer and a reader that move together leave the
//! reading alone and move the stored value. Either way the frozen text turns
//! red under an unchanged `SCHEMA_VERSION`.
//!
//! **Absolute units.** A reading names its unit. A width is in px, a font size
//! in pt, a colour is `#rrggbb`, a position a one-based row and column. A
//! position resolves the stable id the key carries, so the line says which
//! cell it is about.
//!
//! **What holds the freeze honest.** A stored value no reader resolves fails
//! [`every_stored_value_has_a_reading`], and a declared key the scenario does
//! not reach fails [`every_declared_key_has_a_reading`]. The scenario is the
//! shape belt's, so the two belts read the same workbook. Its values are frozen
//! here, so a command added to it lands with a raised version, like a new key.

use ambre_core::Rgb;
use std::collections::BTreeSet;

use ambre_bus::{CellBorder, CellStyle, TrameCommand};
use ambre_core::schema::verdict;
use loro::{ExportMode, LoroValue};

use super::schema_shape::{
    ANY, canonical_root, declared_sheet_prefixes, declared_string_consts, keys_are_schema_words,
    section_of, workbook_of_every_key,
};
use crate::formula::types::NameTarget;
use crate::logic::TrameState;
use crate::logic::schema::{FILTER_KEY, NAMED_RANGES_MAP, SCHEMA_VERSION, SHEET_ORDER, SHEETS_MAP};
use crate::model::{Uid, parse_cell_key};
use crate::{CellPos, Col};

/// The frozen readings, one section per `SCHEMA_VERSION`.
const FROZEN: &str = include_str!("../../schema/readings.txt");

/// The foundation's `meta` root, as a pattern.
const META_ROOT: &str = ambre_core::crdt::FoundationRoot::Meta.as_str();

/// The source the key declarations are read from, as the shape belt reads it.
const SCHEMA_SOURCE: &str = include_str!("schema.rs");

/// One stored value: the root it sits under, the keys down to it, and the value.
struct Stored {
    root: String,
    keys: Vec<String>,
    value: LoroValue,
}

/// Every stored value of a workbook, maps walked to their leaves and lists
/// read item by item.
fn stored_values(value: &LoroValue) -> Vec<Stored> {
    let mut found = Vec::new();
    let LoroValue::Map(roots) = value else {
        return found;
    };
    for (root, value) in roots.iter() {
        walk(root, Vec::new(), value, &mut found);
    }
    found
}

fn walk(root: &str, keys: Vec<String>, value: &LoroValue, found: &mut Vec<Stored>) {
    match value {
        LoroValue::Map(entries) => {
            for (key, child) in entries.iter() {
                let mut below = keys.clone();
                below.push(key.clone());
                walk(root, below, child, found);
            }
        }
        LoroValue::List(items) => {
            for (at, item) in items.iter().enumerate() {
                let mut below = keys.clone();
                below.push(at.to_string());
                walk(root, below, item, found);
            }
        }
        _ => found.push(Stored {
            root: root.to_owned(),
            keys,
            value: value.clone(),
        }),
    }
}

/// A stored value in the one spelling the freeze uses.
fn stored_text(value: &LoroValue) -> String {
    match value {
        LoroValue::String(text) => format!("{:?}", text.as_str()),
        LoroValue::Binary(bytes) => bytes.iter().map(|b| format!("{b:02x}")).collect(),
        LoroValue::Double(n) => n.to_string(),
        LoroValue::I64(n) => n.to_string(),
        LoroValue::Bool(b) => b.to_string(),
        other => format!("{other:?}"),
    }
}

/// The path a value is frozen under: dictionary keys and list positions
/// collapse to [`ANY`], as in the shape, and the reading names the entry.
fn frozen_path(stored: &Stored) -> String {
    let root = canonical_root(&stored.root);
    let mut path = root.clone();
    let mut schema_words = keys_are_schema_words(&root);
    for key in &stored.keys {
        let segment = if schema_words { key.as_str() } else { ANY };
        path.push('/');
        path.push_str(segment);
        schema_words = true;
    }
    path
}

/// A one-based cell position, the spelling every reading uses.
fn cell(at: CellPos) -> String {
    format!("R{}C{}", ordinal(at.row.get()), ordinal(at.col.get()))
}

/// The one-based ordinal of the zero-based position `pos`.
fn ordinal(pos: u32) -> u32 {
    pos.saturating_add(1)
}

/// A packed `0xRRGGBB` colour.
fn colour(rgb: Rgb) -> String {
    format!("#{:06x}", rgb.bits())
}

/// What a resolved style sets, field by field, each in its unit.
///
/// The record is destructured whole: a field added to it fails to compile here
/// until its reading is written.
fn style_reading(bold: bool, italic: bool, style: &CellStyle) -> String {
    let CellStyle {
        underline,
        strikethrough,
        h_align,
        v_align,
        wrap,
        fg,
        bg,
        font,
        size,
        format,
        decimals,
        borders,
    } = style;
    let mut parts = Vec::new();
    for (set, name) in [
        (bold, "bold"),
        (italic, "italic"),
        (*underline, "underlined"),
        (*strikethrough, "struck"),
        (*wrap, "wrapped"),
    ] {
        if set {
            parts.push(name.to_owned());
        }
    }
    parts.push(format!("aligned {h_align:?} {v_align:?}"));
    if let Some(rgb) = fg {
        parts.push(format!("text {}", colour(*rgb)));
    }
    if let Some(rgb) = bg {
        parts.push(format!("fill {}", colour(*rgb)));
    }
    if let Some(font) = font {
        parts.push(format!("font {font:?}"));
    }
    if let Some(size) = size {
        parts.push(format!("{size} pt"));
    }
    parts.push(format!("format {format:?}"));
    if let Some(decimals) = decimals {
        parts.push(format!("{decimals} decimals"));
    }
    for (edge, border) in [
        ("top", borders.top),
        ("right", borders.right),
        ("bottom", borders.bottom),
        ("left", borders.left),
    ] {
        if let Some(CellBorder { style, color }) = border {
            let shade = color.map_or_else(|| "grid".to_owned(), colour);
            parts.push(format!("{edge} {style:?} {shade}"));
        }
    }
    parts.join(", ")
}

/// A name's target, as the formula engine resolved it.
fn target_reading(target: &NameTarget) -> String {
    match target {
        NameTarget::Cell(at) => format!(
            "sheet {} {}",
            at.sheet.index() + 1,
            cell(CellPos::new(at.row, at.col))
        ),
        NameTarget::Range(range) => format!(
            "sheet {} {}:{}",
            range.sheet.index() + 1,
            cell(CellPos::new(range.row_start, range.col_start)),
            cell(CellPos::new(range.row_end, range.col_end))
        ),
    }
}

/// What `reader` reads a stored value as, or `None` when no reader of this
/// build resolves it.
fn reading(reader: &TrameState, stored: &Stored) -> Option<String> {
    let key = stored.keys.first().map(String::as_str)?;
    let root = stored.root.as_str();
    match root {
        META_ROOT if key == ambre_core::schema::MetaKey::SchemaVersion.as_str() => {
            let LoroValue::I64(n) = stored.value else {
                return None;
            };
            let version = u32::try_from(n).ok()?;
            Some(format!(
                "version {version}, {:?}",
                verdict(version, SCHEMA_VERSION)
            ))
        }
        SHEETS_MAP => {
            let tab = reader.sheet_order_ids().iter().position(|id| id == key)?;
            let name = reader.sheet_names().remove(key)?;
            Some(format!("tab {} named {name:?}", tab + 1))
        }
        SHEET_ORDER => {
            let LoroValue::String(id) = &stored.value else {
                return None;
            };
            let tab = reader
                .sheet_order_ids()
                .iter()
                .position(|at| at == id.as_str())?;
            Some(format!("tab {}", tab + 1))
        }
        NAMED_RANGES_MAP => Some(format!(
            "{key} names {}",
            target_reading(reader.named.get(key)?)
        )),
        _ => sheet_reading(reader, stored, key),
    }
}

/// The reading of a value under a per-sheet container of the active sheet.
fn sheet_reading(reader: &TrameState, stored: &Stored, key: &str) -> Option<String> {
    let root = stored.root.as_str();
    let (prefix, sheet) = declared_sheet_prefixes(SCHEMA_SOURCE)
        .into_iter()
        .find_map(|prefix| Some((root.strip_prefix(&prefix)?.to_owned(), prefix)))
        .map(|(sheet, prefix)| (prefix, sheet))?;
    // The derived caches follow the active sheet only, and the scenario writes
    // nothing per sheet elsewhere. A value on another sheet has no reading,
    // and the coverage test names it.
    if sheet != reader.active_sheet {
        return None;
    }
    let index = &reader.index;
    let uid = || Uid::parse(key);
    let at = || {
        let (row, col) = parse_cell_key(key)?;
        Some(CellPos::new(index.row_pos(&row)?, index.col_pos(&col)?))
    };
    let item = || match &stored.value {
        LoroValue::String(id) => Uid::parse(id),
        _ => None,
    };
    match prefix.as_str() {
        "cells_" => {
            let place = at()?;
            let shown = reader.displayed_text(place);
            Some(format!("{} shows {shown:?}", cell(place)))
        }
        "cellstyle_" => {
            let place = at()?;
            let style = reader.styles.get(&place).cloned().unwrap_or_default();
            let read = style_reading(
                reader.bold.contains(&place),
                reader.italic.contains(&place),
                &style,
            );
            Some(format!("{} {read}", cell(place)))
        }
        "col_widths_" => {
            let col = index.col_pos(&uid()?)?;
            Some(format!(
                "column {} is {} px wide",
                ordinal(col.get()),
                reader.col_width(col)
            ))
        }
        "row_heights_" => {
            let row = index.row_pos(&uid()?)?;
            Some(format!(
                "row {} is {} px tall",
                ordinal(row.get()),
                reader.row_height(row)
            ))
        }
        "cols_" => Some(format!(
            "column {}",
            ordinal(index.col_pos(&item()?)?.get())
        )),
        "rows_" => Some(format!("row {}", ordinal(index.row_pos(&item()?)?.get()))),
        "merges_" => {
            let master = at()?;
            let end = reader.merges.get(&master)?;
            Some(format!("{}:{} merged", cell(master), cell(*end)))
        }
        "filters_" if key == FILTER_KEY => {
            let (col, kind) = reader.load_active_filter()?;
            Some(format!(
                "column {} keeps {kind:?}",
                ordinal(index.col_pos(&col)?.get())
            ))
        }
        "filter_hidden_" => {
            let row = index.row_pos(&uid()?)?;
            let shown = if reader.hidden_rows.contains(&row) {
                "hidden"
            } else {
                "shown"
            };
            Some(format!("row {} {shown}", ordinal(row.get())))
        }
        _ => None,
    }
}

/// A workbook opened from `state`'s snapshot, through the full-state read
/// every reload takes.
fn reopened(state: &TrameState) -> TrameState {
    let snapshot = state
        .doc
        .export(ExportMode::Snapshot)
        .expect("a workbook exports its snapshot");
    let mut reader = TrameState::with_peer(ambre_core::PeerId(2));
    reader
        .import_snapshot(&snapshot)
        .expect("a workbook this build wrote reopens");
    reader
}

/// A workbook's readings, as one reader reads them.
#[derive(Default)]
struct Readings {
    /// The lines the freeze holds.
    lines: BTreeSet<String>,
    /// The stored values no reader resolved.
    unread: Vec<String>,
    /// The roots and the keys above a resolved value, which the coverage
    /// assertion answers from.
    keys: BTreeSet<String>,
}

/// The readings of a workbook as `reader` reads it.
fn readings_of(state: &TrameState, reader: &TrameState) -> Readings {
    let mut readings = Readings::default();
    for stored in stored_values(&state.doc.get_deep_value()) {
        let path = frozen_path(&stored);
        let value = stored_text(&stored.value);
        match reading(reader, &stored) {
            Some(read) => {
                readings.lines.insert(format!("{path} = {value} => {read}"));
                readings.keys.insert(canonical_root(&stored.root));
                readings.keys.insert(stored.root.clone());
                readings.keys.extend(stored.keys.iter().cloned());
            }
            None => readings.unread.push(format!("{path} = {value}")),
        }
    }
    readings
}

fn render(lines: &BTreeSet<String>) -> String {
    lines.iter().map(|line| format!("{line}\n")).collect()
}

#[test]
fn the_reading_a_build_makes_is_the_reading_frozen_for_its_version() {
    let state = workbook_of_every_key();
    let written = render(&readings_of(&state, &reopened(&state)).lines);
    let Some(frozen) = section_of(FROZEN, SCHEMA_VERSION) else {
        panic!(
            "`schema/readings.txt` carries no `== v{SCHEMA_VERSION}` section. \
             The version rose without a freeze; add the section below:\n{written}"
        );
    };
    assert_eq!(
        written, frozen,
        "what this build stores or reads is not what was frozen for \
         SCHEMA_VERSION {SCHEMA_VERSION}. A value moved its unit or its \
         meaning, and every workbook written before reads wrong. Raise \
         SCHEMA_VERSION and freeze the new reading as its own section \
         (`SCHEMA-017`)."
    );
}

#[test]
fn every_stored_value_has_a_reading() {
    let state = workbook_of_every_key();
    let unread = readings_of(&state, &reopened(&state)).unread;
    assert!(
        unread.is_empty(),
        "no reader of this build resolves these stored values, so the freeze \
         holds nothing of what they mean. Write their reading in \
         `schema_reading.rs`:\n{}",
        unread.join("\n")
    );
}

#[test]
fn every_declared_key_has_a_reading() {
    let state = workbook_of_every_key();
    let keys = readings_of(&state, &reopened(&state)).keys;
    for key in declared_string_consts(SCHEMA_SOURCE) {
        assert!(
            keys.contains(&key),
            "`{key}` is declared as a schema key and no reading carries it"
        );
    }
    for prefix in declared_sheet_prefixes(SCHEMA_SOURCE) {
        let root = format!("{prefix}{ANY}");
        assert!(
            keys.contains(&root),
            "`{prefix}{{sheet}}` is declared as a per-sheet container and no \
             reading carries it"
        );
    }
}

#[test]
fn a_stored_value_moving_moves_the_reading() {
    let state = workbook_of_every_key();
    let before = readings_of(&state, &reopened(&state)).lines;
    let mut moved = workbook_of_every_key();
    // The same width written in points: the key and the type stay, so only the
    // value column can show it.
    moved.handle_command(TrameCommand::ResizeCol {
        version: ambre_bus::ViewportVersion(0),
        sheet_id: ambre_bus::SheetId(super::schema::DEFAULT_SHEET.to_owned()),
        col: Col::new(0),
        width_px: 90.0,
    });
    let after = readings_of(&moved, &reopened(&moved)).lines;
    assert_ne!(render(&before), render(&after));
}

#[test]
fn a_reader_moving_moves_the_reading() {
    let state = workbook_of_every_key();
    let before = readings_of(&state, &reopened(&state)).lines;
    // A reader that takes the stored width as points: the stored bytes are
    // the same, only what this build makes of them moved.
    let mut misread = reopened(&state);
    let width = misread.col_width(Col::new(0));
    misread.col_widths.insert(Col::new(0), width * 96.0 / 72.0);
    let after = readings_of(&state, &misread).lines;
    assert_ne!(
        render(&before),
        render(&after),
        "a reader moving its unit has to move the reading, or the freeze \
         holds only the bytes"
    );
}
