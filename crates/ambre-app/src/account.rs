// ambre-app/src/account.rs
// ============================================================================
// The account keys on the launcher (`ambre-spec-launcher.md` section 1.6).
//
// A setup creates the keys, then shows the recovery words once (`CHROME-098`,
// `CHROME-099`). A recovery reads the words back and hands them to the Sync
// worker. The worker refuses a phrase that spells no seed, and the form shows
// why (`ZK-043`, `CHROME-128`, `CHROME-130`).
//
// The words reach this thread inside a `Confidential<RecoveryWords>`. Only
// `present_words` writes them, into the list of the face. They leave this
// thread with the face: no signal, store or log keeps them (`CHROME-099`). A
// passphrase leaves its field with the submission (`CHROME-129`).
// ============================================================================

use ambre_bus::{AuthFailure, RestCommand};
use ambre_core::{Confidential, DisplayName, Email, Passphrase, RecoveryWords, Validated};
use ambre_logic::app::LogicCommand;
use leptos::prelude::*;

use crate::outbox::ShellOutbox;

/// The face of the account keys that covers the launcher.
#[derive(Clone, Debug, Default)]
pub enum KeysFace {
    /// No face: the launcher shows its sign-in panel or its list.
    #[default]
    Closed,
    /// The setup form: a name, an address and a passphrase (`CHROME-098`).
    Setup,
    /// The recovery words of the account the setup created, shown once
    /// (`CHROME-099`). The face holds the one copy of this thread.
    Words(Confidential<RecoveryWords>),
    /// The recovery form: the words and a new passphrase (`CHROME-128`).
    Recover,
}

/// The state of the keys faces, which the launcher and the switchboard share.
#[derive(Clone, Copy)]
pub struct KeysSignals {
    /// The face shown, if any.
    pub face: RwSignal<KeysFace>,
    /// The last refusal the Sync worker answered to the form shown.
    pub refusal: RwSignal<Option<AuthFailure>>,
}

impl Default for KeysSignals {
    fn default() -> Self {
        Self::new()
    }
}

impl KeysSignals {
    /// No face shown, and no refusal.
    pub fn new() -> Self {
        Self {
            face: RwSignal::new(KeysFace::Closed),
            refusal: RwSignal::new(None),
        }
    }

    /// Shows `face`, with no refusal of an earlier form.
    pub fn open(self, face: KeysFace) {
        // The face first: the form it replaces unmounts before the refusal
        // clears, so no line of that form reads it again.
        self.face.set(face);
        self.refusal.set(None);
    }

    /// The account owes its key setup (`CHROME-098`). Words on screen stay:
    /// they are shown once, and a late answer must not hide them.
    pub fn owe_setup(self) {
        if !self
            .face
            .with_untracked(|face| matches!(face, KeysFace::Words(_)))
        {
            self.open(KeysFace::Setup);
        }
    }

    /// Shows the recovery words of the account just created (`CHROME-099`).
    pub fn show_words(self, words: Confidential<RecoveryWords>) {
        self.open(KeysFace::Words(words));
    }

    /// The Sync worker refused the form shown: the form says why. A refusal
    /// with no form shown belongs to another flow, and changes nothing here.
    pub fn refuse(self, cause: AuthFailure) {
        if self
            .face
            .with_untracked(|face| matches!(face, KeysFace::Setup | KeysFace::Recover))
        {
            self.refusal.set(Some(cause));
        }
    }

    /// Closes the face. Words on screen leave the view and this thread: the
    /// last copy drops, and wipes its bytes (`CHROME-099`).
    pub fn close(self) {
        self.open(KeysFace::Closed);
    }

    /// Whether a face covers the launcher.
    pub fn is_open(self) -> bool {
        self.face.with(|face| !matches!(face, KeysFace::Closed))
    }
}

