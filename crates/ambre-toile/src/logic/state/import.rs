//! Atomic foreign-file import into [`ToileState`](super::ToileState)
//! (`toile-import-export`). An [`ImportBatch`](crate::logic::interop::ImportBatch)
//! produced by a pure converter is applied here as ONE undo group. The pass
//! order resolves every slot / group reference before it is used. It is
//! creates -> styles -> groups -> connectors -> media -> `rebuild_caches`.
//!
//! A large batch is applied in slices of [`IMPORT_SLICE`] units, one commit
//! each, so the Logic thread answers other messages between them
//! (`TOILE-IO-028`). One Loro undo group spans every slice.

use ambre_core::Count;
use ambre_core::Validated;
use ambre_core::hash::HashMap;
use ambre_core::index::Pos;

use crate::message::ToileConstruct;
use ambre_bus::RestEvent;
use ambre_bus::interop::{
    ConversionProgress, Direction, InteropAnswer, InteropError, Occurrences, Outcome, Steps,
};
use ambre_core::Seq;
use kurbo::Point;
use loro::{LoroMap, LoroValue};

use crate::logic::command::{
    ImportSlot, InitContent, MediaRef, Request, ToileCommand, ToileEvent, ToileOutcome,
    ToileRequest,
};
use crate::logic::interop::{
    ContentHash, ImportBatch, ImportConnector, ImportElement, ImportError, ImportFrame,
    ImportFrameRank, ImportGroup, ImportGroupRank, ImportRef, ImportReport, Limits, MediaBlob,
    import_board, import_raster, import_svg, place_batch_at,
};
use crate::message::ElementStyle;
use ambre_core::id::{Element, Id};

use super::ToileState;
use ambre_core::crdt::CrdtName as _;

/// A JPEG picture whose thumbnail the converter thread decodes. The file and
/// its drop point wait here, and nothing is applied until the answer lands
/// (`TOILE-IO-131`).
pub(super) struct PendingPicture {
    data: ambre_core::Payload<ambre_client_core::FileBytes>,
    size: ambre_core::Size<ambre_core::Px<ambre_core::Image>>,
    at: Point,
}

/// What a converter module read for the import in flight.
pub(super) enum ConvertedFile {
    /// The drawing of an SVG file, or why the module gave none.
    Drawing(Result<ambre_convert::images::SvgTree, InteropError>),
    /// The pixels of a WebP picture, or why the module gave none.
    Picture(Result<ambre_convert::images::Pixels, InteropError>),
    /// The board of a Figma or a Miro export, or why the module gave none.
    Board(Result<ambre_convert::boards::BoardTree, InteropError>),
    /// The deck the Office module read from a presentation, with the
    /// thumbnail of each JPEG picture, or why the module gave none.
    Deck(Result<ReadDeck, InteropError>),
}

/// The deck the Office module read, and the thumbnails it decoded: one per
/// picture of the deck, in slide order then item order.
pub(super) struct ReadDeck {
    pub(super) deck: ambre_pptx::model::ImportedDeck,
    pub(super) thumbs: Vec<Option<ambre_convert::images::Pixels>>,
}
use super::names::{Fields, MediaKey, Root};

/// Rewrite the pending `slot` to the real blake2b `hash` in both the `media`
/// blob at `slot` and every `image` element that referenced it
/// (`toile-media`). Called as each upload confirmation lands, so by the time
/// the batch is applied no pending slot survives.
fn rewrite_hash(batch: &mut ImportBatch, slot: Pos<ImportSlot>, hash: &Validated<ContentHash>) {
    let pending = MediaRef::Pending(slot);
    let stored = MediaRef::Stored(hash.clone());
    if let Some(blob) = slot.to_index().and_then(|i| batch.media.get_mut(i)) {
        blob.media = stored.clone();
    }
    for cmd in &mut batch.commands {
        if let ToileCommand::CreateElement {
            content: InitContent::Image(media),
            ..
        } = cmd
            && *media == pending
        {
            *media = stored.clone();
        }
    }
}

/// An import applied only once every embedded media blob has a real content hash
/// back from Sync (`toile-media`). Holds the fully converted batch and the set
/// of upload `Seq<Request>`s still outstanding. Each id maps to the `media` slot
/// it will resolve.
pub(super) struct PendingImport {
    batch: ImportBatch,
    /// Outstanding `UploadMedia` request -> the `batch.media` index it fills.
    outstanding: HashMap<Seq<Request>, Pos<ImportSlot>>,
}

/// A number of import units. A unit is one create, one style, one group,
/// one connector or one frame.
pub enum ImportUnits {}

impl ambre_core::NarrowCount for ImportUnits {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("ImportUnitCount");
}

/// A sliced import reports its progress in its own units (`INTEROP-008`).
impl ambre_core::Recounts<ImportUnits> for Steps {}

/// How many entries the board's `media` map holds, or a seed added.
pub enum MediaEntries {}

impl ambre_core::NarrowCount for MediaEntries {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("MediaEntryCount");
}

/// How many imports wait on their media uploads.
pub enum PendingImports {}

impl ambre_core::NarrowCount for PendingImports {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("PendingImportCount");
}

/// How many units one slice of an import applies (`TOILE-IO-028`). A slice
/// size is a bound, not a count: nothing counts it up. A running import only
/// adds it to the units it has applied.
#[derive(Debug, Clone, Copy)]
pub(super) struct ImportSlice(Count<ImportUnits>);

impl ImportSlice {
    /// The units applied once one more slice runs after `done` of them.
    fn end_after(self, done: Count<ImportUnits>) -> Count<ImportUnits> {
        done.plus(self.get())
    }

    /// The units one slice applies.
    pub(super) const fn get(self) -> Count<ImportUnits> {
        self.0
    }
}

/// The slice a running import applies per call. The `import` bench reads its
/// cost against the keystroke pass row of `ambre-spec-budgets.md`.
pub(super) const IMPORT_SLICE: ImportSlice = ImportSlice(Count::<ImportUnits>::new(64));

/// How much of a running import one call applies.
#[derive(Clone, Copy)]
enum ImportPace {
    /// One slice of [`IMPORT_SLICE`] units, so the board's other commands
    /// wait no longer than that.
    Slice,
    /// Every unit left, before an edit or a second import.
    Whole,
}

/// An import being applied one slice at a time (`TOILE-IO-028`). The Logic
/// thread resumes it between its messages, so a large file never holds the
/// board's other commands back for longer than one slice.
pub(super) struct ImportJob {
    /// The creates not applied yet, in slot order.
    commands: std::vec::IntoIter<ToileCommand>,
    /// How many creates the batch carried.
    creates: Count<ImportUnits>,
    styles: Vec<Option<ElementStyle>>,
    rotations: Vec<ambre_core::Radians>,
    groups: Vec<ImportGroup>,
    frames: Vec<ImportFrame>,
    connectors: Vec<ImportConnector>,
    turns: Vec<ambre_core::Radians>,
    media: Vec<MediaBlob>,
    report: ImportReport,
    /// The element each create slot minted, `None` when it was rejected.
    slot_ids: Vec<Option<Id<Element>>>,
    /// The group each group entry built, `None` when it was not.
    group_ids: Vec<Option<Id<Element>>>,
    /// Units applied so far, across the four phases in order.
    done: Count<ImportUnits>,
}

impl ImportJob {
    fn new(batch: ImportBatch) -> Self {
        let creates = batch.commands.len();
        Self {
            commands: batch.commands.into_iter(),
            creates: Count::<ImportUnits>::of(creates),
            styles: batch.styles,
            rotations: batch.rotations,
            group_ids: Vec::with_capacity(batch.groups.len()),
            groups: batch.groups,
            frames: batch.frames,
            connectors: batch.connectors,
            turns: batch.turns,
            media: batch.media,
            report: batch.report,
            slot_ids: Vec::with_capacity(creates),
            done: Count::<ImportUnits>::ZERO,
        }
    }

    fn total(&self) -> Count<ImportUnits> {
        self.creates
            .plus(Count::<ImportUnits>::of(self.styles.len()))
            .plus(Count::<ImportUnits>::of(self.groups.len()))
            .plus(Count::<ImportUnits>::of(self.frames.len()))
            .plus(Count::<ImportUnits>::of(self.connectors.len()))
    }

    /// Where the next unit falls: its phase, and its rank in that phase. A
    /// rank is what the earlier phases leave of `done`, so the subtraction
    /// runs here alone, on the lengths of the phase vectors.
    fn step(&self) -> ImportStep {
        let done = usize::try_from(self.done.get()).unwrap_or(usize::MAX);
        let creates = usize::try_from(self.creates.get()).unwrap_or(usize::MAX);
        let Some(rank) = done.checked_sub(creates) else {
            return ImportStep::Create(Pos::<ImportElement>::from_usize(done));
        };
        if rank < self.styles.len() {
            return ImportStep::Style(Pos::<ImportElement>::from_usize(rank));
        }
        let rank = rank - self.styles.len();
        if rank < self.groups.len() {
            return ImportStep::Group(Pos::<ImportGroupRank>::from_usize(rank));
        }
        let rank = rank - self.groups.len();
        if rank < self.frames.len() {
            return ImportStep::Frame(Pos::<ImportFrameRank>::from_usize(rank));
        }
        self.connectors
            .get(rank - self.frames.len())
            .cloned()
            .map_or(ImportStep::Past, ImportStep::Connector)
    }
}

/// The phase of one import unit, with what that phase needs to apply it.
enum ImportStep {
    /// The create at this slot.
    Create(Pos<ImportElement>),
    /// The style of the element at this slot.
    Style(Pos<ImportElement>),
    /// The group at this rank.
    Group(Pos<ImportGroupRank>),
    /// The frame at this rank of the batch's frames.
    Frame(Pos<ImportFrameRank>),
    /// This connector.
    Connector(ImportConnector),
    /// No unit: the job is past its last frame.
    Past,
}

impl ToileState {
    /// Map the drawing the images module read from a dropped SVG document,
    /// and apply it (`toile-import-export`). `Err` carries the module's
    /// refusal, or why it did not load.
    pub(super) fn import_svg_drawing(
        &mut self,
        read: Result<ambre_convert::images::SvgTree, InteropError>,
        at: Point,
    ) -> ToileOutcome {
        match read {
            Ok(tree) => self.begin_import(import_svg(&tree, &Limits::default()), at),
            Err(error) => Self::interop_failed(Direction::Import, error),
        }
    }

    /// Map the board the images module read from a Figma or a Miro export,
    /// and apply it (`toile-import-export`). `Err` carries the module's
    /// refusal, or why it did not load.
    pub(super) fn import_board_file(
        &mut self,
        read: Result<ambre_convert::boards::BoardTree, InteropError>,
        at: Point,
    ) -> ToileOutcome {
        match read {
            Ok(tree) => self.begin_import(import_board(&tree, &Limits::default()), at),
            Err(error) => Self::interop_failed(Direction::Import, error),
        }
    }

    /// Map the deck the Office module read from a presentation onto frames
    /// of the deck, and apply it (`toile-pptx-import`). `format` names the
    /// file in the report. `Err` carries the module's refusal, or why it did
    /// not load.
    pub(super) fn import_deck_file(
        &mut self,
        read: Result<ReadDeck, InteropError>,
        format: ambre_bus::interop::FileFormat,
        at: Point,
    ) -> ToileOutcome {
        match read {
            Ok(read) => self.begin_import(
                crate::logic::interop::import_deck(
                    &read.deck,
                    &read.thumbs,
                    format,
                    &Limits::default(),
                ),
                at,
            ),
            Err(error) => Self::interop_failed(Direction::Import, error),
        }
    }

