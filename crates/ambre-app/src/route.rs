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
// (see `lib.rs`, `leptos_router`). The wgpu canvas, Logic thread and Sync worker
// are created once, and the active document is swapped in place. The worker's
// in-memory session keys survive a route change. This module is reduced to the
// pure path<->`Route` mapping reused by the router shell.
// ============================================================================

use ambre_bus::DocType;
use ambre_core::id::{Document, Id};
use ambre_core::{FixedBytes, InviterPin};

/// The page the URL resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// Root: the document list / landing page.
    Landing,
    /// A specific document, with its type known from the path.
    Doc {
        doc_type: DocType,
        document_id: Id<Document>,
    },
}

impl Route {
    /// The routed document, if this is a document route.
    pub fn doc(&self) -> Option<(DocType, Id<Document>)> {
        match self {
            Route::Doc {
                doc_type,
                document_id,
            } => Some((*doc_type, *document_id)),
            Route::Landing => None,
        }
    }

    /// The document the route binds the engine to. A Murmure page binds none:
    /// it opens the channels its sidebar selects, never a document by address.
    pub fn document(&self) -> Option<(DocType, Id<Document>)> {
        self.doc()
            .filter(|(doc_type, _)| *doc_type != DocType::Murmure)
    }
}

/// What the per-frame route poll does on a route change. Each field acts on
/// the live engine: a document closes, a document opens, the canvas shows a
/// document or freezes. No field builds a second engine, so no route change
/// ends the first (`ARCH-010`, `ARCH-016`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// The document the Logic thread closes.
    pub close: Option<Id<Document>>,
    /// The document the Logic thread opens.
    pub open: Option<(DocType, Id<Document>)>,
    /// The document the canvas shows, or `None` when it shows none: at home,
    /// or on a Murmure page.
    pub show: Option<Id<Document>>,
}

/// Whether the canvas renders on `route`. Home freezes it behind the
/// launcher. A page renders, a Murmure page too, though it binds no document:
/// its message list and its composer are drawn there.
pub fn cover(route: &Route) -> ambre_canvas::Cover {
    match route {
        Route::Landing => ambre_canvas::Cover::Launcher,
        Route::Doc { .. } => ambre_canvas::Cover::Document,
    }
}

/// The step from the previous route to the current one, or `None` when the
/// route did not change. A move inside one document opens nothing.
pub fn step(prev: Option<&Route>, cur: &Route) -> Option<Step> {
    if prev == Some(cur) {
        return None;
    }
    let prev_doc = prev.and_then(Route::document);
    let cur_doc = cur.document();
    let moved = prev_doc.map(|(_, id)| id) != cur_doc.map(|(_, id)| id);
    Some(Step {
        close: prev_doc.filter(|_| moved).map(|(_, id)| id),
        open: cur_doc.filter(|_| moved),
        show: cur_doc.map(|(_, id)| id),
    })
}

/// URL slug for a document type: the one word of the set. Inverse of
/// [`slug_to_type`].
fn type_slug(doc_type: DocType) -> &'static str {
    doc_type.as_str()
}

