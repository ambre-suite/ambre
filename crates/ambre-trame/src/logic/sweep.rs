// ambre-trame/src/logic/sweep.rs
// The removal for good of the lines an ended session hid (`TRAME-305`)
//
// Part of the `TrameState` impl. Submodule of `logic`, so it reaches the
// private fields and helpers of the parent module through `use super::*`.
//
// A block delete hides its lines under one register, so the session that hid
// them can undo it (`TRAME-302`). Once that session has ended, no undo can
// bring them back, and the elements and their cells only weigh on the
// document. The sweep removes them, as the Prose sweep removes a hidden block
// (`ambre-spec-undo.md section 3`): the same rule decides whether a session
// runs (`ambre_crdt::session`). It runs off the gesture, one slice per pass of the
// Logic thread, as an Excel import does (`TRAME-IO-013`). Only an editor
// starts it: a viewer writes nothing.

use ambre_core::crdt::CommitOrigin;
use ambre_core::id::{Id, User};
use ambre_core::index::Len;

use super::line_cells;
use super::remote::{LocalNote, Readers};
use super::*;
use crate::formula::engine::Removal;
use crate::model::{Axes, Slot, SweepLeft, Swept, UidLeft};

/// The elements one slice of a sweep removes at most, both axes and every
/// sheet together. A slice of this many rows on the sheet of 10k formulas
/// holds the structural edit row of `ambre-spec-budgets.md section 1`
/// (`TRAME-307`).
pub(super) const SWEEP_SLICE: Len<Slot> = Len::new(32);

ambre_core::define_flag! {
    /// Whether a sweep of the hidden lines runs.
    pub(super) enum Sweeping {
        /// No sweep runs.
        Idle,
        /// A sweep runs: the next pass removes its next slice.
        Running,
    }
}

ambre_core::define_flag! {
    /// Whether this session owes a newcomer its presence.
    pub(super) enum Greeting {
        /// Every session met has heard from this one.
        Given,
        /// A session joined that has not heard from this one yet.
        Owed,
    }
}

impl TrameState {
    /// Start a sweep of the lines an ended session hid, one slice per call of
    /// [`resume`](Self::resume) (`TRAME-305`). The Logic thread calls it
    /// after a compaction's snapshot, for an editor alone. A sweep runs when
    /// a register is left to read, and [`has_pending_work`] then says so.
    ///
    /// [`has_pending_work`]: Self::has_pending_work
    pub fn start_sweep(&mut self) {
        let hides = self
            .sheets_by_idx
            .iter()
            .any(|sheet| Axes::of(&self.doc, sheet).hides_any());
        self.sweep = if hides {
            Sweeping::Running
        } else {
            Sweeping::Idle
        };
    }

    /// Whether a sweep runs.
    pub(super) fn sweeping(&self) -> bool {
        self.sweep == Sweeping::Running
    }

    /// Remove the next slice of what ended sessions hid, on every sheet, and
    /// carry the caches across it. The commit is no step of the history. The
    /// sweep stops once nothing is left to remove.
    ///
    /// Never inlined: `scripts/instructions.py` counts it by its name.
    #[inline(never)]
    pub(super) fn sweep_slice(&mut self) -> TrameOutcome {
        let order = self.sheet_order_ids();
        drop(self.take_imported());
        self.engine.changed.clear();
        let own = ambre_core::PeerId::new(self.doc.peer_id());
        let mut budget = SWEEP_SLICE;
        let mut more = SweepLeft::Done;
        let mut wrote = false;
        for sheet in self.sheets_by_idx.clone() {
            let read;
            let index = if sheet == self.active_sheet {
                &self.index
            } else {
                read = self.build_sheet_index(&sheet);
                &read
            };
            let runs = |peer| {
                ambre_crdt::session::session_runs(&self.doc, own, peer, |user| {
                    self.live.contains(user)
                })
            };
            let axes = Axes::of(&self.doc, &sheet);
            let rows = axes.sweep::<Row>(index, &runs, budget);
            budget = budget.saturating_sub(swept_len(&rows));
            let cols = axes.sweep::<Col>(index, &runs, budget);
            budget = budget.saturating_sub(swept_len(&cols));
            for swept in [&rows, &cols] {
                if swept.more == SweepLeft::More {
                    more = SweepLeft::More;
                }
            }
            if rows.lines.is_empty() && cols.lines.is_empty() {
                continue;
            }
            wrote = true;
            self.purge_swept(&sheet, &rows, Axis::Row);
            self.purge_swept(&sheet, &cols, Axis::Col);
            if budget == Len::ZERO {
                more = SweepLeft::More;
                break;
            }
        }
        if !wrote {
            // The generations of every swept line go last, in one walk of
            // each record for the whole sweep, not one per slice.
            self.sweep = Sweeping::Idle;
            let record = std::mem::take(&mut self.sweep_record);
            if record.is_empty() {
                return TrameOutcome::display_only(Vec::new());
            }
            for (sheet, gone) in &record {
                Axes::of(&self.doc, sheet).purge_record(gone);
            }
        } else if more == SweepLeft::Done && self.sweep_record.is_empty() {
            self.sweep = Sweeping::Idle;
        }
        // The commit is noted as an import is, so the caches follow what it
        // wrote as they follow a peer's sweep.
        self.note_local(LocalNote::Noted);
        self.doc
            .set_next_commit_origin(CommitOrigin::Sweep.as_str());
        self.doc.commit();
        self.note_local(LocalNote::Ignored);
        let written = self.take_imported();
        self.leave_a_deleted_sheet(&order);
        if self.carries(&written) {
            let (conflicts, repaint) =
                self.carry_imported(&written, Readers::OnMove, Removal::Drop);
            let display = self.merge_events(conflicts, repaint);
            TrameOutcome {
                display,
                update: self.flush_delta(),
                ephemeral: None,
            }
        } else {
            self.after_structural(None)
        }
    }