    /// Convert a dropped raster image and apply it (`toile-import-export`). The mime
    /// is re-derived from the bytes by the decoder; the parameter is accepted for
    /// the command surface but not needed here.
    ///
    /// A JPEG is decoded on the converter thread. Its header is held to the
    /// bound here. The board then waits for the thumbnail, with nothing
    /// applied (`TOILE-IO-131`).
    pub(super) fn import_image_bytes(
        &mut self,
        bytes: &ambre_core::Payload<ambre_client_core::FileBytes>,
        at: Point,
    ) -> ToileOutcome {
        if !ambre_convert::jpeg::is_jpeg(bytes) {
            return self.begin_import(import_raster(bytes, &Limits::default()), at);
        }
        let limits = Limits::default();
        let Some(size) = ambre_convert::jpeg::stored_size(bytes) else {
            return Self::interop_failed(
                Direction::Import,
                InteropError::ParseFailed(ambre_core::FileRefusal::Malformed),
            );
        };
        if let Err(e) = limits.check_image_dims(size) {
            return Self::interop_failed(Direction::Import, e.interop());
        }
        let id = ambre_bus::next_request();
        self.decoding.insert(
            id,
            PendingPicture {
                data: bytes.clone(),
                size,
                at,
            },
        );
        ToileOutcome {
            display: Vec::new(),
            update: None,
            ephemeral: None,
            requests: vec![ToileRequest::DecodePicture {
                id,
                request: ambre_convert::jpeg::JpegDecode {
                    data: bytes.clone(),
                    bound: limits.max_image,
                    size: ambre_convert::images::thumb_size(size),
                    turn: ambre_convert::jpeg::JpegTurn::Stored,
                },
            }],
        }
    }

    /// The answer of the converter thread for a JPEG this board inserts: the
    /// picture lands where the user dropped it. A module may not load, offline
    /// on a first use, or it may refuse the file. The import then fails with
    /// the reason, and the board is not touched (`TOILE-IO-132`). A ticket
    /// this board does not hold answers nothing.
    pub fn handle_decoded(
        &mut self,
        id: Seq<Request>,
        thumb: Result<ambre_convert::images::Pixels, ambre_convert::JobFailure>,
    ) -> ToileOutcome {
        let Some(PendingPicture { data, size, at }) = self.decoding.remove(&id) else {
            return ToileOutcome::display_only(Vec::new());
        };
        let thumb = match thumb {
            Ok(thumb) => thumb,
            Err(failure) => {
                let error = match failure {
                    ambre_convert::JobFailure::Unavailable => {
                        InteropError::Stopped(ambre_core::StopCause::ConverterUnavailable)
                    }
                    ambre_convert::JobFailure::Refused => {
                        InteropError::ParseFailed(ambre_core::FileRefusal::Malformed)
                    }
                };
                return Self::interop_failed(Direction::Import, error);
            }
        };
        let media = MediaRef::Pending(Pos::<ImportSlot>::ZERO);
        let batch = crate::logic::interop::jpeg_blob(data, size, &thumb, &Limits::default(), media)
            .map(crate::logic::interop::raster_batch);
        self.begin_import(batch, at)
    }

    /// Route a converted batch. A parse / limit failure aborts with a failure
    /// and nothing is applied. A media-free batch takes the FAST PATH and lands
    /// now in one undo group. A media-bearing batch is stashed as a
    /// [`PendingImport`] and issues one `UploadMedia` request per blob, applying
    /// nothing until every real hash returns (`toile-media`).
    fn begin_import(
        &mut self,
        converted: Result<ImportBatch, ImportError>,
        at: Point,
    ) -> ToileOutcome {
        let mut batch = match converted {
            Ok(b) => b,
            Err(e) => return Self::interop_failed(Direction::Import, e.interop()),
        };
        // The file lands where the user put it, before anything is applied
        // (`TOILE-IO-023`). The media round trip may hold the batch for a while,
        // and the drop point is a fact of the gesture, not of the answer.
        place_batch_at(&mut batch, at);
        if batch.media.is_empty() {
            return self.finish_import(batch);
        }
        // Async media: one upload per blob, nothing applied yet.
        let mut outstanding = HashMap::default();
        let mut requests = Vec::with_capacity(batch.media.len());
        for (slot, blob) in batch.media.iter().enumerate() {
            let id = ambre_bus::next_request();
            outstanding.insert(id, Pos::<ImportSlot>::from_usize(slot));
            requests.push(ToileRequest::UploadMedia {
                id,
                data: blob.plaintext.clone(),
            });
        }
        self.pending_imports
            .push(PendingImport { batch, outstanding });
        ToileOutcome {
            display: Vec::new(),
            update: None,
            ephemeral: None,
            requests,
        }
    }

    /// Start applying a fully-resolved batch, and apply its first slice now
    /// (`TOILE-IO-028`). A batch of one slice or less lands whole in this
    /// call; a larger one leaves a job the Logic thread resumes between its
    /// messages.
    ///
    /// An import already running is finished first, so two imports never share
    /// an undo group. One Loro undo group spans every slice, which keeps the
    /// whole file one undo step (`TOILE-IO-001`).
    fn finish_import(&mut self, batch: ImportBatch) -> ToileOutcome {
        self.start_import(batch, ImportPace::Slice)
    }

    /// Open the import's undo group and apply its first `pace`.
    fn start_import(&mut self, batch: ImportBatch, pace: ImportPace) -> ToileOutcome {
        let mut display = self.finish_running_import();
        self.doc.commit();
        self.history.seal();
        self.import_job = Some(Box::new(ImportJob::new(batch)));
        display.extend(self.run_import(pace));
        self.emit_with_update(display)
    }

    /// Apply a batch whole, in one call, and hand back its report: the tests'
    /// way past the media round trip and the slicing.
    #[cfg(test)]
    pub(super) fn apply_import(&mut self, batch: ImportBatch) -> (ImportReport, ToileOutcome) {
        let outcome = self.start_import(batch, ImportPace::Whole);
        let report = outcome
            .display
            .iter()
            .find_map(|e| match e {
                ToileEvent::Interop(InteropAnswer::ImportCompleted { report }) => {
                    Some(report.clone())
                }
                _ => None,
            })
            .expect("a whole import closes with its report");
        (report, outcome)
    }

    /// Whether an import is still being applied (`TOILE-IO-028`). The Logic
    /// thread reads it to come back to this board without waiting for a
    /// message.
    #[must_use]
    pub fn has_pending_work(&self) -> bool {
        self.import_job.is_some()
    }

    /// Apply the next slice of the running import: at most [`IMPORT_SLICE`]
    /// units, one commit and one delta (`TOILE-IO-028`). The last slice closes
    /// the import and carries its report. With no import running it answers
    /// nothing.
    pub fn resume(&mut self) -> ToileOutcome {
        if self.import_job.is_none() {
            return ToileOutcome::display_only(Vec::new());
        }
        let display = self.run_import(ImportPace::Slice);
        let outcome = self.emit_with_update(display);
        self.announce_write(outcome)
    }

    /// Apply the rest of a running import at once, without flushing its delta.
    ///
    /// Called before a command that writes the board, and before a second
    /// import. The command's own flush then carries both, in one delta. An
    /// edit therefore never lands inside the import's undo group, and never
    /// between two of its slices.
    pub(super) fn finish_running_import(&mut self) -> Vec<ToileEvent> {
        if self.import_job.is_none() {
            return Vec::new();
        }
        self.run_import(ImportPace::Whole)
    }

    /// Apply the `pace` of the running import, then commit. The caller
    /// flushes the delta. A slice short of the end repaints, and the last one
    /// closes the import.
    fn run_import(&mut self, pace: ImportPace) -> Vec<ToileEvent> {
        let Some(mut job) = self.import_job.take() else {
            return Vec::new();
        };
        let end = match pace {
            ImportPace::Slice => IMPORT_SLICE.end_after(job.done).min(job.total()),
            ImportPace::Whole => job.total(),
        };
        while job.done < end {
            self.apply_unit(&mut job);
            job.done = job.done.incremented();
        }
        self.doc.commit();
        if job.done < job.total() {
            // Each slice short of the end says how far the import got
            // (`INTEROP-008`), counted in the batch's own units.
            let progress = ConversionProgress::new(
                Direction::Import,
                Count::<Steps>::recount(job.done),
                Count::<Steps>::recount(job.total()),
            );
            self.import_job = Some(job);
            return vec![
                self.viewport_event(),
                ToileEvent::Interop(InteropAnswer::Progress(progress)),
            ];
        }
        self.close_import(*job)
    }

    /// The import's last step: the media map, the commit that ends the undo
    /// group, and one rebuild of every derived cache.
    fn close_import(&mut self, job: ImportJob) -> Vec<ToileEvent> {
        let ImportJob {
            media,
            mut report,
            slot_ids,
            frames,
            ..
        } = job;
        self.seed_media(&media);
        self.doc.commit();
        self.history.close();
        self.rebuild_caches();
        report.record_count(
            ToileConstruct::Element,
            Outcome::Carried,
            Count::<Occurrences>::of(slot_ids.iter().filter(|s| s.is_some()).count()),
        );
        let mut events = vec![
            self.viewport_event(),
            ToileEvent::Interop(InteropAnswer::ImportCompleted { report }),
        ];
        // The deck panel holds no order of its own, so a deck the import grew
        // is read again (`TOILE-UI-343`).
        if !frames.is_empty() {
            events.push(ToileEvent::DeckChanged);
        }
        events
    }

    /// Handle a service-plane response routed back to this document
    /// (`toile-media`): a `MediaUploaded` fills one outstanding upload of a
    /// pending import with its real hash. When the last blob resolves, the batch
    /// lands atomically. An id matching no pending import is a stale response
    /// (the import was undone / the doc reopened) and is dropped.
    ///
    /// # Panics
    ///
    /// When the outstanding upload it just matched is absent from the pending
    /// import. The position lookup above establishes it.
    pub fn handle_service_event(&mut self, id: Seq<Request>, evt: RestEvent) -> ToileOutcome {
        let RestEvent::MediaUploaded { content_hash, .. } = evt else {
            // Only upload confirmations concern an import; ignore anything else.
            return ToileOutcome::display_only(Vec::new());
        };
        let Some(pos) = self
            .pending_imports
            .iter()
            .position(|p| p.outstanding.contains_key(&id))
        else {
            return ToileOutcome::display_only(Vec::new());
        };
        let Some(pending) = self.pending_imports.get_mut(pos) else {
            return ToileOutcome::display_only(Vec::new());
        };
        if let Some(slot) = pending.outstanding.remove(&id) {
            rewrite_hash(&mut pending.batch, slot, &content_hash);
        }
        let ready = pending.outstanding.is_empty();
        if !ready {
            // More uploads still in flight; apply nothing yet.
            return ToileOutcome::display_only(Vec::new());
        }
        let pending = self.pending_imports.remove(pos);
        self.finish_import(pending.batch)
    }

    /// Handle a service-plane failure routed back to this document
    /// (`toile-media`): a failed media upload aborts the whole pending import
    /// (atomicity), leaving the board untouched. An id matching no pending
    /// import is dropped.
    pub fn handle_service_error(
        &mut self,
        id: Seq<Request>,
        failure: ambre_core::ApiFailure,
    ) -> ToileOutcome {
        let Some(pos) = self
            .pending_imports
            .iter()
            .position(|p| p.outstanding.contains_key(&id))
        else {
            return ToileOutcome::display_only(Vec::new());
        };
        self.pending_imports.remove(pos);
        Self::interop_failed(
            Direction::Import,
            InteropError::Stopped(ambre_core::StopCause::UploadFailed(failure)),
        )
    }

    /// Number of imports awaiting media uploads. Test / integration accessor.
    pub fn pending_import_count(&self) -> Count<PendingImports> {
        Count::<PendingImports>::of(self.pending_imports.len())
    }

