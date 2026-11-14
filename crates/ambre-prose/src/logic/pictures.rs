//! The picture commands, and the answers of the media flow
//! (`features/ambre-spec-prose-images.md section 3`).
//!
//! A picture block holds an address and a display size. The bytes travel
//! through the Sync worker: Prose mints a request, the Logic dispatcher
//! lowers it, and the answer comes back here by its id. Nothing here makes a
//! network call or names a key (`PROSE-025`).

use std::collections::HashMap;
use std::sync::Arc;

use ambre_bus::RestEvent;
use ambre_bus::rest::Request;
use ambre_core::index::{Axis, Pos};
use ambre_core::{AltText, Caption, ContentHash, MediaBytes, Payload, Seq, Validated};
use loro::{LoroValue, TreeID};

use super::ProseState;
use crate::doc;
use crate::events::{BlockId, ProseEvent, ProseOutcome, block_id};
use crate::message::{
    AspectLock, BlockType, DecodedPicture, DisplaySize, ImageAlign, MediaFailure, MediaRequest,
    ProseCommand, ProseError, ProseRefusal, RefusedAction,
};
use crate::picture::{self, ImageRecord, Want};

/// A picture chosen from a file, stored through the media flow, and written
/// once its address comes back (`PROSE-FEAT-316`).
pub(super) struct PendingInsert {
    parent_id: Option<TreeID>,
    after_id: Option<TreeID>,
    alt: Validated<AltText>,
    size: DisplaySize,
    /// Decoded once on the way in, so the author sees the picture with no
    /// download.
    shown: Prepared,
}

/// What an insert from a file shows once its block is written.
pub(super) enum Prepared {
    /// A PNG, decoded on the way in.
    Decoded(Arc<DecodedPicture>),
    /// A JPEG, decoded on the converter thread once the address came back.
    /// The block shows the placeholder until then (`PROSE-FEAT-330`).
    Jpeg(ambre_convert::jpeg::JpegDecode),
}

/// What a request in flight waits for.
pub(super) enum InFlight {
    /// The bytes of an address the view shows.
    Shown(Validated<ContentHash>),
    /// The address of a picture chosen from a file.
    Insert(Box<PendingInsert>),
    /// The address of a picture of an import (`PROSE-FEAT-319`).
    Import,
    /// The bytes of a picture of an export (`PROSE-IO-007`).
    Export,
}

/// The index space of the distinct pictures an import uploads: one place
/// per upload, however many pictures of the file share its bytes.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Distinct {}

impl Axis for Distinct {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("Distinct");
}

/// An import whose pictures are being stored. Nothing is written until
/// every upload answered, so the file lands whole (`PROSE-IO-011`).
pub(super) struct Uploading {
    ir: Box<crate::interop::ProseIr>,
    report: crate::message::ConversionReport,
    now: ambre_core::Instant<ambre_core::Authored>,
    /// The distinct picture each upload stores, by its request.
    outstanding: HashMap<Seq<Request>, Pos<Distinct>>,
    /// The distinct picture of each picture of the form, in the order of
    /// the walk, or `None` for one with no bytes to store. The form gave its
    /// bytes up to the upload, so the answer maps an address by its place.
    slots: Vec<Option<Pos<Distinct>>>,
    /// The address each distinct picture got, by its place among them.
    /// `None` until its upload answers, and for good when it failed.
    addresses: ambre_core::index::IdxVec<Distinct, Option<Validated<ContentHash>>>,
}

/// An export whose pictures are being fetched. The walk starts once every
/// download answered, on the document as it stands then
/// (`PROSE-IO-040`, `PROSE-IO-105`).
pub(super) struct Fetching {
    format: crate::message::ExportFormat,
    outstanding: HashMap<Seq<Request>, Validated<ContentHash>>,
    fetched: HashMap<Validated<ContentHash>, Payload<MediaBytes>>,
    /// The bytes `fetched` holds, held under [`EXPORT_BYTES_CAP`].
    held: ambre_core::Count<ambre_core::Octets>,
}

/// A conversion whose media answered while another conversion ran.
pub(super) enum Queued {
    Import {
        ir: Box<crate::interop::ProseIr>,
        report: crate::message::ConversionReport,
        now: ambre_core::Instant<ambre_core::Authored>,
    },
    Export {
        format: crate::message::ExportFormat,
        fetched: HashMap<Validated<ContentHash>, Payload<MediaBytes>>,
    },
}

/// The encoded picture bytes one export holds (`PROSE-IO-106`,
/// `ambre-spec-budgets.md section 6`). A picture past it goes out as a
/// placeholder and a line of the report.
pub(super) const EXPORT_BYTES_CAP: ambre_core::Count<ambre_core::Octets> =
    ambre_core::Count::new(32 * 1024 * 1024);

/// Whether an export that holds `held` bytes of pictures takes `more`
/// within [`EXPORT_BYTES_CAP`] (`PROSE-IO-106`).
fn admits(
    held: ambre_core::Count<ambre_core::Octets>,
    more: ambre_core::Count<ambre_core::Octets>,
) -> bool {
    held.plus(more) <= EXPORT_BYTES_CAP
}

