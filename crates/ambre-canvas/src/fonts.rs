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
//      `fetch()` of its face (same origin, no CDN -- see spec sec.3).
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
