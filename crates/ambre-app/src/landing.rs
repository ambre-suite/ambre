// ambre-app/src/landing.rs
// ============================================================================
// Landing page (UI thread): the overlay shown on the `/` route.
//
// A plain Leptos DOM view, consistent with the chrome, which is also Leptos. It
// is rendered as a full-bleed overlay ON TOP of the persistent canvas. The canvas
// is never unmounted, only covered (and its rAF paused) while on the landing.
// It has two faces, driven by the `username` signal:
//
//   - Signed out (`username == None`): a sign-in panel. In dev (`dev_mode`),
//     a username field that dispatches `DevLogin` directly to the already
//     running Logic thread. No page reload, and the Sync worker keeps its keys.
//     Otherwise an OIDC button. The panel carries the Ambre wordmark + tagline.
//   - Signed in (`username == Some(..)`): the user's document list plus a
//     "new document" action. Opening or creating a document is a client
//     pushState to `/trame/<id>`; the persistent canvas swaps to it in place.
//
// Document cards (`ambre-spec-launcher.md`) show the type icon and the DECRYPTED
// name (or a dimmed "Untitled" placeholder). They also show the relative
// last-modified time and a discreet shared badge (member count > 1). Per-card
// inline rename and delete (behind an irreversible-action confirmation
// `<dialog>`) wire the existing `RenameDocument` / `DeleteDocument` REST flow.
// Listing requires a session, so the signed-out panel gates access.
// ============================================================================

use ambre_bus::{DocType, DocumentId, LogicCommand, RequestId, RestCommand};
use leptos::prelude::*;
use leptos_router::hooks::use_navigate;

use ambre_chrome::{ComposedName, Toasts};

use crate::route;

/// One row in the document list, already reduced to display-ready fields.
/// `name` is the plaintext title decrypted by the Sync thread (`None` = never
/// named -> "Untitled" placeholder, `ambre-spec-launcher.md`, CHROME-036).
#[derive(Clone)]
pub struct DocItem {
    pub document_id: DocumentId,
    pub doc_type: DocType,
    pub name: Option<String>,
    /// Epoch milliseconds of the last modification (routing metadata, plaintext).
    pub updated_at: ambre_bus::Timestamp,
    /// Total member count; `> 1` => shared.
    pub member_count: ambre_core::MemberCount,
}

/// How the server lets a user sign in, as `/api/auth/config` reports it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SignInMode {
    /// Dev login: a bare username, no identity provider.
    Dev,
    /// The OIDC provider.
    Oidc,
}

impl SignInMode {
    /// The mode for the server's `dev_mode` flag.
    pub fn from_dev_flag(dev: bool) -> Self {
        if dev { Self::Dev } else { Self::Oidc }
    }

    pub fn is_dev(self) -> bool {
        self == Self::Dev
    }
}

/// Reactive handles the launcher fills as auth state and the document list
/// resolve. They are shared with the per-frame relay (session, document
/// list) and the config fetch (`dev_mode`).
#[derive(Clone, Copy)]
pub struct LandingSignals {
    /// Documents to list (only meaningful once signed in).
    pub docs: RwSignal<Vec<DocItem>>,
    /// Current user's display name once `SessionReady` arrives; `None` = signed out.
    pub username: RwSignal<Option<ambre_core::DisplayName>>,
    /// Whether the server enables dev login; `None` until `/api/auth/config` answers.
    pub dev_mode: RwSignal<Option<SignInMode>>,
    /// The app kind this page booted with (`lib.rs`, fixed per page load): drives
    /// the in-place-swap vs. hard-reload choice when opening/creating a document
    /// ([`route::needs_reload`]).
    pub booted: DocType,
    /// Where a refused `.ambre` file reaches the user (`DOCFILE-005`). The
    /// launcher raises no other notification, so this is the one handle it
    /// needs.
    pub toasts: Toasts,
    /// Where a plaintext `.ambre` container goes once read: the per-frame
    /// relay stores its payload sealed (`RESID-025`).
    pub opened_file: RwSignal<Option<crate::platform::OpenedFile>>,
}

