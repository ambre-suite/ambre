//! The code rules whose facts span more than one unit, joined over the
//! records of every unit the driver wrote (`ambre-spec-ui.md` section 2).
//!
//! The driver reads each view, each shell signal and each event arm with
//! types, and records what it holds as a fact. A shell lives in one crate
//! and the views it feeds in others, so the join waits for every unit.
//!
//! | Check | Rows |
//! | :--- | :--- |
//! | a view field of a type a shell signal carries | `UI-THREAD-059` |
//! | an event a relay writes a signal off while a view arm stores a field | `UI-THREAD-003` |
//! | a client build on the wasm target that unwinds on a panic | `ARCH-015` |
//! | a file a code rule closes that no compile reads | each row below |

use std::collections::BTreeMap;

use crate::records::Records;

/// A file each code rule of the driver closes, and the rows it holds. A rule
/// over a file that moved reads nothing, so a file no compile reads fails
/// closed.
pub(crate) const ANCHORS: &[(&str, &str)] = &[
    ("crates/ambre-canvas/src/render/mod.rs", "CANVAS-060"),
    ("crates/ambre-draw/src/lib.rs", "CANVAS-056"),
    (
        "crates/ambre-draw/src/kinetic.rs",
        "KINETIC-001, KINETIC-012",
    ),
    ("crates/ambre-prose/src/view/mod.rs", "KINETIC-001"),
    ("crates/ambre-toile/src/ui/viewport.rs", "KINETIC-001"),
    ("crates/ambre-trame/src/ui/view/scroll.rs", "KINETIC-001"),
    ("crates/ambre-murmure/src/view.rs", "KINETIC-001"),
    ("crates/ambre-toile/src/ui/view.rs", "KINETIC-001"),
    (
        "crates/ambre-trame/src/model/axis.rs",
        "ambre-spec-trame-schema.md section 5",
    ),
    (
        "crates/ambre-trame/src/logic/schema.rs",
        "ambre-spec-trame-schema.md section 5",
    ),
    ("crates/ambre-prose/src/view/walk.rs", "CANVAS-074"),
    ("crates/ambre-trame/src/render/walk.rs", "CANVAS-074"),
    ("crates/ambre-toile/src/ui/walk.rs", "CANVAS-074"),
    (
        "crates/ambre-app/src/lib.rs",
        "UI-THREAD-059, UI-THREAD-060",
    ),
    (
        "crates/ambre-trame/src/chrome/relay.rs",
        "UI-THREAD-003, UI-THREAD-059, UI-THREAD-060",
    ),
    (
        "crates/ambre-toile/src/chrome/relay.rs",
        "UI-THREAD-003, UI-THREAD-059, UI-THREAD-060",
    ),
    ("crates/ambre-bus/src/lib.rs", "BUS-029"),
    (
        "crates/ambre-server/src/routes/mod.rs",
        "SRV-179, SRV-180, SRV-181",
    ),
    ("crates/ambre-server/src/error.rs", "SRV-181, SRV-182"),
    ("crates/ambre-sync/src/http.rs", "SRV-179, SRV-180, SRV-182"),
    ("crates/ambre-logic/src/lib.rs", "ARCH-015"),
];

/// An event whose view arm keeps a field while the relay writes a signal,
/// the two holding different parts of it, and why (`UI-THREAD-003`).
const ONE_EVENT_COPY_ACCEPTED: &[(&str, &str)] = &[
    (
        "TrameEvent::SheetList -> sheets",
        "the view keeps the active sheet alone; the tab list is the strip's",
    ),
    (
        "TrameEvent::FilterApplied -> filter_popover",
        "the view keeps the filtered column and publishes the flag; the popover is chrome alone",
    ),
    (
        "TrameEvent::FilterCleared -> filter_popover",
        "the view keeps the filtered column and publishes the flag; the popover is chrome alone",
    ),
    (
        "ToileEvent::SelectionStats -> sel_stats",
        "the relay takes the band's reply alone; the view takes the bounds of a fit or a find focus",
    ),
    (
        "ToileEvent::SlideChanged -> presenter_slide",
        "the view keeps the glide; the panel keeps the slide's position",
    ),
    (
        "ToileEvent::SlideChanged -> presenter_notes",
        "the view keeps the glide; the panel keeps the speaker notes",
    ),
    (
        "ToileEvent::SlideChanged -> presenter_frame",
        "the view keeps the glide; the panel keeps the notes' frame",
    ),
];

