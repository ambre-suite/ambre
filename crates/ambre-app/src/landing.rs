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

use ambre_bus::{DocType, DocumentId, LogicCommand, RequestId, RestCommand, SecretBytes};
use leptos::html;
use leptos::prelude::*;
use leptos_router::hooks::use_navigate;

use ambre_chrome::Toasts;

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
    pub updated_at: u64,
    /// Total member count; `> 1` => shared.
    pub member_count: u32,
}

/// Reactive handles the launcher fills as auth state and the document list
/// resolve. They are shared with the per-frame relay (session, document
/// list) and the config fetch (`dev_mode`).
#[derive(Clone, Copy)]
pub struct LandingSignals {
    /// Documents to list (only meaningful once signed in).
    pub docs: RwSignal<Vec<DocItem>>,
    /// Current user's display name once `SessionReady` arrives; `None` = signed out.
    pub username: RwSignal<Option<String>>,
    /// Whether the server enables dev login; `None` until `/api/auth/config` answers.
    pub dev_mode: RwSignal<Option<bool>>,
    /// The app kind this page booted with (`lib.rs`, fixed per page load): drives
    /// the in-place-swap vs. hard-reload choice when opening/creating a document
    /// ([`route::needs_reload`]).
    pub booted: DocType,
    /// Where a refused `.ambre` file reaches the user (`DOCFILE-005`). The
    /// launcher raises no other notification, so this is the one handle it
    /// needs.
    pub toasts: Toasts,
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
    let sealed_file = RwSignal::new(None::<Vec<u8>>);
    let passphrase_open = RwSignal::new(false);
    let on_sealed = Callback::new(move |bytes: Vec<u8>| {
        sealed_file.set(Some(bytes));
        passphrase_open.set(true);
    });
    let on_passphrase = {
        let to_logic = to_logic.clone();
        Callback::new(move |passphrase: String| {
            let Some(sealed) = sealed_file.get_untracked() else {
                return;
            };
            sealed_file.set(None);
            let _ = to_logic.try_send(LogicCommand::Rest {
                id: RequestId::next(),
                cmd: RestCommand::OpenDocFile {
                    sealed,
                    passphrase: SecretBytes::from(passphrase.into_bytes()),
                },
            });
        })
    };

    // Confirmation dialog for the irreversible delete (E2EE: no server-side
    // trash). Holds the pending target so the dialog can name it before the user
    // confirms; `<dialog>` provides focus trap / Escape / backdrop for free.
    let pending_delete = RwSignal::new(None::<(DocumentId, String)>);
    let dialog_ref = NodeRef::<html::Dialog>::new();

    // Distinct clones for the two view sites (the `view!` macro's capture
    // analysis cannot share one binding across both closures).
    let to_logic_cards = to_logic.clone();
    let to_logic_signin = to_logic.clone();

    let confirm_delete = {
        let to_logic = to_logic.clone();
        move || {
            if let Some((document_id, _)) = pending_delete.get() {
                let _ = to_logic.try_send(LogicCommand::Rest {
                    id: RequestId::next(),
                    cmd: RestCommand::DeleteDocument { document_id },
                });
                // Optimistic removal -- the relay also re-lists on `DocumentDeleted`.
                docs.update(|v| v.retain(|d| d.document_id != document_id));
            }
            pending_delete.set(None);
            if let Some(dlg) = dialog_ref.get() {
                let _ = dlg.close();
            }
        }
    };

