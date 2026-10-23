//! Colours, typed by their packing.
//!
//! An application packs a colour into a `u32`, in one of two byte orders.
//! Prose and Toile store `0xRRGGBBAA`, and Trame stores `0xRRGGBB`. A bare `u32`
//! lets one packing reach a reader of the other, and the colour comes out with
//! its channels shifted. One type per packing makes that a compile error, and
//! a conversion between the two packings a named call.
//!
//! Each type serializes as the bare `u32` it wraps, so a stored style and a
//! bus message keep the bytes they had.

use serde::{Deserialize, Serialize};

/// A colour packed `0xRRGGBBAA`: red in the top byte, alpha in the bottom one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Rgba(u32);

/// A colour packed `0x00RRGGBB`, with no alpha. The top byte is always zero.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Rgb(u32);

impl Rgba {
    /// The colour a `0xRRGGBBAA` integer packs.
    #[must_use]
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    /// The `0xRRGGBBAA` integer, as a document or a message stores it.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// The colour of four channels.
    #[must_use]
    pub const fn from_channels(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self(u32::from_be_bytes([r, g, b, a]))
    }

    /// The four channels, in the order red, green, blue, alpha.
    #[must_use]
    pub const fn channels(self) -> [u8; 4] {
        self.0.to_be_bytes()
    }

    /// The colour without its alpha.
    #[must_use]
    pub const fn rgb(self) -> Rgb {
        Rgb(self.0 >> 8)
    }
}

impl Rgb {
    /// The colour a `0xRRGGBB` integer packs. A top byte is dropped, since
    /// this packing has no alpha.
    #[must_use]
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits & 0x00ff_ffff)
    }

    /// The `0xRRGGBB` integer, as a document or a message stores it.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// The colour of three channels.
    #[must_use]
    pub const fn from_channels(r: u8, g: u8, b: u8) -> Self {
        Self(u32::from_be_bytes([0, r, g, b]))
    }

    /// The three channels, in the order red, green, blue.
    #[must_use]
    pub const fn channels(self) -> [u8; 3] {
        let [_, r, g, b] = self.0.to_be_bytes();
        [r, g, b]
    }

    /// This colour with an alpha, packed alpha last.
    #[must_use]
    pub const fn to_rgba(self, alpha: u8) -> Rgba {
        Rgba((self.0 << 8) | alpha as u32)
    }
}

/// The packed integer in hexadecimal, so a test message can name a colour.
macro_rules! lower_hex {
    ($($ty:ty),*) => {$(
        impl std::fmt::LowerHex for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                std::fmt::LowerHex::fmt(&self.0, f)
            }
        }
    )*};
}

lower_hex!(Rgba, Rgb);

/// The CSS hexadecimal notation `#rrggbbaa`, which a style property reads.
impl std::fmt::Display for Rgba {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{:08x}", self.0)
    }
}

/// The CSS hexadecimal notation `#rrggbb`, which a style property and the
/// value of an `<input type="color">` read.
impl std::fmt::Display for Rgb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{:06x}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::{Rgb, Rgba};

    // One orange in the two packings: red 0xff, green 0x80, blue 0x20,
    // alpha 0x40.
    const ORANGE_RGBA: u32 = 0xff80_2040;
    const ORANGE_RGB: u32 = 0x00ff_8020;

    #[test]
    fn each_packing_reads_the_known_channels() {
        assert_eq!(
            Rgba::from_bits(ORANGE_RGBA).channels(),
            [0xff, 0x80, 0x20, 0x40]
        );
        assert_eq!(Rgb::from_bits(ORANGE_RGB).channels(), [0xff, 0x80, 0x20]);
        assert_eq!(
            Rgba::from_channels(0xff, 0x80, 0x20, 0x40).bits(),
            ORANGE_RGBA
        );
        assert_eq!(Rgb::from_channels(0xff, 0x80, 0x20).bits(), ORANGE_RGB);
    }

    #[test]
    fn a_conversion_keeps_the_channels() {
        let rgba = Rgba::from_bits(ORANGE_RGBA);
        let rgb = Rgb::from_bits(ORANGE_RGB);
        assert_eq!(rgba.rgb(), rgb);
        assert_eq!(rgb.to_rgba(0x40), rgba);
        assert_eq!(rgba.rgb().to_rgba(0x40), rgba);
    }

    #[test]
    fn rgb_drops_a_top_byte() {
        assert_eq!(Rgb::from_bits(0x40ff_8020).bits(), ORANGE_RGB);
    }

    #[test]
    fn a_colour_prints_as_its_css_hex() {
        assert_eq!(Rgba::from_bits(ORANGE_RGBA).to_string(), "#ff802040");
        assert_eq!(Rgb::from_bits(ORANGE_RGB).to_string(), "#ff8020");
        assert_eq!(Rgb::from_bits(0x0000_0102).to_string(), "#000102");
    }

    #[test]
    fn each_packing_serializes_as_its_integer() {
        let json = serde_json::to_string(&Rgb::from_bits(ORANGE_RGB)).ok();
        assert_eq!(json, serde_json::to_string(&ORANGE_RGB).ok());
        let bytes = postcard::to_allocvec(&Rgba::from_bits(ORANGE_RGBA)).ok();
        assert_eq!(bytes, postcard::to_allocvec(&ORANGE_RGBA).ok());
        let back: Option<Rgb> = serde_json::from_str("16744480").ok();
        assert_eq!(back, Some(Rgb::from_bits(ORANGE_RGB)));
    }
}
