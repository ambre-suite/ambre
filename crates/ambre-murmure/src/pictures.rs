//! The decoded images a view previews, and the clock that plays them (the
//! Murmure attachments spec).
//!
//! The Logic thread decodes each image off the render thread
//! (`MURMURE-135`). The view keeps the frames here, bounded, and a paint
//! only picks the frame its clock reaches (`MURMURE-136`). A paint under
//! reduced motion picks the first, and nothing off screen is painted, so
//! nothing off screen plays (`MURMURE-130`, `MURMURE-131`).

use std::collections::HashMap;

use ambre_bus::{MediaRef, MediaType, Picture};
use ambre_draw::ImageKey;

/// The bytes the page holds decoded (`ambre-spec-budgets.md section 6`).
/// Past it, the pictures painted longest ago go, never one on screen.
pub const PICTURES_BYTES: usize = 16 * 1024 * 1024;

/// The types the Logic thread decodes for a preview. Any other file shows
/// as its chip alone.
const PREVIEWS: [&str; 3] = ["image/png", "image/jpeg", "image/gif"];

/// Whether a file of `media_type` shows a preview.
pub fn previews(media_type: &MediaType) -> bool {
    let named = media_type.0.trim().to_ascii_lowercase();
    PREVIEWS.contains(&named.as_str())
}

enum Held {
    /// Asked of the Logic thread, not answered yet.
    Asked,
    /// Decoded, and the paint that last drew it.
    Shown { picture: Picture, painted: u64 },
    /// Neither fetched nor decoded: its chip stands alone.
    Failed,
}

/// A region that paints pictures. The two regions share one shelf, and
/// what either drew last stays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Channel,
    Thread,
}

/// The pictures of the page, by the file they decode.
pub struct Pictures {
    held: HashMap<MediaRef, Held>,
    bytes: usize,
    cap: usize,
    /// The count of paints, which orders the pictures by their last one.
    paint: u64,
    /// The paint each screen last started: what it drew is on screen.
    screens: [u64; 2],
}

impl Default for Pictures {
    fn default() -> Self {
        Self::new(PICTURES_BYTES)
    }
}

impl Pictures {
    /// A shelf that holds `cap` bytes, past the pictures on screen.
    pub fn new(cap: usize) -> Self {
        Self {
            held: HashMap::new(),
            bytes: 0,
            cap,
            paint: 0,
            screens: [0; 2],
        }
    }

    /// A new paint of `screen` starts: what it draws is on screen, in place
    /// of what its last paint drew.
    pub fn begin_paint(&mut self, screen: Screen) {
        self.paint += 1;
        let [channel, thread] = &mut self.screens;
        match screen {
            Screen::Channel => *channel = self.paint,
            Screen::Thread => *thread = self.paint,
        }
    }

    /// `media_ref` is drawn by this paint. Returns `true` when it is to be
    /// asked of the Logic thread: never asked, or dropped since.
    pub fn want(&mut self, media_ref: &MediaRef) -> bool {
        match self.held.get_mut(media_ref) {
            Some(Held::Shown { painted, .. }) => {
                *painted = self.paint;
                false
            }
            Some(Held::Asked | Held::Failed) => false,
            None => {
                self.held.insert(media_ref.clone(), Held::Asked);
                true
            }
        }
    }

    /// The Logic thread's answer for `media_ref`. `None` leaves its chip
    /// alone. Returns `false` for a picture this shelf never asked.
    pub fn land(&mut self, media_ref: MediaRef, picture: Option<Picture>) -> bool {
        if !matches!(self.held.get(&media_ref), Some(Held::Asked)) {
            return false;
        }
        let held = match picture {
            Some(picture) => {
                self.bytes += picture.bytes();
                Held::Shown {
                    picture,
                    painted: self.paint,
                }
            }
            None => Held::Failed,
        };
        self.held.insert(media_ref, held);
        self.evict();
        true
    }

    /// The decoded picture of `media_ref`, if it is held.
    pub fn get(&self, media_ref: &MediaRef) -> Option<&Picture> {
        match self.held.get(media_ref) {
            Some(Held::Shown { picture, .. }) => Some(picture),
            _ => None,
        }
    }

    /// Whether `media_ref` could not be shown.
    pub fn failed(&self, media_ref: &MediaRef) -> bool {
        matches!(self.held.get(media_ref), Some(Held::Failed))
    }

    /// The bytes the shelf holds decoded.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Drop the pictures painted longest ago until the shelf fits its cap.
    /// A picture the last paint of a screen drew stays, past the cap if it
    /// must. It is on screen, and dropping it would fetch it again.
    fn evict(&mut self) {
        while self.bytes > self.cap {
            let coldest = self
                .held
                .iter()
                .filter_map(|(media_ref, held)| match held {
                    Held::Shown { painted, .. } if !self.screens.contains(painted) => {
                        Some((*painted, media_ref))
                    }
                    _ => None,
                })
                .min_by_key(|(painted, _)| *painted)
                .map(|(_, media_ref)| media_ref.clone());
            let Some(coldest) = coldest else { return };
            if let Some(Held::Shown { picture, .. }) = self.held.remove(&coldest) {
                self.bytes -= picture.bytes();
            }
        }
    }
}

