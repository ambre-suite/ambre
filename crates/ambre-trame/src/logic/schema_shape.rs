//! The schema shape belt: the key set and the value types a build writes,
//! frozen per `SCHEMA_VERSION` (`SCHEMA-018`).
//!
//! A shape is the workbook with every value removed: one line per reachable
//! path, each naming the type the value under it holds. Adding a key, dropping
//! one, or changing an `i64` into an `f64` moves a line; changing a cell from
//! `"a"` to `"b"` does not. The frozen text is therefore a diff a reviewer
//! reads, and a build that writes a new shape under an unchanged
//! `SCHEMA_VERSION` turns red.
//!
//! **Two segments collapse.** A per-sheet container carries its sheet's id in
//! its own name. A register keyed by a stable id pair carries peer ids in its
//! keys. A shape that kept either would differ between two runs of the same
//! build, so both become [`ANY`].
//!
//! **The binary registers.** Two of them hold a `postcard` blob rather than a
//! scalar. `postcard` decodes a blob whole: a field added, dropped, reordered
//! or retyped changes what the bytes mean. `binary` alone would say
//! nothing about that, so the shape also carries the encoding of one canonical
//! value per blob, as hex. The layout is then frozen with the rest, and a
//! layout change moves a line.
//!
//! **What holds the freeze honest.** A shape is only as wide as the scenario
//! that builds it. A key no command reaches leaves no line, so dropping it
//! would go unnoticed. [`every_declared_key_appears_in_the_shape`] closes that
//! gap: it reads `schema.rs` at compile time, and demands a line for every key
//! and every per-sheet container it declares.
//!
//! **What it cannot hold.** A value whose meaning moves under an unchanged key
//! and an unchanged type leaves the shape identical (`SCHEMA-017`). That half
//! is the reading belt's, in `schema_reading.rs`, over this same scenario.

use ambre_core::Rgb;
use std::collections::BTreeSet;

use ambre_bus::{
    BorderStyle, Borders, CellBorder, CellFormat, CellStyle, FilterKind, FilterOp, HAlign,
    SelectionKind, StylePatch, TrameCommand, VAlign,
};
use ambre_core::schema::{SCHEMA_META_MAP, SCHEMA_VERSION_KEY, migration_peer, step_fingerprint};
use loro::{ExportMode, LoroDoc, LoroValue};

use crate::logic::TrameState;
use crate::logic::schema::{
    DEFAULT_SHEET, FilterRecord, MIGRATIONS, SCHEMA_VERSION, SHEETS_MAP, cells_map_name,
    cols_list_name, filters_map_name, rows_list_name,
};
use crate::logic::styles::StyleRecord;
use crate::model::{Uid, cell_key};

/// The frozen shapes, one section per `SCHEMA_VERSION`.
const FROZEN: &str = include_str!("../../schema/shapes.txt");

/// The frozen scaffolding identities, one section per `SCHEMA_VERSION`.
const SCAFFOLD: &str = include_str!("../../schema/scaffold.txt");

/// What each migration step writes, one section per `SCHEMA_VERSION`.
const STEPS: &str = include_str!("../../schema/migrations.txt");

/// The source the key declarations are read from, so the coverage assertion
/// cannot drift from the module it guards.
const SCHEMA_SOURCE: &str = include_str!("schema.rs");

/// The segment a sheet id or a stable id pair collapses to.
pub(super) const ANY: &str = "*";

/// The declared `&str` constants that name something other than a schema key,
/// and so carry no line of their own. `MATERIALIZE_ORIGIN` is a Loro commit
/// origin, read back from a subscription rather than written into a register.
const NOT_SCHEMA_KEYS: [&str; 1] = ["mat"];

/// The roots whose keys are schema words rather than data. Every other map is
/// a dictionary keyed by an id, a sheet name or a user-chosen name, so its
/// keys collapse to [`ANY`].
pub(super) fn keys_are_schema_words(root: &str) -> bool {
    root == ambre_core::schema::SCHEMA_META_MAP || root.starts_with("filters_")
}

/// The type a value holds, named in the one spelling the freeze uses.
///
/// Not the Rust type and not Loro's. The number shapes Loro stores stay apart
/// here, because an `i64` key turning into an `f64` one is the drift this belt
/// exists to catch.
fn type_name(value: &LoroValue) -> &'static str {
    match value {
        LoroValue::Null => "null",
        LoroValue::Bool(_) => "bool",
        LoroValue::Double(_) => "f64",
        LoroValue::I64(_) => "i64",
        LoroValue::Binary(_) => "binary",
        LoroValue::String(_) => "string",
        LoroValue::List(_) => "list",
        LoroValue::Map(_) => "map",
        LoroValue::Container(_) => "container",
    }
}

