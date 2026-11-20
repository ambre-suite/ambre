// ambre-app/src/keys_gate.rs
// ============================================================================
// The keys gate of the home screen (`ambre-spec-launcher.md` section 1.6).
//
// A provider sign-in vouches for who the person is, and opens no key. The home
// screen then shows one of two faces before the list. An account that holds
// its keys sees a passphrase field that opens them (`CHROME-097`). An account
// that owes them sees the setup that creates them (`CHROME-098`). A later
// refusal for a setup still owed brings the setup back over the list. The
// unlock face links to the recovery form, the one way in after a forgotten
// passphrase (`CHROME-133`).
//
// The passphrase lives in the field alone. It is read once, at submission,
// into a `Confidential`, and the field is emptied in the same step. No signal,
// store or log line of this thread holds it afterwards (`CHROME-100`). The words of a
// refusal, the worker's or the field's own, sit in one alert line, which a
// screen reader announces when it changes (`CHROME-131`).
//
// Each face names the account the provider vouched for, so the setup asks for
// no name or address the provider gave (`CHROME-140`). Each face offers a
// sign-out, for a person who signed in with the wrong account (`CHROME-142`).
// ============================================================================

use ambre_bus::KeySetup;

/// The face a provider account shows before its list.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum KeysFace {
    /// The account holds its keys: a passphrase opens them (`CHROME-097`).
    Unlock,
    /// The account owes its keys: the setup creates them (`CHROME-098`).
    SetUp,
}

impl KeysFace {
    /// The face a provider sign-in calls for, by whether the relay stores the
    /// account's keys.
    pub(crate) fn after_sign_in(keys: KeySetup) -> Self {
        match keys {
            KeySetup::Stored => Self::Unlock,
            KeySetup::Absent => Self::SetUp,
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) use view::{GateRefusal, KeysGate, keys_gate_view};

#[cfg(target_arch = "wasm32")]
mod view {
    use super::KeysFace;
    use ambre_bus::{AuthFailure, RestCommand};
    use ambre_chrome::PassphraseIntent;
    use ambre_core::{Confidential, DisplayName, Email, Passphrase, Validated};
    use ambre_logic::app::LogicCommand;
    use leptos::html;
    use leptos::prelude::*;

    /// Why the gate keeps the person on its face.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub(crate) enum GateRefusal {
        /// The field held nothing to send, or a confirmation that differs.
        Typed(ambre_chrome::PassphraseRefusal),
        /// The Sync worker refused the unlock or the setup: a wrong passphrase,
        /// or a weak one with its reason (`ZK-036`).
        Worker(AuthFailure),
    }

    /// Who the provider signed in: the name and the address it gave
    /// (`CHROME-140`).
    #[derive(Clone, PartialEq, Eq, Debug)]
    pub(crate) struct GateIdentity {
        pub(crate) display_name: Validated<DisplayName>,
        pub(crate) email: Email,
    }

    /// The state of the gate: the face it shows, the account it names and the
    /// last refusal. It holds no passphrase, by construction (`CHROME-100`).
    #[derive(Clone, Copy)]
    pub(crate) struct KeysGate {
        /// `None` while no provider account waits on its keys.
        pub(crate) face: RwSignal<Option<KeysFace>>,
        /// The account the provider signed in, once it answered.
        pub(crate) identity: RwSignal<Option<GateIdentity>>,
        /// The words the alert line shows, or `None` for a quiet line.
        pub(crate) refusal: RwSignal<Option<GateRefusal>>,
    }

    impl KeysGate {
        /// A gate that shows no face.
        pub(crate) fn new() -> Self {
            Self {
                face: RwSignal::new(None),
                identity: RwSignal::new(None),
                refusal: RwSignal::new(None),
            }
        }

        /// Name on each face the account the provider signed in.
        pub(crate) fn identify(self, display_name: Validated<DisplayName>, email: Email) {
            self.identity.set(Some(GateIdentity {
                display_name,
                email,
            }));
        }

        /// Close the gate: the session is ready, and the list shows.
        pub(crate) fn open(self) {
            self.face.set(None);
            self.identity.set(None);
            self.refusal.set(None);
        }
    }

    /// Whether a submission waits on the worker's answer.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Submission {
        /// Nothing sent, or the last one answered with a refusal.
        Idle,
        /// Sent, and no refusal came back yet.
        Sent,
    }

