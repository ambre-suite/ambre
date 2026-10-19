//! The attachments of a message
//! (`specs/5-Murmure/features/ambre-spec-murmure-attachments.md`).
//!
//! A message holds each file by its address, its name, its media type and its
//! size, never its bytes (`MURMURE-017`). The bytes travel sealed through the
//! shared media flow, uploaded before the message is sent (`MURMURE-132`).
//! The list is written with the message and never changes after, so two
//! peers never race on it.

use ambre_bus::{Attachment, ByteSize, MediaRef, MediaType};
use loro::{Container, LoroList, LoroMap, ValueOrContainer};

use crate::error::MurmureError;
use crate::schema::{
    self, ATTACHMENT_MEDIA, ATTACHMENT_NAME, ATTACHMENT_SIZE, ATTACHMENT_TYPE, MESSAGE_ATTACHMENTS,
};

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
pub(crate) fn write(message: &LoroMap, attachments: &[Attachment]) -> Result<(), MurmureError> {
    if attachments.is_empty() {
        return Ok(());
    }
    let list = message.insert_container(MESSAGE_ATTACHMENTS, LoroList::new())?;
    for attachment in attachments {
        let entry = list.push_container(LoroMap::new())?;
        entry.insert(ATTACHMENT_MEDIA, attachment.media_ref.as_str())?;
        entry.insert(ATTACHMENT_NAME, attachment.name.as_str())?;
        entry.insert(ATTACHMENT_TYPE, attachment.media_type.0.as_str())?;
        // Loro has no unsigned integer; a file past `i64::MAX` bytes is not
        // one the relay accepts, so saturating loses nothing real.
        let size = i64::try_from(attachment.size.0).unwrap_or(i64::MAX);
        entry.insert(ATTACHMENT_SIZE, size)?;
    }
    Ok(())
}

/// The attachments of one message, in the order they were written. An entry
/// this build cannot read is skipped.
pub(crate) fn read(message: &LoroMap) -> Vec<Attachment> {
    let Some(ValueOrContainer::Container(Container::List(list))) = message.get(MESSAGE_ATTACHMENTS)
    else {
        return Vec::new();
    };
    let mut attachments = Vec::with_capacity(list.len());
    list.for_each(|item| {
        if let ValueOrContainer::Container(Container::Map(entry)) = item
            && let Some(attachment) = read_entry(&entry)
        {
            attachments.push(attachment);
        }
    });
    attachments
}

/// One attachment, `None` when a field is absent or unreadable.
fn read_entry(entry: &LoroMap) -> Option<Attachment> {
    Some(Attachment {
        media_ref: MediaRef::parse(&schema::string(entry, ATTACHMENT_MEDIA)?)?,
        name: schema::string(entry, ATTACHMENT_NAME)?,
        media_type: MediaType(schema::string(entry, ATTACHMENT_TYPE)?),
        size: ByteSize(schema::unsigned(entry, ATTACHMENT_SIZE)?),
    })
}
