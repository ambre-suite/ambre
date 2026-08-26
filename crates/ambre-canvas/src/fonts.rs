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
//      shared `FontContext` and reports whether coverage changed, so the caller
//      can drop the stale (tofu) layouts and force a repaint.
//
// A script is fetched at most once regardless of the outcome (success, HTTP
// failure, or still in flight): `requested` latches on the first sighting.
// ============================================================================

use std::cell::RefCell;
use std::rc::Rc;

use rustc_hash::FxHashSet;

// -----------------------------------------------------------------------------
// Base fonts (fetched eagerly at boot)
// -----------------------------------------------------------------------------

/// The base font faces registered at startup: Noto Sans (regular / bold /
/// italic / bold-italic, real faces selected by `FontWeight` / `FontStyle`) plus
/// the alternate DejaVu serif and monospace families for the formatting toolbar
/// picker (`trame-cell-styles`). Together they cover Latin, Latin-Extended,
/// Cyrillic, and Greek; scripts outside that are fetched on demand by
/// [`LazyFontLoader`]. Same-origin URLs (no CDN, `UI-THREAD-004`); a missing file
/// logs a warning and leaves that face absent rather than failing boot.
#[cfg(target_arch = "wasm32")]
const BASE_FONTS: [(&str, &str); 6] = [
    ("/fonts/NotoSans-Regular.ttf", "NotoSans-Regular.ttf"),
    ("/fonts/NotoSans-Bold.ttf", "NotoSans-Bold.ttf"),
    ("/fonts/NotoSans-Italic.ttf", "NotoSans-Italic.ttf"),
    ("/fonts/NotoSans-BoldItalic.ttf", "NotoSans-BoldItalic.ttf"),
    ("/fonts/DejaVuSerif.ttf", "DejaVuSerif.ttf"),
    ("/fonts/DejaVuSansMono.ttf", "DejaVuSansMono.ttf"),
];

/// Fetch the [`BASE_FONTS`] same-origin and register every face that resolves
/// into `font_cx`. Shared by the UI-thread GPU bootstrap and any other consumer
/// that needs a text-capable `FontContext` (e.g. the PDF export pass, whose
/// fresh context would otherwise have zero faces and emit no glyphs). Fetch runs
/// where a browser event loop exists; a face that fails to fetch is skipped.
#[cfg(target_arch = "wasm32")]
pub async fn load_base_fonts(font_cx: &mut parley::FontContext) {
    for (url, label) in BASE_FONTS {
        match crate::host::fetch_bytes(url).await {
            Some(bytes) => {
                font_cx.collection.register_fonts(bytes.into(), None);
                tracing::info!("{label} loaded");
            }
            None => tracing::warn!("Failed to fetch {url}"),
        }
    }
}

/// Fetch the [`BASE_FONTS`] same-origin and return each face's bytes that
/// resolved. Used by the Main thread to seed the Logic thread's PDF-export
/// `FontContext` (`LogicCommand::LoadFonts`): the Logic worker cannot fetch, so
/// the Main thread -- which has a browser event loop -- fetches on its behalf
/// and ships the bytes across. A face that fails to fetch is simply omitted.
#[cfg(target_arch = "wasm32")]
pub async fn fetch_base_font_faces() -> Vec<Vec<u8>> {
    let mut faces = Vec::new();
    for (url, _) in BASE_FONTS {
        if let Some(bytes) = crate::host::fetch_bytes(url).await {
            faces.push(bytes);
        }
    }
    faces
}

/// Build a fresh [`parley::FontContext`] with the [`BASE_FONTS`] registered.
#[cfg(target_arch = "wasm32")]
pub async fn font_context_from() -> parley::FontContext {
    let mut font_cx = parley::FontContext::new();
    load_base_fonts(&mut font_cx).await;
    font_cx
}

/// Build a [`parley::FontContext`] by registering each already-fetched face in
/// `faces`. Cross-platform and fetch-free, unlike [`font_context_from`]: the
/// Logic thread cannot fetch (it blocks on its inbound selector and has no
/// browser event loop), so it builds its PDF-export context from the base-font
/// bytes the Main thread fetched and delivered (`LogicCommand::LoadFonts`).
pub fn font_context_from_bytes(faces: impl IntoIterator<Item = Vec<u8>>) -> parley::FontContext {
    let mut font_cx = parley::FontContext::new();
    for bytes in faces {
        font_cx.collection.register_fonts(bytes.into(), None);
    }
    font_cx
}

/// A script whose coverage is absent from the base fonts and is fetched on
/// demand from the Ambre static server.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LazyFont {
    /// Han ideographs, Kana, Hangul, CJK symbols and full/half-width forms.
    Cjk,
    /// Arabic, its supplements, and the Arabic presentation forms.
    Arabic,
    /// Devanagari.
    Devanagari,
    /// Full-color emoji (Noto Color Emoji, CBDT bitmap build). Cross-app
    /// capability: reaction badges (Toile), message reactions (Murmure), and
    /// any inline emoji. Rasterized into the RGBA color atlas (see `atlas.rs`).
    Emoji,
}