/// The landing overlay view. `to_logic` lets the dev sign-in panel dispatch
/// `DevLogin`, and the document cards dispatch rename/delete, to the live Logic
/// thread without a page reload.
pub fn landing_view(
    signals: LandingSignals,
    to_logic: ambre_bus::conduit::Sender<LogicCommand>,
) -> impl IntoView {
    let LandingSignals {
        docs,
        username,
        dev_mode,
        booted,
        toasts,
        opened_file,
    } = signals;

    // New document (`ambre-spec-launcher.md`): the app-agnostic type picker hands
    // us the chosen `DocType`. Generate the id client-side (UUID v7). The server
    // row is created lazily on the first edit, so no request here and no name
    // prompt (the clean default is unnamed). Same-kind creates swap in place
    // (keeping the Sync worker), a cross-kind create hard-reloads to re-boot the
    // right `AppView` -- both handled by `open_document`/`route::needs_reload`.
    let on_pick = {
        let navigate = use_navigate();
        Callback::new(move |doc_type: DocType| {
            open_document(&navigate, doc_type, DocumentId::new(), booted);
        })
    };

    // A sealed file waits here between the picker and the Sync worker
    // (`DOCFILE-014`). The bytes stay in this signal, not in the dialog: the
    // prompt owns the passphrase and nothing else, and the two only meet on
    // submission.
    let sealed_file = RwSignal::new(None::<ambre_core::SealedContainer>);
    let passphrase_open = RwSignal::new(ambre_chrome::Openness::Closed);
    let on_sealed = Callback::new(move |file: ambre_core::SealedContainer| {
        sealed_file.set(Some(file));
        passphrase_open.set(ambre_chrome::Openness::Open);
    });
    let on_passphrase = {
        let to_logic = to_logic.clone();
        Callback::new(move |passphrase: ambre_core::Passphrase| {
            let Some(sealed) = sealed_file.get_untracked() else {
                return;
            };
            sealed_file.set(None);
            let _ = to_logic.try_send(LogicCommand::Rest {
                id: RequestId::next(),
                cmd: RestCommand::OpenDocFile { sealed, passphrase },
            });
        })
    };

    // The irreversible delete (E2EE: no server-side trash). A card only names
    // its target here; `DeleteConfirm` renders the name and holds the dispatch
    // behind its confirming action (`CHROME-041`).
    let pending_delete = RwSignal::new(None::<(DocumentId, ComposedName)>);

    // Distinct clones for the two view sites (the `view!` macro's capture
    // analysis cannot share one binding across both closures).
    let to_logic_cards = to_logic.clone();
    let to_logic_signin = to_logic.clone();
    let to_logic_signout = to_logic.clone();

    let confirm_delete = {
        let to_logic = to_logic.clone();
        Callback::new(move |document_id: DocumentId| {
            let _ = to_logic.try_send(LogicCommand::Rest {
                id: RequestId::next(),
                cmd: RestCommand::DeleteDocument { document_id },
            });
            // Optimistic removal -- the relay also re-lists on `DocumentDeleted`.
            docs.update(|v| v.retain(|d| d.document_id != document_id));
        })
    };

    view! {
        <div class="ambre-landing">
            <header class="ambre-landing-header">
                <span class="title">"Ambre"</span>
                <Show when=move || username.get().is_some()>
                    <div class="ambre-landing-actions">
                        <span class="ambre-landing-user">
                            {move || username.get().map(ambre_core::DisplayName::into_string).unwrap_or_default()}
                        </span>
                        {sign_out_control(to_logic_signout.clone())}
                        <NewDocPicker on_pick=on_pick />
                        <button
                            class="ambre-open-file"
                            title="Open an .ambre file from this device"
                            on:click=move |_| crate::platform::open_ambre_file_picker(
                                toasts,
                                on_sealed,
                                opened_file,
                            )
                        >
                            "Open a file"
                        </button>
                    </div>
                </Show>
            </header>

            // Signed in: the document list.
            <Show when=move || username.get().is_some()>
                <ul class="ambre-doc-list">
                    <For
                        each=move || docs.get()
                        key=|d| (d.document_id, d.name.clone(), d.updated_at)
                        children={
                            let to_logic = to_logic_cards.clone();
                            move |d: DocItem| {
                                doc_card(d, docs, to_logic.clone(), pending_delete, booted, toasts)
                            }
                        }
                    />
                    <Show when=move || docs.get().is_empty()>
                        <li class="ambre-doc-empty">
                            "No documents yet. Create one above."
                        </li>
                    </Show>
                </ul>
            </Show>

            // Signed out: the sign-in panel (dev field or OIDC button).
            <Show when=move || username.get().is_none()>
                {
                    let to_logic = to_logic_signin.clone();
                    move || sign_in_panel(dev_mode, to_logic.clone())
                }
            </Show>

            <DeleteConfirm pending=pending_delete on_confirm=confirm_delete />

            // The passphrase of a sealed file.
            <ambre_chrome::PassphraseDialog
                open=passphrase_open
                intent=ambre_chrome::PassphraseIntent::Open
                on_submit=on_passphrase
            />
        </div>
    }
}

/// Navigate to a document, choosing between an in-place router swap and a full page
/// reload (`ambre-spec-launcher.md`, CHROME-039). The booted `AppView` is fixed per
/// page load. A target whose render kind differs from the booted kind
/// (`route::needs_reload`) must hard-navigate (`location.set_href`) to re-boot the
/// right view. A same-kind target uses the router pushState `navigate`, which swaps
/// the document in place and keeps the Sync worker/session alive.
fn open_document(
    navigate: &impl Fn(&str, leptos_router::NavigateOptions),
    doc_type: DocType,
    document_id: DocumentId,
    booted: DocType,
) {
    let path = route::doc_path(doc_type, document_id);
    if route::needs_reload(doc_type, booted) {
        if let Some(window) = web_sys::window() {
            let _ = window.location().set_href(&path);
        }
    } else {
        navigate(&path, Default::default());
    }
}

