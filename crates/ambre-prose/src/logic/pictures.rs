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
use ambre_core::{ContentHash, MediaBytes, Payload, Seq, Validated};
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
    alt: String,
    size: DisplaySize,
    /// Decoded once on the way in, so the author sees the picture with no
    /// download.
    decoded: Arc<DecodedPicture>,
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

/// An import whose pictures are being stored. Nothing is written until
/// every upload answered, so the file lands whole (`PROSE-IO-011`).
pub(super) struct Uploading {
    ir: Box<crate::interop::ProseIr>,
    report: crate::message::ConversionReport,
    now: ambre_core::Instant<ambre_core::Authored>,
    /// The distinct picture each upload stores, by its request.
    outstanding: HashMap<Seq<Request>, usize>,
    /// The address each distinct picture got, by its place among them.
    /// `None` until its upload answers, and for good when it failed.
    addresses: Vec<Option<Validated<ContentHash>>>,
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

/// Which field of a picture a text command writes.
#[derive(Clone, Copy)]
enum PictureText {
    Alt,
    Caption,
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
        alt: &str,
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
        alt: &str,
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
            .and_then(|()| attrs.insert(doc::AttrKey::ImageAlt, alt))
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
        (alt, caption): (&str, &str),
        size: DisplaySize,
        align: ImageAlign,
    ) -> Result<TreeID, ProseOutcome> {
        let id = self.write_image(parent_id, after_id, address, alt, size)?;
        let tree = doc::blocks(&self.doc);
        let attrs = doc::node_of(&tree, &id).and_then(|node| doc::attrs_at(&node));
        let written = attrs.map(|attrs| {
            attrs
                .insert(doc::AttrKey::ImageCaption, caption)
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
        alt: String,
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
        let Ok(decoded) = picture::decode(&stored) else {
            return ProseOutcome::refused(ProseRefusal::PictureUnreadable);
        };
        let id = ambre_bus::next_request();
        self.in_flight.insert(
            id,
            InFlight::Insert(Box::new(PendingInsert {
                parent_id,
                after_id,
                alt,
                size: picture::display_of_native(native),
                decoded: Arc::new(decoded),
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
    fn set_image_text(&mut self, id: &TreeID, field: PictureText, text: &str) -> ProseOutcome {
        if self.image_record(id).is_none() {
            return ProseOutcome::refused(ProseRefusal::NotAPicture(block_id(*id)));
        }
        let key = match field {
            PictureText::Alt => doc::AttrKey::ImageAlt,
            PictureText::Caption => doc::AttrKey::ImageCaption,
        };
        self.write_attrs(id, &[(key, LoroValue::from(text))])
    }

    pub(super) fn set_image_alt(&mut self, id: &TreeID, alt: &str) -> ProseOutcome {
        self.set_image_text(id, PictureText::Alt, alt)
    }

    pub(super) fn set_image_caption(&mut self, id: &TreeID, caption: &str) -> ProseOutcome {
        self.set_image_text(id, PictureText::Caption, caption)
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
                self.picture_arrived(&address, &data)
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
        for image in images_of(&mut ir) {
            // A picture is stored without its metadata, as one chosen from a
            // file is (`PROSE-FEAT-326`). One this client cannot strip is
            // neither a PNG nor a JPEG: nothing here could show it, so it is
            // left out and named.
            image.bytes = image.bytes.take().and_then(|bytes| {
                let data = picture::stripped(&bytes.data.into_role()).ok()?;
                Some(ambre_prose_form::IrImageBytes {
                    format: bytes.format,
                    data: data.into_role(),
                })
            });
            if let Some(bytes) = &image.bytes
                && !distinct
                    .iter()
                    .any(|seen| seen.as_bytes() == bytes.data.as_bytes())
            {
                distinct.push(bytes.data.clone().into_role());
            }
        }
        if distinct.is_empty() {
            return self.land_import(ir, report, now);
        }
        let mut outstanding = HashMap::new();
        for (slot, data) in distinct.into_iter().enumerate() {
            let id = ambre_bus::next_request();
            outstanding.insert(id, slot);
            self.in_flight.insert(id, InFlight::Import);
            self.media_outbox.push(MediaRequest::Upload { id, data });
        }
        let slots = outstanding.len();
        self.uploading = Some(Uploading {
            ir: Box::new(ir),
            report,
            now,
            outstanding,
            addresses: vec![None; slots],
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
            addresses,
            ..
        }) = self.uploading.take()
        else {
            return ProseOutcome::nothing();
        };
        let mut distinct: Vec<Vec<u8>> = Vec::new();
        for image in images_of(&mut ir) {
            let Some(bytes) = image.bytes.take() else {
                continue;
            };
            let slot = match distinct
                .iter()
                .position(|seen| seen == bytes.data.as_bytes())
            {
                Some(slot) => slot,
                None => {
                    distinct.push(bytes.data.as_bytes().to_vec());
                    distinct.len() - 1
                }
            };
            image.address = addresses.get(slot).cloned().flatten();
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
    /// that does not decode fails for good (`PROSE-FEAT-057`).
    fn picture_arrived(
        &mut self,
        address: &Validated<ContentHash>,
        data: &Payload<MediaBytes>,
    ) -> ProseOutcome {
        match picture::decode(data) {
            Ok(decoded) => self.hold_picture(address, Arc::new(decoded)),
            Err(failure) => self.picture_failed(address, failure),
        }
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
            decoded,
        } = pending;
        let mut outcome = self.insert_image(parent_id, after_id, address, &alt, size);
        if outcome.update.is_some() {
            outcome
                .display
                .extend(self.hold_picture(address, decoded).display);
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
            alt: "a red square".to_owned(),
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
        assert_eq!(held.alt, "a red square");
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
            alt: String::new(),
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
            alt: String::new(),
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
            alt: String::new(),
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
            caption: "the caption".to_owned(),
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
        assert_eq!(held.alt, "a red square");
        assert_eq!(held.caption, "the caption");
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
            alt: "other".to_owned(),
            size: size(5.0, 5.0),
        });
        assert!(other.update.is_some());
        state.handle_command(ProseCommand::SetImageAlt {
            block_id: image,
            alt: "a blue square".to_owned(),
        });
        state.handle_command(ProseCommand::SetImageCaption {
            block_id: image,
            caption: "Figure 1".to_owned(),
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
            alt: "x".to_owned(),
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
                alt: "x".to_owned(),
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
            alt: "chosen".to_owned(),
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
        assert_eq!(held.alt, "chosen");
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
            alt: String::new(),
        });
        assert_eq!(refusal(&refused), ProseRefusal::PictureUnreadable);
        assert!(uploads(&mut state).is_empty());
        state.handle_command(ProseCommand::InsertImageFile {
            parent_id: None,
            after_id: None,
            data: Payload::new(png(3, 3)),
            alt: String::new(),
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
            alt: String::new(),
        });
        let [(_, data)] = &uploads(&mut state)[..] else {
            panic!("one upload");
        };
        for left_out in [b"ACME".as_slice(), b"secret place", b"a secret comment"] {
            assert!(!contains(data.as_bytes(), left_out), "{left_out:?} left");
        }
    }
}
