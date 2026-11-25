# AMBRE -- FEATURE: MEMBERSHIP AND THE WRAPPED KEY

**Type:** Horizontal (framework)

> Sharing a document hands one person the key to it, sealed for that person alone. The relay stores
> the sealed copy and can open none of them. What the recipient may then do, and whose cursor is
> whose, come from the same answer.

One request answers three questions at once: which key opens this document, who the other members
are, and what this client is allowed to do. This document owns all three, because they arrive
together and drift apart the moment they are fetched twice.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `SYNC`, shared with
`ambre-spec-sync.md` and the four other documents of this layer.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-039` | `INV` | No key material MUST cross to the Logic thread on a member row. | `ambre-spec-zero-knowledge.md`, `ZK-002` | type-level -- the member row the bus declares carries an identity, a name, an address and a role, and no key field, so there is no shape for one to travel in | none | verified |
| `SYNC-092` | `INV` | The wrapped key, the member directory and this client's role MUST come from one answer, never from two fetches. | this spec section 3 | `ambre_sync::updates::tests::the_key_the_directory_and_the_role_come_from_one_answer` -- the open reads all three through one function that takes one answer and nothing else | none | verified |
| `SYNC-132` | `INV` | Every invite, revoke and role change MUST be recorded as an entry of the membership log, signed by the member who made it. | this spec section 3 | `ambre_sync::sharing::tests::a_new_document_signs_its_first_entry_and_seals_its_digest` -- the creation posts the first entry, signed by the creator; `::an_invite_posts_its_signed_entry_beside_the_copy`; `::a_revoke_posts_the_entry_that_removes_the_member`; `::a_member_who_leaves_signs_their_own_removal`; `::a_role_change_posts_the_entry_that_gives_the_role` -- each change posts an entry its author signed; `::an_invite_overtaken_in_the_log_is_signed_again_after_it` -- an entry whose place another took is signed again at the next one; `ambre_sync::members::tests::the_author_is_the_signed_in_user` -- the signer is the signed-in user, and nobody before the signing key lands; `ambre_sync::sharing::tests::a_log_that_keeps_moving_ends_after_the_tries` -- an invite and a leave read the log again at each try, and give up after the last; `ambre_sync::sharing::tests::a_role_change_ends_after_its_tries` -- a role change ends after its tries, and another refusal ends it at once | none | verified |
| `SYNC-102` | `INV` | An entry MUST name its document, its member, its act and the digest of the entry before it. | this spec section 3 | `ambre_protocol::membership::tests::every_field_but_the_signature_is_signed` -- a change to any named field breaks the signature; `::an_entry_names_a_previous_digest_unless_it_is_the_first`; `ambre_sync::membership_log::tests::a_row_is_verified_only_through_a_chain_to_the_first_entry` -- an entry with a wrong digest breaks the chain | none | verified |
| `SYNC-197` | `INV` | An entry MUST name the exchange key of its member, under the signature of its author. | this spec section 3; `ambre-spec-key-rotation.md`, `SYNC-198` | `ambre_protocol::membership::tests::every_field_but_the_signature_is_signed` -- a change of the exchange key changes the bytes signed; `ambre_sync::membership_log::tests::the_exchange_key_is_the_one_the_adding_entry_signs` -- the walk holds the key the adding entry signs, a role change keeps it, a revoke drops it, and a swapped key ends the chain | none | verified |
| `SYNC-103` | `INV` | A member row MUST count as verified only when a chain of valid entries links it to the first entry of the log. | this spec section 3 | `ambre_sync::membership_log::tests::a_row_is_verified_only_through_a_chain_to_the_first_entry` -- a row past a broken link, or under another first entry, is unverified; `ambre_sync::membership_log::tests::only_an_owner_creating_for_themselves_roots_the_log`; `::an_entry_signed_for_another_document_ends_the_chain` -- a first entry other than an owner creating for themselves, and an entry signed for another document, each verify nothing | none | verified |
| `SYNC-104` | `INV` | The first entry of a log MUST reach a new member from its inviter, sealed with the wrapped key, never from the relay alone. | this spec section 3; `SYNC-073` | `ambre_sync::sharing::tests::a_new_document_signs_its_first_entry_and_seals_its_digest` -- the owner's copy seals the digest of the first entry; `::an_invite_posts_its_signed_entry_beside_the_copy` -- the invitee's copy opens to the same digest; `ambre_crypto::tests::the_log_anchor_travels_sealed_with_the_key` -- the anchor rides inside the seal; `::the_log_anchor_travels_sealed_with_a_catalogue_key` -- and inside a seal under a catalogue key | none | verified |
| `SYNC-289` | `INV` | A wrapped key MUST open with the byte of its version, and its seal MUST bind the wrap domain, that byte and the document whose rows carry the copy as additional data. A wrap with no version byte, or one a reader opens as a copy of another document, MUST be refused. | this spec section 3, the wrap; `SYNC-073`; `ambre-spec-server.md`, `SRV-272` | `ambre_crypto::tests::a_wrap_opens_for_its_own_document_alone_and_only_versioned` -- a genuine wrap served for another document does not open, nor one whose byte names no version, nor one stripped of the byte; `ambre_core::encoded::tests::a_fixed_role_refuses_any_other_length` -- the wire reads no unversioned length; `ambre_sync::updates::tests::a_channel_document_opens_under_the_log_and_links_of_its_meta` -- a copy of the meta does not unwrap as one of its segment | none | verified |
| `SYNC-038` | `INV` | The member list shown to the user MUST NOT carry an invented identity. | this spec section 3 | `ambre_sync::sharing::tests::map_members_parses_roles_and_refuses_malformed_ids` -- a row whose identifier does not parse is refused when the answer is read, so no row is left to give a fresh one | none | verified |

### 1.2 A document is shared with someone

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-036` | `POST` | The re-fetched invitee key MUST equal the one the user confirmed. | `ambre-spec-key-verification.md` | `ambre_sync::sharing::tests::invite_pin_mismatch_refuses_and_never_wraps` -- a key substituted at the second phase answers a 409 naming the change; `::fetch_invitee_key_emits_key_and_matching_fingerprint` -- the fingerprint the user reads is the one that will be compared | none | verified |
| `SYNC-161` | `FAIL` | Given an invitee email the relay answers with another user than the one confirmed, the invite MUST be refused before any wrap. | `ambre-spec-key-verification.md`; this spec section 3 | `ambre_sync::sharing::tests::an_email_naming_another_user_refuses_the_invite` -- a 409 naming the change, and only the search ran | none | verified |
| `SYNC-172` | `POST` | An invitee known by id alone MUST have its keys fetched by that id, checked against the pin, and shared by that id. | `ambre-spec-key-verification.md`; `ambre-spec-server.md`, `SRV-224` | `ambre_sync::sharing::tests::an_invite_by_id_fetches_the_keys_of_that_id_and_shares_by_it` -- the keys of the id are fetched with no search, and the share names the id alone; `::an_invite_by_id_refuses_a_key_other_than_the_pin` -- a substituted key answers a 409, and only the fetch ran | none | verified |
| `SYNC-010` | `POST` | A shared document key MUST be wrapped for the invitee's exchange key alone. | this spec section 3 | `ambre_sync::sharing::tests::invite_request_wraps_for_the_invitee_only` -- the invitee unwraps and a third party's key fails closed; `ambre_crypto::tests::kek_binds_peer_identity`; `::wrap_unwrap_roundtrip` -- the primitive under it | none | verified |
| `SYNC-073` | `POST` | The key that wraps a document key MUST bind the two identities the wrap travels between. | audit `M14`; this spec section 3 | `ambre_crypto::tests::kek_binds_peer_identity` -- a key derived for one pair does not open for another, which is what keeps a wrap from being replayed between members; `::a_shared_secret_differs_per_peer` -- the secret under the key comes from the exchange, so the two public keys alone derive nothing | none | verified |
| `SYNC-035` | `FAIL` | A wrap MUST be refused for a peer key with no contribution, rather than sealed under a predictable secret. | audit `M14`; this spec section 3 | `ambre_crypto::tests::weak_peer_public_key_rejected` -- the key is refused at the primitive; `ambre_sync::sharing::tests::invite_request_propagates_bad_pubkey`; `::decode_exchange_pubkey_rejects_bad_input` -- the refusal survives the layer above rather than being swallowed | none | verified |
| `SYNC-208` | `POST` | An invite that lands MUST hand the inviter a link to the document that carries the pin of the inviter's own signing key. | this spec section 3; `SYNC-205` | `ambre_sync::sharing::tests::invite_success_posts_wrapped_key_and_emits_invitesent` -- the landed invite carries the pin of the key that signs the inviter's entries; `ambre_app::switchboard::tests::a_landed_invite_shows_the_link_pinned_to_its_inviter` -- the shell hands the share dialog of the document the link pinned to that key; `ambre_app::route::tests::an_invite_fragment_reads_back_to_its_pin` -- the fragment of the link reads back to the same pin | none | verified |
| `SYNC-245` | `POST` | An invite that lands MUST show its link in the share dialog of its document, or in the toolbar of a Murmure page, with a button that copies the link inside the click. | this spec section 3; `SYNC-208` | `ambre_app::shell::share::tests::the_dialog_shows_the_last_invite_link_of_its_document` -- the dialog shows the link of the last invite and a copy button, and no link of another document; `ambre_app::switchboard::tests::a_landed_invite_shows_the_link_pinned_to_its_inviter` -- no effect writes the clipboard when the invite lands; `ambre_app::switchboard::tests::a_landed_invite_on_a_murmure_page_shows_its_link` -- a Murmure page shows the link of an invite into a space, whose pin the invitee's page reads (`SYNC-236`); `ambre_belt::prose::tests::the_share_dialog_copies_the_invite_link_on_a_click` -- on Chrome and on Firefox, a click on the button puts the link on the clipboard, and the link opens the document for the invitee | none | verified |
| `SYNC-246` | `POST` | An invite on a document the relay does not hold yet MUST wait for its creation, start it, and then go ahead. | this spec section 3, the invite steps; `ambre-spec-media.md`, `SYNC-145` | `ambre_sync::members::tests::an_invite_waits_for_its_document_on_the_relay` -- an invite on a local-only document answers nothing, then lands once the document is created; `ambre_belt::prose::tests::an_invite_on_a_fresh_document_lands` -- an invite from a Prose document with no edit shows the invitee in the member list, and the invitee opens the document from the link | none | verified |
| `SYNC-247` | `FAIL` | Given an invite that waits for a creation that fails, or for a document closed first, the client MUST refuse the invite. | this spec section 3, the invite steps; `SYNC-246` | `ambre_sync::members::tests::an_invite_waits_for_its_document_on_the_relay` -- a creation that fails and a close that comes first each answer one refusal of the invite | none | verified |
| `SYNC-093` | `FRAME` | A refused invitation MUST wrap nothing: no request MUST leave beyond the one that fetched the key. | `ambre-spec-key-verification.md`; audit `M14` | `ambre_sync::sharing::tests::invite_pin_mismatch_refuses_and_never_wraps` -- exactly one call was made, the search, so no wrap ran and no share request fired. It is the row that makes the pin comparison worth anything: refusing after the wrap has left tells the attacker the key anyway | none | verified |

