// ambre-app/src/route.rs
// ============================================================================
// URL routing (UI thread). Maps `window.location.pathname` to a `Route`:
//
//   /                  -> Landing   (document list + "new document")
//   /trame/<docid>     -> Doc { doc_type: Trame, document_id }
//   /prose/<docid>     -> Doc { doc_type: Prose, document_id }   (future)
//
// The document type lives in the path so the client knows which engine to boot
// before resolving the document. Navigation between routes is a CLIENT pushState
// (see `lib.rs`, `leptos_router`): the wgpu canvas, Logic thread and Sync worker
// are created once and the active document is swapped in place, so the worker's
// in-memory session keys survive a route change. This module is reduced to the
// pure path<->`Route` mapping reused by the router shell.
// ============================================================================

use ambre_bus::{DocType, DocumentId};

/// The page the URL resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// Root: the document list / landing page.
    Landing,
    /// A specific document, with its type known from the path.
    Doc {
        doc_type: DocType,
        document_id: DocumentId,
    },
}

impl Route {
    /// The routed document, if this is a document route.
    pub fn doc(&self) -> Option<(DocType, DocumentId)> {
        match self {
            Route::Doc {
                doc_type,
                document_id,
            } => Some((*doc_type, *document_id)),
            Route::Landing => None,
        }
    }
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
pub fn parse(path: &str) -> Route {
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