/// A workbook's shape: the lines the freeze holds, plus the key segments the
/// coverage assertion answers from.
#[derive(Default)]
struct Shape {
    lines: BTreeSet<String>,
    keys: BTreeSet<String>,
}

impl Shape {
    /// Record one path and the type under it.
    fn record(&mut self, path: &str, value: &LoroValue) {
        self.lines.insert(format!("{path}: {}", type_name(value)));
    }
}

/// Every `format!("x_{sheet}")` the schema declares, as the prefix `x_`.
///
/// Reading them rather than listing them is what makes a new per-sheet
/// container fail the coverage assertion instead of passing unseen.
pub(super) fn declared_sheet_prefixes(source: &str) -> Vec<String> {
    let mut found = Vec::new();
    for line in source.lines() {
        let Some((_, tail)) = line.split_once("format!(\"") else {
            continue;
        };
        let Some((name, _)) = tail.split_once("{sheet}") else {
            continue;
        };
        if !name.is_empty() {
            found.push(name.to_owned());
        }
    }
    found.sort();
    found.dedup();
    found
}

/// The name a root is frozen under: a per-sheet container loses its sheet id.
pub(super) fn canonical_root(name: &str) -> String {
    for prefix in declared_sheet_prefixes(SCHEMA_SOURCE) {
        if name.starts_with(&prefix) {
            return format!("{prefix}{ANY}");
        }
    }
    name.to_owned()
}

/// Walk a map, recording every key and descending into the nested maps.
///
/// `schema_words` says whether this level's keys are the schema's own. Below a
/// dictionary level the keys are schema words again: an id names an entry, and
/// the entry's fields are the schema's.
fn walk_map(prefix: &str, value: &LoroValue, schema_words: bool, shape: &mut Shape) {
    let LoroValue::Map(entries) = value else {
        return;
    };
    for (key, child) in entries.iter() {
        shape.keys.insert(key.clone());
        let segment = if schema_words { key.as_str() } else { ANY };
        let path = format!("{prefix}/{segment}");
        shape.record(&path, child);
        walk_map(&path, child, true, shape);
    }
}

/// The shape of a workbook: every root register, the entries under it, and the
/// `postcard` layouts its binary registers carry.
fn shape_of(doc: &LoroDoc) -> Shape {
    let mut shape = Shape::default();
    let LoroValue::Map(roots) = doc.get_deep_value() else {
        return shape;
    };

    for (name, value) in roots.iter() {
        let root = canonical_root(name);
        shape.keys.insert(name.clone());
        shape.keys.insert(root.clone());
        shape.record(&root, value);
        match value {
            LoroValue::Map(_) => walk_map(&root, value, keys_are_schema_words(&root), &mut shape),
            LoroValue::List(items) => {
                // A list holds entries, never keys: one line for the element
                // type, whatever the length the scenario happened to reach.
                for item in items.iter() {
                    shape.record(&format!("{root}/{ANY}"), item);
                }
            }
            _ => {}
        }
    }

    for (path, hex) in postcard_layouts() {
        shape.lines.insert(format!("postcard/{path}: {hex}"));
    }
    shape
}

/// The `postcard` layout of each binary register, as the encoding of one
/// canonical value.
///
/// Every field is set, and to a value of its own. A reordering of two
/// same-typed fields then moves the bytes like any other layout change. Nothing
/// here is spelled with `..Default::default()`: a field added to one of these
/// records must fail to compile rather than encode silently.
fn postcard_layouts() -> Vec<(String, String)> {
    let style = StyleRecord {
        bold: true,
        italic: true,
        style: CellStyle {
            underline: true,
            strikethrough: true,
            h_align: HAlign::Right,
            v_align: VAlign::Bottom,
            wrap: true,
            fg: Some(Rgb::from_bits(0x01_02_03)),
            bg: Some(Rgb::from_bits(0x04_05_06)),
            font: Some("f".to_owned()),
            size: Some(7),
            format: CellFormat::Time,
            decimals: Some(8),
            borders: Borders {
                top: Some(CellBorder {
                    style: BorderStyle::Double,
                    color: Some(Rgb::from_bits(0x0a_0b_0c)),
                }),
                right: None,
                bottom: Some(CellBorder {
                    style: BorderStyle::Dotted,
                    color: None,
                }),
                left: None,
            },
        },
    };
    let condition = FilterRecord {
        col: "c".to_owned(),
        kind: FilterKind::Condition {
            op: FilterOp::Ne,
            value: "v".to_owned(),
        },
    };
    let values = FilterRecord {
        col: "c".to_owned(),
        kind: FilterKind::Values(vec!["v".to_owned()]),
    };
    vec![
        (format!("cellstyle_{ANY}/{ANY}"), hex_of(&style)),
        (
            format!("filters_{ANY}/active#condition"),
            hex_of(&condition),
        ),
        (format!("filters_{ANY}/active#values"), hex_of(&values)),
    ]
}

