//! The PNG and GIF decodes the images module serves, and the header reads
//! every build carries (`ambre-spec-architecture.md section 2`, the converter
//! thread).
//!
//! A caller reads the header on the Logic thread, refuses a picture past its
//! bound and computes the size it wants. The images module decodes on the
//! converter thread, holds the header and the decoder to the bound again,
//! scales and, for a PNG, turns. So the Logic thread never decodes a PNG or a
//! GIF picture of Prose or a Murmure preview (`ARCH-097`).

use ambre_core::{Count, FileBytes, Image, Octets, Payload, PayloadKind, Px, Size};
use serde::{Deserialize, Serialize};

use crate::jpeg::JpegTurn;

/// One PNG decode asked of the images module. Answered by
/// [`crate::Converted::Pixels`] at `size`, turned when `turn` asks for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PngDecode {
    /// The file, whole.
    pub data: Payload<FileBytes>,
    /// The largest size the file may declare, each edge apart. The module
    /// holds the header and the decoder to it.
    pub bound: Size<Px<Image>>,
    /// The size of the answer before any turn: the stored size, or a smaller
    /// one the module scales the samples down to. Never larger than the file.
    pub size: Size<Px<Image>>,
    /// Whether the answer applies the EXIF orientation of the `eXIf` chunk,
    /// as a JPEG decode applies the one of its APP1 segment.
    pub turn: JpegTurn,
}

/// One GIF decode asked of the images module. Answered by
/// [`crate::Converted::Frames`], each frame at `size`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GifDecode {
    /// The file, whole.
    pub data: Payload<FileBytes>,
    /// The largest logical screen the file may declare, each edge apart.
    pub bound: Size<Px<Image>>,
    /// The size of each frame of the answer: the logical screen, or a
    /// smaller size the module scales each frame down to.
    pub size: Size<Px<Image>>,
    /// The bytes the frames of the answer may hold. The first frame is
    /// always kept, and the frames after it stop at this budget. A budget of
    /// zero asks for the first frame alone.
    pub budget: Count<Octets>,
}

/// Why the images module refused a PNG or a GIF decode. The words a person
/// reads are made where the refusal is shown; `Display` gives the words of
/// the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum RasterRefusal {
    /// The file is not of the format the request names, or its header does
    /// not read.
    #[error("the file is not of the format this decode reads")]
    OtherFormat,
    /// The header declares a size past the bound of the request.
    #[error(
        "the picture is {}x{}, past the {}x{} it may span",
        stored.width.get(),
        stored.height.get(),
        bound.width.get(),
        bound.height.get()
    )]
    PastBound {
        stored: Size<Px<Image>>,
        bound: Size<Px<Image>>,
    },
    /// The size asked for is empty, or larger than the file.
    #[error("the size asked for is not one the picture can give")]
    SizeUnavailable,
    /// The size the decoder read differs from the one the header declares.
    #[error("the size the decoder read is not the one the header declares")]
    FrameMismatch,
    /// The decoder refused the file, or its first frame.
    #[error("the decoder refused the file")]
    Undecodable,
}

/// The eight bytes every PNG file opens with.
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
/// The two signatures a GIF file opens with.
const GIF_SIGNATURES: [&[u8; 6]; 2] = [b"GIF87a", b"GIF89a"];

/// Whether `file` opens as a PNG file.
#[must_use]
pub fn is_png<K: PayloadKind>(file: &Payload<K>) -> bool {
    file.as_bytes().starts_with(&PNG_SIGNATURE)
}

/// Whether `file` opens as a GIF file.
#[must_use]
pub fn is_gif<K: PayloadKind>(file: &Payload<K>) -> bool {
    let bytes = file.as_bytes();
    GIF_SIGNATURES
        .iter()
        .any(|signature| bytes.starts_with(signature.as_slice()))
}

/// The size the `IHDR` chunk of `file` declares. `None` for a file that is
/// no PNG, a first chunk that is no `IHDR`, or a size of zero.
#[must_use]
pub fn png_size<K: PayloadKind>(file: &Payload<K>) -> Option<Size<Px<Image>>> {
    if !is_png(file) {
        return None;
    }
    let bytes = file.as_bytes();
    // The signature, the chunk length, then the chunk type.
    if bytes.get(12..16)? != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes.get(16..20)?.try_into().ok()?);
    let height = u32::from_be_bytes(bytes.get(20..24)?.try_into().ok()?);
    (width > 0 && height > 0).then(|| Size::new(Px::new(width), Px::new(height)))
}

/// The logical screen the header of `file` declares: the size of each frame
/// of the animation. `None` for a file that is no GIF, or a size of zero.
#[must_use]
pub fn gif_size<K: PayloadKind>(file: &Payload<K>) -> Option<Size<Px<Image>>> {
    if !is_gif(file) {
        return None;
    }
    let bytes = file.as_bytes();
    let width = u16::from_le_bytes(bytes.get(6..8)?.try_into().ok()?);
    let height = u16::from_le_bytes(bytes.get(8..10)?.try_into().ok()?);
    (width > 0 && height > 0)
        .then(|| Size::new(Px::new(u32::from(width)), Px::new(u32::from(height))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(file: &[u8]) -> Payload<FileBytes> {
        Payload::new(file.to_vec())
    }

    /// A PNG header of `width` by `height`: the signature and the `IHDR`.
    fn png_header(width: u32, height: u32) -> Vec<u8> {
        let mut out = PNG_SIGNATURE.to_vec();
        out.extend_from_slice(&13u32.to_be_bytes());
        out.extend_from_slice(b"IHDR");
        out.extend_from_slice(&width.to_be_bytes());
        out.extend_from_slice(&height.to_be_bytes());
        out.extend_from_slice(&[8, 6, 0, 0, 0]);
        out
    }

    #[test]
    fn a_png_header_gives_its_size_and_a_broken_one_none() {
        assert_eq!(
            png_size(&bytes(&png_header(4032, 3024))),
            Some(Size::new(Px::new(4032), Px::new(3024)))
        );
        assert_eq!(png_size(&bytes(&png_header(0, 3))), None);
        assert_eq!(png_size(&bytes(&PNG_SIGNATURE)), None);
        let mut other = png_header(3, 3);
        other[12..16].copy_from_slice(b"tEXt");
        assert_eq!(png_size(&bytes(&other)), None);
        assert_eq!(png_size(&bytes(b"GIF89a\x03\x00\x03\x00")), None);
    }

    #[test]
    fn a_gif_header_gives_its_logical_screen_and_a_broken_one_none() {
        for signature in GIF_SIGNATURES {
            let mut file = signature.to_vec();
            file.extend_from_slice(&[0x40, 0x01, 0xF0, 0x00]);
            assert!(is_gif(&bytes(&file)));
            assert_eq!(
                gif_size(&bytes(&file)),
                Some(Size::new(Px::new(320), Px::new(240)))
            );
        }
        assert_eq!(gif_size(&bytes(b"GIF89a\x00\x00\x01\x00")), None);
        assert_eq!(gif_size(&bytes(b"GIF89a\x01")), None);
        assert_eq!(gif_size(&bytes(&png_header(3, 3))), None);
    }
}