/// The "new document" type picker (`ambre-spec-launcher.md`, CHROME-038). A single header
/// control that, when opened, offers one entry per launcher-creatable `DocType`
/// ([`DocType::creatable`]), each labelled with its icon monogram + generic type
/// label ([`DocType::generic_label`]). Picking an entry runs `on_pick` with the
/// chosen type and closes the menu.
///
/// App-agnostic by construction: it iterates `DocType::creatable()` and never
/// names a single type, so a new creatable engine appears here automatically.
#[component]
pub fn NewDocPicker(
    /// Invoked with the chosen document type when the user picks an entry.
    on_pick: Callback<DocType>,
) -> impl IntoView {
    // Whether the type menu is expanded. A picked entry (or a re-click on the
    // button) collapses it.
    let open = RwSignal::new(false);
    let menu_ref = NodeRef::<leptos::html::Ul>::new();
    // An opened menu takes the focus, so the keys reach it (`CHROME-026`).
    Effect::new(move |_| {
        if let Some(menu) = menu_ref.get() {
            ambre_chrome::focus_first_item(&menu);
        }
    });

    view! {
        <div class="ambre-new-doc-picker">
            <button
                class="ambre-new-doc"
                aria-haspopup="menu"
                aria-expanded=move || ambre_chrome::Openness::from_bool(open.get()).aria()
                on:click=move |_| open.update(|o| *o = !*o)
            >
                "New document"
            </button>
            <Show when=move || open.get()>
                <ul
                    class="ambre-new-doc-menu"
                    role="menu"
                    node_ref=menu_ref
                    on:keydown=move |ev| {
                        if let Some(menu) = menu_ref.get_untracked() {
                            ambre_chrome::on_menu_keydown(&ev, &menu, || open.set(false));
                        }
                    }
                >
                    {DocType::creatable()
                        .iter()
                        .map(|&doc_type| {
                            view! {
                                <li
                                    class="ambre-new-doc-option"
                                    role="menuitem"
                                    tabindex="-1"
                                    aria-label=doc_type.generic_label()
                                    on:click=move |_| {
                                        open.set(false);
                                        on_pick.run(doc_type);
                                    }
                                >
                                    <span aria-hidden="true" class=format!(
                                        "ambre-doc-icon ambre-doc-icon-{}",
                                        doc_type.name().to_lowercase(),
                                    )>{doc_type.icon()}</span>
                                    <span class="ambre-new-doc-option-label">
                                        {doc_type.generic_label()}
                                    </span>
                                </li>
                            }
                        })
                        .collect_view()}
                </ul>
            </Show>
        </div>
    }
}

/// One document card: type-icon tile + name (or "Untitled") + relative time +
/// shared badge. It carries inline rename and a delete that routes through
/// the confirmation dialog.
fn doc_card(
    d: DocItem,
    docs: RwSignal<Vec<DocItem>>,
    to_logic: ambre_bus::conduit::Sender<LogicCommand>,
    pending_delete: RwSignal<Option<(DocumentId, ComposedName)>>,
    booted: DocType,
    toasts: Toasts,
) -> impl IntoView {
    let doc_type = d.doc_type;
    let id = d.document_id;
    let shared = d.member_count.is_shared();
    let updated_at = d.updated_at;
    let display_name = d.name.clone();

    // Inline-rename editing state, seeded with the current name.
    let editing = RwSignal::new(false);
    let edit_value = RwSignal::new(d.name.clone().unwrap_or_default());

    let on_open = {
        let navigate = use_navigate();
        move |_| {
            if !editing.get() {
                open_document(&navigate, doc_type, id, booted);
            }
        }
    };

    let commit_rename = {
        let to_logic = to_logic.clone();
        move || {
            // A name past its budget makes no `Name`. It is refused here, with
            // the field left open to shorten it, rather than cut or sent to
            // fail (`SYNC-178`).
            let Ok(name) = ambre_core::Name::new(edit_value.get().trim()) else {
                toasts.warning(ambre_core::NAME_TOO_LONG);
                return;
            };
            editing.set(false);
            if !name.is_empty() {
                let _ = to_logic.try_send(LogicCommand::Rest {
                    id: RequestId::next(),
                    cmd: RestCommand::RenameDocument {
                        document_id: id,
                        name: name.clone(),
                    },
                });
                // Optimistic update -- the relay also re-lists on `DocumentRenamed`.
                docs.update(|v| {
                    if let Some(item) = v.iter_mut().find(|x| x.document_id == id) {
                        item.name = Some(name.to_string());
                    }
                });
            }
        }
    };

    view! {
        <li class="ambre-doc-item">
            <span class=format!("ambre-doc-icon ambre-doc-icon-{}", doc_type.name().to_lowercase())>
                {doc_type.icon()}
            </span>
            <div class="ambre-doc-main" on:click=on_open>
                <Show
                    when=move || editing.get()
                    fallback=move || card_name(display_name.clone())
                >
                    {
                        let commit_rename = commit_rename.clone();
                        view! {
                            <input
                                class="ambre-doc-rename-input"
                                prop:value=move || edit_value.get()
                                on:click=|e| e.stop_propagation()
                                on:input=move |e| edit_value.set(event_target_value(&e))
                                on:keydown={
                                    let commit_rename = commit_rename.clone();
                                    move |e| {
                                        if e.key() == "Enter" {
                                            commit_rename();
                                        } else if e.key() == "Escape" {
                                            editing.set(false);
                                        }
                                    }
                                }
                                on:blur=move |_| commit_rename()
                            />
                        }
                    }
                </Show>
                <span class="ambre-doc-meta">
                    {rel_time(updated_at)}
                    <Show when=move || shared>
                        <span class="ambre-doc-shared">"Shared"</span>
                    </Show>
                </span>
            </div>
            <div class="ambre-doc-actions">
                <button
                    class="ambre-doc-action"
                    title="Rename"
                    on:click=move |e| {
                        e.stop_propagation();
                        edit_value.set(
                            docs.get_untracked()
                                .iter()
                                .find(|x| x.document_id == id)
                                .and_then(|x| x.name.clone())
                                .unwrap_or_default(),
                        );
                        editing.set(true);
                    }
                >
                    "Rename"
                </button>
                <button
                    class="ambre-doc-action ambre-doc-action-danger"
                    title="Delete"
                    on:click=move |e| {
                        e.stop_propagation();
                        let label = docs.with_untracked(|list| {
                            let doc = list.iter().find(|x| x.document_id == id);
                            ComposedName::or_placeholder(doc.and_then(|x| x.name.as_deref()))
                        });
                        pending_delete.set(Some((id, label)));
                    }
                >
                    "Delete"
                </button>
            </div>
        </li>
    }
}

