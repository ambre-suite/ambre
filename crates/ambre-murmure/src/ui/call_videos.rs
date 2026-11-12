//! The video region of a call in the channel region (the Murmure calls spec, section 3, the
//! video region).
//!
//! While the page receives a video of its call, the channel region gives a band to the videos,
//! above its composer. Each video takes a tile, in the order of its slot (`MURMURE-620`). With no
//! video, the band takes no height (`MURMURE-621`). This module decides the band and the tiles.
//! The render runtime draws the picture of each slot inside its tile (`CALL-015`).

use ambre_core::index::Pos;
use ambre_core::{Px, Size, VideoSlots, View};
use ambre_draw::{DrawSurface, Rect};

use crate::ui::paint::view_rect;

/// The height the band takes in a tall region.
pub const VIDEO_BAND: Px<View> = Px::<View>::new(180.0);

/// The share of the height of the region the band takes at most, so a short window keeps a list.
const BAND_SHARE: f32 = 0.4;

/// The gap around each tile.
const TILE_GAP: Px<View> = Px::<View>::new(8.0);

/// The width of a tile over its height: the aspect of a camera.
const TILE_ASPECT: f32 = 16.0 / 9.0;

/// The height of the band in a region of `size` that shows the videos of `slots`.
pub fn band_height(size: Size<Px<View>>, slots: &[Pos<VideoSlots>]) -> Px<View> {
    if slots.is_empty() {
        return Px::<View>::ZERO;
    }
    VIDEO_BAND.min(size.height * BAND_SHARE)
}

/// The tile of the video of each of `slots`, in the order given, in the band `at`. The tiles
/// share one row, each at the aspect of a camera, and the row stays inside the width. The row is
/// centred.
pub fn tiles(at: Band, slots: &[Pos<VideoSlots>]) -> Vec<Rect> {
    let Band {
        top,
        height: band,
        width,
    } = at;
    let Ok(count) = u16::try_from(slots.len()) else {
        return Vec::new();
    };
    if count == 0 {
        return Vec::new();
    }
    let n = f32::from(count);
    let gap = TILE_GAP.get();
    let free = band.get() - 2.0 * gap;
    let across = (width.get() - (n + 1.0) * gap) / n;
    let tile_width = (free * TILE_ASPECT).min(across);
    let tile_height = tile_width / TILE_ASPECT;
    if tile_width <= 0.0 || tile_height <= 0.0 {
        return Vec::new();
    }
    let row = n * tile_width + (n - 1.0) * gap;
    let left = (width.get() - row) / 2.0;
    let y = top.get() + (band.get() - tile_height) / 2.0;
    (0..count)
        .map(|i| {
            let x = left + f32::from(i) * (tile_width + gap);
            view_rect(
                Px::<View>::new(x),
                Px::<View>::new(y),
                Px::<View>::new(x + tile_width),
                Px::<View>::new(y + tile_height),
            )
        })
        .collect()
}

/// Paints the band `at`: its backdrop, then the backdrop of the tile of each video of `slots`.
/// The render runtime draws the picture of each slot over its tile (`CALL-015`). A video is
/// screen state, which no primitive of the drawing surface takes (`CANVAS-056`).
pub fn paint<S: DrawSurface>(
    surface: &mut S,
    slots: &[Pos<VideoSlots>],
    at: Band,
    palette: &ambre_canvas::Palette,
) {
    if slots.is_empty() || at.height <= Px::<View>::ZERO {
        return;
    }
    surface.fill_rect(
        view_rect(Px::<View>::ZERO, at.top, at.width, at.top + at.height),
        palette.header_bg,
    );
    for tile in tiles(at, slots) {
        // A tile shows its backdrop until the picture of its slot arrives.
        surface.fill_rect(tile, palette.content_pending);
    }
}

/// The video of each of `slots` in its tile of the band `at`, in the order given.
pub fn placed(at: Band, slots: &[Pos<VideoSlots>]) -> Vec<(Pos<VideoSlots>, Rect)> {
    slots.iter().copied().zip(tiles(at, slots)).collect()
}

