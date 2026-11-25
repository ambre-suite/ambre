// ambre-app/src/account.rs
// ============================================================================
// The account keys on the launcher (`ambre-spec-launcher.md` section 1.6).
//
// A key setup on the gate creates the keys, then the words show here once
// (`CHROME-098`, `CHROME-099`). A recovery reads the words back and hands them
// to the Sync worker. The worker refuses a phrase that spells no seed, and the
// form shows why (`ZK-043`, `CHROME-128`, `CHROME-130`). No face creates an account: an
// account comes from a provider, or from the development sign-in (`SRV-319`).
//
// The passphrase form of the account settings changes the passphrase of a
// session. It takes the current passphrase, then the new one twice, and shows
// each refusal of the Sync worker in its own words (`CHROME-145`).
//
// The words reach this thread inside a `Confidential<RecoveryWords>`. Only
// `present_words` writes them, into the list of the face. They leave this
// thread with the face: no signal, store or log keeps them (`CHROME-099`). A
// passphrase leaves its field with the submission (`CHROME-129`).
//
// A provider sign-in may reclaim the address of the account (`SRV-029`). The
// launcher then says so, and says that shares to the address now reach the
// verified account (`CHROME-146`).
// ============================================================================

use ambre_bus::{AuthFailure, RestCommand};
use ambre_chrome::commands::Locale;
use ambre_core::{AddressStanding, Confidential, Passphrase, RecoveryWords};
use ambre_logic::app::LogicCommand;
use leptos::prelude::*;

use crate::outbox::ShellOutbox;

/// The face of the account keys that covers the launcher.
#[derive(Clone, Debug, Default)]
pub enum KeysFace {
    /// No face: the launcher shows its sign-in panel or its list.
    #[default]
    Closed,
    /// The recovery words of the keys a setup created, shown once
    /// (`CHROME-099`). The face holds the one copy of this thread.
    Words(Confidential<RecoveryWords>),
    /// The recovery form: the words and a new passphrase (`CHROME-128`).
    Recover,
    /// The passphrase form of the account settings: the current passphrase
    /// and a new one (`CHROME-145`).
    ChangePassphrase,
}

ambre_core::define_flag! {
    /// Whether the launcher shows under the keys faces.
    pub enum Launcher {
        /// The launcher shows: no face, or the words in a modal over it.
        Shown,
        /// The recovery form replaces it.
        Covered,
    }
}

/// The state of the keys faces, which the launcher and the switchboard share.
#[derive(Clone, Copy)]
pub struct KeysSignals {
    /// The face shown, if any.
    pub face: RwSignal<KeysFace>,
    /// The last refusal the Sync worker answered to the form shown.
    pub refusal: RwSignal<Option<AuthFailure>>,
    /// Whether a provider sign-in reclaimed the address of the account
    /// (`CHROME-146`). A page boots afresh on a sign-out, so a new session
    /// starts from `Held`.
    pub address: RwSignal<AddressStanding>,
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
            address: RwSignal::new(AddressStanding::Held),
        }
    }

    /// The relay said that a provider sign-in reclaimed the address of the
    /// account: the launcher names the reclaim (`CHROME-146`).
    pub fn address_reclaimed(self) {
        self.address.set(AddressStanding::Reclaimed);
    }

    /// Shows `face`, with no refusal of an earlier form.
    pub fn open(self, face: KeysFace) {
        // The face first: the form it replaces unmounts before the refusal
        // clears, so no line of that form reads it again.
        self.face.set(face);
        self.refusal.set(None);
    }

    /// Shows the recovery words of the account just created (`CHROME-099`).
    pub fn show_words(self, words: Confidential<RecoveryWords>) {
        self.open(KeysFace::Words(words));
    }

    /// The Sync worker refused the form shown: the form says why. A refusal
    /// with no form shown belongs to another flow, and changes nothing here.
    pub fn refuse(self, cause: AuthFailure) {
        if self.has_form() {
            self.refusal.set(Some(cause));
        }
    }

    /// Closes the face. Words on screen leave the view and this thread: the
    /// last copy drops, and wipes its bytes (`CHROME-099`).
    pub fn close(self) {
        self.open(KeysFace::Closed);
    }

    /// Whether a form shows, the recovery or the passphrase change:
    /// a refusal of the Sync worker then reads on that form.
    pub fn has_form(self) -> bool {
        self.face
            .with_untracked(|face| matches!(face, KeysFace::Recover | KeysFace::ChangePassphrase))
    }

    /// Whether a form replaces the launcher. The recovery words show in a
    /// modal over it instead, so the list of the new session stays under
    /// them (`CHROME-137`).
    pub fn launcher(self) -> Launcher {
        self.face.with(|face| match face {
            KeysFace::Recover | KeysFace::ChangePassphrase => Launcher::Covered,
            KeysFace::Closed | KeysFace::Words(_) => Launcher::Shown,
        })
    }
}