    /// Apply one unit of the running import. The units run in a strict order,
    /// so every slot or group reference resolves before it is used:
    ///
    /// 1. **creates** -> `create_element_inner`, minting an [`Id<Element>`] per slot;
    /// 2. **styles** -> `restyle_inner` on the slot's fresh element (style is an
    ///    independent LWW register, so it cannot ride the create);
    /// 3. **groups** -> `group_inner`, innermost-first, so a `Group(i)` member
    ///    always resolves to an already-built group;
    /// 4. **connectors** -> `create_connector_inner`, endpoints resolved by ref.
    ///
    /// The media map and the cache rebuild follow in
    /// [`close_import`](Self::close_import). A create that unexpectedly fails is
    /// recorded in the report and its slot left unresolved, so any group or
    /// connector referencing it is skipped. A trusted converter should never
    /// emit one.
    fn apply_unit(&mut self, job: &mut ImportJob) {
        match job.step() {
            ImportStep::Create(slot) => self.apply_import_create(job, slot),
            ImportStep::Style(slot) => {
                let unit = slot.to_usize();
                if let (Some(Some(id)), Some(Some(style))) =
                    (job.slot_ids.get(unit), job.styles.get(unit))
                {
                    self.restyle_inner(std::slice::from_ref(id), style);
                }
            }
            ImportStep::Group(rank) => self.apply_import_group(job, rank),
            ImportStep::Frame(rank) => self.apply_import_frame(job, rank),
            ImportStep::Connector(conn) => self.apply_import_connector(job, &conn),
            // `done` stops at the last frame, so a unit past them is no unit.
            ImportStep::Past => {}
        }
    }

    /// Apply the create at `slot`, the next one the job's iterator holds.
    fn apply_import_create(&mut self, job: &mut ImportJob, slot: Pos<ImportElement>) {
        let unit = slot.to_usize();
        let created = match job.commands.next() {
            Some(ToileCommand::CreateElement {
                kind,
                at,
                size,
                content,
                fill,
                ..
            }) => self
                .create_element_inner(kind, at, size, content, fill)
                .ok(),
            // The batch contract is creates-only; anything else is a
            // converter bug. Drop it rather than corrupt slot indices.
            _ => None,
        };
        // A rotation cannot ride the create, as a style cannot. It joins the
        // same unit, so the box the next group reads is already turned.
        if let (Some(id), Some(radians)) = (&created, job.rotations.get(unit))
            && *radians != ambre_core::Radians::ZERO
            && let Some(em) = self.element_map(id)
        {
            let _ = em.insert(super::names::ElementKey::Rot, radians.get());
            self.update_bounds(id, &em);
        }
        if created.is_none() {
            tracing::warn!("[Toile] the import could not create its element at slot {unit}");
            job.report.record(ToileConstruct::Element, Outcome::Dropped);
        }
        if let (Some(id), Some(turn)) = (&created, job.turns.get(unit))
            && *turn != ambre_core::Radians::ZERO
        {
            self.turn_inner(id, *turn);
        }
        job.slot_ids.push(created);
    }

    /// Build the group at `rank`, innermost first, so a `Group(i)` member
    /// always resolves to a group already built.
    fn apply_import_group(&mut self, job: &mut ImportJob, rank: Pos<ImportGroupRank>) {
        let unit = rank.to_usize();
        if let Some(group) = job.groups.get(unit) {
            let members: Vec<Id<Element>> = group
                .members
                .iter()
                .filter_map(|r| self.resolve_ref(*r, &job.slot_ids, &job.group_ids))
                .collect();
            // Fewer than two resolved members: the element skips already
            // say why, so the group drops without a second line.
            let built = if members.len() < 2 {
                None
            } else {
                match self.group_inner(&members) {
                    Ok(id) => Some(id),
                    Err(e) => {
                        tracing::warn!("[Toile] the import could not build group {unit}: {e:?}");
                        job.report.record(ToileConstruct::Group, Outcome::Dropped);
                        None
                    }
                }
            };
            if built.is_some() {
                // Seed the fresh group's derived bounds now, so a later
                // `Group(i)` member passes `group_inner`'s existence check
                // (bounds-keyed) before the final `rebuild_caches`.
                self.recompute_groups();
            }
            job.group_ids.push(built);
        }
    }

    /// Title the frame at `rank` and fill it with its members, resolved
    /// through the slots and groups built (`TOILE-IO-140`). A frame whose
    /// create was refused has no members to take, and the create's own skip
    /// already says why. A frame of a deck joins the end of the deck with its
    /// notes (`toile-pptx-import`).
    fn apply_import_frame(&mut self, job: &mut ImportJob, rank: Pos<ImportFrameRank>) {
        let Some(entry) = job.frames.get(rank.to_usize()) else {
            return;
        };
        let Some(frame) =
            self.resolve_ref(ImportRef::Slot(entry.frame), &job.slot_ids, &job.group_ids)
        else {
            return;
        };
        let members: Vec<Id<Element>> = entry
            .members
            .iter()
            .filter_map(|r| self.resolve_ref(*r, &job.slot_ids, &job.group_ids))
            .collect();
        if let Some(notes) = &entry.notes {
            self.fill_frame_inner(&frame, &members, &entry.name);
            if let Some(slide) = self.frame(&frame) {
                self.append_slides_inner(&[(slide, notes.as_str())]);
            }
            return;
        }
        if let Some(fm) = self.element_map(&frame) {
            let name: String = entry
                .name
                .trim()
                .chars()
                .take(crate::message::FRAME_NAME_MAX.to_usize())
                .collect();
            if !name.is_empty() {
                let _ = fm.insert(super::names::ElementKey::Name, name.as_str());
            }
        }
        if !members.is_empty() {
            self.frame_children_inner(&frame, &members, &[]);
            // A member taken into the frame leaves no box behind in a group.
            self.recompute_groups();
        }
    }

    /// Draw `conn`, its two ends resolved through the slots and groups built.
    fn apply_import_connector(&mut self, job: &mut ImportJob, conn: &ImportConnector) {
        let (Some(from), Some(to)) = (
            self.resolve_ref(conn.from, &job.slot_ids, &job.group_ids),
            self.resolve_ref(conn.to, &job.slot_ids, &job.group_ids),
        ) else {
            job.report
                .record(ToileConstruct::Connector, Outcome::Dropped);
            return;
        };
        // A refused connector is left out and reported, as a refused create.
        if let Err(e) = self.create_connector_inner(
            from,
            conn.from_anch,
            to,
            conn.to_anch,
            conn.path_type,
            conn.arrows,
        ) {
            tracing::warn!("[Toile] the import could not draw a connector: {e:?}");
            job.report
                .record(ToileConstruct::Connector, Outcome::Dropped);
        }
    }

    /// Resolve an [`ImportRef`] to a live [`Id<Element>`], or `None` if its slot or
    /// group was not created, or a peer deleted it between two slices.
    fn resolve_ref(
        &self,
        r: ImportRef,
        slot_ids: &[Option<Id<Element>>],
        group_ids: &[Option<Id<Element>>],
    ) -> Option<Id<Element>> {
        let id = match r {
            ImportRef::Slot(i) => slot_ids.get(i.to_usize()).cloned().flatten(),
            ImportRef::Group(i) => group_ids.get(i.to_usize()).cloned().flatten(),
        }?;
        self.bounds.contains_key(&id).then_some(id)
    }
    /// Populate the `media` root map from a batch's plaintext blobs
    /// (`toile-elements`), **without** committing. Each blob's `hash` (the real
    /// blake2b, resolved Sync-side by the time this runs) keys a `{w, h, mime,
    /// thumb}` map. Dedup is content-addressed: a hash already present is
    /// left untouched, so N elements sharing one image seed exactly one entry.
    /// Returns the count of NEW entries created (existing keys are skipped).
    ///
    /// Commit-free by design: the caller (`apply_import`) folds this into the one
    /// import undo group alongside the element creates.
    pub(super) fn seed_media(&mut self, blobs: &[MediaBlob]) -> Count<MediaEntries> {
        let media = self.doc.get_map(Root::Media.as_str());
        let mut created = Count::<MediaEntries>::ZERO;
        for blob in blobs {
            let Some(key) = blob.media.stored().map(Validated::as_str) else {
                // No address from Sync yet: a pending slot never becomes a key.
                continue;
            };
            if media.get(key).is_some() {
                // The entry already exists (dedup).
                continue;
            }
            let Ok(entry) = media
                .insert_container(key, LoroMap::new())
                .map(Fields::<MediaKey>::new)
            else {
                continue;
            };
            let _ = entry.insert(MediaKey::W, i64::from(blob.size.width.get()));
            let _ = entry.insert(MediaKey::H, i64::from(blob.size.height.get()));
            let _ = entry.insert(MediaKey::Mime, blob.mime.as_str());
            let _ = entry.insert(
                MediaKey::Thumb,
                LoroValue::Binary(blob.thumb.as_bytes().to_vec().into()),
            );
            created = created.incremented();
        }
        created
    }

    /// Number of entries in the `media` map. Test / integration accessor.
    pub fn media_count(&self) -> Count<MediaEntries> {
        match self.doc.get_map(Root::Media.as_str()).get_deep_value() {
            LoroValue::Map(m) => Count::<MediaEntries>::of(m.len()),
            _ => Count::<MediaEntries>::ZERO,
        }
    }

    /// The `(w, h, mime)` load-info of a media entry, if present. Test accessor.
    #[cfg(test)]
    pub(super) fn media_meta(&self, hash: &str) -> Option<(i64, i64, String)> {
        let LoroValue::Map(m) = self.doc.get_map(Root::Media.as_str()).get_deep_value() else {
            return None;
        };
        let LoroValue::Map(entry) = m.get(hash)? else {
            return None;
        };
        let entry = super::names::FieldValues::<MediaKey>::new(entry);
        let w = match entry.get(MediaKey::W)? {
            LoroValue::I64(v) => *v,
            _ => return None,
        };
        let h = match entry.get(MediaKey::H)? {
            LoroValue::I64(v) => *v,
            _ => return None,
        };
        let mime = match entry.get(MediaKey::Mime)? {
            LoroValue::String(s) => s.to_string(),
            _ => return None,
        };
        Some((w, h, mime))
    }
}

#[cfg(test)]
mod tests {
    use ambre_core::index::Pos;
    use ambre_core::{Count, Elements, Rgba, Validated};
    use kurbo::{Point, Size};

    use crate::logic::command::{ImportSlot, InitContent, MediaRef, ToileCommand};
    use crate::logic::interop::{
        ContentHash, ImportBatch, ImportElement, ImportGroup, ImportGroupRank, ImportRef, MediaBlob,
    };
    use crate::logic::model::{ElementKind, ElementStyle, ShapeKind};
    use crate::logic::state::ToileState;

    use super::{MediaEntries, PendingImports};
    use crate::message::{ToileConstruct, ToileImport, ToileImportFormat};
    use ambre_bus::interop::{
        ConversionProgress, InteropAnswer, InteropError, InteropFailure, InteropRequest, Outcome,
    };

    /// An import of `data` read as `format`, landing at `at`.
    fn import_cmd(
        format: ToileImportFormat,
        data: ambre_core::Payload<ambre_client_core::FileBytes>,
        at: Point,
    ) -> ToileCommand {
        ToileCommand::Interop(InteropRequest::Import(ToileImport { format, data, at }))
    }

    /// The progress a slice reported, if it reported one.
    fn progress_of(out: &crate::logic::command::ToileOutcome) -> Option<ConversionProgress> {
        out.display.iter().find_map(|e| match e {
            ToileEvent::Interop(InteropAnswer::Progress(progress)) => Some(*progress),
            _ => None,
        })
    }

    /// A `CreateElement` for a plain rect at `x`, used to fill import slots.
    fn rect_cmd(x: f64) -> ToileCommand {
        ToileCommand::CreateElement {
            kind: ElementKind::Shape(ShapeKind::Rect),
            at: Point::new(x, 0.0),
            size: Size::new(10.0, 10.0),
            content: InitContent::None,
            fill: None,
            correlate: None,
        }
    }

