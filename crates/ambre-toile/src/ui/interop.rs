// ambre-toile/src/ui/interop.rs
// ============================================================================
// What the board accepts from the file system (toile-import-export), pure. The
// shell owns the file dialog and the drop listener, and asks the board what it
// reads instead of holding a list of its own. The answer is host-tested rather
// than discovered at a drop.
// ============================================================================

use ambre_bus::app::toile::ElementCount;
use ambre_draw::Point;

/// The `accept` attribute of the import picker: the formats a converter reads
/// today (`TOILE-IO-020`). SVG is named twice on purpose -- browsers match the
/// extension on some platforms and the type on others.
pub const IMPORT_ACCEPT: &str = ".svg,image/svg+xml,image/png,image/jpeg,image/webp,image/gif";

/// Whether a converter reads this media type. A drop carries whatever type the
/// browser guessed, so this is the one gate before a file is read into memory.
pub fn accepts(mime: &ambre_core::MediaType) -> bool {
    let mime = mime.as_str();
    mime == "image/svg+xml" || mime.starts_with("image/")
}

/// A file the shell read for the board (`TOILE-IO-020`). `at` is the drop point
/// in logical pixels, or `None` when the picker started the import and there is
/// no point to speak of.
#[derive(Clone, Debug, PartialEq)]
pub struct DroppedFile {
    pub bytes: ambre_core::FileBytes,
    pub mime: ambre_core::MediaType,
    pub at: Option<Point>,
}

/// What an import tells the user (`TOILE-IO-022`). The count is the point: a
/// user who dropped a drawing wants to know it arrived. A lossy import says so
/// in the same breath, naming the first few constructs it dropped. A list nobody
/// reads at the corner of a screen is the rejected alternative.
pub fn import_message(imported: ElementCount, skipped: &[String]) -> String {
    let head = match imported.get() {
        1 => "Imported 1 element".to_owned(),
        n => format!("Imported {n} elements"),
    };
    if skipped.is_empty() {
        return head;
    }
    let named: Vec<&str> = skipped
        .iter()
        .take(SKIPPED_NAMED)
        .map(String::as_str)
        .collect();
    let rest = skipped.len() - named.len();
    let tail = if rest == 0 {
        named.join(", ")
    } else {
        format!("{}, and {rest} more", named.join(", "))
    };
    format!("{head}; dropped {tail}")
}

/// How many dropped constructs a toast names before it counts the rest.
const SKIPPED_NAMED: usize = 3;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_format_a_converter_reads_is_accepted() {
        for mime in [
            "image/svg+xml",
            "image/png",
            "image/jpeg",
            "image/webp",
            "image/gif",
        ] {
            assert!(accepts(&mime.into()), "{mime}");
        }
    }

    #[test]
    fn a_format_no_converter_reads_is_refused() {
        for mime in ["application/pdf", "text/csv", "", "application/json"] {
            assert!(!accepts(&mime.into()), "{mime}");
        }
    }

    /// The picker's filter and the drop gate must agree. A type the picker offers
    /// and the gate then refuses is a file the user chose and the board dropped.
    #[test]
    fn the_picker_offers_only_what_the_gate_accepts() {
        for entry in IMPORT_ACCEPT.split(',') {
            if !entry.starts_with('.') {
                assert!(accepts(&entry.into()), "{entry} is offered but refused");
            }
        }
    }
}

#[cfg(test)]
mod message_tests {
    use super::import_message;
    use ambre_bus::app::toile::ElementCount;

    #[test]
    fn a_clean_import_reports_its_count() {
        assert_eq!(
            import_message(ElementCount::new(1), &[]),
            "Imported 1 element"
        );
        assert_eq!(
            import_message(ElementCount::new(12), &[]),
            "Imported 12 elements"
        );
    }

    #[test]
    fn a_lossy_import_names_what_it_dropped() {
        let skipped = vec!["gradient".to_owned(), "filter".to_owned()];
        assert_eq!(
            import_message(ElementCount::new(3), &skipped),
            "Imported 3 elements; dropped gradient, filter"
        );
    }

    /// A long list is counted, not recited: a toast nobody can read is a
    /// construct dropped in silence by another means.
    #[test]
    fn a_long_list_of_dropped_constructs_is_counted() {
        let skipped: Vec<String> = (0..7).map(|i| format!("c{i}")).collect();
        assert_eq!(
            import_message(ElementCount::new(1), &skipped),
            "Imported 1 element; dropped c0, c1, c2, and 4 more"
        );
    }
}