    /// Delete every cell and style keyed by an element `swept` removed from
    /// axis `axis` of `sheet`. Delete what a uid no element carries any more
    /// keys too: its size and its filter mark. Its generations go in the last
    /// slice of the sweep.
    fn purge_swept(&mut self, sheet: &Id<Sheet>, swept: &Swept, axis: Axis) {
        if swept.lines.is_empty() {
            return;
        }
        for root in line_cells::PURGED {
            let keys = line_cells::read(&self.lines).keys_on(
                root,
                sheet,
                axis,
                swept.lines.iter().flat_map(|line| &line.removed.elements),
            );
            if keys.is_empty() {
                continue;
            }
            let map = self.doc.get_map(SheetContainer::of(root, sheet).as_str());
            for key in keys {
                let _ = map.delete(key.as_str());
                if let Some(held) = self.unplaced.get_mut(sheet) {
                    held.remove(&key);
                }
            }
        }
        let gone: Vec<Uid> = swept
            .lines
            .iter()
            .filter(|line| line.uid_left == UidLeft::Gone)
            .map(|line| line.removed.uid)
            .collect();
        if gone.is_empty() {
            return;
        }
        let line_maps = match axis {
            Axis::Row => vec![SheetRoot::RowHeights, SheetRoot::FilterHidden],
            Axis::Col => vec![SheetRoot::ColWidths],
        };
        for root in line_maps {
            let map = self.doc.get_map(SheetContainer::of(root, sheet).as_str());
            if map.is_empty() {
                continue;
            }
            for uid in &gone {
                let key = uid.to_key_part();
                if map.get(&key).is_some() {
                    let _ = map.delete(&key);
                }
            }
        }
        self.sweep_record
            .entry(sheet.clone())
            .or_default()
            .extend(gone);
    }

    /// Note that `user` has a session open on this workbook, from a presence
    /// sample it sent. A newcomer is owed this session's own sample.
    pub fn note_live(&mut self, user: Id<User>) {
        if self.live.insert(user) {
            self.owe_greeting = Greeting::Owed;
        }
    }

    /// Forget `user`, on the relay's word that its last session left
    /// (`PRESENCE-014`). A sweep may then remove what its sessions hid.
    pub fn drop_live(&mut self, user: Id<User>) {
        self.live.remove(&user);
    }

    /// The sample that tells the other sessions this one is open. It goes
    /// once the workbook is ready, so a session that never moves still counts
    /// as running (`TRAME-305`). Trame draws no collaborator yet, so the
    /// sample carries nothing.
    #[must_use]
    pub fn announce(&self) -> ambre_core::Payload<ambre_core::EphemeralBytes> {
        ambre_core::Payload::new(Vec::new())
    }

    /// The answer owed to a session that joined since this one last spoke.
    /// Taken once per newcomer.
    pub fn take_greeting(&mut self) -> Option<ambre_core::Payload<ambre_core::EphemeralBytes>> {
        if std::mem::replace(&mut self.owe_greeting, Greeting::Given) == Greeting::Given {
            return None;
        }
        Some(self.announce())
    }
}

/// How many elements one axis of a slice removed.
fn swept_len(swept: &Swept) -> Len<Slot> {
    Len::of(
        swept
            .lines
            .iter()
            .map(|line| line.removed.elements.len())
            .sum(),
    )
}