    #[test]
    fn import_is_one_undo_group() {
        let mut st = ToileState::new();
        let batch = ImportBatch {
            commands: (0..5).map(|i| rect_cmd(i as f64 * 20.0)).collect(),
            styles: vec![None; 5],
            ..ImportBatch::new(ambre_bus::interop::FileFormat::Svg)
        };
        let (report, outcome) = st.apply_import(batch);
        assert_eq!(
            report
                .count(ToileConstruct::Element, Outcome::Carried)
                .get(),
            5
        );
        assert_eq!(st.element_count(), Count::<Elements>::new(5));
        assert!(outcome.update.is_some(), "one CRDT delta is broadcast");
        assert!(
            outcome.requests.is_empty(),
            "no media -> no service request"
        );
        // The single strongest invariant: one Undo empties the whole import.
        st.handle_command(ToileCommand::Undo);
        assert_eq!(st.element_count(), Count::<Elements>::ZERO);
    }

    #[test]
    fn import_applies_styles_per_slot() {
        let mut st = ToileState::new();
        let style = ElementStyle {
            fill: Some(Rgba::from_bits(0x3366_CCFF)),
            ..ElementStyle::default()
        };
        let batch = ImportBatch {
            commands: vec![rect_cmd(0.0)],
            styles: vec![Some(style.clone())],
            ..ImportBatch::new(ambre_bus::interop::FileFormat::Svg)
        };
        let (report, _) = st.apply_import(batch);
        assert_eq!(
            report
                .count(ToileConstruct::Element, Outcome::Carried)
                .get(),
            1
        );
        // The one element carries the imported fill, not the default.
        let vp = st.derive_viewport();
        let fills: Vec<Option<Rgba>> = vp
            .nodes
            .iter()
            .filter_map(|n| match n {
                crate::logic::command::DisplayNode::Element(e) => Some(e.style.fill),
                _ => None,
            })
            .collect();
        assert_eq!(fills, vec![Some(Rgba::from_bits(0x3366_CCFF))]);
    }

    #[test]
    fn import_rebuilds_nested_groups() {
        let mut st = ToileState::new();
        // Slots 0,1,2 are rects; group 0 = {0,1}; group 1 = {2, group 0}.
        let batch = ImportBatch {
            commands: vec![rect_cmd(0.0), rect_cmd(20.0), rect_cmd(40.0)],
            styles: vec![None; 3],
            groups: vec![
                ImportGroup {
                    members: vec![
                        ImportRef::Slot(Pos::<ImportElement>::ZERO),
                        ImportRef::Slot(Pos::<ImportElement>::new(1)),
                    ],
                },
                ImportGroup {
                    members: vec![
                        ImportRef::Slot(Pos::<ImportElement>::new(2)),
                        ImportRef::Group(Pos::<ImportGroupRank>::ZERO),
                    ],
                },
            ],
            ..ImportBatch::new(ambre_bus::interop::FileFormat::Svg)
        };
        let (report, _) = st.apply_import(batch);
        assert_eq!(
            report
                .count(ToileConstruct::Element, Outcome::Carried)
                .get(),
            3
        );
        // Three rects plus two group elements.
        assert_eq!(st.element_count(), Count::<Elements>::new(5));
        // Still one undo group: a single Undo removes every element AND group.
        st.handle_command(ToileCommand::Undo);
        assert_eq!(st.element_count(), Count::<Elements>::ZERO);
    }

    #[test]
    fn import_seeds_the_media_map() {
        let mut st = ToileState::new();
        let batch = ImportBatch {
            commands: vec![rect_cmd(0.0)],
            styles: vec![None],
            media: vec![blob("img-hash", 32, 32, "image/png")],
            ..ImportBatch::new(ambre_bus::interop::FileFormat::Svg)
        };
        st.apply_import(batch);
        assert_eq!(st.media_count(), Count::<MediaEntries>::new(1));
        assert_eq!(
            st.media_meta(&hx("img-hash")),
            Some((32, 32, "image/png".to_owned()))
        );
    }

    /// The address a test names `label`: the hex of its bytes, padded with
    /// zeros to sixty-four characters. Two labels give two addresses.
    fn hx(label: &str) -> String {
        let hex: String = label.bytes().map(|b| format!("{b:02x}")).collect();
        format!("{hex:0<64}")
    }

    /// The address `label` names, as Sync hands it back.
    fn hash_of(label: &str) -> Validated<ContentHash> {
        Validated::<ContentHash>::parse(hx(label)).expect("a label of at most 32 bytes")
    }

    fn blob(hash: &str, w: u32, h: u32, mime: &str) -> MediaBlob {
        MediaBlob {
            media: MediaRef::Stored(hash_of(hash)),
            plaintext: ambre_core::Payload::<ambre_core::MediaBytes>::new(vec![0u8; 4]),
            mime: ambre_core::MediaType::from(mime),
            size: ambre_core::Size::new(ambre_core::Px::new(w), ambre_core::Px::new(h)),
            thumb: ambre_core::Payload::<ambre_core::ThumbnailPng>::new(vec![1, 2, 3]),
        }
    }

    #[test]
    fn seed_media_writes_load_info() {
        let mut st = ToileState::new();
        let n = st.seed_media(&[blob("abc", 640, 480, "image/png")]);
        assert_eq!(n, Count::<MediaEntries>::new(1));
        assert_eq!(st.media_count(), Count::<MediaEntries>::new(1));
        assert_eq!(
            st.media_meta(&hx("abc")),
            Some((640, 480, "image/png".to_owned()))
        );
    }

    #[test]
    fn seed_media_dedups_repeated_hashes() {
        let mut st = ToileState::new();
        // Two blobs with the same content hash collapse to one entry (dedup).
        let n = st.seed_media(&[
            blob("dup", 10, 10, "image/png"),
            blob("dup", 10, 10, "image/png"),
        ]);
        assert_eq!(
            n,
            Count::<MediaEntries>::new(1),
            "identical hashes seed exactly one entry"
        );
        assert_eq!(st.media_count(), Count::<MediaEntries>::new(1));
        // A later seed of the same hash is a no-op (already present).
        assert_eq!(
            st.seed_media(&[blob("dup", 10, 10, "image/png")]),
            Count::<MediaEntries>::ZERO
        );
        assert_eq!(st.media_count(), Count::<MediaEntries>::new(1));
    }

    #[test]
    fn seed_media_skips_unresolved_hash() {
        let mut st = ToileState::new();
        // An empty text spells no address, so no blob can carry one as a key.
        // A blob Sync has not answered yet carries its pending slot instead.
        assert_eq!(Validated::<ContentHash>::parse("").ok(), None);
        let pending = MediaBlob {
            media: MediaRef::Pending(Pos::<ImportSlot>::ZERO),
            ..blob("unused", 10, 10, "image/png")
        };
        let n = st.seed_media(&[pending]);
        assert_eq!(n, Count::<MediaEntries>::ZERO);
        assert_eq!(st.media_count(), Count::<MediaEntries>::ZERO);
    }

    /// `TOILE-IO-059`: the slot an import names a blob by until Sync answers
    /// is refused as a key, the very slot the decoder mints included.
    #[test]
    fn a_placeholder_token_never_becomes_a_key() {
        let mut st = ToileState::new();
        let decoded = crate::logic::interop::import_raster(
            &ambre_core::Payload::<ambre_client_core::FileBytes>::new(solid_png(4, 4)),
            &crate::logic::interop::Limits::default(),
        )
        .expect("a picture");
        let minted = decoded.media[0].clone();
        assert_eq!(minted.media.stored(), None, "{:?}", minted.media);
        let pending = MediaBlob {
            media: MediaRef::Pending(Pos::<ImportSlot>::ZERO.next()),
            ..blob("unused", 1, 1, "image/png")
        };
        assert_eq!(
            st.seed_media(&[minted, pending]),
            Count::<MediaEntries>::ZERO
        );
        assert_eq!(st.media_count(), Count::<MediaEntries>::ZERO);
        assert_eq!(
            st.seed_media(&[blob("realhash", 1, 1, "image/png")]),
            Count::<MediaEntries>::new(1)
        );
    }

    // -- command path (fast / async / failure) --------------------------------

    use ambre_bus::RestEvent;
    use ambre_core::id::{Document, Id};
    use std::io::Cursor;

    use crate::logic::command::{Request, ToileEvent, ToileOutcome, ToileRequest};
    use ambre_core::Seq;

    /// A minimal solid-color PNG produced with the `image` crate (no fixture).
    fn solid_png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    fn first_upload_id(out: &ToileOutcome) -> Seq<Request> {
        match out.requests.first().expect("an upload request") {
            ToileRequest::UploadMedia { id, .. } => *id,
            _ => panic!("expected UploadMedia"),
        }
    }

    fn has_report(out: &ToileOutcome) -> bool {
        out.display.iter().any(|e| {
            matches!(
                e,
                ToileEvent::Interop(InteropAnswer::ImportCompleted { .. })
            )
        })
    }

    #[test]
    fn svg_import_takes_the_fast_path() {
        let mut st = ToileState::new();
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect x="0" y="0" width="5" height="5" fill="#ff0000"/></svg>"##;
        let out = st.handle_converted(import_cmd(
            ToileImportFormat::Svg,
            ambre_core::Payload::<ambre_client_core::FileBytes>::new(svg.to_vec()),
            Point::ZERO,
        ));
        // No media -> applied synchronously, one CRDT delta, no service request.
        assert!(out.update.is_some());
        assert!(out.requests.is_empty());
        assert!(has_report(&out), "an ImportReport is surfaced");
        assert_eq!(st.element_count(), Count::<Elements>::new(1));
        assert_eq!(st.pending_import_count(), Count::<PendingImports>::ZERO);
    }

    /// A file lands where the user dropped it, not at the coordinates it carried
    /// (`TOILE-IO-023`).
    #[test]
    fn an_import_lands_on_the_drop_point() {
        let mut st = ToileState::new();
        // The rect sits at (40, 40) inside the drawing; dropped at (300, 200) it
        // must be the batch's top-left corner that moves there.
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><rect x="40" y="40" width="5" height="5"/></svg>"##;
        st.handle_converted(import_cmd(
            ToileImportFormat::Svg,
            ambre_core::Payload::<ambre_client_core::FileBytes>::new(svg.to_vec()),
            Point::new(300.0, 200.0),
        ));
        let id = st
            .layer_order()
            .iter()
            .find_map(|n| match n {
                crate::logic::model::NodeId::Element(e) => Some(e.clone()),
                _ => None,
            })
            .expect("the import created one element");
        let r = st.bounds_of(&id).expect("the element has bounds");
        assert_eq!((r.x0, r.y0), (300.0, 200.0));
    }

    #[test]
    fn a_broken_file_fails_without_touching_the_board() {
        let mut st = ToileState::new();
        let out = st.handle_converted(import_cmd(
            ToileImportFormat::Svg,
            ambre_core::Payload::<ambre_client_core::FileBytes>::new(b"this is not svg".to_vec()),
            Point::ZERO,
        ));
        assert!(
            out.display
                .iter()
                .any(|e| matches!(e, ToileEvent::Interop(InteropAnswer::Failed(_))))
        );
        assert!(out.update.is_none());
        assert_eq!(st.element_count(), Count::<Elements>::ZERO);
    }

    /// Covers `TOILE-IO-071`: an import onto a populated board leaves each
    /// element it found there as it was.
    ///
    /// The file carries a group, so the apply pass regroups and restyles. The
    /// stored map of the earlier element is compared whole, before and after.
    #[test]
    fn an_import_leaves_the_board_it_lands_on_alone() {
        let mut st = ToileState::new();
        let held = st
            .handle_command(ToileCommand::CreateElement {
                kind: crate::logic::model::ElementKind::Shape(crate::logic::model::ShapeKind::Rect),
                at: Point::new(5.0, 5.0),
                size: kurbo::Size::new(40.0, 40.0),
                content: crate::logic::command::InitContent::None,
                fill: None,
                correlate: None,
            })
            .display
            .into_iter()
            .find_map(|e| match e {
                ToileEvent::ElementCreated { id, .. } => Some(id),
                _ => None,
            })
            .expect("a minted id");
        let before = st.element_map(&held).expect("held").map().get_deep_value();

        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><g><rect x="0" y="0" width="5" height="5" fill="#ff0000"/><rect x="10" y="0" width="5" height="5"/></g></svg>"##;
        st.handle_converted(import_cmd(
            ToileImportFormat::Svg,
            ambre_core::Payload::<ambre_client_core::FileBytes>::new(svg.to_vec()),
            Point::ZERO,
        ));
        assert!(
            st.element_count() > Count::<Elements>::new(1),
            "the file landed"
        );
        assert_eq!(
            st.element_map(&held).expect("held").map().get_deep_value(),
            before
        );
    }

