//! The browser tab title (`ComposedName::tab_title`).
//!
//! The frame loop recomputes the title each frame, so a rename or a
//! late-arriving document list updates the tab. The DOM write happens only
//! when the title changes.

use ambre_bus::DocType;
use ambre_chrome::ComposedName;
use ambre_core::id::{Document, Id};
use leptos::prelude::*;

use crate::landing::DocItem;

/// The tab title: the document list it reads names from, and the title last
/// written.
pub(crate) struct TabTitle {
    docs: RwSignal<Vec<DocItem>>,
    last: Option<ComposedName>,
}

impl TabTitle {
    /// A tab title that has written nothing yet.
    pub(crate) fn new(docs: RwSignal<Vec<DocItem>>) -> Self {
        Self { docs, last: None }
    }

    /// Write the title of the routed document, or of the landing route on
    /// `None`, when it differs from the last one written.
    pub(crate) fn show(&mut self, cur_doc: Option<(DocType, Id<Document>)>) {
        let title = match cur_doc {
            Some((doc_type, document_id)) => {
                let name = self.docs.with_untracked(|list| {
                    list.iter()
                        .find(|d| d.document_id == document_id)
                        .and_then(|d| d.name.clone())
                });
                ComposedName::tab_title(Some((doc_type, name.as_ref())))
            }
            None => ComposedName::tab_title(None),
        };
        if self.last.as_ref() != Some(&title) {
            if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                title.set_as_title(&doc);
            }
            self.last = Some(title);
        }
    }
}