/// Parse a path slug back into a [`DocType`].
fn slug_to_type(slug: &str) -> Option<DocType> {
    slug.parse().ok()
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
            match (slug_to_type(type_slug), Id::<Document>::parse_str(id_str)) {
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

/// The field of a link fragment that carries the inviter's pin (`SYNC-208`).
/// A fragment never leaves the browser in a request, so the relay that
/// serves the page never reads the pin off the address.
const INVITER_FIELD: &str = "inviter=";

/// The fragment an invite link appends to the document's path: the pin of the
/// inviter's signing key, in hexadecimal (`SYNC-208`). Inverse of
/// [`inviter_in`].
pub fn invite_fragment(inviter: &FixedBytes<InviterPin>) -> String {
    format!("#{INVITER_FIELD}{}", inviter.to_hex())
}

/// The inviter's pin a link fragment carries, or `None` when the fragment
/// names none or spells no pin (`SYNC-209`). A malformed pin pins nothing,
/// and the open then runs as it would with no link.
pub fn inviter_in(fragment: &str) -> Option<FixedBytes<InviterPin>> {
    fragment
        .trim_start_matches('#')
        .split('&')
        .find_map(|field| field.strip_prefix(INVITER_FIELD))
        .and_then(|hex| FixedBytes::from_hex(hex).ok())
}

/// The inviter's pin the fragment of `window.location` carries.
pub fn current_inviter() -> Option<FixedBytes<InviterPin>> {
    let hash = web_sys::window()?.location().hash().ok()?;
    inviter_in(&hash)
}

/// The canonical path for a document route (for example `/trame/<uuid>`).
pub fn doc_path(doc_type: DocType, document_id: Id<Document>) -> String {
    format!("/{}/{}", type_slug(doc_type), document_id)
}

/// Whether opening `target` from a page booted onto `booted` requires a
/// **full page reload** rather than an in-place `AppView::set_document` swap.
///
/// The booted canvas view is fixed per page load. `set_document` can swap the
/// document id **within the same app kind**, but cannot switch kinds (Trame,
/// Toile, Prose). So a target whose render kind differs from the booted kind must
/// hard-navigate (`location.set_href`) to re-boot the right `AppView`. A
/// same-kind target uses a router pushState that keeps the Sync worker/session
/// (`ambre-spec-launcher.md`, CHROME-039).
pub fn needs_reload(target: DocType, booted: DocType) -> bool {
    target != booted
}

/// The type of the view a page booted with. The landing route boots the
/// Trame view on a throwaway document.
pub fn booted_kind(route: &Route) -> DocType {
    route.doc().map_or(DocType::Trame, |(doc_type, _)| doc_type)
}

// -- Tests (wasm tier) -------------------------------------------------------
// The crate is `#![cfg(target_arch = "wasm32")]`, so even this pure path<->Route
// mapping is only reachable on the wasm target. These run in the browser the
// crate's lib test configures (`just test-wasm`), and need no DOM.
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
            let id = Id::<Document>::mint();
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
        let id = Id::<Document>::mint();
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
        let id = Id::<Document>::mint();
        assert_eq!(parse(&format!("/widget/{id}")), Route::Landing);
    }

    #[wasm_bindgen_test]
    fn malformed_id_falls_back_to_landing() {
        assert_eq!(parse("/trame/not-a-uuid"), Route::Landing);
    }

    #[wasm_bindgen_test]
    fn trailing_and_extra_segments_tolerated() {
        // A trailing slash leaves the two leading segments intact.
        let id = Id::<Document>::mint();
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
    fn a_route_walk_reuses_the_one_engine() {
        // `ARCH-010`, `ARCH-016`: every route change is a close, an open and a
        // show on the live engine. A second engine has no step to come from.
        let (a, b) = (Id::<Document>::mint(), Id::<Document>::mint());
        let doc = |document_id| Route::Doc {
            doc_type: DocType::Trame,
            document_id,
        };
        let walk = [
            Route::Landing,
            doc(a),
            doc(b),
            doc(b),
            Route::Landing,
            doc(a),
        ];
        let mut prev: Option<&Route> = None;
        let mut steps = Vec::new();
        for cur in &walk {
            steps.push(step(prev, cur));
            prev = Some(cur);
        }
        let s = |close, open: Option<Id<Document>>, show| {
            Some(Step {
                close,
                open: open.map(|id| (DocType::Trame, id)),
                show,
            })
        };
        assert_eq!(
            steps,
            vec![
                s(None, None, None),
                s(None, Some(a), Some(a)),
                s(Some(a), Some(b), Some(b)),
                None,
                s(Some(b), None, None),
                s(None, Some(a), Some(a)),
            ]
        );
    }

    #[wasm_bindgen_test]
    fn a_murmure_page_opens_no_document() {
        let page = Route::Doc {
            doc_type: DocType::Murmure,
            document_id: Id::<Document>::mint(),
        };
        assert_eq!(page.document(), None);
        assert_eq!(
            step(Some(&Route::Landing), &page),
            Some(Step {
                close: None,
                open: None,
                show: None
            })
        );
        assert_eq!(booted_kind(&page), DocType::Murmure);
        assert_eq!(booted_kind(&Route::Landing), DocType::Trame);
    }

    /// A Murmure page binds no document and still renders, so its composer
    /// is drawn and takes a press. Home alone freezes the canvas.
    #[wasm_bindgen_test]
    fn a_murmure_page_renders_and_home_freezes() {
        let murmure = Route::Doc {
            doc_type: DocType::Murmure,
            document_id: Id::<Document>::mint(),
        };
        assert_eq!(cover(&murmure), ambre_canvas::Cover::Document);
        assert_eq!(cover(&Route::Landing), ambre_canvas::Cover::Launcher);
        let prose = Route::Doc {
            doc_type: DocType::Prose,
            document_id: Id::<Document>::mint(),
        };
        assert_eq!(cover(&prose), ambre_canvas::Cover::Document);
    }

    #[wasm_bindgen_test]
    fn cross_kind_open_needs_reload() {
        // Same render kind as the booted view -> in-place swap, no reload.
        assert!(!needs_reload(DocType::Trame, DocType::Trame));
        assert!(!needs_reload(DocType::Toile, DocType::Toile));
        assert!(!needs_reload(DocType::Prose, DocType::Prose));
        // Different kind -> must hard-navigate to re-boot the right AppView.
        assert!(needs_reload(DocType::Toile, DocType::Trame));
        assert!(needs_reload(DocType::Trame, DocType::Toile));
        assert!(needs_reload(DocType::Prose, DocType::Trame));
    }

    /// Holds `SYNC-208` and `SYNC-209` at the address. The fragment of an
    /// invite link reads back to the same pin, beside other fields. A fragment
    /// with no pin, or a malformed one, pins nothing.
    #[wasm_bindgen_test]
    fn an_invite_fragment_reads_back_to_its_pin() {
        let pin = FixedBytes::<InviterPin>::from_array([0xA5; 32]);
        let fragment = invite_fragment(&pin);
        assert_eq!(inviter_in(&fragment), Some(pin));
        assert_eq!(inviter_in(&format!("#x=1&{}", &fragment[1..])), Some(pin));
        assert_eq!(inviter_in(""), None);
        assert_eq!(inviter_in("#inviter=zz"), None);
        assert_eq!(inviter_in("#inviter=a5a5"), None);
    }
}
