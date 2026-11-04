// ambre-app/src/tofu.rs
// The recipient's key, compared and confirmed before a share
// (`ambre-spec-key-verification.md`). The classification, the confirmation
// and the prompt live here. This file alone builds the second phase of an
// invitation and the request of a direct message, so every route to either
// passes a decision.

use ambre_bus::{
    Authored, DirectMember, DirectMessageDraft, FixedBytes, Instant, LogicCommand, RestCommand,
    Role,
};
use ambre_chrome::Dialog;
use ambre_core::id::{Channel, Document, Id, User};
use leptos::prelude::*;

/// What a confirmed key is for.
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum Purpose {
    /// An invitation to a document, sent to the recipient's email.
    Invite {
        document_id: Id<Document>,
        email: ambre_bus::Email,
    },
    /// The direct message of this user and the recipient alone (the Murmure
    /// members spec). `channel_id` names the request.
    Direct { channel_id: Id<Channel> },
    /// An invitation of a space member into an open channel, by id (the
    /// Murmure channels spec). The invitee joins as an editor.
    Channel { channel_id: Id<Channel> },
}

/// A pending trust decision. Built by the per-frame relay from a fetched
/// key, once that key has been compared with the pinned one.
///
/// Only the states that interrupt the user become a prompt. That is a **new**
/// key ([`KeyContact::New`], the first contact) or a **changed** key
/// ([`KeyContact::Changed`], a divergence from the pin). A **trusted** key
/// never does: the relay proceeds straight to what the key is for.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct TofuPrompt {
    /// What the key is for.
    pub(crate) purpose: Purpose,
    /// The recipient, under whose identity the pin is filed.
    pub(crate) user_id: Id<User>,
    /// The recipient's display name, shown in the prompt. `None` when the
    /// relay named no one, and the prompt reads "This member".
    pub(crate) display_name: Option<ambre_core::Validated<ambre_core::DisplayName>>,
    /// Raw X25519 key the server just returned: pinned on confirmation and
    /// sent beside the request, so Sync wraps only if it still matches.
    pub(crate) exchange_pub_key: FixedBytes<ambre_core::ExchangePublicKey>,
    /// Preformatted fingerprint (`ambre_crypto::fingerprint`); displayed verbatim.
    pub(crate) fingerprint: ambre_bus::KeyFingerprint,
    /// Whether the key is a first contact or diverges from the pin.
    pub(crate) contact: KeyContact,
}

/// How a fetched key that interrupts the user relates to the pin.
///
/// The two states select different words and a warning, so they are an
/// `enum`. A swap of the two would hide a substituted key behind the prompt
/// of a first contact.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum KeyContact {
    /// No key is pinned for the recipient: the first contact.
    New,
    /// A key is pinned, and the fetched one differs from it.
    Changed,
}

impl TofuPrompt {
    /// The email the prompt shows under the name, for an invitation alone.
    fn email(&self) -> Option<&ambre_bus::Email> {
        match &self.purpose {
            Purpose::Invite { email, .. } => Some(email),
            Purpose::Direct { .. } | Purpose::Channel { .. } => None,
        }
    }
}

/// What a fetched key leads to.
#[derive(Debug)]
pub(crate) enum Next {
    /// The pinned key came back: the request goes out with no prompt.
    Proceed(LogicCommand),
    /// A new or a changed key: the user decides before anything is sent.
    Ask(TofuPrompt),
}

/// Classify a fetched key against the pin (`CHROME-050`, `CHROME-051`).
/// `role` is the one an invitation grants; a direct message ignores it.
pub(crate) fn classify(
    prompt: TofuPrompt,
    pinned: Option<FixedBytes<ambre_core::ExchangePublicKey>>,
    role: Role,
) -> Next {
    match pinned {
        Some(key) if key == prompt.exchange_pub_key => Next::Proceed(request(&prompt, role)),
        Some(_) => Next::Ask(TofuPrompt {
            contact: KeyContact::Changed,
            ..prompt
        }),
        None => Next::Ask(TofuPrompt {
            contact: KeyContact::New,
            ..prompt
        }),
    }
}

/// Pin the confirmed key, then dispatch the request that carries it
/// (`CHROME-052`). The pin is filed under the recipient's identity alone.
pub(crate) fn confirm(
    prompt: &TofuPrompt,
    role: Role,
    pin: impl FnOnce(Id<User>, &FixedBytes<ambre_core::ExchangePublicKey>),
    dispatch: impl FnOnce(LogicCommand),
) {
    pin(prompt.user_id, &prompt.exchange_pub_key);
    dispatch(request(prompt, role));
}

