// ambre-app/src/landing.rs
// ============================================================================
// Landing page (UI thread): the page shown at `/`.
//
// A plain Leptos DOM view (consistent with the chrome, which is also Leptos):
// the canvas is NOT booted here. It has two faces, driven by the `username`
// signal:
//
//   - Signed out (`username == None`): a sign-in panel. In dev (`dev_mode`),
//     a username field that navigates to `/?user=<name>` -- reusing the whole
//     existing DevLogin boot path. Otherwise an OIDC button.
//   - Signed in (`username == Some(..)`): the user's document list plus a
//     "new document" action. Opening or creating a document navigates to
//     `/trame/<id>` (a full page load), where the canvas flow takes over.
//
// Listing documents requires a session, so the signed-out panel gates access.
// ============================================================================

use std::cell::RefCell;
use std::rc::Rc;

use ambre_bus::{DocType, DocumentId};
use leptos::prelude::*;

use crate::route;

/// One row in the document list, already reduced to display-ready fields.
#[derive(Clone)]
pub struct DocItem {
    pub document_id: DocumentId,
    pub doc_type: DocType,
}

/// Reactive handles the launcher fills as auth state and the document list
/// resolve; published back out of `mount_landing` so the relay loop and the
/// config fetch can drive the view.
#[derive(Clone)]
pub struct LandingSignals {
    /// Documents to list (only meaningful once signed in).
    pub docs: RwSignal<Vec<DocItem>>,
    /// Current user's display name once `SessionReady` arrives; `None` = signed out.
    pub username: RwSignal<Option<String>>,
    /// Whether the server enables dev login; `None` until `/api/auth/config` answers.
    pub dev_mode: RwSignal<Option<bool>>,
}

/// Mount the landing page onto the page body and publish its signals via
/// `slot` so the launcher can fill them (config flag, session, document list).
pub fn mount_landing(slot: Rc<RefCell<Option<LandingSignals>>>) {
    leptos::mount::mount_to_body(move || {
        let signals = LandingSignals {
            docs: RwSignal::new(Vec::<DocItem>::new()),
            username: RwSignal::new(None::<String>),
            dev_mode: RwSignal::new(None::<bool>),
        };
        *slot.borrow_mut() = Some(signals.clone());
        let LandingSignals { docs, username, dev_mode } = signals;

        // New document: generate the id client-side (UUID v7) and navigate. The
        // server row is created lazily on the first edit, so no request here.
        let on_new = move |_| {
            route::navigate_to_doc(DocType::Trame, DocumentId::new());
        };

        view! {
            <div class="ambre-landing">
                <header class="ambre-landing-header">
                    <span class="title">"Ambre"</span>
                    <Show when=move || username.get().is_some()>
                        <span class="spacer"></span>
                        <span class="ambre-landing-user">
                            {move || username.get().unwrap_or_default()}
                        </span>
                        <button class="ambre-new-doc" on:click=on_new>
                            "New Trame"
                        </button>
                    </Show>
                </header>

                // Signed in: the document list.
                <Show when=move || username.get().is_some()>
                    <ul class="ambre-doc-list">
                        <For
                            each=move || docs.get()
                            key=|d| d.document_id
                            children=move |d: DocItem| {
                                let doc_type = d.doc_type;
                                let id = d.document_id;
                                let label = format!("{} - {}", doc_type.name(), short_id(id));
                                let on_open = move |_| route::navigate_to_doc(doc_type, id);
                                view! {
                                    <li class="ambre-doc-item" on:click=on_open>
                                        {label}
                                    </li>
                                }
                            }
                        />
                        <Show when=move || docs.get().is_empty()>
                            <li class="ambre-doc-empty">"No documents yet. Create one above."</li>
                        </Show>
                    </ul>
                </Show>

                // Signed out: the sign-in panel (dev field or OIDC button).
                <Show when=move || username.get().is_none()>
                    {move || sign_in_panel(dev_mode)}
                </Show>
            </div>
        }
    });
}

/// The signed-out sign-in panel. Renders a dev-login field when the server
/// reports `dev_mode`, an OIDC button otherwise, and a neutral placeholder
/// until `/api/auth/config` has answered.
fn sign_in_panel(dev_mode: RwSignal<Option<bool>>) -> impl IntoView {
    move || match dev_mode.get() {
        None => view! {
            <div class="ambre-signin">
                <p class="ambre-signin-loading">"Connecting..."</p>
            </div>
        }
        .into_any(),
        Some(true) => {
            // Dev login: navigate to `/?user=<name>`, reusing the existing
            // DevLogin boot path. Default to a convenient identity.
            let value = RwSignal::new("alice".to_owned());
            let go = move || {
                let name = value.get();
                let name = name.trim();
                if !name.is_empty() {
                    if let Some(window) = web_sys::window() {
                        let _ = window.location().set_href(&format!("/?user={name}"));
                    }
                }
            };
            view! {
                <div class="ambre-signin">
                    <h2>"Dev sign in"</h2>
                    <input
                        class="ambre-signin-input"
                        prop:value=move || value.get()
                        on:input=move |e| value.set(event_target_value(&e))
                        on:keydown=move |e| { if e.key() == "Enter" { go(); } }
                        placeholder="username"
                    />
                    <button class="ambre-signin-btn" on:click=move |_| go()>
                        "Enter"
                    </button>
                </div>
            }
            .into_any()
        }
        Some(false) => view! {
            <div class="ambre-signin">
                <h2>"Sign in"</h2>
                <button
                    class="ambre-signin-btn"
                    on:click=move |_| {
                        if let Some(window) = web_sys::window() {
                            let _ = window.location().set_href("/api/auth/google/login");
                        }
                    }
                >
                    "Sign in with Google"
                </button>
            </div>
        }
        .into_any(),
    }
}

/// First 8 hex chars of the id, for a compact label (names are E2EE and not
/// decryptable without opening the document).
fn short_id(id: DocumentId) -> String {
    id.to_string().chars().take(8).collect()
}