    view! {
        <div class="ambre-landing">
            <header class="ambre-landing-header">
                <span class="title">"Ambre"</span>
                <Show when=move || username.get().is_some()>
                    <div class="ambre-landing-actions">
                        <span class="ambre-landing-user">
                            {move || username.get().unwrap_or_default()}
                        </span>
                        <NewDocPicker on_pick=on_pick />
                        <button
                            class="ambre-open-file"
                            title="Open an .ambre file from this device"
                            on:click=move |_| crate::platform::open_ambre_file_picker(
                                toasts,
                                on_sealed,
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
                                doc_card(d, docs, to_logic.clone(), pending_delete, dialog_ref, booted)
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

            // Delete confirmation -- native `<dialog>`, opened imperatively when a
            // card requests a delete (the target name is rendered reactively).
            <dialog node_ref=dialog_ref class="ambre-confirm-dialog">
                <h3>"Delete document?"</h3>
                <p>
                    "\""
                    {move || {
                        pending_delete
                            .get()
                            .map(|(_, label)| label)
                            .unwrap_or_default()
                    }}
                    "\" will be permanently deleted. This cannot be undone."
                </p>
                <div class="ambre-confirm-actions">
                    <button
                        class="ambre-btn-secondary"
                        on:click=move |_| {
                            pending_delete.set(None);
                            if let Some(dlg) = dialog_ref.get() {
                                let _ = dlg.close();
                            }
                        }
                    >
                        "Cancel"
                    </button>
                    <button
                        class="ambre-btn-danger"
                        on:click=move |_| confirm_delete()
                    >
                        "Delete"
                    </button>
                </div>
            </dialog>

            // The passphrase of a sealed file. No `on_open`/`on_close`: this
            // overlay already covers the canvas, and its rAF already stopped.
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

    view! {
        <div class="ambre-new-doc-picker">
            <button
                class="ambre-new-doc"
                aria-haspopup="menu"
                aria-expanded=move || if open.get() { "true" } else { "false" }
                on:click=move |_| open.update(|o| *o = !*o)
            >
                "New document"
            </button>
            <Show when=move || open.get()>
                <ul class="ambre-new-doc-menu" role="menu">
                    {DocType::creatable()
                        .iter()
                        .map(|&doc_type| {
                            view! {
                                <li
                                    class="ambre-new-doc-option"
                                    role="menuitem"
                                    on:click=move |_| {
                                        open.set(false);
                                        on_pick.run(doc_type);
                                    }
                                >
                                    <span class=format!(
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
    pending_delete: RwSignal<Option<(DocumentId, String)>>,
    dialog_ref: NodeRef<html::Dialog>,
    booted: DocType,
) -> impl IntoView {
    let doc_type = d.doc_type;
    let id = d.document_id;
    let shared = d.member_count > 1;
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
            let name = edit_value.get().trim().to_owned();
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
                        item.name = Some(name.clone());
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
                    fallback=move || {
                        let name = display_name.clone();
                        view! {
                            <span class=move || {
                                if name.is_some() {
                                    "ambre-doc-name"
                                } else {
                                    "ambre-doc-name ambre-doc-name-untitled"
                                }
                            }>
                                {name.clone().unwrap_or_else(|| "Untitled".to_owned())}
                            </span>
                        }
                    }
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
                        let label = docs
                            .get_untracked()
                            .iter()
                            .find(|x| x.document_id == id)
                            .and_then(|x| x.name.clone())
                            .unwrap_or_else(|| "Untitled".to_owned());
                        pending_delete.set(Some((id, label)));
                        if let Some(dlg) = dialog_ref.get() {
                            let _ = dlg.show_modal();
                        }
                    }
                >
                    "Delete"
                </button>
            </div>
        </li>
    }
}

/// Formats an epoch-millisecond timestamp as a coarse relative time ("2 h ago").
/// `0` (no server timestamp) renders as an empty string rather than a fake date.
fn rel_time(updated_at_ms: u64) -> String {
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
fn sign_in_panel(
    dev_mode: RwSignal<Option<bool>>,
    to_logic: ambre_bus::conduit::Sender<LogicCommand>,
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
        Some(false) => view! {
            <div class="ambre-signin">
                <span class="ambre-signin-wordmark">"Ambre"</span>
                <p class="ambre-signin-tagline">
                    "Private collaborative office suite. End-to-end encrypted."
                </p>
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

// -- Tests (wasm tier) -------------------------------------------------------
// The crate is `#![cfg(target_arch = "wasm32")]`, so the launcher chrome is only
// reachable on the wasm target. These mount the type picker into a real headless
// Chrome document and drive it through `web-sys`, run via `wasm-pack test
// --headless --chrome crates/ambre-app` (`just test-wasm`).
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
}
