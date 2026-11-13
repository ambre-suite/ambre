//! The JPEG decode the images module serves, and the header read every build
//! carries (`ambre-spec-architecture.md section 2`, the converter modules).
//!
//! The Main instance links no JPEG decoder (`ARCH-078`). It reads the header
//! alone, which is a walk of the segments up to the frame. So each caller
//! refuses a picture past its bound and computes the size it wants before any
//! request crosses. The images module decodes, scales and turns the picture,
//! and answers the samples at that size.

use ambre_core::{FileBytes, Image, Payload, PayloadKind, Px, Size};
use serde::{Deserialize, Serialize};

/// One JPEG decode asked of the images module. Answered by
/// [`crate::Converted::Pixels`] at `size`, turned when `turn` asks for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JpegDecode {
    /// The file, whole.
    pub data: Payload<FileBytes>,
    /// The largest size the file may declare, each edge apart. The module
    /// holds the header and the decoder to it, so a header that lies is
    /// refused before the samples expand.
    pub bound: Size<Px<Image>>,
    /// The size of the answer before any turn: the stored size, or a smaller
    /// one the module scales the samples down to. Never larger than the file.
    pub size: Size<Px<Image>>,
    pub turn: JpegTurn,
}

/// Whether the answer applies the orientation the file's EXIF states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JpegTurn {
    /// The samples as the file stores them.
    Stored,
    /// The samples turned as the EXIF orientation states, so the picture
    /// stands as it was taken. A turn by a quarter swaps the two sides of
    /// [`JpegDecode::size`].
    Upright,
}

/// The start-of-image marker every JPEG file opens with.
const SOI: [u8; 2] = [0xFF, 0xD8];
/// The start of the scan: no frame header follows it.
const SOS: u8 = 0xDA;

/// Whether `file` opens as a JPEG file. A reader asks before it chooses the
/// images module for a picture.
#[must_use]
pub fn is_jpeg<K: PayloadKind>(file: &Payload<K>) -> bool {
    file.as_bytes().starts_with(&SOI)
}

/// The size the frame header of `file` declares, as the file stores it: its
/// orientation not applied. `None` for a file that is no JPEG, a walk that
/// meets a broken segment, or a size of zero.
#[must_use]
pub fn stored_size<K: PayloadKind>(file: &Payload<K>) -> Option<Size<Px<Image>>> {
    if !is_jpeg(file) {
        return None;
    }
    let file = file.as_bytes();
    let mut at = SOI.len();
    loop {
        // A run of fill bytes may stand before a marker.
        while file.get(at..at + 2) == Some([0xFF, 0xFF].as_slice()) {
            at += 1;
        }
        let [0xFF, marker] = *file.get(at..at + 2)? else {
            return None;
        };
        if marker == SOS {
            return None;
        }
        let len = usize::from(u16::from_be_bytes(
            file.get(at + 2..at + 4)?.try_into().ok()?,
        ));
        if is_frame(marker) {
            // Precision, then the height and the width, each on two bytes.
            let frame = file.get(at + 4..at + 9)?;
            let height = u16::from_be_bytes([*frame.get(1)?, *frame.get(2)?]);
            let width = u16::from_be_bytes([*frame.get(3)?, *frame.get(4)?]);
            return (width > 0 && height > 0)
                .then(|| Size::new(Px::new(u32::from(width)), Px::new(u32::from(height))));
        }
        at = (at + 2).checked_add(len)?;
    }
}

/// Whether `marker` opens a frame header: SOF0 to SOF15, less the Huffman
/// table, the JPEG extension and the arithmetic conditioning, which share
/// the range.
const fn is_frame(marker: u8) -> bool {
    matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The segments of a JPEG up to its frame: a JFIF header, then a frame
    /// of `width` by `height`.
    fn header(marker: u8, width: u16, height: u16) -> Vec<u8> {
        let mut bytes = SOI.to_vec();
        bytes.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x05, b'J', b'F', b'I']);
        bytes.extend_from_slice(&[0xFF, 0xC4, 0x00, 0x03, 0x00]);
        bytes.extend_from_slice(&[0xFF, marker, 0x00, 0x0B, 0x08]);
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&[0x01, 0x01, 0x11, 0x00]);
        bytes
    }

    fn bytes(file: Vec<u8>) -> Payload<FileBytes> {
        Payload::new(file)
    }

    #[test]
    fn the_frame_header_gives_the_stored_size() {
        for marker in [0xC0, 0xC1, 0xC2, 0xCF] {
            assert_eq!(
                stored_size(&bytes(header(marker, 4032, 3024))),
                Some(Size::new(Px::new(4032), Px::new(3024)))
            );
        }
    }

    #[test]
    fn a_file_with_no_frame_or_of_no_area_has_no_size() {
        assert_eq!(stored_size(&bytes(header(0xC0, 0, 10))), None);
        assert_eq!(stored_size(&bytes(b"\x89PNG\r\n\x1a\n".to_vec())), None);
        assert_eq!(stored_size(&bytes(SOI.to_vec())), None);
        let mut cut = header(0xC0, 10, 10);
        cut.truncate(cut.len() - 8);
        assert_eq!(stored_size(&bytes(cut)), None);
        let mut scan_first = SOI.to_vec();
        scan_first.extend_from_slice(&[0xFF, SOS, 0x00, 0x02]);
        assert_eq!(stored_size(&bytes(scan_first)), None);
    }
}
