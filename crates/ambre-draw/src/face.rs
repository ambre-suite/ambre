// ambre-draw/src/face.rs
// ============================================================================
// One copy of each font face per thread (`CANVAS-097`).
//
// A face arrives as a byte vector, and a thread hands it to several holders.
// The holders are a context per Prose document, the export context, and the
// list a document opened later is seeded from. A clone of the vector per holder costs the whole face
// each time, and the CJK and emoji faces weigh megabytes.
//
// So a thread admits each face once into a `FaceRegistry`, which wraps the bytes
// in the reference-counted blob fontique reads. Every holder takes a clone of
// that blob, which shares the bytes. A context registers the blob itself, so a
// collection reads the bytes the registry holds and copies none.
// ============================================================================

use ambre_core::{FontBytes, Payload};
use parley::fontique::{Blob, SourceKind};

/// One font file, shared by every holder on a thread (`CANVAS-097`).
///
/// A clone shares the bytes: it costs a reference count, never the face.
/// Two handles are the same copy when [`FontFace::same_copy`] says so, and a
/// face built twice from equal bytes is two copies.
#[derive(Clone)]
pub struct FontFace(Blob<u8>);

impl FontFace {
    /// Wrap the bytes of one font file. The bytes move into the shared blob,
    /// so the wrap copies nothing.
    #[must_use]
    pub fn new(face: Payload<FontBytes>) -> Self {
        Self(Blob::from(face.into_bytes()))
    }

    /// Whether `other` reads the very bytes this handle reads, rather than an
    /// equal copy of them.
    #[must_use]
    pub fn same_copy(&self, other: &FontFace) -> bool {
        std::ptr::eq(self.0.data(), other.0.data())
    }

    /// Register the face into `collection`, which takes a clone of the blob
    /// and so shares the bytes. Answers what the collection registered.
    pub(crate) fn register_into(
        &self,
        collection: &mut parley::fontique::Collection,
    ) -> Vec<(parley::fontique::FamilyId, Vec<parley::fontique::FontInfo>)> {
        collection.register_fonts(self.0.clone(), None)
    }
}

impl std::fmt::Debug for FontFace {
    /// The length alone: the bytes of a font say nothing a reader needs.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("FontFace")
            .field(&ambre_core::Count::<ambre_core::Octets>::of(self.0.len()))
            .finish()
    }
}

/// Every face one thread holds, each once (`CANVAS-097`).
///
/// The registry is the one place a face enters the thread. A holder that needs
/// a face takes a handle from [`FaceRegistry::faces`], and a context takes the
/// whole set through [`FaceRegistry::seed`]. A context built later, for a
/// document or an export, then reads the same bytes as the contexts before it.
#[derive(Debug, Default)]
pub struct FaceRegistry {
    faces: Vec<FontFace>,
}

impl FaceRegistry {
    /// An empty registry: the state of a thread before the first delivery.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Admit the bytes of one font file, and hand back the face when it is new.
    ///
    /// Bytes equal to a face already held answer `None`, and the registry keeps
    /// its own copy. A face delivered twice then lives once, and a context that
    /// already holds it registers it no second time.
    pub fn admit(&mut self, face: Payload<FontBytes>) -> Option<FontFace> {
        if self
            .faces
            .iter()
            .any(|held| held.0.data() == face.as_bytes())
        {
            return None;
        }
        let face = FontFace::new(face);
        self.faces.push(face.clone());
        Some(face)
    }

    /// The faces held, in the order they were admitted.
    #[must_use]
    pub fn faces(&self) -> &[FontFace] {
        &self.faces
    }

    /// Whether `face` is the copy this registry holds.
    #[must_use]
    pub fn holds(&self, face: &FontFace) -> bool {
        self.faces.iter().any(|held| held.same_copy(face))
    }

    /// Register every face held into `font_cx`, sharing the bytes.
    pub fn seed(&self, font_cx: &mut parley::FontContext) {
        for face in &self.faces {
            crate::text::register_face(font_cx, face);
        }
    }
}

/// Every face `font_cx` holds in memory, one handle per distinct copy.
///
/// The handles share the bytes the collection reads. A caller checks with
/// [`FaceRegistry::holds`] that a context reads no copy of its own.
#[must_use]
pub fn faces_of(font_cx: &mut parley::FontContext) -> Vec<FontFace> {
    let collection = &mut font_cx.collection;
    let names: Vec<String> = collection.family_names().map(str::to_owned).collect();
    let mut copies: Vec<FontFace> = Vec::new();
    for name in names {
        let Some(info) = collection
            .family_id(&name)
            .and_then(|id| collection.family(id))
        else {
            continue;
        };
        for font in info.fonts() {
            if let SourceKind::Memory(blob) = font.source().kind() {
                let face = FontFace(blob.clone());
                if !copies.iter().any(|held| held.same_copy(&face)) {
                    copies.push(face);
                }
            }
        }
    }
    copies
}

#[cfg(test)]
mod tests {
    use super::{FaceRegistry, FontFace, faces_of};
    use crate::text::{register_face, sealed_font_context};
    use ambre_core::{FontBytes, Payload};

    /// One shipped face, read off the tree.
    fn shipped(file: &str) -> Payload<FontBytes> {
        let path = format!("{}/../ambre-app/fonts/{file}", env!("CARGO_MANIFEST_DIR"));
        Payload::new(std::fs::read(path).expect("the workspace ships the face"))
    }

    #[test]
    fn a_context_seeded_from_the_registry_reads_the_registry_copy() {
        let mut registry = FaceRegistry::new();
        assert!(registry.admit(shipped("NotoSans-Regular.ttf")).is_some());
        assert!(registry.admit(shipped("DejaVuSerif.ttf")).is_some());
        let mut first = sealed_font_context();
        let mut second = sealed_font_context();
        registry.seed(&mut first);
        registry.seed(&mut second);
        for font_cx in [&mut first, &mut second] {
            let copies = faces_of(font_cx);
            assert_eq!(copies.len(), 2, "one copy per face");
            assert!(copies.iter().all(|face| registry.holds(face)));
        }
    }

    #[test]
    fn equal_bytes_delivered_twice_are_held_once() {
        let mut registry = FaceRegistry::new();
        assert!(registry.admit(shipped("DejaVuSerif.ttf")).is_some());
        assert!(registry.admit(shipped("DejaVuSerif.ttf")).is_none());
        assert_eq!(registry.faces().len(), 1);
    }

    #[test]
    fn a_face_built_apart_is_another_copy() {
        // The check the registry rests on tells a shared handle from an equal
        // copy, or a context holding its own copy would pass as shared.
        let registry = {
            let mut registry = FaceRegistry::new();
            registry.admit(shipped("DejaVuSerif.ttf"));
            registry
        };
        let apart = FontFace::new(shipped("DejaVuSerif.ttf"));
        assert!(!registry.holds(&apart));
        let mut font_cx = sealed_font_context();
        register_face(&mut font_cx, &apart);
        assert!(
            faces_of(&mut font_cx)
                .iter()
                .all(|face| !registry.holds(face))
        );
        assert!(
            registry
                .faces()
                .iter()
                .all(|face| face.clone().same_copy(face))
        );
    }
}
