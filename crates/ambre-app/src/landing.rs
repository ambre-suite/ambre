// ambre-app/src/landing.rs
// ============================================================================
// Landing page (UI thread): the overlay shown on the `/` route.
//
// A plain Leptos DOM view, consistent with the chrome, which is also Leptos. It
// is rendered as a full-bleed overlay ON TOP of the persistent canvas. The canvas
// is never unmounted, only covered (and its rAF paused) while on the landing.
// Its faces are driven by the `username` signal and the keys gate:
//
//   - Signed out (`username == None`): a sign-in panel. In dev (`dev_mode`),
//     a username field that dispatches `DevLogin` directly to the already
//     running Logic thread. No page reload, and the Sync worker keeps its keys.
//     Otherwise an OIDC button. The panel carries the Ambre wordmark + tagline.
//   - Signed in with the provider, keys not open yet: the keys gate
//     (`keys_gate.rs`), an unlock or the setup of the keys. It stands before
//     both other faces while it shows.
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

use ambre_bus::{DocType, RestCommand};
use ambre_core::id::{Document, Id};
use ambre_logic::app::LogicCommand;
use leptos::prelude::*;
use leptos_router::hooks::use_navigate;

use ambre_chrome::{ComposedName, Toasts};

use crate::route;
use ambre_core::Flag as _;
use ambre_core::{Instant, Relayed, Sec, UnixMs};

pub use crate::doc_list::DocItem;

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
/// resolve. They are shared with the switchboard (session, document
/// list) and the config fetch (`dev_mode`).
#[derive(Clone, Copy)]
pub struct LandingSignals {
    /// Documents to list (only meaningful once signed in).
    pub docs: RwSignal<Vec<DocItem>>,
    /// Current user's display name once `SessionReady` arrives; `None` = signed out.
    pub username: RwSignal<Option<ambre_core::Validated<ambre_core::DisplayName>>>,
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
    /// Where a plaintext `.ambre` container goes once read: the switchboard stores its payload sealed (`RESID-025`).
    pub opened_file: RwSignal<Option<crate::platform::OpenedFile>>,
    /// The face of the account keys, which covers the sign-in panel and the
    /// list while it is open (`ambre-spec-launcher.md` section 1.6).
    pub keys: crate::account::KeysSignals,
    /// The face a provider account shows before its list: an unlock or the
    /// setup of its keys (`CHROME-097`, `CHROME-098`).
    pub gate: crate::keys_gate::KeysGate,
}