/// Why a form refused its own fields, before anything left this thread.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FieldRefusal {
    /// No passphrase was typed.
    Empty,
    /// The passphrase and its confirmation differ.
    Mismatch,
    /// The display name breaks the account rule.
    Name,
}

/// The face of the account keys, or nothing when it is closed.
pub fn keys_view(keys: KeysSignals, to_logic: ShellOutbox) -> impl IntoView {
    move || match keys.face.get() {
        KeysFace::Closed => ().into_any(),
        KeysFace::Setup => setup_form(keys, to_logic.clone()).into_any(),
        KeysFace::Words(words) => words_panel(keys, words).into_any(),
        KeysFace::Recover => recovery_form(keys, to_logic.clone()).into_any(),
    }
}

/// The line that says why a form refused, from this thread or from the Sync
/// worker. The words of a worker refusal are its own (`ambre-bus`).
fn refusal_line(keys: KeysSignals, local: RwSignal<Option<FieldRefusal>>) -> impl IntoView {
    // A line whose form already unmounted reads no refusal of its own.
    let text = move || match local.try_get().flatten() {
        Some(FieldRefusal::Empty) => "Type a passphrase.".to_owned(),
        Some(FieldRefusal::Mismatch) => "The two passphrases do not match.".to_owned(),
        Some(FieldRefusal::Name) => "Type a display name.".to_owned(),
        None => keys
            .refusal
            .get()
            .map(|cause| cause.to_string())
            .unwrap_or_default(),
    };
    view! { <p class="ambre-keys-refusal" role="alert">{text}</p> }
}

/// Reads the passphrase and its confirmation out of their fields, and empties
/// both: the text leaves the page with the submission (`CHROME-129`).
fn take_passphrase(
    first: NodeRef<leptos::html::Input>,
    second: NodeRef<leptos::html::Input>,
) -> Result<Confidential<Passphrase>, FieldRefusal> {
    let take = |field: NodeRef<leptos::html::Input>| {
        field.get_untracked().map(|input| {
            let typed = Confidential::<Passphrase>::new(input.value());
            input.set_value("");
            typed
        })
    };
    let (Some(typed), Some(confirmed)) = (take(first), take(second)) else {
        return Err(FieldRefusal::Empty);
    };
    if typed.is_empty() {
        return Err(FieldRefusal::Empty);
    }
    if !typed.matches(&confirmed) {
        return Err(FieldRefusal::Mismatch);
    }
    Ok(typed)
}

/// The setup form (`CHROME-098`). The Sync worker answers with the words, or
/// with the refusal the form shows.
fn setup_form(keys: KeysSignals, to_logic: ShellOutbox) -> impl IntoView {
    let name = NodeRef::<leptos::html::Input>::new();
    let email = NodeRef::<leptos::html::Input>::new();
    let first = NodeRef::<leptos::html::Input>::new();
    let second = NodeRef::<leptos::html::Input>::new();
    let local = RwSignal::new(None::<FieldRefusal>);
    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        keys.refusal.set(None);
        let passphrase = match take_passphrase(first, second) {
            Ok(passphrase) => passphrase,
            Err(refusal) => {
                local.set(Some(refusal));
                return;
            }
        };
        let (Some(name), Some(email)) = (name.get_untracked(), email.get_untracked()) else {
            return;
        };
        let Ok(display_name) = Validated::<DisplayName>::parse(name.value()) else {
            local.set(Some(FieldRefusal::Name));
            return;
        };
        local.set(None);
        to_logic.send(LogicCommand::Rest {
            id: ambre_bus::next_request(),
            cmd: RestCommand::SetupAccount {
                passphrase,
                email: Email::from(email.value()),
                display_name,
            },
        });
    };
    view! {
        <form class="ambre-keys ambre-keys-setup" on:submit=submit>
            <h2>"Set up your account"</h2>
            <p class="ambre-keys-note">
                "Your passphrase opens your keys on this device. Ambre never sees it."
            </p>
            <input node_ref=name class="ambre-keys-name" aria-label="Display name"
                placeholder="Display name" autocomplete="name" />
            <input node_ref=email class="ambre-keys-email" type="email" aria-label="Email address"
                placeholder="Email address" autocomplete="email" />
            <input node_ref=first class="ambre-keys-passphrase" type="password"
                aria-label="Passphrase" placeholder="Passphrase" autocomplete="new-password" />
            <input node_ref=second class="ambre-keys-confirm" type="password"
                aria-label="Confirm the passphrase" placeholder="Confirm the passphrase"
                autocomplete="new-password" />
            {refusal_line(keys, local)}
            <div class="ambre-keys-actions">
                <button type="button" class="ambre-keys-cancel" on:click=move |_| keys.close()>
                    "Cancel"
                </button>
                <button type="submit" class="ambre-keys-submit">"Create the account"</button>
            </div>
        </form>
    }
}