    /// The id of the alert line, which each field names as its description.
    const REFUSAL_ID: ambre_core::StaticKey<ambre_core::ElementIds> =
        ambre_core::StaticKey::new("ambre-keys-refusal");

    /// Read a field once and empty it, so its text outlives the submission
    /// nowhere on this thread (`CHROME-100`).
    fn take_field(field: NodeRef<html::Input>) -> Confidential<Passphrase> {
        let Some(input) = field.get_untracked() else {
            return Confidential::<Passphrase>::new(String::new());
        };
        let typed = Confidential::<Passphrase>::new(input.value());
        input.set_value("");
        typed
    }

    /// The face the gate shows, or nothing while no account waits on its keys.
    /// `keys` opens the recovery form over the gate (`CHROME-133`).
    pub(crate) fn keys_gate_view(
        gate: KeysGate,
        keys: crate::account::KeysSignals,
        to_logic: crate::outbox::ShellOutbox,
    ) -> impl IntoView {
        move || {
            let face = gate.face.get()?;
            let first = NodeRef::<html::Input>::new();
            let second = NodeRef::<html::Input>::new();
            let submission = RwSignal::new(Submission::Idle);
            let sign_out = crate::landing::sign_out_control(to_logic.clone());
            let to_logic = to_logic.clone();
            // A `Callback` is `Copy`, so the Enter key of either field and the
            // button each hold it.
            let submit = Callback::new(move |()| {
                let typed = take_field(first);
                let confirmed = take_field(second);
                let intent = match face {
                    KeysFace::Unlock => PassphraseIntent::Open,
                    KeysFace::SetUp => PassphraseIntent::Seal,
                };
                if let Err(refusal) = ambre_chrome::vet_passphrase(&typed, &confirmed, intent) {
                    gate.refusal.set(Some(GateRefusal::Typed(refusal)));
                    return;
                }
                let cmd = match face {
                    KeysFace::Unlock => RestCommand::Unlock { passphrase: typed },
                    KeysFace::SetUp => RestCommand::SetUpKeys { passphrase: typed },
                };
                // A refused send leaves the face live, to try again (`BUS-018`).
                let sent = to_logic.send(LogicCommand::Rest {
                    id: ambre_bus::next_request(),
                    cmd,
                });
                if sent.accepted() {
                    gate.refusal.set(None);
                    submission.set(Submission::Sent);
                }
            });
            let on_key = move |ev: web_sys::KeyboardEvent| {
                if ev.key() == "Enter" {
                    submit.run(());
                }
            };
            let waiting =
                move || submission.get() == Submission::Sent && gate.refusal.get().is_none();
            let (heading, note, action) = match face {
                KeysFace::Unlock => (
                    "Unlock your account",
                    "Type your account passphrase. It opens your keys on this device.",
                    "Unlock",
                ),
                KeysFace::SetUp => (
                    "Choose your account passphrase",
                    "It seals your keys on this device. Four or more random words resist a \
                     guess. Nobody can reset it: the recovery words shown next are the only \
                     other way in.",
                    "Set up",
                ),
            };
            Some(view! {
                <div class="ambre-signin ambre-keys">
                    <span class="ambre-signin-wordmark">"Ambre"</span>
                    <h2>{heading}</h2>
                    // The provider gave the name and the address: the face
                    // shows them, and asks for neither (`CHROME-140`).
                    {move || gate.identity.get().map(|identity| view! {
                        <p class="ambre-keys-identity">
                            "Signed in as "
                            <span class="ambre-keys-name">{identity.display_name.into_string()}</span>
                            " ("
                            <span class="ambre-keys-email">{identity.email.as_str().to_owned()}</span>
                            ")"
                        </p>
                    })}
                    <p class="ambre-keys-note">{note}</p>
                    <input
                        class="ambre-signin-input ambre-keys-passphrase"
                        type="password"
                        autocomplete=if face == KeysFace::Unlock { "current-password" } else { "new-password" }
                        aria-label="Passphrase"
                        aria-describedby=REFUSAL_ID.as_str()
                        aria-invalid=move || if gate.refusal.get().is_some() { "true" } else { "false" }
                        placeholder="Passphrase"
                        node_ref=first
                        on:keydown=on_key
                    />
                    <Show when=move || face == KeysFace::SetUp>
                        <input
                            class="ambre-signin-input ambre-keys-confirm"
                            type="password"
                            autocomplete="new-password"
                            aria-label="Passphrase again"
                            aria-describedby=REFUSAL_ID.as_str()
                            placeholder="Passphrase again"
                            node_ref=second
                            on:keydown=on_key
                        />
                    </Show>
                    <p id=REFUSAL_ID.as_str() class="ambre-keys-refusal" role="alert">
                        {move || match gate.refusal.get() {
                            None => String::new(),
                            Some(GateRefusal::Typed(refusal)) => refusal.message().to_owned(),
                            Some(GateRefusal::Worker(cause)) => cause.to_string(),
                        }}
                    </p>
                    <button
                        class="ambre-signin-btn"
                        disabled=waiting
                        on:click=move |_| submit.run(())
                    >
                        {action}
                    </button>
                    // The words open the account under a new passphrase. The
                    // form covers the gate, which shows again once it closes.
                    <Show when=move || face == KeysFace::Unlock>
                        <button
                            class="ambre-signin-link ambre-keys-recover"
                            on:click=move |_| keys.open(crate::account::KeysFace::Recover)
                        >
                            "Forgot the passphrase? Recover the account with its recovery words"
                        </button>
                    </Show>
                    // The header and its sign-out show only once a session
                    // holds, so each face carries its own (`CHROME-142`).
                    <div class="ambre-keys-leave">{sign_out}</div>
                </div>
            })
        }
    }