/// One line of the reclaim notice (`CHROME-146`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ReclaimLine {
    /// The fact: the address left the account.
    Title,
    /// What the fact means for a share to the address.
    Meaning,
}

impl ReclaimLine {
    /// The words of the line in `locale`. A function of the locale, as a
    /// command title is (`ambre-spec-commands.md`): the view supplies the
    /// language, and this table owns the words.
    fn text(self, locale: Locale) -> &'static str {
        match (self, locale) {
            (Self::Title, Locale::En) => "Your address was reclaimed",
            (Self::Title, Locale::Fr) => "Votre adresse a \u{e9}t\u{e9} reprise",
            (Self::Meaning, Locale::En) => {
                "An account that its provider verified now holds this address. Documents \
                 shared with the address now reach that account, not yours. Your documents \
                 and keys stay with you."
            }
            (Self::Meaning, Locale::Fr) => {
                "Un compte v\u{e9}rifi\u{e9} par son fournisseur d\u{e9}tient \
                 d\u{e9}sormais cette adresse. Les documents partag\u{e9}s avec cette \
                 adresse parviennent \u{e0} ce compte, plus au v\u{f4}tre. Vos documents \
                 et vos cl\u{e9}s restent les v\u{f4}tres."
            }
        }
    }
}

/// The reclaim notice of the launcher, or nothing while the account holds its
/// address (`CHROME-146`). It names the reclaim, then what a share to the
/// address now reaches. It stays for the whole session: a toast would leave
/// before a person reads it.
pub fn reclaim_notice(keys: KeysSignals) -> impl IntoView {
    move || match keys.address.get() {
        AddressStanding::Held => ().into_any(),
        AddressStanding::Reclaimed => view! {
            <section class="ambre-address-reclaimed" role="alert">
                <h2>{move || ReclaimLine::Title.text(ambre_chrome::language())}</h2>
                <p>{move || ReclaimLine::Meaning.text(ambre_chrome::language())}</p>
            </section>
        }
        .into_any(),
    }
}

/// Why a form refused its own fields, before anything left this thread.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FieldRefusal {
    /// No passphrase was typed.
    Empty,
    /// The passphrase and its confirmation differ.
    Mismatch,
}

/// The face of the account keys, or nothing when it is closed.
pub fn keys_view(keys: KeysSignals, to_logic: ShellOutbox) -> impl IntoView {
    move || match keys.face.get() {
        KeysFace::Closed => ().into_any(),
        KeysFace::Words(words) => words_dialog(keys, words).into_any(),
        KeysFace::Recover => recovery_form(keys, to_logic.clone()).into_any(),
        KeysFace::ChangePassphrase => change_form(keys, to_logic.clone()).into_any(),
    }
}