/// One record's `postcard` encoding, as lower-case hex.
fn hex_of<T: serde::Serialize>(record: &T) -> String {
    let bytes = postcard::to_allocvec(record).unwrap_or_default();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Render a shape as the freeze holds it: one line per path, sorted.
fn render(shape: &Shape) -> String {
    let mut out = String::new();
    for line in &shape.lines {
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// The section of the freeze that names this version, or `None` when the freeze
/// carries no such section.
fn frozen_section(version: u32) -> Option<String> {
    section_of(FROZEN, version)
}

/// The section of a frozen text that names this version.
pub(super) fn section_of(frozen: &str, version: u32) -> Option<String> {
    let header = format!("== v{version}");
    let mut taken = String::new();
    let mut inside = false;
    for line in frozen.lines() {
        if line.starts_with("== ") {
            if inside {
                break;
            }
            inside = line.trim_end() == header;
            continue;
        }
        if inside && !line.is_empty() {
            taken.push_str(line);
            taken.push('\n');
        }
    }
    inside.then_some(taken)
}

/// Every `const NAME: &str = "literal";` a source declares.
///
/// The declarations are the schema's key vocabulary, and reading them here is
/// what makes a new key fail the coverage assertion rather than pass unseen.
pub(super) fn declared_string_consts(source: &str) -> Vec<String> {
    let mut found = Vec::new();
    for line in source.lines() {
        let bare = line.trim_start();
        // The declaration's visibility is not part of the key it names.
        let bare = bare
            .split_once("const ")
            .filter(|(before, _)| before.is_empty() || before.starts_with("pub"))
            .map(|(_, rest)| rest);
        let Some(rest) = bare else {
            continue;
        };
        let Some((_, tail)) = rest.split_once(": &str = ") else {
            continue;
        };
        let Some(literal) = tail.strip_prefix('"').and_then(|t| t.split('"').next()) else {
            continue;
        };
        if !literal.is_empty() && !NOT_SCHEMA_KEYS.contains(&literal) {
            found.push(literal.to_owned());
        }
    }
    found
}

/// A workbook that reaches every key the schema declares.
///
/// Every per-sheet container written, a second sheet, a named range, a filter
/// and a styled cell. It is deliberately not a realistic workbook: its job is
/// width, and the coverage assertion fails when it stops having it.
pub(super) fn workbook_of_every_key() -> TrameState {
    let mut state = TrameState::with_peer(ambre_core::PeerId(1));
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "hello".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 1,
        col: 0,
        text: "=1+1".to_owned(),
    });
    state.handle_command(TrameCommand::SetActiveCell {
        row: 0,
        col: 0,
        anchor_row: 0,
        anchor_col: 0,
        extra: Vec::new(),
        kind: SelectionKind::Cells,
    });
    state.handle_command(TrameCommand::SetCellStyle {
        // A size and a colour beside the bold, so the reading belt has a
        // measure and a colour to resolve (`SCHEMA-017`).
        patch: StylePatch {
            bold: Some(true),
            size: Some(Some(14)),
            fg: Some(Some(Rgb::from_bits(0x33_66_99))),
            ..Default::default()
        },
    });
    state.handle_command(TrameCommand::ResizeCol {
        version: ambre_bus::ViewportVersion(0),
        sheet_id: ambre_bus::SheetId(DEFAULT_SHEET.to_owned()),
        col: 0,
        width_px: 120.0,
    });
    state.handle_command(TrameCommand::ResizeRow {
        version: ambre_bus::ViewportVersion(0),
        sheet_id: ambre_bus::SheetId(DEFAULT_SHEET.to_owned()),
        row: 0,
        height_px: 33.0,
    });
    state.handle_command(TrameCommand::MergeCells {
        row: 3,
        col: 0,
        row_end: 4,
        col_end: 1,
    });
    state.handle_command(TrameCommand::DefineName {
        name: "N".to_owned(),
        reference: "A1".to_owned(),
    });
    state.handle_command(TrameCommand::ApplyFilter {
        sheet_id: ambre_bus::SheetId(DEFAULT_SHEET.to_owned()),
        col: 0,
        kind: FilterKind::Condition {
            op: FilterOp::Eq,
            value: "hello".to_owned(),
        },
    });
    state.handle_command(TrameCommand::CreateSheet {
        name: "Two".to_owned(),
    });
    state
}

#[test]
fn the_shape_a_build_writes_is_the_shape_frozen_for_its_version() {
    let state = workbook_of_every_key();
    let written = render(&shape_of(&state.doc));
    let Some(frozen) = frozen_section(SCHEMA_VERSION) else {
        panic!(
            "`schema/shapes.txt` carries no `== v{SCHEMA_VERSION}` section. \
             The version rose without a freeze; add the section below:\n{written}"
        );
    };
    assert_eq!(
        written, frozen,
        "the shape this build writes is not the shape frozen for \
         SCHEMA_VERSION {SCHEMA_VERSION}. A key, a value type or a `postcard` \
         layout moved, so raise SCHEMA_VERSION and freeze the new shape as its \
         own section (`SCHEMA-018`)."
    );
}

/// Every build of one version writes the same scaffolding (`AUTHOR-024`).
/// The identity hashes the scaffolding's bytes, so it moves with any of them.
#[test]
fn the_scaffold_a_build_writes_is_the_scaffold_frozen_for_its_version() {
    let written = format!("{:016x}\n", super::scaffold_identity());
    assert_eq!(
        section_of(SCAFFOLD, SCHEMA_VERSION).as_deref(),
        Some(written.as_str()),
        "the scaffolding this build writes is not the one frozen for \
         SCHEMA_VERSION {SCHEMA_VERSION} in `schema/scaffold.txt`. Another \
         build of that version writes it under another identity, and a delta \
         that waits on it never applies. Raise SCHEMA_VERSION and append a section."
    );
}

/// A workbook as `version` stored it, written under a fixed identity. `None`
/// writes no register, which is the layout before the register existed.
fn workbook_at(version: Option<u32>, write: impl FnOnce(&LoroDoc)) -> LoroDoc {
    let doc = LoroDoc::new();
    doc.set_peer_id(1)
        .expect("a fresh document takes a peer id");
    doc.get_map(SHEETS_MAP)
        .insert(DEFAULT_SHEET, "Sheet1")
        .expect("the sheet");
    write(&doc);
    if let Some(version) = version {
        doc.get_map(SCHEMA_META_MAP)
            .insert(SCHEMA_VERSION_KEY, i64::from(version))
            .expect("the register");
    }
    doc.commit();
    doc
}

/// The canonical workbook of the version a step leaves: that version's layout,
/// holding what the step rewrites. Each step reads its own, so a step added
/// later never moves what an earlier one writes.
fn canonical(from: u32) -> LoroDoc {
    match from {
        0 => workbook_at(None, |_| {}),
        // The flat filter keys the `1 -> 2` step folds.
        1 => workbook_at(Some(1), |doc| {
            let filters = doc.get_map(filters_map_name(DEFAULT_SHEET));
            for (key, value) in [
                ("col", "0000000000000001_00000000"),
                ("kind", "condition"),
                ("op", "gt"),
                ("value", "1"),
            ] {
                filters.insert(key, value).expect("a flat key");
            }
        }),
        // A positional formula the `2 -> 3` step names by its cells.
        2 => workbook_at(Some(2), |doc| {
            let uid = |counter| Uid {
                peer: ambre_core::PeerId(1),
                counter,
            };
            let rows = doc.get_movable_list(rows_list_name(DEFAULT_SHEET));
            for counter in 0..3 {
                rows.push(uid(counter).to_key_part()).expect("a row");
            }
            let cols = doc.get_movable_list(cols_list_name(DEFAULT_SHEET));
            for counter in 3..5 {
                cols.push(uid(counter).to_key_part()).expect("a column");
            }
            doc.get_map(cells_map_name(DEFAULT_SHEET))
                .insert(&cell_key(uid(0), uid(4)), "=SUM(A2:A3)+A2")
                .expect("the formula");
        }),
        _ => panic!("the step from version {from} has no canonical workbook: write one here"),
    }
}

/// Each step of the registry, run as the gate runs it on its canonical
/// workbook. One line per step: `from -> to` and what it wrote.
fn steps_written() -> Vec<String> {
    MIGRATIONS
        .iter()
        .map(|step| {
            let stored = canonical(step.from)
                .export(ExportMode::Snapshot)
                .expect("a snapshot");
            let scratch = LoroDoc::new();
            scratch.import(&stored).expect("the snapshot imports");
            let heads = scratch.oplog_frontiers();
            scratch
                .set_peer_id(migration_peer(heads.iter().map(|id| (id.peer, id.counter))).0)
                .expect("the migration identity");
            let before = scratch.oplog_vv();
            (step.apply)(&scratch).expect("the step runs on its canonical workbook");
            let written = scratch
                .export(ExportMode::updates(&before))
                .expect("the step's writes export");
            format!(
                "{} -> {} {:016x}",
                step.from,
                step.to,
                step_fingerprint(&written)
            )
        })
        .collect()
}

/// Every build of one version runs each migration step alike (`AUTHOR-024`).
/// A step stamps the version it reaches, so a raised version moves every line
/// and freezes a section of its own.
#[test]
fn each_migration_step_writes_what_its_version_froze() {
    let listed: String = steps_written()
        .iter()
        .map(|line| format!("{line}\n"))
        .collect();
    assert_eq!(
        section_of(STEPS, SCHEMA_VERSION).as_deref(),
        Some(listed.as_str()),
        "a migration step this build runs writes other operations than the \
         ones frozen for SCHEMA_VERSION {SCHEMA_VERSION} in \
         `schema/migrations.txt`. A peer on another build of that version \
         migrates the same workbook another way. Restore the step, or raise \
         SCHEMA_VERSION and append a section."
    );
}

#[test]
fn every_declared_key_appears_in_the_shape() {
    let state = workbook_of_every_key();
    let shape = shape_of(&state.doc);
    for key in declared_string_consts(SCHEMA_SOURCE) {
        assert!(
            shape.keys.contains(&key),
            "`{key}` is declared as a schema key and no line of the shape \
             carries it. Either the scenario stopped reaching it, or the key is \
             new and the scenario has to write it before the freeze can hold it."
        );
    }
    for prefix in declared_sheet_prefixes(SCHEMA_SOURCE) {
        let root = format!("{prefix}{ANY}");
        assert!(
            shape.keys.contains(&root),
            "`{prefix}{{sheet}}` is declared as a per-sheet container and the \
             scenario never writes one. The freeze cannot hold what no command \
             reaches."
        );
    }
}

#[test]
fn a_value_changing_leaves_the_shape_alone() {
    let first = workbook_of_every_key();
    let mut second = workbook_of_every_key();
    second.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "a different cell entirely".to_owned(),
    });
    assert_eq!(
        render(&shape_of(&first.doc)),
        render(&shape_of(&second.doc)),
        "the shape reads the types and not the values"
    );
}

