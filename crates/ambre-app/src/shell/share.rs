// ambre-app/src/shell/share.rs
// The member list of the share dialog. Each row carries the member's role and
// what the signed-in user may do there. They revoke another member, or leave
// the document from their own row (`SYNC-141`). Every action leaves as a
// `RestCommand` through `send`, so a test reads the command without a worker.

use ambre_bus::{MemberInfo, MemberStanding, RestCommand, Role};
use ambre_core::id::{Document, Id, User};
use leptos::prelude::*;

/// What the signed-in user may do on one row of the member list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowAction {
    /// Another member's row: revoke their access.
    Revoke,
    /// The user's own row, when they do not own the document: leave it.
    Leave,
    /// The user's own row as an owner. An owner cannot leave what they own, and
    /// revoking themselves would be a departure that chooses the next key.
    None,
}

/// The action a row offers, from whose row it is and the role it holds. A row
/// read before the session is known is treated as another member's.
pub(crate) fn row_action(member: Id<User>, role: Role, me: Option<Id<User>>) -> RowAction {
    match me {
        Some(me) if me == member && role == Role::Owner => RowAction::None,
        Some(me) if me == member => RowAction::Leave,
        _ => RowAction::Revoke,
    }
}

/// The mark a row carries when the signed membership log does not vouch for
/// it (`SYNC-108`). A verified row carries none.
fn standing_badge(standing: MemberStanding) -> Option<impl IntoView> {
    let (label, title) = match standing {
        MemberStanding::Verified => return None,
        MemberStanding::Unverified => (
            "Unverified",
            "No signed entry links this member to the document's creator",
        ),
        MemberStanding::Revoked => ("Revoked", "A signed entry removed this member"),
    };
    Some(view! { <span class="ambre-share-member-standing" title=title>{label}</span> })
}

