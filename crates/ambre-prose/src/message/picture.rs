//! The vocabulary of a picture in the document
//! (`features/ambre-spec-prose-images.md section 3`).
//!
//! The document holds an address and a display size. The pixels travel apart,
//! through the media flow of the Sync worker, and reach the view as decoded
//! samples on the shared heap. Nothing here names a key or a network call.

use std::fmt;
use std::sync::Arc;

use ambre_core::{ContentHash, Doc, Image, Payload, Px, RgbaPixels, Size, Validated};

/// Where a picture sits across its line (`PROSE-FEAT-217`).
///
/// `Left` and `Right` ask for text wrapped beside the picture. The shaper has
/// no exclusion zones, so the layout lays both out as `Block`, and the
/// document keeps what the author asked for (`PROSE-FEAT-209`).
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ImageAlign {
    /// A block of its own, at the left edge of the text. The default.
    #[default]
    Block = 0,
    /// Left, text wrapped to the right. Laid out as `Block`.
    Left = 1,
    /// Right, text wrapped to the left. Laid out as `Block`.
    Right = 2,
    /// Centred, no wrapping.
    Center = 3,
}

impl ImageAlign {
    /// The stored byte (`ambre-spec-prose-schema.md section 3`).
    #[must_use]
    pub const fn byte(self) -> u8 {
        self as u8
    }

    /// The alignment a stored byte names. A byte no build knows reads as
    /// `Block`, the alignment a peer that wrote it would see laid out.
    #[must_use]
    pub const fn from_byte(byte: u8) -> Self {
        match byte {
            1 => Self::Left,
            2 => Self::Right,
            3 => Self::Center,
            _ => Self::Block,
        }
    }

    /// Whether the layout centres the box: every other alignment draws it at
    /// the left edge of the text.
    #[must_use]
    pub const fn is_centred(self) -> bool {
        matches!(self, Self::Center)
    }
}

/// The display size of a picture, in the pixels of the page at zoom one. It
/// is never the native size of the file (`ambre-spec-prose-schema.md section
/// 3`).
pub type DisplaySize = Size<Px<Doc>>;

/// Why a picture failed for good (`PROSE-FEAT-057`). Neither is asked for
/// again without a reader's gesture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MediaFailure {
    /// The Sync worker answered no plaintext: the relay does not hold the
    /// blob, the blob did not open, or the network failed past its retries.
    Unavailable,
    /// The plaintext is not a picture this build decodes, or its header
    /// declares an edge past the source bound (`PROSE-FEAT-314`).
    Undecodable,
}

/// The decoded samples of one picture, at most the cache edge on each side
/// (`PROSE-FEAT-315`). The view paints them and never decodes.
#[derive(Clone, PartialEq)]
pub struct DecodedPicture {
    /// Non-premultiplied RGBA8, row-major, `width * height * 4` bytes.
    pub pixels: Payload<RgbaPixels>,
    pub size: Size<Px<Image>>,
}

impl DecodedPicture {
    /// The bytes the samples hold, the figure the cache cap counts
    /// (`PROSE-FEAT-054`).
    #[must_use]
    pub fn held(&self) -> ambre_core::Count<ambre_core::Octets> {
        self.pixels.len()
    }
}

impl fmt::Debug for DecodedPicture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DecodedPicture")
            .field("size", &self.size)
            .field("pixels", &self.pixels)
            .finish()
    }
}

/// How a picture's stored pixels turn to stand as it is shown, as the EXIF
/// orientation of a JPEG states it. The stored file keeps this one tag of its
/// metadata, so it displays as it was taken (`PROSE-FEAT-326`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Orientation {
    /// Shown as stored.
    #[default]
    Upright,
    FlipHorizontal,
    Rotate180,
    FlipVertical,
    /// A quarter turn clockwise.
    Rotate90,
    /// A quarter turn counter-clockwise.
    Rotate270,
    /// Mirrored across the top-left to bottom-right diagonal.
    Transpose,
    /// Mirrored across the top-right to bottom-left diagonal.
    Transverse,
}

impl Orientation {
    /// Whether the shown picture is the stored one on its side: its width
    /// is the stored height.
    #[must_use]
    pub const fn swaps_sides(self) -> bool {
        matches!(
            self,
            Self::Rotate90 | Self::Rotate270 | Self::Transpose | Self::Transverse
        )
    }
}

/// A picture as a print draws it: the stored file, which the PDF module
/// embeds at full resolution, never decoded on the Logic thread
/// (`PROSE-IO-107`).
#[derive(Clone, PartialEq)]
pub struct PrintedPicture {
    pub format: ambre_draw::EncodedFormat,
    pub bytes: Payload<ambre_core::MediaBytes>,
    /// The stored pixel size, before the orientation turns it.
    pub size: Size<Px<Image>>,
    pub orientation: Orientation,
}

impl fmt::Debug for PrintedPicture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PrintedPicture")
            .field("format", &self.format)
            .field("bytes", &self.bytes)
            .field("size", &self.size)
            .field("orientation", &self.orientation)
            .finish()
    }
}

/// What the view shows for one address. A fact of the session, never of the
/// document (`PROSE-FEAT-207`).
#[derive(Clone, Debug, PartialEq)]
pub enum PictureShown {
    /// Asked for or not yet asked for: the neutral placeholder
    /// (`PROSE-UI-146`).
    Waiting,
    /// Decoded, ready to paint.
    Ready(Arc<DecodedPicture>),
    /// Failed for good: the broken mark and the retry (`PROSE-UI-046`,
    /// `PROSE-UI-145`).
    Failed(MediaFailure),
    /// The stored file, for a print alone: the screen never holds one.
    Printed(Arc<PrintedPicture>),
}

/// A request a picture needs from the Sync worker. Prose mints the request
/// id, so it matches the answer back. The Logic dispatcher adds the document
/// and lowers it to a service-plane command: Prose makes no network call and
/// names no key (`PROSE-025`).
#[derive(Debug)]
pub enum MediaRequest {
    /// Store the plaintext of a picture and answer its address.
    Upload {
        id: ambre_core::Seq<ambre_bus::rest::Request>,
        data: Payload<ambre_core::MediaBytes>,
    },
    /// Fetch and open the blob an address names.
    Download {
        id: ambre_core::Seq<ambre_bus::rest::Request>,
        content_hash: Validated<ContentHash>,
    },
}

/// An address a request carries. Read by a test that drives the answer.
impl MediaRequest {
    /// The request id.
    #[must_use]
    pub fn id(&self) -> ambre_core::Seq<ambre_bus::rest::Request> {
        match self {
            Self::Upload { id, .. } | Self::Download { id, .. } => *id,
        }
    }
}
