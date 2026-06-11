// ambre-app/src/landing.rs
// ============================================================================
// Landing page (UI thread): the overlay shown on the `/` route.
//
// A plain Leptos DOM view (consistent with the chrome, which is also Leptos),
// rendered as a full-bleed overlay ON TOP of the persistent canvas -- the canvas
// is never unmounted, only covered (and its rAF paused) while on the landing.
// It has two faces, driven by the `username` signal:
//
//   - Signed out (`username == None`): a sign-in panel. In dev (`dev_mode`),
//     a username field that dispatches `DevLogin` directly to the (already
//     running) Logic thread -- no page reload, the Sync worker keeps its keys.
//     Otherwise an OIDC button.
//   - Signed in (`username == Some(..)`): the user's document list plus a
//     "new document" action. Opening or creating a document is a client
//     pushState to `/trame/<id>`; the persistent canvas swaps to it in place.
//
// Listing documents requires a session, so the signed-out panel gates access.
// ============================================================================

use ambre_bus::{DocType, DocumentId, LogicCommand, RequestId, RestCommand};
use leptos::prelude::*;
use leptos_router::hooks::use_navigate;

use crate::route;

/// One row in the document list, already reduced to display-ready fields.
#[derive(Clone)]
pub struct DocItem {
    pub document_id: DocumentId,
    pub doc_type: DocType,
}

/// Reactive handles the launcher fills as auth state and the document list
/// resolve; shared with the per-frame relay (session, document list) and the
/// config fetch (`dev_mode`).
#[derive(Clone, Copy)]
pub struct LandingSignals {
    /// Documents to list (only meaningful once signed in).
    pub docs: RwSignal<Vec<DocItem>>,
    /// Current user's display name once `SessionReady` arrives; `None` = signed out.
    pub username: RwSignal<Option<String>>,
    /// Whether the server enables dev login; `None` until `/api/auth/config` answers.
    pub dev_mode: RwSignal<Option<bool>>,
}

/// The landing overlay view. `to_logic` lets the dev sign-in panel dispatch
/// `DevLogin` to the live Logic thread without a page reload.
pub fn landing_view(
    signals: LandingSignals,
    to_logic: flume::Sender<LogicCommand>,
) -> impl IntoView {
    let LandingSignals { docs, username, dev_mode } = signals;

    // New document: generate the id client-side (UUID v7) and pushState-navigate.
    // The server row is created lazily on the first edit, so no request here.
    let on_new = {
        let navigate = use_navigate();
        move |_| {
            navigate(
                &route::doc_path(DocType::Trame, DocumentId::new()),
                Default::default(),
            );
        }
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
                    <button class="ambre-new-doc" on:click=on_new.clone()>
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
                            let navigate = use_navigate();
                            let on_open = move |_| {
                                navigate(&route::doc_path(doc_type, id), Default::default());
                            };
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
                {
                    let to_logic = to_logic.clone();
                    move || sign_in_panel(dev_mode, to_logic.clone())
                }
            </Show>
        </div>
    }
}

/// The signed-out sign-in panel. Renders a dev-login field when the server
/// reports `dev_mode`, an OIDC button otherwise, and a neutral placeholder
/// until `/api/auth/config` has answered.
fn sign_in_panel(
    dev_mode: RwSignal<Option<bool>>,
    to_logic: flume::Sender<LogicCommand>,
) -> impl IntoView {
    move || match dev_mode.get() {
        None => view! {
            <div class="ambre-signin">
                <p class="ambre-signin-loading">"Connecting..."</p>
            </div>
        }
        .into_any(),
        Some(true) => {
            // Dev login: dispatch `DevLogin` to the live Logic thread (no reload).
            // The user (and derived keys) are created server-side on the fly; the
            // session reply flips `username`, revealing the document list.
            let value = RwSignal::new("alice".to_owned());
            let to_logic = to_logic.clone();
            let go = move || {
                let name = value.get();
                let name = name.trim().to_owned();
                if !name.is_empty() {
                    let _ = to_logic.try_send(LogicCommand::Rest {
                        id: RequestId::next(),
                        cmd: RestCommand::DevLogin { username: name },
                    });
                }
            };
            view! {
                <div class="ambre-signin">
                    <h2>"Dev sign in"</h2>
                    <input
                        class="ambre-signin-input"
                        prop:value=move || value.get()
                        on:input=move |e| value.set(event_target_value(&e))
                        on:keydown={
                            let go = go.clone();
                            move |e| { if e.key() == "Enter" { go(); } }
                        }
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
