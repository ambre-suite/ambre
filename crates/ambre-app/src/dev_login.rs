//! The development sign-in of the page (`ZK-045`): the `?user=` read, the
//! remembered name and the sign-in field. A bare name derives every key of
//! its account, so each item here sits behind the `dev-login` feature, which
//! no release build turns on. Without the feature, each function does
//! nothing, and the page sends no `DevLogin` whatever the relay answers.

#[cfg(feature = "dev-login")]
use ambre_bus::RestCommand;
#[cfg(feature = "dev-login")]
use ambre_logic::app::LogicCommand;
use leptos::prelude::*;

use crate::outbox::ShellOutbox;

/// Resolve the dev login identity from `?user=<name>` in the boot URL.
///
/// The engine lives for the whole session (SPA). A single boot-time
/// `DevLogin` therefore derives the session keys once, and they persist
/// across client navigations. The sign-in field dispatches `DevLogin`
/// directly thereafter.
///
/// `None` when the address names no user, or a text that breaks the rule of
/// a display name.
#[cfg(feature = "dev-login")]
fn user_from_url() -> Option<ambre_core::Validated<ambre_core::DisplayName>> {
    let window = web_sys::window()?;
    window
        .location()
        .search()
        .ok()
        .and_then(|s| web_sys::UrlSearchParams::new_with_str(&s).ok())
        .and_then(|p| p.get("user"))
        .and_then(|u| ambre_core::Validated::<ambre_core::DisplayName>::parse(u).ok())
}

/// Read the dev username remembered from a prior session, if any.
#[cfg(feature = "dev-login")]
pub(crate) fn stored_user() -> Option<ambre_core::Validated<ambre_core::DisplayName>> {
    crate::platform::stored_dev_user()
}

/// Remember `username` as the active dev identity.
#[cfg(feature = "dev-login")]
pub(crate) fn remember_user(username: &ambre_core::Validated<ambre_core::DisplayName>) {
    crate::platform::remember_dev_user(username);
}

/// Forget the remembered dev identity, so the next boot signs no one in.
#[cfg(feature = "dev-login")]
pub(crate) fn forget_user() {
    crate::platform::forget_dev_user();
}

/// A build without the development sign-in stores no name to forget.
#[cfg(not(feature = "dev-login"))]
pub(crate) fn forget_user() {}

/// The boot sign-in: `?user=` wins, else the dev user remembered from a
/// prior session. A refresh of a deep link carries no query string, and
/// still re-establishes the session. A relay outside development mode
/// refuses the `DevLogin` this sends. Answers [`BootSignIn::Sent`] when a
/// name signs the page in.
#[cfg(feature = "dev-login")]
pub(crate) fn sign_in_at_boot(outbox: &ShellOutbox) -> BootSignIn {
    LOGIC.with(|slot| *slot.borrow_mut() = Some(outbox.clone()));
    let Some(username) = user_from_url().or_else(stored_user) else {
        return BootSignIn::None;
    };
    tracing::info!("Dev login requested for user {username:?}");
    remember_user(&username);
    outbox.send(LogicCommand::Rest {
        id: ambre_bus::next_request(),
        cmd: RestCommand::DevLogin { username },
    });
    BootSignIn::Sent
}

#[cfg(feature = "dev-login")]
thread_local! {
    /// The writer to the Logic thread that [`dev_interface_language`] sends on.
    static LOGIC: std::cell::RefCell<Option<ShellOutbox>> = const { std::cell::RefCell::new(None) };
}

/// Sets the interface language of the account to `tag`, a BCP 47 tag. No
/// control of the page sets the language yet, so the browser belt calls this
/// export to prove that the language follows the account (`SETTINGS-024`). A
/// tag that does not parse sets nothing. Only a `dev-login` build exports it.
#[cfg(feature = "dev-login")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn dev_interface_language(tag: &str) {
    let Ok(language) = ambre_client_core::DocumentLocale::parse(tag) else {
        tracing::warn!("Dev: the interface language names no tag");
        return;
    };
    let command = LogicCommand::ChangeSetting {
        change: ambre_client_core::SettingChange::InterfaceLanguage(language),
    };
    LOGIC.with(|slot| {
        if let Some(logic) = slot.borrow().as_ref() {
            logic.send(command);
        }
    });
}

/// A build without the development sign-in reads no `?user=` and no stored
/// name at boot (`ZK-045`).
#[cfg(not(feature = "dev-login"))]
pub(crate) fn sign_in_at_boot(_outbox: &ShellOutbox) -> BootSignIn {
    BootSignIn::None
}

/// Whether the development sign-in signed the page in at boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BootSignIn {
    /// No name: the page resumes the session the device keeps, if any.
    None,
    /// A name signs the page in.
    #[cfg_attr(
        not(feature = "dev-login"),
        expect(dead_code, reason = "only the development sign-in sends a name")
    )]
    Sent,
}

/// The sign-in field of the development panel: a username, sent as
/// `DevLogin` to the live Logic thread with no reload. The relay creates the
/// user on the fly, and the session reply reveals the document list.
#[cfg(feature = "dev-login")]
pub(crate) fn sign_in_field(to_logic: ShellOutbox) -> AnyView {
    let value = RwSignal::new("alice".to_owned());
    let go = move || {
        // A name the account rule refuses, a blank one among them,
        // sends nothing: the relay would refuse it (`SRV-234`).
        if let Ok(name) = ambre_core::Validated::<ambre_core::DisplayName>::parse(value.get()) {
            // Remember the dev identity so a later refresh re-establishes
            // the session without `?user=` in the URL.
            remember_user(&name);
            to_logic.send(LogicCommand::Rest {
                id: ambre_bus::next_request(),
                cmd: RestCommand::DevLogin { username: name },
            });
        }
    };
    view! {
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
    }
    .into_any()
}

/// A build without the development sign-in shows no name field: the
/// development panel keeps its passphrase setup alone (`ZK-045`).
#[cfg(not(feature = "dev-login"))]
pub(crate) fn sign_in_field(_to_logic: ShellOutbox) -> AnyView {
    ().into_any()
}
