//! The files a draft carries while they upload, and once they did
//! (`specs/5-Murmure/features/ambre-spec-murmure-attachments.md`).
//!
//! The view asks the Logic thread to upload each file, and hands its request
//! here. A file joins the draft only once the relay answered its upload
//! (`MURMURE-132`). A failed upload adds nothing, and the page tells the user
//! (`MURMURE-133`). A draft never sends while one of its files still uploads.

use std::collections::HashMap;

use ambre_bus::{Attachment, ByteSize, ContentHash, MediaType, RequestId};
use ambre_core::{ByteCount, ExportFilename, KnownMediaType};

use crate::composer::DraftOf;

ambre_core::define_index! {
    /// The place of a file among the uploaded files of one draft, counted
    /// from zero in the order they landed.
    pub struct FileIndex(u32);
    /// A number of files of a draft.
    pub struct FileCount;
    /// The signed gap between two files of a draft.
    pub struct FileDelta(i64);
    /// A run of files of a draft.
    pub struct FileSpan;
    /// A list indexed by the files of a draft.
    pub struct FileVec;
}

/// A file the member dropped or pasted on the composer, as the page read it.
///
/// `Debug` prints the length of the name alone, and the bytes print their
/// length too: both are plaintext the member chose.
#[derive(Clone, PartialEq, Eq)]
pub struct PickedFile {
    pub name: String,
    /// The type the browser named, empty when it named none.
    pub media_type: MediaType,
    pub data: ambre_core::Payload<ambre_core::MediaBytes>,
}

impl std::fmt::Debug for PickedFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PickedFile")
            .field("name", &ByteCount::from_len(self.name.len()))
            .field("media_type", &self.media_type)
            .field("data", &self.data)
            .finish()
    }
}

/// A size as a reader reads it: "512 B", "2 KB", "3.4 MB".
pub fn size_label(size: ByteSize) -> String {
    const KB: ByteSize = ByteSize(1024);
    const MB: ByteSize = ByteSize(1024 * 1024);
    // Kilobytes round up, so the unit follows the rounded size: a size short
    // of a megabyte by less than a kilobyte reads in megabytes.
    let kilobytes = size.0.div_ceil(KB.0);
    let rounded = ByteSize(kilobytes.saturating_mul(KB.0));
    match size {
        size if size < KB => format!("{} B", size.0),
        _ if rounded < MB => format!("{kilobytes} KB"),
        size => format!("{:.1} MB", size.0 as f64 / MB.0 as f64),
    }
}

/// The type a file falls back to when the browser names none.
const UNKNOWN_TYPE: KnownMediaType = KnownMediaType::OctetStream;

impl PickedFile {
    /// The type the attachment records.
    pub fn media_type(&self) -> MediaType {
        match self.media_type.as_str().trim() {
            "" => UNKNOWN_TYPE.media_type(),
            named => MediaType(named.to_owned()),
        }
    }
}

/// The types a click opens in a tab of their own. Every other file saves.
/// A blob address runs in the page's origin. So a type that carries script,
/// such as SVG or HTML, never opens: it would run in another's session.
const OPENS: [KnownMediaType; 5] = [
    KnownMediaType::Png,
    KnownMediaType::Jpeg,
    KnownMediaType::Gif,
    KnownMediaType::Webp,
    KnownMediaType::Avif,
];

/// The type a file of `media_type` opens under in a tab, `None` when it
/// saves as a plain stream of bytes.
pub fn opens_as(media_type: &MediaType) -> Option<KnownMediaType> {
    KnownMediaType::of(media_type).filter(|known| OPENS.contains(known))
}

/// The stem a fetched file saves under when its name leaves nothing.
const FALLBACK_STEM: &str = "attachment";

/// The name a fetched file saves under. The sender's file system gave the
/// name, so it is another user's input. It loses every path step and every
/// character a host refuses, and fits the bound (`DOCFILE-019`,
/// `DOCFILE-020`).
pub fn download_name(file: &Attachment) -> ExportFilename {
    ExportFilename::from_received(&file.name, FALLBACK_STEM)
}

/// A file whose upload the relay has not answered yet.
///
/// `Debug` prints the length of the name alone: the member named the file.
#[derive(Clone)]
struct Pending {
    of: DraftOf,
    name: String,
    media_type: MediaType,
    size: ByteSize,
}

impl std::fmt::Debug for Pending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pending")
            .field("of", &self.of)
            .field("name", &ByteCount::from_len(self.name.len()))
            .field("media_type", &self.media_type)
            .field("size", &self.size)
            .finish()
    }
}

/// What an answer did to the draft it belonged to.
///
/// `Debug` prints the length of a failed file's name alone.
#[derive(Clone, PartialEq, Eq)]
pub enum Uploaded {
    /// The file joined the draft.
    Ready(DraftOf),
    /// The upload failed, so the draft carries nothing of it. The view tells
    /// the user, naming the file.
    Failed { of: DraftOf, name: String },
}

impl std::fmt::Debug for Uploaded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ready(of) => f.debug_tuple("Ready").field(of).finish(),
            Self::Failed { of, name } => f
                .debug_struct("Failed")
                .field("of", of)
                .field("name", &ByteCount::from_len(name.len()))
                .finish(),
        }
    }
}

/// The uploads of every draft, keyed by the request that carries each.
///
/// `Debug` counts the files uploaded: each attachment names its file.
#[derive(Default)]
pub struct Uploads {
    pending: HashMap<RequestId, Pending>,
    ready: HashMap<DraftOf, Vec<Attachment>>,
}