    // -- Tests (wasm tier) ---------------------------------------------------
    // The gate is DOM: these mount it in headless Chrome and drive it through
    // `web-sys` (`just test-wasm`).
    #[cfg(test)]
    mod tests {
        use super::*;
        use leptos::mount::mount_to;
        use wasm_bindgen::JsCast;
        use wasm_bindgen_test::*;

        wasm_bindgen_test_configure!(run_in_browser);

        async fn tick() {
            let promise = js_sys::Promise::new(&mut |resolve, _| {
                let _ = web_sys::window()
                    .unwrap()
                    .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 0);
            });
            let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
        }

        /// The gate mounted in a fresh container, the gate's state, and what
        /// it sent to the Logic thread.
        fn mounted() -> (
            web_sys::Element,
            KeysGate,
            crate::account::KeysSignals,
            ambre_bus::conduit::Receiver<LogicCommand>,
        ) {
            let document = web_sys::window().unwrap().document().unwrap();
            let container = document.create_element("div").unwrap();
            document.body().unwrap().append_child(&container).unwrap();
            let (tx, rx) = ambre_bus::conduit::local::<LogicCommand>(ambre_core::Bounded::<
                ambre_bus::conduit::ChannelSlots,
            >::of::<8>());
            let outbox = crate::outbox::ShellOutbox::new(tx);
            let gate = KeysGate::new();
            let keys = crate::account::KeysSignals::new();
            let html: web_sys::HtmlElement = container.clone().unchecked_into();
            std::mem::forget(mount_to(html, move || {
                keys_gate_view(gate, keys, outbox.clone())
            }));
            (container, gate, keys, rx)
        }

        fn field(container: &web_sys::Element, class: &str) -> web_sys::HtmlInputElement {
            container
                .query_selector(&format!(".{class}"))
                .unwrap()
                .unwrap_or_else(|| panic!("no .{class}"))
                .unchecked_into()
        }

        fn click_action(container: &web_sys::Element) {
            let button: web_sys::HtmlElement = container
                .query_selector(".ambre-signin-btn")
                .unwrap()
                .unwrap()
                .unchecked_into();
            button.click();
        }

        fn alert(container: &web_sys::Element) -> web_sys::Element {
            container
                .query_selector("[role=alert]")
                .unwrap()
                .expect("the gate has an alert line")
        }

        /// The passphrase each command sent carries, by kind.
        fn sent(rx: &ambre_bus::conduit::Receiver<LogicCommand>) -> Vec<(&'static str, bool)> {
            rx.try_iter()
                .filter_map(|cmd| match cmd {
                    LogicCommand::Rest {
                        cmd: RestCommand::Unlock { passphrase },
                        ..
                    } => Some(("unlock", passphrase.matches(&typed("anchovy glider")))),
                    LogicCommand::Rest {
                        cmd: RestCommand::SetUpKeys { passphrase },
                        ..
                    } => Some(("setup", passphrase.matches(&typed("anchovy glider")))),
                    _ => None,
                })
                .collect()
        }

        fn typed(text: &str) -> Confidential<Passphrase> {
            Confidential::<Passphrase>::new(text.to_owned())
        }