### 1.3 A document is opened, and its key unwrapped

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-016` | `POST` | The role carried on the ready signal MUST be this client's own row in the members answer. | this spec section 3; `ambre-spec-message-bus.md`, `BUS-008` | `ambre_sync::sharing::tests::own_role_reads_our_own_row` -- a peer holding owner does not raise ours; `::own_role_reads_writable_roles` -- the writable side; `ambre_sync::updates::tests::document_ready_carries_the_role_on_both_paths` -- both open paths carry it | none | verified |
| `SYNC-017` | `FAIL` | A role that cannot be resolved MUST default to the read-only one, never to a writable one. | this spec section 3 | `ambre_sync::sharing::tests::own_role_falls_back_to_viewer_when_unresolvable` -- both unresolvable cases, no session identity yet and no row for us in the answer, answer the viewer role | none | verified |
| `SYNC-034` | `POST` | A recipient MUST unwrap against the exchange key of whoever wrapped, never against its own. | this spec section 3 | `ambre_crypto::tests::ecdh_symmetric`; `::kek_binds_peer_identity` -- the primitive under the rule; `ambre_sync::updates::tests::a_copy_unwraps_against_its_wrapper_and_never_its_holder` -- a peer's copy opens against the peer and fails against the holder, and a row naming no wrapper reads as self-wrapped | none | verified |
| `SYNC-040` | `FAIL` | A transient failure of the members request MUST NOT be read as a document that does not exist. | this spec section 3 | `ambre_sync::updates::tests::a_failed_members_request_is_not_an_absent_document` -- a transport failure is classified apart from an absence, and the open reports it instead of making the document local; `ambre_sync::updates::tests::a_refused_members_request_keeps_the_relay_refusal` -- a refused request keeps the relay's answer as the cause of the failed open, which never reads as a relay out of reach | none | verified |
| `SYNC-279` | `FAIL` | Given an open whose invite link pinned an inviter, a members answer that reads absent MUST fail the open instead of making the document local. | this spec section 3; `SYNC-040`, `SYNC-205` | `ambre_sync::updates::tests::a_pinned_document_never_opens_local_on_a_404` -- an absent answer opens local only with no pin | none | verified |
| `SYNC-012` | `FRAME` | A failed unwrap of a document key MUST NOT be replaced by a freshly generated one. | derived (audit `H3`) | `ambre_sync::updates::tests::a_key_that_does_not_unwrap_opens_nothing` -- a tampered key row fails the open, and no key comes out in its place | none | verified |
| `SYNC-173` | `FAIL` | Given an open that fails, the client MUST tell a key it does not hold apart from a failure that a later open can clear. | this spec section 3; `MURMURE-218` | `ambre_sync::updates::tests::a_key_that_does_not_unwrap_opens_nothing` -- a key row that does not unwrap fails as a key not received; `ambre_sync::updates::tests::a_key_the_invitation_does_not_name_is_refused` -- so does a key the invitation does not name; `ambre_sync::websocket::tests::a_first_replay_given_up_fails_the_open` -- a stopped replay fails as transient; `ambre_sync::updates::tests::a_refused_members_request_fails_the_open_as_refused` -- a `4xx` members answer fails as refused, with its code, and a `5xx` stays transient | none | verified |
| `SYNC-200` | `FAIL` | Given a key that no writer the log verifies named, in an entry or as the key a link it signed starts from, the client MUST refuse the key. | this spec section 3; `SYNC-121`, `SYNC-123` | `ambre_sync::key_vouch::tests::a_key_no_writer_named_is_refused` -- the key the creation names opens, and so does a key a writer's link starts from, while a key the relay wrapped itself is refused with a link the relay signed or with none; `::a_copy_with_no_anchored_log_is_refused` -- a copy with no anchor is refused; `ambre_sync::membership_log::tests::only_a_writers_entry_names_a_key` -- a self-join and a reader's departure name no key; `ambre_sync::updates::tests::a_key_the_relay_substitutes_is_refused_at_open` -- a members answer where the relay re-wrapped a key of its own and dropped the invite fails the open, and a key no writer named under a wrapper the log verifies fails it as a key no member signed (`SYNC-212`) | none | verified |
| `SYNC-201` | `FAIL` | Given a copy wrapped under an exchange key that is neither this client's nor one a valid entry signs for a verified member, the client MUST refuse the key. | this spec section 3; `SYNC-034`, `SYNC-103`, `SYNC-197` | `ambre_sync::key_vouch::tests::a_wrapper_the_log_does_not_verify_is_refused` -- a copy this client wrapped and one a verified member wrapped pass, a wrapper the log signs for no member is refused, and under a space a wrapper no row carries passes only as a present writer of the space; `ambre_sync::key_vouch::tests::a_wrapper_key_the_relay_swaps_on_a_row_is_refused` -- the relay's key on a verified member's row is refused, under a space too, and a withheld row takes no vouch away; `ambre_sync::updates::tests::a_key_the_relay_substitutes_is_refused_at_open` -- the honest answer, wrapped by a verified member, opens, and the swap under the whole log is refused | none | verified |
| `SYNC-282` | `FAIL` | Given an entry that binds a signing key the log already binds to another member, the walk MUST stop at it. | this spec section 3; `SYNC-215` | `ambre_sync::membership_log::tests::a_key_bound_to_one_member_binds_no_other` -- an editor's invite of a third user under Alice's key stops the walk, Alice's key stays hers, and an invite under a key of its own walks | none | verified |
| `SYNC-283` | `FAIL` | Given a copy under a space whose wrapper no row carries, the client MUST refuse the key unless the space holds that wrapper as a present writer at its head. | this spec section 3; `SYNC-201` | `ambre_sync::key_vouch::tests::a_wrapper_the_log_does_not_verify_is_refused` -- an empty space word and a space reader refuse the wrapper, and a present writer of the space passes; `ambre_sync::key_vouch::tests::a_key_no_writer_named_is_refused` -- a key the relay wraps under a space is refused at its wrapper | none | verified |
| `SYNC-284` | `FAIL` | Given a document not open, a rename MUST seal its name only under a key that the checks of an open accept. | this spec section 3; `SYNC-200`, `SYNC-201`, `SYNC-272` | `ambre_sync::updates::tests::a_rename_of_a_document_not_open_takes_a_checked_key_alone` -- the key path the rename task runs reads the members and the links over a scripted transport, takes the key Alice named, and refuses a key the relay wrapped for Bob under its own exchange key | none | verified |
| `SYNC-290` | `FAIL` | Given a log head this client walked or signed in an earlier session, a key a members answer hands MUST pass the checks of an open from that head, so a log the relay cut before an entry walked then installs no key. | this spec section 3; `SYNC-272` | `ambre_sync::members::tests::a_log_head_outlives_the_session_through_the_local_store` -- a walked footing goes to the store with its key once, and the kept log the next session holds refuses a log cut before its head; `ambre_logic::dispatch::tests::a_kept_log_goes_to_the_store_and_back_to_sync`; `ambre_app::kept::tests::the_kept_logs_of_a_user_go_ahead_of_its_opens_alone` -- held per user, a log moved on the page wins; `ambre_logic::persistence::wasm_tests::a_kept_log_reads_back_for_its_user_and_a_later_head_replaces_it` -- at rest, wasm tier; `ambre_bus::host::a_kept_log_reads_back_from_its_row` -- the row the bus host decodes | none | verified |
| `SYNC-291` | `FAIL` | Given a listing of documents or of channels, a name MUST open only under the key the walked or kept log of its document accepted last, or a key the chain of `SYNC-302` reaches from it, with no request of its own; a document never walked MUST show no name. | this spec section 3; `SYNC-284`, `SYNC-290` | `ambre_sync::channels::tests::a_listed_name_opens_under_the_users_key_of_its_meta` -- a name opens under the kept key, and a meta never walked or whose log accepted another key shows none | none | verified |
| `SYNC-302` | `FAIL` | Given a listed name sealed under a key older than the one the kept log of its document accepted last, the client MUST open it through the links the latest open or rotation of that log held in the session, with no request of its own. | this spec section 3; `SYNC-291`, `SYNC-119`, `SYNC-204` | `ambre_sync::documents::tests::a_listed_name_sealed_before_the_last_rotation_opens_through_the_kept_chain` -- a name sealed one rotation back opens through the kept chain, and shows none with no chain, under another kept key or with no kept log, and an open keeps the links it accepted; `ambre_sync::channels::tests::a_listed_name_opens_under_the_users_key_of_its_meta` -- a listing of channels takes the kept chains beside the kept keys | none | verified |
| `SYNC-308` | `FAIL` | Given a kept log, the local store MUST keep the chain of links its latest open or rotation held beside it, under the same account, and a reload MUST rebuild that chain; a stored chain the kept log no longer extends MUST be refused whole. | this spec section 3; `SYNC-290`, `SYNC-302`, `SYNC-121` | `ambre_sync::members::tests::a_kept_chain_outlives_the_session_and_one_its_log_no_longer_extends_is_refused` -- the chain rides beside the kept log and stays the chain of the session, the next session rebuilds it and opens a key two rotations back, and a link past the kept head, a link the kept key never reaches and a log walked from another anchor are refused; `ambre_bus::host::a_kept_chain_reads_back_and_an_older_row_reads_as_none` -- the row reads the chain back, and a row of the older form or with bytes past it reads as none; `ambre_sync::chain::tests::a_kept_link_at_the_kept_place_after_another_entry_is_refused` -- a link at the place of the kept head after another entry refuses the chain, and one at or before the head keeps it | none | verified |
| `SYNC-313` | `FAIL` | Given a kept log, its row MUST hold only the links the walk back from its kept key reaches, nearest first, and no more than the bound of `ambre-spec-budgets.md` section 6; a reload MUST accept that row under the checks of `SYNC-308` alone. | this spec section 3; `SYNC-308`, `SYNC-119`; `ambre-spec-budgets.md` | `ambre_sync::members::tests::the_row_of_a_kept_log_keeps_the_nearest_links_its_key_walks` -- a document rotated forty times, with a link past a history boundary, keeps the 32 nearest links of the walk under the ceiling in bytes, and the next session accepts the row, reaches the key 32 rotations back and none older | none | verified |
| `SYNC-202` | `FAIL` | Given a key that a link a writer signed replaced, the client MUST refuse the key. | this spec section 3; `SYNC-116`; audit `N13` | `ambre_sync::key_vouch::tests::a_key_a_writer_replaced_is_refused` -- a copy the relay kept from before a rotation is refused once the link of that rotation is served | none | verified |
| `SYNC-221` | `FAIL` | Given a key that only links vouch for, and a writer who named another key in an entry at or past the head of each such link, the client MUST refuse the key. | audit `N14`; this spec section 3; `ambre-spec-key-rotation.md`, `SYNC-204` | `ambre_sync::key_vouch::tests::a_key_a_revoked_writer_links_is_refused` -- a revoked editor's link at a head before its revoke is kept by the signer check, and refused as backdated since the revoke named another key, while the owner's own link at the head she read passes | none | verified |
| `SYNC-222` | `FAIL` | Given a member who left on their own, a key a writer named before the departure or a link before it vouches for, and a members answer that owes no rotation, the client MUST refuse the key. A departure MUST name no key. | this spec section 3; `SYNC-221`; `ambre-spec-key-rotation.md`, `SYNC-127`, `SYNC-130` | `ambre_sync::key_vouch::tests::a_key_held_before_a_departure_is_refused_unless_the_debt_is_owed` -- a leaver's link at a head before its departure, the key before it and the creation's key are each refused under a settled answer and stand while the debt is owed, and the key that pays the debt stands; `ambre_sync::key_vouch::tests::a_departure_names_no_key` -- a departure that names a key of the leaver's leaves it unvouched | none | verified |
| `SYNC-225` | `FAIL` | Given a log that holds a revoke naming a key, the client MUST refuse any other key that no chain of links leads back to that key from. | this spec section 3; `SYNC-221`; `ambre-spec-key-rotation.md`, `SYNC-224` | `ambre_sync::key_vouch::tests::a_key_that_does_not_follow_the_latest_revoke_is_refused` -- with the revoke's link withheld, the key before the revoke passes every other check and is refused, as is a key whose link leads to that key alone, while the revoke's key and a key whose link leads back to it stand | none | verified |
| `SYNC-205` | `FAIL` | Given an open of a document whose invite link pinned an inviter, the client MUST refuse the key unless a valid entry of the log the open walks is signed by the key the pin names. | this spec section 3; `SYNC-104`, `SYNC-200`, `SYNC-201`; audit `N15` | `ambre_sync::key_vouch::tests::a_log_the_pinned_inviter_did_not_sign_is_refused` -- a whole log the relay rooted at a creator of its own passes every other check, and the pin of the true inviter refuses it while it admits the true log, and a log with no anchor or a pin of a member who signed nothing admits nothing; `ambre_sync::updates::tests::an_open_pinned_to_its_inviter_refuses_a_log_the_relay_rooted` -- the open accepts the relay's log unpinned and refuses it pinned, as a key not received; `ambre_sync::updates::tests::a_pinned_inviter_outlives_a_close` -- the pin holds for every open of the session; `ambre_crypto::tests::an_inviter_pin_is_stable_and_differs_per_key` -- the pin under it | none | verified |
| `SYNC-209` | `POST` | The pin a link carries MUST reach the Sync worker before the open that link drives. | this spec section 3; `SYNC-205` | `ambre_app::route::tests::an_invite_fragment_reads_back_to_its_pin` -- the route reads the pin off the fragment, and a malformed one pins nothing; `ambre_logic::dispatch::tests::a_pinned_inviter_reaches_sync_before_its_open` -- the Logic thread forwards the pin ahead of the open | none | verified |
| `SYNC-210` | `FAIL` | Given an open of a Prose, Trame or Toile document that names no pinned inviter, the client MUST refuse the key unless this client wrapped its own copy or a valid entry of the log is signed by this client's own signing key. | this spec section 3; `SYNC-205` | `ambre_sync::key_vouch::tests::an_open_with_no_link_is_refused_unless_this_client_vouches_for_the_log` -- an invitee with no link is refused on the inviter's log and on a log the relay rooted alike, a copy the invitee wrapped itself opens, and the creator opens a copy another member wrapped for it, since it signed the creation, but not when the open knows no signing key of its own; `ambre_sync::updates::tests::an_open_pinned_to_its_inviter_refuses_a_log_the_relay_rooted` -- the open with no link fails as a key not received on either log | none | verified |
| `SYNC-212` | `FAIL` | Given a key refused because no writer the log verifies named it, or because the pinned inviter signed no entry of the log, the open MUST fail as a key no member signed, never as a key not received. | this spec section 3; `SYNC-173`, `SYNC-200`, `SYNC-205`; `ambre-spec-message-bus.md`, `BUS-030` | `ambre_sync::key_vouch::tests::only_a_key_no_member_signed_fails_as_the_relays_fault` -- an unnamed key and a log the inviter did not sign fail as a key no member signed, and every refusal an honest relay can cause stays a key not received; `ambre_sync::updates::tests::a_key_the_relay_substitutes_is_refused_at_open` -- a key no writer named, under the genuine log and a wrapper it verifies, fails the open that way; `ambre_sync::updates::tests::an_open_pinned_to_its_inviter_refuses_a_log_the_relay_rooted` -- so does a log the relay rooted, pinned, while the open with no link stays a key not received; `ambre_bus::host::a_key_no_member_signed_crosses_the_bus_as_its_own_failure` -- the failure crosses the bus with its own bytes and its own words | none | verified |
| `SYNC-226` | `FAIL` | Given a key that the links a writer signed do not walk back to the latest key a valid entry of the log names, the client MUST refuse the key. | audit `N23`; this spec section 3; `SYNC-200`, `SYNC-202`, `SYNC-213` | `ambre_sync::key_vouch::tests::a_key_behind_the_latest_key_the_log_names_is_refused` -- a genuine copy of the key a revoke replaced, served with the link past it withheld, is refused while every other check passes, the key the revoke names opens, and so does a later key whose links walk back to it, while a key whose links start from an older key is refused; `ambre_sync::updates::tests::a_first_open_refuses_a_key_behind_the_latest_key_the_log_names` -- that first open fails as a key not received, and the honest answer opens | none | verified |
| `SYNC-229` | `POST` | The open of a document of a channel MUST check its key and its writers against the log and the links of the `meta` that the sealed state of the channel names. | this spec section 3; `SYNC-200`, `SYNC-211`; `ambre-spec-server.md`, `SRV-272` | `ambre_sync::updates::tests::a_channel_document_opens_under_the_log_and_links_of_its_meta` -- a segment opened naming its `meta` walks the meta's log, grants its writers alone, opens the key a meta's link reaches and keeps that link in its chain, a key the relay wrapped itself is refused, and the same answer walked under the segment reaches no key; `ambre_logic::channels::tests::each_document_of_an_opening_channel_names_its_meta` -- the meta, then each live segment it names, opens naming that meta | none | verified |
| `SYNC-230` | `POST` | A channel whose space opens after it MUST walk the log of its `meta` again under the rule of that space, and that walk MUST decide again each delta an earlier walk refused as no writer's. | this spec section 3; `SYNC-106`, `SYNC-211`, `SYNC-229` | `ambre_sync::updates::tests::a_channel_opened_before_its_space_walks_its_log_again_under_the_spaces_rule` -- a self-joiner's delta refused under the document rule lands once the Logic thread marks the channel listed, a repeated mark walks nothing, a new word of the space walks the log again, and a word that names no writer of the creation grants nobody; `ambre_logic::spaces::tests::a_space_opened_after_its_channel_marks_that_channel_listed` -- a space opened after a channel marks that channel listed, and no channel left unopened | none | verified |
| `SYNC-299` | `FAIL` | Given a public channel whose catalogue entry holds a channel key, the open of a document of that channel MUST refuse any other channel key, unless links a writer signed walk back from it to that key. | this spec section 3; `SYNC-226`, `SYNC-229`; `5-Murmure/features/ambre-spec-murmure-spaces.md`, `MURMURE-033` | `ambre_sync::key_vouch::tests::a_channel_open_trusts_the_key_its_catalogue_pins` -- the pinned key and a newer key a signed rotation links to it pass, while a key off that chain, a key older than the pin and any key under an entry that did not unseal are refused; `ambre_sync::channels::tests::a_listed_channel_open_pins_the_key_its_catalogue_holds` -- the open of a listed channel pins the key its entry holds in the roots of its `meta`, an entry that does not unseal pins no key, and an unlisted channel pins nothing; `ambre_logic::spaces::tests::a_self_join_hands_the_sealed_key_to_sync_and_writes_no_catalogue` -- the open that follows a self-join hands Sync the space and the catalogue entry | none | verified |
| `SYNC-231` | `FAIL` | Given a walk of a channel log that finds its creation signed by no writer of the space that lists it, the client MUST fail the open of each document of that channel, and the channel MUST end, even when an earlier walk accepted its key and its deltas. | this spec section 3; `SYNC-229`, `SYNC-230`; `ambre-spec-zero-knowledge.md` | `ambre_sync::updates::tests::a_channel_log_rooted_outside_its_space_fails_the_open_at_the_second_walk` -- a channel opened before its space lands a forged delta, then the walk under the space's word fails the open as `RootOutsideSpace`, drops what it held and lands no later delta; `ambre_logic::channels::tests::a_channel_rooted_outside_its_space_ends_and_fails_its_open` -- an open channel ends whole, every document leaves Sync and the view drops it as failed, with one notice; `ambre_logic::channels::tests::an_opening_rooted_outside_its_space_ends` -- an opening ends the same way; `ambre_sync::key_vouch::tests::a_channel_log_no_space_writer_created_is_refused` -- a first open under the word of the space refuses the forged root, and reports the same cause | none | verified |
| `SYNC-232` | `FAIL` | Given a channel document whose log walks again under the rule of its space, the client MUST land no delta and seal no frame under its key until the answer of that walk. A walk that roots the log in the space MUST then land each delta and send each frame it held, in order. | this spec section 3; `SYNC-230`, `SYNC-231`; `ambre-spec-zero-knowledge.md` | `ambre_sync::updates::tests::a_channel_log_walking_again_lands_and_seals_nothing_until_its_answer` -- between the mark and the answer, a writer of the first walk lands nothing, and an edit, a snapshot and a presence sample seal nothing, an older answer decides nothing, and a walk that finds the log rooted outside wipes the held edit and seals no later one; `ambre_sync::updates::tests::a_channel_log_walked_again_and_confirmed_lands_and_seals_what_it_held` -- a walk that roots the log in the space lands the held delta and sends the two held edits in order | none | verified |
| `SYNC-233` | `FAIL` | Given a channel whose log roots outside its space, the client MUST erase the local copy of each document of that channel, whether the channel was open, opening or closed. | this spec section 3; `SYNC-231`; `ambre-spec-residency.md` | `ambre_logic::channels::tests::a_channel_rooted_outside_its_space_erases_each_local_copy` -- a channel closed with a copy of each document reopens, the open of its `meta` fails as `RootOutsideSpace`, and each copy leaves the store, read back or not; `ambre_logic::store_ledger::tests::an_erased_copy_leaves_the_count` -- the ledger stops counting an erased copy and keeps no row of it | none | verified |
| `SYNC-234` | `FAIL` | Given a channel document whose log walks again under the rule of its space, every seal under its key and every adoption of its snapshot MUST wait for the answer of that walk. Past the bound of the walk, the client MUST drop each edit it held, report it and send no later edit until the document reopens. | this spec section 3; `SYNC-232`, `SYNC-155`; `ambre-spec-budgets.md`; `ambre-spec-zero-knowledge.md` | `ambre_sync::updates::tests::a_refused_frame_waits_for_the_walk_and_leaves_at_its_own_clock` -- a frame the relay refused waits for the walk, leaves at its own clock once the log roots in the space, and never leaves when it roots outside; `ambre_sync::updates::tests::a_snapshot_fetched_while_the_log_walks_is_not_adopted` -- a snapshot a writer of the first walk signed reaches no check of the Logic thread, before and after a walk that roots the log outside; `ambre_sync::updates::tests::a_walk_past_its_bound_drops_its_edits_aloud_and_sends_none` -- the edit past the bound raises `EditRefused`, and no edit leaves after the answer; `ambre_sync::channels::tests::every_request_sealing_under_an_unsettled_key_is_refused` -- a media upload, a rename, an invitation, a revoke, a segment close and a channel name are refused as `LogUnsettled`, and an owed rotation is not paid, during the walk and once the log roots outside; `ambre_sync::channels::tests::a_walk_defers_its_snapshots_within_its_bound_and_hands_them_back` -- the walk defers each snapshot once, refuses one past the bound, and its answer hands each one back | none | verified |
| `SYNC-235` | `FAIL` | Given a Murmure open, the client MUST refuse the key unless this client wrapped its own copy, or a valid entry of the log is signed by this client's own signing key or by a root of the session. | this spec section 3; `SYNC-205`, `SYNC-210`; audit `N15` | `ambre_sync::key_vouch::tests::a_murmure_log_no_root_of_the_session_signed_is_refused` -- a whole log the relay rooted at a creator of its own is refused with no root and with the root of the true inviter, the true log opens on that root alone, and a root that signed no entry admits nothing; `ambre_sync::updates::tests::a_channel_document_opens_under_the_log_and_links_of_its_meta` -- a segment opens under the root its Murmure link pinned | none | verified |
| `SYNC-236` | `POST` | The pin a Murmure link carries MUST reach the Sync worker as a root of the session, before the spaces the page opens. | this spec section 3; `SYNC-235` | `ambre_app::route::tests::a_murmure_link_pins_its_inviter_for_the_session` -- a Murmure page reads the pin off its fragment, and a document route or the landing route pins nothing there; `ambre_logic::dispatch::tests::a_murmure_inviter_reaches_sync_before_the_spaces_it_roots` -- the Logic thread forwards the pin ahead of the open of a space; `ambre_sync::updates::tests::a_murmure_link_and_an_accepted_murmure_log_root_the_murmure_opens_alone` -- the pin roots every Murmure open, and no Prose, Trame or Toile open | none | verified |
| `SYNC-237` | `POST` | A Murmure log that an open accepts, or that this client signs an invitee into, MUST make each member it keeps a root of the session. | this spec section 3; `SYNC-235` | `ambre_sync::key_vouch::tests::a_member_of_an_accepted_murmure_log_roots_the_next_open` -- the members of an accepted space log root a direct message one of them created, a member the log revoked roots nothing, and a log with no anchor roots nobody; `ambre_sync::updates::tests::a_murmure_link_and_an_accepted_murmure_log_root_the_murmure_opens_alone` -- an accepted Murmure log adds its members on the Sync loop, a Prose log adds none, and an invite into a Murmure document roots its invitee while one into a Prose document does not; `ambre_sync::sharing::tests::invite_success_posts_wrapped_key_and_emits_invitesent` -- the landed invite carries the pin of the key its entry names | none | verified |
| `SYNC-238` | `FAIL` | Given an open of a document whose earlier open, in this session or an earlier one, proved the pin of its link, the client MUST refuse the key unless a valid entry of the log is signed by the key that pin names. | this spec section 3; `SYNC-205`, `SYNC-210`; `ambre-spec-residency.md` | `ambre_logic::dispatch::tests::a_proven_inviter_goes_to_the_local_store` -- the pin the Sync worker reports proved goes to the local store as a row of its document; `ambre_logic::persistence::wasm_tests::a_kept_inviter_outlives_the_page_and_is_never_replaced` -- wasm tier: the pin reads back on a later open of the browser's store, a user's trust pin beside it is no inviter, and the copy leaving leaves it; `ambre_app::kept::tests::an_open_waits_for_the_kept_pins` -- the kept pin goes ahead of the open; `ambre_sync::updates::tests::an_open_pinned_to_its_inviter_refuses_a_log_the_relay_rooted` -- the pin the open names refuses the log the relay rooted and admits the true one; `ambre_belt::prose::tests::an_invitee_reopens_from_the_list_on_the_pin_its_link_proved` -- browser tier: the invitee reloads and opens from the document list with no link, and the same user in a browser that kept no pin is refused | none | verified |
| `SYNC-239` | `FAIL` | Given a link that pins another inviter than the one kept for its document, the client MUST keep the kept pin and check the open against it. | this spec section 3; `SYNC-238` | `ambre_app::kept::tests::a_kept_pin_wins_over_a_link_and_is_never_replaced` -- the kept pin goes ahead of the open in place of the link's, and a later proof replaces nothing; `ambre_logic::persistence::wasm_tests::a_kept_inviter_outlives_the_page_and_is_never_replaced` -- wasm tier: the store refuses a second row for the document | none | verified |
| `SYNC-240` | `POST` | The Main thread MUST route no open of a document before the local store has answered with its kept pins. | this spec section 3; `SYNC-238` | `ambre_app::kept::tests::an_open_waits_for_the_kept_pins` -- an open waits until the store answers, and a store that never opens answers with none; `ambre_belt::prose::tests::an_invitee_reopens_from_the_list_on_the_pin_its_link_proved` -- browser tier: the first open after a reload passes on the kept pin | none | verified |
| `SYNC-241` | `FAIL` | Given a Murmure open of a document whose earlier open, in this session or an earlier one, proved a pin or a root of the session, the client MUST refuse the key unless a valid entry of the log is signed by the key that pin names. | this spec section 3; `SYNC-238`, `SYNC-235`; audit `N15` | `ambre_sync::key_vouch::tests::a_kept_murmure_root_admits_its_log_after_a_restart_and_no_relay_log` -- after a restart the session holds no root and refuses the true log, and the kept root admits that log and refuses the log the relay rooted; `ambre_sync::updates::tests::a_kept_murmure_pin_roots_its_document_alone_after_a_restart` -- a restarted worker holds no root, and the kept pin the Main thread hands back roots its own document alone; `ambre_belt::murmure::tests::an_invitee_reopens_murmure_without_the_link_on_the_roots_it_kept` -- browser tier: the invitee reloads Murmure with no link, and the channel opens and reads again | none | verified |
| `SYNC-242` | `POST` | A Murmure open that a root of the session admits MUST hand that root to the local store as the kept pin of its document. A log that no pin and no root signed MUST hand none. | this spec section 3; `SYNC-238`, `SYNC-235` | `ambre_sync::key_vouch::tests::a_kept_murmure_root_admits_its_log_after_a_restart_and_no_relay_log` -- the open on the root of the link names that root to keep, and the log the relay rooted names none; `ambre_logic::dispatch::tests::a_proven_inviter_goes_to_the_local_store` -- the pin reaches the local store as a row of its document; `ambre_belt::murmure::tests::an_invitee_reopens_murmure_without_the_link_on_the_roots_it_kept` -- browser tier: the reload finds the roots the first session kept | none | verified |
| `SYNC-243` | `POST` | The Main thread MUST send every pin the local store kept to the Sync worker ahead of the boot of a Murmure session. | this spec section 3; `SYNC-240`, `SYNC-241` | `ambre_app::kept::tests::a_murmure_boot_waits_for_every_kept_pin` -- the boot waits until the store answers, then takes every kept pin, a proof of this page included; `ambre_belt::murmure::tests::an_invitee_reopens_murmure_without_the_link_on_the_roots_it_kept` -- browser tier: the first space open after a reload passes on a kept pin | none | verified |
| `SYNC-244` | `POST` | The read of the kept pins MUST answer the pins that the sessions of the signed-in user proved, and no pin of another user. | this spec section 3; `SYNC-238`, `SYNC-240`, `SYNC-243` | `ambre_logic::persistence::wasm_tests::a_second_account_reads_none_of_the_first_accounts_inviters` -- wasm tier: two accounts of one store each read back their own pins alone, the second keeps its own pin for a document the first pinned, and a row keyed by the document alone reads back for neither; `ambre_app::kept::tests::a_second_account_reads_none_of_the_first_accounts_pins` -- no pin goes before a session names the user, and a second account waits for its own answer, takes none of the first account's pins, and drops a late answer for the first | none | verified |
| `SYNC-260` | `FAIL` | Given a document an open named under a type other than Murmure, the client MUST NOT install a key that an open labelled Murmure brought, for the session, and MUST drop such a key it holds before that open answers. | this spec section 3; `SYNC-200`, `SYNC-201`, `SYNC-202` | `ambre_sync::updates::tests::a_document_relabelled_murmure_keeps_the_key_its_own_open_installed` -- a Toile document relabelled Murmure keeps the key its own open installed, against the Murmure command and the Murmure answer alike, while a document no other open named still opens as Murmure; `ambre_sync::updates::tests::an_open_under_another_type_drops_the_key_a_murmure_open_installed` -- the Toile open drops the unchecked key while it waits, and a late Murmure answer installs nothing; `ambre_sync::updates::tests::a_channel_no_other_open_named_opens_as_murmure` -- the refusal binds to a typed document alone; `ambre_sync::census::tests::a_census_of_a_murmure_document_another_open_claimed_fails` -- a census fails on a Murmure document another open claimed, and opens a claimed document of another type | none | verified |

### 1.4 An ephemeral is attributed

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-014` | `POST` | An ephemeral MUST reach the Logic thread under the identity the member directory maps its author key to. | this spec section 3 | `ambre_sync::ephemeral_tests::author_resolves_to_the_member_behind_the_signing_key`; `ambre_sync::sharing::tests::presence_directory_maps_valid_entries`; `::list_members_refreshes_the_presence_directory` -- the directory the resolution reads | none | verified |
| `SYNC-037` | `POST` | The member directory MUST be rebuilt whenever membership changes. | this spec section 3 | `ambre_sync::sharing::tests::list_members_refreshes_the_presence_directory` -- the members answer replaces it; `ambre_sync::updates::tests::a_change_of_members_reads_the_directory_again_and_a_crossed_answer_changes_nothing` -- the relay's word (`SRV-177`) lists again, and a crossed older answer changes nothing | none | verified |
| `SYNC-150` | `POST` | A change of members committed while a document opens MUST reach the directory that open builds. | this spec section 3; `SRV-177` | `ambre_sync::updates::tests::a_subscription_reads_the_members_a_change_during_the_open_left_out` -- the relay's acknowledgement of a subscription lists the members again, so a newcomer committed before any word could reach this client resolves | none | verified |
| `SYNC-151` | `FRAME` | The relay's word that the members changed MUST NOT cost an open document more than one member request. | this spec section 3; `SRV-177` | `ambre_sync::updates::tests::a_change_of_members_costs_one_listing_whose_rows_reach_logic` -- the word costs one listing, and its fresh answer reaches Logic with the rows; `ambre_logic::spaces::tests::a_change_of_members_hands_the_rows_and_costs_the_space_no_request` -- an open space takes those rows and asks for none | none | verified |
| `SYNC-015` | `FAIL` | An ephemeral whose author key matches no member MUST be dropped, never delivered under a guessed identity. | this spec section 3 | `ambre_sync::ephemeral_tests::unknown_author_key_is_not_resolved`; `ambre_sync::sharing::tests::presence_directory_skips_unparsable_rows_without_inventing_ids` -- a members answer with two rows whose signing key is malformed reads whole, the directory skips both, and only the sound row reaches it | none | verified |
| `SYNC-215` | `FAIL` | Given a member row whose signing key no valid entry of an anchored log bound to that member, the client MUST leave the key out of the directory. | audit `N19`; this spec section 3; `SYNC-197` | `ambre_sync::sharing::tests::a_row_naming_a_key_the_log_binds_to_another_member_is_dropped` -- a row that names another member's key is dropped and the true rows stand, and with no anchored log every row stands | none | verified |
| `SYNC-094` | `FRAME` | Rebuilding the directory MUST NOT keep an entry the new membership does not carry. | this spec section 3; `ambre-spec-key-rotation.md` | `ambre_sync::ephemeral_tests::refreshing_the_directory_drops_revoked_members` -- a key that resolved before the rebuild resolves to nothing after it, where a merge instead of a replacement would leave a revoked member's cursor on the document for as long as the tab stays open | none | verified |