/// A card's name: the decrypted name (`CHROME-036`), or a dimmed placeholder.
///
/// It takes the name alone, so no fallback can reach for the identifier or
/// the kind (`CHROME-079`). The text is a [`ComposedName`], which no rename
/// can take back (`CHROME-080`).
fn card_name(name: Option<String>) -> impl IntoView {
    let named = name.as_deref().is_some_and(|n| !n.trim().is_empty());
    let class = if named {
        "ambre-doc-name"
    } else {
        "ambre-doc-name ambre-doc-name-untitled"
    };
    let shown = ComposedName::or_placeholder(name.as_deref());
    view! { <span class=class>{shown.into_view()}</span> }
}

/// The confirmation every delete passes through (`CHROME-041`).
///
/// A card only sets `pending`: the modal names the target, and only its
/// confirming action runs `on_confirm`. Cancel, Escape and the backdrop clear
/// `pending` and dispatch nothing.
#[component]
fn DeleteConfirm(
    /// The document a card asked to delete, with the name to confirm against.
    pending: RwSignal<Option<(DocumentId, ComposedName)>>,
    /// The dispatch, run with the confirmed document.
    on_confirm: Callback<DocumentId>,
) -> impl IntoView {
    let open = RwSignal::new(ambre_chrome::Openness::Closed);
    Effect::new(move |_| open.set(ambre_chrome::Openness::from_bool(pending.get().is_some())));
    let on_close = Callback::new(move |()| pending.set(None));
    let confirm = move |_| {
        if let Some((document_id, _)) = pending.get_untracked() {
            on_confirm.run(document_id);
        }
        pending.set(None);
    };
    view! {
        <ambre_chrome::Dialog open=open on_close=on_close class="ambre-confirm-dialog">
            <h3>"Delete document?"</h3>
            <p>
                "\""
                {move || pending.get().map(|(_, label)| label.into_view())}
                "\" will be permanently deleted. This cannot be undone."
            </p>
            <div class="ambre-confirm-actions">
                <button class="ambre-btn-secondary" on:click=move |_| pending.set(None)>
                    "Cancel"
                </button>
                <button class="ambre-btn-danger" on:click=confirm>
                    "Delete"
                </button>
            </div>
        </ambre_chrome::Dialog>
    }
}

/// Formats an epoch-millisecond timestamp as a coarse relative time ("2 h ago").
/// `0` (no server timestamp) renders as an empty string rather than a fake date.
fn rel_time(updated_at: ambre_bus::Timestamp) -> String {
    let updated_at_ms = updated_at.0;
    if updated_at_ms == 0 {
        return String::new();
    }
    let now = js_sys::Date::now();
    let delta_s = ((now - updated_at_ms as f64) / 1000.0).max(0.0);
    if delta_s < 60.0 {
        "just now".to_owned()
    } else if delta_s < 3600.0 {
        format!("{} min ago", (delta_s / 60.0) as u64)
    } else if delta_s < 86_400.0 {
        format!("{} h ago", (delta_s / 3600.0) as u64)
    } else if delta_s < 2_592_000.0 {
        format!("{} d ago", (delta_s / 86_400.0) as u64)
    } else {
        format!("{} mo ago", (delta_s / 2_592_000.0) as u64)
    }
}

