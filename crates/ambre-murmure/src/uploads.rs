//! The files a draft carries while they upload, and once they did
//! (`specs/5-Murmure/features/ambre-spec-murmure-attachments.md`).
//!
//! The view asks the Logic thread to upload each file, and hands its request
//! here. A file joins the draft only once the relay answered its upload
//! (`MURMURE-132`). A failed upload adds nothing, and the page tells the user
//! (`MURMURE-133`). A draft never sends while one of its files still uploads.

use std::collections::HashMap;

use ambre_bus::{Attachment, ByteSize, MediaRef, MediaType, RequestId};

use crate::composer::DraftOf;

/// A file the member dropped or pasted on the composer, as the page read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickedFile {
    pub name: String,
    /// The type the browser named, empty when it named none.
    pub media_type: String,
    pub data: Vec<u8>,
}

/// A size as a reader reads it: "512 B", "2 KB", "3.4 MB".
pub fn size_label(size: ByteSize) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * KB;
    match size.0 {
        bytes if bytes < KB => format!("{bytes} B"),
        bytes if bytes < MB => format!("{} KB", bytes.div_ceil(KB)),
        bytes => format!("{:.1} MB", bytes as f64 / MB as f64),
    }
}

/// The type a file falls back to when the browser names none.
const UNKNOWN_TYPE: &str = "application/octet-stream";

impl PickedFile {
    /// The type the attachment records.
    pub fn media_type(&self) -> MediaType {
        match self.media_type.trim() {
            "" => MediaType(UNKNOWN_TYPE.to_owned()),
            named => MediaType(named.to_owned()),
        }
    }
}

/// The types a click opens in a tab of their own. Every other file saves.
/// A blob address runs in the page's origin. So a type that carries script,
/// such as SVG or HTML, never opens: it would run in another's session.
const OPENS: [&str; 5] = [
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/avif",
];

/// The type a file of `media_type` opens under in a tab, `None` when it
/// saves as a plain stream of bytes.
pub fn opens_as(media_type: &MediaType) -> Option<&'static str> {
    let named = media_type.0.trim().to_ascii_lowercase();
    OPENS.into_iter().find(|&opens| opens == named)
}

/// A file whose upload the relay has not answered yet.
#[derive(Debug, Clone)]
struct Pending {
    of: DraftOf,
    name: String,
    media_type: MediaType,
    size: ByteSize,
}

/// What an answer did to the draft it belonged to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Uploaded {
    /// The file joined the draft.
    Ready(DraftOf),
    /// The upload failed, so the draft carries nothing of it. The view tells
    /// the user, naming the file.
    Failed { of: DraftOf, name: String },
}

/// The uploads of every draft, keyed by the request that carries each.
#[derive(Debug, Default)]
pub struct Uploads {
    pending: HashMap<RequestId, Pending>,
    ready: HashMap<DraftOf, Vec<Attachment>>,
}

impl Uploads {
    /// Hold the upload request `id` for a file of the draft `of`. A reply in
    /// a thread carries no file, so a thread draft refuses it.
    pub fn begin(
        &mut self,
        id: RequestId,
        of: DraftOf,
        name: String,
        media_type: MediaType,
        size: ByteSize,
    ) -> bool {
        if matches!(of, DraftOf::Thread(..)) || name.trim().is_empty() {
            return false;
        }
        self.pending.insert(
            id,
            Pending {
                of,
                name,
                media_type,
                size,
            },
        );
        true
    }

    /// The relay stored the file of `id` under `content_hash`. An address
    /// this build cannot read fails the upload. `None` for a request this
    /// model never held.
    pub fn uploaded(&mut self, id: RequestId, content_hash: &str) -> Option<Uploaded> {
        let pending = self.pending.remove(&id)?;
        let Some(media_ref) = MediaRef::parse(content_hash) else {
            return Some(Uploaded::Failed {
                of: pending.of,
                name: pending.name,
            });
        };
        self.ready.entry(pending.of).or_default().push(Attachment {
            media_ref,
            name: pending.name,
            media_type: pending.media_type,
            size: pending.size,
        });
        Some(Uploaded::Ready(pending.of))
    }

    /// The upload of `id` failed. `None` for a request this model never held.
    pub fn failed(&mut self, id: RequestId) -> Option<Uploaded> {
        let pending = self.pending.remove(&id)?;
        Some(Uploaded::Failed {
            of: pending.of,
            name: pending.name,
        })
    }

