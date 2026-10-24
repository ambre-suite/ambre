// ambre-draw/src/peer.rs
// ============================================================================
// The shared cross-app presence-colour algorithm.
//
// Every collaborator is painted in a stable colour derived purely from the seed
// bytes of their canonical `UserId`. The mapping is a pure function of those 16
// bytes: no per-app state, no random salt. The same user therefore keeps one
// colour across Prose, Trame and Toile. The colour holds even when they are
//
// signed in from two devices (`toile-presence`). Its home is `ambre-draw`
// because that is the render-primitives crate every app's canvas already
// depends on. It is seeded on the canonical `UserId` (not a raw `Uuid`), so the
// newtype is preserved end-to-end through presence. That is the sole reason
// `ambre-draw` takes a scoped `ambre-core` dependency (types only).
// ============================================================================

use ambre_core::{Rgb, UserId};

use crate::Color;

/// A stable, well-separated presence colour for the given peer, seeded on their
/// canonical [`UserId`].
///
/// The id bytes are folded into a hue with FNV-1a, then mapped through HSL at a
/// fixed saturation / lightness. Every peer colour is vivid and legible against
/// both the light and dark board backgrounds. It also stays visually distinct
/// from its neighbours around the wheel.
pub fn peer_color(user_id: &UserId) -> Color {
    let [r, g, b] = Hue::of_user(user_id).peer_rgb().channels();
    Color::from_rgba8(r, g, b, 255)
}

/// The colour of [`peer_color`] as an [`Rgb`]. The DOM chrome draws a user
/// with it beside the canvas: an avatar, a member row.
pub fn peer_rgb(user_id: &UserId) -> Rgb {
    let [r, g, b, _] = peer_color(user_id).to_rgba8().to_u8_array();
    Rgb::from_channels(r, g, b)
}

/// The saturation of every peer colour, in `0.0..=1.0`.
const PEER_SATURATION: f32 = 0.62;

/// The lightness of every peer colour, in `0.0..=1.0`.
const PEER_LIGHTNESS: f32 = 0.55;

/// A whole hue on the colour wheel, in degrees below 360.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Hue(u16);

impl Hue {
    /// The hue of a peer: the id seed bytes folded with FNV-1a, spread over
    /// the full wheel.
    fn of_user(user_id: &UserId) -> Self {
        let mut hash: u32 = 0x811c_9dc5;
        for &b in user_id.as_bytes() {
            hash ^= u32::from(b);
            hash = hash.wrapping_mul(0x0100_0193);
        }
        // `hash % 360` is below 360, so it fits in a u16.
        Self((hash % 360) as u16)
    }

    /// This hue at the peer saturation and lightness, in 8-bit sRGB.
    fn peer_rgb(self) -> Rgb {
        let (s, l) = (PEER_SATURATION, PEER_LIGHTNESS);
        let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
        let hp = f32::from(self.0) / 60.0;
        let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
        let (r1, g1, b1) = match self.0 / 60 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = l - c / 2.0;
        let to_u8 = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
        Rgb::from_channels(to_u8(r1), to_u8(g1), to_u8(b1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The DOM colour of a user is their canvas colour, without its alpha.
    #[test]
    fn the_dom_colour_is_the_canvas_colour() {
        let user = UserId::from_bytes([7u8; 16]);
        let [r, g, b, _] = peer_color(&user).to_rgba8().to_u8_array();
        assert_eq!(peer_rgb(&user).channels(), [r, g, b]);
    }

    #[test]
    fn stable_and_pure() {
        let user = UserId::from_bytes([7u8; 16]);
        assert_eq!(peer_color(&user), peer_color(&user));
    }

    #[test]
    fn distinct_seeds_usually_differ() {
        // Not a guarantee for every pair, but two arbitrary users should not
        // collapse to the same colour in the common case.
        let a = peer_color(&UserId::from_bytes([1u8; 16]));
        let b = peer_color(&UserId::from_bytes([2u8; 16]));
        assert_ne!(a, b);
    }

    #[test]
    fn opaque() {
        // Presence colours are always fully opaque; alpha is applied by the
        // caller per overlay element (halo vs cursor vs laser trail).
        let c = peer_color(&UserId::from_bytes([42u8; 16]));
        assert_eq!(c.components[3], 1.0);
    }
}
