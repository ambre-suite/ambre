// ambre-canvas/src/fonts.rs
// ============================================================================
// Lazy font loading (CANVAS P1).
//
// The base fonts fetched at bootstrap (Noto Sans + DejaVu) cover Latin,
// Latin-Extended, Cyrillic, and Greek. Scripts outside that -- CJK, Arabic,
// Devanagari -- are NOT in the startup bundle: their faces are large and most
// documents never use them. Instead they are fetched on demand the first time
// the renderer shapes a codepoint the base fonts cannot cover (a "tofu" box).
//
// Flow (UI thread, single-threaded):
//   1. `observe(text)` scans each shaped string for a codepoint belonging to a
//      lazy script. The first sighting of a script kicks off an async
//      `fetch()` of its face (same origin, no CDN -- `UI-THREAD-004`).
//   2. The fetch resolves on this same thread (spawn_local) and drops the bytes
//      into a shared inbox.
//   3. Each frame `drain_into(font_cx)` registers any arrived faces into the
//      shared `FontContext` and hands their bytes back. The caller drops the
//      stale (tofu) layouts and forces a repaint. It also ships the same bytes
//      to the Logic thread, which measures on a context of its own.
//
// A script is fetched at most once regardless of the outcome (success, HTTP
// failure, or still in flight): `requested` latches on the first sighting.
// ============================================================================

use std::cell::RefCell;
use std::rc::Rc;

use ambre_core::CountKind;

#[cfg(any(test, target_arch = "wasm32"))]
use ambre_core::{AssetPaths, StaticKey};
use ambre_core::{FontBytes, Payload};
use rustc_hash::FxHashSet;

// -----------------------------------------------------------------------------
// Base fonts (fetched eagerly at boot)
// -----------------------------------------------------------------------------

/// The base font faces registered at startup. Noto Sans covers regular, bold,
/// italic and bold-italic, with real faces selected by `Bounded<FontWeight>` /
/// `FontStyle`. The alternate DejaVu serif and monospace families serve the
/// formatting toolbar picker (`trame-cell-styles`). Together they cover Latin,
/// Latin-Extended, Cyrillic, and Greek; scripts outside that are fetched on
/// demand by [`LazyFontLoader`]. Same-origin URLs (no CDN, `UI-THREAD-004`); a
/// missing file logs a warning and leaves that face absent rather than failing
/// boot.
#[cfg(any(test, target_arch = "wasm32"))]
const BASE_FONTS: [StaticKey<AssetPaths>; 6] = [
    StaticKey::new("/fonts/NotoSans-Regular.ttf"),
    StaticKey::new("/fonts/NotoSans-Bold.ttf"),
    StaticKey::new("/fonts/NotoSans-Italic.ttf"),
    StaticKey::new("/fonts/NotoSans-BoldItalic.ttf"),
    StaticKey::new("/fonts/DejaVuSerif.ttf"),
    StaticKey::new("/fonts/DejaVuSansMono.ttf"),
];

/// Fetch the [`BASE_FONTS`] same-origin and return each face's bytes that
/// resolved. The UI thread builds its own context from them, and the Main
/// thread ships the same bytes to the Logic thread (`LogicCommand::LoadFonts`),
/// which cannot fetch. A face that fails to fetch is logged and omitted.
#[cfg(target_arch = "wasm32")]
pub async fn fetch_base_font_faces() -> Vec<Payload<FontBytes>> {
    let mut faces = Vec::new();
    for url in BASE_FONTS {
        // The log names the face by its file, the last segment of its URL.
        let path = url.as_str();
        let label = path.rsplit('/').next().unwrap_or(path);
        match crate::host::fetch_bytes(url).await {
            Some(bytes) => {
                tracing::info!("{label} loaded");
                faces.push(Payload::new(bytes));
            }
            None => tracing::warn!("Failed to fetch {url}"),
        }
    }
    faces
}

/// Build the UI thread's [`parley::FontContext`] with the [`BASE_FONTS`]
/// registered, on the one constructor [`font_context_from_bytes`].
#[cfg(target_arch = "wasm32")]
pub async fn font_context_from() -> parley::FontContext {
    font_context_from_bytes(fetch_base_font_faces().await)
}

/// Build a [`parley::FontContext`] holding the faces in `faces` and nothing else
/// (`CANVAS-069`).
///
/// The context is sealed, so no face of the machine is discoverable from it. A
/// codepoint the shipped faces miss draws as tofu here and on every peer alike.
/// Fetch-free, so a host test reaches the same constructor the browser runs.
pub fn font_context_from_bytes(
    faces: impl IntoIterator<Item = Payload<FontBytes>>,
) -> parley::FontContext {
    let mut font_cx = ambre_draw::sealed_font_context();
    for bytes in faces {
        ambre_draw::register_face(&mut font_cx, &ambre_draw::FontFace::new(bytes));
    }
    font_cx
}