    /// Covers `TOILE-IO-094`: JSON that is neither a Figma nor a Miro export
    /// is refused by name, and the board is left as it was.
    #[test]
    fn json_of_neither_tool_is_refused_by_name() {
        let mut st = ToileState::new();
        let out = st.handle_converted(import_cmd(
            ToileImportFormat::Board,
            ambre_core::Payload::<ambre_client_core::FileBytes>::new(b"{\"name\":\"x\"}".to_vec()),
            Point::ZERO,
        ));
        assert!(out.update.is_none(), "the refusal wrote to the board");
        assert!(
            out.display.iter().any(|e| matches!(
                e,
                ToileEvent::Interop(InteropAnswer::Failed(InteropFailure {
                    error: InteropError::ParseFailed(ambre_core::FileRefusal::OtherDocument),
                    ..
                }))
            )),
            "{:?}",
            out.display
        );
        assert_eq!(st.element_count(), Count::<Elements>::ZERO);
    }

    /// An import of the board fixture `json` through the images engine the
    /// host links, and its report.
    fn import_board_fixture(st: &mut ToileState, json: &str) -> crate::message::ToileReport {
        let out = st.handle_converted(import_cmd(
            ToileImportFormat::Board,
            ambre_core::Payload::<ambre_client_core::FileBytes>::new(json.as_bytes().to_vec()),
            Point::ZERO,
        ));
        while st.has_pending_work() {
            st.resume();
        }
        out.display
            .iter()
            .find_map(|e| match e {
                ToileEvent::Interop(InteropAnswer::ImportCompleted { report }) => {
                    Some(report.clone())
                }
                _ => None,
            })
            .expect("the board lands in one call")
    }

    const FIGMA: &str = include_str!("../interop/fixtures/figma_board.json");
    const MIRO: &str = include_str!("../interop/fixtures/miro_board.json");

    /// Covers `TOILE-IO-140`, `TOILE-IO-015` and `TOILE-IO-143`. A Figma
    /// frame lands titled and holding its children. The connector between
    /// two mapped nodes lands, and one undo takes the whole file back.
    #[test]
    fn a_figma_board_lands_with_its_frames_and_connectors_in_one_undo() {
        let mut st = ToileState::new();
        let report = import_board_fixture(&mut st, FIGMA);
        assert_eq!(
            report
                .count(ToileConstruct::Element, Outcome::Carried)
                .get(),
            12
        );
        let frames: Vec<_> = st
            .frame_ids()
            .into_iter()
            .filter(|f| st.frame_name(f).as_deref() == Some("Sprint plan"))
            .collect();
        let [frame] = frames.as_slice() else {
            panic!("one frame is titled Sprint plan: {frames:?}");
        };
        assert_eq!(st.children_of(frame).len(), 5, "the five nodes it drew");
        assert_eq!(st.connector_ids().len(), 1);
        st.handle_command(ToileCommand::Undo);
        assert_eq!(st.element_count(), Count::<Elements>::ZERO);
        assert!(st.connector_ids().is_empty());
    }

    /// Covers `TOILE-IO-144`: a Miro item keeps its rotation, and a child
    /// of a frame lands at its parent's corner plus its own offset.
    #[test]
    fn a_miro_board_keeps_its_rotation_and_its_frame() {
        let mut st = ToileState::new();
        import_board_fixture(&mut st, MIRO);
        let frames = st.frame_ids();
        let [frame] = frames.as_slice() else {
            panic!("one frame: {frames:?}");
        };
        assert_eq!(st.frame_name(frame).as_deref(), Some("Retro"));
        assert_eq!(
            st.children_of(frame).len(),
            2,
            "the sticky and the labelled shape"
        );
        let ids: Vec<_> = st.bounds.keys().cloned().collect();
        let turned = ids
            .into_iter()
            .filter_map(|id| st.element_map(&id))
            .map(|em| super::super::codec::rotation_of(&em).get())
            .filter(|r| (r - std::f64::consts::FRAC_PI_4).abs() < 1e-9)
            .count();
        assert_eq!(turned, 2, "the shape and its label turn by 45 degrees");
        assert_eq!(st.connector_ids().len(), 1);
    }

    /// A 6 by 4 WebP picture, written by the lossless encoder of
    /// `image-webp` at test time: the fixture is generated, never downloaded.
    #[cfg(not(target_arch = "wasm32"))]
    fn small_webp() -> Vec<u8> {
        let samples: Vec<u8> = [200, 30, 60].repeat(6 * 4);
        let mut bytes = Vec::new();
        image_webp::WebPEncoder::new(&mut bytes)
            .encode(&samples, 6, 4, image_webp::ColorType::Rgb8)
            .unwrap();
        bytes
    }

