//! The decoded images a view previews, and the clock that plays them (the
//! Murmure attachments spec).
//!
//! The Logic thread decodes each image off the render thread
//! (`MURMURE-135`). The view keeps the frames here, bounded, and a paint
//! only picks the frame its clock reaches (`MURMURE-136`). A paint under
//! reduced motion picks the first, and nothing off screen is painted, so
//! nothing off screen plays (`MURMURE-130`, `MURMURE-131`).

use ambre_core::index::Axis;
use ambre_core::index::Pos;
use std::collections::HashMap;

use ambre_bus::{ContentHash, MediaType, Picture};
use ambre_core::{ByteSize, KnownMediaType, Seq, SeqKind};
use ambre_draw::ImageKey;

use crate::Region;
use ambre_core::{Instant, Ms, PageLoad, TimeSpan};

/// The bytes the page holds decoded (`ambre-spec-budgets.md section 6`).
/// Past it, the pictures painted longest ago go, never one on screen.
pub const PICTURES_MAX_SIZE: ByteSize = ByteSize::new(16 * 1024 * 1024);

/// The types the Logic thread decodes for a preview. Any other file shows
/// as its chip alone.
const PREVIEWS: [KnownMediaType; 3] = [
    KnownMediaType::Png,
    KnownMediaType::Jpeg,
    KnownMediaType::Gif,
];

/// Whether a file of `media_type` shows a preview.
pub fn previews(media_type: &MediaType) -> bool {
    KnownMediaType::of(media_type).is_some_and(|known| PREVIEWS.contains(&known))
}

/// A paint of the shelf's pictures, which orders the pictures by their last
/// one.
enum PaintNumber {}

impl SeqKind for PaintNumber {
    type IssueRight = ();
    const NAME: &'static str = "PaintNumber";
}

/// A frame of a decoded picture, counted from zero in play order.
#[derive(PartialEq, Eq)]
pub enum AnimationFrame {}

impl Axis for AnimationFrame {
    const NAME: &'static str = "Frame";
}

enum Held {
    /// Asked of the Logic thread, not answered yet.
    Asked,
    /// Decoded, and the paint that last drew it.
    Shown {
        picture: Picture,
        painted: Seq<PaintNumber>,
    },
    /// Neither fetched nor decoded: its chip stands alone.
    Failed,
}

/// The paint each screen last started: what it drew is on screen.
#[derive(Default)]
struct Screens {
    channel: Seq<PaintNumber>,
    thread: Seq<PaintNumber>,
}

impl Screens {
    /// Whether the last paint of a screen is `paint`.
    fn last_painted(&self, paint: Seq<PaintNumber>) -> bool {
        self.channel == paint || self.thread == paint
    }
}

/// The pictures of the page, by the file they decode.
pub struct Pictures {
    held: HashMap<ContentHash, Held>,
    bytes: ByteSize,
    cap: ByteSize,
    /// The latest paint, which orders the pictures by their last one.
    paint: Seq<PaintNumber>,
    screens: Screens,
}

impl Default for Pictures {
    fn default() -> Self {
        Self::new(PICTURES_MAX_SIZE)
    }
}

impl Pictures {
    /// A shelf that holds `cap` bytes, past the pictures on screen.
    pub fn new(cap: ByteSize) -> Self {
        Self {
            held: HashMap::new(),
            bytes: ByteSize::new(0),
            cap,
            paint: Seq::ZERO,
            screens: Screens::default(),
        }
    }

    /// A new paint of `screen` starts: what it draws is on screen, in place
    /// of what its last paint drew.
    pub fn begin_paint(&mut self, screen: Region) {
        self.paint = self.paint.next();
        match screen {
            Region::Channel => self.screens.channel = self.paint,
            Region::Thread => self.screens.thread = self.paint,
        }
    }

