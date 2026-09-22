//! Family resolution: every label the document carries becomes a face the
//! shaper actually holds (`PROSE-034`).
//!
//! A label reaches here from `meta/defaults` or from a paragraph style, and
//! neither is checked against anything when it is written. An unmatched label
//! hands the shaper a layout with no glyphs in it. That measures as an empty
//! block and paginates as a blank page. So the label is resolved against the
//! loaded collection first, and an unmatched one falls back.

use parley::FontContext;
use parley::fontique::{FamilyId, GenericFamily};

/// The face a family label resolves to, and whether the label was found.
///
/// The flag is what a caller reports on: a document laid out on fallback faces
/// breaks its pages where another device would not. Nothing acts on it yet,
/// and the `Two peers read the same page number` Story is where it lands
/// (`ambre-roadmap.md` E5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedFamily {
    pub family: FamilyId,
    pub matched: bool,
}

/// Resolve a family label to a loaded face.
///
/// The label is tried as it is written, then as the generic families a
/// collection always answers. `None` only when the collection holds no face at
/// all, which is a context nothing seeded.
#[must_use]
pub fn resolve(font_cx: &mut FontContext, label: &str) -> Option<ResolvedFamily> {
    if let Some(family) = font_cx.collection.family_id(label) {
        return Some(ResolvedFamily {
            family,
            matched: true,
        });
    }
    let generic = generic_for(label);
    let guessed = font_cx.collection.generic_families(generic).next();
    let family = match guessed {
        Some(family) => family,
        None => font_cx
            .collection
            .generic_families(GenericFamily::SansSerif)
            .next()?,
    };
    Some(ResolvedFamily {
        family,
        matched: false,
    })
}

/// The generic family a label falls back to when no face carries its name.
///
/// A label the document wrote is a human name, so the only reading available
/// is its own spelling. It is a guess, and it is marked as one by the
/// `matched` flag the caller gets back.
fn generic_for(label: &str) -> GenericFamily {
    let lowered = label.to_ascii_lowercase();
    if lowered.contains("mono") || lowered.contains("code") {
        GenericFamily::Monospace
    } else if lowered.contains("serif") && !lowered.contains("sans") {
        GenericFamily::Serif
    } else {
        GenericFamily::SansSerif
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A context holding the system faces, which every generic family answers
    /// from. Nothing here registers a face of its own.
    fn context() -> FontContext {
        FontContext::new()
    }

    #[test]
    fn a_family_no_face_carries_still_resolves_to_a_loaded_one() {
        let mut font_cx = context();
        let seeded = font_cx
            .collection
            .generic_families(GenericFamily::SansSerif)
            .next()
            .is_some();
        let resolved = resolve(&mut font_cx, "No Face Named This 12345");
        // An empty collection is a context nothing seeded, and the only case
        // the function is allowed to answer nothing for.
        assert_eq!(resolved.is_some(), seeded);
        let Some(resolved) = resolved else {
            return;
        };
        assert!(!resolved.matched);
        assert!(font_cx.collection.family_name(resolved.family).is_some());
    }

    #[test]
    fn a_monospace_label_falls_back_to_a_monospace_face() {
        assert_eq!(generic_for("Absent Mono"), GenericFamily::Monospace);
        assert_eq!(generic_for("Absent Code Pro"), GenericFamily::Monospace);
        assert_eq!(generic_for("Absent Serif"), GenericFamily::Serif);
        assert_eq!(generic_for("Absent Sans Serif"), GenericFamily::SansSerif);
    }
}
