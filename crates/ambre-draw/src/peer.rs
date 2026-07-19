// ambre-draw/src/peer.rs
// ============================================================================
// The shared cross-app presence-colour algorithm.
//
// Every collaborator is painted in a stable colour derived purely from the
// seed bytes of their canonical `UserId`. Because the mapping is a pure
// function of those 16 bytes -- no per-app state, no random salt -- the same
// user keeps one colour across Prose, Trame and Toile, and a single colour
// even when signed in from two devices (`ambre-spec-toile-ui.md` §8). Its home
// is `ambre-draw` because that is the render-primitives crate every app's
// canvas already depends on. It is seeded on the canonical `UserId` (not a raw
// `Uuid`) so the newtype is preserved end-to-end through presence; this is the
// sole reason `ambre-draw` takes a scoped `ambre-core` dependency (types only).
// ============================================================================

use ambre_core::UserId;

use crate::Color;

/// A stable, well-separated presence colour for the given peer, seeded on their
/// canonical [`UserId`].
///
/// The id bytes are folded into a hue with FNV-1a and mapped through HSL at a
/// fixed saturation / lightness, so every peer colour is vivid and legible
/// against both the light and dark board backgrounds while staying visually
/// distinct from its neighbours around the wheel.
pub fn peer_color(user_id: &UserId) -> Color {
    // FNV-1a over the id seed bytes -> a well-mixed 32-bit hash.
    let mut hash: u32 = 0x811c_9dc5;
    for &b in user_id.as_bytes() {
        hash ^= b as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    // Hue spread across the full wheel; fixed S/L keeps every peer legible.
    let hue = (hash % 360) as f32;
    let (r, g, b) = hsl_to_rgb(hue, 0.62, 0.55);
    Color::from_rgba8(r, g, b, 255)
}

/// HSL (hue in degrees, saturation / lightness in `0.0..=1.0`) to 8-bit sRGB.
fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let to_u8 = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    (to_u8(r1), to_u8(g1), to_u8(b1))
}

#[cfg(test)]
mod tests {
    use super::*;

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