/// Each picture of `ir` that carries bytes, in a walk of the blocks in
/// document order, the nested ones included.
fn images_of(ir: &mut crate::interop::ProseIr) -> Vec<&mut ambre_prose_form::IrImage> {
    use crate::interop::IrBlock;
    fn walk<'a>(blocks: &'a mut [IrBlock], out: &mut Vec<&'a mut ambre_prose_form::IrImage>) {
        for block in blocks {
            match block {
                IrBlock::Image(image) => out.push(image),
                IrBlock::ListItem { children, .. } => walk(children, out),
                IrBlock::Table { rows, .. } => {
                    for cell in rows.iter_mut().flat_map(|row| row.cells.iter_mut()) {
                        walk(&mut cell.blocks, out);
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&mut ir.blocks, &mut out);
    out
}

impl ProseState {
    // -- the commands ------------------------------------------------------

    /// Write one picture block at a place in the tree (`PROSE-FEAT-210`).
    /// The nesting table puts a picture at the root alone, so any other
    /// parent is refused (`PROSE-FEAT-052`, `PROSE-FEAT-211`).
    pub(super) fn insert_image(
        &mut self,
        parent_id: Option<TreeID>,
        after_id: Option<TreeID>,
        address: &Validated<ContentHash>,
        alt: &Validated<AltText>,
        size: DisplaySize,
    ) -> ProseOutcome {
        let Ok(size) = picture::clamp_display(size) else {
            return ProseOutcome::refused(ProseRefusal::PictureWithoutArea);
        };
        match self.write_image(parent_id, after_id, address, alt, size) {
            Ok(_) => self.commit_with(Vec::new()),
            Err(refused) => refused,
        }
    }

    /// Mint the block and write its record, without the commit.
    fn write_image(
        &mut self,
        parent_id: Option<TreeID>,
        after_id: Option<TreeID>,
        address: &Validated<ContentHash>,
        alt: &Validated<AltText>,
        size: DisplaySize,
    ) -> Result<TreeID, ProseOutcome> {
        let id = self.mint_placed(parent_id, after_id, BlockType::Image)?;
        let tree = doc::blocks(&self.doc);
        let attrs = doc::node_of(&tree, &id).and_then(|node| doc::attrs_at(&node));
        let Some(attrs) = attrs else {
            return Err(Self::err_write(doc::WriteError::Refused(
                doc::RefusalCause::AttrMapRefused,
            )));
        };
        let written = attrs
            .insert(doc::AttrKey::ContentHash, address.as_str())
            .and_then(|()| attrs.insert(doc::AttrKey::ImageAlt, LoroValue::from(alt.as_str())))
            .and_then(|()| attrs.insert(doc::AttrKey::ImageWidth, side(size.width)))
            .and_then(|()| attrs.insert(doc::AttrKey::ImageHeight, side(size.height)))
            .and_then(|()| {
                attrs.insert(
                    doc::AttrKey::ImageAlign,
                    LoroValue::I64(i64::from(ImageAlign::Block.byte())),
                )
            });
        written.map(|()| id).map_err(Self::err_write)
    }

    /// Write a picture an import carries, its texts and alignment with it.
    /// The commit is the import's own.
    pub(super) fn write_imported_image(
        &mut self,
        parent_id: Option<TreeID>,
        after_id: Option<TreeID>,
        address: &Validated<ContentHash>,
        (alt, caption): (&Validated<AltText>, &Validated<Caption>),
        size: DisplaySize,
        align: ImageAlign,
    ) -> Result<TreeID, ProseOutcome> {
        let id = self.write_image(parent_id, after_id, address, alt, size)?;
        let tree = doc::blocks(&self.doc);
        let attrs = doc::node_of(&tree, &id).and_then(|node| doc::attrs_at(&node));
        let written = attrs.map(|attrs| {
            attrs
                .insert(
                    doc::AttrKey::ImageCaption,
                    LoroValue::from(caption.as_str()),
                )
                .and_then(|()| {
                    attrs.insert(
                        doc::AttrKey::ImageAlign,
                        LoroValue::I64(i64::from(align.byte())),
                    )
                })
        });
        match written {
            Some(Ok(())) => Ok(id),
            Some(Err(e)) => Err(Self::err_write(e)),
            None => Err(Self::err_write(doc::WriteError::Refused(
                doc::RefusalCause::AttrMapRefused,
            ))),
        }
    }

    /// Check a file the reader chose, decode it, and store it through the
    /// media flow. The block is written when the address comes back
    /// (`PROSE-FEAT-316`). A file this build cannot read is refused before
    /// any byte leaves (`PROSE-FEAT-314`).
    pub(super) fn insert_image_file(
        &mut self,
        parent_id: Option<TreeID>,
        after_id: Option<TreeID>,
        data: &Payload<ambre_core::FileBytes>,
        alt: Validated<AltText>,
    ) -> ProseOutcome {
        let tree = doc::blocks(&self.doc);
        // A gesture that names no place takes the caret's.
        let after_id = match (parent_id, after_id) {
            (None, None) => self.caret_root(&tree),
            (_, after_id) => after_id,
        };
        let Ok(parent) = super::blocks::landing_parent(&tree, parent_id, after_id) else {
            return Self::err_nesting();
        };
        if !doc::nesting_allows(&tree, parent, BlockType::Image) {
            return Self::err_nesting();
        }
        let data: Payload<MediaBytes> = data.clone().into_role();
        let native = match picture::native_size(&data) {
            Ok(native) => native,
            Err(_) => return ProseOutcome::refused(ProseRefusal::PictureUnreadable),
        };
        // The location, the device and the dates the file carries never
        // leave: only the stripped bytes are stored (`PROSE-FEAT-326`).
        let Ok(stored) = picture::stripped(&data) else {
            return ProseOutcome::refused(ProseRefusal::PictureUnreadable);
        };
        let shown = match picture::decode(stored.clone()) {
            Ok(picture::Decode::Done(decoded)) => Prepared::Decoded(Arc::new(decoded)),
            Ok(picture::Decode::Jpeg(request)) => Prepared::Jpeg(request),
            Err(_) => return ProseOutcome::refused(ProseRefusal::PictureUnreadable),
        };
        let id = ambre_bus::next_request();
        self.in_flight.insert(
            id,
            InFlight::Insert(Box::new(PendingInsert {
                parent_id,
                after_id,
                alt,
                size: picture::display_of_native(native),
                shown,
            })),
        );
        self.media_outbox
            .push(MediaRequest::Upload { id, data: stored });
        ProseOutcome::nothing()
    }

    /// The root block the caret sits in, or the last root block when the
    /// caret names none this document holds.
    fn caret_root(&self, tree: &loro::LoroTree) -> Option<TreeID> {
        let mut at = self.caret.filter(|id| doc::node_of(tree, id).is_some());
        while let Some(id) = at {
            match tree.parent(id) {
                Some(loro::TreeParentId::Node(parent)) => at = Some(parent),
                _ => break,
            }
        }
        at.filter(|id| !doc::is_aside(tree, id))
            .or_else(|| doc::roots(tree).last().copied())
    }

    /// Validate, clamp and write a display size (`PROSE-FEAT-058`,
    /// `PROSE-FEAT-059`, `PROSE-FEAT-213`). The address, the alignment and
    /// the texts are not touched (`PROSE-FEAT-216`).
    pub(super) fn resize_image(
        &mut self,
        id: &TreeID,
        asked: DisplaySize,
        lock: AspectLock,
    ) -> ProseOutcome {
        let Some(record) = self.image_record(id) else {
            return ProseOutcome::refused(ProseRefusal::NotAPicture(block_id(*id)));
        };
        let decoded = record
            .address
            .as_ref()
            .and_then(|address| self.pictures.decoded(address))
            .map(|picture| picture.size);
        let wanted = picture::locked_size(asked, record.size, lock, decoded);
        let Ok(size) = picture::clamp_display(wanted) else {
            return ProseOutcome::refused(ProseRefusal::PictureWithoutArea);
        };
        self.write_attrs(
            id,
            &[
                (doc::AttrKey::ImageWidth, side(size.width)),
                (doc::AttrKey::ImageHeight, side(size.height)),
            ],
        )
    }

    /// Store the alignment named, whether or not the layout honours it
    /// (`PROSE-FEAT-217`, `PROSE-FEAT-218`).
    pub(super) fn set_image_align(&mut self, id: &TreeID, align: ImageAlign) -> ProseOutcome {
        if self.image_record(id).is_none() {
            return ProseOutcome::refused(ProseRefusal::NotAPicture(block_id(*id)));
        }
        self.write_attrs(
            id,
            &[(
                doc::AttrKey::ImageAlign,
                LoroValue::I64(i64::from(align.byte())),
            )],
        )
    }

    /// Store a text on the field it names, on that block alone
    /// (`PROSE-FEAT-219`, `PROSE-FEAT-220`).
    fn set_image_text(&mut self, id: &TreeID, field: (doc::AttrKey, LoroValue)) -> ProseOutcome {
        if self.image_record(id).is_none() {
            return ProseOutcome::refused(ProseRefusal::NotAPicture(block_id(*id)));
        }
        self.write_attrs(id, &[field])
    }

    pub(super) fn set_image_alt(&mut self, id: &TreeID, alt: &Validated<AltText>) -> ProseOutcome {
        self.set_image_text(id, (doc::AttrKey::ImageAlt, LoroValue::from(alt.as_str())))
    }

    pub(super) fn set_image_caption(
        &mut self,
        id: &TreeID,
        caption: &Validated<Caption>,
    ) -> ProseOutcome {
        self.set_image_text(
            id,
            (
                doc::AttrKey::ImageCaption,
                LoroValue::from(caption.as_str()),
            ),
        )
    }

    /// Put the picture's address back to unasked and ask again
    /// (`PROSE-FEAT-221`). The document is not written, and no other address
    /// is touched (`PROSE-FEAT-222`).
    pub(super) fn retry_image_load(&mut self, id: &TreeID) -> ProseOutcome {
        let Some(address) = self.image_record(id).and_then(|record| record.address) else {
            return ProseOutcome::refused(ProseRefusal::NotAPicture(block_id(*id)));
        };
        self.pictures.forget(&address);
        self.want_picture(&address);
        ProseOutcome::display_only(vec![self.picture_state(&address)])
    }

    /// Write `fields` on the attribute map of `id`, then commit.
    fn write_attrs(&mut self, id: &TreeID, fields: &[(doc::AttrKey, LoroValue)]) -> ProseOutcome {
        let tree = doc::blocks(&self.doc);
        let Some(attrs) = doc::node_of(&tree, id).and_then(|node| doc::attrs_at(&node)) else {
            return Self::err_no_block(id);
        };
        for (key, value) in fields {
            if let Err(e) = attrs.insert(*key, value.clone()) {
                return Self::err_write(e);
            }
        }
        self.commit_with(Vec::new())
    }

    /// The record of the live picture block `id`.
    pub(super) fn image_record(&self, id: &TreeID) -> Option<ImageRecord> {
        picture::read_image(&doc::blocks(&self.doc), id)
    }

    // -- the media flow ----------------------------------------------------

    /// The requests the media flow owes the Sync worker, drained. The Logic
    /// dispatcher lowers each one with the document's id.
    pub fn take_media_requests(&mut self) -> Vec<MediaRequest> {
        std::mem::take(&mut self.media_outbox)
    }

    /// Mark `address` painted, and ask for it when nothing holds it yet
    /// (`PROSE-FEAT-320`). A failed address is not asked again
    /// (`PROSE-FEAT-057`).
    pub(super) fn want_picture(&mut self, address: &Validated<ContentHash>) {
        if self.pictures.want(address) == Want::Fetch {
            let id = ambre_bus::next_request();
            self.in_flight.insert(id, InFlight::Shown(address.clone()));
            self.media_outbox.push(MediaRequest::Download {
                id,
                content_hash: address.clone(),
            });
        }
    }

    /// Ask for every picture the layout handed to the view carries.
    pub(super) fn want_pictures_of(&mut self, addresses: Vec<Validated<ContentHash>>) {
        for address in addresses {
            self.want_picture(&address);
        }
    }

    /// The event that tells the view what `address` shows now, naming every
    /// block that shows it (`PROSE-060`).
    fn picture_state(&self, address: &Validated<ContentHash>) -> ProseEvent {
        ProseEvent::PictureState {
            content_hash: address.clone(),
            shown: self.pictures.shown(address),
            blocks: self.blocks_showing(address),
        }
    }

    /// The answer of a request this document made, or `None` when the id is
    /// not one of its own. A picture arriving writes nothing to the document
    /// (`PROSE-061`): only a picture chosen from a file writes its block.
    pub fn handle_media_event(&mut self, id: Seq<Request>, evt: RestEvent) -> Option<ProseOutcome> {
        let waiting = self.in_flight.remove(&id)?;
        Some(match (waiting, evt) {
            (InFlight::Shown(address), RestEvent::MediaDownloaded { data, .. }) => {
                self.picture_arrived(&address, data)
            }
            (InFlight::Insert(pending), RestEvent::MediaUploaded { content_hash, .. }) => {
                self.insert_uploaded(*pending, &content_hash)
            }
            (InFlight::Import, RestEvent::MediaUploaded { content_hash, .. }) => {
                self.import_answered(id, Some(content_hash))
            }
            (InFlight::Export, RestEvent::MediaDownloaded { data, .. }) => {
                self.export_answered(id, Some(data))
            }
            (InFlight::Import, _) => self.import_answered(id, None),
            (InFlight::Export, _) => self.export_answered(id, None),
            (InFlight::Shown(address), _) => {
                self.picture_failed(&address, MediaFailure::Unavailable)
            }
            (InFlight::Insert(_), _) => Self::upload_refused(),
        })
    }

    /// The failure of a request this document made, or `None` when the id is
    /// not one of its own.
    pub fn handle_media_error(&mut self, id: Seq<Request>) -> Option<ProseOutcome> {
        let waiting = self.in_flight.remove(&id)?;
        Some(match waiting {
            InFlight::Shown(address) => self.picture_failed(&address, MediaFailure::Unavailable),
            InFlight::Insert(_) => Self::upload_refused(),
            InFlight::Import => self.import_answered(id, None),
            InFlight::Export => self.export_answered(id, None),
        })
    }

    // -- an import's pictures ----------------------------------------------

    /// Store each distinct picture `ir` carries, then land the import once
    /// every address answered (`PROSE-FEAT-319`). A file with no picture
    /// lands now. An import already waiting is dropped for this one.
    pub(super) fn upload_pictures(
        &mut self,
        mut ir: crate::interop::ProseIr,
        report: crate::message::ConversionReport,
        now: ambre_core::Instant<ambre_core::Authored>,
    ) -> ProseOutcome {
        let mut distinct: Vec<Payload<MediaBytes>> = Vec::new();
        let mut slots = Vec::new();
        for image in images_of(&mut ir) {
            // A picture is stored without its metadata, as one chosen from a
            // file is (`PROSE-FEAT-326`). One this client cannot strip is
            // neither a PNG nor a JPEG: nothing here could show it, so it is
            // left out and named. The form gives its bytes up here, so the
            // import holds one copy of each picture until its upload leaves.
            let stripped = image
                .bytes
                .take()
                .and_then(|bytes| picture::stripped(&bytes.data.into_role()).ok());
            slots.push(stripped.map(|data| {
                match distinct
                    .iter()
                    .position(|seen| seen.as_bytes() == data.as_bytes())
                {
                    Some(slot) => Pos::<Distinct>::from_usize(slot),
                    None => {
                        distinct.push(data);
                        Pos::<Distinct>::from_usize(distinct.len() - 1)
                    }
                }
            }));
        }
        if distinct.is_empty() {
            return self.land_import(ir, report, now);
        }
        let mut outstanding = HashMap::new();
        for (slot, data) in distinct.into_iter().enumerate() {
            let id = ambre_bus::next_request();
            outstanding.insert(id, Pos::<Distinct>::from_usize(slot));
            self.in_flight.insert(id, InFlight::Import);
            self.media_outbox.push(MediaRequest::Upload { id, data });
        }
        let count = outstanding.len();
        self.uploading = Some(Uploading {
            ir: Box::new(ir),
            report,
            now,
            outstanding,
            slots,
            addresses: (0..count).map(|_| None).collect(),
        });
        ProseOutcome::nothing()
    }

    /// One upload of the waiting import answered: its address, or `None`
    /// when it stored nothing. The last answer lands the import, each
    /// picture with the address of its bytes, and one whose upload failed
    /// left out and named (`PROSE-FEAT-319`).
    fn import_answered(
        &mut self,
        id: Seq<Request>,
        address: Option<Validated<ContentHash>>,
    ) -> ProseOutcome {
        let Some(uploading) = self.uploading.as_mut() else {
            return ProseOutcome::nothing();
        };
        let Some(slot) = uploading.outstanding.remove(&id) else {
            return ProseOutcome::nothing();
        };
        if let Some(held) = uploading.addresses.get_mut(slot) {
            *held = address;
        }
        if !uploading.outstanding.is_empty() {
            return ProseOutcome::nothing();
        }
        let Some(Uploading {
            mut ir,
            report,
            now,
            slots,
            addresses,
            ..
        }) = self.uploading.take()
        else {
            return ProseOutcome::nothing();
        };
        for (image, slot) in images_of(&mut ir).into_iter().zip(slots) {
            image.address = slot.and_then(|slot| addresses.get(slot).cloned().flatten());
        }
        self.start_or_queue(Queued::Import { ir, report, now })
    }

    // -- an export's pictures ----------------------------------------------

    /// Fetch every picture the document addresses, then start the export
    /// on what came back (`PROSE-IO-007`). A document with no picture starts
    /// now. An export already waiting is dropped for this one.
    pub(super) fn fetch_pictures(&mut self, format: crate::message::ExportFormat) -> ProseOutcome {
        let tree = doc::blocks(&self.doc);
        let mut addresses: Vec<Validated<ContentHash>> = tree
            .nodes()
            .iter()
            .filter(|id| doc::node_of(&tree, id).is_some())
            .filter_map(|id| picture::read_image(&tree, id).and_then(|record| record.address))
            .collect();
        addresses.sort_unstable();
        addresses.dedup();
        if addresses.is_empty() {
            return self.start_or_queue(Queued::Export {
                format,
                fetched: HashMap::new(),
            });
        }
        let mut outstanding = HashMap::new();
        for address in addresses {
            let id = ambre_bus::next_request();
            self.in_flight.insert(id, InFlight::Export);
            self.media_outbox.push(MediaRequest::Download {
                id,
                content_hash: address.clone(),
            });
            outstanding.insert(id, address);
        }
        self.fetching = Some(Fetching {
            format,
            outstanding,
            fetched: HashMap::new(),
            held: ambre_core::Count::ZERO,
        });
        ProseOutcome::nothing()
    }

    /// One download of the waiting export answered: the bytes, or `None`
    /// when the media flow served none. A picture past the export's cap is
    /// let go. The last answer starts the export.
    fn export_answered(
        &mut self,
        id: Seq<Request>,
        data: Option<Payload<MediaBytes>>,
    ) -> ProseOutcome {
        let Some(fetching) = self.fetching.as_mut() else {
            return ProseOutcome::nothing();
        };
        let Some(address) = fetching.outstanding.remove(&id) else {
            return ProseOutcome::nothing();
        };
        if let Some(data) = data
            && admits(fetching.held, data.len())
        {
            fetching.held = fetching.held.plus(data.len());
            fetching.fetched.insert(address, data);
        }
        if !fetching.outstanding.is_empty() {
            return ProseOutcome::nothing();
        }
        let Some(Fetching {
            format, fetched, ..
        }) = self.fetching.take()
        else {
            return ProseOutcome::nothing();
        };
        self.start_or_queue(Queued::Export { format, fetched })
    }

    /// Start `next` now when no conversion runs, or after the one that does.
    pub(super) fn start_or_queue(&mut self, next: Queued) -> ProseOutcome {
        if self.conversion.is_some() {
            self.queued.push_back(next);
            return ProseOutcome::nothing();
        }
        self.start_queued(next)
    }

    /// Start a conversion whose media answered.
    pub(super) fn start_queued(&mut self, next: Queued) -> ProseOutcome {
        match next {
            Queued::Import { ir, report, now } => self.land_import(*ir, report, now),
            Queued::Export { format, fetched } => self.start_export_on(format, fetched),
        }
    }

    /// Decode the bytes of `address`, hold them, and tell the view. A blob
    /// that does not decode fails for good (`PROSE-FEAT-057`). A JPEG leaves
    /// for the converter thread, and the picture waits for its answer
    /// (`PROSE-FEAT-330`).
    fn picture_arrived(
        &mut self,
        address: &Validated<ContentHash>,
        data: Payload<MediaBytes>,
    ) -> ProseOutcome {
        match picture::decode(data) {
            Ok(picture::Decode::Done(decoded)) => self.hold_picture(address, Arc::new(decoded)),
            Ok(picture::Decode::Jpeg(request)) => {
                self.ask_decode(address, request);
                ProseOutcome::nothing()
            }
            Err(failure) => self.picture_failed(address, failure),
        }
    }

    /// Owe the converter thread the JPEG decode of `address`. The address
    /// stays in flight, so the cache never evicts it, and the view shows the
    /// placeholder (`PROSE-FEAT-055`).
    fn ask_decode(
        &mut self,
        address: &Validated<ContentHash>,
        request: ambre_convert::jpeg::JpegDecode,
    ) {
        if !self.pictures.is_loading(address) {
            self.pictures.forget(address);
            self.pictures.want(address);
        }
        let id = ambre_bus::next_request();
        self.decoding.insert(id, address.clone());
        self.decode_outbox.push((id, request));
    }

    /// The JPEG decodes this document owes the converter thread, drained. The
    /// Logic dispatcher sends each one and never waits for it.
    pub fn take_decode_requests(&mut self) -> Vec<(Seq<Request>, ambre_convert::jpeg::JpegDecode)> {
        std::mem::take(&mut self.decode_outbox)
    }

    /// The answer of a JPEG decode this document asked for, or `None` when the
    /// ticket is not one of its own. A module that did not load fails the
    /// picture as unavailable, and a reader may ask again. A file the module
    /// refused fails it as undecodable (`PROSE-FEAT-331`).
    pub fn handle_decoded(
        &mut self,
        id: Seq<Request>,
        decoded: Result<ambre_convert::images::Pixels, MediaFailure>,
    ) -> Option<ProseOutcome> {
        let address = self.decoding.remove(&id)?;
        Some(match decoded {
            Ok(pixels) => self.hold_picture(&address, Arc::new(picture::from_pixels(pixels))),
            Err(failure) => self.picture_failed(&address, failure),
        })
    }

    /// Hold `decoded` under `address`, then tell the view of it and of every
    /// address the cap evicted.
    fn hold_picture(
        &mut self,
        address: &Validated<ContentHash>,
        decoded: Arc<DecodedPicture>,
    ) -> ProseOutcome {
        let evicted = self.pictures.insert(address, decoded);
        let mut display = vec![self.picture_state(address)];
        display.extend(evicted.iter().map(|gone| self.picture_state(gone)));
        ProseOutcome::display_only(display)
    }

    fn picture_failed(
        &mut self,
        address: &Validated<ContentHash>,
        failure: MediaFailure,
    ) -> ProseOutcome {
        self.pictures.fail(address, failure);
        ProseOutcome::display_only(vec![self.picture_state(address)])
    }

    /// Write the block of a picture chosen from a file, now that its address
    /// came back, and show it from the decode already made.
    fn insert_uploaded(
        &mut self,
        pending: PendingInsert,
        address: &Validated<ContentHash>,
    ) -> ProseOutcome {
        let PendingInsert {
            parent_id,
            after_id,
            alt,
            size,
            shown,
        } = pending;
        let mut outcome = self.insert_image(parent_id, after_id, address, &alt, size);
        if outcome.update.is_some() {
            match shown {
                Prepared::Decoded(decoded) => outcome
                    .display
                    .extend(self.hold_picture(address, decoded).display),
                Prepared::Jpeg(request) => {
                    self.ask_decode(address, request);
                    outcome.display.push(self.picture_state(address));
                }
            }
        }
        outcome
    }

    /// The answer of an upload that stored nothing: the author is told, and
    /// nothing is written (`PROSE-FEAT-318`).
    fn upload_refused() -> ProseOutcome {
        ProseOutcome::display_only(vec![ProseEvent::Error(ProseError::new(
            RefusedAction::Command(ProseCommand::INSERT_IMAGE_NAME),
            ProseRefusal::PictureNotStored,
        ))])
    }

    /// The live picture blocks addressing one address. Sorted, so two peers
    /// name them in one order.
    pub(super) fn blocks_showing(&self, address: &Validated<ContentHash>) -> Vec<BlockId> {
        let tree = doc::blocks(&self.doc);
        let mut ids: Vec<TreeID> = tree
            .nodes()
            .into_iter()
            .filter(|id| {
                picture::read_image(&tree, id)
                    .and_then(|record| record.address)
                    .as_ref()
                    == Some(address)
            })
            .collect();
        ids.sort_unstable();
        ids.into_iter().map(block_id).collect()
    }

    /// What the view shows for `address` now. Read by a test.
    #[cfg(test)]
    pub(crate) fn shown(&self, address: &Validated<ContentHash>) -> crate::message::PictureShown {
        self.pictures.shown(address)
    }
}

/// A display side as the document stores it: a whole number of pixels.
fn side(px: ambre_core::Px<ambre_core::Doc>) -> LoroValue {
    // A side clamped under the display bound fits an `i64` exactly.
    #[expect(clippy::cast_possible_truncation, reason = "a clamped display side")]
    LoroValue::I64(px.round().get() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::BlockId;
    use crate::logic::tests::{arrive, hash_of, png};
    use crate::message::{PictureShown, ProseCommand};
    use ambre_core::{Px, Size};

    fn size(w: f32, h: f32) -> DisplaySize {
        Size::new(Px::new(w), Px::new(h))
    }

    /// A document holding one picture of `address`, after its first block.
    fn with_picture(peer: u64, address: &Validated<ContentHash>) -> (ProseState, BlockId) {
        let mut state =
            ProseState::with_peer(ambre_core::PeerId::new(peer)).expect("a document is built");
        let first = state.root_blocks()[0];
        let outcome = state.handle_command(ProseCommand::InsertImage {
            parent_id: None,
            after_id: Some(first),
            content_hash: address.clone(),
            alt: ambre_core::Validated::new("a red square"),
            size: size(200.0, 100.0),
        });
        assert!(
            outcome.update.is_some(),
            "the insert writes: {:?}",
            outcome.display
        );
        let image = state
            .root_blocks()
            .into_iter()
            .find(|id| state.block_type_of(id) == Some(BlockType::Image))
            .expect("a picture block");
        (state, image)
    }

    fn record(state: &ProseState, block: BlockId) -> ImageRecord {
        state
            .image_record(&crate::events::tree_id(block))
            .expect("a picture block")
    }

    fn refusal(outcome: &ProseOutcome) -> ProseRefusal {
        outcome
            .display
            .iter()
            .find_map(|event| match event {
                ProseEvent::Error(error) => Some(error.reason.clone()),
                _ => None,
            })
            .expect("a refusal")
    }

    /// `PROSE-FEAT-210`, `PROSE-FEAT-205`: an insert writes one picture block
    /// at the named place, holding the address and the size it carried. The
    /// document holds the address, never the pixels.
    #[test]
    fn inserting_writes_one_picture_block_with_its_address_and_size() {
        let before = ProseState::with_peer(ambre_core::PeerId::new(1))
            .expect("a document is built")
            .export_snapshot()
            .len();
        let (state, image) = with_picture(1, &hash_of('a'));
        let roots = state.root_blocks();
        assert_eq!(roots.len(), 2);
        assert_eq!(roots[1], image, "after the block it named");
        let held = record(&state, image);
        assert_eq!(held.address, Some(hash_of('a')));
        assert_eq!(held.size, size(200.0, 100.0));
        assert_eq!(held.alt.as_str(), "a red square");
        // Two hundred by a hundred pixels is 80 000 bytes of samples; the
        // block adds a few hundred bytes of address and attributes.
        let grown = state.export_snapshot().len().get() - before.get();
        assert!(grown < 2_000, "the document grew by {grown} bytes");
    }

    /// `PROSE-FEAT-052`, `PROSE-FEAT-211`: a picture under a cell or a list
    /// item is refused, and the refusal reaches the author.
    #[test]
    fn an_insertion_under_a_cell_or_a_list_item_is_refused_and_named() {
        let (mut state, table) =
            crate::logic::tables::tests::document_with_table(ambre_core::PeerId::new(1), 1, 1);
        let cell = crate::logic::tables::tests::cell_at(&state, table, 0, 0);
        let outcome = state.handle_command(ProseCommand::InsertImage {
            parent_id: Some(crate::events::block_id(cell)),
            after_id: None,
            content_hash: hash_of('a'),
            alt: ambre_core::Validated::default(),
            size: size(10.0, 10.0),
        });
        assert!(outcome.update.is_none());
        assert_eq!(refusal(&outcome), ProseRefusal::NestingRefused);
        let first = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertBlock {
            parent_id: None,
            after_id: Some(first),
            init: crate::events::BlockInit::ListItem {
                style: crate::message::ListStyle::Bullet,
            },
            style_id: None,
        });
        let item = state
            .root_blocks()
            .into_iter()
            .find(|id| state.block_type_of(id) == Some(BlockType::ListItem))
            .expect("a list item");
        let outcome = state.handle_command(ProseCommand::InsertImage {
            parent_id: Some(item),
            after_id: None,
            content_hash: hash_of('a'),
            alt: ambre_core::Validated::default(),
            size: size(10.0, 10.0),
        });
        assert_eq!(refusal(&outcome), ProseRefusal::NestingRefused);
        assert!(
            !state
                .root_blocks()
                .into_iter()
                .any(|id| state.block_type_of(&id) == Some(BlockType::Image))
        );
    }

    /// `PROSE-FEAT-212`: inserting a picture changes no text, no order of
    /// the blocks around it, and nothing the cache holds.
    #[test]
    fn inserting_a_picture_changes_no_text_and_no_order() {
        let mut state = ProseState::with_peer(ambre_core::PeerId::new(1)).expect("a document");
        let first = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertText {
            block_id: first,
            pos: ambre_core::index::Pos::<crate::message::DocChar>::new(0),
            text: "one".to_owned(),
        });
        state.handle_command(ProseCommand::SplitBlock {
            block_id: first,
            pos: ambre_core::index::Pos::<crate::message::DocChar>::new(3),
        });
        let before: Vec<_> = state.root_blocks();
        let texts: Vec<_> = before.iter().map(|id| state.text_of(id)).collect();
        let held = state.pictures.held();
        state.handle_command(ProseCommand::InsertImage {
            parent_id: None,
            after_id: Some(first),
            content_hash: hash_of('b'),
            alt: ambre_core::Validated::default(),
            size: size(10.0, 10.0),
        });
        let after: Vec<_> = state
            .root_blocks()
            .into_iter()
            .filter(|id| state.block_type_of(id) != Some(BlockType::Image))
            .collect();
        assert_eq!(after, before);
        let texts_after: Vec<_> = after.iter().map(|id| state.text_of(id)).collect();
        assert_eq!(texts_after, texts);
        assert_eq!(state.pictures.held(), held);
    }

    /// `PROSE-FEAT-058`, `PROSE-FEAT-059`, `PROSE-FEAT-213`: a resize to zero
    /// is refused, and a huge one stores the clamped value.
    #[test]
    fn a_resize_to_zero_is_refused_and_a_huge_one_stored_clamped() {
        let (mut state, image) = with_picture(1, &hash_of('a'));
        let outcome = state.handle_command(ProseCommand::ResizeImage {
            block_id: image,
            size: size(0.0, 50.0),
            lock_aspect: AspectLock::Free,
        });
        assert_eq!(refusal(&outcome), ProseRefusal::PictureWithoutArea);
        assert_eq!(record(&state, image).size, size(200.0, 100.0));
        state.handle_command(ProseCommand::ResizeImage {
            block_id: image,
            size: size(90_000.0, 50.0),
            lock_aspect: AspectLock::Free,
        });
        assert_eq!(record(&state, image).size, size(4096.0, 50.0));
    }

    /// `PROSE-FEAT-214`, `PROSE-FEAT-215`, `PROSE-FEAT-216`: with the aspect
    /// locked, the side that moved most drives and the other follows the
    /// decoded ratio. Nothing but the size moves.
    #[test]
    fn a_locked_resize_takes_the_decoded_ratio_and_touches_nothing_else() {
        let (mut state, image) = with_picture(1, &hash_of('a'));
        state.handle_command(ProseCommand::SetImageCaption {
            block_id: image,
            caption: ambre_core::Validated::new("the caption"),
        });
        state.handle_command(ProseCommand::SetImageAlign {
            block_id: image,
            align: ImageAlign::Center,
        });
        // The file is four times as wide as tall; the stored size is two.
        arrive(&mut state, &hash_of('a'), png(40, 10));
        state.handle_command(ProseCommand::ResizeImage {
            block_id: image,
            size: size(400.0, 110.0),
            lock_aspect: AspectLock::Locked,
        });
        let held = record(&state, image);
        assert_eq!(held.size, size(400.0, 100.0));
        assert_eq!(held.address, Some(hash_of('a')));
        assert_eq!(held.alt.as_str(), "a red square");
        assert_eq!(held.caption.as_str(), "the caption");
        assert_eq!(held.align, ImageAlign::Center);
    }

    /// `PROSE-FEAT-217`, `PROSE-FEAT-218`: aligning stores the alignment
    /// named, the two the layout does not honour included, and nothing else.
    #[test]
    fn aligning_stores_the_alignment_and_nothing_else() {
        let (mut state, image) = with_picture(1, &hash_of('a'));
        for align in [
            ImageAlign::Left,
            ImageAlign::Right,
            ImageAlign::Center,
            ImageAlign::Block,
        ] {
            state.handle_command(ProseCommand::SetImageAlign {
                block_id: image,
                align,
            });
            let held = record(&state, image);
            assert_eq!(held.align, align);
            assert_eq!(held.size, size(200.0, 100.0));
            assert_eq!(held.address, Some(hash_of('a')));
        }
        assert_eq!(state.root_blocks().len(), 2);
    }

    /// `PROSE-FEAT-219`, `PROSE-FEAT-220`: a text lands on the field it
    /// names, on that block alone, and moves no other field.
    #[test]
    fn a_text_lands_on_its_field_alone() {
        let (mut state, image) = with_picture(1, &hash_of('a'));
        let first = state.root_blocks()[0];
        let other = state.handle_command(ProseCommand::InsertImage {
            parent_id: None,
            after_id: Some(first),
            content_hash: hash_of('b'),
            alt: ambre_core::Validated::new("other"),
            size: size(5.0, 5.0),
        });
        assert!(other.update.is_some());
        state.handle_command(ProseCommand::SetImageAlt {
            block_id: image,
            alt: ambre_core::Validated::new("a blue square"),
        });
        state.handle_command(ProseCommand::SetImageCaption {
            block_id: image,
            caption: ambre_core::Validated::new("Figure 1"),
        });
        let held = record(&state, image);
        assert_eq!(
            (held.alt.as_str(), held.caption.as_str()),
            ("a blue square", "Figure 1")
        );
        assert_eq!(held.size, size(200.0, 100.0));
        assert_eq!(held.align, ImageAlign::Block);
        assert_eq!(held.address, Some(hash_of('a')));
        let untouched = state
            .root_blocks()
            .into_iter()
            .filter(|id| *id != image)
            .find_map(|id| state.image_record(&crate::events::tree_id(id)))
            .expect("the other picture");
        assert_eq!(
            (untouched.alt.as_str(), untouched.caption.as_str()),
            ("other", "")
        );
        let refused = state.handle_command(ProseCommand::SetImageAlt {
            block_id: first,
            alt: ambre_core::Validated::new("x"),
        });
        assert_eq!(refusal(&refused), ProseRefusal::NotAPicture(first));
    }

    /// The download the last drain asked for `address`, if any.
    fn download_of(state: &mut ProseState, address: &Validated<ContentHash>) -> Vec<Seq<Request>> {
        state
            .take_media_requests()
            .into_iter()
            .filter_map(|request| match request {
                MediaRequest::Download { id, content_hash } if content_hash == *address => Some(id),
                _ => None,
            })
            .collect()
    }

    /// `PROSE-FEAT-320`: a layout that carries a picture asks for it once,
    /// and a second layout asks nothing.
    #[test]
    fn a_layout_asks_once_for_each_picture_it_carries() {
        let (mut state, _) = with_picture(1, &hash_of('a'));
        let before = state.export_snapshot();
        state.layout_event();
        assert_eq!(download_of(&mut state, &hash_of('a')).len(), 1);
        assert_eq!(state.export_snapshot(), before, "asking writes nothing");
        state.handle_command(ProseCommand::InsertText {
            block_id: state.root_blocks()[0],
            pos: ambre_core::index::Pos::<crate::message::DocChar>::new(0),
            text: "x".to_owned(),
        });
        state.layout_event();
        assert!(download_of(&mut state, &hash_of('a')).is_empty());
    }

    /// `PROSE-FEAT-057`, `PROSE-FEAT-207`: a download that fails shows the
    /// failure, writes nothing, and a later layout asks nothing again.
    #[test]
    fn a_failed_download_writes_nothing_and_is_not_asked_again() {
        let (mut state, image) = with_picture(1, &hash_of('a'));
        state.layout_event();
        let [id] = download_of(&mut state, &hash_of('a'))[..] else {
            panic!("one download");
        };
        let before = state.export_snapshot();
        let outcome = state
            .handle_media_error(id)
            .expect("the request is this document's");
        assert!(outcome.update.is_none());
        assert_eq!(state.export_snapshot(), before);
        assert!(outcome.display.iter().any(|event| matches!(
            event,
            ProseEvent::PictureState { shown: PictureShown::Failed(MediaFailure::Unavailable), blocks, .. }
                if blocks == &[image]
        )));
        state.handle_command(ProseCommand::InsertText {
            block_id: state.root_blocks()[0],
            pos: ambre_core::index::Pos::<crate::message::DocChar>::new(0),
            text: "x".to_owned(),
        });
        state.layout_event();
        assert!(download_of(&mut state, &hash_of('a')).is_empty());
        // Bytes that do not decode fail for good as well.
        let outcome = arrive(&mut state, &hash_of('c'), b"not a picture".to_vec());
        assert!(outcome.display.iter().any(|event| matches!(
            event,
            ProseEvent::PictureState {
                shown: PictureShown::Failed(MediaFailure::Undecodable),
                ..
            }
        )));
    }

    /// `PROSE-FEAT-221`, `PROSE-FEAT-222`: asking again puts the address
    /// back to waiting and downloads it once more. The document is not
    /// written, and another address keeps its state.
    #[test]
    fn asking_again_puts_the_picture_back_and_downloads_it_once_more() {
        let (mut state, image) = with_picture(1, &hash_of('a'));
        arrive(&mut state, &hash_of('b'), png(2, 2));
        state.layout_event();
        let [id] = download_of(&mut state, &hash_of('a'))[..] else {
            panic!("one download");
        };
        state.handle_media_error(id);
        let before = state.export_snapshot();
        let outcome = state.handle_command(ProseCommand::RetryImageLoad { block_id: image });
        assert!(outcome.update.is_none());
        assert_eq!(state.export_snapshot(), before);
        assert_eq!(state.shown(&hash_of('a')), PictureShown::Waiting);
        assert_eq!(download_of(&mut state, &hash_of('a')).len(), 1);
        assert!(matches!(state.shown(&hash_of('b')), PictureShown::Ready(_)));
    }

    /// A JPEG of `width` by `height`, written by the encoder of the `image`
    /// crate at test time.
    fn jpeg(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut bytes)
            .encode_image(&image::RgbImage::from_pixel(
                width,
                height,
                image::Rgb([200, 10, 10]),
            ))
            .expect("a JPEG encodes");
        bytes
    }

    /// Run each decode `state` owes on the images engine the host links, as
    /// the converter thread does, and hand back every answer.
    fn serve_decodes(state: &mut ProseState) -> Vec<ProseOutcome> {
        let converter = ambre_convert::Converter::load(
            ambre_core::ConverterFamily::Images,
            &ambre_core::Payload::new(Vec::new()),
        )
        .expect("the host links the images engine");
        state
            .take_decode_requests()
            .into_iter()
            .filter_map(|(id, request)| {
                let answer =
                    match converter.convert(&ambre_convert::Conversion::DecodeJpeg(request)) {
                        Ok(ambre_convert::Converted::Pixels(pixels)) => Ok(pixels),
                        _ => Err(MediaFailure::Undecodable),
                    };
                state.handle_decoded(id, answer)
            })
            .collect()
    }

    /// `PROSE-FEAT-330`: a JPEG that arrives is no decode of the Logic
    /// thread. The picture waits, one decode leaves for the converter thread
    /// at the cache edge and upright, and its answer makes the picture ready.
    #[test]
    fn a_jpeg_that_arrives_waits_for_the_converter_thread_then_shows() {
        let (mut state, _) = with_picture(1, &hash_of('a'));
        let outcome = arrive(&mut state, &hash_of('a'), jpeg(4096, 1024));
        assert!(outcome.display.is_empty(), "nothing is shown yet");
        assert_eq!(state.shown(&hash_of('a')), PictureShown::Waiting);
        let requests = state.take_decode_requests();
        let [(id, request)] = &requests[..] else {
            panic!("one decode leaves: {}", requests.len());
        };
        assert_eq!(request.size, Size::new(Px::new(2048), Px::new(512)));
        assert_eq!(request.turn, ambre_convert::jpeg::JpegTurn::Upright);
        assert_eq!(
            request.bound,
            Size::new(picture::MAX_SOURCE_EDGE, picture::MAX_SOURCE_EDGE)
        );
        state.decode_outbox.push((*id, request.clone()));
        let answers = serve_decodes(&mut state);
        assert_eq!(answers.len(), 1);
        let PictureShown::Ready(decoded) = state.shown(&hash_of('a')) else {
            panic!("the picture is ready once decoded");
        };
        assert_eq!(decoded.size, Size::new(Px::new(2048), Px::new(512)));
        // A header past the source bound leaves no decode at all.
        let mut wide = jpeg(8, 8);
        let sof = wide
            .windows(2)
            .position(|w| w == [0xFF, 0xC0])
            .expect("a baseline frame");
        wide[sof + 7..sof + 9].copy_from_slice(&8193u16.to_be_bytes());
        arrive(&mut state, &hash_of('b'), wide);
        assert!(state.take_decode_requests().is_empty());
        assert_eq!(
            state.shown(&hash_of('b')),
            PictureShown::Failed(MediaFailure::Undecodable)
        );
    }

    /// `PROSE-FEAT-331`: offline on a first use, the images module does not
    /// load. The picture fails as unavailable, and asking again downloads it
    /// and leaves one more decode, which the module now serves.
    #[test]
    fn a_jpeg_whose_module_did_not_load_fails_and_asking_again_decodes_it() {
        let (mut state, image) = with_picture(1, &hash_of('a'));
        arrive(&mut state, &hash_of('a'), jpeg(16, 8));
        let [(id, _)] = state.take_decode_requests()[..] else {
            panic!("one decode leaves");
        };
        let outcome = state
            .handle_decoded(id, Err(MediaFailure::Unavailable))
            .expect("the ticket is the document's");
        assert!(outcome.update.is_none());
        assert_eq!(
            state.shown(&hash_of('a')),
            PictureShown::Failed(MediaFailure::Unavailable)
        );
        state.handle_command(ProseCommand::RetryImageLoad { block_id: image });
        assert_eq!(state.shown(&hash_of('a')), PictureShown::Waiting);
        let [id] = download_of(&mut state, &hash_of('a'))[..] else {
            panic!("one download");
        };
        state.handle_media_event(
            id,
            ambre_bus::RestEvent::MediaDownloaded {
                document_id: ambre_core::id::Id::from_bytes([1; 16]),
                content_hash: hash_of('a'),
                data: ambre_core::Payload::new(jpeg(16, 8)),
            },
        );
        assert_eq!(serve_decodes(&mut state).len(), 1);
        assert!(matches!(state.shown(&hash_of('a')), PictureShown::Ready(_)));
    }

    /// `PROSE-FEAT-330`: a JPEG chosen from a file is checked and stored at
    /// once, and its decode leaves once the address comes back. The block
    /// shows the placeholder until the answer.
    #[test]
    fn a_jpeg_from_a_file_is_written_then_decoded_off_the_logic_thread() {
        let mut state =
            ProseState::with_peer(ambre_core::PeerId::new(1)).expect("a document is built");
        let outcome = state.handle_command(ProseCommand::InsertImageFile {
            parent_id: None,
            after_id: None,
            data: ambre_core::Payload::new(jpeg(32, 16)),
            alt: ambre_core::Validated::default(),
        });
        assert!(outcome.update.is_none());
        assert!(
            state.take_decode_requests().is_empty(),
            "no decode before the address"
        );
        let id = state
            .take_media_requests()
            .into_iter()
            .find_map(|request| match request {
                crate::message::MediaRequest::Upload { id, .. } => Some(id),
                crate::message::MediaRequest::Download { .. } => None,
            })
            .expect("the file is stored");
        let outcome = state
            .handle_media_event(
                id,
                ambre_bus::RestEvent::MediaUploaded {
                    document_id: ambre_core::id::Id::from_bytes([1; 16]),
                    content_hash: hash_of('c'),
                },
            )
            .expect("the upload is the document's");
        assert!(outcome.update.is_some(), "the block is written");
        assert_eq!(state.shown(&hash_of('c')), PictureShown::Waiting);
        assert_eq!(serve_decodes(&mut state).len(), 1);
        let PictureShown::Ready(decoded) = state.shown(&hash_of('c')) else {
            panic!("the picture is ready once decoded");
        };
        assert_eq!(decoded.size, Size::new(Px::new(32), Px::new(16)));
    }

    /// `PROSE-FEAT-056`: an eviction frees the samples and never the
    /// address: the block keeps it, and the document is not written.
    #[test]
    fn an_eviction_writes_nothing_to_the_document() {
        let (mut state, image) = with_picture(1, &hash_of('a'));
        // 2048 by 2048 pixels is 16 MiB of samples: five of them pass the cap.
        let large = png(2048, 2048);
        arrive(&mut state, &hash_of('a'), large.clone());
        let before = state.export_snapshot();
        let mut evicted = false;
        for digit in ['b', 'c', 'd', 'e'] {
            let outcome = arrive(&mut state, &hash_of(digit), large.clone());
            evicted |= outcome.display.iter().any(|event| {
                matches!(
                    event,
                    ProseEvent::PictureState { content_hash, shown: PictureShown::Waiting, .. }
                        if *content_hash == hash_of('a')
                )
            });
            assert!(outcome.update.is_none());
        }
        assert!(evicted, "the oldest picture left the cache");
        assert_eq!(state.export_snapshot(), before);
        assert_eq!(record(&state, image).address, Some(hash_of('a')));
    }

    /// `PROSE-FEAT-208`: two peers resizing one picture at once converge on
    /// one size, both sides of it from one peer.
    #[test]
    fn two_peers_resizing_one_picture_converge() {
        let (mut alice, image) = with_picture(1, &hash_of('a'));
        let mut bob = ProseState::with_peer(ambre_core::PeerId::new(2)).expect("a document");
        bob.import_snapshot(&alice.export_snapshot())
            .expect("the seed imports");
        // Bob writes more before, so his clock runs ahead of Alice's.
        let earlier = bob
            .handle_command(ProseCommand::SetImageAlt {
                block_id: image,
                alt: ambre_core::Validated::new("x"),
            })
            .update
            .expect("a delta");
        let from_alice = alice
            .handle_command(ProseCommand::ResizeImage {
                block_id: image,
                size: size(300.0, 30.0),
                lock_aspect: AspectLock::Free,
            })
            .update
            .expect("a delta");
        let from_bob = bob
            .handle_command(ProseCommand::ResizeImage {
                block_id: image,
                size: size(50.0, 500.0),
                lock_aspect: AspectLock::Free,
            })
            .update
            .expect("a delta");
        bob.apply_remote(&from_alice);
        alice.apply_remote(&earlier);
        alice.apply_remote(&from_bob);
        let (a, b) = (record(&alice, image).size, record(&bob, image).size);
        assert_eq!(a, b);
        assert!(a == size(300.0, 30.0) || a == size(50.0, 500.0), "{a:?}");
    }

    /// `PROSE-FEAT-209`: every alignment comes back from a round trip of the
    /// document's bytes as it was written, the two the layout does not
    /// honour included.
    #[test]
    fn every_alignment_survives_a_round_trip() {
        for align in [
            ImageAlign::Block,
            ImageAlign::Left,
            ImageAlign::Right,
            ImageAlign::Center,
        ] {
            let (mut state, image) = with_picture(1, &hash_of('a'));
            state.handle_command(ProseCommand::SetImageAlign {
                block_id: image,
                align,
            });
            let mut reader = ProseState::with_peer(ambre_core::PeerId::new(2)).expect("a document");
            reader
                .import_snapshot(&state.export_snapshot())
                .expect("it imports");
            assert_eq!(record(&reader, image).align, align);
        }
    }

    /// The upload the last drain asked for, with its bytes.
    fn uploads(state: &mut ProseState) -> Vec<(Seq<Request>, Payload<MediaBytes>)> {
        state
            .take_media_requests()
            .into_iter()
            .filter_map(|request| match request {
                MediaRequest::Upload { id, data } => Some((id, data)),
                MediaRequest::Download { .. } => None,
            })
            .collect()
    }

    fn uploaded(address: &Validated<ContentHash>) -> RestEvent {
        RestEvent::MediaUploaded {
            document_id: ambre_core::id::Id::from_bytes([1; 16]),
            content_hash: address.clone(),
        }
    }

    /// `PROSE-FEAT-316`, `PROSE-FEAT-324`: a picture from a file is stored
    /// first. Its block is written once the address comes back, and it shows
    /// at once from the decode already made.
    #[test]
    fn a_picture_from_a_file_is_written_once_its_address_comes_back() {
        let mut state = ProseState::with_peer(ambre_core::PeerId::new(1)).expect("a document");
        let first = state.root_blocks()[0];
        let outcome = state.handle_command(ProseCommand::InsertImageFile {
            parent_id: None,
            after_id: Some(first),
            data: Payload::new(png(30, 20)),
            alt: ambre_core::Validated::new("chosen"),
        });
        assert!(
            outcome.update.is_none(),
            "nothing is written before the address"
        );
        let [(id, data)] = &uploads(&mut state)[..] else {
            panic!("one upload");
        };
        assert_eq!(data.as_bytes(), png(30, 20).as_slice());
        assert_eq!(state.root_blocks().len(), 1);
        let outcome = state
            .handle_media_event(*id, uploaded(&hash_of('f')))
            .expect("the answer is this document's");
        assert!(outcome.update.is_some());
        let image = state.root_blocks()[1];
        let held = record(&state, image);
        assert_eq!(held.address, Some(hash_of('f')));
        assert_eq!(held.size, size(30.0, 20.0));
        assert_eq!(held.alt.as_str(), "chosen");
        assert!(matches!(state.shown(&hash_of('f')), PictureShown::Ready(_)));
    }

    /// `PROSE-FEAT-314`, `PROSE-FEAT-318`: a file this build cannot read is
    /// refused before any byte leaves. One the media flow did not store is
    /// named to the author and writes nothing.
    #[test]
    fn a_file_the_media_flow_did_not_store_writes_nothing_and_is_named() {
        let mut state = ProseState::with_peer(ambre_core::PeerId::new(1)).expect("a document");
        let refused = state.handle_command(ProseCommand::InsertImageFile {
            parent_id: None,
            after_id: None,
            data: Payload::new(b"not a picture".to_vec()),
            alt: ambre_core::Validated::default(),
        });
        assert_eq!(refusal(&refused), ProseRefusal::PictureUnreadable);
        assert!(uploads(&mut state).is_empty());
        state.handle_command(ProseCommand::InsertImageFile {
            parent_id: None,
            after_id: None,
            data: Payload::new(png(3, 3)),
            alt: ambre_core::Validated::default(),
        });
        let [(id, _)] = &uploads(&mut state)[..] else {
            panic!("one upload");
        };
        let before = state.export_snapshot();
        let outcome = state
            .handle_media_error(*id)
            .expect("the answer is this document's");
        assert_eq!(refusal(&outcome), ProseRefusal::PictureNotStored);
        assert!(outcome.update.is_none());
        assert_eq!(state.export_snapshot(), before);
    }

    /// `PROSE-IO-106`: the export holds its picture bytes under its cap. A
    /// picture that would pass it is let go, and a smaller one after it is
    /// still held.
    #[test]
    fn a_picture_past_the_export_cap_is_let_go() {
        let half = EXPORT_BYTES_CAP.get() / 2;
        let held = ambre_core::Count::<ambre_core::Octets>::new(half + 1);
        assert!(admits(ambre_core::Count::ZERO, held));
        assert!(!admits(held, held));
        assert!(admits(held, ambre_core::Count::new(half - 1)));
    }

    /// `PROSE-FEAT-326`: a picture chosen from a file leaves for the Sync
    /// worker without the location, the camera and the comment it held.
    #[test]
    fn a_picture_from_a_file_is_stored_without_its_metadata() {
        use crate::picture::tests::{contains, jpeg_with_metadata};
        let mut state = ProseState::with_peer(ambre_core::PeerId::new(1)).expect("a document");
        state.handle_command(ProseCommand::InsertImageFile {
            parent_id: None,
            after_id: None,
            data: Payload::new(jpeg_with_metadata()),
            alt: ambre_core::Validated::default(),
        });
        let [(_, data)] = &uploads(&mut state)[..] else {
            panic!("one upload");
        };
        for left_out in [b"ACME".as_slice(), b"secret place", b"a secret comment"] {
            assert!(!contains(data.as_bytes(), left_out), "{left_out:?} left");
        }
    }
}
