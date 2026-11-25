//! The threads a removal from the channel names (`SYNC-321`).
//!
//! The `meta` names segments and amendments, and no thread. A segment names
//! the thread of each of its roots (`MURMURE-107`). The amendments of a
//! locked segment record the thread of a late first reply (`MURMURE-484`). Before a removal, the channel reads every
//! segment the `meta` lists and every amendments document no fold erased.
//! The census then refuses a relay listing that hides one of those threads.
//!
//! A document the channel does not hold opens for that read alone, one at a
//! time, so the read holds one copy at most. It closes once read.

use std::collections::HashSet;

use ambre_core::id::{Document, Id, Message, Segment};
use loro::LoroDoc;

use super::{ChannelState, amending, threading};
use crate::logic::delta::{self, Arrival, Rules};
use crate::logic::error::MurmureError;
use crate::logic::schema::{self, DocumentKind, HeaderKey};

/// The read of the threads a removal names, while it runs.
#[derive(Debug, Default)]
pub(super) struct Sweep {
    /// The thread documents the documents read so far name.
    threads: Vec<Id<Document>>,
    /// The link receipts the documents read so far hold (`MURMURE-739`).
    receipts: Vec<ambre_bus::LinkReceipt>,
    /// The segments read, with their amendments.
    swept: HashSet<Id<Segment>>,
    /// The document open for the sweep alone, while its bytes arrive.
    reading: Option<Reading>,
}

/// A document open for the sweep alone.
#[derive(Debug)]
struct Reading {
    document_id: Id<Document>,
    /// The segment it is, or the segment whose amendments it is.
    segment: Id<Segment>,
    part: Part,
    /// Its payloads, in the order they came.
    arrivals: Vec<Arrival>,
}

/// What a document open for the sweep is to its segment.
#[derive(Debug)]
enum Part {
    /// The segment itself, with the amendments the `meta` names for it.
    Segment { amendments: Option<Id<Document>> },
    /// Its amendments. The segment checks the author of each amendment: the
    /// copy read just before, or `None` for the one the channel holds.
    Amendments { segment: Option<LoroDoc> },
}

/// The next move of the read of the threads a removal names.
#[derive(Debug, PartialEq, Eq)]
pub enum SweepStep {
    /// Open this document for the read alone. Its bytes go to
    /// [`ChannelState::apply_remote`] or [`ChannelState::apply_signed`], and
    /// [`ChannelState::sweep_ready`] reads it once they are all in.
    Open(Id<Document>),
    /// A document is on its way: the read goes on once it is ready.
    Wait,
    /// Every document is read: what the sealed state of the channel names.
    /// The read ends.
    Done(Swept),
}

/// What the read of the threads a removal names found, each document once.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Swept {
    /// The segments and amendments the `meta` names (`SYNC-320`).
    pub documents: Vec<Id<Document>>,
    /// The threads those name and the `meta` does not (`SYNC-321`). Only a
    /// thread may pass a listing under an owner's erasure (`SYNC-322`).
    pub threads: Vec<Id<Document>>,
    /// The link receipts those documents hold beside their threads. A thread
    /// under a valid one takes no erasure (`SYNC-370`, `MURMURE-739`).
    pub receipts: Vec<ambre_bus::LinkReceipt>,
}

impl ChannelState {
    /// The next move of the read of the threads a removal names
    /// (`SYNC-321`). The first call starts the read.
    ///
    /// A segment or an amendments document the view is loading makes the
    /// read wait: its ready, or its failure, calls this again.
    pub fn sweep_step(&mut self) -> SweepStep {
        let entries = amending::swept_entries(&self.meta.doc);
        let mut sweep = self.sweep.take().unwrap_or_default();
        let step = self.next_sweep(&mut sweep, entries);
        if !matches!(step, SweepStep::Done(_)) {
            self.sweep = Some(sweep);
        }
        step
    }

    fn next_sweep(
        &self,
        sweep: &mut Sweep,
        entries: Vec<(Id<Segment>, Option<Id<Document>>)>,
    ) -> SweepStep {
        if sweep.reading.is_some() {
            return SweepStep::Wait;
        }
        for (segment, amendments) in entries {
            if sweep.swept.contains(&segment) {
                continue;
            }
            let loading = self.pending.contains_key(&segment)
                || amendments.is_some_and(|id| self.amendments_pending.contains_key(&id));
            if loading {
                return SweepStep::Wait;
            }
            let Some(doc) = self.doc(segment.document_id()) else {
                let document_id = segment.document_id();
                sweep.reading = Some(Reading::of(
                    document_id,
                    segment,
                    Part::Segment { amendments },
                ));
                return SweepStep::Open(document_id);
            };
            sweep.add(threading::named_threads(doc));
            sweep.receipts.extend(threading::named_receipts(doc));
            if let Some(id) = amendments {
                match self.amendments.get(&segment).filter(|held| held.id == id) {
                    Some(held) => {
                        sweep.add(amending::recorded_threads(&held.doc));
                        sweep
                            .receipts
                            .extend(amending::recorded_receipts(&held.doc));
                    }
                    None => {
                        let part = Part::Amendments { segment: None };
                        sweep.reading = Some(Reading::of(id, segment, part));
                        return SweepStep::Open(id);
                    }
                }
            }
            sweep.swept.insert(segment);
        }
        let documents = amending::named_documents(&self.meta.doc);
        let threads = sweep
            .threads
            .drain(..)
            .filter(|thread| !documents.contains(thread))
            .collect();
        let receipts = std::mem::take(&mut sweep.receipts);
        SweepStep::Done(Swept {
            documents,
            threads,
            receipts,
        })
    }