### 1.5 A membership change is recorded

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-105` | `POST` | An invite MUST post its signed entry and the wrapped key in one request. | this spec section 3 | `ambre_sync::sharing::tests::an_invite_posts_its_signed_entry_beside_the_copy` -- the one share request carries both | none | verified |
| `SYNC-123` | `POST` | The signed entry of an invite MUST name the key it wraps. | this spec section 3; `SYNC-115` | `ambre_sync::sharing::tests::an_invite_posts_its_signed_entry_beside_the_copy` -- the entry names the identifier the copy carries; `ambre_sync::channels::tests::a_direct_message_makes_every_member_an_editor_under_one_key` -- the creation and each invite of a direct message name the key, and the role change installs none | none | verified |
| `SYNC-124` | `FAIL` | Given a wrapped key whose identifier differs from the one its invite names, the client MUST refuse the key. | this spec section 3; `SYNC-115` | `ambre_sync::updates::tests::a_key_the_invitation_does_not_name_is_refused` -- a verified invite naming another key leaves the copy unheld | none | verified |
| `SYNC-106` | `POST` | An entry signed by the member it adds MUST count as valid only under a rule of the application that owns the document. | this spec section 3 | `ambre_sync::membership_log::tests::a_self_join_counts_only_under_the_applications_rule` -- the same self-join verifies under an accepting rule and is dropped under the default one; `ambre_sync::sharing::tests::the_member_list_admits_a_self_join_only_in_a_marked_log` -- at the member list, only the log of a catalogued channel's `meta` admits it; `ambre_sync::membership_log::tests::a_public_channel_admits_a_self_join_up_to_editor` -- the public channel rule admits no self-join above editor; `ambre_sync::channels::tests::a_listing_marks_the_meta_of_a_listed_channel_alone` -- a listing marks the log of meta alone, and forgets it once no catalogue lists the channel; `ambre_sync::channels::tests::a_self_join_extends_the_log_the_join_head_names` -- the self-join takes the place and the link the join head names, with the role the relay grants, and an empty log takes none | none | verified |
| `SYNC-190` | `POST` | The log of a public channel MUST admit the self-join of a member of its space from the moment the relay holds the channel, on the client that created it too. | this spec section 3; `SYNC-106` | `ambre_sync::channels::tests::a_created_public_channel_admits_a_self_join_at_once` -- a created public channel marks its log at once, a refused creation and a private channel mark none | none | verified |
| `SYNC-227` | `FAIL` | Given a self-join of a channel log whose member the space log does not hold present under the same signing and exchange keys, the client MUST count it invalid. | audit `N24`; this spec section 3; `SYNC-106` | `ambre_sync::membership_log::tests::a_self_join_counts_only_for_a_present_member_of_the_space_under_its_keys` -- a self-join under a key of the relay, under a swapped exchange key, for a member the space never held, for a member it removed, and with no space word at hand each end the chain, so no call admits the member and no rotation wraps for it; `ambre_sync::sharing::tests::the_member_list_admits_a_self_join_only_in_a_marked_log` -- at the member list, the public-channel rule with no space word verifies no self-join | none | verified |
| `SYNC-107` | `FRAME` | Recording an entry MUST NOT change an earlier entry of the log. | this spec section 3 | `ambre_server::routes::tests::handler_tests::the_register_keeps_each_signed_entry_beside_the_rows` -- a third entry leaves the first two byte for byte, and a stale position is refused | none | verified |
| `SYNC-266` | `FAIL` | Given a members answer whose log does not walk from the anchor the open accepted, a settle, a revoke, an invite or a role change MUST write nothing. | this spec section 3; `SYNC-104`, `SYNC-198`; audit `N32` | type-level -- `entry_after` and `rotate_entries_for` take an `AnchoredLog`, which `Footing::walk` alone builds, and only the checks of an open, a creation this client signed or a sealed catalogue anchor build a `Footing`; `ambre_sync::sharing::tests::a_settle_posts_nothing_on_a_log_the_relay_rooted` -- a log the relay rooted on a copy it wrapped, signing its own exchange key, gets no post; `ambre_sync::sharing::tests::a_revoke_posts_nothing_on_a_log_the_relay_rooted` -- the same answer gets no removal, and the user reads a refusal; `ambre_sync::sharing::tests::an_invite_posts_nothing_on_a_log_the_relay_rooted` -- the same answer gets no share; `ambre_sync::sharing::tests::a_role_change_patches_nothing_on_a_log_the_relay_rooted` -- the same answer gets no role change | none | verified |
| `SYNC-267` | `FAIL` | Given a members answer whose log does not pass through the latest head this client walked or signed, a path that writes members or keys MUST write nothing. | this spec section 3; `SYNC-266`; audit `N33` | `ambre_sync::sharing::tests::a_settle_posts_nothing_on_a_log_cut_before_a_head_it_walked` -- the genuine log cut before a revoke this client walked, with the row of the revoked member, gets no post; `ambre_sync::sharing::tests::a_write_stands_on_the_head_this_client_signed` -- a revoke leaves the head after its own entry, a settle then refuses the log cut before it and accepts the log that holds it; `ambre_sync::sharing::tests::a_footing_moves_forward_on_its_own_log_alone` -- an earlier head and a log on another anchor move nothing | none | verified |
| `SYNC-268` | `FAIL` | Given a log of the `meta` of a channel that does not pass through the latest head this client walked or signed in the session, a re-key of that channel MUST post nothing. | this spec section 3; `SYNC-267`; audit `N33` | `ambre_sync::sharing::tests::a_channel_rekey_posts_nothing_on_a_log_cut_before_a_head_it_walked` -- the genuine log cut before the removal of a member the space still holds gets no post, nor does a log of another `meta`, while the genuine log wraps for the members it holds and a first re-key leaves the head it walked; `ambre_sync::space_rotation::tests::a_rekey_leaves_its_footing_for_the_next_one` -- the footing a re-key leaves lands on the log of its `meta` and no other anchor finds it, while a failed re-key lands nothing | none | verified |
| `SYNC-272` | `FAIL` | Given a members answer whose log does not pass through the latest head this client walked or signed, a fetched rotation, a reopen and a members refresh MUST change nothing. | this spec section 3; `SYNC-267` | `ambre_sync::sharing::tests::a_key_read_refuses_a_log_cut_before_a_revoke_the_session_walked` -- a log cut before a revoke the session walked, a wrap that seals another anchor and a footing of another log are refused, while a log that extends the footing walks and a first open walks from the wrap; `ambre_sync::sharing::tests::a_members_refresh_on_a_cut_log_changes_nothing` -- a refresh on the cut log lands no directory, writers or heads, and a head under check hears of a failed read; `ambre_sync::sharing::tests::a_members_refresh_that_extends_the_footing_lands` -- the whole log lands and moves the footing on | none | verified |

### 1.6 A member list is checked

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-108` | `FAIL` | Given a member row with no valid chain, the client MUST show it as unverified. | this spec section 3 | `ambre_sync::sharing::tests::the_member_list_shows_what_the_log_verifies` -- a row the log does not link reaches the list as unverified; `ambre_sync::membership_log::tests::an_entry_its_author_did_not_sign_or_may_not_make_verifies_nothing`; `ambre_sync::membership_log::tests::an_entry_beyond_its_authors_standing_ends_the_chain` -- an editor re-inviting the owner, or an owner revoking a non-member, ends the chain | none | verified |
| `SYNC-109` | `FAIL` | Given an invite replayed after a later revoke of the same member, the client MUST count the member as revoked. | this spec section 3 | `ambre_sync::membership_log::tests::an_invite_replayed_after_a_revoke_leaves_the_member_revoked` | none | verified |
| `SYNC-110` | `POST` | The role a client shows for a member MUST be the role of the latest valid entry for that member. | this spec section 3 | `ambre_sync::membership_log::tests::the_role_is_the_latest_valid_entrys`; `ambre_sync::sharing::tests::the_member_list_shows_what_the_log_verifies` -- the list carries the logged role over the relay's column; `::a_role_change_posts_the_entry_that_gives_the_role` | none | verified |
| `SYNC-217` | `FAIL` | Given an entry that names, for a present member, a signing key or an exchange key other than the ones the log holds, the client MUST count it invalid. | audit `N17`; this spec section 3; `SRV-266`; `ambre-spec-session.md`, `SRV-030`; `ambre-spec-zero-knowledge.md`, `ZK-009` | `ambre_sync::membership_log::tests::no_entry_replaces_a_present_members_keys` -- an editor's re-invite of present Bob, a self-join for him, and an owner's re-invite under another pair, another exchange key or another signing key each end the chain and leave Bob's keys, while an owner's re-invite under the keys the log holds lands | none | verified |
| `SYNC-324` | `FAIL` | Given an invite of a present member whose keys the relay serves differ from the ones the log holds, the client MUST post nothing and MUST report a relay fault that names the member. | this spec section 3; `SYNC-217`; `ambre-spec-key-rotation.md`, `SYNC-198` | `ambre_sync::sharing::tests::an_invite_of_a_present_member_under_other_keys_posts_nothing` -- another exchange key, another signing key and both keys served for a present editor each end the owner's invite on `KeyReplaced` naming him after the keys and the members alone, with no share posted, while the invite under the keys the log holds lands and keeps them | none | verified |
| `SYNC-325` | `FAIL` | Given an entry that adds again a member the log revoked, under a signing key or an exchange key other than the last ones the log bound to that member, the client MUST count it invalid. | this spec section 3; `SYNC-217`; `ambre-spec-session.md`, `SRV-030`; `ambre-spec-zero-knowledge.md`, `ZK-009` | `ambre_sync::membership_log::tests::a_revoked_member_returns_under_the_pair_the_log_last_bound` -- after the owner revokes Bob, an owner's re-invite under another pair, another exchange key or another signing key, an editor's re-invite and an admitted self-join under another pair each end the chain and leave Bob revoked, while an owner's re-invite under the pair the log last bound lands | none | verified |
| `SYNC-326` | `FAIL` | Given an invite of a member the log revoked, whose keys the relay serves differ from the last ones the log bound to that member, the client MUST post nothing and MUST report a relay fault that names the member. | this spec section 3; `SYNC-324`, `SYNC-325`; `ambre-spec-key-rotation.md`, `SYNC-198` | `ambre_sync::sharing::tests::a_re_invite_of_a_revoked_member_under_other_keys_posts_nothing` -- another exchange key, another signing key and both keys served for a revoked editor each end the owner's re-invite on `KeyReplaced` naming him after the keys and the members alone, with no share posted, while the re-invite under the pair the log last bound lands and makes him present under it | none | verified |
| `SYNC-111` | `FRAME` | Checking a member list MUST NOT block the open of the document, beyond the checks of its key. | this spec section 3 | `ambre_sync::updates::tests::a_key_the_invitation_does_not_name_is_refused` -- the open reads the log and still completes, the check deciding only which key it holds | none | verified |
| `SYNC-206` | `POST` | Every update MUST carry, under its author's signature, the head of the membership log its author last read. | this spec section 3; `ambre-spec-sealed-envelope.md` | `ambre_sync::withheld::tests::a_log_served_whole_clears_the_check_and_heads_the_next_delta` -- the next delta signs the head of the log the client last read; `ambre_sync::pipeline::tests::a_rewritten_log_head_breaks_the_signature` -- a head rewritten in transit fails the signature, alone and at the open | none | verified |
| `SYNC-207` | `FAIL` | Given a member's signed head that the log this client read does not pass through, the client MUST read the log again, and MUST warn the user when that log still does not pass through it. | this spec section 3; `ambre-spec-key-rotation.md` | `ambre_sync::withheld::tests::a_peer_detects_a_revoke_the_relay_hid_from_it` -- a relay hides a revoke and its rotation from Bob, Alice's next delta under the new key still proves its head, the log read again lacks the revoke, and Bob is warned once; `::a_log_served_whole_clears_the_check_and_heads_the_next_delta` -- an honest relay serves the entry and nobody is warned; `::a_head_no_member_signed_asks_nothing` -- a key outside the directory, or a broken signature, asks for no log; `::a_head_past_the_log_asks_once_and_a_log_without_it_is_withheld` -- one read at a time, and an answer to a read that left before the head proves nothing; `::a_served_log_holds_each_head_it_passes_through_and_no_other` -- a head past the log or forking from it is not held | none | verified |
| `SYNC-298` | `FAIL` | Given a member's delta that opens under a key other than the current one, numbered by the relay after a delta under the current key, whose signed head the valid walk passes through short of the head this client read, the client MUST warn the user once for that member. | this spec section 3; `SYNC-206`, `SYNC-207`; `ambre-spec-key-rotation.md` | `ambre_sync::withheld::tests::a_member_writing_behind_the_log_is_seen_by_a_peer` -- a relay hides a revoke from Bob, and his delta under the retired key with the head before the revoke warns Alice once, while history numbered before her delta under the current key, a delta in flight before the rotation, and a stale delta whose head holds the revoke warn nobody; `ambre_sync::withheld::tests::the_current_key_is_in_use_from_its_lowest_delta` -- a later delta under the current key keeps its first number, and a rotation counts the next key from its own | none | verified |
| `SYNC-218` | `POST` | Every update MUST carry, under its author's signature, the last entry of the log its author last read, unless that log is empty. | this spec section 3; `SYNC-206` | `ambre_sync::withheld::tests::a_log_served_whole_clears_the_check_and_heads_the_next_delta` -- the next delta carries the head and the entry it names; `::a_head_names_the_last_entry_served_and_an_empty_log_none` -- the last entry served, valid or not, and none on an empty log; `ambre_sync::pipeline::tests::a_stripped_log_entry_breaks_the_signature` -- an entry stripped or swapped in transit fails the signature; `ambre_protocol::tests::a_log_entry_costs_an_update_at_most_250_bytes` -- the entry on the wire, against its budget row | none | verified |
| `SYNC-219` | `FAIL` | Given a head whose update or presence sample carries no entry for it, or an entry no member allowed by this client's log signed, the client MUST NOT read the log again for it, nor warn the user. | this spec section 3; `SYNC-207`, `SYNC-263` | `ambre_sync::withheld::tests::a_head_whose_entry_no_allowed_member_signed_asks_nothing` -- no entry, an editor's revoke of the owner, the owner named under another key, an entry of another head or document, and a self-join each ask nothing, and the owner's entry asks once; `ambre_sync::withheld::tests::a_peer_detects_a_revoke_the_relay_hid_from_it` -- the owner's revoke, proved, still warns; `::a_forged_presence_entry_asks_nothing` -- a sample with no entry, with an entry no allowed member signed, or rewritten in transit asks for no log, and the sample with the owner's entry asks once | none | verified |
| `SYNC-262` | `POST` | Every presence sample MUST carry, under its sender's signature, the head of the membership log its sender last read, and the last entry of that log unless the log is empty. | this spec section 3; `SYNC-206`, `SYNC-218`; `ambre-spec-presence.md` | `ambre_sync::withheld::tests::a_relay_that_drops_updates_but_forwards_presence_is_caught` -- each sample Alice sends leaves under the head of the log she last read and the entry it names; `ambre_sync::pipeline::tests::a_rewritten_presence_head_breaks_the_signature` -- a head or an entry rewritten in transit fails the signature; `ambre_protocol::tests::a_log_head_and_entry_cost_an_ephemeral_at_most_293_bytes` -- the two fields on the wire, against their budget row | none | verified |
| `SYNC-263` | `FAIL` | Given a presence sample whose signed head the log this client read does not pass through, the client MUST read the log again under the proof of `SYNC-219`, and MUST warn the user when that log still does not pass through it. | this spec section 3; `SYNC-207`, `SYNC-219` | `ambre_sync::withheld::tests::a_relay_that_drops_updates_but_forwards_presence_is_caught` -- a scripted relay drops Alice's delta and forwards her cursor, sealed under a key Bob lacks. Bob asks for the log, the log lacks the revoke, and Bob is warned once, and the next sample asks nothing more; `::a_forged_presence_entry_asks_nothing` -- a forged sample asks nothing | none | verified |
| `SYNC-220` | `FAIL` | Given a signed head this client asked about, or found withheld (`SYNC-207`), the client MUST hold each edit unsealed until a log that passes through the head is served. | this spec section 3; `SYNC-207`; `ambre-spec-key-rotation.md` | `ambre_sync::withheld::tests::a_warned_member_seals_nothing_until_the_log_passes_through_the_head` -- no edit leaves during the ask or once warned, the session stays heavy on an eviction, and each edit leaves in order once a log holds the head; `::a_failed_listing_keeps_the_hold_and_the_next_head_asks_again` -- a failed listing keeps the hold, and the next signed head asks again; `::a_head_under_check_holds_edits_until_a_log_passes_through_it`; `::a_failed_listing_of_another_check_changes_nothing` | none | verified |
| `SYNC-273` | `FAIL` | Given a served log whose valid walk does not pass through a signed head under check, the client MUST NOT clear that check, even when an entry the walk refused precedes the entry that head names. | this spec section 3; `SYNC-207`, `SYNC-220` | `ambre_sync::withheld::tests::a_served_entry_the_walk_refuses_clears_no_head_under_check` -- an entry no member signed sits before the revoke the head names, the served heads hold the head and the walk does not, and the head is found withheld with its edits held | none | verified |
| `SYNC-274` | `FAIL` | Given a document whose edits wait for a rotation owed or for a head under check, the client MUST NOT send a presence sample. | this spec section 3; `SYNC-128`, `SYNC-220`, `SYNC-224` | `ambre_sync::withheld::tests::presence_is_dropped_while_an_edit_would_wait` -- a sample leaves on a clear check, none while a head is under check, one again once the check clears, and none while a rotation is owed | none | verified |

