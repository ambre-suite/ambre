//! The attachments of a message
//! (`specs/5-Murmure/features/ambre-spec-murmure-attachments.md`).
//!
//! A message holds each file by its address, its name, its media type and its
//! size, never its bytes (`MURMURE-017`). The bytes travel sealed through the
//! shared media flow, uploaded before the message is sent (`MURMURE-132`).
//! The list is written with the message and never changes after, so two
//! peers never race on it.

use ambre_bus::{Attachment, ByteSize, ContentHash, MediaType};
use ambre_core::Validated;
use loro::{Container, LoroList, LoroMap, ValueOrContainer};

use crate::error::MurmureError;
use crate::schema::{self, AttachmentKey, Fields, MessageKey};

/// Whether each attachment names a file: a name that is not blank.
pub(crate) fn fits(attachments: &[Attachment]) -> bool {
    attachments
        .iter()
        .all(|attachment| !attachment.name.trim().is_empty())
}

/// Write `attachments` under `message`. A message with none holds no list,
/// as absence reads as none (`MURMURE-018`).
///
/// # Errors
/// [`MurmureError::Crdt`] when Loro refuses a write.
pub(crate) fn write(
    message: &Fields<MessageKey>,
    attachments: &[Attachment],
) -> Result<(), MurmureError> {
    if attachments.is_empty() {
        return Ok(());
    }
    let list = message.insert_container(MessageKey::Attachments, LoroList::new())?;
    for attachment in attachments {
        let entry = Fields::<AttachmentKey>::of(list.push_container(LoroMap::new())?);
        entry.insert(AttachmentKey::Media, attachment.media_ref.as_str())?;
        entry.insert(AttachmentKey::Name, attachment.name.as_str())?;
        entry.insert(AttachmentKey::Type, attachment.media_type.as_str())?;
        // Loro has no unsigned integer; a file past `i64::MAX` bytes is not
        // one the relay accepts, so saturating loses nothing real.
        let size = i64::try_from(attachment.size.get()).unwrap_or(i64::MAX);
        entry.insert(AttachmentKey::Size, size)?;
    }
    Ok(())
}

/// The attachments of one message, in the order they were written. An entry
/// this build cannot read is skipped.
pub(crate) fn read(message: &Fields<MessageKey>) -> Vec<Attachment> {
    let Some(ValueOrContainer::Container(Container::List(list))) =
        message.get(MessageKey::Attachments)
    else {
        return Vec::new();
    };
    let mut attachments = Vec::with_capacity(list.len());
    list.for_each(|item| {
        if let Some(entry) = schema::fields_of(item)
            && let Some(attachment) = read_entry(&entry)
        {
            attachments.push(attachment);
        }
    });
    attachments
}

/// One attachment, `None` when a field is absent or unreadable.
fn read_entry(entry: &Fields<AttachmentKey>) -> Option<Attachment> {
    Some(Attachment {
        media_ref: Validated::<ContentHash>::parse(&schema::string(entry, AttachmentKey::Media)?)
            .ok()?,
        name: schema::string(entry, AttachmentKey::Name)?,
        media_type: MediaType::new(schema::string(entry, AttachmentKey::Type)?),
        size: ByteSize::new(schema::unsigned(entry, AttachmentKey::Size)?),
    })
}
