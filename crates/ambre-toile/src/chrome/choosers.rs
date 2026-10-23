// ambre-toile/src/chrome/choosers.rs
// ============================================================================
// Toile's three choosers (`ARCH-018`): a web embed, a document embed and a
// link. What each one places on the board is Toile's to decide, so the
// dialogs live here. The launcher hands in only what it alone holds: the
// documents this user may open, and the document it has open.
//
// An address is normalized here, where the user can still read the refusal
// (`TOILE-088`). Only `https` survives the gate, so nothing that could run in
// the board's stead reaches a frame.
// ============================================================================

use ambre_bus::app::toile::LinkTarget;
use ambre_bus::{DocType, DocumentId};
use ambre_chrome::{ComposedName, Dialog};
use leptos::prelude::*;

use super::handler::ToileDialogs;
use crate::ui::deck::DeckSignals;
use crate::ui::embed::{EmbedTarget, PlacedEmbed, normalize_web_address};
use crate::ui::link::LinkEdit;

/// A document the user may embed or link to, as the launcher's list names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocChoice {
    /// The document's identity.
    pub document_id: DocumentId,
    /// Its kind, which decides how an embed of it renders.
    pub doc_type: DocType,
    /// Its name, if it has one.
    pub name: Option<String>,
}

/// The web embed, document embed and link dialogs (`TOILE-087`, `TOILE-086`,
/// `toile-links`).
#[component]
pub fn ToileChoosers(
    /// The open state and the typed text of each chooser.
    dialogs: ToileDialogs,
    /// The documents this user may open, as the launcher last fetched them.
    #[prop(into)]
    docs: Signal<Vec<DocChoice>>,
    /// The document open now, which no chooser offers.
    #[prop(into)]
    here: Signal<Option<DocumentId>>,
    /// The deck, whose frames a link may point at.
    deck: DeckSignals,
    /// Where a chosen embed is published for the view to place.
    placed: RwSignal<Option<PlacedEmbed>>,
    /// Where a chosen link is published for the view to set.
    set_link: RwSignal<Option<LinkEdit>>,
) -> impl IntoView {
    let ToileDialogs {
        embed_web_open,
        embed_web_text,
        embed_web_error,
        embed_doc_open,
        link_open,
        link_text,
        link_error,
    } = dialogs;
    // A board embedding itself is a session that opens a session that opens a
    // session, so the open document is never offered.
    let others = move || {
        let here = here.get();
        docs.get()
            .into_iter()
            .filter(|d| Some(d.document_id) != here)
            .collect::<Vec<_>>()
    };
    let place_web =
        move || place_web_embed(embed_web_text, embed_web_error, embed_web_open, placed);
    let link_web = move || set_web_link(link_text, link_error, link_open, set_link);

    view! {
        // Embed a web page (`TOILE-087`).
        <Dialog open=embed_web_open class="ambre-embed-web">
            <div class="ambre-embed-panel">
                <h2>"Embed a web page"</h2>
                <input
                    type="url"
                    placeholder="example.com"
                    prop:value=move || embed_web_text.get()
                    on:input=move |ev| {
                        embed_web_text.set(event_target_value(&ev));
                        embed_web_error.set(None);
                    }
                    on:keydown=move |ev: web_sys::KeyboardEvent| {
                        if ev.key() == "Enter" {
                            place_web();
                        }
                    }
                />
                <p class="ambre-embed-error" aria-live="polite">
                    {move || embed_web_error.get().unwrap_or("")}
                </p>
                <div class="ambre-dialog-actions">
                    <button on:click=move |_| embed_web_open.set(false)>"Cancel"</button>
                    <button on:click=move |_| place_web()>"Embed"</button>
                </div>
            </div>
        </Dialog>

        // Embed another Ambre document (`TOILE-086`). The list is the one the
        // launcher already holds: this opens no document, so it touches none
        // (`TOILE-225`).
        <Dialog open=embed_doc_open class="ambre-embed-doc">
            <div class="ambre-embed-panel">
                <h2>"Embed a document"</h2>
                <ul class="ambre-embed-docs">
                    <For each=others key=|d| d.document_id let:doc>
                        {
                            let (document_id, doc_type) = (doc.document_id, doc.doc_type);
                            let target = EmbedTarget::Document { document_id, doc_type };
                            view! {
                                <li>
                                    <button on:click=move |_| {
                                        placed.set(Some(PlacedEmbed { target: target.clone(), at: None }));
                                        embed_doc_open.set(false);
                                    }>{label(&doc)}</button>
                                </li>
                            }
                        }
                    </For>
                </ul>
                <div class="ambre-dialog-actions">
                    <button on:click=move |_| embed_doc_open.set(false)>"Cancel"</button>
                </div>
            </div>
        </Dialog>

        // Put a link on the selected element (`toile-links`). Three targets and
        // no more: an address, a frame on this board, or another document. The
        // frame list is the deck's, which holds every frame. That includes the
        // off-screen ones a link exists to reach.
        <Dialog open=link_open class="ambre-link-chooser">
            <div class="ambre-embed-panel">
                <h2>"Link this element"</h2>
                <h3>"To a web page"</h3>
                <input
                    type="url"
                    placeholder="example.com"
                    prop:value=move || link_text.get()
                    on:input=move |ev| {
                        link_text.set(event_target_value(&ev));
                        link_error.set(None);
                    }
                    on:keydown=move |ev: web_sys::KeyboardEvent| {
                        if ev.key() == "Enter" {
                            link_web();
                        }
                    }
                />
                <p class="ambre-embed-error" aria-live="polite">
                    {move || link_error.get().unwrap_or("")}
                </p>
                <h3>"To a frame on this board"</h3>
                <ul class="ambre-embed-docs">
                    <For
                        each=move || {
                            let deck = deck.view.get();
                            deck.slides.into_iter().chain(deck.off_deck).collect::<Vec<_>>()
                        }
                        key=|f| f.id.clone()
                        let:frame
                    >
                        {
                            let id = frame.id.clone();
                            let label = if frame.name.is_empty() {
                                "Untitled frame".to_owned()
                            } else {
                                frame.name.clone()
                            };
                            view! {
                                <li>
                                    <button on:click=move |_| {
                                        set_link.set(Some(LinkEdit(Some(LinkTarget::Frame(id.clone())))));
                                        link_open.set(false);
                                    }>{label}</button>
                                </li>
                            }
                        }
                    </For>
                </ul>
                <h3>"To another document"</h3>
                <ul class="ambre-embed-docs">
                    <For each=others key=|d| d.document_id let:doc>
                        {
                            let (document_id, doc_type) = (doc.document_id, doc.doc_type);
                            view! {
                                <li>
                                    <button on:click=move |_| {
                                        let target = LinkTarget::Document { document_id, doc_type };
                                        set_link.set(Some(LinkEdit(Some(target))));
                                        link_open.set(false);
                                    }>{label(&doc)}</button>
                                </li>
                            }
                        }
                    </For>
                </ul>
                <div class="ambre-dialog-actions">
                    <button on:click=move |_| link_open.set(false)>"Cancel"</button>
                    <button on:click=move |_| link_web()>"Link"</button>
                </div>
            </div>
        </Dialog>
    }
}