/// The request the key is for, carrying the key the user saw so Sync can
/// refuse a substitution (`CHROME-047`). Private: [`classify`] and
/// [`confirm`] are the only routes to it.
///
/// It names the user the key came with, and the instant it leaves, which the
/// Logic thread cannot read for itself.
fn request(prompt: &TofuPrompt, role: Role) -> LogicCommand {
    let now = Instant::<Authored>::new(js_sys::Date::now() as u64);
    match &prompt.purpose {
        Purpose::Invite { document_id, email } => LogicCommand::Rest {
            id: ambre_bus::next_request(),
            cmd: RestCommand::InviteToDocument {
                document_id: *document_id,
                invitee_email: Some(email.clone()),
                invitee: prompt.user_id,
                role,
                expect_exchange_pub_key: prompt.exchange_pub_key,
                now,
            },
        },
        Purpose::Direct { channel_id } => LogicCommand::OpenDirectMessage {
            draft: DirectMessageDraft {
                channel_id: *channel_id,
                members: vec![DirectMember {
                    user_id: prompt.user_id,
                    exchange_pub_key: prompt.exchange_pub_key,
                }],
            },
            now,
        },
        Purpose::Channel { channel_id } => LogicCommand::InviteToChannel {
            id: ambre_bus::next_request(),
            channel_id: *channel_id,
            invitee: DirectMember {
                user_id: prompt.user_id,
                exchange_pub_key: prompt.exchange_pub_key,
            },
            now,
        },
    }
}