#[test]
fn a_key_appearing_moves_the_shape() {
    let state = workbook_of_every_key();
    let before = render(&shape_of(&state.doc));
    state
        .doc
        .get_map(ambre_core::schema::SCHEMA_META_MAP)
        .insert("a_key_no_build_writes", 1_i64)
        .expect("the register takes a key");
    assert_ne!(
        before,
        render(&shape_of(&state.doc)),
        "a key appearing has to move the shape, or the freeze holds nothing"
    );
}

#[test]
fn a_value_type_changing_moves_the_shape() {
    let state = workbook_of_every_key();
    let before = render(&shape_of(&state.doc));
    // A cell is stored as its raw text; the same key under a number is the
    // drift a reviewer cannot be asked to spot.
    let LoroValue::Map(roots) = state.doc.get_deep_value() else {
        panic!("the document is a map of roots");
    };
    let cells = roots
        .keys()
        .find(|name| name.starts_with("cells_"))
        .expect("the scenario writes a cell");
    let map = state.doc.get_map(cells.as_str());
    let key = match map.get_value() {
        LoroValue::Map(entries) => entries.keys().next().cloned().expect("a cell is written"),
        _ => panic!("a cell register is a map"),
    };
    map.insert(&key, 1_i64).expect("the register takes a key");
    assert_ne!(
        before,
        render(&shape_of(&state.doc)),
        "a value type changing has to move the shape"
    );
}
