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

use crate::bounded::{Bounded, Bounds, Defaulted, Refusal};

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
    pub const fn to_rgba(self, alpha: Alpha) -> Rgba {
        Rgba((self.0 << 8) | alpha.0 as u32)
    }
}

/// The alpha channel of a packed colour, as a byte: 0 shows nothing, 255
/// shows everything. Every byte is an alpha, so nothing is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Alpha(u8);

impl Alpha {
    /// Everything shows.
    pub const OPAQUE: Self = Self(u8::MAX);

    /// The alpha a channel byte `byte` holds.
    #[must_use]
    pub const fn new(byte: u8) -> Self {
        Self(byte)
    }

    /// The byte, for the call into a renderer that takes the channel.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
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

/// An opacity, from 0, transparent, to 1, opaque.
///
/// The value is finite and lies in that range. It serializes as the bare
/// `f32` it wraps, and a decoded value outside the range is refused.
pub enum Opacity {}

impl Bounds for Opacity {
    type Repr = f32;
    const MIN: f32 = 0.0;
    const MAX: f32 = 1.0;
    const NAME: &'static str = "Opacity";
    const REFUSAL: Refusal = Refusal::new("an opacity is a finite number from 0 to 1, not ", "");
}

impl Defaulted for Opacity {
    const DEFAULT: Bounded<Self> = Bounded::<Self>::OPAQUE;
}

impl Bounded<Opacity> {
    /// Nothing shows.
    pub const TRANSPARENT: Self = Self::MIN;
    /// Everything shows.
    pub const OPAQUE: Self = Self::MAX;

    /// The opacity nearest `value`: a value past either end reads as that
    /// end, and a NaN reads as transparent. A `const` item names an opacity
    /// through it.
    #[must_use]
    pub const fn clamped(value: f32) -> Self {
        if value.is_nan() {
            Self::TRANSPARENT
        } else {
            Self::at(value.clamp(0.0, 1.0))
        }
    }

    /// The opacity multiplied by `factor`, held in range: a fade at `factor`.
    #[must_use]
    pub fn scaled(self, factor: f32) -> Self {
        Self::clamped(self.get() * factor)
    }

    /// The opacity a fraction `t` of the way from `self` to `to`.
    #[must_use]
    pub fn lerp(self, to: Self, t: f64) -> Self {
        Self::clamped(self.get() + (to.get() - self.get()) * t as f32)
    }

    /// The alpha a channel of alpha `alpha` draws with at this opacity.
    #[must_use]
    pub fn fade(self, alpha: Alpha) -> Alpha {
        Alpha((f32::from(alpha.0) / 255.0 * self.get() * 255.0).round() as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::{Alpha, Opacity, Rgb, Rgba};
    use crate::Bounded;

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
        assert_eq!(rgb.to_rgba(Alpha::new(0x40)), rgba);
        assert_eq!(rgba.rgb().to_rgba(Alpha::new(0x40)), rgba);
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

    #[test]
    fn an_opacity_lies_between_zero_and_one() {
        assert_eq!(Bounded::<Opacity>::new(1.5), None);
        assert_eq!(Bounded::<Opacity>::new(f32::NAN), None);
        assert_eq!(
            Bounded::<Opacity>::clamped(-1.0),
            Bounded::<Opacity>::TRANSPARENT
        );
        assert_eq!(
            Bounded::<Opacity>::clamped(f32::NAN),
            Bounded::<Opacity>::TRANSPARENT
        );
        assert_eq!(
            Bounded::<Opacity>::OPAQUE.scaled(2.0),
            Bounded::<Opacity>::OPAQUE
        );
        assert_eq!(
            Bounded::<Opacity>::new(0.5).unwrap().fade(Alpha::OPAQUE),
            Alpha::new(128)
        );
        let past: Result<Bounded<Opacity>, _> = serde_json::from_str("2.0");
        assert!(past.is_err(), "a decoded opacity past 1 is refused");
    }

    /// A `const` opacity holds the range a run-time one holds.
    #[test]
    fn a_const_opacity_lies_between_zero_and_one() {
        const PAST: Bounded<Opacity> = Bounded::<Opacity>::clamped(3.0);
        const BELOW: Bounded<Opacity> = Bounded::<Opacity>::clamped(-0.5);
        const NOT_A_NUMBER: Bounded<Opacity> = Bounded::<Opacity>::clamped(f32::NAN);
        const FAINT: Bounded<Opacity> = Bounded::<Opacity>::clamped(0.3);
        assert_eq!(PAST, Bounded::<Opacity>::OPAQUE);
        assert_eq!(BELOW, Bounded::<Opacity>::TRANSPARENT);
        assert_eq!(NOT_A_NUMBER, Bounded::<Opacity>::TRANSPARENT);
        assert_eq!(FAINT, Bounded::<Opacity>::new(0.3).expect("in range"));
    }
}