    /// A file of the draft `of` still uploads, so the draft does not send.
    pub fn busy(&self, of: DraftOf) -> bool {
        self.pending.values().any(|pending| pending.of == of)
    }

    /// The files the draft `of` carries, in the order they uploaded.
    pub fn ready(&self, of: DraftOf) -> &[Attachment] {
        self.ready.get(&of).map_or(&[], Vec::as_slice)
    }

    /// The names of the files of the draft `of` that still upload, in the
    /// order they were asked.
    pub fn uploading(&self, of: DraftOf) -> Vec<&str> {
        let mut pending: Vec<(&RequestId, &Pending)> = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.of == of)
            .collect();
        pending.sort_by_key(|(id, _)| id.0);
        pending
            .into_iter()
            .map(|(_, pending)| pending.name.as_str())
            .collect()
    }

    /// Give the draft `of` back the files of a send its author cancelled
    /// (`MURMURE-558`).
    pub fn restore(&mut self, of: DraftOf, attachments: Vec<Attachment>) {
        if !attachments.is_empty() {
            self.ready.entry(of).or_default().extend(attachments);
        }
    }

    /// Take the file at `index` off the draft `of`. The relay keeps the blob
    /// until a delete of a message carrying it.
    pub fn remove(&mut self, of: DraftOf, index: usize) {
        if let Some(files) = self.ready.get_mut(&of)
            && index < files.len()
        {
            files.remove(index);
        }
    }

    /// The files leave with the draft that sends them.
    pub(crate) fn take(&mut self, of: DraftOf) -> Vec<Attachment> {
        self.ready.remove(&of).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::{ChannelId, MessageId};

    const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn channel() -> DraftOf {
        DraftOf::Channel(ChannelId::new())
    }

    fn begin(uploads: &mut Uploads, id: u64, of: DraftOf, name: &str) -> bool {
        uploads.begin(
            RequestId(id),
            of,
            name.to_owned(),
            MediaType("image/png".to_owned()),
            ByteSize(10),
        )
    }

    /// `MURMURE-132`: a file joins its draft once the relay answered, and the
    /// draft holds while it uploads.
    #[test]
    fn a_file_joins_its_draft_once_its_upload_is_answered() {
        let mut uploads = Uploads::default();
        let of = channel();
        assert!(begin(&mut uploads, 1, of, "a.png"));
        assert!(uploads.busy(of));
        assert!(uploads.ready(of).is_empty());
        assert_eq!(
            uploads.uploaded(RequestId(1), HASH),
            Some(Uploaded::Ready(of))
        );
        assert!(!uploads.busy(of));
        assert_eq!(uploads.ready(of)[0].media_ref.as_str(), HASH);
        assert_eq!(uploads.take(of).len(), 1);
        assert!(uploads.ready(of).is_empty());
    }

    /// `MURMURE-133`: a failed upload, or an address this build cannot read,
    /// adds nothing and names its file.
    #[test]
    fn a_failed_upload_adds_nothing_and_names_its_file() {
        let mut uploads = Uploads::default();
        let of = channel();
        begin(&mut uploads, 1, of, "a.png");
        begin(&mut uploads, 2, of, "b.png");
        let failed = |name: &str| {
            Some(Uploaded::Failed {
                of,
                name: name.to_owned(),
            })
        };
        assert_eq!(uploads.failed(RequestId(1)), failed("a.png"));
        assert_eq!(uploads.uploaded(RequestId(2), "not-hex"), failed("b.png"));
        assert!(!uploads.busy(of));
        assert!(uploads.ready(of).is_empty());
        assert_eq!(uploads.failed(RequestId(3)), None);
    }

    /// A raster image opens; a type that can carry script saves.
    #[test]
    fn only_a_raster_image_opens_in_a_tab() {
        let opens = |named: &str| opens_as(&MediaType(named.to_owned()));
        assert_eq!(opens("image/png"), Some("image/png"));
        assert_eq!(opens(" IMAGE/JPEG "), Some("image/jpeg"));
        for saves in ["image/svg+xml", "text/html", "application/pdf", ""] {
            assert_eq!(opens(saves), None, "{saves} saves");
        }
    }

    #[test]
    fn a_thread_draft_or_a_blank_name_takes_no_file() {
        let mut uploads = Uploads::default();
        let thread = DraftOf::Thread(ChannelId::new(), MessageId::new());
        assert!(!begin(&mut uploads, 1, thread, "a.png"));
        assert!(!begin(&mut uploads, 2, channel(), "  "));
        assert!(uploads.pending.is_empty());
    }
}
