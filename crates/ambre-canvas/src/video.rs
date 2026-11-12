// ambre-canvas/src/video.rs
// ============================================================================
// The received videos of a call, as the render runtime draws them
// (`ambre-spec-call-media.md section 3`, video on the canvas).
//
// The page plays each video receiver into a detached video element of its
// own, and hands the element here under the slot of its receiver. No element
// enters the page (`CALL-015`). A paint that draws a slot copies the current
// picture of its element into a GPU texture of the slot
// (`WgpuSurface::draw_video`). So no picture crosses WebAssembly memory, and
// the texture stays on the UI thread (`CANVAS-001`).
//
// The registry is a thread-local of the UI thread: the elements are page
// objects, and only that thread holds them.
// ============================================================================

use ambre_core::{Image, Px, Size};
use ambre_draw::Rect;

/// The box inside `dest` that a picture of `frame` fills with its aspect kept, centred on
/// the free axis. A picture of no size fills nothing.
pub fn contain(frame: Size<Px<Image>>, dest: Rect) -> Option<Rect> {
    let (width, height) = (f64::from(frame.width.get()), f64::from(frame.height.get()));
    if width <= 0.0 || height <= 0.0 || dest.width() <= 0.0 || dest.height() <= 0.0 {
        return None;
    }
    let scale = (dest.width() / width).min(dest.height() / height);
    let (w, h) = (width * scale, height * scale);
    let x0 = dest.x0 + (dest.width() - w) / 2.0;
    let y0 = dest.y0 + (dest.height() - h) / 2.0;
    Some(Rect::new(x0, y0, x0 + w, y0 + h))
}

#[cfg(target_arch = "wasm32")]
mod page {
    use std::cell::RefCell;

    use ambre_core::index::Pos;
    use ambre_core::{Image, Px, Size, VideoSlots};
    use wasm_bindgen::JsCast;

    /// The video of one slot: the detached element that plays it, and its track.
    struct Video {
        slot: Pos<VideoSlots>,
        element: web_sys::HtmlVideoElement,
        track: web_sys::MediaStreamTrack,
    }

    thread_local! {
        /// The video of each slot the page received, in the order of its slot.
        static VIDEOS: RefCell<Vec<Video>> = const { RefCell::new(Vec::new()) };
    }

    /// Plays `track` into a detached element and holds it under `slot`. A slot handed twice
    /// keeps the later track. The element is muted and never enters the page (`CALL-015`).
    pub fn show(slot: Pos<VideoSlots>, track: &web_sys::MediaStreamTrack) {
        let element = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.create_element("video").ok())
            .and_then(|e| e.dyn_into::<web_sys::HtmlVideoElement>().ok());
        let (Some(element), Ok(stream)) = (element, web_sys::MediaStream::new()) else {
            tracing::warn!("[Video] no element for the video of a slot");
            return;
        };
        stream.add_track(track);
        element.set_muted(true);
        element.set_autoplay(true);
        let _ = element.set_attribute("playsinline", "");
        element.set_src_object(Some(&stream));
        let _ = element.play();
        VIDEOS.with(|videos| {
            let mut videos = videos.borrow_mut();
            videos.retain(|v| v.slot != slot);
            videos.push(Video {
                slot,
                element,
                track: Clone::clone(track),
            });
            videos.sort_by_key(|v| v.slot);
        });
    }

    /// Drops the video of every slot: the call ended.
    pub fn forget_all() {
        let gone = VIDEOS.with(|videos| std::mem::take(&mut *videos.borrow_mut()));
        for video in gone {
            let _ = video.element.pause();
            video.element.set_src_object(None);
        }
    }

    /// Whether the video of `slot` holds a picture and its track carries media now.
    fn is_live(video: &Video) -> bool {
        // From `HAVE_CURRENT_DATA`, the element holds the picture of its current position.
        video.element.ready_state() >= web_sys::HtmlMediaElement::HAVE_CURRENT_DATA
            && video.element.video_width() > 0
            && !video.track.muted()
            && video.track.ready_state() == web_sys::MediaStreamTrackState::Live
    }

    /// The slots whose video holds a picture now, in the order of their slot.
    pub fn live() -> Vec<Pos<VideoSlots>> {
        VIDEOS.with(|videos| {
            videos
                .borrow()
                .iter()
                .filter(|v| is_live(v))
                .map(|v| v.slot)
                .collect()
        })
    }

    /// Whether the page holds a video under `slot`.
    pub(crate) fn holds(slot: Pos<VideoSlots>) -> bool {
        VIDEOS.with(|videos| videos.borrow().iter().any(|v| v.slot == slot))
    }

    /// The element of `slot` and the size of its picture, while it holds one.
    pub(crate) fn frame(
        slot: Pos<VideoSlots>,
    ) -> Option<(web_sys::HtmlVideoElement, Size<Px<Image>>)> {
        VIDEOS.with(|videos| {
            let videos = videos.borrow();
            let video = videos.iter().find(|v| v.slot == slot && is_live(v))?;
            let size = Size::new(
                Px::<Image>::new(video.element.video_width()),
                Px::<Image>::new(video.element.video_height()),
            );
            Some((video.element.clone(), size))
        })
    }
}

#[cfg(target_arch = "wasm32")]
pub use page::{forget_all, live, show};
#[cfg(target_arch = "wasm32")]
pub(crate) use page::{frame, holds};

/// The slots whose video holds a picture now. A host build holds no video.
#[cfg(not(target_arch = "wasm32"))]
pub fn live() -> Vec<ambre_core::index::Pos<ambre_core::VideoSlots>> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(w: u32, h: u32) -> Size<Px<Image>> {
        Size::new(Px::<Image>::new(w), Px::<Image>::new(h))
    }

    /// `CALL-015`: a picture keeps its aspect inside its tile, centred on the free axis, and
    /// fills the other.
    #[test]
    fn a_picture_keeps_its_aspect_inside_its_tile() {
        let tile = Rect::new(10.0, 20.0, 170.0, 110.0);
        // A 4:3 picture in a 16:9 tile fills its height.
        let fit = contain(size(640, 480), tile).expect("a box");
        assert!((fit.height() - 90.0).abs() < 1e-9, "{fit:?}");
        assert!((fit.width() - 120.0).abs() < 1e-9, "{fit:?}");
        assert!((fit.x0 - 30.0).abs() < 1e-9 && (fit.y0 - 20.0).abs() < 1e-9);
        // A 16:9 picture fills the whole tile.
        assert_eq!(contain(size(1280, 720), tile), Some(tile));
        // A tall picture fills the height and centres across.
        let tall = contain(size(90, 160), tile).expect("a box");
        assert!((tall.center().x - tile.center().x).abs() < 1e-9);
        assert!((tall.height() - 90.0).abs() < 1e-9);
        // No picture yet, or no room, fills nothing.
        assert_eq!(contain(size(0, 480), tile), None);
        assert_eq!(
            contain(size(640, 480), Rect::new(0.0, 0.0, 0.0, 10.0)),
            None
        );
    }
}