impl LazyFont {
    /// Classify a codepoint into the lazy script that covers it, or `None` when
    /// the base fonts already cover it (Latin, Cyrillic, Greek, punctuation,
    /// symbols...). Only a `Some` codepoint can ever trigger a fetch.
    ///
    /// Ranges follow the Unicode blocks; they intentionally err on the side of
    /// the common blocks per script rather than exhaustively (a rare unassigned
    /// codepoint in a covered block simply maps to that script's face, which is
    /// harmless -- it still renders tofu, but from the right family).
    pub fn for_char(c: char) -> Option<LazyFont> {
        match c as u32 {
            // Arabic, Arabic Supplement, Arabic Extended-A, and the two
            // presentation-forms blocks.
            0x0600..=0x06FF
            | 0x0750..=0x077F
            | 0x08A0..=0x08FF
            | 0xFB50..=0xFDFF
            | 0xFE70..=0xFEFF => Some(LazyFont::Arabic),
            // Devanagari.
            0x0900..=0x097F => Some(LazyFont::Devanagari),
            // CJK: symbols & punctuation, Hiragana, Katakana (0x3000..0x30FF),
            // Han ext-A (0x3400..0x4DBF), Han unified (0x4E00..0x9FFF), Hangul
            // syllables (0xAC00..0xD7AF), CJK compatibility ideographs
            // (0xF900..0xFAFF), half/full-width forms (0xFF00..0xFFEF).
            0x3000..=0x30FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xAC00..=0xD7AF
            | 0xF900..=0xFAFF
            | 0xFF00..=0xFFEF => Some(LazyFont::Cjk),
            // Emoji: the supplementary-plane pictograph blocks (symbols &
            // pictographs, supplemental, extended-A), the enclosed/symbol
            // blocks incl. regional indicators (0x1F000..=0x1F2FF), plus the
            // BMP Miscellaneous Symbols + Dingbats (0x2600..=0x27BF) and the
            // Misc Symbols and Arrows subset that is emoji (0x2B00..=0x2BFF).
            // The BMP text-arrows block (0x2190..=0x21FF) is deliberately left
            // to the base fonts: those glyphs render as monochrome symbols, not
            // color emoji, so claiming them here would fetch the emoji face and
            // still tofu. ZWJ sequences and skin-tone modifiers are handled by
            // font shaping once the face is loaded.
            0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2B00..=0x2BFF => Some(LazyFont::Emoji),
            _ => None,
        }
    }

    /// Same-origin asset URL for this script's on-demand face. These files must
    /// be present under `crates/ambre-app/fonts/` (copied to `dist/fonts/` by
    /// Trunk) -- a missing file simply logs a warning and leaves tofu boxes.
    fn asset_url(self) -> &'static str {
        match self {
            LazyFont::Cjk => "/fonts/NotoSansSC-Regular.otf",
            LazyFont::Arabic => "/fonts/NotoSansArabic-Regular.ttf",
            LazyFont::Devanagari => "/fonts/NotoSansDevanagari-Regular.ttf",
            LazyFont::Emoji => "/fonts/NotoColorEmoji.ttf",
        }
    }
}

/// Font blobs fetched since the last drain, awaiting registration into the
/// `FontContext`. Shared with the in-flight fetch tasks (same thread).
#[derive(Default)]
struct Inbox {
    pending: Vec<Vec<u8>>,
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

    /// Register any fetched faces into `font_cx`. Returns `true` when at least
    /// one new face was registered, signalling the caller to drop stale layouts
    /// (which shaped as tofu) and force a repaint so they re-shape with glyphs.
    pub fn drain_into(&mut self, font_cx: &mut parley::FontContext) -> bool {
        let pending = std::mem::take(&mut self.inbox.borrow_mut().pending);
        if pending.is_empty() {
            return false;
        }
        for bytes in pending {
            // Registered as an additional family in the shared collection;
            // Parley's script-based fallback then selects it for runs the base
            // families cannot shape.
            font_cx.collection.register_fonts(bytes.into(), None);
        }
        true
    }

    /// Number of scripts a fetch has been started for (test/diagnostic hook).
    #[cfg(any(test, not(target_arch = "wasm32")))]
    pub fn requested_count(&self) -> usize {
        self.requested.len()
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
                    inbox.borrow_mut().pending.push(bytes);
                }
                None => tracing::warn!("lazy font {url} fetch failed -- glyphs render as tofu"),
            }
        });
    }

    /// Native builds have no `fetch`; detection is still exercised (the request
    /// is latched) but no bytes ever arrive. Keeps the host-side tests browser-free.
    #[cfg(not(target_arch = "wasm32"))]
    fn request(&self, font: LazyFont) {
        let _ = font.asset_url();
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
        assert_eq!(loader.requested_count(), 0);

        // A Han codepoint triggers exactly one request...
        loader.observe("mixed 中文 text");
        assert_eq!(loader.requested_count(), 1);
        // ...and re-observing the same script does not fetch again.
        loader.observe("more 漢字");
        assert_eq!(loader.requested_count(), 1);

        // A different script adds one more request.
        loader.observe("عربى");
        assert_eq!(loader.requested_count(), 2);
    }

    #[test]
    fn drain_is_empty_without_fetches() {
        // Native builds never populate the inbox, so a drain is always a no-op.
        let mut loader = LazyFontLoader::new();
        loader.observe("中文");
        let mut font_cx = parley::FontContext::new();
        assert!(!loader.drain_into(&mut font_cx));
    }
}