/// The frame of `picture` its clock reaches at `now_ms`, and when the next
/// one is due. A still picture, or a paint under reduced motion, shows the
/// first frame, and none is due (`MURMURE-130`).
pub fn frame_at(picture: &Picture, now_ms: f64, still: bool) -> (usize, Option<f64>) {
    let total: u64 = picture.frames.iter().map(|f| u64::from(f.delay_ms)).sum();
    if still || picture.frames.len() < 2 || total == 0 {
        return (0, None);
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let at = now_ms.max(0.0) as u64 % total;
    let mut until = 0;
    for (i, frame) in picture.frames.iter().enumerate() {
        until += u64::from(frame.delay_ms);
        if at < until {
            #[allow(clippy::cast_precision_loss)]
            return (i, Some(now_ms + (until - at) as f64));
        }
    }
    (0, None)
}

/// The key of frame `index` of `media_ref`: one texture per frame, so a
/// frame played again is drawn from what the renderer already holds.
pub fn frame_key(media_ref: &MediaRef, index: usize) -> ImageKey {
    let ImageKey(base) = ImageKey::from_address(media_ref.as_str());
    ImageKey(base ^ (index as u64).rotate_right(16))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::PictureFrame;

    fn file(c: char) -> MediaRef {
        MediaRef::parse(&format!("{c}{}", "0".repeat(63))).expect("a content hash")
    }

    fn picture(delays: &[u32], bytes: usize) -> Picture {
        Picture {
            width: 1,
            height: 1,
            frames: delays
                .iter()
                .map(|&delay_ms| PictureFrame {
                    pixels: vec![0; bytes],
                    delay_ms,
                })
                .collect(),
        }
    }

    /// `MURMURE-136`: a paint picks the frame its clock reaches, and names
    /// when the next is due. It decodes nothing: the frames are held.
    #[test]
    fn the_clock_picks_the_frame_and_names_the_next() {
        let gif = picture(&[100, 50, 50], 4);
        assert_eq!(frame_at(&gif, 0.0, false), (0, Some(100.0)));
        assert_eq!(frame_at(&gif, 120.0, false), (1, Some(150.0)));
        assert_eq!(frame_at(&gif, 199.0, false), (2, Some(200.0)));
        assert_eq!(frame_at(&gif, 230.0, false), (0, Some(300.0)), "it loops");
    }

    /// `MURMURE-130`: under reduced motion an animation shows its first
    /// frame, and no frame is ever due. A still image is never due either.
    #[test]
    fn reduced_motion_holds_the_first_frame() {
        let gif = picture(&[100, 50], 4);
        assert_eq!(frame_at(&gif, 120.0, true), (0, None));
        assert_eq!(frame_at(&picture(&[0], 4), 120.0, false), (0, None));
    }

    /// Each frame has its own key, and each file its own.
    #[test]
    fn each_frame_has_its_own_key() {
        let (a, b) = (file('a'), file('b'));
        assert_ne!(frame_key(&a, 0), frame_key(&a, 1));
        assert_ne!(frame_key(&a, 0), frame_key(&b, 0));
        assert_eq!(frame_key(&a, 1), frame_key(&a, 1));
    }

    /// A picture is asked once, and lands only if it was asked. A failure
    /// is held, so it is not asked again.
    #[test]
    fn a_picture_is_asked_once_and_lands_once() {
        let mut shelf = Pictures::new(1024);
        shelf.begin_paint(Screen::Channel);
        let (a, b) = (file('a'), file('b'));
        assert!(shelf.want(&a));
        assert!(!shelf.want(&a), "asked already");
        assert!(
            !shelf.land(b.clone(), Some(picture(&[0], 4))),
            "never asked"
        );
        assert!(shelf.land(a.clone(), Some(picture(&[0], 4))));
        assert!(shelf.get(&a).is_some());
        assert!(shelf.want(&b));
        assert!(shelf.land(b.clone(), None));
        assert!(shelf.failed(&b));
        assert!(!shelf.want(&b), "a failure is not asked again");
    }

    /// Past its cap the shelf drops the picture painted longest ago, never
    /// one the last paint of a screen drew.
    #[test]
    fn past_the_cap_the_coldest_picture_goes_and_none_on_screen() {
        let mut shelf = Pictures::new(10);
        let (a, b, c, d) = (file('a'), file('b'), file('c'), file('d'));
        shelf.begin_paint(Screen::Channel);
        shelf.want(&a);
        shelf.land(a.clone(), Some(picture(&[0], 6)));
        shelf.begin_paint(Screen::Channel);
        shelf.want(&b);
        shelf.land(b.clone(), Some(picture(&[0], 6)));
        assert!(shelf.get(&a).is_none(), "the coldest went");
        assert_eq!(shelf.bytes(), 6);
        shelf.want(&c);
        shelf.land(c.clone(), Some(picture(&[0], 6)));
        assert!(
            shelf.get(&b).is_some() && shelf.get(&c).is_some(),
            "both on screen"
        );
        assert!(shelf.want(&a), "a dropped picture is asked again");

        shelf.begin_paint(Screen::Thread);
        shelf.want(&d);
        shelf.land(d.clone(), Some(picture(&[0], 6)));
        assert!(shelf.get(&b).is_some(), "the channel still shows it");
        assert!(shelf.get(&d).is_some());
    }

    /// Only the types the Logic thread decodes preview.
    #[test]
    fn only_a_decoded_type_previews() {
        let typed = |t: &str| previews(&MediaType(t.to_owned()));
        assert!(typed("image/gif") && typed(" IMAGE/PNG ") && typed("image/jpeg"));
        assert!(!typed("image/webp") && !typed("image/svg+xml") && !typed("text/html"));
    }
}
