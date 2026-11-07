//! The document locale register, `meta.locale`, as Prose, Trame and Toile
//! store it (`ambre-spec-locale.md` section 3).
//!
//! One key of the root `meta` map holds one tag. The map is last-writer-wins
//! per key, so two peers that set the locale at once converge on one tag
//! (`LOCALE-004`). The three applications read and write it here, so the
//! spelling and the reading of a malformed value cannot drift between them.

use ambre_core::DocumentLocale;
use ambre_core::crdt::FoundationRoot;
use ambre_core::schema::MetaKey;

use crate::Fields;

/// The `meta` map of `doc`, typed by the foundation keys.
fn meta(doc: &loro::LoroDoc) -> Fields<MetaKey> {
    Fields::new(doc.get_map(FoundationRoot::Meta.as_str()))
}

/// The locale `doc` carries. An absent register, a value that is not text and
/// a malformed tag all read as no language (`LOCALE-002`).
#[must_use]
pub fn read_locale(doc: &loro::LoroDoc) -> DocumentLocale {
    match meta(doc).value(MetaKey::Locale) {
        Some(loro::LoroValue::String(text)) => DocumentLocale::read(Some(text.as_str())),
        _ => DocumentLocale::none(),
    }
}

/// Writes `locale` into the register of `doc`, without a commit: the caller
/// commits, so the write joins the caller's delta. No language deletes the key,
/// so a document that never named one and a document that cleared it store the
/// same thing.
///
/// # Errors
///
/// The error Loro returns for the write.
pub fn write_locale(doc: &loro::LoroDoc, locale: &DocumentLocale) -> loro::LoroResult<()> {
    let meta = meta(doc);
    match locale.tag() {
        Some(tag) => meta.insert(MetaKey::Locale, tag.as_str()),
        None if meta.contains(MetaKey::Locale) => meta.delete(MetaKey::Locale),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(id: u64) -> loro::LoroDoc {
        let doc = loro::LoroDoc::new();
        doc.set_peer_id(id).unwrap();
        doc
    }

    fn locale(text: &str) -> DocumentLocale {
        DocumentLocale::parse(text).unwrap()
    }

    fn exchange(a: &loro::LoroDoc, b: &loro::LoroDoc) {
        let to_b = a.export(loro::ExportMode::all_updates()).unwrap();
        let to_a = b.export(loro::ExportMode::all_updates()).unwrap();
        b.import(&to_b).unwrap();
        a.import(&to_a).unwrap();
    }

    #[test]
    fn a_written_locale_reads_back_and_none_clears_it() {
        let doc = peer(1);
        assert_eq!(read_locale(&doc), DocumentLocale::none());
        write_locale(&doc, &locale("fr-CA")).unwrap();
        doc.commit();
        assert_eq!(read_locale(&doc), locale("fr-CA"));
        write_locale(&doc, &DocumentLocale::none()).unwrap();
        doc.commit();
        assert_eq!(read_locale(&doc), DocumentLocale::none());
    }

    /// A peer may store any value under the key. Text that is no tag, and a
    /// value that is not text, read as no language.
    #[test]
    fn a_malformed_register_reads_as_no_language() {
        let doc = peer(1);
        let map = doc.get_map(FoundationRoot::Meta.as_str());
        map.insert(MetaKey::Locale.as_str(), "fr_CA").unwrap();
        assert_eq!(read_locale(&doc), DocumentLocale::none());
        map.insert(MetaKey::Locale.as_str(), 7_i64).unwrap();
        assert_eq!(read_locale(&doc), DocumentLocale::none());
    }

    /// Two peers set the locale at once and converge on one of the two tags.
    /// A write made after the other was seen stands on both.
    #[test]
    fn two_peers_setting_the_locale_converge_on_the_later_write() {
        let (a, b) = (peer(1), peer(2));
        write_locale(&a, &locale("fr")).unwrap();
        a.commit();
        write_locale(&b, &locale("de")).unwrap();
        b.commit();
        exchange(&a, &b);
        let merged = read_locale(&a);
        assert_eq!(merged, read_locale(&b));
        assert!(merged == locale("fr") || merged == locale("de"));

        write_locale(&a, &locale("it")).unwrap();
        a.commit();
        exchange(&a, &b);
        assert_eq!(read_locale(&a), locale("it"));
        assert_eq!(read_locale(&b), locale("it"));
    }
}