/// The line that says why a form refused, from this thread or from the Sync
/// worker. The words of a worker refusal are its own (`ambre-bus`).
fn refusal_line(keys: KeysSignals, local: RwSignal<Option<FieldRefusal>>) -> impl IntoView {
    // A line whose form already unmounted reads no refusal of its own.
    let text = move || match local.try_get().flatten() {
        Some(FieldRefusal::Empty) => "Type a passphrase.".to_owned(),
        Some(FieldRefusal::Mismatch) => "The two passphrases do not match.".to_owned(),
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

/// Reads the current passphrase out of its field, and empties it: the text
/// leaves the page with the submission (`CHROME-129`).
fn take_current(
    field: NodeRef<leptos::html::Input>,
) -> Result<Confidential<Passphrase>, FieldRefusal> {
    let Some(input) = field.get_untracked() else {
        return Err(FieldRefusal::Empty);
    };
    let typed = Confidential::<Passphrase>::new(input.value());
    input.set_value("");
    if typed.is_empty() {
        return Err(FieldRefusal::Empty);
    }
    Ok(typed)
}

/// The passphrase form of the account settings (`CHROME-145`). The Sync
/// worker answers `PassphraseChanged`, which closes the form. Or it answers a
/// refusal the form shows (`CHROME-143`). That is a wrong current passphrase,
/// a weak new one, or a stale seal, which asks for a new try (`SYNC-295`).
fn change_form(keys: KeysSignals, to_logic: ShellOutbox) -> impl IntoView {
    let current = NodeRef::<leptos::html::Input>::new();
    let first = NodeRef::<leptos::html::Input>::new();
    let second = NodeRef::<leptos::html::Input>::new();
    let local = RwSignal::new(None::<FieldRefusal>);
    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        keys.refusal.set(None);
        // Every field empties, a refused one included (`CHROME-129`).
        let current_passphrase = take_current(current);
        let new_passphrase = take_passphrase(first, second);
        let (current_passphrase, new_passphrase) = match (current_passphrase, new_passphrase) {
            (Ok(current), Ok(new)) => (current, new),
            (Err(refusal), _) | (_, Err(refusal)) => {
                local.set(Some(refusal));
                return;
            }
        };
        local.set(None);
        to_logic.send(LogicCommand::Rest {
            id: ambre_bus::next_request(),
            cmd: RestCommand::ChangePassphrase {
                current_passphrase,
                new_passphrase,
            },
        });
    };
    view! {
        <form class="ambre-keys ambre-keys-change" on:submit=submit>
            <h2>"Change your passphrase"</h2>
            <p class="ambre-keys-note">
                "Type your current passphrase, then choose a new one. Your recovery words \
                 stay the same."
            </p>
            <input node_ref=current class="ambre-keys-current" type="password"
                aria-label="Current passphrase" placeholder="Current passphrase"
                autocomplete="current-password" />
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
                <button type="submit" class="ambre-keys-submit">"Change the passphrase"</button>
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

/// The recovery words in the shared modal over the launcher (`CHROME-137`).
/// Only their confirmation closes it: Escape or a stray click on the backdrop
/// would drop words shown once.
fn words_dialog(keys: KeysSignals, words: Confidential<RecoveryWords>) -> impl IntoView {
    let open = RwSignal::new(ambre_chrome::Openness::Open);
    // The dialog asks its children as a function. The slot hands the words
    // over once, so no second copy waits in it.
    let slot = std::sync::Mutex::new(Some(words));
    view! {
        <ambre_chrome::Dialog
            open=open
            dismissal=ambre_chrome::Dismissal::Explicit
            class={ambre_core::StaticKey::<ambre_core::CssClasses>::new("ambre-words-dialog")}
        >
            {slot
                .lock()
                .ok()
                .and_then(|mut words| words.take())
                .map(|words| words_panel(keys, words))}
        </ambre_chrome::Dialog>
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

    /// `CHROME-137`: the words show in an open modal over the launcher,
    /// which they leave in place. A native close, the one Escape performs,
    /// keeps them on screen. Only their confirmation closes the modal. The
    /// recovery form still replaces the launcher.
    #[wasm_bindgen_test]
    async fn the_words_open_in_a_modal_that_only_their_confirmation_closes() {
        let owner = Owner::new();
        owner.set();
        let keys = KeysSignals::new();
        let (to_logic, _rx) = outbox();
        let container = mount_in_container(move || keys_view(keys, to_logic));
        keys.open(KeysFace::Recover);
        assert_eq!(keys.launcher(), Launcher::Covered, "the recovery");

        keys.show_words(Confidential::<RecoveryWords>::new(PHRASE.to_owned()));
        tick().await;
        assert_eq!(
            keys.launcher(),
            Launcher::Shown,
            "the words hide the launcher"
        );
        let modal: web_sys::HtmlDialogElement = find(&container, "dialog.ambre-words-dialog")
            .expect("the words show in a modal")
            .dyn_into()
            .unwrap();
        assert!(modal.open());
        assert!(find(&container, "dialog[open] .ambre-recovery-words li").is_some());

        // The `close` event is queued: wait for it, after the modal's own
        // handler, which was registered first.
        let closed = js_sys::Promise::new(&mut |resolve, _| {
            modal
                .add_event_listener_with_callback("close", &resolve)
                .unwrap();
        });
        modal.close();
        let _ = wasm_bindgen_futures::JsFuture::from(closed).await;
        assert!(modal.open(), "a native close dropped the words");
        assert!(find(&container, ".ambre-recovery-words li").is_some());

        find(&container, ".ambre-words-done")
            .unwrap()
            .dyn_into::<web_sys::HtmlElement>()
            .unwrap()
            .click();
        tick().await;
        assert!(
            find(&container, "dialog").is_none(),
            "the modal outlived the words"
        );
        container.remove();
    }

    /// `CHROME-145`, `CHROME-143`, `CHROME-129`: the passphrase form sends
    /// the current and the new passphrase once, and empties its three fields.
    /// Each refusal of the Sync worker reads on the form in its own words. The
    /// refusals are a wrong current passphrase, a weak new one and a stale
    /// re-seal. Fields that refuse send nothing.
    #[wasm_bindgen_test]
    async fn the_passphrase_change_sends_both_passphrases_and_shows_each_refusal() {
        let owner = Owner::new();
        owner.set();
        let keys = KeysSignals::new();
        let (to_logic, rx) = outbox();
        let container = mount_in_container(move || keys_view(keys, to_logic));
        keys.open(KeysFace::ChangePassphrase);
        tick().await;
        assert_eq!(keys.launcher(), Launcher::Covered);
        assert!(keys.has_form());

        let fill = |current: &str, new: &str, again: &str| {
            type_into(&container, ".ambre-keys-current", current);
            type_into(&container, ".ambre-keys-passphrase", new);
            type_into(&container, ".ambre-keys-confirm", again);
        };
        fill(
            "pelican harbor voltage minnow",
            "anchovy glider ruckus tadpole",
            "anchovy glider ruckus tadpole",
        );
        submit(&container);
        tick().await;
        for field in [
            ".ambre-keys-current",
            ".ambre-keys-passphrase",
            ".ambre-keys-confirm",
        ] {
            assert_eq!(
                value(&container, field),
                "",
                "{field} outlived the submission"
            );
        }
        let sent: Vec<LogicCommand> = rx.try_iter().collect();
        match sent.as_slice() {
            [
                LogicCommand::Rest {
                    cmd:
                        RestCommand::ChangePassphrase {
                            current_passphrase,
                            new_passphrase,
                        },
                    ..
                },
            ] => {
                assert!(current_passphrase.matches(&Confidential::<Passphrase>::new(
                    "pelican harbor voltage minnow".to_owned()
                )));
                assert!(new_passphrase.matches(&Confidential::<Passphrase>::new(
                    "anchovy glider ruckus tadpole".to_owned()
                )));
            }
            _ => panic!("the form did not send one change"),
        }

        for cause in [
            AuthFailure::BadPassphrase,
            AuthFailure::WeakPassphrase(ambre_core::PassphraseWeakness::TooShort),
            AuthFailure::StaleReseal,
        ] {
            keys.refuse(cause);
            tick().await;
            assert_eq!(
                find(&container, ".ambre-keys-refusal")
                    .unwrap()
                    .text_content()
                    .unwrap(),
                cause.to_string()
            );
        }
        assert!(AuthFailure::StaleReseal.to_string().contains("Try again"));

        // No current passphrase, or two new ones that differ, send nothing.
        fill(
            "",
            "anchovy glider ruckus tadpole",
            "anchovy glider ruckus tadpole",
        );
        submit(&container);
        tick().await;
        fill(
            "pelican harbor voltage minnow",
            "anchovy glider",
            "anchovy glide",
        );
        submit(&container);
        tick().await;
        assert!(
            rx.try_iter().next().is_none(),
            "a refused form sent a change"
        );
        assert_eq!(
            find(&container, ".ambre-keys-refusal")
                .unwrap()
                .text_content()
                .unwrap(),
            "The two passphrases do not match."
        );

        // The answer closes the form.
        keys.close();
        tick().await;
        assert!(find(&container, ".ambre-keys-change").is_none());
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