/// The signed-out sign-in panel. Renders a dev-login field when the server
/// reports `dev_mode`, an OIDC button otherwise, and a neutral placeholder
/// until `/api/auth/config` has answered.
/// The sign-out control (`CHROME-107`). It asks the live threads to end the
/// session, once: the page boots home afresh when the Sync worker confirms,
/// so nothing here clears anything.
fn sign_out_control(to_logic: ambre_bus::conduit::Sender<LogicCommand>) -> impl IntoView {
    let leaving = RwSignal::new(false);
    view! {
        <button
            class="ambre-sign-out"
            disabled=move || leaving.get()
            on:click=move |_| {
                if leaving.get_untracked() {
                    return;
                }
                leaving.set(true);
                let _ = to_logic.try_send(LogicCommand::Rest {
                    id: RequestId::next(),
                    cmd: RestCommand::Logout,
                });
            }
        >
            {move || if leaving.get() { "Signing out..." } else { "Sign out" }}
        </button>
    }
}

fn sign_in_panel(
    dev_mode: RwSignal<Option<SignInMode>>,
    to_logic: ambre_bus::conduit::Sender<LogicCommand>,
) -> impl IntoView {
    move || match dev_mode.get() {
        None => view! {
            <div class="ambre-signin">
                <p class="ambre-signin-loading">"Connecting..."</p>
            </div>
        }
        .into_any(),
        Some(SignInMode::Dev) => {
            // Dev login: dispatch `DevLogin` to the live Logic thread (no reload).
            // The user (and derived keys) are created server-side on the fly; the
            // session reply flips `username`, revealing the document list.
            let value = RwSignal::new("alice".to_owned());
            let to_logic = to_logic.clone();
            let go = move || {
                // A name the account rule refuses, a blank one among them,
                // sends nothing: the relay would refuse it (`SRV-234`).
                if let Ok(name) = ambre_core::DisplayName::new(&value.get()) {
                    // Remember the dev identity so a later refresh re-establishes
                    // the session without `?user=` in the URL.
                    crate::remember_dev_user(&name);
                    let _ = to_logic.try_send(LogicCommand::Rest {
                        id: RequestId::next(),
                        cmd: RestCommand::DevLogin { username: name },
                    });
                }
            };
            view! {
                <div class="ambre-signin">
                    <span class="ambre-signin-wordmark">"Ambre"</span>
                    <p class="ambre-signin-tagline">
                        "Private collaborative office suite. End-to-end encrypted."
                    </p>
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
        Some(SignInMode::Oidc) => {
            // Ask the Sync worker for the provider's address; the reply sends
            // the browser there (`CHROME-095`). The page itself requests nothing.
            let to_logic = to_logic.clone();
            view! {
                <div class="ambre-signin">
                    <span class="ambre-signin-wordmark">"Ambre"</span>
                    <p class="ambre-signin-tagline">
                        "Private collaborative office suite. End-to-end encrypted."
                    </p>
                    <button
                        class="ambre-signin-btn"
                        on:click=move |_| {
                            let _ = to_logic.try_send(LogicCommand::Rest {
                                id: RequestId::next(),
                                cmd: RestCommand::ProviderSignIn {
                                    provider: ambre_core::OidcProvider::Google,
                                },
                            });
                        }
                    >
                        "Sign in with Google"
                    </button>
                </div>
            }
            .into_any()
        }
    }
}

// -- Tests (wasm tier) -------------------------------------------------------
// The crate is `#![cfg(target_arch = "wasm32")]`, so the launcher chrome is only
// reachable on the wasm target. These mount the type picker into a real headless
// Chrome document and drive it through `web-sys` (`just test-wasm`).
#[cfg(test)]
mod tests {
    use super::*;
    use leptos::mount::mount_to;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    fn document() -> web_sys::Document {
        web_sys::window().unwrap().document().unwrap()
    }

    /// Mount a view into its own container appended to the body, mirroring the
    /// Trame chrome tests. A per-test container keeps the runner's own harness
    /// DOM nodes untouched, and `.forget()` keeps the view alive for assertions.
    fn mount_in_container<F, N>(f: F) -> web_sys::Element
    where
        F: FnOnce() -> N + 'static,
        N: IntoView + 'static,
    {
        let container = document().create_element("div").unwrap();
        document().body().unwrap().append_child(&container).unwrap();
        let target: web_sys::HtmlElement = container.clone().dyn_into().unwrap();
        mount_to(target, f).forget();
        container
    }

    fn click(el: &web_sys::Element) {
        el.dyn_ref::<web_sys::HtmlElement>().unwrap().click();
    }

    /// Yield to the event loop so leptos flushes the reactive DOM update a click
    /// scheduled. CSR render effects run on a microtask, not synchronously in the
    /// click handler.
    async fn tick() {
        let promise = js_sys::Promise::new(&mut |resolve, _| {
            web_sys::window()
                .unwrap()
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 0)
                .unwrap();
        });
        let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
    }

    /// The picker starts collapsed, opens to exactly one entry per creatable
    /// `DocType` (in order), and picking an entry reports that type and closes.
    #[wasm_bindgen_test]
    async fn picker_lists_creatable_types_and_reports_pick() {
        // A reactive owner backs the sink signal (Send + Sync, unlike an `Rc`,
        // which the `Callback` bound rejects) and the picker's own signals.
        let owner = Owner::new();
        owner.set();
        let picks = RwSignal::new(Vec::<DocType>::new());
        let on_pick = Callback::new(move |dt: DocType| picks.update(|v| v.push(dt)));

        let container = mount_in_container(move || view! { <NewDocPicker on_pick=on_pick /> });

        // Collapsed: the trigger is present, no options yet.
        let button = container.query_selector(".ambre-new-doc").unwrap().unwrap();
        assert_eq!(
            button.get_attribute("aria-expanded").as_deref(),
            Some("false")
        );
        assert_eq!(
            container
                .query_selector_all(".ambre-new-doc-option")
                .unwrap()
                .length(),
            0
        );

        // Open: one entry per creatable type, in the canonical order, each
        // carrying its generic type label (not the brand name).
        click(&button);
        tick().await;
        let options = container
            .query_selector_all(".ambre-new-doc-option")
            .unwrap();
        let creatable = DocType::creatable();
        assert_eq!(options.length() as usize, creatable.len());
        for (i, &dt) in creatable.iter().enumerate() {
            let text = options
                .item(i as u32)
                .unwrap()
                .text_content()
                .unwrap_or_default();
            assert!(
                text.contains(dt.generic_label()),
                "entry {i} missing label {}: {text}",
                dt.generic_label()
            );
        }

        // Pick the first entry -> the callback fires with that type and the menu
        // collapses again.
        let first: web_sys::Element = options.item(0).unwrap().dyn_into().unwrap();
        click(&first);
        tick().await;
        assert_eq!(picks.get_untracked().as_slice(), &[creatable[0]]);
        assert_eq!(
            container
                .query_selector_all(".ambre-new-doc-option")
                .unwrap()
                .length(),
            0
        );

        container.remove();
    }

    /// Dispatch a bubbling keydown carrying `key` on `target`.
    fn press(target: &web_sys::Element, key: &str) {
        let init = web_sys::KeyboardEventInit::new();
        init.set_key(key);
        init.set_bubbles(true);
        init.set_cancelable(true);
        let ev =
            web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
        target.dispatch_event(&ev).unwrap();
    }

    /// The type menu is driven by the keyboard alone: it takes the focus, End
    /// and a letter move it, Enter picks, and Escape closes (`CHROME-026`).
    #[wasm_bindgen_test]
    async fn the_type_menu_is_operable_by_keyboard_alone() {
        let owner = Owner::new();
        owner.set();
        let picks = RwSignal::new(Vec::<DocType>::new());
        let on_pick = Callback::new(move |dt: DocType| picks.update(|v| v.push(dt)));
        let container = mount_in_container(move || view! { <NewDocPicker on_pick=on_pick /> });
        let button = container.query_selector(".ambre-new-doc").unwrap().unwrap();
        let creatable = DocType::creatable();
        let focused = || document().active_element();

        click(&button);
        tick().await;
        let menu = container.query_selector("[role=menu]").unwrap().unwrap();
        let options = menu.query_selector_all("[role=menuitem]").unwrap();
        let at = |i: usize| options.item(i as u32).and_then(|n| n.dyn_into().ok());
        assert_eq!(focused(), at(0), "the menu took the focus");

        press(&menu, "End");
        assert_eq!(focused(), at(creatable.len() - 1), "End jumped to the last");
        // The letter answers to the type's word, not its monogram.
        let last = creatable[creatable.len() - 1].generic_label();
        let letter = last
            .chars()
            .next()
            .unwrap()
            .to_ascii_lowercase()
            .to_string();
        press(&menu, "Home");
        press(&menu, &letter);
        assert_eq!(focused(), at(creatable.len() - 1), "the letter found it");

        press(&menu, "Enter");
        tick().await;
        assert_eq!(
            picks.get_untracked().as_slice(),
            &[creatable[creatable.len() - 1]]
        );
        assert!(container.query_selector("[role=menu]").unwrap().is_none());

        click(&button);
        tick().await;
        let menu = container.query_selector("[role=menu]").unwrap().unwrap();
        press(&menu, "Escape");
        tick().await;
        assert!(container.query_selector("[role=menu]").unwrap().is_none());
        assert_eq!(picks.get_untracked().len(), 1, "Escape picked nothing");
        container.remove();
    }

    /// The provider control asks for the provider's address, and goes nowhere
    /// on its own: the page stays put until the address arrives (`CHROME-095`).
    #[wasm_bindgen_test]
    async fn the_provider_control_asks_for_the_address() {
        let owner = Owner::new();
        owner.set();
        let (tx, rx) =
            ambre_bus::conduit::local::<LogicCommand>(ambre_bus::conduit::ChannelCapacity::new(8));
        let dev_mode = RwSignal::new(Some(SignInMode::Oidc));
        let before = web_sys::window().unwrap().location().href().unwrap();

        let container = mount_in_container(move || sign_in_panel(dev_mode, tx));
        let button = container
            .query_selector(".ambre-signin-btn")
            .unwrap()
            .unwrap();
        click(&button);
        tick().await;

        let sent: Vec<LogicCommand> = rx.try_iter().collect();
        assert_eq!(sent.len(), 1);
        assert!(matches!(
            &sent[0],
            LogicCommand::Rest { cmd: RestCommand::ProviderSignIn { provider }, .. }
                if *provider == ambre_core::OidcProvider::Google
        ));
        assert_eq!(
            web_sys::window().unwrap().location().href().unwrap(),
            before
        );
        container.remove();
    }

    /// The sign-out control asks the live threads to end the session, once,
    /// and leaves the page where it is until the worker confirms
    /// (`CHROME-107`).
    #[wasm_bindgen_test]
    async fn the_sign_out_control_ends_the_session_once() {
        let owner = Owner::new();
        owner.set();
        let (tx, rx) =
            ambre_bus::conduit::local::<LogicCommand>(ambre_bus::conduit::ChannelCapacity::new(8));
        let before = web_sys::window().unwrap().location().href().unwrap();

        let container = mount_in_container(move || sign_out_control(tx));
        let button = container
            .query_selector(".ambre-sign-out")
            .unwrap()
            .unwrap();
        click(&button);
        tick().await;
        click(&button);
        tick().await;

        let sent: Vec<LogicCommand> = rx.try_iter().collect();
        assert_eq!(sent.len(), 1, "one sign-out, however many clicks");
        assert!(matches!(
            &sent[0],
            LogicCommand::Rest {
                cmd: RestCommand::Logout,
                ..
            }
        ));
        assert!(button.has_attribute("disabled"));
        assert_eq!(
            web_sys::window().unwrap().location().href().unwrap(),
            before
        );
        container.remove();
    }

    /// A card shows the decrypted name it was handed (`CHROME-036`). An
    /// unnamed document shows the dimmed placeholder, and no identifier or
    /// kind word (`CHROME-079`).
    #[wasm_bindgen_test]
    fn a_card_shows_the_name_or_the_placeholder() {
        let named = mount_in_container(|| card_name(Some("Budget 2026".to_owned())));
        let span = named.query_selector(".ambre-doc-name").unwrap().unwrap();
        assert_eq!(span.text_content().as_deref(), Some("Budget 2026"));
        assert_eq!(span.class_name(), "ambre-doc-name");
        named.remove();

        let unnamed = mount_in_container(|| card_name(None));
        let span = unnamed.query_selector(".ambre-doc-name").unwrap().unwrap();
        assert_eq!(span.text_content().as_deref(), Some("Untitled"));
        assert!(span.class_name().contains("ambre-doc-name-untitled"));
        for &kind in DocType::creatable() {
            assert_ne!("Untitled", kind.generic_label(), "never the kind's word");
        }
        unnamed.remove();
    }

    /// A delete is confirmed against the document's own name before anything
    /// is dispatched (`CHROME-041`).
    ///
    /// Asking opens the modal on the name and dispatches nothing. Cancel
    /// dispatches nothing, and only the confirming action dispatches.
    #[wasm_bindgen_test]
    async fn a_delete_waits_for_the_confirmation_on_the_name() {
        let owner = Owner::new();
        owner.set();
        let pending = RwSignal::new(None::<(DocumentId, ComposedName)>);
        let confirmed = RwSignal::new(Vec::<DocumentId>::new());
        let on_confirm = Callback::new(move |id| confirmed.update(|v| v.push(id)));
        let container = mount_in_container(move || {
            view! { <DeleteConfirm pending=pending on_confirm=on_confirm /> }
        });
        let dialog: web_sys::HtmlDialogElement = container
            .query_selector("dialog")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        let button = |class: &str| {
            container
                .query_selector(&format!(".{class}"))
                .unwrap()
                .unwrap()
        };

        let id = DocumentId::new();
        let payroll = || ComposedName::or_placeholder(Some("Payroll"));
        pending.set(Some((id, payroll())));
        leptos::task::tick().await;
        assert!(dialog.open(), "a delete opens the confirmation");
        let text = dialog.text_content().unwrap_or_default();
        assert!(text.contains("\"Payroll\""), "it names the target: {text}");
        assert!(
            confirmed.get_untracked().is_empty(),
            "nothing dispatched yet"
        );

        click(&button("ambre-btn-secondary"));
        tick().await;
        assert!(!dialog.open(), "cancel closes it");
        assert!(pending.get_untracked().is_none());
        assert!(
            confirmed.get_untracked().is_empty(),
            "cancel dispatches nothing"
        );

        pending.set(Some((id, payroll())));
        leptos::task::tick().await;
        click(&button("ambre-btn-danger"));
        tick().await;
        assert_eq!(
            confirmed.get_untracked(),
            vec![id],
            "the confirmation dispatches"
        );
        assert!(!dialog.open());
        container.remove();
    }

    /// The sign-in panel shows neither route until the server has answered
    /// which one applies (`CHROME-045`).
    #[wasm_bindgen_test]
    async fn the_sign_in_waits_for_the_answer() {
        let owner = Owner::new();
        owner.set();
        let (tx, _rx) =
            ambre_bus::conduit::local::<LogicCommand>(ambre_bus::conduit::ChannelCapacity::new(8));
        let dev_mode = RwSignal::new(None);
        let container = mount_in_container(move || sign_in_panel(dev_mode, tx));
        let routes = |c: &web_sys::Element| {
            (
                c.query_selector(".ambre-signin-input").unwrap().is_some(),
                c.query_selector(".ambre-signin-btn").unwrap().is_some(),
            )
        };
        assert_eq!(
            routes(&container),
            (false, false),
            "no route before the answer"
        );
        assert!(
            container
                .query_selector(".ambre-signin-loading")
                .unwrap()
                .is_some()
        );

        dev_mode.set(Some(SignInMode::Dev));
        tick().await;
        assert_eq!(
            routes(&container),
            (true, true),
            "the dev route once answered"
        );
        container.remove();
    }

    /// Every creatable type maps to a path that round-trips back to a matching
    /// document route. So a picked type boots the right engine on the hard nav.
    #[wasm_bindgen_test]
    fn creatable_types_round_trip_through_doc_path() {
        for &doc_type in DocType::creatable() {
            let id = DocumentId::new();
            let route = route::parse(&route::doc_path(doc_type, id));
            assert_eq!(route.doc(), Some((doc_type, id)));
        }
    }

    /// A rename past the name cap is refused with a toast, sends nothing and
    /// leaves the field open. Shortened, it goes (`SYNC-178`).
    #[wasm_bindgen_test]
    async fn a_rename_past_the_cap_is_refused_with_a_toast() {
        let owner = Owner::new();
        owner.set();
        let (tx, rx) =
            ambre_bus::conduit::local::<LogicCommand>(ambre_bus::conduit::ChannelCapacity::new(8));
        let toasts = Toasts::new();
        let item = DocItem {
            document_id: DocumentId::new(),
            doc_type: DocType::Prose,
            name: Some("Plan".to_owned()),
            updated_at: ambre_bus::Timestamp(0),
            member_count: ambre_core::MemberCount::ONE,
        };
        let docs = RwSignal::new(vec![item.clone()]);
        let pending_delete = RwSignal::new(None);
        let container = mount_in_container(move || {
            view! {
                <leptos_router::components::Router>
                    {doc_card(item, docs, tx, pending_delete, DocType::Prose, toasts)}
                    <ambre_chrome::ToastHost toasts=toasts />
                </leptos_router::components::Router>
            }
        });
        let rename = container
            .query_selector(".ambre-doc-action[title='Rename']")
            .unwrap()
            .unwrap();
        click(&rename);
        tick().await;

        let commit = |text: String| {
            let input: web_sys::HtmlInputElement = container
                .query_selector(".ambre-doc-rename-input")
                .unwrap()
                .expect("the field is open")
                .dyn_into()
                .unwrap();
            input.set_value(&text);
            input
                .dispatch_event(&web_sys::Event::new("input").unwrap())
                .unwrap();
            press(&input, "Enter");
        };
        commit("a".repeat(ambre_core::NAME_MAX_CHARS + 1));
        tick().await;
        assert_eq!(rx.try_iter().count(), 0, "nothing leaves");
        let toast = container
            .query_selector(".ambre-toast")
            .unwrap()
            .and_then(|t| t.text_content())
            .unwrap_or_default();
        assert!(toast.contains(ambre_core::NAME_TOO_LONG), "{toast}");
        assert!(
            container
                .query_selector(".ambre-doc-rename-input")
                .unwrap()
                .is_some(),
            "the field stays open to shorten it"
        );
        assert_eq!(docs.get_untracked()[0].name.as_deref(), Some("Plan"));

        commit("Budget".to_owned());
        tick().await;
        let sent: Vec<LogicCommand> = rx.try_iter().collect();
        assert!(matches!(
            sent.as_slice(),
            [LogicCommand::Rest { cmd: RestCommand::RenameDocument { name, .. }, .. }]
                if name.as_str() == "Budget"
        ));
        container.remove();
    }
}