/// The members of the current document, one row each. `send` takes every
/// command a row issues; the caller stamps it with a request id.
#[component]
pub(crate) fn ShareMembers(
    members: RwSignal<Vec<MemberInfo>>,
    me: RwSignal<Option<Id<User>>>,
    current_doc: RwSignal<Option<Id<Document>>>,
    send: Callback<RestCommand>,
) -> impl IntoView {
    // A departure takes a second click: it cannot be undone from this side,
    // since only an owner can invite the user back.
    let confirm_leave = RwSignal::new(false);
    view! {
        <ul class="ambre-share-members">
            <For each=move || members.get() key=|m| m.user_id let:member>
                {
                    let uid = member.user_id;
                    let cur_role = member.role;
                    let on_role = move |ev| {
                        let Some(document_id) = current_doc.get_untracked() else {
                            return;
                        };
                        if let Ok(role) = event_target_value(&ev).parse::<Role>() {
                            send.run(RestCommand::UpdateDocumentMemberRole {
                                document_id,
                                user_id: uid,
                                role,
                            });
                        }
                    };
                    let action = move || row_action(uid, cur_role, me.get());
                    view! {
                        <li>
                            <span class="ambre-share-member-name">
                                {member.display_name.as_str().to_owned()}
                                {standing_badge(member.standing)}
                            </span>
                            <span class="ambre-share-member-email">{member.email.as_str().to_owned()}</span>
                            <select prop:value=cur_role.as_str() on:change=on_role>
                                <option value="owner" selected=cur_role == Role::Owner>"Owner"</option>
                                <option value="editor" selected=cur_role == Role::Editor>"Editor"</option>
                                <option value="viewer" selected=cur_role == Role::Viewer>"Viewer"</option>
                            </select>
                            {move || match action() {
                                RowAction::Revoke => view! {
                                    <button
                                        class="ambre-share-revoke"
                                        title="Revoke access"
                                        aria-label="Revoke access"
                                        on:click=move |_| {
                                            if let Some(document_id) = current_doc.get_untracked() {
                                                send.run(RestCommand::RevokeFromDocument {
                                                    document_id,
                                                    user_id: uid,
                                                });
                                            }
                                        }
                                    >
                                        "Revoke"
                                    </button>
                                }
                                .into_any(),
                                RowAction::Leave => view! {
                                    <Show
                                        when=move || confirm_leave.get()
                                        fallback=move || view! {
                                            <button
                                                class="ambre-share-leave"
                                                title="Leave this document"
                                                on:click=move |_| confirm_leave.set(true)
                                            >
                                                "Leave"
                                            </button>
                                        }
                                    >
                                        <span class="ambre-share-leave-ask">
                                            "Only an owner can invite you back."
                                        </span>
                                        <button
                                            class="ambre-share-leave-confirm"
                                            on:click=move |_| {
                                                confirm_leave.set(false);
                                                if let Some(document_id) = current_doc.get_untracked() {
                                                    send.run(RestCommand::LeaveDocument { document_id });
                                                }
                                            }
                                        >
                                            "Leave document"
                                        </button>
                                        <button on:click=move |_| confirm_leave.set(false)>
                                            "Cancel"
                                        </button>
                                    </Show>
                                }
                                .into_any(),
                                RowAction::None => ().into_any(),
                            }}
                        </li>
                    }
                }
            </For>
        </ul>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use leptos::mount::mount_to;
    use leptos::wasm_bindgen::JsCast;
    use wasm_bindgen_test::wasm_bindgen_test;

    fn member(byte: u8, role: Role) -> MemberInfo {
        MemberInfo {
            user_id: Id::from_bytes([byte; 16]),
            display_name: ambre_core::DisplayName::new(&format!("member {byte}")).unwrap(),
            email: ambre_bus::Email::new(format!("m{byte}@example.com")),
            role,
            standing: MemberStanding::Verified,
        }
    }

    /// Mount the list for a document owned by member 1, with `me` signed in,
    /// and hand back the root and every command a row sent.
    fn mount(me: u8) -> (web_sys::Element, RwSignal<Vec<RestCommand>>, Id<Document>) {
        let document_id = Id::<Document>::mint();
        let sent = RwSignal::new(Vec::<RestCommand>::new());
        let members = RwSignal::new(vec![member(1, Role::Owner), member(2, Role::Editor)]);
        let me = RwSignal::new(Some(Id::from_bytes([me; 16])));
        let current_doc = RwSignal::new(Some(document_id));
        let send = Callback::new(move |cmd| sent.update(|v| v.push(cmd)));
        let container = document().create_element("div").unwrap();
        document().body().unwrap().append_child(&container).unwrap();
        let target: web_sys::HtmlElement = container.clone().dyn_into().unwrap();
        mount_to(target, move || {
            view! { <ShareMembers members=members me=me current_doc=current_doc send=send /> }
        })
        .forget();
        (container, sent, document_id)
    }

    fn click(root: &web_sys::Element, selector: &str) {
        let el: web_sys::HtmlElement = root
            .query_selector(selector)
            .unwrap()
            .unwrap_or_else(|| panic!("no `{selector}` on screen"))
            .dyn_into()
            .unwrap();
        el.click();
    }

    /// `SYNC-141`: an editor leaves from their own row, after a confirmation,
    /// and the one command sent is the departure from this document.
    #[wasm_bindgen_test]
    async fn a_member_who_does_not_own_the_document_leaves_it_from_their_row() {
        let (root, sent, document_id) = mount(2);
        // The owner's row offers a revoke; the user's own row does not.
        assert_eq!(
            root.query_selector_all(".ambre-share-revoke")
                .unwrap()
                .length(),
            1
        );
        click(&root, ".ambre-share-leave");
        assert!(sent.get_untracked().is_empty(), "one click sends nothing");
        // The confirmation is drawn by an effect, which runs on the next tick.
        leptos::task::tick().await;
        click(&root, ".ambre-share-leave-confirm");
        let sent = sent.get_untracked();
        assert_eq!(sent.len(), 1, "one command: {sent:?}");
        assert!(
            matches!(sent[0], RestCommand::LeaveDocument { document_id: d } if d == document_id),
            "{sent:?}"
        );
        root.remove();
    }

    /// `SYNC-142`: an owner's own row offers no departure and no revoke.
    #[wasm_bindgen_test]
    fn an_owner_is_offered_no_departure() {
        let (root, _sent, _) = mount(1);
        assert!(root.query_selector(".ambre-share-leave").unwrap().is_none());
        assert_eq!(
            root.query_selector_all(".ambre-share-revoke")
                .unwrap()
                .length(),
            1
        );
        root.remove();
    }

    /// The row decision, without a browser: whose row, and which role.
    #[wasm_bindgen_test]
    fn each_row_offers_the_action_its_owner_may_take() {
        let (me, other) = (Id::from_bytes([1; 16]), Id::from_bytes([2; 16]));
        assert_eq!(row_action(other, Role::Owner, Some(me)), RowAction::Revoke);
        assert_eq!(row_action(me, Role::Editor, Some(me)), RowAction::Leave);
        assert_eq!(row_action(me, Role::Viewer, Some(me)), RowAction::Leave);
        assert_eq!(row_action(me, Role::Owner, Some(me)), RowAction::None);
        assert_eq!(row_action(me, Role::Editor, None), RowAction::Revoke);
    }
}