    /// `media_ref` is drawn by this paint. Returns `true` when it is to be
    /// asked of the Logic thread: never asked, or dropped since.
    pub fn want(&mut self, media_ref: &ContentHash) -> bool {
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
    pub fn land(&mut self, media_ref: ContentHash, picture: Option<Picture>) -> bool {
        if !matches!(self.held.get(&media_ref), Some(Held::Asked)) {
            return false;
        }
        let held = match picture {
            Some(picture) => {
                self.bytes = self.bytes.saturating_add(picture.bytes());
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
    pub fn get(&self, media_ref: &ContentHash) -> Option<&Picture> {
        match self.held.get(media_ref) {
            Some(Held::Shown { picture, .. }) => Some(picture),
            _ => None,
        }
    }

    /// Whether `media_ref` could not be shown.
    pub fn failed(&self, media_ref: &ContentHash) -> bool {
        matches!(self.held.get(media_ref), Some(Held::Failed))
    }

    /// The bytes the shelf holds decoded.
    pub fn bytes(&self) -> ByteSize {
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
                    Held::Shown { painted, .. } if !self.screens.last_painted(*painted) => {
                        Some((*painted, media_ref))
                    }
                    _ => None,
                })
                .min_by_key(|(painted, _)| *painted)
                .map(|(_, media_ref)| media_ref.clone());
            let Some(coldest) = coldest else { return };
            if let Some(Held::Shown { picture, .. }) = self.held.remove(&coldest) {
                self.bytes = self.bytes.saturating_sub(picture.bytes());
            }
        }
    }
}

/// The motion the user asks of the page: the full one, or reduced, where
/// each animated picture holds its first frame (`MURMURE-130`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Motion {
    #[default]
    Full,
    Reduced,
}

/// The frame of `picture` its clock reaches at `now`, and when the next
/// one is due. A still picture, or a paint under reduced motion, shows the
/// first frame, and none is due (`MURMURE-130`).
pub fn frame_at(
    picture: &Picture,
    now: Instant<PageLoad>,
    motion: Motion,
) -> (Pos<AnimationFrame>, Option<Instant<PageLoad>>) {
    let total: u64 = picture.frames.iter().map(|f| f.delay.get()).sum();
    if motion == Motion::Reduced || picture.frames.len() < 2 || total == 0 {
        return (Pos::<AnimationFrame>::ZERO, None);
    }
    let at = now.get().max(0.0) as u64 % total;
    let mut until = 0;
    for (i, frame) in picture.frames.iter().enumerate() {
        until += frame.delay.get();
        if at < until {
            let due = TimeSpan::<Ms>::new(until - at).to_fractional();
            return (Pos::<AnimationFrame>::from_usize(i), Some(now.after(due)));
        }
    }
    (Pos::<AnimationFrame>::ZERO, None)
}

/// The frame `index` of `picture`, `None` past its last.
pub fn frame(picture: &Picture, index: Pos<AnimationFrame>) -> Option<&ambre_bus::PictureFrame> {
    picture.frames.get(index.to_index()?)
}

