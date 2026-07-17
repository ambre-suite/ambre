//! Atomic foreign-file import into [`ToileState`](super::ToileState)
//! (`toile-interop` §1/§5). An [`ImportBatch`](crate::logic::interop::ImportBatch)
//! produced by a pure converter is applied here as ONE undo group: every
//! `CreateElement`, style, group, connector and media seed shares a single
//! `doc.commit()` via the commit-free `*_inner` helpers (§2.E).
//!
//! Step 4 lands the media-map seeding half; the full `apply_import` pass (creates
//! -> styles -> groups -> connectors -> media -> one commit) arrives in step 5.

use loro::{LoroMap, LoroValue};

use crate::logic::interop::MediaBlob;

use super::{MEDIA, ToileState};

impl ToileState {
    /// Populate the `media` root map from a batch's plaintext blobs
    /// (`toile-logic §2.B`), **without** committing. Each blob's `hash` (the real
    /// blake2b, resolved Sync-side by the time this runs) keys a `{w, h, mime,
    /// thumb}` map. Dedup is content-addressed (§5): a hash already present is
    /// left untouched, so N elements sharing one image seed exactly one entry.
    /// Returns the count of NEW entries created (existing keys are skipped).
    ///
    /// Commit-free by design: the caller (`apply_import`) folds this into the one
    /// import undo group alongside the element creates.
    // The production caller (`apply_import`) lands in step 5; exercised by tests
    // meanwhile.
    #[allow(dead_code)]
    pub(super) fn seed_media(&mut self, blobs: &[MediaBlob]) -> usize {
        let media = self.doc.get_map(MEDIA);
        let mut created = 0;
        for blob in blobs {
            let key = blob.hash.as_str();
            if key.is_empty() || media.get(key).is_some() {
                // No real hash yet, or the entry already exists (dedup, §5).
                continue;
            }
            let Ok(entry) = media.insert_container(key, LoroMap::new()) else {
                continue;
            };
            let _ = entry.insert("w", blob.width as i64);
            let _ = entry.insert("h", blob.height as i64);
            let _ = entry.insert("mime", blob.mime.as_str());
            let _ = entry.insert("thumb", LoroValue::Binary(blob.thumb.clone().into()));
            created += 1;
        }
        created
    }

    /// Number of entries in the `media` map. Test / integration accessor.
    pub fn media_count(&self) -> usize {
        match self.doc.get_map(MEDIA).get_deep_value() {
            LoroValue::Map(m) => m.len(),
            _ => 0,
        }
    }

    /// The `(w, h, mime)` load-info of a media entry, if present. Test accessor.
    #[cfg(test)]
    pub(super) fn media_meta(&self, hash: &str) -> Option<(i64, i64, String)> {
        let LoroValue::Map(m) = self.doc.get_map(MEDIA).get_deep_value() else {
            return None;
        };
        let LoroValue::Map(entry) = m.get(hash)? else {
            return None;
        };
        let w = match entry.get("w")? {
            LoroValue::I64(v) => *v,
            _ => return None,
        };
        let h = match entry.get("h")? {
            LoroValue::I64(v) => *v,
            _ => return None,
        };
        let mime = match entry.get("mime")? {
            LoroValue::String(s) => s.to_string(),
            _ => return None,
        };
        Some((w, h, mime))
    }
}

#[cfg(test)]
mod tests {
    use crate::logic::interop::{ContentHash, MediaBlob};
    use crate::logic::state::ToileState;

    fn blob(hash: &str, w: u32, h: u32, mime: &str) -> MediaBlob {
        MediaBlob {
            hash: ContentHash(hash.to_owned()),
            plaintext: vec![0u8; 4],
            mime: mime.to_owned(),
            width: w,
            height: h,
            thumb: vec![1, 2, 3],
        }
    }

    #[test]
    fn seed_media_writes_load_info() {
        let mut st = ToileState::new();
        let n = st.seed_media(&[blob("abc", 640, 480, "image/png")]);
        assert_eq!(n, 1);
        assert_eq!(st.media_count(), 1);
        assert_eq!(
            st.media_meta("abc"),
            Some((640, 480, "image/png".to_owned()))
        );
    }

    #[test]
    fn seed_media_dedups_repeated_hashes() {
        let mut st = ToileState::new();
        // Two blobs with the same content hash collapse to one entry (§5 dedup).
        let n = st.seed_media(&[
            blob("dup", 10, 10, "image/png"),
            blob("dup", 10, 10, "image/png"),
        ]);
        assert_eq!(n, 1, "identical hashes seed exactly one entry");
        assert_eq!(st.media_count(), 1);
        // A later seed of the same hash is a no-op (already present).
        assert_eq!(st.seed_media(&[blob("dup", 10, 10, "image/png")]), 0);
        assert_eq!(st.media_count(), 1);
    }

    #[test]
    fn seed_media_skips_unresolved_hash() {
        let mut st = ToileState::new();
        // An empty hash means Sync has not returned the blake2b yet: never seed a
        // placeholder key collaborators could not resolve.
        let n = st.seed_media(&[blob("", 10, 10, "image/png")]);
        assert_eq!(n, 0);
        assert_eq!(st.media_count(), 0);
    }
}