        /// `CHROME-097`, `CHROME-100`: an account that holds its keys sees one
        /// passphrase field. A submission sends its text once to unlock, and
        /// the field is empty from then on.
        #[wasm_bindgen_test]
        async fn an_unlock_sends_the_passphrase_once_and_empties_the_field() {
            let (container, gate, _keys, rx) = mounted();
            assert!(
                container.query_selector("input").unwrap().is_none(),
                "no face before a sign-in"
            );
            gate.face.set(Some(KeysFace::Unlock));
            tick().await;
            assert!(
                container
                    .query_selector(".ambre-keys-confirm")
                    .unwrap()
                    .is_none(),
                "an unlock asks once"
            );
            let passphrase = field(&container, "ambre-keys-passphrase");
            assert_eq!(passphrase.type_(), "password");
            passphrase.set_value("anchovy glider");
            click_action(&container);
            tick().await;
            assert_eq!(sent(&rx), [("unlock", true)]);
            assert_eq!(passphrase.value(), "", "the field outlived the submission");

            // A second click while the worker has not answered sends nothing.
            click_action(&container);
            tick().await;
            assert!(sent(&rx).is_empty());
        }

        /// `CHROME-097`: a wrong passphrase reads its refusal in the alert
        /// line, and the face stays for another try.
        #[wasm_bindgen_test]
        async fn a_wrong_passphrase_is_said_and_the_face_stays() {
            let (container, gate, _keys, rx) = mounted();
            gate.face.set(Some(KeysFace::Unlock));
            tick().await;
            field(&container, "ambre-keys-passphrase").set_value("anchovy glider");
            click_action(&container);
            tick().await;
            gate.refusal
                .set(Some(GateRefusal::Worker(AuthFailure::BadPassphrase)));
            tick().await;
            assert_eq!(
                alert(&container).text_content().unwrap_or_default(),
                AuthFailure::BadPassphrase.to_string()
            );
            field(&container, "ambre-keys-passphrase").set_value("anchovy glider");
            click_action(&container);
            tick().await;
            assert_eq!(sent(&rx), [("unlock", true), ("unlock", true)]);
        }

        /// `CHROME-098`, `CHROME-100`: an account that owes its keys sees the
        /// setup. Two differing fields send nothing and say so; two that agree
        /// send the setup once, and both fields are empty from then on.
        #[wasm_bindgen_test]
        async fn a_setup_sends_only_a_confirmed_passphrase() {
            let (container, gate, _keys, rx) = mounted();
            gate.face.set(Some(KeysFace::SetUp));
            tick().await;
            let passphrase = field(&container, "ambre-keys-passphrase");
            let confirm = field(&container, "ambre-keys-confirm");
            passphrase.set_value("anchovy glider");
            confirm.set_value("anchovy glide");
            click_action(&container);
            tick().await;
            assert!(sent(&rx).is_empty(), "a mismatch was sent");
            assert_eq!(
                alert(&container).text_content().unwrap_or_default(),
                ambre_chrome::PassphraseRefusal::Mismatch.message()
            );
            assert_eq!(
                (passphrase.value(), confirm.value()),
                (String::new(), String::new())
            );

            passphrase.set_value("anchovy glider");
            confirm.set_value("anchovy glider");
            click_action(&container);
            tick().await;
            assert_eq!(sent(&rx), [("setup", true)]);
            assert_eq!(
                (passphrase.value(), confirm.value()),
                (String::new(), String::new())
            );
            assert_eq!(alert(&container).text_content().unwrap_or_default(), "");
        }

        /// `CHROME-131`: the words the worker gives a weak setup passphrase sit
        /// in the alert line, which each field names as its description. A
        /// screen reader announces the line when it changes, and reads it again
        /// from the field.
        #[wasm_bindgen_test]
        async fn a_weak_setup_passphrase_is_announced_with_its_reason() {
            let (container, gate, _keys, _rx) = mounted();
            gate.face.set(Some(KeysFace::SetUp));
            tick().await;
            let refusal =
                AuthFailure::WeakPassphrase(ambre_core::PassphraseWeakness::CommonPassword);
            gate.refusal.set(Some(GateRefusal::Worker(refusal)));
            tick().await;
            let line = alert(&container);
            assert_eq!(line.text_content().unwrap_or_default(), refusal.to_string());
            assert!(
                refusal.to_string().contains("common password"),
                "the words name the reason"
            );
            let id = line.id();
            assert!(!id.is_empty());
            for class in ["ambre-keys-passphrase", "ambre-keys-confirm"] {
                assert_eq!(
                    field(&container, class).get_attribute("aria-describedby"),
                    Some(id.clone()),
                    "{class}"
                );
            }
            assert_eq!(
                field(&container, "ambre-keys-passphrase").get_attribute("aria-invalid"),
                Some("true".to_owned())
            );
        }