/// The landing overlay view. `to_logic` lets the dev sign-in panel dispatch
/// `DevLogin`, and the document cards dispatch rename/delete, to the live Logic
/// thread without a page reload.
pub fn landing_view(
    signals: LandingSignals,
    to_logic: crate::outbox::ShellOutbox,
) -> impl IntoView {
    let LandingSignals {
        docs,
        username,
        dev_mode,
        booted,
        toasts,
        opened_file,
        keys,
        gate,
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
            open_document(&navigate, doc_type, Id::<Document>::mint(), booted);
        })
    };

    // A sealed file waits here between the picker and the Sync worker
    // (`DOCFILE-014`). The bytes stay in this signal, not in the dialog: the
    // prompt owns the passphrase and nothing else, and the two only meet on
    // submission.
    let sealed_file = RwSignal::new(None::<ambre_core::Payload<ambre_core::SealedContainer>>);
    let passphrase_open = RwSignal::new(ambre_chrome::Openness::Closed);
    let on_sealed = Callback::new(
        move |file: ambre_core::Payload<ambre_core::SealedContainer>| {
            sealed_file.set(Some(file));
            passphrase_open.set(ambre_chrome::Openness::Open);
        },
    );
    let on_passphrase = {
        let to_logic = to_logic.clone();
        Callback::new(
            move |passphrase: ambre_core::Confidential<ambre_core::Passphrase>| {
                let Some(sealed) = sealed_file.get_untracked() else {
                    return;
                };
                sealed_file.set(None);
                to_logic.send(LogicCommand::Rest {
                    id: ambre_bus::next_request(),
                    cmd: RestCommand::OpenDocFile { sealed, passphrase },
                });
            },
        )
    };

    // The irreversible delete (E2EE: no server-side trash). A card only names
    // its target here; `DeleteConfirm` renders the name and holds the dispatch
    // behind its confirming action (`CHROME-041`).
    let pending_delete = RwSignal::new(None::<(Id<Document>, ComposedName)>);

    // Distinct clones for the two view sites (the `view!` macro's capture
    // analysis cannot share one binding across both closures).
    let to_logic_cards = to_logic.clone();
    let to_logic_signin = to_logic.clone();
    let to_logic_signout = to_logic.clone();
    let to_logic_keys = to_logic.clone();
    let to_logic_gate = to_logic.clone();
    // The gate stands before the list and the sign-in panel alike: a face
    // shown means the keys are still to open or to set up.
    let gated = move || gate.face.get().is_some();

    let confirm_delete = {
        let to_logic = to_logic.clone();
        Callback::new(move |document_id: Id<Document>| {
            let sent = to_logic.send(LogicCommand::Rest {
                id: ambre_bus::next_request(),
                cmd: RestCommand::DeleteDocument { document_id },
            });
            // Optimistic removal -- the relay also re-lists on `DocumentDeleted`.
            // A refused delete leaves the card, which the user can try again
            // (`BUS-018`).
            if sent.accepted() {
                docs.update(|v| v.retain(|d| d.document_id != document_id));
            }
        })
    };

    view! {
        <div class="ambre-landing">
            <header class="ambre-landing-header">
                <span class="title">"Ambre"</span>
                <Show when=move || username.get().is_some()>
                    <div class="ambre-landing-actions">
                        <span class="ambre-landing-user">
                            {move || username.get().map(ambre_core::Validated::into_string).unwrap_or_default()}
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

            // A provider account whose keys are still to open or to set up.
            // The account face, the recovery words above all, covers the gate.
            <Show when=move || !keys.is_open()>
                {crate::keys_gate::keys_gate_view(gate, keys, to_logic_gate.clone())}
            </Show>

            // Signed in: the document list.
            <Show when=move || username.get().is_some() && !keys.is_open() && !gated()>
                {document_list(docs, to_logic_cards.clone(), pending_delete, booted, toasts)}
            </Show>

            // Signed out: the sign-in panel (dev field or OIDC button).
            <Show when=move || username.get().is_none() && !keys.is_open() && !gated()>
                {
                    let to_logic = to_logic_signin.clone();
                    move || sign_in_panel(dev_mode, keys, to_logic.clone())
                }
            </Show>

            // The setup, the recovery words or the recovery, over both.
            {crate::account::keys_view(keys, to_logic_keys)}

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

/// The search field above the list, then the list in its bands
/// (`ambre-spec-launcher.md` section 1.8).
///
/// The query lives on this thread alone. It filters the rows the list already
/// holds, and no keystroke reaches another thread (`CHROME-122`). The field
/// carries its own accessible name, and the count a search lists is announced
/// politely (`CHROME-123`).
fn document_list(
    docs: RwSignal<Vec<DocItem>>,
    to_logic: crate::outbox::ShellOutbox,
    pending_delete: RwSignal<Option<(Id<Document>, ComposedName)>>,
    booted: DocType,
    toasts: Toasts,
) -> impl IntoView {
    let query = RwSignal::new(String::new());
    // A plain derivation, not a memo. The list is a handful of rows, and a
    // memo costs more code in the first download than the arrangement costs
    // time. The bands read the clock when the list or the query changes, as
    // the relative time of a card does.
    let rows = move || {
        let now = Instant::<UnixMs>::new(js_sys::Date::now() as u64);
        docs.with(|list| query.with(|q| crate::doc_list::arrange(list, q, now)))
    };
    let searching = move || query.with(|q| !q.trim().is_empty());
    // One polite status line says what the list holds. That is the
    // invitation of an empty list, the count a query lists, or the news that
    // the query answers nothing (`CHROME-123`, `CHROME-124`). It is empty,
    // and hidden, otherwise.
    let status = move || {
        if docs.with(Vec::is_empty) {
            return "No documents yet. Create one above.".to_owned();
        }
        if !searching() {
            return String::new();
        }
        match rows().len() {
            0 => "No document name contains this search.".to_owned(),
            1 => "1 document".to_owned(),
            n => format!("{n} documents"),
        }
    };
    view! {
        <div class="ambre-doc-search" role="search">
            <input
                type="search"
                class="ambre-doc-search-input"
                aria-label="Search documents by name"
                placeholder="Search documents"
                on:input=move |e| query.set(event_target_value(&e))
                on:keydown=move |e| {
                    // Escape empties the query and keeps the focus in the
                    // field (`CHROME-125`).
                    if e.key() == "Escape" {
                        event_target::<web_sys::HtmlInputElement>(&e).set_value("");
                        query.set(String::new());
                    }
                }
            />
        </div>
        // `status` implies a polite live region, so it carries no `aria-live`.
        <p class="ambre-doc-empty" role="status">{status}</p>
        // One flat list: a band's heading rides on its first row, so the
        // cards keep one `<For>` and one `<ul>`.
        <ul class="ambre-doc-list">
            <For
                each=rows
                key=|row| (row.doc.document_id, row.doc.name.clone(), row.doc.updated_at, row.opens)
                children=move |row: crate::doc_list::Row| {
                    let heading = row.opens.map(|band| {
                        view! {
                            <li>
                                <h2 class="ambre-doc-group-heading">{band.heading()}</h2>
                            </li>
                        }
                    });
                    (heading, doc_card(row.doc, docs, to_logic.clone(), pending_delete, booted, toasts))
                }
            />
        </ul>
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
    document_id: Id<Document>,
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
    let open = RwSignal::new(ambre_chrome::Openness::Closed);
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
                aria-expanded=move || open.get().aria()
                on:click=move |_| open.update(|o| *o = o.toggled())
            >
                "New document"
            </button>
            <Show when=move || open.get() == ambre_chrome::Openness::Open>
                <ul
                    class="ambre-new-doc-menu"
                    role="menu"
                    node_ref=menu_ref
                    on:keydown=move |ev| {
                        if let Some(menu) = menu_ref.get_untracked() {
                            ambre_chrome::on_menu_keydown(&ev, &menu, || open.set(ambre_chrome::Openness::Closed));
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
                                        open.set(ambre_chrome::Openness::Closed);
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
    to_logic: crate::outbox::ShellOutbox,
    pending_delete: RwSignal<Option<(Id<Document>, ComposedName)>>,
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
    let edit_value = RwSignal::new(
        d.name
            .as_ref()
            .map(|n| n.as_str().to_owned())
            .unwrap_or_default(),
    );

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
            let Ok(name) =
                ambre_core::Validated::<ambre_core::Name>::parse(edit_value.get().trim())
            else {
                toasts.warning(ambre_core::NAME_TOO_LONG);
                return;
            };
            editing.set(false);
            if !ambre_core::name_is_empty(&name) {
                let sent = to_logic.send(LogicCommand::Rest {
                    id: ambre_bus::next_request(),
                    cmd: RestCommand::RenameDocument {
                        document_id: id,
                        name: name.clone(),
                    },
                });
                // Optimistic update -- the relay also re-lists on `DocumentRenamed`.
                // A refused rename keeps the old name on the card (`BUS-018`).
                if sent.accepted() {
                    docs.update(|v| {
                        if let Some(item) = v.iter_mut().find(|x| x.document_id == id) {
                            item.name = Some(name);
                        }
                    });
                }
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
                                .and_then(|x| x.name.as_ref().map(|n| n.as_str().to_owned()))
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
                            ComposedName::or_placeholder(doc.and_then(|x| x.name.as_ref()))
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
fn card_name(name: Option<ambre_core::Validated<ambre_core::Name>>) -> impl IntoView {
    let named = name.as_ref().is_some_and(|n| !n.as_str().trim().is_empty());
    let class = if named {
        "ambre-doc-name"
    } else {
        "ambre-doc-name ambre-doc-name-untitled"
    };
    let shown = ComposedName::or_placeholder(name.as_ref());
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
    pending: RwSignal<Option<(Id<Document>, ComposedName)>>,
    /// The dispatch, run with the confirmed document.
    on_confirm: Callback<Id<Document>>,
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
        <ambre_chrome::Dialog
            open=open
            on_close=on_close
            class={ambre_core::StaticKey::<ambre_core::CssClasses>::new("ambre-confirm-dialog")}
        >
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

/// Formats the relay's last write as a coarse relative time ("2 h ago"). No
/// relay time renders as an empty string rather than a fake date.
///
/// The reader's clock meets the relay's through `Instant::age_at` alone, for
/// display: a reader whose clock runs behind reads "just now".
fn rel_time(updated_at: Option<Instant<Relayed>>) -> String {
    let Some(updated_at) = updated_at else {
        return String::new();
    };
    let now = Instant::<UnixMs>::new(js_sys::Date::now() as u64);
    let age_s = updated_at.age_at(now).to_floor::<Sec>().get();
    if age_s < 60 {
        "just now".to_owned()
    } else if age_s < 3600 {
        format!("{} min ago", age_s / 60)
    } else if age_s < 86_400 {
        format!("{} h ago", age_s / 3600)
    } else if age_s < 2_592_000 {
        format!("{} d ago", age_s / 86_400)
    } else {
        format!("{} mo ago", age_s / 2_592_000)
    }
}

/// The signed-out sign-in panel. Renders a dev-login field when the server
/// reports `dev_mode`, an OIDC button otherwise, and a neutral placeholder
/// until `/api/auth/config` has answered.
/// The sign-out control (`CHROME-107`). It asks the live threads to end the
/// session, once: the page boots home afresh when the Sync worker confirms,
/// so nothing here clears anything.
fn sign_out_control(to_logic: crate::outbox::ShellOutbox) -> impl IntoView {
    let leaving = RwSignal::new(false);
    view! {
        <button
            class="ambre-sign-out"
            disabled=move || leaving.get()
            on:click=move |_| {
                if leaving.get_untracked() {
                    return;
                }
                // A refused sign-out leaves the button live (`BUS-018`).
                let sent = to_logic.send(LogicCommand::Rest {
                    id: ambre_bus::next_request(),
                    cmd: RestCommand::Logout,
                });
                if sent.accepted() {
                    leaving.set(true);
                }
            }
        >
            {move || if leaving.get() { "Signing out..." } else { "Sign out" }}
        </button>
    }
}

fn sign_in_panel(
    dev_mode: RwSignal<Option<SignInMode>>,
    keys: crate::account::KeysSignals,
    to_logic: crate::outbox::ShellOutbox,
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
                if let Ok(name) =
                    ambre_core::Validated::<ambre_core::DisplayName>::parse(value.get())
                {
                    // Remember the dev identity so a later refresh re-establishes
                    // the session without `?user=` in the URL.
                    crate::remember_dev_user(&name);
                    to_logic.send(LogicCommand::Rest {
                        id: ambre_bus::next_request(),
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
                    // A development relay answers the setup with no provider,
                    // so these two reach the keys faces a provider sign-in
                    // reaches in a deployment (`CHROME-098`).
                    <button
                        class="ambre-signin-link ambre-signin-setup"
                        on:click=move |_| keys.open(crate::account::KeysFace::Setup)
                    >
                        "Set up an account with a passphrase"
                    </button>
                    <button
                        class="ambre-signin-link ambre-signin-recover"
                        on:click=move |_| keys.open(crate::account::KeysFace::Recover)
                    >
                        "Recover an account with its recovery words"
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
                            to_logic.send(LogicCommand::Rest {
                                id: ambre_bus::next_request(),
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
        let (tx, rx) = ambre_bus::conduit::local::<LogicCommand>(ambre_core::Bounded::<
            ambre_bus::conduit::ChannelSlots,
        >::of::<8>());
        let dev_mode = RwSignal::new(Some(SignInMode::Oidc));
        let before = web_sys::window().unwrap().location().href().unwrap();

        let container = mount_in_container(move || {
            sign_in_panel(
                dev_mode,
                crate::account::KeysSignals::new(),
                crate::outbox::ShellOutbox::new(tx),
            )
        });
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
        let (tx, rx) = ambre_bus::conduit::local::<LogicCommand>(ambre_core::Bounded::<
            ambre_bus::conduit::ChannelSlots,
        >::of::<8>());
        let before = web_sys::window().unwrap().location().href().unwrap();

        let container =
            mount_in_container(move || sign_out_control(crate::outbox::ShellOutbox::new(tx)));
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
        let named = mount_in_container(|| {
            card_name(Some(
                ambre_core::Validated::<ambre_core::Name>::parse("Budget 2026").unwrap(),
            ))
        });
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
        let pending = RwSignal::new(None::<(Id<Document>, ComposedName)>);
        let confirmed = RwSignal::new(Vec::<Id<Document>>::new());
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

        let id = Id::<Document>::mint();
        let payroll = || {
            ComposedName::or_placeholder(Some(
                &ambre_core::Validated::<ambre_core::Name>::parse("Payroll").unwrap(),
            ))
        };
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
        let (tx, _rx) = ambre_bus::conduit::local::<LogicCommand>(ambre_core::Bounded::<
            ambre_bus::conduit::ChannelSlots,
        >::of::<8>());
        let dev_mode = RwSignal::new(None);
        let container = mount_in_container(move || {
            sign_in_panel(
                dev_mode,
                crate::account::KeysSignals::new(),
                crate::outbox::ShellOutbox::new(tx),
            )
        });
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
            let id = Id::<Document>::mint();
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
        let (tx, rx) = ambre_bus::conduit::local::<LogicCommand>(ambre_core::Bounded::<
            ambre_bus::conduit::ChannelSlots,
        >::of::<8>());
        let toasts = Toasts::new();
        let item = DocItem {
            document_id: Id::<Document>::mint(),
            doc_type: DocType::Prose,
            name: Some(ambre_core::Validated::<ambre_core::Name>::parse("Plan").unwrap()),
            updated_at: None,
            member_count: ambre_core::Count::<ambre_core::Members>::ONE,
        };
        let docs = RwSignal::new(vec![item.clone()]);
        let pending_delete = RwSignal::new(None);
        let container = mount_in_container(move || {
            view! {
                <leptos_router::components::Router>
                    {doc_card(item, docs, crate::outbox::ShellOutbox::new(tx), pending_delete, DocType::Prose, toasts)}
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
        assert_eq!(
            docs.get_untracked()[0]
                .name
                .as_ref()
                .map(ambre_core::Validated::as_str),
            Some("Plan")
        );

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

    /// Three documents, two written today and one forty days ago, in the list
    /// mounted with its search field. The receiver hears whatever the list
    /// sends.
    fn mounted_list() -> (web_sys::Element, ambre_bus::conduit::Receiver<LogicCommand>) {
        let (tx, rx) = ambre_bus::conduit::local::<LogicCommand>(ambre_core::Bounded::<
            ambre_bus::conduit::ChannelSlots,
        >::of::<8>());
        let now_ms = js_sys::Date::now() as u64;
        let item = |name: &str, age_ms: u64| DocItem {
            document_id: Id::<Document>::mint(),
            doc_type: DocType::Prose,
            name: Some(ambre_core::Validated::<ambre_core::Name>::parse(name).unwrap()),
            updated_at: Some(ambre_core::Instant::<Relayed>::new(now_ms - age_ms)),
            member_count: ambre_core::Count::<ambre_core::Members>::ONE,
        };
        let docs = RwSignal::new(vec![
            item("Old budget", 40 * 86_400_000),
            item("Payroll", 7_200_000),
            item("Budget 2026", 3_600_000),
        ]);
        let pending_delete = RwSignal::new(None);
        let toasts = Toasts::new();
        let container = mount_in_container(move || {
            view! {
                <leptos_router::components::Router>
                    {document_list(docs, crate::outbox::ShellOutbox::new(tx), pending_delete, DocType::Prose, toasts)}
                </leptos_router::components::Router>
            }
        });
        (container, rx)
    }

    fn search_field(container: &web_sys::Element) -> web_sys::HtmlInputElement {
        container
            .query_selector(".ambre-doc-search-input")
            .unwrap()
            .expect("the search field")
            .dyn_into()
            .unwrap()
    }

    fn type_query(field: &web_sys::HtmlInputElement, text: &str) {
        field.set_value(text);
        field
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
    }

    /// The text of every node `selector` finds, in document order.
    fn texts(container: &web_sys::Element, selector: &str) -> Vec<String> {
        let nodes = container.query_selector_all(selector).unwrap();
        (0..nodes.length())
            .filter_map(|i| nodes.item(i))
            .map(|n| n.text_content().unwrap_or_default())
            .collect()
    }

    /// The list shows its bands under their headings, and a query keeps the
    /// names that contain it in those bands (`CHROME-118`, `CHROME-119`,
    /// `CHROME-120`). A query that answers nothing says so, never the
    /// invitation of an empty list (`CHROME-124`). No keystroke leaves the UI
    /// thread (`CHROME-122`).
    #[wasm_bindgen_test]
    async fn a_search_filters_the_bands_and_sends_nothing() {
        let owner = Owner::new();
        owner.set();
        let (container, rx) = mounted_list();
        tick().await;
        assert_eq!(
            texts(&container, ".ambre-doc-group-heading"),
            vec!["Past day".to_owned(), "Older".to_owned()]
        );
        assert_eq!(
            texts(&container, ".ambre-doc-name"),
            vec![
                "Budget 2026".to_owned(),
                "Payroll".to_owned(),
                "Old budget".to_owned()
            ]
        );

        let field = search_field(&container);
        type_query(&field, "BUDGET");
        tick().await;
        assert_eq!(
            texts(&container, ".ambre-doc-name"),
            vec!["Budget 2026".to_owned(), "Old budget".to_owned()]
        );
        assert_eq!(
            texts(&container, ".ambre-doc-group-heading"),
            vec!["Past day".to_owned(), "Older".to_owned()]
        );
        assert_eq!(
            texts(&container, ".ambre-doc-empty"),
            vec!["2 documents".to_owned()]
        );

        type_query(&field, "invoice");
        tick().await;
        assert!(texts(&container, ".ambre-doc-name").is_empty());
        assert!(texts(&container, ".ambre-doc-group-heading").is_empty());
        let empty = texts(&container, ".ambre-doc-empty");
        assert_eq!(
            empty,
            vec!["No document name contains this search.".to_owned()],
            "the search says it answers nothing"
        );
        assert_eq!(rx.try_iter().count(), 0, "a search sends nothing");
        container.remove();
    }

    /// The search field carries an accessible name inside a search landmark,
    /// and the count a query lists is announced politely (`CHROME-123`).
    /// Escape empties the query and lists every document again
    /// (`CHROME-125`).
    #[wasm_bindgen_test]
    async fn the_search_field_is_named_announces_its_count_and_clears_on_escape() {
        let owner = Owner::new();
        owner.set();
        let (container, _rx) = mounted_list();
        tick().await;
        let field = search_field(&container);
        assert_eq!(
            field.get_attribute("aria-label").as_deref(),
            Some("Search documents by name")
        );
        assert_eq!(field.type_(), "search");
        let landmark = field.parent_element().unwrap();
        assert_eq!(landmark.get_attribute("role").as_deref(), Some("search"));
        let status = container
            .query_selector("[role='status']")
            .unwrap()
            .unwrap();
        assert_eq!(
            status.text_content().as_deref(),
            Some(""),
            "nothing is announced before a search"
        );

        type_query(&field, "budget");
        tick().await;
        assert_eq!(status.text_content().as_deref(), Some("2 documents"));
        type_query(&field, "payroll");
        tick().await;
        assert_eq!(status.text_content().as_deref(), Some("1 document"));
        type_query(&field, "invoice");
        tick().await;
        assert_eq!(
            status.text_content().as_deref(),
            Some("No document name contains this search.")
        );

        press(&field, "Escape");
        tick().await;
        assert_eq!(field.value(), "", "Escape empties the field");
        assert_eq!(texts(&container, ".ambre-doc-name").len(), 3);
        assert_eq!(status.text_content().as_deref(), Some(""));
        container.remove();
    }
}
