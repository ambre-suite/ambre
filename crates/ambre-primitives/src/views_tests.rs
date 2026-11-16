use super::{ANCHORS, views};
use crate::records::Records;

/// The records of `facts`, with every file the code rules close read.
fn facts(facts: &[(&str, &str, &str)]) -> Records {
    let mut records = Records::default();
    for (name, value, at) in facts {
        records.add(&format!("fact\t{name}\t{value}\t{at}"));
    }
    for (anchor, _) in ANCHORS {
        records.add(&format!("file\t/work/{anchor}"));
    }
    records
}

/// The facts of a tree that keeps each rule.
const KEPT: &[(&str, &str, &str)] = &[
    (
        "signal-state",
        "zoom ambre_chrome::status::ZoomFactor",
        "crates/ambre-app/src/lib.rs:203",
    ),
    (
        "view-state",
        "GridView.layout ambre_trame::render::layout::GridLayout",
        "crates/ambre-trame/src/ui/grid/mod.rs:9",
    ),
    (
        "relay-writes",
        "TrameEvent::SheetList sheets",
        "crates/ambre-trame/src/chrome/relay.rs:12",
    ),
    (
        "view-stores",
        "TrameEvent::SheetList active",
        "crates/ambre-trame/src/ui/view/mod.rs:40",
    ),
    ("wasm-panic", "abort", "ambre-app"),
];

// A tree that keeps each rule passes. A view field of a signal's type is
// refused, read in another crate than the shell (`UI-THREAD-059`). So is a
// view arm that stores off an event the relay writes a signal off. A pair on
// the accepted list stays free (`UI-THREAD-003`).
#[test]
fn a_view_field_or_a_view_arm_beside_a_signal_of_another_crate_is_refused() {
    assert_eq!(views(&facts(KEPT)), Vec::<String>::new());
    let mut copied = KEPT.to_vec();
    copied.push((
        "view-state",
        "ToileView.zoom ambre_chrome::status::ZoomFactor",
        "crates/ambre-toile/src/ui/view.rs:7",
    ));
    copied.push((
        "relay-writes",
        "ToileEvent::Zoomed zoom",
        "crates/ambre-toile/src/chrome/relay.rs:30",
    ));
    copied.push((
        "view-stores",
        "ToileEvent::Zoomed camera.zoom",
        "crates/ambre-toile/src/ui/view.rs:90",
    ));
    assert_eq!(
        views(&facts(&copied)),
        vec![
            "crates/ambre-toile/src/ui/view.rs:7: `ToileView.zoom` holds a \
             `ambre_chrome::status::ZoomFactor`, which the `zoom` signal already holds \
             (UI-THREAD-059)",
            "crates/ambre-toile/src/ui/view.rs:90: `ToileEvent::Zoomed` sets `self.camera.zoom` \
             here, and crates/ambre-toile/src/chrome/relay.rs:30 writes the `zoom` signal off it \
             (UI-THREAD-003)",
        ]
    );
}

// A client unit that unwinds is refused, and so is a run that read no client
// unit: the rule never holds by reading nothing (`ARCH-015`).
#[test]
fn a_client_build_that_unwinds_or_a_run_that_read_none_is_refused() {
    let mut unwinds = KEPT.to_vec();
    unwinds.push(("wasm-panic", "unwind", "ambre-logic"));
    assert_eq!(
        views(&facts(&unwinds)),
        vec!["ambre-logic: the client build does not abort on a panic: `unwind` (ARCH-015)"]
    );
    assert_eq!(views(&facts(&[])).len(), 3);
}

// A file a code rule closes that no compile reads fails closed
// (`KINETIC-012`).
#[test]
fn a_closed_file_no_compile_reads_is_refused() {
    let mut records = facts(KEPT);
    records
        .files
        .retain(|f| !f.ends_with("crates/ambre-draw/src/kinetic.rs"));
    assert_eq!(
        views(&records),
        vec![
            "crates/ambre-draw/src/kinetic.rs: no compile reads it, so KINETIC-001, \
             KINETIC-012 reads nothing, and fails closed"
        ]
    );
}

// A listed Logic path to the view code that some unit takes passes. An entry
// no unit takes is refused, so the list only shrinks on its own
// (`UI-THREAD-064`).
#[test]
fn a_listed_logic_path_to_the_view_code_that_no_unit_takes_is_refused() {
    let taken = "crates/ambre-toile/src/logic/state/export.rs -> ambre_toile::ui::render";
    let left = "crates/ambre-toile/src/logic/state/import.rs -> ambre_canvas::theme";
    let mut kept = KEPT.to_vec();
    kept.push(("logic-view-listed", taken, "-"));
    kept.push((
        "logic-view-reached",
        taken,
        "crates/ambre-toile/src/logic/state/export.rs:41",
    ));
    assert_eq!(views(&facts(&kept)), Vec::<String>::new());
    kept.push(("logic-view-listed", left, "-"));
    assert_eq!(
        views(&facts(&kept)),
        vec![format!(
            "{left}: an entry of LOGIC_VIEW_REACH that no unit takes; remove it from \
             crates/ambre-primitives/driver/src/logic.rs (UI-THREAD-064)"
        )]
    );
}