impl std::fmt::Debug for Uploads {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ready: usize = self.ready.values().map(Vec::len).sum();
        f.debug_struct("Uploads")
            .field("pending", &self.pending)
            .field("ready", &ready)
            .finish()
    }
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

    /// The relay stored the file of `id` under `content_hash`, which its
    /// reader already checked. `None` for a request this model never held.
    pub fn uploaded(&mut self, id: RequestId, content_hash: &ContentHash) -> Option<Uploaded> {
        let pending = self.pending.remove(&id)?;
        self.ready.entry(pending.of).or_default().push(Attachment {
            media_ref: content_hash.clone(),
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
    pub fn remove(&mut self, of: DraftOf, index: FileIndex) {
        if let Some(files) = self.ready.get_mut(&of)
            && let Some(index) = index.to_index().filter(|&index| index < files.len())
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

    /// A size reads in the largest unit it reaches once rounded up. So a
    /// size just short of a megabyte reads "1.0 MB", never "1024 KB".
    #[test]
    fn a_size_label_never_reads_1024_kb() {
        assert_eq!(size_label(ByteSize(512)), "512 B");
        assert_eq!(size_label(ByteSize(1024)), "1 KB");
        assert_eq!(size_label(ByteSize(1025)), "2 KB");
        assert_eq!(size_label(ByteSize(1023 * 1024)), "1023 KB");
        assert_eq!(size_label(ByteSize(1023 * 1024 + 1)), "1.0 MB");
        assert_eq!(size_label(ByteSize(1024 * 1024 - 1)), "1.0 MB");
        assert_eq!(size_label(ByteSize(3_565_158)), "3.4 MB");
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

    /// The debug form of a picked file and of a file still uploading names
    /// the length of its name, never a char of it.
    #[test]
    fn a_file_name_debugs_its_length_alone() {
        let picked = PickedFile {
            name: "secret.pdf".to_owned(),
            media_type: MediaType("application/pdf".to_owned()),
            data: ambre_core::Payload::<ambre_core::MediaBytes>::new(b"secret".to_vec()),
        };
        let shown = format!("{picked:?}");
        assert!(!shown.contains("secret"), "{shown}");
        assert!(shown.contains("name: ByteCount(10)"), "{shown}");
        assert!(shown.contains("MediaBytes(6 bytes)"), "{shown}");
        let mut uploads = Uploads::default();
        assert!(begin(&mut uploads, 1, channel(), "secret.pdf"));
        uploads.restore(channel(), vec![received("secret.txt")]);
        let shown = format!("{uploads:?}");
        assert!(!shown.contains("secret"), "{shown}");
        assert!(shown.contains("name: ByteCount(10)"), "{shown}");
        assert!(shown.contains("ready: 1"), "{shown}");
    }

    /// The debug form of a failed upload names the length of the file's
    /// name, never a char of it.
    #[test]
    fn a_failed_upload_debugs_its_name_length_alone() {
        let failed = Uploaded::Failed {
            of: channel(),
            name: "secret.pdf".to_owned(),
        };
        let shown = format!("{failed:?}");
        assert!(!shown.contains("secret"), "{shown}");
        assert!(shown.contains("name: ByteCount(10)"), "{shown}");
    }

    /// A fetched file named `name`.
    fn received(name: &str) -> Attachment {
        Attachment {
            media_ref: ContentHash::parse(HASH).expect("an address"),
            name: name.to_owned(),
            media_type: MediaType("application/pdf".to_owned()),
            size: ByteSize(10),
        }
    }

    /// `DOCFILE-019`, `DOCFILE-020`: a name another member sent never names a
    /// place outside the download folder, and fits the bound. A safe extension
    /// stays.
    #[test]
    fn a_received_name_saves_inside_the_download_folder() {
        let long = format!("{}.pdf", "a".repeat(300));
        let cases = [
            ("../../.ssh/authorized_keys", "authorized_keys"),
            ("..\\..\\boot.ini", "boot.ini"),
            ("..", "attachment"),
            ("a\u{0}b\nc.pdf", "abc.pdf"),
            ("report.pdf", "report.pdf"),
        ];
        for (sent, saved) in cases {
            let name = download_name(&received(sent));
            assert_eq!(name.as_str(), saved, "sent as {sent:?}");
            assert!(ambre_core::ExportFilename::new(name.as_str()).is_ok());
        }
        let name = download_name(&received(&long));
        assert!(ambre_core::ExportFilename::new(name.as_str()).is_ok());
        assert!(name.as_str().ends_with(".pdf"), "{}", name.as_str());
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
            uploads.uploaded(RequestId(1), &ContentHash::parse(HASH).expect("an address")),
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
        assert_eq!(uploads.failed(RequestId(2)), failed("b.png"));
        assert!(!uploads.busy(of));
        assert!(uploads.ready(of).is_empty());
        assert_eq!(uploads.failed(RequestId(3)), None);
    }

    /// A raster image opens; a type that can carry script saves.
    #[test]
    fn only_a_raster_image_opens_in_a_tab() {
        let opens = |named: &str| opens_as(&MediaType(named.to_owned()));
        assert_eq!(opens("image/png"), Some(KnownMediaType::Png));
        assert_eq!(opens(" IMAGE/JPEG "), Some(KnownMediaType::Jpeg));
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