/// The key of frame `index` of `media_ref`: one texture per frame, so a
/// frame played again is drawn from what the renderer already holds.
pub fn frame_key(media_ref: &ContentHash, index: Pos<AnimationFrame>) -> ImageKey {
    let base = ImageKey::from_address(media_ref).get();
    ImageKey::new(base ^ u64::from(index.get()).rotate_right(16))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::PictureFrame;
    use ambre_bus::{Instant, PageLoad};
    use ambre_core::Px;
    use ambre_core::{Ms, TimeSpan};

    fn file(c: char) -> ContentHash {
        ContentHash::parse(&format!("{c}{}", "0".repeat(63))).expect("a content hash")
    }

    fn picture(delays: &[u32], bytes: usize) -> Picture {
        Picture {
            size: ambre_core::Size::new(Px::new(1), Px::new(1)),
            frames: delays
                .iter()
                .map(|&delay| PictureFrame {
                    pixels: vec![0; bytes].into(),
                    delay: TimeSpan::<Ms>::new(u64::from(delay)),
                })
                .collect(),
        }
    }

    /// `MURMURE-136`: a paint picks the frame its clock reaches, and names
    /// when the next is due. It decodes nothing: the frames are held.
    #[test]
    fn the_clock_picks_the_frame_and_names_the_next() {
        let gif = picture(&[100, 50, 50], 4);
        assert_eq!(
            frame_at(&gif, Instant::<PageLoad>::new(0.0), Motion::Full),
            (
                Pos::<AnimationFrame>::new(0),
                Some(Instant::<PageLoad>::new(100.0))
            )
        );
        assert_eq!(
            frame_at(&gif, Instant::<PageLoad>::new(120.0), Motion::Full),
            (
                Pos::<AnimationFrame>::new(1),
                Some(Instant::<PageLoad>::new(150.0))
            )
        );
        assert_eq!(
            frame_at(&gif, Instant::<PageLoad>::new(199.0), Motion::Full),
            (
                Pos::<AnimationFrame>::new(2),
                Some(Instant::<PageLoad>::new(200.0))
            )
        );
        assert_eq!(
            frame_at(&gif, Instant::<PageLoad>::new(230.0), Motion::Full),
            (
                Pos::<AnimationFrame>::new(0),
                Some(Instant::<PageLoad>::new(300.0))
            ),
            "it loops"
        );
    }

    /// `MURMURE-130`: under reduced motion an animation shows its first
    /// frame, and no frame is ever due. A still image is never due either.
    #[test]
    fn reduced_motion_holds_the_first_frame() {
        let gif = picture(&[100, 50], 4);
        assert_eq!(
            frame_at(&gif, Instant::<PageLoad>::new(120.0), Motion::Reduced),
            (Pos::<AnimationFrame>::ZERO, None)
        );
        assert_eq!(
            frame_at(
                &picture(&[0], 4),
                Instant::<PageLoad>::new(120.0),
                Motion::Full
            ),
            (Pos::<AnimationFrame>::ZERO, None)
        );
    }

    /// Each frame has its own key, and each file its own.
    #[test]
    fn each_frame_has_its_own_key() {
        let (a, b) = (file('a'), file('b'));
        let (first, second) = (Pos::<AnimationFrame>::new(0), Pos::<AnimationFrame>::new(1));
        assert_ne!(frame_key(&a, first), frame_key(&a, second));
        assert_ne!(frame_key(&a, first), frame_key(&b, first));
        assert_eq!(frame_key(&a, second), frame_key(&a, second));
    }

    /// A picture is asked once, and lands only if it was asked. A failure
    /// is held, so it is not asked again.
    #[test]
    fn a_picture_is_asked_once_and_lands_once() {
        let mut shelf = Pictures::new(ByteSize::new(1024));
        shelf.begin_paint(Region::Channel);
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
        let mut shelf = Pictures::new(ByteSize::new(10));
        let (a, b, c, d) = (file('a'), file('b'), file('c'), file('d'));
        shelf.begin_paint(Region::Channel);
        shelf.want(&a);
        shelf.land(a.clone(), Some(picture(&[0], 6)));
        shelf.begin_paint(Region::Channel);
        shelf.want(&b);
        shelf.land(b.clone(), Some(picture(&[0], 6)));
        assert!(shelf.get(&a).is_none(), "the coldest went");
        assert_eq!(shelf.bytes(), ByteSize::new(6));
        shelf.want(&c);
        shelf.land(c.clone(), Some(picture(&[0], 6)));
        assert!(
            shelf.get(&b).is_some() && shelf.get(&c).is_some(),
            "both on screen"
        );
        assert!(shelf.want(&a), "a dropped picture is asked again");

        shelf.begin_paint(Region::Thread);
        shelf.want(&d);
        shelf.land(d.clone(), Some(picture(&[0], 6)));
        assert!(shelf.get(&b).is_some(), "the channel still shows it");
        assert!(shelf.get(&d).is_some());
    }

    /// Only the types the Logic thread decodes preview.
    #[test]
    fn only_a_decoded_type_previews() {
        let typed = |t: &str| previews(&MediaType::new(t.to_owned()));
        assert!(typed("image/gif") && typed(" IMAGE/PNG ") && typed("image/jpeg"));
        assert!(!typed("image/webp") && !typed("image/svg+xml") && !typed("text/html"));
    }
}
