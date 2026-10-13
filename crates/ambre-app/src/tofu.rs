// ambre-app/src/tofu.rs
// The recipient's key, compared and confirmed before a share
// (`ambre-spec-key-verification.md`). The classification, the confirmation
// and the prompt live here. This file alone builds the second phase of an
// invitation, so every route to it passes a decision.

use ambre_bus::{DocumentId, RestCommand, Role, UserId};
use ambre_chrome::Dialog;
use leptos::prelude::*;

/// A pending trust decision for the share dialog. Built by the per-frame
/// relay from the first phase's answer, once the fetched key has been
/// compared with the pinned one.
///
/// Only the states that interrupt the user become a prompt. That is a **new**
/// key (`changed == false`, the first invitation) or a **changed** key
/// (`changed == true`, a divergence from the pin). A **trusted** key never
/// does: the relay proceeds straight to the second phase.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct TofuPrompt {
    /// The document being shared.
    pub(crate) document_id: DocumentId,
    /// The recipient, under whose identity the pin is filed.
    pub(crate) user_id: UserId,
    /// The recipient's display name, shown in the prompt.
    pub(crate) display_name: String,
    /// The recipient's email, shown in the prompt and sent with the invitation.
    pub(crate) email: String,
    /// Raw X25519 key the server just returned: pinned on confirmation and sent
    /// as `expect_exchange_pub_key` so Sync wraps only if it still matches.
    pub(crate) exchange_pub_key: [u8; 32],
    /// Preformatted fingerprint (`ambre_crypto::fingerprint`); displayed verbatim.
    pub(crate) fingerprint: String,
    /// `false` = new (first invitation), `true` = changed (diverged from the pin).
    pub(crate) changed: bool,
}

/// What a fetched key leads to.
#[derive(Debug)]
pub(crate) enum Next {
    /// The pinned key came back: the invitation goes out with no prompt.
    Invite(RestCommand),
    /// A new or a changed key: the user decides before anything is sent.
    Ask(TofuPrompt),
}

/// Classify a fetched key against the pin (`CHROME-050`, `CHROME-051`).
pub(crate) fn classify(prompt: TofuPrompt, pinned: Option<[u8; 32]>, role: Role) -> Next {
    match pinned {
        Some(key) if key == prompt.exchange_pub_key => Next::Invite(invitation(&prompt, role)),
        Some(_) => Next::Ask(TofuPrompt {
            changed: true,
            ..prompt
        }),
        None => Next::Ask(TofuPrompt {
            changed: false,
            ..prompt
        }),
    }
}

/// Pin the confirmed key, then dispatch the invitation that carries it
/// (`CHROME-052`). The pin is filed under the recipient's identity alone.
pub(crate) fn confirm(
    prompt: &TofuPrompt,
    role: Role,
    pin: impl FnOnce([u8; 16], [u8; 32]),
    dispatch: impl FnOnce(RestCommand),
) {
    pin(*prompt.user_id.as_bytes(), prompt.exchange_pub_key);
    dispatch(invitation(prompt, role));
}

/// The second phase, carrying the key the user saw so Sync can refuse a
/// substitution (`CHROME-047`). Private: [`classify`] and [`confirm`] are the
/// only routes to it.
fn invitation(prompt: &TofuPrompt, role: Role) -> RestCommand {
    RestCommand::InviteToDocument {
        document_id: prompt.document_id,
        invitee_email: prompt.email.clone(),
        role,
        expect_exchange_pub_key: prompt.exchange_pub_key,
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
    let open = RwSignal::new(false);
    Effect::new(move |_| open.set(prompt.get().is_some()));
    let on_close = Callback::new(move |()| prompt.set(None));

    view! {
        <Dialog open=open on_close=on_close class="ambre-tofu">
            {move || {
                prompt
                    .get()
                    .map(|p| {
                        let changed = p.changed;
                        let fp_copy = p.fingerprint.clone();
                        let confirm_p = p.clone();
                        view! {
                            <div class="ambre-tofu-panel">
                                <h2>
                                    {if changed {
                                        "Recipient key changed"
                                    } else {
                                        "Verify recipient key"
                                    }}
                                </h2>
                                <p class="ambre-tofu-who">
                                    <span class="ambre-tofu-name">{p.display_name.clone()}</span>
                                    <span class="ambre-tofu-email">{p.email.clone()}</span>
                                </p>
                                {changed
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
                                <p class="ambre-tofu-fingerprint">{p.fingerprint.clone()}</p>
                                <button
                                    r#type="button"
                                    class="ambre-tofu-copy"
                                    on:click=move |_| {
                                        if let Some(win) = web_sys::window() {
                                            let _ = win.navigator().clipboard().write_text(&fp_copy);
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
                                        {if changed { "Re-pin and invite" } else { "Confirm and invite" }}
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
            document_id: DocumentId::new(),
            user_id: UserId::new(),
            display_name: "Bob".to_owned(),
            email: "bob@test.local".to_owned(),
            exchange_pub_key: [byte; 32],
            fingerprint: "ABCD EFGH".to_owned(),
            changed: false,
        }
    }

    /// The key the invitation carries, or `None` for a prompt.
    fn invited_with(next: &Next) -> Option<[u8; 32]> {
        match next {
            Next::Invite(RestCommand::InviteToDocument {
                expect_exchange_pub_key,
                ..
            }) => Some(*expect_exchange_pub_key),
            _ => None,
        }
    }

    /// A key never seen interrupts, a substituted one alerts, and only the
    /// pinned one goes out unasked (`CHROME-050`, `CHROME-051`).
    #[wasm_bindgen_test]
    fn a_new_or_changed_key_asks_and_only_the_pinned_key_invites() {
        match classify(fetched(1), None, Role::Editor) {
            Next::Ask(p) => assert!(!p.changed, "a first contact is new"),
            other => panic!("a new key invited: {other:?}"),
        }
        match classify(fetched(1), Some([9; 32]), Role::Editor) {
            Next::Ask(p) => assert!(p.changed, "a substituted key is a change"),
            other => panic!("a substituted key invited: {other:?}"),
        }
        let trusted = classify(fetched(1), Some([1; 32]), Role::Editor);
        assert_eq!(invited_with(&trusted), Some([1; 32]));
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
            |user, key| log.borrow_mut().push(format!("pin {:?} {}", user, key[0])),
            |cmd| {
                let RestCommand::InviteToDocument {
                    role,
                    expect_exchange_pub_key,
                    ..
                } = cmd
                else {
                    panic!("not an invitation: {cmd:?}");
                };
                assert_eq!(role, Role::Viewer);
                log.borrow_mut()
                    .push(format!("dispatch {}", expect_exchange_pub_key[0]));
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
        for changed in [false, true] {
            let m = mount(TofuPrompt {
                changed,
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
                "changed: {changed}"
            );
            assert_eq!(
                last.class_name(),
                "ambre-tofu-confirm",
                "changed: {changed}"
            );
            m.root.remove();
        }
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
