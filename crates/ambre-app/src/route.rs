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

/// Whether the render kind of `doc_type` is the Toile canvas view. The booted
/// `AppView` is a binary split today (`lib.rs`, `boot_is_toile`): the Toile
/// vector board vs. everything else (the Trame grid). Prose/Murmure have no view
/// yet, so they fall on the non-Toile side like Trame.
fn is_toile_kind(doc_type: DocType) -> bool {
    matches!(doc_type, DocType::Toile)
}

/// Whether opening `target` from a page booted with `boot_is_toile` requires a
/// **full page reload** rather than an in-place `AppView::set_document` swap.
///
/// The booted canvas view is fixed per page load: `set_document` can swap the
/// document id **within the same app kind**, but cannot switch kinds
/// (Trame <-> Toile). So a target whose render kind differs from the booted kind
/// must hard-navigate (`location.set_href`) to re-boot the right `AppView`;
/// a same-kind target uses a router pushState that keeps the Sync worker/session
/// (`ambre-spec-chrome.md 2.G` "Open").
pub fn needs_reload(target: DocType, boot_is_toile: bool) -> bool {
    is_toile_kind(target) != boot_is_toile
}

// ── Tests (wasm tier) ───────────────────────────────────────────────────────
// The crate is `#![cfg(target_arch = "wasm32")]`, so even this pure path<->Route
// mapping is only reachable on the wasm target. These run under node via the
// `wasm-bindgen-test-runner` (`wasm-pack test`); no DOM/browser needed.
#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    #[wasm_bindgen_test]
    fn root_is_landing() {
        assert_eq!(parse("/"), Route::Landing);
        assert_eq!(parse(""), Route::Landing);
    }

    #[wasm_bindgen_test]
    fn each_doc_type_slug_parses() {
        for doc_type in [
            DocType::Prose,
            DocType::Trame,
            DocType::Toile,
            DocType::Murmure,
        ] {
            let id = DocumentId::new();
            let route = parse(&doc_path(doc_type, id));
            assert_eq!(
                route,
                Route::Doc {
                    doc_type,
                    document_id: id
                }
            );
            assert_eq!(route.doc(), Some((doc_type, id)));
        }
    }

    #[wasm_bindgen_test]
    fn path_doc_roundtrip() {
        // `parse` is the exact inverse of `doc_path` on every doc type.
        let id = DocumentId::new();
        assert_eq!(
            parse(&doc_path(DocType::Trame, id)),
            Route::Doc {
                doc_type: DocType::Trame,
                document_id: id
            }
        );
    }

    #[wasm_bindgen_test]
    fn unknown_slug_falls_back_to_landing() {
        let id = DocumentId::new();
        assert_eq!(parse(&format!("/widget/{id}")), Route::Landing);
    }

    #[wasm_bindgen_test]
    fn malformed_id_falls_back_to_landing() {
        assert_eq!(parse("/trame/not-a-uuid"), Route::Landing);
    }

    #[wasm_bindgen_test]
    fn trailing_and_extra_segments_tolerated() {
        // A trailing slash leaves the two leading segments intact.
        let id = DocumentId::new();
        assert_eq!(
            parse(&format!("/trame/{id}/")),
            Route::Doc {
                doc_type: DocType::Trame,
                document_id: id
            }
        );
        // A bare type with no id is not a document route.
        assert_eq!(parse("/trame"), Route::Landing);
    }

    #[wasm_bindgen_test]
    fn landing_route_has_no_doc() {
        assert_eq!(Route::Landing.doc(), None);
    }

    #[wasm_bindgen_test]
    fn cross_kind_open_needs_reload() {
        // Same render kind as the booted view -> in-place swap, no reload.
        assert!(!needs_reload(DocType::Trame, false));
        assert!(!needs_reload(DocType::Toile, true));
        // Different kind -> must hard-navigate to re-boot the right AppView.
        assert!(needs_reload(DocType::Toile, false));
        assert!(needs_reload(DocType::Trame, true));
    }
}
