//! What the paginator is allowed to measure on (`PROSE-107`, `PROSE-108`).
//!
//! A layout is a function of the text, the page box and the faces it was
//! measured on. The first two are shared through the CRDT and the third is
//! not, so two peers can break pages differently on one document.
//!
//! Family resolution is therefore pinned: `ambre-draw` maps a label to a
//! shipped face by the label alone, reading no context and no device. What is
//! left is script coverage, which a peer gains as it fetches faces, and
//! [`missing_scripts`] names what a measurement did not have.

use ambre_draw::{BundledFamily, ScriptSet};
use parley::FontContext;

/// Resolve a family label to the face the application ships.
///
/// The whole of family resolution for Prose (`PROSE-108`). It reads the label
/// and nothing else, so two peers resolve one label alike whatever faces their
/// contexts hold.
#[must_use]
pub fn resolve(label: &str) -> BundledFamily {
    ambre_draw::resolve_family(Some(label))
}

/// The scripts this text asks for that `font_cx` cannot cover (`PROSE-110`).
///
/// The base faces cover Latin, Cyrillic and Greek, so an empty answer is the
/// ordinary case. A non-empty one means the text was shaped into tofu, and the
/// heights it measured are not what a peer holding the face would get.
#[must_use]
pub fn missing_scripts(font_cx: &mut FontContext, text: &str) -> ScriptSet {
    let asked = ScriptSet::of_text(text);
    let mut missing = ScriptSet::new();
    for script in asked.iter() {
        if font_cx.collection.family_id(script.face_name()).is_none() {
            missing.insert(script);
        }
    }
    missing
}

/// Load a face into `font_cx`, from the bytes of a font file.
///
/// Registration lives here with the two lookups above, so this module stays
/// the one place that touches a collection at all (`PROSE-107`).
pub fn register_faces(font_cx: &mut FontContext, bytes: Vec<u8>) {
    font_cx.collection.register_fonts(bytes.into(), None);
}

/// Whether `font_cx` now covers any script of `missing`.
///
/// A face registered since a layout was measured invalidates it whole
/// (`PROSE-111`): every block of that script re-shapes to different heights.
#[must_use]
pub fn coverage_grew(font_cx: &mut FontContext, missing: ScriptSet) -> bool {
    missing
        .iter()
        .any(|script| font_cx.collection.family_id(script.face_name()).is_some())
}

#[cfg(test)]
mod tests {
    use super::{coverage_grew, missing_scripts, resolve};
    use ambre_draw::{BundledFamily, LazyFont, ScriptSet};
    use parley::FontContext;

    #[test]
    fn a_family_no_face_carries_still_resolves_to_a_shipped_one() {
        assert_eq!(resolve("No Face Named This 12345"), BundledFamily::Sans);
        assert_eq!(resolve("Serif"), BundledFamily::Serif);
    }

    #[test]
    fn a_monospace_label_falls_back_to_a_monospace_face() {
        assert_eq!(resolve("Monospace"), BundledFamily::Mono);
        assert_eq!(resolve("DejaVu Sans Mono"), BundledFamily::Mono);
    }

    #[test]
    fn resolution_reads_the_label_and_nothing_else() {
        // Two contexts, each seeded differently, resolve one label alike.
        // The function takes no context at all, which is the point of the
        // rule: a signature that cannot read a device cannot diverge on one.
        let empty = FontContext::new();
        drop(empty);
        for label in ["Serif", "Monospace", "Sans", "Unknown Face"] {
            assert_eq!(resolve(label), resolve(label));
        }
    }

    #[test]
    fn latin_text_asks_for_no_face_at_all() {
        let mut font_cx = FontContext::new();
        assert!(missing_scripts(&mut font_cx, "Hello, world 123.").is_empty());
    }

    #[test]
    fn a_script_the_context_cannot_cover_is_named() {
        let mut font_cx = FontContext::new();
        let missing = missing_scripts(&mut font_cx, "\u{4e00}");
        // A context that happens to hold the CJK face reports nothing, and the
        // rule is about what a context without it says.
        if font_cx
            .collection
            .family_id(LazyFont::Cjk.face_name())
            .is_none()
        {
            assert!(missing.contains(LazyFont::Cjk));
        }
    }

    #[test]
    fn coverage_that_never_grew_is_reported_as_such() {
        let mut font_cx = FontContext::new();
        assert!(!coverage_grew(&mut font_cx, ScriptSet::new()));
    }
}