/// The key verification modal. A modal, never a toast: a possible
/// substitution must not dismiss itself. Escape, the backdrop and the refusing
/// action clear `prompt` and leave `confirmed` alone. The confirming action
/// hands the prompt to `confirmed`, for the relay to pin and dispatch.
#[component]
pub(crate) fn TofuDialog(
    /// The decision waiting for the user, or `None`.
    prompt: RwSignal<Option<TofuPrompt>>,
    /// Set to the prompt the user confirmed. The relay takes it from here.
    confirmed: RwSignal<Option<TofuPrompt>>,
) -> impl IntoView {
    // The modal mirrors the prompt; a native close clears it.
    let open = RwSignal::new(ambre_chrome::Openness::Closed);
    Effect::new(move |_| open.set(ambre_chrome::Openness::from_bool(prompt.get().is_some())));
    let on_close = Callback::new(move |()| prompt.set(None));

    view! {
        <Dialog open=open on_close=on_close class="ambre-tofu">
            {move || {
                prompt
                    .get()
                    .map(|p| {
                        let contact = p.contact;
                        let direct = matches!(p.purpose, Purpose::Direct { .. });
                        // The address leaves its type only at the text node.
                        let email = p.email().map(ToString::to_string);
                        let fp_copy = p.fingerprint.clone();
                        let confirm_p = p.clone();
                        view! {
                            <div class="ambre-tofu-panel">
                                <h2>
                                    {match contact {
                                        KeyContact::Changed => "Recipient key changed",
                                        KeyContact::New => "Verify recipient key",
                                    }}
                                </h2>
                                <p class="ambre-tofu-who">
                                    <span class="ambre-tofu-name">{p.display_name.as_ref().map_or("This member", ambre_core::Validated::as_str).to_owned()}</span>
                                    {email
                                        .map(|email| {
                                            view! { <span class="ambre-tofu-email">{email}</span> }
                                        })}
                                </p>
                                {(contact == KeyContact::Changed)
                                    .then(|| {
                                        view! {
                                            <p class="ambre-tofu-warning" role="alert">
                                                "This user's key differs from the one you trusted before. \
                                                 A legitimate cause exists (they reinstalled and lost their \
                                                 keys), but a compromised server can also substitute keys. \
                                                 Re-verify out of band before proceeding."
                                            </p>
                                        }
                                    })}
                                <p class="ambre-tofu-fingerprint">{p.fingerprint.as_str().to_owned()}</p>
                                <button
                                    r#type="button"
                                    class="ambre-tofu-copy"
                                    on:click=move |_| {
                                        if let Some(win) = web_sys::window() {
                                            let _ = win.navigator().clipboard().write_text(fp_copy.as_str());
                                        }
                                    }
                                >
                                    "Copy fingerprint"
                                </button>
                                <p class="ambre-tofu-limit">
                                    "Without out-of-band verification (in person, phone, SMS), this \
                                     protection reduces to trusting the server. Compare this fingerprint \
                                     with the recipient over a channel the server does not control."
                                </p>
                                <div class="ambre-dialog-actions">
                                    // Refusal comes first: it pins nothing
                                    // and sends nothing.
                                    <button
                                        r#type="button"
                                        class="ambre-tofu-cancel"
                                        on:click=move |_| prompt.set(None)
                                    >
                                        "Cancel"
                                    </button>
                                    <button
                                        r#type="button"
                                        class="ambre-tofu-confirm"
                                        on:click=move |_| {
                                            confirmed.set(Some(confirm_p.clone()));
                                            prompt.set(None);
                                        }
                                    >
                                        {match (contact, direct) {
                                            (KeyContact::Changed, false) => "Re-pin and invite",
                                            (KeyContact::New, false) => "Confirm and invite",
                                            (KeyContact::Changed, true) => "Re-pin and message",
                                            (KeyContact::New, true) => "Confirm and message",
                                        }}
                                    </button>
                                </div>
                            </div>
                        }
                    })
            }}
        </Dialog>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use leptos::mount::mount_to;
    use leptos::wasm_bindgen::JsCast;
    use std::cell::RefCell;
    use wasm_bindgen_test::wasm_bindgen_test;

    /// A prompt for a key of `byte`, as the first phase answers it.
    fn fetched(byte: u8) -> TofuPrompt {
        TofuPrompt {
            purpose: Purpose::Invite {
                document_id: Id::<Document>::mint(),
                email: ambre_core::Email::from("bob@test.local"),
            },
            user_id: Id::<User>::mint(),
            display_name: ambre_core::Validated::<ambre_core::DisplayName>::parse("Bob").ok(),
            exchange_pub_key: FixedBytes::from_array([byte; 32]),
            fingerprint: ambre_bus::KeyFingerprint::new("ABCD EFGH".to_owned()),
            contact: KeyContact::New,
        }
    }

    /// The same key, fetched for a direct message.
    fn fetched_for_direct(byte: u8) -> TofuPrompt {
        TofuPrompt {
            purpose: Purpose::Direct {
                channel_id: Id::<Channel>::mint(),
            },
            ..fetched(byte)
        }
    }

    /// The key the invitation carries, or `None` for a prompt.
    fn invited_with(next: &Next) -> Option<[u8; 32]> {
        match next {
            Next::Proceed(LogicCommand::Rest {
                cmd:
                    RestCommand::InviteToDocument {
                        expect_exchange_pub_key,
                        ..
                    },
                ..
            }) => Some(*(*expect_exchange_pub_key).as_array()),
            _ => None,
        }
    }

    /// A key never seen interrupts, a substituted one alerts, and only the
    /// pinned one goes out unasked (`CHROME-050`, `CHROME-051`).
    #[wasm_bindgen_test]
    fn a_new_or_changed_key_asks_and_only_the_pinned_key_invites() {
        for prompt in [fetched(1), fetched_for_direct(1)] {
            match classify(prompt.clone(), None, Role::Editor) {
                Next::Ask(p) => assert_eq!(p.contact, KeyContact::New, "a first contact is new"),
                other => panic!("a new key went out: {other:?}"),
            }
            match classify(prompt, Some(FixedBytes::from_array([9; 32])), Role::Editor) {
                Next::Ask(p) => assert_eq!(
                    p.contact,
                    KeyContact::Changed,
                    "a substituted key is a change"
                ),
                other => panic!("a substituted key went out: {other:?}"),
            }
        }
        let trusted = classify(
            fetched(1),
            Some(FixedBytes::from_array([1; 32])),
            Role::Editor,
        );
        assert_eq!(invited_with(&trusted), Some([1; 32]));
    }

    /// A direct message goes out under the channel id the prompt names. It
    /// carries the recipient alone and the key the user saw (the Murmure
    /// members spec). The pin lands first, as for an invitation.
    #[wasm_bindgen_test]
    fn a_trusted_member_key_opens_the_direct_message_of_the_two() {
        let prompt = fetched_for_direct(6);
        let Purpose::Direct { channel_id } = prompt.purpose else {
            unreachable!()
        };
        let expected = vec![DirectMember {
            user_id: prompt.user_id,
            exchange_pub_key: FixedBytes::from_array([6; 32]),
        }];
        let Next::Proceed(LogicCommand::OpenDirectMessage { draft, .. }) = classify(
            prompt.clone(),
            Some(FixedBytes::from_array([6; 32])),
            Role::Viewer,
        ) else {
            panic!("a pinned key asks again");
        };
        assert_eq!(draft.channel_id, channel_id);
        assert_eq!(draft.members, expected);

        let log = RefCell::new(Vec::new());
        confirm(
            &prompt,
            Role::Viewer,
            |_, key| log.borrow_mut().push(format!("pin {}", key.as_array()[0])),
            |cmd| {
                let LogicCommand::OpenDirectMessage { draft, .. } = cmd else {
                    panic!("not a direct message: {cmd:?}");
                };
                assert_eq!(draft.members, expected);
                log.borrow_mut().push("dispatch".to_owned());
            },
        );
        assert_eq!(log.into_inner(), ["pin 6", "dispatch"]);
    }

    /// An invitation into a channel names the channel and the member, with the
    /// key the user saw. It asks first, as every share does.
    #[wasm_bindgen_test]
    fn a_trusted_key_invites_the_member_into_the_channel() {
        let channel_id = Id::<Channel>::mint();
        let prompt = TofuPrompt {
            purpose: Purpose::Channel { channel_id },
            ..fetched(7)
        };
        assert!(prompt.email().is_none(), "an invite by id shows no email");
        assert!(matches!(
            classify(prompt.clone(), None, Role::Editor),
            Next::Ask(_)
        ));
        let Next::Proceed(LogicCommand::InviteToChannel {
            channel_id: sent,
            invitee,
            ..
        }) = classify(
            prompt.clone(),
            Some(FixedBytes::from_array([7; 32])),
            Role::Editor,
        )
        else {
            panic!("a pinned key did not invite into the channel");
        };
        assert_eq!(sent, channel_id);
        assert_eq!(
            invitee,
            DirectMember {
                user_id: prompt.user_id,
                exchange_pub_key: FixedBytes::from_array([7; 32]),
            }
        );
    }

    /// The pin lands before the invitation leaves, under the recipient's
    /// identity, and the invitation carries the pinned key (`CHROME-052`).
    #[wasm_bindgen_test]
    fn a_confirmation_pins_before_it_dispatches() {
        let prompt = fetched(4);
        let log = RefCell::new(Vec::new());
        confirm(
            &prompt,
            Role::Viewer,
            |user, key| {
                log.borrow_mut()
                    .push(format!("pin {:?} {}", user.as_bytes(), key.as_array()[0]))
            },
            |cmd| {
                let LogicCommand::Rest {
                    cmd:
                        RestCommand::InviteToDocument {
                            role,
                            expect_exchange_pub_key,
                            ..
                        },
                    ..
                } = cmd
                else {
                    panic!("not an invitation: {cmd:?}");
                };
                assert_eq!(role, Role::Viewer);
                log.borrow_mut().push(format!(
                    "dispatch {}",
                    expect_exchange_pub_key.as_array()[0]
                ));
            },
        );
        let pin = format!("pin {:?} 4", prompt.user_id.as_bytes());
        assert_eq!(log.into_inner(), vec![pin, "dispatch 4".to_owned()]);
    }

    /// A mounted prompt, its two signals, and the element to remove after.
    struct Mounted {
        root: web_sys::Element,
        dialog: web_sys::HtmlDialogElement,
        prompt: RwSignal<Option<TofuPrompt>>,
        confirmed: RwSignal<Option<TofuPrompt>>,
    }

    /// Mount the modal with `prompt` pending, and wait for it to open.
    async fn mount(prompt: TofuPrompt) -> Mounted {
        let root = document().create_element("div").unwrap();
        document().body().unwrap().append_child(&root).unwrap();
        let target: web_sys::HtmlElement = root.clone().dyn_into().unwrap();
        let pending = RwSignal::new(Some(prompt));
        let confirmed = RwSignal::new(None);
        let handle = mount_to(target, move || {
            view! { <TofuDialog prompt=pending confirmed=confirmed /> }
        });
        handle.forget();
        leptos::task::tick().await;
        let dialog: web_sys::HtmlDialogElement = root
            .query_selector("dialog")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        assert!(dialog.open(), "the prompt opens the modal");
        Mounted {
            root,
            dialog,
            prompt: pending,
            confirmed,
        }
    }

    /// The element of `selector` inside the mounted modal.
    fn find(m: &Mounted, selector: &str) -> web_sys::HtmlElement {
        m.root
            .query_selector(selector)
            .unwrap()
            .unwrap_or_else(|| panic!("no {selector}"))
            .dyn_into()
            .unwrap()
    }

    /// A future that resolves once the dialog's native `close` event fires.
    fn closed(dialog: &web_sys::HtmlDialogElement) -> wasm_bindgen_futures::JsFuture {
        let dialog = dialog.clone();
        let promise = js_sys::Promise::new(&mut |resolve, _| {
            dialog
                .add_event_listener_with_callback("close", &resolve)
                .unwrap();
        });
        wasm_bindgen_futures::JsFuture::from(promise)
    }

    /// The prompt states the limit of the protection, and its refusing action
    /// comes before the confirming one, new key or changed (`CHROME-054`,
    /// `CHROME-077`).
    #[wasm_bindgen_test]
    async fn the_prompt_states_its_limit_and_offers_refusal_first() {
        for contact in [KeyContact::New, KeyContact::Changed] {
            let m = mount(TofuPrompt {
                contact,
                ..fetched(2)
            })
            .await;
            let limit = find(&m, ".ambre-tofu-limit").text_content().unwrap();
            assert!(limit.contains("reduces to trusting the server"), "{limit}");
            assert!(limit.contains("out-of-band"), "{limit}");
            let actions = m
                .root
                .query_selector_all(".ambre-dialog-actions button")
                .unwrap();
            let first: web_sys::Element = actions.item(0).unwrap().dyn_into().unwrap();
            let last: web_sys::Element = actions.item(1).unwrap().dyn_into().unwrap();
            assert_eq!(
                first.class_name(),
                "ambre-tofu-cancel",
                "contact: {contact:?}"
            );
            assert_eq!(
                last.class_name(),
                "ambre-tofu-confirm",
                "contact: {contact:?}"
            );
            m.root.remove();
        }
    }

    /// A prompt for a direct message names the member without an email, and
    /// its confirming action opens a conversation rather than an invitation.
    #[wasm_bindgen_test]
    async fn a_prompt_for_a_direct_message_names_the_conversation() {
        let m = mount(fetched_for_direct(7)).await;
        assert!(
            m.root
                .query_selector(".ambre-tofu-email")
                .unwrap()
                .is_none()
        );
        let confirm = find(&m, ".ambre-tofu-confirm").text_content().unwrap();
        assert_eq!(confirm, "Confirm and message");
        m.root.remove();
    }

    /// Escape, the backdrop and the refusing action leave no decision behind
    /// and confirm nothing. So nothing is pinned and nothing is sent
    /// (`CHROME-055`, `CHROME-075`, `CHROME-076`).
    #[wasm_bindgen_test]
    async fn every_refusal_route_leaves_nothing_to_pin_or_send() {
        for route in ["escape", "backdrop", "cancel"] {
            let m = mount(fetched(3)).await;
            let shut = closed(&m.dialog);
            match route {
                // A headless browser drops a synthetic Escape; this is the
                // close the browser performs for it.
                "escape" => m.dialog.close(),
                "backdrop" => {
                    let click = web_sys::MouseEvent::new("click").unwrap();
                    m.dialog.dispatch_event(&click).unwrap();
                }
                _ => find(&m, ".ambre-tofu-cancel").click(),
            }
            shut.await.unwrap();
            leptos::task::tick().await;
            assert_eq!(m.prompt.get_untracked(), None, "{route} leaves a decision");
            assert_eq!(m.confirmed.get_untracked(), None, "{route} confirms");
            m.root.remove();
        }
    }

    /// The confirming action hands the prompt on and closes the modal.
    #[wasm_bindgen_test]
    async fn the_confirming_action_hands_the_prompt_to_the_relay() {
        let prompt = fetched(5);
        let m = mount(prompt.clone()).await;
        let shut = closed(&m.dialog);
        find(&m, ".ambre-tofu-confirm").click();
        shut.await.unwrap();
        assert_eq!(m.confirmed.get_untracked(), Some(prompt));
        assert_eq!(m.prompt.get_untracked(), None);
        m.root.remove();
    }
}