/// Where the band lies in its region.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Band {
    /// The top of the band.
    pub top: Px<View>,
    /// The height of the band.
    pub height: Px<View>,
    /// The width of the region.
    pub width: Px<View>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_draw::testing::{DrawOp, RecordingSurface};

    fn px(v: f32) -> Px<View> {
        Px::<View>::new(v)
    }

    /// The first `count` slots.
    fn slots(count: u32) -> Vec<Pos<VideoSlots>> {
        (0..count).map(Pos::<VideoSlots>::new).collect()
    }

    /// A band at `top`, `height` tall, in a region `width` wide.
    fn band(top: f32, height: f32, width: f32) -> Band {
        Band {
            top: px(top),
            height: px(height),
            width: px(width),
        }
    }

    /// `MURMURE-621`: with no video the band takes no height, and it never takes more than its
    /// share of a short region.
    #[test]
    fn no_video_takes_no_band_and_a_short_region_keeps_its_list() {
        let tall = Size::new(px(800.0), px(900.0));
        assert_eq!(band_height(tall, &[]), Px::<View>::ZERO);
        assert_eq!(band_height(tall, &slots(1)), VIDEO_BAND);
        let short = Size::new(px(800.0), px(300.0));
        assert_eq!(band_height(short, &slots(2)), px(120.0));
        assert!(tiles(band(0.0, 180.0, 800.0), &[]).is_empty());
    }

    /// `MURMURE-620`: each video takes a tile of its own in one row, in the order given. Each
    /// tile keeps the aspect of a camera, inside the band and the width.
    #[test]
    fn each_video_takes_a_tile_in_one_row_inside_the_band() {
        let at = band(500.0, 180.0, 800.0);
        for count in 1..=9 {
            let row = tiles(at, &slots(count));
            assert_eq!(row.len(), slots(count).len());
            for pair in row.windows(2) {
                let [a, b] = pair else { continue };
                assert!(a.x1 < b.x0, "the tiles overlap: {a:?} {b:?}");
                assert!((a.y0 - b.y0).abs() < 1e-6, "one row");
            }
            for tile in &row {
                assert!(tile.x0 >= 0.0 && tile.x1 <= 800.0, "{tile:?}");
                assert!(tile.y0 >= 500.0 && tile.y1 <= 680.0, "{tile:?}");
                let aspect = tile.width() / tile.height();
                assert!((aspect - f64::from(TILE_ASPECT)).abs() < 1e-3, "{aspect}");
            }
        }
        // One video fills the height of the band, less its gaps, and sits centred.
        let one = tiles(at, &slots(1));
        let [tile] = one.as_slice() else {
            panic!("one tile")
        };
        assert!((tile.height() - 164.0).abs() < 1e-3, "{tile:?}");
        assert!((tile.center().x - 400.0).abs() < 1e-3, "{tile:?}");
    }

    /// `MURMURE-620`: the paint draws the backdrop of the band, then one of each tile, and
    /// each slot keeps its tile in the order given. No video paints no band.
    #[test]
    fn the_band_places_each_slot_in_its_tile() {
        let palette = ambre_canvas::Theme::Light.palette();
        let slots = [Pos::<VideoSlots>::new(0), Pos::<VideoSlots>::new(3)];
        let at = band(400.0, 180.0, 800.0);
        let row = tiles(at, &slots);
        assert_eq!(
            placed(at, &slots),
            vec![(slots[0], row[0]), (slots[1], row[1])]
        );
        let mut surface = RecordingSurface::new();
        paint(&mut surface, &slots, at, palette);
        let fills: Vec<Rect> = surface
            .ops()
            .iter()
            .filter_map(|op| match op {
                DrawOp::FillRect { rect, .. } => Some(*rect),
                _ => None,
            })
            .collect();
        assert_eq!(
            fills,
            vec![Rect::new(0.0, 400.0, 800.0, 580.0), row[0], row[1]]
        );
        let mut none = RecordingSurface::new();
        paint(&mut none, &[], at, palette);
        assert!(none.ops().is_empty(), "no video paints no band");
        assert!(placed(at, &[]).is_empty());
    }
}
