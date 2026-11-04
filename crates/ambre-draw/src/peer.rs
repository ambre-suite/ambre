// ambre-draw/src/peer.rs
// ============================================================================
// The shared cross-app presence-colour algorithm.
//
// Every collaborator is painted in a stable colour derived purely from the seed
// bytes of their canonical `Id<User>`. The mapping is a pure function of those 16
// bytes: no per-app state, no random salt. The same user therefore keeps one
// colour across Prose, Trame and Toile. The colour holds even when they are
//
// signed in from two devices (`toile-presence`). Its home is `ambre-draw`
// because that is the render-primitives crate every app's canvas already
// depends on. It is seeded on the canonical `Id<User>` (not a raw `Uuid`), so the
// newtype is preserved end-to-end through presence. That is the sole reason
// `ambre-draw` takes a scoped `ambre-core` dependency (types only).
// ============================================================================

use ambre_core::id::{Id, User};
use ambre_core::{Bounded, Bounds, Refusal, Rgb};

use crate::Color;

/// A stable, well-separated presence colour for the given peer, seeded on their
/// canonical [`Id<User>`].
///
/// The id bytes are folded into a hue with FNV-1a, then mapped through HSL at a
/// fixed saturation / lightness. Every peer colour is vivid and legible against
/// both the light and dark board backgrounds. It also stays visually distinct
/// from its neighbours around the wheel.
pub fn peer_color(user_id: &Id<User>) -> Color {
    let [r, g, b] = hue_rgb(hue_of_user(user_id)).channels();
    Color::from_rgba8(r, g, b, 255)
}

/// The colour of [`peer_color`] as an [`Rgb`]. The DOM chrome draws a user
/// with it beside the canvas: an avatar, a member row.
pub fn peer_rgb(user_id: &Id<User>) -> Rgb {
    let [r, g, b, _] = peer_color(user_id).to_rgba8().to_u8_array();
    Rgb::from_channels(r, g, b)
}

/// The saturation of every peer colour, in `0.0..=1.0`.
const PEER_SATURATION: f32 = 0.62;

/// The lightness of every peer colour, in `0.0..=1.0`.
const PEER_LIGHTNESS: f32 = 0.55;

/// A whole hue on the colour wheel, in degrees below 360: the marker of a
/// [`Bounded<Hue>`], private to the peer colour.
enum Hue {}

impl Bounds for Hue {
    type Repr = u16;
    const MIN: u16 = 0;
    const MAX: u16 = 359;
    const NAME: &'static str = "Hue";
    const REFUSAL: Refusal = Refusal::new("a hue lies in 0..=359 degrees, not ", "");
}

/// The hue of a peer: the id seed bytes folded with FNV-1a, spread over the
/// full wheel.
fn hue_of_user(user_id: &Id<User>) -> Bounded<Hue> {
    let mut hash: u32 = 0x811c_9dc5;
    for &b in user_id.as_bytes() {
        hash ^= u32::from(b);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    // `hash % 360` is below 360, so it fits the bounds; the lowest hue
    // stands in for a value the remainder never gives.
    u16::try_from(hash % 360)
        .ok()
        .and_then(Bounded::new)
        .unwrap_or(Bounded::MIN)
}

/// The hue `hue` at the peer saturation and lightness, in 8-bit sRGB.
fn hue_rgb(hue: Bounded<Hue>) -> Rgb {
    let (s, l) = (PEER_SATURATION, PEER_LIGHTNESS);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = f32::from(hue.get()) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match hue.get() / 60 {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The DOM colour of a user is their canvas colour, without its alpha.
    #[test]
    fn the_dom_colour_is_the_canvas_colour() {
        let user = Id::<User>::from_bytes([7u8; 16]);
        let [r, g, b, _] = peer_color(&user).to_rgba8().to_u8_array();
        assert_eq!(peer_rgb(&user).channels(), [r, g, b]);
    }

    #[test]
    fn stable_and_pure() {
        let user = Id::<User>::from_bytes([7u8; 16]);
        assert_eq!(peer_color(&user), peer_color(&user));
    }

    #[test]
    fn distinct_seeds_usually_differ() {
        // Not a guarantee for every pair, but two arbitrary users should not
        // collapse to the same colour in the common case.
        let a = peer_color(&Id::<User>::from_bytes([1u8; 16]));
        let b = peer_color(&Id::<User>::from_bytes([2u8; 16]));
        assert_ne!(a, b);
    }

    /// A hue lies below 360 degrees, and the peer colour reads it as before:
    /// red at zero, green at 120, blue at 240.
    #[test]
    fn a_hue_lies_below_360_and_spans_the_wheel() {
        assert_eq!(Bounded::<Hue>::new(360), None);
        assert_eq!(Bounded::<Hue>::new(359), Some(Bounded::MAX));
        let channels = |degrees| Bounded::<Hue>::new(degrees).map(|hue| hue_rgb(hue).channels());
        assert_eq!(channels(0), Some([211, 69, 69]));
        assert_eq!(channels(120), Some([69, 211, 69]));
        assert_eq!(channels(240), Some([69, 69, 211]));
        assert!(hue_of_user(&Id::<User>::from_bytes([9u8; 16])).get() < 360);
    }

    #[test]
    fn opaque() {
        // Presence colours are always fully opaque; alpha is applied by the
        // caller per overlay element (halo vs cursor vs laser trail).
        let c = peer_color(&Id::<User>::from_bytes([42u8; 16]));
        assert_eq!(c.components[3], 1.0);
    }
}