/// A script the base faces do not cover, fetched on demand.
///
/// Defined in `ambre-draw` so the Logic thread reaches the same classification
/// without the graphics stack: Prose names the scripts a measurement was
/// missing (`PROSE-110`).
pub use ambre_draw::LazyFont;

/// The lazy scripts a loader has started a fetch for.
pub enum LazyFonts {}

impl ambre_core::NarrowCount for LazyFonts {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("LazyFonts");
}

/// The fetches of lazy faces that one thread has armed (`PDF-019`).
pub enum ArmedFetches {}

impl CountKind for ArmedFetches {
    type Repr = u64;
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("ArmedFetches");
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    /// The fetches armed on this thread. A host build has no fetch to spy on,
    /// so the count stands in for it.
    static ARMED: std::cell::Cell<ambre_core::Count<ArmedFetches>> =
        const { std::cell::Cell::new(ambre_core::Count::ZERO) };
}

/// The fetches of lazy faces armed on the calling thread so far, over every
/// loader (`PDF-019`). A host hook: an export runs on the thread that calls
/// it, so a test reads what that export armed, whatever cache it built.
#[cfg(not(target_arch = "wasm32"))]
pub fn fetches_armed_on_this_thread() -> ambre_core::Count<ArmedFetches> {
    ARMED.with(std::cell::Cell::get)
}

/// Font blobs fetched since the last drain, awaiting registration into the
/// `FontContext`. Shared with the in-flight fetch tasks (same thread).
#[derive(Default)]
struct Inbox {
    pending: Vec<Payload<FontBytes>>,
}

/// Tracks which lazy scripts have been requested and collects fetched faces.
///
/// Lives next to the `FontContext` on the UI thread (`!Send`, like every canvas
/// resource). Owned by [`TextCache`](crate::text_cache::TextCache), which feeds
/// it the strings it shapes.
pub struct LazyFontLoader {
    /// Scripts whose fetch has been started -- never started twice.
    requested: FxHashSet<LazyFont>,
    /// Faces delivered by resolved fetches, awaiting registration.
    inbox: Rc<RefCell<Inbox>>,
}

impl Default for LazyFontLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl LazyFontLoader {
    pub fn new() -> Self {
        Self {
            requested: FxHashSet::default(),
            inbox: Rc::new(RefCell::new(Inbox::default())),
        }
    }

    /// Scan `text` for a codepoint outside base-font coverage and start an async
    /// fetch for each newly-seen lazy script. Cheap on the common (all-covered)
    /// path -- a single range check per char and no allocation.
    pub fn observe(&mut self, text: &str) {
        for c in text.chars() {
            if let Some(font) = LazyFont::for_char(c) {
                // `insert` returns true only the first time: latch-and-fetch.
                if self.requested.insert(font) {
                    self.request(font);
                }
            }
        }
    }

    /// Register any fetched faces into `font_cx` and return their bytes. An
    /// empty return means nothing arrived. A non-empty one means coverage
    /// changed: the caller drops the stale layouts, which shaped as tofu, and
    /// forces a repaint so they re-shape with glyphs.
    ///
    /// The bytes come back because this thread is not the only one that
    /// measures on them. The Logic thread paginates Prose on a sealed context
    /// of its own, and cannot fetch. So the same face travels to it over the
    /// bus (`CANVAS-068`). One clone per lazy script, once, against a
    /// document that would otherwise paginate on tofu.
    #[must_use]
    pub fn drain_into(&mut self, font_cx: &mut parley::FontContext) -> Vec<Payload<FontBytes>> {
        let pending = std::mem::take(&mut self.inbox.borrow_mut().pending);
        for bytes in &pending {
            // Registered as an additional family, and wired as the fallback of
            // its scripts: a run in a base family reaches it (`CANVAS-089`).
            ambre_draw::register_face(font_cx, &ambre_draw::FontFace::new(bytes.clone()));
        }
        pending
    }

    /// Number of scripts a fetch has been started for (test/diagnostic hook).
    #[cfg(any(test, not(target_arch = "wasm32")))]
    pub fn requested_count(&self) -> ambre_core::Count<LazyFonts> {
        ambre_core::Count::of(self.requested.len())
    }

    /// Start the async, same-origin fetch of a lazy face and hand the bytes to
    /// the inbox on success. Runs on the UI thread (spawn_local); failures are
    /// logged and leave the tofu boxes in place.
    #[cfg(target_arch = "wasm32")]
    fn request(&self, font: LazyFont) {
        let inbox = self.inbox.clone();
        let url = font.asset_url();
        wasm_bindgen_futures::spawn_local(async move {
            match crate::host::fetch_bytes(url).await {
                Some(bytes) => {
                    tracing::info!("lazy font {url} loaded ({} bytes)", bytes.len());
                    inbox.borrow_mut().pending.push(Payload::new(bytes));
                }
                None => tracing::warn!("lazy font {url} fetch failed -- glyphs render as tofu"),
            }
        });
    }