/// One fact of a view rule, its value split at its first space.
struct Fact<'a> {
    /// The field, the variant or the signal the fact names.
    head: &'a str,
    /// The state type, the field chain or the signal it holds.
    rest: &'a str,
    /// Where the driver read it.
    at: &'a str,
}

/// Each fact of `name`.
fn split<'a>(records: &'a Records, name: &str) -> Vec<Fact<'a>> {
    records
        .facts
        .iter()
        .filter(|(n, _, _)| n == name)
        .filter_map(|(_, value, at)| {
            value.split_once(' ').map(|(head, rest)| Fact {
                head,
                rest,
                at: at.as_str(),
            })
        })
        .collect()
}

/// The refusals of the code rules joined over every unit.
pub(crate) fn views(records: &Records) -> Vec<String> {
    let mut refusals: Vec<String> = ANCHORS
        .iter()
        .filter(|(anchor, _)| !records.files.iter().any(|f| f.ends_with(anchor)))
        .map(|(anchor, rows)| {
            format!("{anchor}: no compile reads it, so {rows} reads nothing, and fails closed")
        })
        .collect();
    // UI-THREAD-059: the signal is the one copy of the state it carries.
    let mut signals: BTreeMap<&str, &str> = BTreeMap::new();
    for fact in split(records, "signal-state") {
        signals.entry(fact.rest).or_insert(fact.head);
    }
    if signals.is_empty() {
        refusals.push(
            "no shell holds a signal of a type of its own, so no view field is read (UI-THREAD-059)"
                .to_owned(),
        );
    }
    for fact in split(records, "view-state") {
        if let Some(signal) = signals.get(fact.rest) {
            refusals.push(format!(
                "{}: `{}` holds a `{}`, which the `{signal}` signal already holds \
                 (UI-THREAD-059)",
                fact.at, fact.head, fact.rest
            ));
        }
    }
    // UI-THREAD-003: no event feeds both a view field and a shell signal.
    let writes = split(records, "relay-writes");
    if writes.is_empty() {
        refusals.push(
            "no relay arm writes a signal off an event, so no view arm is read (UI-THREAD-003)"
                .to_owned(),
        );
    }
    for store in split(records, "view-stores") {
        for write in writes.iter().filter(|w| w.head == store.head) {
            let pair = format!("{} -> {}", store.head, write.rest);
            if !ONE_EVENT_COPY_ACCEPTED.iter().any(|(p, _)| *p == pair) {
                refusals.push(format!(
                    "{}: `{}` sets `self.{}` here, and {} writes the `{}` signal off it \
                     (UI-THREAD-003)",
                    store.at, store.head, store.rest, write.at, write.rest
                ));
            }
        }
    }
    // ARCH-015: the client build aborts on a panic, so no destructor runs.
    let panics: Vec<_> = records
        .facts
        .iter()
        .filter(|(n, _, _)| n == "wasm-panic")
        .collect();
    if panics.is_empty() {
        refusals.push(
            "no client unit on the wasm target was read, so its panic strategy is unknown (ARCH-015)"
                .to_owned(),
        );
    }
    for (_, strategy, unit) in panics {
        if strategy != "abort" {
            refusals.push(format!(
                "{unit}: the client build does not abort on a panic: `{strategy}` (ARCH-015)"
            ));
        }
    }
    refusals
}

#[cfg(test)]
#[path = "views_tests.rs"]
mod tests;