    /// Whether `document_id` is open for the read of the threads a removal
    /// names, and its bytes still arriving.
    pub fn is_swept(&self, document_id: Id<Document>) -> bool {
        self.sweep
            .as_ref()
            .and_then(|sweep| sweep.reading.as_ref())
            .is_some_and(|reading| reading.document_id == document_id)
    }

    /// Keep `arrival` for the document the sweep reads. Returns `false` when
    /// the sweep reads no such document.
    pub(super) fn sweep_arrival(
        &mut self,
        document_id: Id<Document>,
        arrival: impl FnOnce() -> Arrival,
    ) -> bool {
        let Some(reading) = self
            .sweep
            .as_mut()
            .and_then(|sweep| sweep.reading.as_mut())
            .filter(|reading| reading.document_id == document_id)
        else {
            return false;
        };
        reading.arrivals.push(arrival());
        true
    }

    /// The document the sweep reads has all its bytes: read the threads it
    /// names. The caller closes it.
    ///
    /// Returns the amendments to open next for the segment just read, if
    /// any. Otherwise the caller asks [`ChannelState::sweep_step`] again.
    ///
    /// # Errors
    /// [`MurmureError::NotHeld`] when the sweep reads no such document.
    /// Every refusal of the header check and of the authorship check.
    /// [`MurmureError::ForeignSegment`] for amendments of another segment.
    /// An error ends the sweep, so the removal fails.
    pub fn sweep_ready(
        &mut self,
        document_id: Id<Document>,
    ) -> Result<Option<Id<Document>>, MurmureError> {
        let read = self.read_swept(document_id);
        if read.is_err() {
            self.sweep = None;
        }
        read
    }

    fn read_swept(
        &mut self,
        document_id: Id<Document>,
    ) -> Result<Option<Id<Document>>, MurmureError> {
        let reading = self
            .sweep
            .as_mut()
            .and_then(|sweep| sweep.reading.take_if(|r| r.document_id == document_id))
            .ok_or(MurmureError::NotHeld(document_id))?;
        let segment = reading.segment;
        let (threads, receipts, next) = match reading.part {
            Part::Segment { amendments } => {
                let doc = schema::admit_with(
                    DocumentKind::Segment,
                    self.channel_id,
                    document_id,
                    |doc| {
                        delta::import_arrivals(doc, reading.arrivals, document_id, Rules::Segment)
                    },
                )?;
                let threads = threading::named_threads(&doc);
                let receipts = threading::named_receipts(&doc);
                let next = amendments.map(|id| {
                    let part = Part::Amendments { segment: Some(doc) };
                    Reading::of(id, segment, part)
                });
                (threads, receipts, next)
            }
            Part::Amendments { segment: read } => {
                let segment_doc = read
                    .as_ref()
                    .or_else(|| self.doc(segment.document_id()))
                    .ok_or(MurmureError::NotHeld(segment.document_id()))?;
                let rules = Rules::Amendments {
                    segment: segment_doc,
                };
                let doc = schema::admit_with(
                    DocumentKind::Amendments,
                    self.channel_id,
                    document_id,
                    |doc| delta::import_arrivals(doc, reading.arrivals, document_id, rules),
                )?;
                let named = schema::string(&schema::header(&doc), HeaderKey::Segment)
                    .and_then(|s| Id::<Segment>::parse_str(&s).ok());
                if named != Some(segment) {
                    return Err(MurmureError::ForeignSegment(document_id));
                }
                let receipts = amending::recorded_receipts(&doc);
                (amending::recorded_threads(&doc), receipts, None)
            }
        };
        let sweep = self
            .sweep
            .as_mut()
            .ok_or(MurmureError::NotHeld(document_id))?;
        sweep.add(threads);
        sweep.receipts.extend(receipts);
        let opened = next.as_ref().map(|reading| reading.document_id);
        match next {
            Some(reading) => sweep.reading = Some(reading),
            None => {
                sweep.swept.insert(segment);
            }
        }
        Ok(opened)
    }

    /// End the read of the threads a removal names, if one runs. Returns the
    /// document it holds open, which the caller closes.
    pub fn end_sweep(&mut self) -> Option<Id<Document>> {
        self.sweep
            .take()
            .and_then(|sweep| sweep.reading)
            .map(|reading| reading.document_id)
    }
}

impl Sweep {
    /// Keep each thread of `named` once.
    fn add(&mut self, named: Vec<(Id<Message>, Id<Document>)>) {
        for (_, thread) in named {
            if !self.threads.contains(&thread) {
                self.threads.push(thread);
            }
        }
    }
}

impl Reading {
    fn of(document_id: Id<Document>, segment: Id<Segment>, part: Part) -> Self {
        Self {
            document_id,
            segment,
            part,
            arrivals: Vec::new(),
        }
    }
}