/// A document's name as a chooser lists it, which no rename takes back
/// (`CHROME-080`).
fn label(doc: &DocChoice) -> impl IntoView + use<> {
    ComposedName::or_placeholder(doc.name.as_deref()).into_view()
}

/// Resolve what the user typed into an address the board may hold, and place
/// it (`TOILE-087`, `TOILE-088`). A refusal stays in the dialog, beside the
/// field the user fixes it in. Otherwise the user would find a box on the
/// board that never paints.
fn place_web_embed(
    typed: RwSignal<String>,
    error: RwSignal<Option<&'static str>>,
    open: RwSignal<bool>,
    placed: RwSignal<Option<PlacedEmbed>>,
) {
    match normalize_web_address(&typed.get_untracked()) {
        Ok(url) => {
            placed.set(Some(PlacedEmbed {
                target: EmbedTarget::Web(url),
                at: None,
            }));
            open.set(false);
        }
        Err(e) => error.set(Some(e.message())),
    }
}

/// Put the typed address through the gate an embed's address goes through, and
/// hand the board a link to it (`TOILE-066`). A refusal stays in the dialog.
fn set_web_link(
    typed: RwSignal<String>,
    error: RwSignal<Option<&'static str>>,
    open: RwSignal<bool>,
    edit: RwSignal<Option<LinkEdit>>,
) {
    match normalize_web_address(&typed.get_untracked()) {
        Ok(url) => {
            edit.set(Some(LinkEdit(Some(LinkTarget::Url(
                ambre_core::LinkUrl::new(url.into_string()),
            )))));
            open.set(false);
        }
        Err(e) => error.set(Some(e.message())),
    }
}
