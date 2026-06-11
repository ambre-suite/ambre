// ambre-app/src/route.rs
// ============================================================================
// URL routing (UI thread). Maps `window.location.pathname` to a `Route`:
//
//   /                  -> Landing   (document list + "new document")
//   /trame/<docid>     -> Doc { doc_type: Trame, document_id }
//   /prose/<docid>     -> Doc { doc_type: Prose, document_id }   (future)
//
// The document type lives in the path so the client knows which engine to boot
// before resolving the document. Navigation between routes is a full page load
// (set `location.href`): the wgpu canvas, Logic thread and Sync worker are
// created once per boot, so swapping the active document in a live canvas is out
// of scope for this iteration. Browser back/forward work natively as a result.
// ============================================================================

use ambre_bus::{DocType, DocumentId};

/// The page the URL resolves to.
#[derive(Debug, Clone)]
pub enum Route {
    /// Root: the document list / landing page.
    Landing,
    /// A specific document, with its type known from the path.
    Doc {
        doc_type: DocType,
        document_id: DocumentId,
    },
}

/// URL slug for a document type (lowercased name). Inverse of [`slug_to_type`].
fn type_slug(doc_type: DocType) -> &'static str {
    match doc_type {
        DocType::Prose => "prose",
        DocType::Trame => "trame",
        DocType::Toile => "toile",
        DocType::Murmure => "murmure",
    }
}

/// Parse a path slug back into a [`DocType`].
fn slug_to_type(slug: &str) -> Option<DocType> {
    match slug {
        "prose" => Some(DocType::Prose),
        "trame" => Some(DocType::Trame),
        "toile" => Some(DocType::Toile),
        "murmure" => Some(DocType::Murmure),
        _ => None,
    }
}

/// Resolve the current route from `window.location.pathname`.
///
/// Anything that does not parse cleanly (unknown type slug, malformed id) falls
/// back to [`Route::Landing`] rather than erroring.
pub fn current() -> Route {
    let Some(window) = web_sys::window() else {
        return Route::Landing;
    };
    let path = window.location().pathname().unwrap_or_default();
    parse(&path)
}

/// Pure path parser, split out for testability.
fn parse(path: &str) -> Route {
    let mut segments = path.split('/').filter(|s| !s.is_empty());
    match (segments.next(), segments.next()) {
        (Some(type_slug), Some(id_str)) => {
            match (slug_to_type(type_slug), DocumentId::parse_str(id_str)) {
                (Some(doc_type), Ok(document_id)) => Route::Doc {
                    doc_type,
                    document_id,
                },
                _ => Route::Landing,
            }
        }
        _ => Route::Landing,
    }
}

/// The canonical path for a document route (e.g. `/trame/<uuid>`).
pub fn doc_path(doc_type: DocType, document_id: DocumentId) -> String {
    format!("/{}/{}", type_slug(doc_type), document_id)
}

/// Navigate to a document (full page load). Used by the landing's "open" /
/// "new document" actions.
pub fn navigate_to_doc(doc_type: DocType, document_id: DocumentId) {
    if let Some(window) = web_sys::window() {
        let _ = window
            .location()
            .set_href(&doc_path(doc_type, document_id));
    }
}

/// Navigate back to the landing page (full page load).
pub fn navigate_home() {
    if let Some(window) = web_sys::window() {
        let _ = window.location().set_href("/");
    }
}