    /// Native builds have no `fetch`; detection is still exercised (the request
    /// is latched and counted on its thread) but no bytes ever arrive. Keeps
    /// the host-side tests browser-free.
    #[cfg(not(target_arch = "wasm32"))]
    fn request(&self, font: LazyFont) {
        let _ = font.asset_url();
        ARMED.with(|armed| armed.set(armed.get().incremented()));
    }

    /// Stand in for a resolved fetch, so a test can drain what a browser would
    /// have delivered.
    #[cfg(test)]
    pub(crate) fn deliver(&self, bytes: Vec<u8>) {
        self.inbox.borrow_mut().pending.push(Payload::new(bytes));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin_and_symbols_never_trigger() {
        for c in "Hello, world! 123 -- Cyrillic: Привет, Greek: Ελληνικά".chars() {
            assert_eq!(LazyFont::for_char(c), None, "char {c:?} should be covered");
        }
    }

    #[test]
    fn scripts_classify() {
        assert_eq!(LazyFont::for_char('中'), Some(LazyFont::Cjk));
        assert_eq!(LazyFont::for_char('あ'), Some(LazyFont::Cjk)); // Hiragana
        assert_eq!(LazyFont::for_char('한'), Some(LazyFont::Cjk)); // Hangul
        assert_eq!(LazyFont::for_char('س'), Some(LazyFont::Arabic));
        assert_eq!(LazyFont::for_char('अ'), Some(LazyFont::Devanagari));
        assert_eq!(LazyFont::for_char('\u{1F44D}'), Some(LazyFont::Emoji)); // thumbs up
        assert_eq!(LazyFont::for_char('\u{2764}'), Some(LazyFont::Emoji)); // heavy heart
        assert_eq!(LazyFont::for_char('\u{1F1EB}'), Some(LazyFont::Emoji)); // regional F
        // Text arrows stay with the base fonts (monochrome), not the emoji face.
        assert_eq!(LazyFont::for_char('\u{2190}'), None); // leftwards arrow
    }

    #[test]
    fn observe_latches_each_script_once() {
        let mut loader = LazyFontLoader::new();
        loader.observe("plain ascii");
        assert_eq!(loader.requested_count().get(), 0);

        // A Han codepoint triggers exactly one request...
        loader.observe("mixed 中文 text");
        assert_eq!(loader.requested_count().get(), 1);
        // ...and re-observing the same script does not fetch again.
        loader.observe("more 漢字");
        assert_eq!(loader.requested_count().get(), 1);

        // A different script adds one more request.
        loader.observe("عربى");
        assert_eq!(loader.requested_count().get(), 2);
    }

    /// A drained face comes back to the caller, which owes it to the threads
    /// that keep a `FontContext` of their own (`CANVAS-073`).
    ///
    /// The Logic thread paginates Prose on a sealed context and cannot fetch.
    /// A face this thread keeps to itself leaves that context short, and the
    /// pages measure on tofu.
    #[test]
    fn a_drained_face_comes_back_for_the_other_threads() {
        let mut loader = LazyFontLoader::new();
        // Any blob will do: `register_fonts` ignores what it cannot parse,
        // and what this holds is the hand-over, not the parsing.
        let face = b"not a real face".to_vec();
        loader.deliver(face.clone());

        let mut font_cx = parley::FontContext::new();
        let drained = loader.drain_into(&mut font_cx);

        assert_eq!(drained, vec![face], "the bytes registered here travel on");
        assert!(
            loader.drain_into(&mut font_cx).is_empty(),
            "a face is handed over once"
        );
    }

    /// The UI thread's context holds the faces it was built from and nothing
    /// of the machine (`CANVAS-069`).
    ///
    /// The families below ship with common desktop distributions and none is
    /// one of Ambre's own faces. A default context finds them where they exist.
    #[test]
    fn a_built_context_discovers_no_face_of_the_machine() {
        let mut font_cx = super::font_context_from_bytes(Vec::new());
        for family in ["DejaVu Sans", "Liberation Sans", "FreeSans", "Arial"] {
            assert!(
                font_cx.collection.family_id(family).is_none(),
                "the UI context found `{family}` on the machine"
            );
        }
    }

    /// Every face, base or lazy, is fetched from a path on the page's own
    /// origin (`CANVAS-051`).
    ///
    /// A scheme, a host or a protocol-relative prefix would tell a third party
    /// which scripts a user reads.
    #[test]
    fn every_face_is_fetched_from_the_page_origin() {
        let base = BASE_FONTS.iter().copied();
        let lazy = LazyFont::ALL.iter().map(|font| font.asset_url());
        for url in base.chain(lazy) {
            let path = url.as_str();
            assert!(path.starts_with("/fonts/"), "`{url}` leaves the origin");
            assert!(!path.contains("//"), "`{url}` names a host");
        }
    }

    #[test]
    fn drain_is_empty_without_fetches() {
        // Native builds never populate the inbox, so a drain is always a no-op.
        let mut loader = LazyFontLoader::new();
        loader.observe("中文");
        let mut font_cx = parley::FontContext::new();
        assert!(loader.drain_into(&mut font_cx).is_empty());
    }
}