### 1.7 A delta is bound to a writer

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-211` | `FAIL` | Given a delta or a snapshot whose signer no valid entry of the log ever granted a writing role, the client MUST refuse it before it reaches the document, whatever key opens it. A delta MUST first wait for a listing of the members asked after it arrived, and that listing MUST decide it. | this spec section 3; `SYNC-103`, `SYNC-121`, `SYNC-150` | `ambre_sync::updates::tests::a_relay_forwarding_a_readers_delta_reaches_no_document` -- a reader's delta under the current key and under a retired one, forwarded by the relay, reaches no document and raises a warning each, while the owner's and a removed writer's land, and a listing in flight is waited for; `ambre_sync::updates::tests::a_snapshot_no_writer_signed_opens_nothing` -- a reader's snapshot opens nothing and the open replays from the start, the owner's opens; `ambre_sync::updates::tests::a_newcomers_delta_waits_for_the_listing_asked_after_it` -- a newcomer's delta waits, an answer to a listing asked before it leaves it waiting, and the listing that names the newcomer lands it; `ambre_sync::membership_log::tests::a_writer_once_granted_stays_a_writer_and_a_viewer_never_is` -- a viewer and a stranger are no writers, a writer demoted then removed stays one, and a log with no anchor checks nobody | none | verified |
| `SYNC-310` | `FAIL` | Given the deltas held for a listing, or those a listing refused, past a cap of the quarantine in count or in bytes, the client MUST drop their oldest entry until they fit. | this spec section 3; `SYNC-211`, `SYNC-230`; `ambre-spec-delta-order.md`, `SYNC-276`; `ambre-spec-budgets.md` section 1 | `ambre_sync::updates::tests::a_hold_past_its_bytes_drops_the_oldest` -- three held deltas of 12 MiB pass the 32 MiB ceiling under the count cap, the oldest leaves with one warning, and the refused deltas lose their oldest the same way | none | verified |
| `SYNC-214` | `FAIL` | Given a members answer whose walked log drops a writer the held log granted, the client MUST keep the writers and the space word it held. | audit `N18`; this spec section 3; `SYNC-211` | `ambre_sync::updates::tests::a_listing_that_drops_a_writer_leaves_the_writers_of_the_open` -- a listing that leaves the log unanchored, and one the relay rooted at a log of its own, each leave the writers of the open, and the reader's delta that waited reaches no document | none | verified |
| `SYNC-300` | `FAIL` | Given a revoke another member signed, its entry MUST cap the revoked member at the highest clock of their deltas its author applied, and a delta of that member past the cap MUST reach no document. A document that applied such a delta MUST drop its state and its local copy and read the document again. | this spec section 3; `SYNC-211`, `SYNC-214` | `ambre_sync::membership_log::tests::a_revoke_caps_the_clock_of_the_writer_it_ends` -- a delta at the cap counts, one past it and a snapshot do not, and a later invite lifts the cap, which readmits under `SYNC-301`; `ambre_sync::sharing::tests::a_revoke_posts_the_entry_that_removes_the_member` -- the posted revoke caps the member at their latest applied clock; `ambre_sync::updates::tests::a_revoked_writer_writes_nothing_past_the_cap_of_its_revoke` -- a document past the cap is read again, and read again it takes the delta at the cap alone; `ambre_logic::dispatch::tests::a_document_read_again_drops_its_copy_and_opens_anew`; `ambre_sync::membership_log::tests::the_signed_bytes_and_the_signature_of_an_entry_stay_pinned` -- the cap is under the signature; `ambre_sync::membership_log::tests::a_cap_on_another_act_caps_nobody` -- a change of role that carries a cap caps and readmits nobody; `ambre_sync::light::tests::an_eviction_keeps_the_clocks_the_session_applied` -- the clocks of an evicted session reach the reopen | none | verified |
| `SYNC-301` | `FAIL` | Given a writer capped by a revoke and granted a writing role again by a later entry, a delta of that writer past the cap MUST reach a document only when it signs a head the valid walk passes through at or past that entry. A snapshot that writer signs MUST stand nowhere. | this spec section 3; `SYNC-300`, `SYNC-206` | `ambre_sync::membership_log::tests::a_re_invite_admits_only_the_writes_past_its_own_entry` -- under the cap any head counts, and past it a head of the exclusion, a head the walk never passed and a forged digest are refused, the head after the invite counts, a second exclusion stacks on the first, and the readmitted writer's snapshot stands nowhere; `ambre_sync::applied_tests::a_head_past_the_bound_merges_into_the_empty_head` -- past 64 heads of one author, a delta past the cap counts under the empty head, never under the head of the invite; `ambre_sync::updates::tests::a_re_invite_admits_no_write_of_the_exclusion` -- a peer that applied a write of the exclusion reads the document again, and read again it lands the history and the write after the invite alone | none | verified |
| `SYNC-306` | `FAIL` | Given a departure of a document, its entry MUST cap the leaver at the last clock this client wrote in it, under the leaver's signature, and a delta of the leaver past the cap MUST reach no document. A later grant readmits the leaver as `SYNC-301` states. | this spec section 3; `SYNC-300`, `SYNC-301`, `SYNC-127` | `ambre_sync::membership_log::tests::a_departure_caps_the_clock_of_its_leaver` -- a delta at the cap counts, one past it and a snapshot do not, a raised cap breaks the signature, and a later invite readmits only the writes past it; `ambre_sync::sharing::tests::leaving_posts_the_removal_alone_and_chooses_no_key` -- the posted departure signs its cap; `ambre_sync::updates::tests::a_departure_caps_the_leaver_at_the_last_clock_it_wrote` -- the cap is the clock sealed last or a higher one applied, and the departure of a document not open reads no cap from nothing (`SYNC-314` opens it first); `ambre_sync::sharing::tests::leaving_a_channel_posts_this_users_removal_on_its_meta` -- a channel departure caps the leaver in each open document of the channel (`SYNC-311`) | none | verified |
| `SYNC-311` | `FAIL` | Given a revoke or a departure on the log of a channel's `meta`, its entry MUST cap the member in each document of the channel open on its author's client, each at the member's own clock there. A delta of that member past the cap of its document MUST reach no document. | this spec section 3; `SYNC-300`, `SYNC-306`, `SYNC-229` | `ambre_sync::sharing::tests::a_channel_revoke_caps_the_member_in_each_open_document` -- the posted revoke caps Carol in the `meta`, in a segment at her clock there and in a thread at zero, and the walk binds each cap to its own document; `ambre_sync::sharing::tests::leaving_a_channel_posts_this_users_removal_on_its_meta` -- the departure caps the leaver in the `meta` and in each open document of the channel, and in no document of another log; `ambre_sync::membership_log::tests::a_channel_revoke_caps_each_document_at_its_own_clock` -- each document admits up to its own cap, a later invite readmits each past its own cap alone, and a raised cap or a cap on another act stops the walk; `ambre_protocol::membership::tests::the_caps_of_a_channel_travel_in_the_slot_of_the_cap` -- the caps travel under the signature, an entry with none keeps its bytes, and an unsorted or oversized list is refused; `ambre_sync::channels::tests::the_open_documents_of_a_channel_are_those_its_meta_vouches_for` -- the `meta`, a closed document and an open document of another channel are left out | none | verified |
| `SYNC-312` | `FAIL` | Given a revoke or a departure on the log of a channel's `meta`, the client MUST open each document of the channel the relay lists and the Logic thread does not hold, and its entry MUST cap the member in each one. A listing with no `meta` of that log, or an open that fails, MUST fail the removal before it posts. | this spec section 3; `SYNC-311`, `SYNC-229`, `SYNC-300` | `ambre_sync::census::tests::a_channel_revoke_opens_each_closed_document_and_caps_the_member_there` -- a revoke opens the closed thread and segment under the `meta`, reads Carol's clock in each, runs once both replays are done, then closes both, and the Logic thread hears none of the census opens; `ambre_sync::census::tests::a_census_open_that_fails_fails_the_removal` -- a failed open, or a document read again past a cap, fails the departure and closes every census document; `ambre_sync::census::tests::a_listing_with_no_meta_or_another_fails`; `ambre_sync::census::tests::a_census_document_the_logic_thread_opens_stays_open`; `ambre_sync::census::tests::an_evicted_document_keeps_its_light_subscription_through_a_census`; `ambre_sync::sharing::tests::a_channel_revoke_caps_the_member_in_each_open_document` -- the revoke caps each document its params hold; `ambre_sync::census::tests::an_open_outside_the_census_leaves_it_waiting` -- a failed open of another document fails no census; `::a_snapshot_held_at_the_open_ends_the_replay_of_a_census_document` -- a snapshot held at the open ends the replay, and one held for a confirmation does not; `::no_event_of_a_census_open_reaches_the_logic_thread`; `::a_revoke_on_a_meta_no_listing_names_fails_before_it_posts` | none | verified |
| `SYNC-314` | `FAIL` | Given a departure of a document the Logic thread does not hold, the client MUST open it first, and the departure MUST name the last clock this user wrote in it (`SYNC-306`). An open that fails MUST fail the departure before it posts. | this spec section 3; `SYNC-306`, `SYNC-312` | `ambre_sync::census::tests::a_departure_of_a_closed_document_opens_it_to_name_the_cap` -- the leave opens the document under the type the relay holds, claims no type for the session, names the clock this user wrote there, then closes it; `ambre_sync::updates::tests::a_departure_caps_the_leaver_at_the_last_clock_it_wrote` -- the departure itself still reads no cap from a document not open | none | verified |
| `SYNC-315` | `FAIL` | Given a removal on the log of a channel's `meta`, its entry MUST cap the member at zero in each document of the channel it names no cap for, under its author's signature. A document the relay hid from the census MUST then take no write of the member. | this spec section 3; `SYNC-312`, `SYNC-311` | `ambre_sync::membership_log::tests::a_floored_revoke_caps_every_unnamed_document_at_zero` -- a document the revoke names no cap for refuses Bob's first write, a named one keeps its own cap, a revoke with no floor caps nothing there, and a later invite readmits past the floor alone; `ambre_protocol::membership::tests::the_floor_of_a_channel_travels_in_the_slot_of_the_cap_under_the_signature` -- the floor travels with a list or none, under the signature, and on a revoke alone; `ambre_sync::sharing::tests::a_channel_revoke_caps_the_member_in_each_open_document` -- the posted revoke caps a document it never opened at zero; `ambre_sync::sharing::tests::leaving_a_channel_posts_this_users_removal_on_its_meta` -- the departure floors the channel; `ambre_sync::census::tests::a_channel_departure_opens_its_closed_meta_for_its_own_cap` -- a closed `meta` opens for the census, so the floor never refuses the leaver's own history there; `ambre_sync::updates::tests::a_start_past_a_lifted_floor_opens_under_a_head_past_the_grant` -- the floor binds a start at the open too, and a grant lifts it under a confirmed head (`SYNC-317`) | none | verified |
| `SYNC-319` | `FAIL` | Given a removal on the log of a channel's `meta` that caps more documents than one entry holds, the client MUST post the caps past the first `DOCUMENT_CAPS_MAX` in continuations after its entry, in the same request. A continuation MUST be a revoke of the same member by the same author, with no key and no cap of its own, and the walk MUST refuse any other entry as one. | this spec section 3; `SYNC-311`, `SYNC-312`; `ambre-spec-budgets.md` section 1 | `ambre_sync::membership_log::tests::a_run_of_continuations_caps_every_document_and_nothing_else_continues` -- 40 caps take three entries and cap Bob in each document, and a continuation by another author, for another member, with a key, with a cap, with no document cap, or after a revoke with no document cap stops the walk; `ambre_sync::sharing::tests::a_revoke_from_a_large_channel_caps_every_document_over_a_run_of_entries` -- the revoke posts two continuations in one request, each within the caps of one entry, the walk caps Carol in all 40 documents, and the footing the task hands back passes the whole run; `ambre_sync::census::tests::a_channel_past_the_caps_of_one_entry_opens_every_document` -- seventeen documents open for the census, and no refusal comes | none | verified |
| `SYNC-320` | `FAIL` | Given a removal on the log of a channel's `meta`, the census MUST refuse a listing that leaves out a document the sealed state of that `meta` names, and the removal MUST fail before it posts. A removal that names no such document MUST fail too. | this spec section 3; `SYNC-312`, `SYNC-315` | `ambre_sync::census::tests::a_listing_that_hides_a_named_document_fails_the_removal` -- a listing short of a named segment fails a revoke and a departure, and one that holds every named document passes; `ambre_sync::census::tests::a_channel_removal_with_no_named_documents_fails_before_it_lists` -- a departure or a revoke with no named documents, or none at all, fails and asks the relay nothing; `ambre_murmure::logic::amending_tests::the_meta_names_each_segment_and_the_amendments_no_fold_erased` -- the `meta` names each segment and the amendments a fold kept; `ambre_logic::channels::tests::a_revoke_on_a_channel_meta_names_what_the_meta_names` -- a revoke waits for the `meta` of a channel still opening, then names its segment; `ambre_logic::channels::tests::a_revoke_on_a_closed_channel_opens_it_then_closes_it_again`; `ambre_logic::channels::tests::a_revoke_whose_channel_fails_to_open_sends_nothing`; `ambre_logic::spaces::tests::a_left_channel_leaves_the_client_once_the_relay_answers` -- the departure names the segment of the `meta`; `ambre_sync::census::tests::a_revoke_on_a_meta_no_listing_names_fails_before_it_posts` -- a revoke on a `meta` no listing names fails, under a channel open here or under named documents | none | verified |
| `SYNC-321` | `FAIL` | Given a removal on the log of a channel's `meta`, the census MUST refuse a listing that leaves out a thread a segment or a live amendments document of that channel names, unless an owner's erasure covers it (`SYNC-322`), and the removal MUST fail before it posts. A document the read of those threads cannot open or admit MUST fail the removal too. | this spec section 3; `SYNC-320`, `SYNC-315`, `SYNC-322`, `MURMURE-107`, `MURMURE-484` | `ambre_murmure::logic::thread_tests::a_removal_reads_the_thread_each_segment_names` -- a segment the view loads makes the read wait, a segment not held opens for the read alone and drops, the thread its root names joins the threads once, apart from the documents of the `meta`, and a segment of another channel ends the read; `ambre_murmure::logic::folding_tests::a_removal_reads_the_thread_the_amendments_of_a_segment_record` -- a final segment not held, then its amendments, open one at a time, the thread they record joins the threads, and amendments of another segment end the read; `ambre_logic::channels::tests::a_revoke_names_the_thread_a_segment_not_held_names` -- the revoke waits for the read, the segment closes once read, and the revoke names its thread apart; `ambre_logic::channels::tests::a_revoke_whose_segment_fails_to_open_sends_nothing`; `ambre_sync::census::tests::a_listing_that_hides_a_named_document_fails_the_removal` -- the census refuses a listing short of any named document | none | verified |
| `SYNC-322` | `FAIL` | Given a removal on the log of a channel's `meta` whose listing leaves out a thread it names, the census MUST pass that thread only under an erasure of it that a present owner of that log signed for that channel. A document the `meta` names MUST take no erasure. | this spec section 3; `SYNC-321`, `SRV-347`, `MURMURE-255` | `ambre_sync::erasure::tests::a_hidden_thread_passes_only_under_an_owners_erasure` -- an owner's erasure clears its one thread, and an erasure by a key the log holds as no owner, of another channel, of another thread or under a forged signature clears none; `ambre_sync::census::tests::only_a_hidden_thread_goes_to_the_erasure_check` -- a thread the listing holds needs no erasure, and a hidden segment of the `meta` fails the removal; `ambre_sync::census::tests::an_erasure_stands_on_the_footing_of_the_meta` -- the erasure names the `meta`, its footing and this user, and none stands with no footing | none | verified |
| `SYNC-323` | `FAIL` | Given a removal whose listing leaves out a thread no erasure covers, a client the log holds as a present owner MUST sign an erasure of that thread and post it before the removal posts. Any other client MUST post nothing and fail the removal. | this spec section 3; `SYNC-322`, `SRV-347` | `ambre_sync::sharing::tests::an_owner_erases_a_hidden_thread_and_no_one_else_does` -- the owner posts an erasure that clears the thread for every census, a held erasure posts nothing, a user who is no owner posts nothing and fails, and an erasure the relay refuses fails the removal | none | verified |
| `SYNC-303` | `FAIL` | Given a document of a Murmure channel that applied a delta past the cap of a revoke, the client MUST end the whole channel, erase the local copy of each of its documents and open the channel again. | this spec section 3; `SYNC-300`, `SYNC-233` | `ambre_logic::channels::tests::a_channel_read_again_drops_each_copy_and_opens_anew` -- each document closes in Sync and leaves the store, the channel then asks the relay again, and a document no channel holds reads nothing | none | verified |
| `SYNC-304` | `FAIL` | Given an open, a snapshot that folds a delta past the cap of its writer MUST open nothing, and the open MUST replay from a start under every cap (`SYNC-307`). A reopen whose local copy applied such a delta MUST read the document again. | this spec section 3; `SYNC-300`, `RESID-010` | `ambre_sync::updates::tests::an_open_refuses_a_delta_past_the_cap_of_its_writer` -- the owner's snapshot that folds a clock past the cap opens nothing, the replay starts at the first delta and lands the clock at the cap alone, and a snapshot under the cap opens; `ambre_sync::light::tests::a_reopen_past_a_revoke_cap_reads_the_document_again` -- the clocks of an evicted session travel to its reopen, which reads the document again past a cap and not under one; `ambre_sync::updates::tests::a_start_past_a_cap_is_one_whose_snapshot_folds_a_delta_past_it` -- a start folding a delta past the cap fetches the reference, and one at the cap or one that does not decode does not | none | verified |
| `SYNC-307` | `FAIL` | Given an open whose start folds a delta past a cap, the open MUST start from the trusted reference when the reference folds none, and replay the deltas past its horizon alone; with no such reference, it MUST replay from the first delta. Either start MUST reach the clocks a full replay of the same log reaches. | this spec section 3; `SYNC-304`, `AUTHOR-030`, `SRV-127` | `ambre_sync::updates::tests::a_snapshot_past_a_cap_starts_from_the_reference_as_a_full_replay_does` -- the open starts from the reference and asks past its horizon, applies the deltas a full replay applies past it and reaches its clocks, and a reference past the cap replays from the first delta; `ambre_sync::snapshot_trust::tests::a_trusted_latest_keeps_the_trusted_reference_to_fall_back_on` -- only a trusted reference older than the start is a fallback | none | verified |
| `SYNC-309` | `FAIL` | Given an open, the replay the relay serves MUST hand the document no delta past the cap of its writer, and MUST land every delta under the caps. | this spec section 3; `SYNC-300`, `SYNC-304` | `ambre_sync::updates::tests::an_open_replay_refuses_a_delta_past_the_cap_of_its_writer` -- a fresh open replays the whole log in one batch, which carries the capped writer's clock at the cap and the other writer's delta alone, warns once per refused delta and leaves no write past the cap applied | none | verified |
| `SYNC-316` | `FAIL` | Given an open, a start that folds a clock of a writer past any cap a later grant lifted MUST open nothing, as `SYNC-304` states, unless `SYNC-317` admits it; every lifted cap of that writer counts, the latest one alone does not. | this spec section 3; `SYNC-301`, `SYNC-304`, `SYNC-307` | `ambre_sync::updates::tests::a_start_past_a_lifted_cap_opens_nothing` -- after a revoke at cap 1 and an invite, a start that folds Bob's clock 2 or 3 opens nothing and replays from the first delta, one that folds his clock 1 opens, and a copy that folded his clock 2 is read again; `ambre_sync::membership_log::tests::a_fold_past_any_lifted_cap_of_its_author_is_past_a_cap` -- after two exclusions each lifted, a fold past the first cap counts as past a cap, and Alice is bound by none | none | verified |
| `SYNC-317` | `FAIL` | Given an open, a start that folds a clock of a writer past a cap a later grant lifted MUST open when a member other than its author confirmed it under a log head the valid walk passes through at or past that grant. A standing cap MUST still refuse it. | this spec section 3; `SYNC-301`, `SYNC-316`; `ambre-spec-op-authorship.md`, `AUTHOR-030`, `AUTHOR-038` | `ambre_sync::updates::tests::a_start_past_a_lifted_cap_opens_under_a_head_past_the_grant` -- after a revoke at cap 1 and an invite, a start folding Bob's clock 3 at horizon 9 opens under the head after the invite and replays past its horizon alone, as after a trim, and is never read again, while the head of the exclusion or no head opens nothing; `ambre_sync::membership_log::tests::a_confirmed_head_past_the_grant_proves_a_fold_past_its_lifted_cap` -- the head after the invite proves the fold, the head of the exclusion, a forged head and no head do not, and a standing cap refuses it under any head; `ambre_sync::snapshot_trust::tests::a_confirmation_holds_under_the_head_it_signs_alone` -- the start carries the signed head as its proof, and a head the relay rewrote or stripped trusts nothing; `ambre_sync::updates::tests::a_start_past_a_lifted_floor_opens_under_a_head_past_the_grant` -- a thread the revoke named no cap for takes the floor of zero (`SYNC-315`), and a start folding Bob's clock 1 there opens under the head after the invite alone | none | verified |

* **Budget.** the key exchange costs one elliptic-curve operation per member at open, charged to
  the crypto rows of `ambre-spec-budgets.md section 1`. The directory and the role are read off an
  answer the open already fetches, so both are free. The log adds one check per entry.
* **Boundary.** the wrapped key crosses the relay and the relay can open none of them: `SYNC-010`
  seals each copy for one exchange key. What it learns is who is a member of what, which is the
  membership metadata `ambre-spec-zero-knowledge.md` accepts.
* **Verification.** 121 `verified`. Every pure transform holds, out of the browser: the wrap, the
  weak key, the pin, the invitee, the role and the log walk. What is loose is the branches that only
  run against a live relay.
---

## 2. WHAT EACH USER SEES

Three peers, because sharing needs a third. The last column joins the example to its verification.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| invites Bob after confirming his key | opens the document | Alice's content | Bob reads it: the key was wrapped for him alone (`SYNC-010`) | `sharing::tests::invite_success_posts_wrapped_key_and_emits_invitesent` |
| invites Bob, and the relay serves a different key at the wrap | -- | nothing new | Alice is told the invite is refused, and nothing is wrapped (`SYNC-036`) | `sharing::tests::invite_pin_mismatch_refuses_and_never_wraps` |
| invites Bob as a reader | tries to edit | Alice's content | Bob sees no editing affordance (`SYNC-016`) | `sharing::tests::own_role_reads_our_own_row` |
| invites Bob, whose row the relay serves malformed | opens the document | Alice's content | Bob's cursor never appears on Alice's screen, rather than appearing as someone else (`SYNC-015`) | `sharing::tests::presence_directory_skips_unparsable_rows_without_inventing_ids` |
| moves her cursor | watches on two devices | nothing | one Alice, in one colour (`SYNC-014`, `PRESENCE-005`) | `ephemeral_tests::author_resolves_to_the_member_behind_the_signing_key` |
| opens the document while the network drops the members answer | edits | Bob's edit | Alice is told the relay could not be reached, and no empty document replaces hers (`SYNC-040`) | `updates::tests::a_failed_members_request_is_not_an_absent_document` |

---

## 3. MODEL

**CRDT containers.** none. Membership is the relay's table, not the document's content. Putting it
in the document would make every member's identity part of the encrypted state, unable to change
without a delta.

**One answer, three uses.** The members request is issued once at open, again when the relay
acknowledges a subscription, and again whenever membership changes. The relay's word of a change
reaches only a subscribed client, so the acknowledgement closes the gap the open leaves
(`SYNC-150`). Sync hands the rows of a refresh to the view too, so one request serves both
(`SYNC-151`). Every row of it carries the same four fields, and each is read by a different
rule.

| Field of a member row | Answers | Rule |
|---|---|---|
| the wrapped document key | which key opens this document, for this client | `SYNC-012`, `SYNC-034` |
| the wrapper's exchange key | whose key to run the exchange against | `SYNC-034` |
| the signing key | which member authored an ephemeral | `SYNC-014`, `SYNC-015` |
| the role | what this client may do, and what to show for each member | `SYNC-016`, `SYNC-017` |

**The wrap.** The document key is sealed under a key derived from an X25519 exchange between the
wrapper's private key and the recipient's public one. The shared secret is symmetric, so the
recipient must run the exchange against the **wrapper's** public key, never against its own
(`SYNC-034`). Who wrapped is recorded by the relay, per member row: the inviter for a shared
member, the member themselves for the owner's copy at creation.

A copy opens with a version byte. Its additional data is the wrap domain, that byte and the
document whose rows carry it (`SYNC-289`). That document is the `meta` of a document of a channel,
whose rows the relay serves (`SRV-272`), or else the document itself. The relay can thus serve no
genuine copy of one document as a copy of another. MVP has not shipped, so no reader accepts a
copy without the byte. The key the copy holds is still the relay's to choose, which the checks of
an open answer.

**Why the own key looks like it works.** A document with one member is a self-wrap, so the exchange
degenerates and a client using its own key derives the correct secret. The defect only appears on
the first shared document, which is why the rule is stated rather than left to the code.

**An invite, in two phases.** The trust step between them is
`ambre-spec-key-verification.md`; this document owns the wrap.

1. resolve the invitee, fetch their exchange key and compute its fingerprint. **Nothing is
   wrapped**;
2. the user confirms the fingerprint out of band;
3. re-resolve the invitee. Refuse the invite when the key served now differs from the confirmed one
   (`SYNC-036`), or the email now names another user (`SYNC-161`). A direct message may already
   have named that user in its notice of a third member;
4. wrap the document key for that key, and refuse a key with no contribution (`SYNC-035`);
5. post the wrapped copy. The relay pushes a notice to the invitee, who is not yet subscribed to
   the document.

A new document lives on the client alone until its first edit. The relay then holds no member log
to append the entry to. So phase 2 on such a document waits, and starts its creation
(`SYNC-246`). A creation that fails, or a close that comes first, refuses the invite (`SYNC-247`).

**The directory is membership, checked against the log.** It answers "which member wrote this
envelope". The relay serves the rows, and an anchored log binds each signing key to one member.
A row whose key the log binds to nobody, or to another member, is left out (`SYNC-215`). So the
relay omits a member at worst, and never credits one member's delta to another. Detecting a relay
that substitutes a member's key at the invite stays the trusted-key store's job
(`ambre-spec-key-verification.md`), and presence does not wait on it.

**A row that does not parse is handled two ways, deliberately.**

| Where | A bad row becomes | Why |
|---|---|---|
| the directory | skipped | a cursor under the wrong name is worse than no cursor (`SYNC-015`) |
| our own role, absent or not a document role | the read-only role | hiding an affordance the user has beats offering one the relay refuses (`SYNC-017`) |
| the identifier or the role word, anywhere | the whole answer, refused when read | each is parsed once, at the edge, so no row reaches the list with an invented identity or a guessed role (`SYNC-038`, `SRV-179`, `SRV-180`) |

**The signed membership log.** The relay stays the authority on access, and the log lets each
client check what the relay serves.

- An invite, a revoke and a role change each add one entry, signed by the member who acts
  (`SYNC-132`).
- An entry names the document, the member, the act, the role and the digest of the entry before it
  (`SYNC-102`). The first entry is the creation, signed by the creator.
- An entry names the member's signing key and exchange key (`SYNC-197`). The entry that adds a
  member fixes both. A rotation then wraps for the exchange key the log signs, never for one the
  relay alone serves (`SYNC-198`). An invite signs the key the user confirmed.
- A member keeps both keys for the life of the account. Both derive from the Master Seed
  (`ZK-009`), and the relay writes them once (`SRV-030`). A new device and a recovery unlock the
  same seed, so no honest cause changes either key. A new seed needs a new account.
- So no entry replaces the keys of a present member, an owner's included (`SYNC-217`). A re-invite
  of a present member names the keys the log holds. The relay refuses an entry under any key but
  the registered one (`SRV-311`).
- A served key that differs from the one the log holds for a present member is a relay fault. The
  invite refuses it and posts nothing (`SYNC-324`). Signing it would get the next rotation wrapped
  for a key of the relay's choice.
- A revoke keeps the pair the log last bound to the member. An entry that adds that member again
  names the same pair, whoever signs it (`SYNC-325`). The invite of a revoked member refuses any
  other served pair, as for a present member (`SYNC-326`). So the trust on first use of the first
  invite covers every later invite too.
- A new member learns the first entry from its inviter, inside the sealed wrap (`SYNC-104`). So a
  relay cannot hand a new member a log rooted at a creator of its own, unless it wraps the copy
  itself (section 6).
- A client walks the chain from the first entry. A row it cannot link is shown as unverified
  (`SYNC-103`, `SYNC-108`). A revoke is an entry too, so an old invite replayed after it changes
  nothing (`SYNC-109`).
- An entry the member signs for themselves is valid only under a rule of the owning application
  (`SYNC-106`). Murmure accepts a self-join to a public channel of a space, checked against the
  sealed catalogue of that space. The member must be present in the log of that space, under the
  signing key and the exchange key the self-join names (`SYNC-227`). A client that holds no walk of
  the space log counts no self-join.

```rust
pub struct MembershipEntry {
    pub document: Id<Document>,
    pub position: Seq<LogPlaces>,       // the place in the log, from zero
    pub member: Id<User>,
    pub member_key: PublicKey,          // the member's signing key
    pub member_exchange_key: ExchangePublicKey, // the key a rotation wraps for
    pub act: MembershipAct,
    pub role: Role,
    pub key: Option<KeyId>,             // the key an invite or a self-join wraps
    pub previous: Option<EntryDigest>,  // absent on the first entry alone
    pub author: Id<User>,
    pub signature: Signature,
}
pub enum MembershipAct { Create, Invite, Revoke, ChangeRole, SelfJoin }
pub struct EntryDigest([u8; 32]);
```

What the relay can still do is withhold the latest entries. It can show a list that is late, never
a list it made up. A revoke also rotates the key, so a revoked member reads nothing new either way.

**A withheld entry, seen by a peer.** A relay can hide a revoke and its rotation from one member.
It then takes that member's deltas under the old key and forwards them to the member it removed.
Two rules make the gap visible to the member it is hidden from.

- Each update carries the head of the log its author last read (`SYNC-206`). The head is the
  place of the next entry and the digest of the last one. The head is public metadata under the
  signature, so the relay can drop it and never rewrite it.
- Each update also carries the last entry of that log, which the head names (`SYNC-218`). The
  relay stores that entry already, so the entry tells it nothing new.
- Each presence sample carries the same head and the same entry (`SYNC-262`). Presence flows
  while nobody writes, so a relay that drops every update still lets a head through. The sample
  pays about 264 bytes for it (`ambre-spec-budgets.md section 1`).
- A client reads the head of each update and each sample a member of its directory signed
  (`SYNC-263`). The signature is checked alone, so a frame under a key this client does not hold
  still counts. That frame is the one a hidden rotation brings.
- A head the log this client read does not pass through asks for the log once more. A log that
  still does not pass through it raises a warning (`SYNC-207`). An answer to a read that left
  before the head proves nothing.
- A head counts only when its entry proves it, on an update and on a sample alike (`SYNC-219`).
  The entry sits at the place before the head and hashes to its digest. Its author signed it,
  under an act the log this client read allows that author. A member thus cannot invent a head
  and have the relay blamed for it.
- A self-join proves nothing. Any user may sign one, so a self-join could blame the relay for an
  entry its author never sent.
- From the ask on, each edit of this client waits unsealed (`SYNC-220`). A relay that never
  answers the read would otherwise keep the old key in use. A failed read keeps the edits, and the
  next signed head asks again. The edits leave in order once a log that passes through the head is
  served, by any later read. Then an owed rotation or a new key still applies to them.
- A presence sample is not worth a hold. An edit may wait for a head under check or for an owed
  rotation. The client then sends no sample at all, since the member who left holds the current
  key (`SYNC-274`).

The detection needs one frame of an up-to-date member to reach the member. A relay that hides
every such frame partitions the member, and nothing in the document tells it apart from silence.

**A member writing behind the log, seen by a peer.** The member who read the removal sees the same
gap from the other side (`SYNC-298`).

- A peer's delta opens under a key other than the current one, from the cache or the chain.
- The relay numbered it after a delta under the current key, this client's own echo included. So
  the delta did not leave before the rotation: history and a delta in flight carry lower numbers.
- Its signed head is one the valid walk passes through, short of the head this client read. The
  removal that the rotation answered is past it.

The client then warns the user, once per member. Either the relay withholds the removal from that
member, or the member has not read it yet: the two read alike from here. The member who left may
read that delta. The relay does not refuse a retired key once the debt is paid (`SRV-123`), so an
honest relay also lets a short lag through. A delta held in quarantine and released later is not
read for it.

**How an entry is appended.** The relay checks the signature against the author's registered key.
It checks that the entry names the two keys its member registered (`SRV-104`, `SRV-311`).
It appends at the entry's position alone, under the document lock. Two writers racing for one
position cannot fork the log: the loser is answered `LOG_MOVED`, and signs again after the winner.

**What the open checks of the key.** The relay names the wrapper of each copy. It can thus wrap a
key of its own under an exchange key of its own, and the unwrap succeeds. Before the key becomes
current, at an open and at a rotation, the client checks it against the log:

1. the wrapper is this client, or a member the log verifies (`SYNC-201`). The relay cannot build a
   copy this client wrapped for itself: the exchange needs the secret half on both sides. The
   wrapper's key counts only when a valid entry signs it for that member (`SYNC-197`). The relay
   serves a row's exchange key under no signature, so a row never vouches for it;
2. a writer the log verifies named the key (`SYNC-200`). The creation, an invite and a revoke name
   the key they install or wrap; a rotation names its key in the link it signs. A self-join names
   no key a writer chose;
3. no link a writer signed starts from a newer key and seals this one (`SYNC-202`);
4. a key that only a link names counts when no writer named another key at or past the head the
   link signs (`SYNC-221`). A revoked writer can sign a head before its revoke, and the revoke
   names the key it installs;
5. a key in force before the latest departure counts only while the members answer owes the
   rotation (`SYNC-222`). A departure names no key, so the fourth check misses a link the leaver
   signed before it left. The leaver holds every key in force before the departure, and a writer
   pays an owed debt before it seals;
6. a key counts only when it is the key of the latest revoke, or links chain it back to that key
   (`SYNC-225`). A revoke installs its key in the request that removes the
   member, so every later key chains back to it. The revoked member holds every key before it;
7. the links a writer signed walk back from the key to the latest key a valid entry names
   (`SYNC-226`). A relay can serve a genuine copy from before a revoke and withhold the link past
   it. That copy fails this check, at a first open too, and so does a key behind an entry that is
   no revoke.

The seven checks read a log that the anchor of the copy roots. The relay can wrap a copy itself.
It then chooses that anchor too, and serves a whole log rooted at a creator of its own. The invite
link closes that for the open it drives:

* the link carries the pin of the inviter's signing key in its fragment (`SYNC-208`). The pin is a
  BLAKE2b digest under a tag of its own. A browser sends no fragment in a request, so the relay
  never reads it there;
* the share dialog shows the link, and its button copies the link inside the click (`SYNC-245`).
  A Murmure page shows the link of an invite into a space in its toolbar. A browser such as
  Firefox refuses a clipboard write outside a user gesture, and the invite lands after the gesture
  that confirmed it;
* the shell hands the pin to the Sync worker before the open (`SYNC-209`);
* the open refuses the key unless a valid entry of the log is signed by the key the pin names
  (`SYNC-205`). The relay cannot sign such an entry, and each entry names the digest before it.
  So the anchor is the one the inviter's log starts from, and the seven checks read that log.

The worker keeps the pin for the session, so a reopen and a rotation check the same log.

**A write stands on what the open accepted.** A settle, a revoke, an invite and a role change
read the members again. The anchor that answer seals may be one the relay chose. So each of them
walks the served log from the anchor the open accepted, never from the anchor of the fresh answer
(`SYNC-266`).

The worker keeps that anchor per document, with the latest head this client walked or signed
from it. A listing that passes through that head moves it on, and so does each entry
this client signs. A served log that does not pass through it is cut or rooted elsewhere, and the
write sends nothing (`SYNC-267`). The type of the walked log holds the rule: a write takes no
other log.

**A key binds one member.** The log may bind a signing key to one member. An entry that binds it
to another stops the walk, as any entry that fails a check does (`SYNC-282`). The inviter refuses such
an invite before it signs: the entry would stop every walk after it.

**A space wrapper is a writer the space holds now.** Under a space, a copy may come from a wrapper
no row carries. It counts only when the space log holds that wrapper as a present writer at its
head (`SYNC-283`). A member the space removed wraps no older key again.

**A rename stands on the checks of an open.** A rename of a document not open reads the members.
It runs the checks of an open on the key, from the footing the session holds (`SYNC-284`).

**A read stands on it too.** A fetched rotation, a reopen in the same session and a members
refresh each read the members again. Each walks the served log from the footing the session holds,
never from the anchor of the fresh answer. A log that does not pass through its head installs no
key, opens nothing and lands no directory, writers, space word or head (`SYNC-272`).

A head under check clears only against a head the valid walk passes through. The relay can serve
an entry the walk refuses, then the entry the head names. The served heads then hold the head, and
the check still does not clear (`SYNC-273`).

The worker also keeps the latest footing per log, so the documents of one channel share it. It
outlives the close of a document, and lasts until sign-out. A space admin re-keys a public channel
from the footing of its `meta` (`SYNC-268`).

A re-key with no footing of that `meta` walks from the anchor its catalogue entry seals, as a first
open does. Refusing that re-key would leave the key of a space leaver in force. An open first would
read the same answer of the relay, so it adds no head to check. The head that first walk reaches
then holds every later write of the session.

**The pin outlives the session.** The first open whose log proves the pin of a link keeps it
(`SYNC-238`):

1. the Sync worker reports the pin to the Logic thread, which hands it to the local store;
2. the store keeps it among the trust pins, in a row keyed by the signed-in user and the document
   (`SYNC-244`). The row stays in the clear like the other bookkeeping rows
   (`ambre-spec-residency.md` section 3);
3. at the next page, the Main thread reads back the kept pins of the signed-in user. It routes
   no open until the store answers (`SYNC-240`). A store that never opens answers with
   none, and a row an older build keyed by the document alone reads back for nobody;
4. the Main thread hands the kept pin to the Sync worker ahead of each open of its document, as
   it hands a link's. The open then checks the log against it.

The pins of a user are known once a session names that user, so the open of the page's document
follows the session. Its local copy comes back once Logic adopted the document's session
identity. The copy then lands under that identity and takes no rebuild (`AUTHOR-027`).

The row never leaves the device, and it names a document the relay already knows. A pin the
relay could choose is worth nothing, so the store keeps only a pin the log proved.

**The chain outlives the session.** The local store keeps the links the latest open or rotation
held, in the row of the kept log, under the same account (`SYNC-308`). Each link was signed by a
writer of the log when the open kept it (`SYNC-121`). A reload rebuilds the chain from the row, so a
listed name sealed before the last rotation opens with no request.

**The row stays bounded.** Each rotation adds a link, and the store reads every row at each
session. So the row keeps only the links the walk back from the kept key reaches, nearest first.
It keeps no more than the bound of `ambre-spec-budgets.md` section 6 (`SYNC-313`). A link the walk
never reaches opens no key, as a link before a history boundary.

After a reload, a listing reaches no key past the bound. Its name shows none until an open fetches
the links again. The kept part is a prefix of the walk, so the reload checks it as a whole chain
and trusts nothing more.

The worker refuses a stored chain the kept log no longer extends. One such link names another
document. Another was signed at a head past the kept head. A third is one the kept key does not
walk back to. It refuses the chain of a log this session walked from another anchor too. A refused chain
leaves the kept footing in force, and the next open fetches the links again.

The row changed form when it took the chain. A row of the older form, or one with bytes past the
kept log, reads as no row. Ambre is not released yet, so no migration reads it: the store drops it,
and the next open walks and fetches again. Such a row is never trusted.

**A link that names another inviter.** A kept pin is never replaced (`SYNC-239`). On a document
that keeps a pin, the Main thread hands the kept pin to the open in place of the link's. The kept
pin already proves the anchor, so the link of a second inviter adds no proof. A link the relay or
a stranger crafted thus costs nothing, and never moves the anchor the open checks.

An open with no link and no kept pin fails closed (`SYNC-210`). It passes only on a copy this
client wrapped itself, or on a log with a valid entry its own signing key signed. A creator signed
the creation. The relay can forge neither. Any other open, from the document list or a share
notice, is refused until the user opens the invite link once on this device.

**A document of a channel.** A segment, an amendments document and a thread serve the rows, the
links and the log of the channel's `meta` (`SRV-272`). The links name the `meta`, and so do the
entries. The Logic thread reads the `meta` of each document from the sealed state of the channel,
and the open names it. The worker then walks the meta's log and keeps the meta's links in the chain
of the document. The three checks and the writers read that log (`SYNC-229`).

**A channel opened before its space.** The Logic thread names the space of a public channel from
the catalogue it holds. A channel can open first, so its log walks under the rule of a document,
and a member who joined alone is no writer. Once the catalogue lists the channel, the Logic thread
marks it, and the worker walks the log again under the rule of the space. A new word of the space
walks it again too.

The worker keeps each delta of a channel document that a walk refused as no
writer's, sealed, and the next walk decides it (`SYNC-230`). Until that walk, the refusal stands. A
log whose creation no writer of the space signed grants no writer then.

That walk also fails the open of the document as `RootOutsideSpace`. An earlier walk, run before
this client held the word of the space, accepted the key and the deltas of the log. No member of
the space vouches for either. The Logic thread ends the whole channel, and the view drops it as a
channel that failed to open. A first open under the word of the space fails the same way
(`SYNC-231`).

From the mark to the answer of that walk, the document holds what crosses its key. A writer of the
first walk may be the relay's, and the key that walk accepted too. Each delta waits for the answer,
and each edit of this client waits unsealed. A snapshot and a presence sample are not sealed. A
walk that roots the log in the space lands the deltas and seals the edits, in order. A walk that
finds it rooted outside drops both, and the document seals nothing again (`SYNC-232`).

Every seal under the key of such a document reads the key through one door, which refuses it while
the log walks. That covers an update sent again after a refusal, a media blob, a name, a new segment,
an invitation, a revoke and a rotation owed. A frame the relay refused waits with the edits, at its
own clock. A snapshot fetched during the walk is not adopted: the worker fetches it again after the
answer.

The walk holds at most the bound of the quarantine. Past it, the worker drops every edit it
held and reports `EditRefused`. The document then sends nothing more until it reopens, as after a
refused delta (`SYNC-234`).

The local store can hold a copy of such a channel, written when it closed before its space opened.
No open shows that copy before a walk: a reopen reads it into the opening, and the view waits for
the open. A walk can still find the log rooted outside. The Logic thread then erases the copy of
each document of the channel, so no later open finds it (`SYNC-233`).

**A Murmure open.** A Murmure page binds no document, so no Murmure open names a pinned inviter of
its own. The invite of a space or of a channel still copies a link: the Murmure page, with the pin
of the inviter in its fragment. The Sync worker keeps roots for the session instead:

* the inviter the link of the Murmure page pins (`SYNC-236`);
* each member a Murmure log keeps, once an open accepts that log (`SYNC-237`);
* each invitee this client signs into a Murmure log (`SYNC-237`).

A Murmure open refuses the key unless one of three holds (`SYNC-235`). A root signed a valid entry
of its log, this client's own signing key signed one, or this client wrapped its own copy. The
relay can forge none of the three. The set only grows for the session. A Prose, Trame or Toile open
reads no root of the session (`SYNC-205`, `SYNC-210`).

**A Murmure root outlives the session.** It rides the kept pin of a document, with no store of its
own:

1. an open that a root admits hands that root to the local store, as the kept pin of its document
   (`SYNC-242`). An open that a kept pin admits hands that pin again, and the store keeps the first;
2. a Murmure page names no document, so the Main thread cannot tell which opens follow. Its boot
   waits for the store, and every kept pin of the signed-in user goes to the Sync worker ahead
   of it (`SYNC-243`, `SYNC-244`);
3. a later Murmure open of that document checks its log against the kept pin, as a Prose open does
   (`SYNC-241`). The kept pin wins over the roots of the session.

The relay chooses no kept pin: the store keeps only a root that signed a valid entry of the log.

The creator of a space signed its creation, so the creator opens it with no link. An invitee opens
the space from the link, and its members then root each channel and each direct message one of
them created.

**Why a Murmure label binds nothing else.** The key checks skip a document of a Murmure channel,
as section 6 states. The open learns that type from the relay: the channel listing names each
channel, and the document listing names each type. So the relay can label any document Murmure.

An open under another type comes from the route of a view, and claims the document for the session
(`SYNC-260`). A Murmure open of a claimed document installs nothing, and the claim drops a key such
an open installed first. The unchecked key thus seals only what the Murmure logic writes.

A refused key fails the open as a key not received (`SYNC-173`), and a refused rotation keeps the
current key. Two refusals mean that no member signed the key: no writer named it, or the pinned
inviter signed no entry of the log. The relay chose that key, so the open fails as a key no member
signed (`SYNC-212`), and the view names a security fault. The other refusals stay a key not
received, since an honest relay can cause each one. The cause stays a typed refusal inside the
worker. A failed fetch of the links fails the open as transient, since a later open may clear it.

**Why a delta needs a writer.** Each member holds the document key, a reader too, and the chain
reaches every key the member ever held. A key opens a delta, so the key alone says nothing of the
right to write. The relay refuses a reader's delta, and a relay that forwards one is the case this
check exists for. Before a delta or a snapshot reaches the document, the client checks its signer
against the writers of the log (`SYNC-211`).

The writers are every key a valid entry granted a writing role, at any point. That set only grows,
so two peers that hold the same log give one delta the same verdict (`AUTHOR-007`). A peer that
drops a writer at a demotion would refuse history another peer already applied, and the two would
diverge.

A newcomer's first delta can arrive before this client lists the members again, after a reconnect
above all. A delta from a signer the log does not name waits, sealed, for a listing asked after it
arrived (`SYNC-211`). One listing is in flight at a time, and its answer asks again for what it
left. The held deltas share the two caps of the quarantine, its count and its bytes, and so do
the deltas a listing refused (`SYNC-310`).

**A document older than the log.** Its log is empty, so the relay takes a change with no entry.
It has no first entry, so none of its rows verifies, and the list shows each as unverified. A
document whose log holds an entry refuses a change without one. Its copy carries no anchor, so no
writer names its key and the open refuses it (`SYNC-200`). A relay could otherwise strip the anchor
of any copy to skip the check.

A rotation of such a document wraps the new key for the rotator alone, and reports every other
member (`SYNC-198`).

**A snapshot past a cap starts from the reference.** A start that folds a delta past a cap opens
nothing (`SYNC-304`). The open task reads the clocks the start folds, and fetches the trusted
reference beside it. A reference that folds no delta past a cap starts the open, and the replay
asks for the deltas past its horizon alone (`SYNC-307`). The relay keeps every delta past the
reference (`SRV-127`), so the replay costs those deltas and no more. A replay from the first delta
remains the last resort, and the relay may have trimmed the deltas under the reference.

**A snapshot proves no head.** A snapshot names the highest clock of each author it folds, and no
head. A later grant lifts the cap of a readmitted writer, but a clock past that cap may be a write
of the exclusion. A start that folds such a clock thus opens nothing, and the deltas replay under
their heads (`SYNC-316`). Each cap a grant lifted counts, not the latest one alone. A signed list of
heads would prove nothing: only the signature of each delta binds its head.

**A confirmation proves the head.** Once a readmitted writer writes past the lifted cap, every later
snapshot folds past it, the reference included. The relay trims the deltas under the reference, so a
replay from the first delta can lose them.

A member other than the author who confirms a snapshot
checked it against its own state (`AUTHOR-016`). That state took each delta of the writer under its
head. The confirmer thus signs the head of its walk (`AUTHOR-038`). A start whose confirmation names
a head at or past the grant opens, as a delta signed under that head would (`SYNC-317`).

A confirmation with no head, or a head short of the grant, leaves `SYNC-316` in force.

The open notes each folded clock under that head. A later walk reads them as signed deltas, so the
document is not read again. A standing cap still refuses the start, whatever the head. The price is
the one `AUTHOR-030` states: an owner and a guest of that owner can make such a fold read as checked.

**A replay checks each delta.** The open checks each replayed delta against the cap of its writer,
as it checks a live delta. A batch thus hands the document no write past a cap (`SYNC-309`).

**A departure names its cap.** A leaver signs the last clock it wrote, the clock it sealed last or
a higher one an echo or a snapshot held (`SYNC-306`). Only an open document knows that clock, so a
leave of a document not open opens it first, as a census (`SYNC-314`). A cap read from nothing would
refuse the leaver's own history at every peer.
The walk caps the leaver as it caps a revoked member, and a later grant
readmits it the same way (`SYNC-301`).

**A channel caps each document on its own.** Each document of a channel numbers the clocks of an
author on its own, and the `meta` log vouches for all of them. A revoke or a departure on that log
thus names one cap per document of the channel open on its author's client (`SYNC-311`). The `meta`
takes the cap of the entry, and each other document takes its own. The open of a document of the
channel reads the caps the walk set on that document alone. A document the entry names no cap for
takes zero (`SYNC-315`).

**A census opens what a removal caps.** A removal on the log of a channel's `meta` lists the
documents of the channel first. Each document the Logic thread does not hold opens for a census
(`SYNC-312`). A census open runs the same walk, keys, caps and replay as any open. What the open
would tell the Logic thread reaches the census alone, and no type claims the document
(`SYNC-260`). Once each replay is done, the removal reads the clocks of every open document, and the
census closes the documents it opened.

A departure of a document not open takes the same path, one document long (`SYNC-314`). An evicted
document opens from the relay, and gets its light subscription back at the end.

An open that fails fails the removal. A removal that left one document uncapped would let the member
write there with the relay, so it posts nothing.

**A removal names what the `meta` names.** Only the Logic thread reads the sealed `meta`. Before a
departure or a revoke on the log of a channel's `meta`, it opens the channel when it is closed. It
then sends the documents the `meta` names with the removal: each segment, and each amendments
document no fold erased. The census refuses a listing that leaves one out, and a removal that names
none (`SYNC-320`).

A channel the Logic thread opened for the removal alone closes again. An open that
fails fails the removal, and nothing reaches the Sync worker.

**A removal names the threads too.** The `meta` names no thread. A root names its thread in its
`thread` field (`MURMURE-107`). The amendments of a locked segment record the thread of a late
first reply (`MURMURE-484`).

Before the removal leaves, the Logic thread reads every segment the
`meta` lists and every amendments document no fold erased. A document it does not hold opens for
that read alone, one at a time, and closes once read. The removal then names each thread with the
documents of the `meta`, and the census refuses a listing that leaves one out (`SYNC-321`).

A document the read cannot open or admit fails the removal. A thread it never read would take the
floor of zero, so the removal posts nothing.

**An owner erases a thread the relay never linked.** Any member sets the absent `thread` field of a
root (`MURMURE-255`), and the relay cannot read the field. A field that names no document of the
channel would fail each removal. The Logic thread thus sends the threads apart from the documents
of the `meta`. The census looks up each thread the listing leaves out in the erasures of the
channel.

An erasure counts when a present owner of the log of the `meta` signed it for that channel
and that thread (`SYNC-322`). The census reads the owners from the log it walks from the footing of
the open, never from the relay.

A thread no erasure covers fails the removal, unless this client is an owner. An owner signs an
erasure of each such thread and posts it, then the removal goes on (`SYNC-323`). The relay refuses
an erasure of a document the channel groups (`SRV-347`). So a relay that hides a linked thread
still fails the removal. A document the `meta` names takes no erasure.

**A removal floors the channel at zero.** The census reads the documents of the channel from the
relay's listing. A relay that hides a document leaves no clock to name for it. The entry thus caps
the member at zero in each document of its log it names no cap for (`SYNC-315`). A hidden document
loses the member's history, and takes no write of theirs.

A log no other document walks loses nothing by the floor, so every removal carries it. A closed
`meta` opens for the census too, so the floor never refuses the leaver's own history there.

**A large channel spreads its caps.** One entry carries at most `DOCUMENT_CAPS_MAX` caps, since every
update carries the last entry. The caps past that bound go into continuations after the entry, in
the same request (`SYNC-319`). A continuation revokes the same member again, by the same author, with
no key and no cap of its own. The walk takes it only right after a revoke with document caps.

The caps ride in the slot of the cap of a revoke. An entry with no document cap keeps the bytes it
had before them, so no stored entry, digest or signature changes.

**An entry of an older version.** Version 2 of the entry signs the exchange key, and version 3
the cap of a revoke (`SYNC-300`), each under a new signing domain. No migration rewrites a stored
entry of an older version. Its signature checks under no key, so the walk stops at it, and the rows
past it read as unverified.

**A document the relay does not know is this client's own.** A missing document on open means it
exists on this client alone and has not been created on the relay yet. The local user is then the
only party, and holds the owning role. That inference is right for an absence and wrong for a failure,
which is `SYNC-040`. It is wrong for a document an invite link pinned too. Its inviter signed a log
on the relay, so an absence there is the relay's word alone, and the open fails (`SYNC-279`).

**Code.**

* `crates/ambre-sync/src/sharing.rs` -- the wrap, the directory, the role, the invite and the list;
* `crates/ambre-sync/src/members.rs` -- the handlers that gather the state and spawn the requests;
* `crates/ambre-sync/src/updates.rs` -- the unwrap at open, and the wrapper-key choice;
* `crates/ambre-sync/src/membership_log.rs` -- the walk of the log, and the next entry to sign;
* `crates/ambre-sync/src/key_vouch.rs` -- the seven checks of the key before it becomes current;
* `crates/ambre-protocol/src/membership.rs` -- the entry, its signed bytes and its digest;
* `crates/ambre-crypto/src/ecdh.rs` -- the exchange, the derivation and the seal of the key itself.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| a share entry in a document's menu | phase 1: the invitee is resolved and their fingerprint shown |
| confirming the fingerprint | phase 2: the key is wrapped and posted (`SYNC-036`) |
| opening a member list | the list is refetched, and the directory is rebuilt from it (`SYNC-037`) |
| the relay's word that the members changed | the list of each open document is refetched, and the directory rebuilt from it (`SYNC-037`) |
| changing a member's role | the relay is asked, and the answer refreshes the list |
| receiving a share notice | the document list refreshes, and the user is told, with no reload |
| opening a document | the key, the directory and the role all arrive on one answer |

What a user sees of the role is an affordance that is present or absent. The relay stays the
authority: the role here hides what the relay would refuse anyway.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| One answer carries the key, the directory and the role | a request per question | it is what a reader expects and what makes a stale directory possible: the three change together -- an invite adds a key, a member and a role in one act -- and they are read at the same instant, at open. Apart they cost three round trips and three moments at which they disagree |
| The recipient unwraps against the wrapper's key | unwrapping against the member's own key | it happens to work for a single-member document and fails silently on the first shared one. The exchange is symmetric, so only one of the two public keys yields the secret, and the relay is the only party that knows which member wrapped |
| The directory maps signing keys, and lives in the Sync worker | resolving in the Logic thread | it would need the keys to cross the boundary `SYNC-039` closes. The resolution has to happen where signatures are verified, which is this worker and nowhere else |
| An unresolvable author is dropped | trusting an identifier in the payload | an identity guessed from the payload is a peer's own claim, which is exactly what keying on a resolved identity exists to refuse |
| An unresolvable author is dropped | attributing the envelope to the first member | it puts a name on an envelope nobody can vouch for, and it is wrong silently. Presence is best-effort and self-correcting instead: the next sample arrives a moment later |
| An unresolvable role is the read-only one | defaulting to the owning role | offering an affordance the relay refuses costs a confusing error after the edit, where hiding one the user has costs a click through a menu. The role is a user-experience gate, so the failure directions are asymmetric |
| An unresolvable role is the read-only one | refusing the open | it turns a gate on affordances into a gate on reading, for a value that is missing rather than denied |
| A failed unwrap stops the open | opening with a fresh key and repairing later | it was the behaviour audit `H3` found: a random key opens the document and then poisons it, because this client seals its edits under a key nobody else holds, and every peer sees a commitment failure on content that looks like ordinary history |
| The relay serves a signed log beside its table | a member list the client takes on the relay's word | the relay could show any member, or hide any member. The rule would have to be added to every membership already created, and those created before it could never be checked |
| A revoke is a signed entry | signing the invites alone | the relay could serve an old invite again after a revoke, and the member would reappear verified |
| The first entry travels with the wrapped key | fetching the first entry from the relay | the relay could root a whole log at a creator of its own making, and every entry would check |
| The invite link pins the inviter's signing key | keeping the anchor of the first open, trust on first use | the first open is the one the relay attacks: a relay that wraps the first copy chooses the anchor the client then keeps, and the attack stays invisible for the life of the document |
| The proved pin is kept in the local store, per document | a sealed copy of the pin kept on the relay | the relay could withhold it or serve an older one, and a new route would carry an object the relay stores for each member. The local store already holds the trust pins, and a lost row fails closed (`SYNC-210`) |
| The kept pin wins over the link of a second inviter | replacing the kept pin with the newest link's | a link the relay or a stranger crafted would erase the one proof the relay cannot forge, and the next open would trust the log the new pin anchors |
| The kept pin wins over the link of a second inviter | checking the log against both pins | it costs code in the Sync worker, which holds no room under its ceiling, and adds no proof: the kept pin already fixes the anchor |
| The pin rides in the fragment of the link | a query parameter, or a field the share notice carries | the browser sends a query to the relay, and the relay writes the notice: either way the relay could swap the pin it is meant to be checked against |
| Membership is the relay's table | membership as a CRDT container inside the document | adding a member would take a delta, so the relay could not answer "who may read this" without opening the document, and no invite could reach someone who cannot yet decrypt |
| An entry signs the member's exchange key | a certificate each user signs over their own exchange key, served by the relay | it adds a second signed object, a column and a route, for the binding the entry already carries. The entry that adds a member already names the signing key, and an invite already holds the confirmed exchange key |
| An entry of version 1 verifies nothing | a migration that signs the old entries again | only the author of an entry can sign it, and the relay holds no key. A log is short-lived before the first release, so the cost is a set of documents shown unverified |
| A delta counts when the log ever granted its signer a writing role | only the writers present now | a demotion would make one peer refuse what another applied before it, and the document would diverge. No delta carries a place in the log to date it |
| A delta from an unknown signer waits for a listing | refusing it at once | a newcomer's first delta outruns the word that adds them after a reconnect, and refusing it loses that edit on this client until the next open |
| A head counts when the entry it names checks | the head alone, under the signature of the update | any member could sign a head of no log, and the relay would be blamed for an entry it never had (`SYNC-219`) |
| Every update carries the entry | the entry on the first update after the head moves alone | the relay would drop those few updates and forward the rest. The member would then read heads that prove nothing, and detect nothing |
| A presence sample carries the head and its entry | no head on a sample | a relay that drops every update of the up-to-date members towards one member would hide the gap, and presence flows while nobody writes (`SYNC-263`) |
| A presence sample carries the entry beside the head | the head alone on a sample | a head counts only when its entry proves it (`SYNC-219`), so a sample with the head alone would never ask for the log. A weaker proof for a sample would let any member blame the relay |
| A census opens each document a removal caps | the clock of each author the relay's index of deltas holds | the relay's word on a clock: with the removed member, it raises the cap and the member keeps writing. An open reads only clocks under a signature the walk admits |
| A census opens each document a removal caps | asking the Logic thread to open them | the Logic thread would load each document for nothing, and the census needs no content. The Sync worker already runs the open whole, the snapshot check aside |
| The Logic thread sends the documents of the `meta` with the removal | the Logic thread pushes them to the Sync worker whenever the `meta` changes | a channel never opened here pushes nothing, so its departure or its revoke would fail |
| The Logic thread sends the documents of the `meta` with the removal | the Sync worker asks the Logic thread for them during the census | a second round trip on the bus, and the Logic thread still opens a closed channel to answer it |
| The Logic thread reads every segment and live amendments document for the threads a removal names | the `meta` names each thread | each first reply then writes the `meta` too. A thread whose replier wrote no entry stays unchecked, so the census would still read the segments |
| The Logic thread reads every segment and live amendments document for the threads a removal names | the read holds every document it opens until the removal leaves | a channel with a long history would hold it all at once, against the memory budget of `README.md`. One document at a time holds one copy |
| The Logic thread sends the documents of the `meta` with the removal | the Sync worker reads the `meta` itself | the Sync worker would link the CRDT engine and the Murmure schema, for one list, past its size budget |
| A channel past 16 documents fails the removal | capping the 16 first and leaving the rest | the member keeps writing with the relay in each document left, and nobody sees it |
| A start past a lifted cap stands on a confirmation that signs its confirmer's head | the relay keeps every delta of a readmitted writer past the lifted cap, and the open checks each one | the deltas kept grow with the history of that writer, and each open downloads all of them, against the memory budget of `README.md`. The open would also have to match each fold against the ops of those deltas in the Logic thread. The relay could still trim them, and the open would fail |
| A start past a lifted cap stands on a confirmation that signs its confirmer's head | the snapshot carries the signed deltas it folds past the lifted cap | each snapshot grows with the history of the writer, and no fold is bound to those deltas without the same check in the Logic thread |
| A start past a lifted cap stands on a confirmation that signs its confirmer's head | the confirmation as it stood, with no head | a member who confirmed before it learned the revoke vouches for writes of the exclusion. The relay can delay the revoke to choose that member |
| An invite names the key it wraps | trusting the wrap the relay serves | the relay could hand the invitee another document's key, or an old one, and nothing would tell the two apart |

---

## 6. NOT BUILT, AND NOT YET RULED

* **No test holds the fallback of the wrapper-key choice.** A member row with no wrapper recorded falls
  back to the member's own key, for an older relay. Nothing states how long that fallback lives, and
  no test takes either branch.
* **Nothing states what a role change does to an open document.** The relay answers the request and
  the list refreshes. The role carried on the ready signal was read at open and is not revisited, so
  a demotion takes effect on the next open.
* **No test drives an open.** The whole open path runs in a browser: the members request, the
  unwrap, the directory and the role. The host tier reaches its pure transforms alone.
* **A document created before the log has no first entry.** Its rows stay unverified for ever, and
  no rule states how such a document gains a log.
* **The kept pin lives on one device.** The local store keeps it (`SYNC-238`), so a second
  device, or a cleared browser, needs the link again. Nothing carries the pin from one device to
  another.
* **A tab opened before the kept pin was written does not read it.** The store is read once per
  page (`SYNC-240`), so another tab's first open of the document stays on its own link.
* **A member's exchange key stands on the relay's word.** The log signs a member's signing key,
  never the exchange key, so the wrapper check reads the exchange key off the relay's row.
* **A writer removed from the log still names keys.** A link counts when its signer held a writing
  role at any point. A removed writer and the relay together can thus name a key of their own.
* **A writer another member removed writes nothing past the cap of its revoke.** The revoke caps
  the writer at the highest clock its author applied, and a delta past it reaches no document
  (`SYNC-300`). A later invite readmits only the deltas that sign a head at or past it
  (`SYNC-301`).
* **A writer who leaves or is demoted still writes through a forwarding relay.** A delta counts
  when its signer held a writing role at any point (`SYNC-211`). A departure of a document caps its
  leaver (`SYNC-306`), but a demoted writer holds the current key. No rule yet binds such a delta
  to the role of its signer at the head it signs.
* **A snapshot past a cap with no reference under the caps replays from the first delta.** The
  relay trims the deltas under the oldest confirmed snapshot, so that replay can lack history. No
  rule yet fetches an older snapshot, and none states what the open shows then.
* **A snapshot that folds a write of a readmitted writer opens.** The fold names a clock and no
  head, and a readmission lifts the cap the check reads (`SYNC-301`).
* **A departure cap is the leaver's own word.** A leaver who names a clock past its writes keeps
  writing up to it. An honest leaver names its last clock, so a key stolen later writes nothing.
  A departure with no cap caps nothing.
* **A channel with no owner cannot erase a thread name.** A direct message leaves nobody owner, so
  a `thread` field naming no document of the channel still fails each departure from it
  (`SYNC-322`).
* **An owner trusts the listing when it erases.** No client can tell a thread the relay never
  linked from a linked thread a dishonest relay hides. An owner's census erases both (`SYNC-323`).
  The floor of zero still caps the member there (`SYNC-315`), so the removal loses that history
  alone.
* **A removal opens each segment of the channel twice.** The Logic thread reads it for its threads,
  then the census opens it again for its clocks (`SYNC-312`). The read opens one document at a
  time, so its wait grows with the history of the channel.
* **A document created after the census takes zero.** A segment a close opens, or a thread a reply
  creates, between the listing and the post is not in the census. The member's writes there are
  refused.
* **The floor reaches the snapshots of every document of the channel.** A grant lifts the floor of
  zero with the named caps (`SYNC-315`). A fold past it opens only under a confirmation at or past
  the grant (`SYNC-316`, `SYNC-317`), in every document of the channel, named or not.
* **A census opens every document of the channel.** The floor caps a thread the member never wrote
  in. The census cannot tell such a thread without opening it.
* **A removal posts its run of entries in one request.** The relay appends the run whole or not at
  all (`SRV-339`). A relay that drops part of the run hides entries from every peer, and the head an
  update carries shows it (`SYNC-207`).
* **A census waits on the socket.** Its replay arrives on the socket, and no timer ends a census
  whose replay never comes. The removal then never posts.
* **A census reads the first page of a replay.** The open says it is ready on the first page
  (`SRV-086`), and a replay past one page caps the member below their last clock there.
* **A held delta waits for a listing that may fail.** A listing that fails asks nothing again, so
  its held deltas wait for the next change of the members (`SYNC-211`).
* **A Murmure invitee needs the link on each new device.** The kept roots live in the local store
  of one browser (`SYNC-241`). A second device, or a cleared browser, refuses the spaces of an
  invitee who signed no entry until it opens the link again.
* **The Murmure boot hands the Sync worker every kept pin.** A Murmure page names no document, so
  the boot sends the pins of every Prose, Trame and Toile document too (`SYNC-243`). Each one
  roots only the open of its own document, so no open reads a pin of another.
* **A Murmure root lands late or never.** A member who joins a space after this client accepted
  its log roots nothing until the next open of that log. A direct message such a member creates is
  refused until then. An open that runs before the open of its space is refused the same way.
* **A claim lasts one session.** An open under another type claims its document for the session
  alone (`SYNC-260`). A later session holds no claim until that document opens again under its type.
* **A relay can withhold the latest entries from every frame of a member.** An update and a
  presence sample carry the head their author read (`SYNC-207`, `SYNC-263`). A relay that drops
  every frame of the up-to-date members towards one member partitions it, and no rule detects that.
* **The author of an entry can still blame the relay.** A member signs an entry its role allows,
  announces it and never sends it. Every member may sign its own removal, so every member can do
  this. The warning then means that the relay withheld the entry or that its author never sent
  it, and no rule tells the two apart. A receipt the relay signs at the append would prove that the
  relay held the entry (`SYNC-219`).
* **A head past a promotion proves nothing.** The check reads the roles of the log this client
  holds. An entry whose author gained the right in a hidden entry asks nothing (`SYNC-219`).
* **A warned member asks for no log again by itself.** Its edits wait until a later read, such as
  the member panel's or the next change of the members, serves the head (`SYNC-220`).
* **A held edit can leave under a retired key.** A relay can serve the revoke and still withhold
  the new key. The edits then leave under the key the client holds, and no rule checks it against
  the key the log installs.
* **A reopen forgets a withheld head.** The check starts clear, so an edit left unsent leaves
  under the old key until a signed head arrives again (`SYNC-220`).
* **Nothing rules on a member who cannot be reached.** An invitee whose address resolves to no
  account is refused. An account that exists and never opens the document keeps a wrapped key for
  ever, and no rule bounds that.

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| The trust step before a key is wrapped, and what a fingerprint is | `ambre-spec-key-verification.md` |
| What a revoke rotates, and what a rotation leaves behind | `ambre-spec-key-rotation.md` |
| How an envelope is signed, and what refuses a forged author | `ambre-spec-sealed-envelope.md` |
| What a presence sample carries, and what it paints | `ambre-spec-presence.md` |
| What the relay stores per member, and which role may do what | `ambre-spec-server.md` |
| Where the role gate is applied on a command | `ambre-spec-message-bus.md` |
| Which key opens the document once it is unwrapped, and who holds it | `ambre-spec-sync.md`, `SYNC-002` |
| What the sharing screens look like | `ambre-spec-chrome.md`, `ambre-spec-launcher.md` |