        /// `CHROME-133`: the unlock face links to the recovery form, and the
        /// setup face does not. The link opens the form and sends nothing.
        #[wasm_bindgen_test]
        async fn the_unlock_face_links_to_the_recovery_form() {
            let (container, gate, keys, rx) = mounted();
            gate.face.set(Some(KeysFace::SetUp));
            tick().await;
            assert!(
                container
                    .query_selector(".ambre-keys-recover")
                    .unwrap()
                    .is_none(),
                "the setup offers the recovery"
            );
            gate.face.set(Some(KeysFace::Unlock));
            tick().await;
            let link: web_sys::HtmlElement = container
                .query_selector(".ambre-keys-recover")
                .unwrap()
                .expect("the unlock face links to the recovery")
                .unchecked_into();
            link.click();
            tick().await;
            assert!(
                keys.face
                    .with_untracked(|face| matches!(face, crate::account::KeysFace::Recover)),
                "the link opened no recovery form"
            );
            assert!(sent(&rx).is_empty());
        }

        /// `CHROME-140`: each face names the account the provider signed in,
        /// by the name and the address it gave. The setup holds no field for
        /// either, and its command carries the passphrase alone.
        #[wasm_bindgen_test]
        async fn each_face_names_the_provider_account_and_asks_for_no_identity() {
            let (container, gate, _keys, rx) = mounted();
            gate.identify(
                Validated::<DisplayName>::parse("Ada").unwrap(),
                Email::from("ada@test.local"),
            );
            for face in [KeysFace::Unlock, KeysFace::SetUp] {
                gate.face.set(Some(face));
                tick().await;
                let named = container
                    .query_selector(".ambre-keys-identity")
                    .unwrap()
                    .unwrap_or_else(|| panic!("{face:?} names no account"))
                    .text_content()
                    .unwrap_or_default();
                assert!(named.contains("Ada"), "{face:?}: {named}");
                assert!(named.contains("ada@test.local"), "{face:?}: {named}");
                assert!(
                    container
                        .query_selector("input[type=email]")
                        .unwrap()
                        .is_none()
                        && container
                            .query_selector("input:not([type=password])")
                            .unwrap()
                            .is_none(),
                    "{face:?} asks for the identity again"
                );
            }
            field(&container, "ambre-keys-passphrase").set_value("anchovy glider");
            field(&container, "ambre-keys-confirm").set_value("anchovy glider");
            click_action(&container);
            tick().await;
            assert_eq!(sent(&rx), [("setup", true)]);

            // The session opens: the gate forgets the account it named.
            gate.open();
            assert_eq!(gate.identity.get_untracked(), None);
        }

        /// `CHROME-142`: the unlock and the setup faces each offer a sign-out.
        /// A click sends one sign-out to the live threads, and no passphrase.
        #[wasm_bindgen_test]
        async fn each_face_offers_a_sign_out() {
            for face in [KeysFace::Unlock, KeysFace::SetUp] {
                let (container, gate, _keys, rx) = mounted();
                gate.face.set(Some(face));
                tick().await;
                let button: web_sys::HtmlElement = container
                    .query_selector(".ambre-keys .ambre-sign-out")
                    .unwrap()
                    .unwrap_or_else(|| panic!("{face:?} offers no sign-out"))
                    .unchecked_into();
                button.click();
                button.click();
                tick().await;
                let sent: Vec<LogicCommand> = rx.try_iter().collect();
                assert!(
                    matches!(
                        sent.as_slice(),
                        [LogicCommand::Rest {
                            cmd: RestCommand::Logout,
                            ..
                        }]
                    ),
                    "{face:?}: not one sign-out alone"
                );
                container.remove();
            }
        }

        /// The gate opened by a ready session shows no face and no refusal.
        #[wasm_bindgen_test]
        async fn an_open_gate_shows_nothing() {
            let (container, gate, _keys, _rx) = mounted();
            gate.face.set(Some(KeysFace::Unlock));
            gate.refusal
                .set(Some(GateRefusal::Worker(AuthFailure::BadPassphrase)));
            tick().await;
            gate.open();
            tick().await;
            assert!(container.query_selector(".ambre-keys").unwrap().is_none());
        }
    }
}