/// The recovery words, shown once (`CHROME-099`). The list is filled when it
/// mounts, by the one call that reads the words. The confirmation closes the
/// face, which takes the list off the page and drops the words.
fn words_panel(keys: KeysSignals, words: Confidential<RecoveryWords>) -> impl IntoView {
    let list = NodeRef::<leptos::html::Ol>::new();
    list.on_load(move |list| {
        words.present_words(|phrase| {
            let Some(document) = list.owner_document() else {
                return;
            };
            for word in phrase.split(' ') {
                if let Ok(item) = document.create_element("li") {
                    item.set_text_content(Some(word));
                    let _ = list.append_child(&item);
                }
            }
        });
    });
    view! {
        <div class="ambre-keys ambre-keys-words">
            <h2>"Your recovery words"</h2>
            <p class="ambre-keys-note">
                "Write these 24 words down, in order, and keep them offline. They are the \
                 only way back into your account if you forget your passphrase. Ambre shows \
                 them once, and cannot show them again."
            </p>
            <ol node_ref=list class="ambre-recovery-words" aria-label="Recovery words"></ol>
            <div class="ambre-keys-actions">
                <button class="ambre-keys-submit ambre-words-done" on:click=move |_| keys.close()>
                    "I have written them down"
                </button>
            </div>
        </div>
    }
}

/// The recovery form (`CHROME-128`). The words stay in their field after a
/// refusal, so the person corrects one word rather than typing 24 again.
fn recovery_form(keys: KeysSignals, to_logic: ShellOutbox) -> impl IntoView {
    let words = NodeRef::<leptos::html::Textarea>::new();
    let first = NodeRef::<leptos::html::Input>::new();
    let second = NodeRef::<leptos::html::Input>::new();
    let local = RwSignal::new(None::<FieldRefusal>);
    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        keys.refusal.set(None);
        let new_passphrase = match take_passphrase(first, second) {
            Ok(passphrase) => passphrase,
            Err(refusal) => {
                local.set(Some(refusal));
                return;
            }
        };
        let Some(words) = words.get_untracked() else {
            return;
        };
        local.set(None);
        to_logic.send(LogicCommand::Rest {
            id: ambre_bus::next_request(),
            cmd: RestCommand::RecoverAccount {
                recovery_words: Confidential::<RecoveryWords>::new(words.value()),
                new_passphrase,
            },
        });
    };
    view! {
        <form class="ambre-keys ambre-keys-recover" on:submit=submit>
            <h2>"Recover your account"</h2>
            <p class="ambre-keys-note">
                "Type your 24 recovery words in order, then choose a new passphrase."
            </p>
            <textarea node_ref=words class="ambre-keys-words-field" aria-label="Recovery words"
                placeholder="24 recovery words" rows="4" autocomplete="off"
                autocapitalize="off" spellcheck="false"></textarea>
            <input node_ref=first class="ambre-keys-passphrase" type="password"
                aria-label="New passphrase" placeholder="New passphrase"
                autocomplete="new-password" />
            <input node_ref=second class="ambre-keys-confirm" type="password"
                aria-label="Confirm the new passphrase" placeholder="Confirm the new passphrase"
                autocomplete="new-password" />
            {refusal_line(keys, local)}
            <div class="ambre-keys-actions">
                <button type="button" class="ambre-keys-cancel" on:click=move |_| keys.close()>
                    "Cancel"
                </button>
                <button type="submit" class="ambre-keys-submit">"Recover the account"</button>
            </div>
        </form>
    }
}