    /// A WebP drop waits on the images module, which decodes it. The board
    /// uploads the PNG of those pixels, at the picture's own size. A PNG drop
    /// waits on nothing.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_webp_picture_lands_as_the_png_the_images_module_writes() {
        let webp = || {
            import_cmd(
                ToileImportFormat::Image(ambre_core::MediaType::from("image/webp")),
                ambre_core::Payload::<ambre_client_core::FileBytes>::new(small_webp()),
                Point::ZERO,
            )
        };
        assert_eq!(
            ToileState::conversion_family(&webp()),
            Some(ambre_core::ConverterFamily::Images)
        );
        let png = import_cmd(
            ToileImportFormat::Image(ambre_core::MediaType::from("image/png")),
            ambre_core::Payload::<ambre_client_core::FileBytes>::new(solid_png(2, 2)),
            Point::ZERO,
        );
        assert_eq!(ToileState::conversion_family(&png), None);
        let mut st = ToileState::new();
        let out = st.handle_converted(webp());
        let Some(ToileRequest::UploadMedia { data, .. }) = out.requests.first() else {
            panic!("a picture uploads its blob: {:?}", out.display);
        };
        let back = image::load_from_memory_with_format(data.as_bytes(), image::ImageFormat::Png)
            .expect("the blob is a PNG file");
        assert_eq!((back.width(), back.height()), (6, 4));
    }

    /// A JPEG of `width` by `height`, written by the encoder of the `image`
    /// crate at test time.
    #[cfg(not(target_arch = "wasm32"))]
    fn small_jpeg(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut bytes)
            .encode_image(&image::RgbImage::from_pixel(
                width,
                height,
                image::Rgb([9, 99, 199]),
            ))
            .expect("a JPEG encodes");
        bytes
    }

    /// `TOILE-IO-131`: a JPEG dropped on the board asks the converter thread
    /// for its thumbnail and applies nothing. The answer lands the picture
    /// where it was dropped: its file as it came, at its native size, and
    /// the thumbnail as PNG.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_jpeg_waits_for_its_thumbnail_then_lands_where_it_was_dropped() {
        let mut st = ToileState::new();
        let jpeg = small_jpeg(600, 300);
        let cmd = import_cmd(
            ToileImportFormat::Image(ambre_core::MediaType::from("image/jpeg")),
            ambre_core::Payload::<ambre_client_core::FileBytes>::new(jpeg.clone()),
            Point::new(40.0, 50.0),
        );
        assert_eq!(ToileState::conversion_family(&cmd), None);
        let out = st.handle_command(cmd);
        let [ToileRequest::DecodePicture { id, request }] = &out.requests[..] else {
            panic!("one decode leaves: {:?}", out.display);
        };
        assert_eq!(st.element_count(), Count::<Elements>::ZERO);
        assert_eq!(
            request.size,
            ambre_core::Size::new(ambre_core::Px::new(256), ambre_core::Px::new(128))
        );
        let converter = ambre_convert::Converter::load(
            ambre_core::ConverterFamily::Images,
            &ambre_core::Payload::new(Vec::new()),
        )
        .expect("the host links the images engine");
        let thumb = match converter.convert(&ambre_convert::Conversion::DecodeJpeg(request.clone()))
        {
            Ok(ambre_convert::Converted::Pixels(pixels)) => pixels,
            other => panic!("the module decodes the thumbnail: {other:?}"),
        };
        let out = st.handle_decoded(*id, Ok(thumb));
        let Some(ToileRequest::UploadMedia { data, .. }) = out.requests.first() else {
            panic!("the picture uploads its blob: {:?}", out.display);
        };
        assert_eq!(
            data.as_bytes(),
            jpeg.as_slice(),
            "the file is stored as it came"
        );
        let pending = st
            .pending_imports
            .first()
            .expect("the import waits on its upload");
        let blob = pending.batch.media.first().expect("one blob");
        assert_eq!(blob.mime, ambre_core::MediaType::from("image/jpeg"));
        assert_eq!(
            blob.size,
            ambre_core::Size::new(ambre_core::Px::new(600), ambre_core::Px::new(300))
        );
        let back =
            image::load_from_memory_with_format(blob.thumb.as_bytes(), image::ImageFormat::Png)
                .expect("the thumbnail is a PNG file");
        assert_eq!((back.width(), back.height()), (256, 128));
        // A ticket the board does not hold answers nothing.
        assert!(
            st.handle_decoded(*id, Err(ambre_convert::JobFailure::Refused))
                .display
                .is_empty()
        );
    }

    /// `TOILE-IO-132`: offline on a first use, the images module does not
    /// load. The insert fails with the reason, and the board is untouched.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_jpeg_whose_module_did_not_load_fails_and_changes_nothing() {
        let mut st = ToileState::new();
        let out = st.handle_command(import_cmd(
            ToileImportFormat::Image(ambre_core::MediaType::from("image/jpeg")),
            ambre_core::Payload::<ambre_client_core::FileBytes>::new(small_jpeg(8, 8)),
            Point::ZERO,
        ));
        let [ToileRequest::DecodePicture { id, .. }] = &out.requests[..] else {
            panic!("one decode leaves");
        };
        let before = st.export_snapshot();
        let out = st.handle_decoded(*id, Err(ambre_convert::JobFailure::Unavailable));
        assert!(out.requests.is_empty());
        assert!(out.update.is_none());
        assert!(
            out.display
                .iter()
                .any(|event| matches!(event, ToileEvent::Interop(InteropAnswer::Failed(_)))),
            "the user is told: {:?}",
            out.display
        );
        assert_eq!(st.element_count(), Count::<Elements>::ZERO);
        assert_eq!(st.export_snapshot(), before);
    }

    /// A WebP picture whose module did not answer fails by name, and the
    /// board stays as it was. The read leaves for the converter thread
    /// first, and nothing lands before its answer (`ARCH-129`).
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_webp_picture_without_its_module_fails_and_changes_nothing() {
        let mut st = ToileState::new();
        let out = st.handle_conversion(import_cmd(
            ToileImportFormat::Image(ambre_core::MediaType::from("image/webp")),
            ambre_core::Payload::<ambre_client_core::FileBytes>::new(small_webp()),
            Point::ZERO,
        ));
        assert!(out.requests.is_empty());
        assert!(out.display.is_empty(), "nothing lands before the answer");
        let [(ticket, _)] = st.take_file_requests()[..] else {
            panic!("one read owed to the converter thread");
        };
        let out = st
            .handle_file_converted(ticket, Err(ambre_convert::ConvertError::Absent))
            .expect("the ticket is the board's");
        assert!(out.requests.is_empty());
        assert!(
            out.display
                .iter()
                .any(|event| matches!(event, ToileEvent::Interop(InteropAnswer::Failed(_)))),
            "the user is told: {:?}",
            out.display
        );
        assert_eq!(st.element_count(), Count::<Elements>::ZERO);
    }

    #[test]
    fn image_import_defers_until_media_uploads() {
        let mut st = ToileState::new();
        let out = st.handle_command(import_cmd(
            ToileImportFormat::Image(ambre_core::MediaType::from("image/png")),
            ambre_core::Payload::<ambre_client_core::FileBytes>::new(solid_png(8, 8)),
            Point::ZERO,
        ));
        // Async: one upload issued, nothing applied yet.
        assert_eq!(out.requests.len(), 1);
        assert!(out.update.is_none(), "board unchanged until media resolves");
        assert_eq!(st.element_count(), Count::<Elements>::ZERO);
        assert_eq!(st.pending_import_count(), Count::<PendingImports>::new(1));

        // Sync returns the real hash: the batch lands with it, no `pending:` left.
        let id = first_upload_id(&out);
        let done = st.handle_service_event(
            id,
            RestEvent::MediaUploaded {
                document_id: Id::<Document>::mint(),
                content_hash: hash_of("realhash"),
            },
        );
        assert!(has_report(&done));
        assert!(
            done.update.is_some(),
            "the applied batch broadcasts a delta"
        );
        assert_eq!(st.element_count(), Count::<Elements>::new(1));
        assert_eq!(st.media_count(), Count::<MediaEntries>::new(1));
        assert!(st.media_meta(&hx("realhash")).is_some());
        assert_eq!(st.pending_import_count(), Count::<PendingImports>::ZERO);
        // One Undo removes the whole (now-resolved) import.
        st.handle_command(ToileCommand::Undo);
        assert_eq!(st.element_count(), Count::<Elements>::ZERO);
    }

    /// Covers `TOILE-IO-031`: a picture on the board reaches the page as its
    /// pixels.
    ///
    /// The page reads the store the export fills from the board. With that
    /// store it draws one 8x8 image; with an empty one it draws none.
    #[cfg(feature = "pdf")]
    #[test]
    fn a_picture_reaches_the_page_as_pixels() {
        use ambre_draw::testing::{DrawOp, RecordingSurface};

        let mut st = ToileState::new();
        let out = st.handle_command(import_cmd(
            ToileImportFormat::Image(ambre_core::MediaType::from("image/png")),
            ambre_core::Payload::<ambre_client_core::FileBytes>::new(solid_png(8, 8)),
            Point::ZERO,
        ));
        st.handle_service_event(
            first_upload_id(&out),
            RestEvent::MediaUploaded {
                document_id: Id::<Document>::mint(),
                content_hash: hash_of("realhash"),
            },
        );
        let pages = st.pdf_pages(crate::logic::interop::PageSize::default());
        let paint = |images: &ambre_core::hash::HashMap<
            ambre_core::Validated<ambre_core::ContentHash>,
            crate::ui::render::DecodedImage,
        >| {
            let mut s = RecordingSurface::new();
            crate::logic::interop::paint_page(
                &mut s,
                &pages[0],
                ambre_canvas::Theme::Light.palette(),
                &mut ambre_draw::FontContext::default(),
                &mut ambre_canvas::TextCache::new(),
                (images, &crate::ui::walk::StoredPictures::default()),
            );
            s.count(|op| {
                matches!(
                    op,
                    DrawOp::DrawImage { size, .. }
                        if (size.width.get(), size.height.get()) == (8, 8)
                )
            })
        };
        assert_eq!(paint(&st.decoded_media()).get(), 1, "the picture's pixels");
        assert_eq!(paint(&ambre_core::hash::HashMap::default()).get(), 0);
    }

    /// Covers `TOILE-IO-031` through the recording the Logic thread runs. The
    /// page that draws a picture decodes it from the stored thumbnail, and
    /// the file carries its pixels.
    #[cfg(feature = "pdf")]
    #[test]
    fn a_recorded_page_decodes_its_picture() {
        let mut st = ToileState::new();
        let out = st.handle_command(import_cmd(
            ToileImportFormat::Image(ambre_core::MediaType::from("image/png")),
            ambre_core::Payload::<ambre_client_core::FileBytes>::new(solid_png(8, 8)),
            Point::ZERO,
        ));
        st.handle_service_event(
            first_upload_id(&out),
            RestEvent::MediaUploaded {
                document_id: Id::<Document>::mint(),
                content_hash: hash_of("realhash"),
            },
        );
        let mut recording = st
            .record_deck(crate::logic::interop::PageSize::default())
            .expect("the picture makes a page");
        recording.record_next(&mut ambre_draw::FontContext::default());
        assert!(recording.is_recorded());
        let document = recording.into_document();
        let sizes: Vec<_> = document
            .images
            .iter()
            .map(|image| (image.size.width.get(), image.size.height.get()))
            .collect();
        assert_eq!(sizes, vec![(8, 8)], "the picture's pixels reach the file");
    }

    /// A board holding one imported `w` x `h` picture, its full file, and a
    /// deck recording of it.
    #[cfg(feature = "pdf")]
    fn a_deck_of_one_picture(
        w: u32,
        h: u32,
    ) -> (
        ambre_core::Payload<ambre_core::MediaBytes>,
        crate::logic::interop::DeckRecording,
    ) {
        let mut st = ToileState::new();
        let file = solid_png(w, h);
        let out = st.handle_command(import_cmd(
            ToileImportFormat::Image(ambre_core::MediaType::from("image/png")),
            ambre_core::Payload::<ambre_client_core::FileBytes>::new(file.clone()),
            Point::ZERO,
        ));
        st.handle_service_event(
            first_upload_id(&out),
            RestEvent::MediaUploaded {
                document_id: Id::<Document>::mint(),
                content_hash: hash_of("realhash"),
            },
        );
        let recording = st
            .record_deck(crate::logic::interop::PageSize::default())
            .expect("the picture makes a page");
        (ambre_core::Payload::new(file), recording)
    }

    /// The one download a deck recording asks for, by its id.
    #[cfg(feature = "pdf")]
    fn the_one_download(
        recording: &mut crate::logic::interop::DeckRecording,
    ) -> Seq<ambre_bus::Request> {
        let requests = recording.fetch_full();
        let [ToileRequest::DownloadMedia { id, content_hash }] = &requests[..] else {
            panic!("one download per picture, got {requests:?}");
        };
        assert_eq!(*content_hash, hash_of("realhash"), "it names the address");
        *id
    }

    /// Covers `TOILE-IO-100`. A deck recording asks for the stored file of
    /// its picture, records no page before the answer, and then draws that
    /// file whole: the PDF carries the 600 x 400 picture, not the thumbnail
    /// capped at a 256 px edge. The report counts no approximated picture.
    #[cfg(feature = "pdf")]
    #[test]
    fn a_deck_page_draws_the_stored_file_at_full_resolution() {
        let (file, mut recording) = a_deck_of_one_picture(600, 400);
        let id = the_one_download(&mut recording);
        assert_eq!(
            recording.stage(),
            crate::logic::interop::DeckStage::Fetching,
            "no page before the file answered"
        );
        recording.answer(id, Some(file));
        assert_eq!(
            recording.stage(),
            crate::logic::interop::DeckStage::Recording
        );
        recording.record_next(&mut ambre_draw::FontContext::default());
        let report = recording.report();
        let document = recording.into_document();
        let encoded: Vec<_> = document
            .encoded
            .iter()
            .map(|image| {
                (
                    image.format,
                    image.size.width.get(),
                    image.size.height.get(),
                )
            })
            .collect();
        assert_eq!(
            encoded,
            vec![(ambre_draw::EncodedFormat::Png, 600, 400)],
            "the stored file reaches the page at its own size"
        );
        assert!(
            document.images.is_empty(),
            "no thumbnail is drawn beside it"
        );
        assert!(report.is_lossless(), "{report:?}");
    }

    /// Covers `TOILE-IO-100`, the refusal. The Sync worker opens a blob or
    /// answers an error: an altered ciphertext and a key that does not open
    /// it both end there. Bytes that spell no picture are refused here too.
    /// Either way the page never draws a broken file: it draws the
    /// thumbnail, and the report names the picture as approximated.
    #[cfg(feature = "pdf")]
    #[test]
    fn a_stored_file_that_did_not_open_falls_back_to_the_thumbnail_and_is_reported() {
        let answers = [
            None,
            Some(ambre_core::Payload::<ambre_core::MediaBytes>::new(
                b"\x89PNG not a picture".to_vec(),
            )),
        ];
        for data in answers {
            let (_, mut recording) = a_deck_of_one_picture(600, 400);
            let id = the_one_download(&mut recording);
            recording.answer(id, data);
            recording.record_next(&mut ambre_draw::FontContext::default());
            let report = recording.report();
            assert_eq!(
                report.count(
                    crate::message::ToileConstruct::Picture,
                    ambre_bus::interop::Outcome::Approximated,
                ),
                Count::new(1),
                "the fallback is reported"
            );
            let document = recording.into_document();
            assert!(
                document.encoded.is_empty(),
                "no broken file reaches the page"
            );
            let sizes: Vec<_> = document
                .images
                .iter()
                .map(|image| (image.size.width.get(), image.size.height.get()))
                .collect();
            assert_eq!(sizes, vec![(256, 171)], "the thumbnail is drawn instead");
        }
    }

    /// Covers `TOILE-IO-100`: an answer the recording did not ask for is
    /// not its own, and a second export that settles the first stops
    /// waiting, so the file still comes out from the thumbnail.
    #[cfg(feature = "pdf")]
    #[test]
    fn a_deck_recording_takes_only_its_own_answers() {
        let (file, mut recording) = a_deck_of_one_picture(600, 400);
        let id = the_one_download(&mut recording);
        recording.answer(ambre_bus::next_request(), Some(file));
        assert_eq!(recording.owes(id), Some(&hash_of("realhash")), "still owed");
        recording.stop_waiting();
        assert_eq!(
            recording.stage(),
            crate::logic::interop::DeckStage::Recording
        );
        recording.answer(id, None);
        assert_eq!(recording.owes(id), None, "a late answer is dropped");
    }

    /// `TOILE-IO-036`: the undo of a picture import takes back the entry it
    /// seeded with its element, and a redo brings both back.
    #[test]
    fn undoing_an_image_import_takes_its_media_entry_back() {
        let mut st = ToileState::new();
        let out = st.handle_command(import_cmd(
            ToileImportFormat::Image(ambre_core::MediaType::from("image/png")),
            ambre_core::Payload::<ambre_client_core::FileBytes>::new(solid_png(8, 8)),
            Point::ZERO,
        ));
        st.handle_service_event(
            first_upload_id(&out),
            RestEvent::MediaUploaded {
                document_id: Id::<Document>::mint(),
                content_hash: hash_of("realhash"),
            },
        );
        assert_eq!(
            (st.element_count(), st.media_count()),
            (Count::<Elements>::new(1), Count::<MediaEntries>::new(1))
        );
        st.handle_command(ToileCommand::Undo);
        assert_eq!(
            (st.element_count(), st.media_count()),
            (Count::<Elements>::ZERO, Count::<MediaEntries>::ZERO)
        );
        st.handle_command(ToileCommand::Redo);
        assert_eq!(
            (st.element_count(), st.media_count()),
            (Count::<Elements>::new(1), Count::<MediaEntries>::new(1))
        );
        assert!(st.media_meta(&hx("realhash")).is_some());
    }

    #[test]
    fn a_failed_upload_aborts_the_pending_import() {
        let mut st = ToileState::new();
        let out = st.handle_command(import_cmd(
            ToileImportFormat::Image(ambre_core::MediaType::from("image/png")),
            ambre_core::Payload::<ambre_client_core::FileBytes>::new(solid_png(8, 8)),
            Point::ZERO,
        ));
        let id = first_upload_id(&out);
        let failed = st.handle_service_error(
            id,
            ambre_core::ApiFailure::Relay {
                status: ambre_core::HttpStatus::new(500),
                code: None,
            },
        );
        assert!(
            failed
                .display
                .iter()
                .any(|e| matches!(e, ToileEvent::Interop(InteropAnswer::Failed(_))))
        );
        assert_eq!(
            st.element_count(),
            Count::<Elements>::ZERO,
            "board untouched (atomicity)"
        );
        assert_eq!(
            st.pending_import_count(),
            Count::<PendingImports>::ZERO,
            "the registry is drained"
        );
    }

    #[test]
    fn a_stale_service_response_is_dropped() {
        let mut st = ToileState::new();
        // No pending import: a late MediaUploaded (import undone / doc reopened)
        // is dropped without panic and changes nothing.
        let out = st.handle_service_event(
            ambre_bus::next_request(),
            RestEvent::MediaUploaded {
                document_id: Id::<Document>::mint(),
                content_hash: hash_of("orphan"),
            },
        );
        assert!(out.display.is_empty());
        assert!(out.update.is_none());
        assert_eq!(st.element_count(), Count::<Elements>::ZERO);
    }

    // -- slicing (`TOILE-IO-028`) -------------------------------------------

    /// Elements a large import carries: three slices and a bit.
    const LARGE: usize = SLICE * 3 + 7;

    /// [`IMPORT_SLICE`](super::IMPORT_SLICE) as a loop bound.
    const SLICE: usize = super::IMPORT_SLICE.get().get() as usize;

    fn large_import() -> ToileCommand {
        let mut svg = String::from(r#"<svg xmlns="http://www.w3.org/2000/svg">"#);
        for i in 0..LARGE {
            svg.push_str(&format!(r#"<rect x="{i}" y="0" width="5" height="5"/>"#));
        }
        svg.push_str("</svg>");
        import_cmd(
            ToileImportFormat::Svg,
            ambre_core::Payload::<ambre_client_core::FileBytes>::new(svg.into_bytes()),
            Point::ZERO,
        )
    }

    /// A large import applies one slice in the command, and the rest one slice
    /// per `resume`. Each slice is a delta of its own, and only the last one
    /// reports.
    #[test]
    fn a_large_import_lands_one_slice_at_a_time() {
        let mut st = ToileState::new();
        let first = st.handle_converted(large_import());
        assert!(first.update.is_some(), "the first slice is broadcast");
        assert!(!has_report(&first), "the import is not over");
        let mut reached = progress_of(&first).expect("the first slice reports its progress");
        assert_eq!(
            st.element_count(),
            Count::<Elements>::new(super::IMPORT_SLICE.get().get())
        );
        assert!(st.has_pending_work());
        let mut resumes = 0;
        let mut last = None;
        while st.has_pending_work() {
            let out = st.resume();
            assert!(out.update.is_some(), "every slice is broadcast");
            // `INTEROP-008`: each slice short of the end reports further on,
            // against the same total, and the last one reports instead.
            if st.has_pending_work() {
                let now = progress_of(&out).expect("a slice reports its progress");
                assert!(now.done() > reached.done(), "{now:?} after {reached:?}");
                assert_eq!(now.total(), reached.total());
                reached = now;
            } else {
                assert_eq!(progress_of(&out), None, "the report closes the import");
            }
            resumes += 1;
            last = Some(out);
        }
        // A unit is a create or a style, and the SVG converter styles every slot.
        assert_eq!(resumes, (2 * LARGE).div_ceil(SLICE) - 1);
        assert!(has_report(&last.expect("a last slice")));
        assert_eq!(st.element_count(), Count::<Elements>::of(LARGE));
        assert!(st.resume().update.is_none(), "nothing left to resume");
    }

    /// A camera move passes a running import: it reads nothing the import is
    /// still writing, and the board stays responsive.
    #[test]
    fn a_camera_move_does_not_wait_for_the_import() {
        let mut st = ToileState::new();
        st.handle_converted(large_import());
        let out = st.handle_command(ToileCommand::SetViewport(crate::logic::index::Viewport {
            center: Point::ZERO,
            zoom: ambre_core::Scale::<ambre_core::Board, ambre_core::View>::positive(0.5),
            size: Size::new(800.0, 600.0),
        }));
        assert!(out.update.is_none());
        assert_eq!(
            st.element_count(),
            Count::<Elements>::new(super::IMPORT_SLICE.get().get())
        );
        assert!(st.has_pending_work());
    }

    /// A command that writes the board lands the import whole first. Its one
    /// delta carries the rest of the import and the edit, and a peer that
    /// applies the deltas in order holds both.
    #[test]
    fn an_edit_lands_the_import_first_in_one_delta() {
        let mut st = ToileState::new();
        let mut peer = ToileState::new();
        let first = st.handle_converted(large_import());
        peer.apply_remote(&first.update.expect("first slice"));
        let edit = st.handle_command(rect_cmd(-100.0));
        assert!(!st.has_pending_work(), "the edit finished the import");
        assert!(has_report(&edit), "the edit's answer closes the import");
        peer.apply_remote(&edit.update.expect("one delta"));
        assert_eq!(st.element_count(), Count::<Elements>::of(LARGE + 1));
        assert_eq!(peer.element_count(), Count::<Elements>::of(LARGE + 1));
    }

    /// One Undo removes a sliced import whole: every slice shares one undo
    /// group (`TOILE-IO-001`).
    #[test]
    fn undo_removes_a_sliced_import_whole() {
        let mut st = ToileState::new();
        st.handle_command(rect_cmd(-100.0));
        st.handle_converted(large_import());
        while st.has_pending_work() {
            st.resume();
        }
        assert_eq!(st.element_count(), Count::<Elements>::of(LARGE + 1));
        st.handle_command(ToileCommand::Undo);
        assert_eq!(
            st.element_count(),
            Count::<Elements>::new(1),
            "the earlier rect stays"
        );
    }

    /// An Undo that arrives mid-import lands the import first, then removes
    /// it whole, never half of it.
    #[test]
    fn undo_mid_import_removes_it_whole() {
        let mut st = ToileState::new();
        st.handle_command(rect_cmd(-100.0));
        st.handle_converted(large_import());
        st.handle_command(ToileCommand::Undo);
        assert!(!st.has_pending_work());
        assert_eq!(st.element_count(), Count::<Elements>::new(1));
    }
}

/// A PowerPoint file onto the board (`toile-pptx-import`), through the
/// Office engine the host links in place, the media round trip answered as
/// Sync answers it.
#[cfg(test)]
mod pptx_tests {
    use ambre_bus::RestEvent;
    use ambre_bus::interop::{InteropAnswer, InteropRequest, Outcome};
    use ambre_core::id::{Document, Element, Id};
    use ambre_core::{Count, Elements, Validated};
    use kurbo::Point;

    use crate::logic::command::{ToileCommand, ToileEvent, ToileOutcome, ToileRequest};
    use crate::logic::interop::ContentHash;
    use crate::logic::state::ToileState;
    use crate::message::{ToileConstruct, ToileImport, ToileImportFormat, ToileReport};
    use ambre_pptx::report::DeckConstruct;

    /// The bytes of a fixture of `ambre-pptx` (`fixtures/AUTHORED.md`).
    fn fixture(name: &str) -> ambre_core::Payload<ambre_client_core::FileBytes> {
        let path = format!(
            "{}/../ambre-pptx/tests/fixtures/generated/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        ambre_core::Payload::new(std::fs::read(path).expect("the fixture is committed"))
    }

    fn import(at: Point, data: ambre_core::Payload<ambre_client_core::FileBytes>) -> ToileCommand {
        ToileCommand::Interop(InteropRequest::Import(ToileImport {
            format: ToileImportFormat::Pptx,
            data,
            at,
        }))
    }

    /// Answer every upload `out` asks for, each with its own address, and
    /// answer the outcome of the last one.
    fn upload_all(st: &mut ToileState, out: ToileOutcome) -> ToileOutcome {
        let ids: Vec<_> = out
            .requests
            .iter()
            .filter_map(|request| match request {
                ToileRequest::UploadMedia { id, .. } => Some(*id),
                _ => None,
            })
            .collect();
        let mut last = out;
        for (n, id) in ids.into_iter().enumerate() {
            let hex: String = format!("{n:064x}");
            last = st.handle_service_event(
                id,
                RestEvent::MediaUploaded {
                    document_id: Id::<Document>::mint(),
                    content_hash: Validated::<ContentHash>::parse(hex).expect("an address"),
                },
            );
        }
        last
    }

    /// The report a finished import answered.
    fn report_of(out: &ToileOutcome) -> ToileReport {
        out.display
            .iter()
            .find_map(|e| match e {
                ToileEvent::Interop(InteropAnswer::ImportCompleted { report }) => {
                    Some(report.clone())
                }
                _ => None,
            })
            .expect("the import closes with its report")
    }

    /// The board after the import of `name`, whole, and its report.
    fn imported(name: &str) -> (ToileState, ToileReport) {
        imported_bytes(fixture(name))
    }

    /// The board after the import of `data`, whole, and its report.
    fn imported_bytes(
        data: ambre_core::Payload<ambre_client_core::FileBytes>,
    ) -> (ToileState, ToileReport) {
        imported_from(import(Point::new(100.0, 50.0), data))
    }

    /// The board after the import `cmd` asks for, whole, and its report.
    fn imported_from(cmd: ToileCommand) -> (ToileState, ToileReport) {
        let mut st = ToileState::new();
        let out = st.handle_converted(cmd);
        let mut done = upload_all(&mut st, out);
        while st.has_pending_work() {
            done = st.resume();
        }
        let report = report_of(&done);
        (st, report)
    }

    /// An OpenDocument presentation of two pages: a title, a text frame
    /// and notes on the first, a rectangle on the second.
    fn odp_file() -> ambre_core::Payload<ambre_client_core::FileBytes> {
        use ambre_pptx::opc::PartName;
        use ambre_pptx::opc::odf::{OdfType, OdfWriter};
        let content = concat!(
            r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" "#,
            r#"xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" "#,
            r#"xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" "#,
            r#"xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" "#,
            r#"xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" "#,
            r#"xmlns:presentation="urn:oasis:names:tc:opendocument:xmlns:presentation:1.0" "#,
            r#"xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0">"#,
            r#"<office:automatic-styles><style:style style:name="gr1" style:family="graphic">"#,
            r##"<style:graphic-properties draw:fill="solid" draw:fill-color="#3366cc" draw:stroke="none"/>"##,
            r#"</style:style></office:automatic-styles>"#,
            r#"<office:body><office:presentation>"#,
            r#"<draw:page draw:name="page1"><draw:frame presentation:class="title" svg:x="2cm" svg:y="1cm" svg:width="20cm" svg:height="3cm"><draw:text-box><text:p>Opening</text:p></draw:text-box></draw:frame>"#,
            r#"<draw:frame svg:x="2cm" svg:y="5cm" svg:width="20cm" svg:height="8cm"><draw:text-box><text:p>First point</text:p><text:p>Second point</text:p></draw:text-box></draw:frame>"#,
            r#"<presentation:notes><draw:frame presentation:class="notes"><draw:text-box><text:p>Open with the plan.</text:p><text:p>Then move on.</text:p></draw:text-box></draw:frame></presentation:notes></draw:page>"#,
            r#"<draw:page draw:name="page2"><draw:rect draw:style-name="gr1" svg:x="4cm" svg:y="4cm" svg:width="6cm" svg:height="3cm"/></draw:page>"#,
            r#"</office:presentation></office:body></office:document-content>"#,
        );
        let mut w = OdfWriter::new(OdfType::PRESENTATION);
        w.part(
            PartName::new("content.xml").expect("a part name"),
            OdfType::XML,
            ambre_core::Payload::new(content.as_bytes().to_vec()),
        );
        w.finish().expect("the package writes").into_role()
    }

    /// An OpenDocument presentation lands on the board as its PowerPoint
    /// twin lands: the same frames, titles, notes and members. The report
    /// names the format the user opened.
    #[test]
    fn an_odp_lands_on_the_board_as_its_powerpoint_twin() {
        let odp = odp_file();
        let read = ambre_pptx::read_odp(odp.as_bytes()).expect("the presentation reads");
        let pptx = ambre_pptx::write(&read.deck).expect("the twin writes");
        let (from_odp, odp_report) =
            imported_from(ToileCommand::Interop(InteropRequest::Import(ToileImport {
                format: ToileImportFormat::Odp,
                data: odp,
                at: Point::new(100.0, 50.0),
            })));
        let (from_pptx, pptx_report) = imported_from(import(Point::new(100.0, 50.0), pptx));
        let shape = |st: &ToileState| {
            st.slide_order()
                .iter()
                .map(|f| {
                    let id = Id::<Element>::from(f.clone());
                    (st.frame_name(&id), st.notes(f), st.children_of(&id).len())
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            shape(&from_odp),
            [
                (
                    Some("Opening".to_owned()),
                    Some("Open with the plan.\nThen move on.".to_owned()),
                    2
                ),
                (None, Some(String::new()), 1),
            ]
        );
        assert_eq!(shape(&from_odp), shape(&from_pptx));
        assert_eq!(from_odp.element_count(), from_pptx.element_count());
        assert_eq!(odp_report.format(), ambre_bus::interop::FileFormat::Odp);
        assert_eq!(pptx_report.format(), ambre_bus::interop::FileFormat::Pptx);
        assert_eq!(odp_report.entries(), pptx_report.entries());
    }

    #[test]
    fn each_slide_lands_as_a_frame_of_the_deck_in_order() {
        for name in ["features.pptx", "libreoffice.pptx"] {
            let (st, _) = imported(name);
            let deck = st.slide_order();
            assert_eq!(deck.len(), 3, "{name}");
            let titles: Vec<Option<String>> = deck
                .iter()
                .map(|f| st.frame_name(&Id::<Element>::from(f.clone())))
                .collect();
            assert_eq!(
                titles,
                [
                    Some("Quarterly plan".to_owned()),
                    Some("Three shapes".to_owned()),
                    None
                ],
                "{name}"
            );
        }
    }

    #[test]
    fn the_notes_of_each_slide_reach_its_frame() {
        let (st, _) = imported("features.pptx");
        let notes: Vec<Option<String>> = st.slide_order().iter().map(|f| st.notes(f)).collect();
        assert_eq!(
            notes,
            [
                Some("Open with the plan.".to_owned()),
                Some("Point at each shape.\nThen move on.".to_owned()),
                Some(String::new())
            ]
        );
    }

    /// Each frame holds what its slide shows, the master's shape included.
    #[test]
    fn each_frame_holds_what_its_slide_shows() {
        let (st, _) = imported("features.pptx");
        let deck = st.slide_order();
        let held: Vec<usize> = deck
            .iter()
            .map(|f| st.children_of(&Id::<Element>::from(f.clone())).len())
            .collect();
        // Slide one: the mark of the master and two placeholders.
        // Slide two: the mark, two placeholders, four shapes, a text box.
        // Slide three: the mark, two pictures, two members, a poster, the
        // four cells of the table and the text of its one written cell, and
        // the box of the chart with the text of its values.
        assert_eq!(held, [3, 8, 13]);
    }

    /// The first frame lands where the user dropped the file.
    #[test]
    fn the_deck_lands_on_the_drop_point() {
        let (st, _) = imported("features.pptx");
        let first = st.slide_order().first().cloned().expect("a slide");
        let rect = st
            .element_rect(&Id::<Element>::from(first))
            .expect("the frame has a box");
        assert_eq!((rect.x0, rect.y0), (100.0, 50.0));
        assert_eq!((rect.width(), rect.height()), (720.0, 540.0));
    }

    #[test]
    fn the_report_names_what_the_board_could_not_hold() {
        let (_, report) = imported("features.pptx");
        for (construct, outcome) in [
            (DeckConstruct::Transition, Outcome::Approximated),
            (DeckConstruct::SmartArt, Outcome::Dropped),
            (DeckConstruct::Chart, Outcome::Approximated),
            (DeckConstruct::Table, Outcome::Approximated),
            (DeckConstruct::Video, Outcome::Approximated),
            (DeckConstruct::Animation, Outcome::Dropped),
        ] {
            assert!(
                report.count(ToileConstruct::Deck(construct), outcome) > Count::ZERO,
                "{construct:?}"
            );
        }
        assert_eq!(
            report.count(ToileConstruct::SpeakerNotes, Outcome::Carried),
            Count::new(2)
        );
    }

    #[test]
    fn one_undo_takes_the_whole_deck_back() {
        let (mut st, _) = imported("features.pptx");
        assert!(st.element_count() > Count::<Elements>::ZERO);
        st.handle_command(ToileCommand::Undo);
        assert_eq!(st.element_count(), Count::<Elements>::ZERO);
        assert!(st.slide_order().is_empty());
    }

    /// The deck joins the end of the deck the board holds, and touches none
    /// of the elements already there.
    #[test]
    fn a_deck_leaves_the_board_it_lands_on_alone() {
        let mut st = ToileState::new();
        let held = st
            .handle_command(ToileCommand::CreateElement {
                kind: crate::logic::model::ElementKind::Frame,
                at: Point::new(-2000.0, 0.0),
                size: kurbo::Size::new(400.0, 300.0),
                content: crate::logic::command::InitContent::None,
                fill: None,
                correlate: None,
            })
            .display
            .into_iter()
            .find_map(|e| match e {
                ToileEvent::ElementCreated { id, .. } => Some(id),
                _ => None,
            })
            .expect("a minted id");
        let frame = st.frame(&held).expect("a frame");
        st.handle_command(ToileCommand::SetSlideOrder {
            order: vec![frame.clone()],
        });
        let before = st.element_map(&held).expect("held").map().get_deep_value();
        let out = st.handle_converted(import(Point::ZERO, fixture("features.pptx")));
        let _ = upload_all(&mut st, out);
        while st.has_pending_work() {
            let _ = st.resume();
        }
        let deck = st.slide_order();
        assert_eq!(deck.len(), 4);
        assert_eq!(deck.first(), Some(&frame));
        assert_eq!(
            st.element_map(&held).expect("held").map().get_deep_value(),
            before
        );
    }

    /// A file the reader cannot read imports nothing, and the board stays.
    #[test]
    fn a_broken_file_imports_nothing() {
        let mut st = ToileState::new();
        let out = st.handle_converted(import(
            Point::ZERO,
            ambre_core::Payload::new(b"not a presentation".to_vec()),
        ));
        assert!(
            out.display
                .iter()
                .any(|e| matches!(e, ToileEvent::Interop(InteropAnswer::Failed(_))))
        );
        assert_eq!(st.element_count(), Count::<Elements>::ZERO);
        assert!(st.slide_order().is_empty());
    }

    /// Every PowerPoint input fixture (`fixtures/AUTHORED.md`).
    const DECK_FIXTURES: [&str; 2] = ["features.pptx", "libreoffice.pptx"];

    /// One item of a frame as the board stores it, its box from the
    /// frame's corner. The media address is left out: the upload answers it.
    #[derive(Debug, PartialEq)]
    struct StoredItem {
        kind: crate::logic::model::ElementKind,
        rect: kurbo::Rect,
        rotation: ambre_core::Radians,
        style: crate::message::ElementStyle,
        path: Option<String>,
        text: Option<String>,
        thumbnail: Option<Vec<u8>>,
    }

    /// One connector of a frame as the board stores it, each end by the
    /// turn of its element among the items of the frame.
    #[derive(Debug, PartialEq)]
    struct StoredConnector {
        from: Option<usize>,
        from_anchor: crate::message::Anchor,
        to: Option<usize>,
        to_anchor: crate::message::Anchor,
        path: crate::message::PathType,
        caps: crate::message::EndCaps,
        style: crate::message::ElementStyle,
    }

    /// One slide of the deck as the board stores it.
    #[derive(Debug, PartialEq)]
    struct StoredSlide {
        name: Option<String>,
        notes: Option<String>,
        size: kurbo::Size,
        items: Vec<StoredItem>,
        connectors: Vec<StoredConnector>,
    }

    /// The deck `st` holds, read from the board and never from the export
    /// map: each frame in deck order, its items in paint order.
    fn stored_deck(st: &ToileState) -> Vec<StoredSlide> {
        use crate::logic::model::NodeId;
        st.slide_order()
            .iter()
            .map(|frame| {
                let id = Id::<Element>::from(frame.clone());
                let origin = st.element_rect(&id).expect("the frame has a box");
                let children = st.children_of(&id);
                let members: Vec<Id<Element>> = st
                    .layer_order()
                    .iter()
                    .filter_map(|node| match node {
                        NodeId::Element(e) if children.contains(e) => Some(e.clone()),
                        _ => None,
                    })
                    .collect();
                let items = members
                    .iter()
                    .map(|e| {
                        let el = st.export_element(e).expect("a child is stored");
                        StoredItem {
                            kind: el.kind,
                            rect: el.rect - origin.origin().to_vec2(),
                            rotation: el.rotation,
                            style: el.style,
                            path: el.path.map(|p| p.to_svg()),
                            text: el.text,
                            thumbnail: el.image_png.map(|png| png.as_ref().to_vec()),
                        }
                    })
                    .collect();
                let turn = |e: &Id<Element>| members.iter().position(|m| m == e);
                let wires = st.wires();
                let connectors = st
                    .layer_order()
                    .iter()
                    .filter_map(|node| match node {
                        NodeId::Connector(c) => wires.get(c).map(|w| (c, &w.conn)),
                        NodeId::Element(_) => None,
                    })
                    .filter(|(_, conn)| turn(&conn.from).is_some() || turn(&conn.to).is_some())
                    .map(|(c, conn)| StoredConnector {
                        from: turn(&conn.from),
                        from_anchor: conn.from_anch,
                        to: turn(&conn.to),
                        to_anchor: conn.to_anch,
                        path: conn.path_type,
                        caps: conn.arrows,
                        style: st.style_read(
                            st.connector_map(c)
                                .and_then(|cm| {
                                    super::super::codec::bin_of(
                                        &cm,
                                        super::super::names::ConnectorKey::Style,
                                    )
                                })
                                .as_deref(),
                        ),
                    })
                    .collect();
                StoredSlide {
                    name: st.frame_name(&id),
                    notes: st.notes(frame),
                    size: origin.size(),
                    items,
                    connectors,
                }
            })
            .collect()
    }

    /// The PowerPoint file an export of `st` writes, and its report.
    fn exported_pptx(
        st: &mut ToileState,
    ) -> (
        ambre_core::Payload<ambre_client_core::FileBytes>,
        ToileReport,
    ) {
        let out = st.handle_converted(ToileCommand::Interop(InteropRequest::Export(
            crate::message::ToileExport::Pptx,
        )));
        out.display
            .into_iter()
            .find_map(|e| match e {
                ToileEvent::Interop(InteropAnswer::ExportReady { file, report }) => {
                    Some((file.data, report))
                }
                _ => None,
            })
            .expect("the export answers a file")
    }

    /// `TOILE-IO-172`: each PowerPoint input fixture, imported, exported and
    /// imported again, lands as the same deck the board stored the first
    /// time, compared as stored data.
    #[test]
    fn every_deck_fixture_lands_the_same_deck_after_an_export_and_an_import() {
        for name in DECK_FIXTURES {
            let (mut first, _) = imported(name);
            let (file, exported) = exported_pptx(&mut first);
            let (second, _) = imported_bytes(file);
            let before = stored_deck(&first);
            assert_eq!(before.len(), 3, "{name}: the fixture lands its slides");
            assert!(
                before.iter().any(|s| !s.connectors.is_empty()),
                "{name}: the fixture lands a bound connector"
            );
            assert!(
                before
                    .iter()
                    .flat_map(|s| &s.items)
                    .any(|i| i.thumbnail.is_some()),
                "{name}: the fixture lands a picture"
            );
            let after = stored_deck(&second);
            for (n, (a, b)) in before.iter().zip(&after).enumerate() {
                assert_eq!(b, a, "{name}: slide {n}, the export reported {exported:?}");
            }
            assert_eq!(after.len(), before.len(), "{name}");
        }
    }
}