// -- Tests (wasm tier) -------------------------------------------------------
// The crate is `#![cfg(target_arch = "wasm32")]`; these mount each face into a
// real headless Chrome document (`just test-wasm`).
#[cfg(test)]
mod tests {
    use super::*;
    use ambre_core::RecoveryWordsRefusal;
    use leptos::mount::mount_to;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    /// The phrase of the all-zero seed, a BIP39 vector.
    const PHRASE: &str = "abandon abandon abandon abandon abandon abandon abandon abandon \
        abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon \
        abandon abandon abandon abandon abandon art";

    fn document() -> web_sys::Document {
        web_sys::window().unwrap().document().unwrap()
    }

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

    async fn tick() {
        let promise = js_sys::Promise::new(&mut |resolve, _| {
            web_sys::window()
                .unwrap()
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 0)
                .unwrap();
        });
        let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
    }

    fn outbox() -> (ShellOutbox, ambre_bus::conduit::Receiver<LogicCommand>) {
        let (tx, rx) = ambre_bus::conduit::local::<LogicCommand>(ambre_core::Bounded::<
            ambre_bus::conduit::ChannelSlots,
        >::of::<8>());
        (ShellOutbox::new(tx), rx)
    }

    fn find(container: &web_sys::Element, selector: &str) -> Option<web_sys::Element> {
        container.query_selector(selector).unwrap()
    }

    fn type_into(container: &web_sys::Element, selector: &str, text: &str) {
        let field = find(container, selector).unwrap();
        match field.dyn_ref::<web_sys::HtmlInputElement>() {
            Some(input) => input.set_value(text),
            None => field
                .dyn_ref::<web_sys::HtmlTextAreaElement>()
                .unwrap()
                .set_value(text),
        }
    }

    fn value(container: &web_sys::Element, selector: &str) -> String {
        find(container, selector)
            .unwrap()
            .dyn_into::<web_sys::HtmlInputElement>()
            .unwrap()
            .value()
    }

    fn submit(container: &web_sys::Element) {
        find(container, ".ambre-keys-submit")
            .unwrap()
            .dyn_into::<web_sys::HtmlElement>()
            .unwrap()
            .click();
    }

    /// The words show once, in order, and leave the page and the view with
    /// the confirmation (`CHROME-099`).
    #[wasm_bindgen_test]
    async fn the_words_show_once_and_leave_with_the_confirmation() {
        let owner = Owner::new();
        owner.set();
        let keys = KeysSignals::new();
        let (to_logic, _rx) = outbox();
        let container = mount_in_container(move || keys_view(keys, to_logic));
        keys.show_words(Confidential::<RecoveryWords>::new(PHRASE.to_owned()));
        tick().await;

        let list = find(&container, ".ambre-recovery-words").unwrap();
        let items = list.query_selector_all("li").unwrap();
        assert_eq!(items.length(), 24);
        let shown: Vec<String> = (0..items.length())
            .map(|i| items.item(i).unwrap().text_content().unwrap())
            .collect();
        assert_eq!(
            shown.join(" "),
            PHRASE.split_whitespace().collect::<Vec<_>>().join(" ")
        );

        // A late setup answer leaves the words on screen.
        keys.owe_setup();
        tick().await;
        assert!(find(&container, ".ambre-recovery-words").is_some());

        find(&container, ".ambre-words-done")
            .unwrap()
            .dyn_into::<web_sys::HtmlElement>()
            .unwrap()
            .click();
        tick().await;
        assert!(find(&container, ".ambre-recovery-words").is_none());
        assert!(!container.text_content().unwrap().contains("art"));
        assert!(
            keys.face
                .with_untracked(|face| matches!(face, KeysFace::Closed))
        );
        container.remove();
    }

    /// The setup form sends one setup, and its passphrase leaves both fields
    /// with the submission (`CHROME-098`, `CHROME-129`).
    #[wasm_bindgen_test]
    async fn the_setup_sends_the_passphrase_and_empties_its_fields() {
        let owner = Owner::new();
        owner.set();
        let keys = KeysSignals::new();
        let (to_logic, rx) = outbox();
        let container = mount_in_container(move || keys_view(keys, to_logic));
        keys.owe_setup();
        tick().await;

        type_into(&container, ".ambre-keys-name", "Ada");
        type_into(&container, ".ambre-keys-email", "ada@test.local");
        type_into(
            &container,
            ".ambre-keys-passphrase",
            "correct horse battery staple",
        );
        type_into(
            &container,
            ".ambre-keys-confirm",
            "correct horse battery staple",
        );
        submit(&container);
        tick().await;

        assert_eq!(value(&container, ".ambre-keys-passphrase"), "");
        assert_eq!(value(&container, ".ambre-keys-confirm"), "");
        let sent: Vec<LogicCommand> = rx.try_iter().collect();
        assert!(matches!(
            sent.as_slice(),
            [LogicCommand::Rest { cmd: RestCommand::SetupAccount { display_name, .. }, .. }]
                if display_name.as_str() == "Ada"
        ));

        // A confirmation that differs sends nothing, and says why.
        type_into(
            &container,
            ".ambre-keys-passphrase",
            "correct horse battery staple",
        );
        type_into(&container, ".ambre-keys-confirm", "correct horse battery");
        submit(&container);
        tick().await;
        assert_eq!(value(&container, ".ambre-keys-passphrase"), "");
        assert!(rx.try_iter().next().is_none());
        assert_eq!(
            find(&container, ".ambre-keys-refusal")
                .unwrap()
                .text_content()
                .unwrap(),
            "The two passphrases do not match."
        );
        container.remove();
    }

    /// The recovery form sends the words it reads, and shows the refusal
    /// the Sync worker answers in words (`CHROME-128`, `CHROME-130`).
    #[wasm_bindgen_test]
    async fn the_recovery_sends_the_words_and_shows_the_refusal() {
        let owner = Owner::new();
        owner.set();
        let keys = KeysSignals::new();
        let (to_logic, rx) = outbox();
        let container = mount_in_container(move || keys_view(keys, to_logic));
        keys.open(KeysFace::Recover);
        tick().await;

        type_into(&container, ".ambre-keys-words-field", PHRASE);
        type_into(
            &container,
            ".ambre-keys-passphrase",
            "correct horse battery staple",
        );
        type_into(
            &container,
            ".ambre-keys-confirm",
            "correct horse battery staple",
        );
        submit(&container);
        tick().await;
        let sent: Vec<LogicCommand> = rx.try_iter().collect();
        assert!(matches!(
            sent.as_slice(),
            [LogicCommand::Rest {
                cmd: RestCommand::RecoverAccount { .. },
                ..
            }]
        ));
        assert_eq!(value(&container, ".ambre-keys-passphrase"), "");

        let cause = AuthFailure::BadRecoveryWords(RecoveryWordsRefusal::UnknownWord);
        keys.refuse(cause);
        tick().await;
        assert_eq!(
            find(&container, ".ambre-keys-refusal")
                .unwrap()
                .text_content()
                .unwrap(),
            cause.to_string()
        );

        // A refusal with no form shown belongs to another flow.
        keys.close();
        keys.refuse(cause);
        assert_eq!(keys.refusal.get_untracked(), None);
        container.remove();
    }
}
