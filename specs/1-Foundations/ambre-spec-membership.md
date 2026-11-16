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
| `SYNC-208` | `POST` | An invite that lands MUST hand the inviter a link to the document that carries the pin of the inviter's own signing key. | this spec section 3; `SYNC-205` | `ambre_sync::sharing::tests::invite_success_posts_wrapped_key_and_emits_invitesent` -- the landed invite carries the pin of the key that signs the inviter's entries; `ambre_app::switchboard::tests::a_landed_invite_copies_the_link_pinned_to_its_inviter` -- the shell puts the link pinned to that key on the clipboard; `ambre_app::route::tests::an_invite_fragment_reads_back_to_its_pin` -- the fragment of the link reads back to the same pin | none | verified |
| `SYNC-093` | `FRAME` | A refused invitation MUST wrap nothing: no request MUST leave beyond the one that fetched the key. | `ambre-spec-key-verification.md`; audit `M14` | `ambre_sync::sharing::tests::invite_pin_mismatch_refuses_and_never_wraps` -- exactly one call was made, the search, so no wrap ran and no share request fired. It is the row that makes the pin comparison worth anything: refusing after the wrap has left tells the attacker the key anyway | none | verified |

### 1.3 A document is opened, and its key unwrapped

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-016` | `POST` | The role carried on the ready signal MUST be this client's own row in the members answer. | this spec section 3; `ambre-spec-message-bus.md`, `BUS-008` | `ambre_sync::sharing::tests::own_role_reads_our_own_row` -- a peer holding owner does not raise ours; `::own_role_reads_writable_roles` -- the writable side; `ambre_sync::updates::tests::document_ready_carries_the_role_on_both_paths` -- both open paths carry it | none | verified |
| `SYNC-017` | `FAIL` | A role that cannot be resolved MUST default to the read-only one, never to a writable one. | this spec section 3 | `ambre_sync::sharing::tests::own_role_falls_back_to_viewer_when_unresolvable` -- both unresolvable cases, no session identity yet and no row for us in the answer, answer the viewer role | none | verified |
| `SYNC-034` | `POST` | A recipient MUST unwrap against the exchange key of whoever wrapped, never against its own. | this spec section 3 | `ambre_crypto::tests::ecdh_symmetric`; `::kek_binds_peer_identity` -- the primitive under the rule; `ambre_sync::updates::tests::a_copy_unwraps_against_its_wrapper_and_never_its_holder` -- a peer's copy opens against the peer and fails against the holder, and a row naming no wrapper reads as self-wrapped | none | verified |
| `SYNC-040` | `FAIL` | A transient failure of the members request MUST NOT be read as a document that does not exist. | this spec section 3 | `ambre_sync::updates::tests::a_failed_members_request_is_not_an_absent_document` -- a transport failure is classified apart from an absence, and the open reports it instead of making the document local; `ambre_sync::updates::tests::a_refused_members_request_keeps_the_relay_refusal` -- a refused request keeps the relay's answer as the cause of the failed open, which never reads as a relay out of reach | none | verified |
| `SYNC-012` | `FRAME` | A failed unwrap of a document key MUST NOT be replaced by a freshly generated one. | derived (audit `H3`) | `ambre_sync::updates::tests::a_key_that_does_not_unwrap_opens_nothing` -- a tampered key row fails the open, and no key comes out in its place | none | verified |
| `SYNC-173` | `FAIL` | Given an open that fails, the client MUST tell a key it does not hold apart from a failure that a later open can clear. | this spec section 3; `MURMURE-218` | `ambre_sync::updates::tests::a_key_that_does_not_unwrap_opens_nothing` -- a key row that does not unwrap fails as a key not received; `ambre_sync::updates::tests::a_key_the_invitation_does_not_name_is_refused` -- so does a key the invitation does not name; `ambre_sync::websocket::tests::a_first_replay_given_up_fails_the_open` -- a stopped replay fails as transient; `ambre_sync::updates::tests::a_refused_members_request_fails_the_open_as_refused` -- a `4xx` members answer fails as refused, with its code, and a `5xx` stays transient | none | verified |
| `SYNC-200` | `FAIL` | Given a key that no writer the log verifies named, in an entry or as the key a link it signed starts from, the client MUST refuse the key. | this spec section 3; `SYNC-121`, `SYNC-123` | `ambre_sync::key_vouch::tests::a_key_no_writer_named_is_refused` -- the key the creation names opens, and so does a key a writer's link starts from, while a key the relay wrapped itself is refused with a link the relay signed or with none; `::a_copy_with_no_anchored_log_is_refused` -- a copy with no anchor is refused; `ambre_sync::membership_log::tests::only_a_writers_entry_names_a_key` -- a self-join and a reader's departure name no key; `ambre_sync::updates::tests::a_key_the_relay_substitutes_is_refused_at_open` -- a members answer where the relay re-wrapped a key of its own and dropped the invite fails the open, and a key no writer named under a wrapper the log verifies fails it as a key no member signed (`SYNC-212`) | none | verified |
| `SYNC-201` | `FAIL` | Given a copy wrapped under an exchange key that is neither this client's nor one a valid entry signs for a verified member, the client MUST refuse the key. | this spec section 3; `SYNC-034`, `SYNC-103`, `SYNC-197` | `ambre_sync::key_vouch::tests::a_wrapper_the_log_does_not_verify_is_refused` -- a copy this client wrapped and one a verified member wrapped pass, a wrapper the log signs for no member is refused, and under a space a wrapper no row carries passes as a space writer; `ambre_sync::key_vouch::tests::a_wrapper_key_the_relay_swaps_on_a_row_is_refused` -- the relay's key on a verified member's row is refused, under a space too, and a withheld row takes no vouch away; `ambre_sync::updates::tests::a_key_the_relay_substitutes_is_refused_at_open` -- the honest answer, wrapped by a verified member, opens, and the swap under the whole log is refused | none | verified |
| `SYNC-202` | `FAIL` | Given a key that a link a writer signed replaced, the client MUST refuse the key. | this spec section 3; `SYNC-116`; audit `N13` | `ambre_sync::key_vouch::tests::a_key_a_writer_replaced_is_refused` -- a copy the relay kept from before a rotation is refused once the link of that rotation is served | none | verified |
| `SYNC-205` | `FAIL` | Given an open of a document whose invite link pinned an inviter, the client MUST refuse the key unless a valid entry of the log the open walks is signed by the key the pin names. | this spec section 3; `SYNC-104`, `SYNC-200`, `SYNC-201`; audit `N15` | `ambre_sync::key_vouch::tests::a_log_the_pinned_inviter_did_not_sign_is_refused` -- a whole log the relay rooted at a creator of its own passes every other check, and the pin of the true inviter refuses it while it admits the true log, and a log with no anchor or a pin of a member who signed nothing admits nothing; `ambre_sync::updates::tests::an_open_pinned_to_its_inviter_refuses_a_log_the_relay_rooted` -- the open accepts the relay's log unpinned and refuses it pinned, as a key no member signed (`SYNC-212`); `ambre_sync::updates::tests::a_pinned_inviter_outlives_a_close` -- the pin holds for every open of the session; `ambre_crypto::tests::an_inviter_pin_is_stable_and_differs_per_key` -- the pin under it | none | verified |
| `SYNC-209` | `POST` | The pin a link carries MUST reach the Sync worker before the open that link drives. | this spec section 3; `SYNC-205` | `ambre_app::route::tests::an_invite_fragment_reads_back_to_its_pin` -- the route reads the pin off the fragment, and a malformed one pins nothing; `ambre_logic::dispatch::tests::a_pinned_inviter_reaches_sync_before_its_open` -- the Logic thread forwards the pin ahead of the open | none | verified |
| `SYNC-210` | `FAIL` | Given an open of a document that names no pinned inviter, the client MUST refuse the key unless this client wrapped its own copy or a valid entry of the log is signed by this client's own signing key. | this spec section 3; `SYNC-205` | `ambre_sync::key_vouch::tests::an_open_with_no_link_is_refused_unless_this_client_vouches_for_the_log` -- an invitee with no link is refused on the inviter's log and on a log the relay rooted alike, a copy the invitee wrapped itself opens, and the creator opens a copy another member wrapped for it, since it signed the creation, but not when the open knows no signing key of its own; `ambre_sync::updates::tests::an_open_pinned_to_its_inviter_refuses_a_log_the_relay_rooted` -- the open with no link fails as a key not received on either log | none | verified |
| `SYNC-212` | `FAIL` | Given a key refused because no writer the log verifies named it, or because the pinned inviter signed no entry of the log, the open MUST fail as a key no member signed, never as a key not received. | this spec section 3; `SYNC-173`, `SYNC-200`, `SYNC-205`; `ambre-spec-message-bus.md`, `BUS-030` | `ambre_sync::key_vouch::tests::only_a_key_no_member_signed_fails_as_the_relays_fault` -- an unnamed key and a log the inviter did not sign fail as a key no member signed, and the four refusals an honest relay can cause stay a key not received; `ambre_sync::updates::tests::a_key_the_relay_substitutes_is_refused_at_open` -- a key no writer named, under the genuine log and a wrapper it verifies, fails the open that way; `ambre_sync::updates::tests::an_open_pinned_to_its_inviter_refuses_a_log_the_relay_rooted` -- so does a log the relay rooted, pinned, while the open with no link stays a key not received; `ambre_bus::host::a_key_no_member_signed_crosses_the_bus_as_its_own_failure` -- the failure crosses the bus with its own bytes and its own words | none | verified |

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
| `SYNC-107` | `FRAME` | Recording an entry MUST NOT change an earlier entry of the log. | this spec section 3 | `ambre_server::routes::tests::handler_tests::the_register_keeps_each_signed_entry_beside_the_rows` -- a third entry leaves the first two byte for byte, and a stale position is refused | none | verified |

### 1.6 A member list is checked

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-108` | `FAIL` | Given a member row with no valid chain, the client MUST show it as unverified. | this spec section 3 | `ambre_sync::sharing::tests::the_member_list_shows_what_the_log_verifies` -- a row the log does not link reaches the list as unverified; `ambre_sync::membership_log::tests::an_entry_its_author_did_not_sign_or_may_not_make_verifies_nothing`; `ambre_sync::membership_log::tests::an_entry_beyond_its_authors_standing_ends_the_chain` -- an editor re-inviting the owner, or an owner revoking a non-member, ends the chain | none | verified |
| `SYNC-109` | `FAIL` | Given an invite replayed after a later revoke of the same member, the client MUST count the member as revoked. | this spec section 3 | `ambre_sync::membership_log::tests::an_invite_replayed_after_a_revoke_leaves_the_member_revoked` | none | verified |
| `SYNC-110` | `POST` | The role a client shows for a member MUST be the role of the latest valid entry for that member. | this spec section 3 | `ambre_sync::membership_log::tests::the_role_is_the_latest_valid_entrys`; `ambre_sync::sharing::tests::the_member_list_shows_what_the_log_verifies` -- the list carries the logged role over the relay's column; `::a_role_change_posts_the_entry_that_gives_the_role` | none | verified |
| `SYNC-217` | `FAIL` | Given an entry that replaces the keys of a present member, the client MUST count it valid only when an owner signed an invite. | audit `N17`; this spec section 3; `SRV-266` | `ambre_sync::membership_log::tests::no_entry_but_an_owners_replaces_a_present_members_keys` -- an editor's re-invite of a present editor under keys of its own, and a self-join for a present member under a key it names itself, each end the chain and leave the member's keys, while an owner's re-invite lands | none | verified |
| `SYNC-111` | `FRAME` | Checking a member list MUST NOT block the open of the document, beyond the checks of its key. | this spec section 3 | `ambre_sync::updates::tests::a_key_the_invitation_does_not_name_is_refused` -- the open reads the log and still completes, the check deciding only which key it holds | none | verified |
| `SYNC-206` | `POST` | Every update MUST carry, under its author's signature, the head of the membership log its author last read. | this spec section 3; `ambre-spec-sealed-envelope.md` | `ambre_sync::withheld::tests::a_log_served_whole_clears_the_check_and_heads_the_next_delta` -- the next delta signs the head of the log the client last read; `ambre_sync::pipeline::tests::a_rewritten_log_head_breaks_the_signature` -- a head rewritten in transit fails the signature, alone and at the open | none | verified |
| `SYNC-207` | `FAIL` | Given a member's signed head that the log this client read does not pass through, the client MUST read the log again, and MUST warn the user when that log still does not pass through it. | this spec section 3; `ambre-spec-key-rotation.md` | `ambre_sync::withheld::tests::a_peer_detects_a_revoke_the_relay_hid_from_it` -- a relay hides a revoke and its rotation from Bob, Alice's next delta under the new key still proves its head, the log read again lacks the revoke, and Bob is warned once; `::a_log_served_whole_clears_the_check_and_heads_the_next_delta` -- an honest relay serves the entry and nobody is warned; `::a_head_no_member_signed_asks_nothing` -- a key outside the directory, or a broken signature, asks for no log; `::a_head_past_the_log_asks_once_and_a_log_without_it_is_withheld` -- one read at a time, and an answer to a read that left before the head proves nothing; `::a_served_log_holds_each_head_it_passes_through_and_no_other` -- a head past the log or forking from it is not held | none | verified |

### 1.7 A delta is bound to a writer

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-211` | `FAIL` | Given a delta or a snapshot of a document other than a Murmure one, whose signer no valid entry of the log ever granted a writing role, the client MUST refuse it before it reaches the document, whatever key opens it. A delta MUST first wait for a listing of the members asked after it arrived, and that listing MUST decide it. | this spec section 3; `SYNC-103`, `SYNC-121`, `SYNC-150` | `ambre_sync::updates::tests::a_relay_forwarding_a_readers_delta_reaches_no_document` -- a reader's delta under the current key and under a retired one, forwarded by the relay, reaches no document and raises a warning each, while the owner's and a removed writer's land, and a listing in flight is waited for; `ambre_sync::updates::tests::a_snapshot_no_writer_signed_opens_nothing` -- a reader's snapshot opens nothing and the open replays from the start, the owner's opens; `ambre_sync::updates::tests::a_newcomers_delta_waits_for_the_listing_asked_after_it` -- a newcomer's delta waits, an answer to a listing asked before it leaves it waiting, and the listing that names the newcomer lands it; `ambre_sync::membership_log::tests::a_writer_once_granted_stays_a_writer_and_a_viewer_never_is` -- a viewer and a stranger are no writers, a writer demoted then removed stays one, and a log with no anchor checks nobody; `ambre_sync::updates::tests::a_murmure_document_takes_a_delta_its_own_log_names_no_writer_of` -- a Murmure document keeps no writer from a listing, and a member's delta lands with the creator's | none | verified |
| `SYNC-214` | `FAIL` | Given a members answer whose walked log drops a writer the held log granted, the client MUST keep the writers and the space word it held. | audit `N18`; this spec section 3; `SYNC-211` | `ambre_sync::updates::tests::a_listing_that_drops_a_writer_leaves_the_writers_of_the_open` -- a listing that leaves the log unanchored, and one the relay rooted at a log of its own, each leave the writers of the open, and the reader's delta that waited reaches no document | none | verified |

* **Budget.** the key exchange costs one elliptic-curve operation per member at open, charged to
  the crypto rows of `ambre-spec-budgets.md section 1`. The directory and the role are read off an
  answer the open already fetches, so both are free. The log adds one check per entry.
* **Boundary.** the wrapped key crosses the relay and the relay can open none of them: `SYNC-010`
  seals each copy for one exchange key. What it learns is who is a member of what, which is the
  membership metadata `ambre-spec-zero-knowledge.md` accepts.
* **Verification.** 51 `verified`. Every pure transform holds, out of the browser: the wrap, the
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
- A new member learns the first entry from its inviter, inside the sealed wrap (`SYNC-104`). So a
  relay cannot hand a new member a log rooted at a creator of its own, unless it wraps the copy
  itself (section 6).
- A client walks the chain from the first entry. A row it cannot link is shown as unverified
  (`SYNC-103`, `SYNC-108`). A revoke is an entry too, so an old invite replayed after it changes
  nothing (`SYNC-109`).
- An entry the member signs for themselves is valid only under a rule of the owning application
  (`SYNC-106`). Murmure accepts a self-join to a public channel of a space, checked against the
  sealed catalogue of that space.

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
- A client reads the head of each update a member of its directory signed. The signature is
  checked alone, so a frame under a key this client does not hold still counts. That frame is the
  one a hidden rotation brings.
- A head the log this client read does not pass through asks for the log once more. A log that
  still does not pass through it raises a warning (`SYNC-207`). Either the relay withholds an
  entry, or a member signed a head of no log. An answer to a read that left before the head proves nothing.

The detection needs one frame of an up-to-date member to reach the member. A relay that hides
every such frame partitions the member, and nothing in the document tells it apart from silence.

**How an entry is appended.** The relay checks the signature against the author's registered key.
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
3. no link a writer signed starts from a newer key and seals this one (`SYNC-202`).

The three checks read a log that the anchor of the copy roots. The relay can wrap a copy itself.
It then chooses that anchor too, and serves a whole log rooted at a creator of its own. The invite
link closes that for the open it drives:

* the link carries the pin of the inviter's signing key in its fragment (`SYNC-208`). The pin is a
  BLAKE2b digest under a tag of its own. A browser sends no fragment in a request, so the relay
  never reads it there;
* the shell hands the pin to the Sync worker before the open (`SYNC-209`);
* the open refuses the key unless a valid entry of the log is signed by the key the pin names
  (`SYNC-205`). The relay cannot sign such an entry, and each entry names the digest before it.
  So the anchor is the one the inviter's log starts from, and the three checks read that log.

The worker keeps the pin for the session, so a reopen and a rotation check the same log.

An open with no link fails closed (`SYNC-210`). It passes only on a copy this client wrapped
itself, or on a log with a valid entry its own signing key signed. A creator signed the
creation. The relay can forge neither. Any other open, from the document list or a share notice,
is refused until the user opens the invite link.

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
left. The held deltas share the bound of the quarantine.

**A document older than the log.** Its log is empty, so the relay takes a change with no entry.
It has no first entry, so none of its rows verifies, and the list shows each as unverified. A
document whose log holds an entry refuses a change without one. Its copy carries no anchor, so no
writer names its key and the open refuses it (`SYNC-200`). A relay could otherwise strip the anchor
of any copy to skip the check.

A rotation of such a document wraps the new key for the rotator alone, and reports every other
member (`SYNC-198`).

**An entry of version 1.** Version 2 of the entry signs the exchange key, under a new signing
domain. No migration rewrites a stored entry of version 1. Its signature checks under no key, so
the walk stops at it, and the rows past it read as unverified.

**A document the relay does not know is this client's own.** A missing document on open means it
exists on this client alone and has not been created on the relay yet. The local user is then the
only party, and holds the owning role. That inference is right for an absence and wrong for a failure,
which is `SYNC-040`.

**Code.**

* `crates/ambre-sync/src/sharing.rs` -- the wrap, the directory, the role, the invite and the list;
* `crates/ambre-sync/src/members.rs` -- the handlers that gather the state and spawn the requests;
* `crates/ambre-sync/src/updates.rs` -- the unwrap at open, and the wrapper-key choice;
* `crates/ambre-sync/src/membership_log.rs` -- the walk of the log, and the next entry to sign;
* `crates/ambre-sync/src/key_vouch.rs` -- the three checks of the key before it becomes current;
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
| The pin rides in the fragment of the link | a query parameter, or a field the share notice carries | the browser sends a query to the relay, and the relay writes the notice: either way the relay could swap the pin it is meant to be checked against |
| Membership is the relay's table | membership as a CRDT container inside the document | adding a member would take a delta, so the relay could not answer "who may read this" without opening the document, and no invite could reach someone who cannot yet decrypt |
| An entry signs the member's exchange key | a certificate each user signs over their own exchange key, served by the relay | it adds a second signed object, a column and a route, for the binding the entry already carries. The entry that adds a member already names the signing key, and an invite already holds the confirmed exchange key |
| An entry of version 1 verifies nothing | a migration that signs the old entries again | only the author of an entry can sign it, and the relay holds no key. A log is short-lived before the first release, so the cost is a set of documents shown unverified |
| A delta counts when the log ever granted its signer a writing role | only the writers present now | a demotion would make one peer refuse what another applied before it, and the document would diverge. No delta carries a place in the log to date it |
| A delta from an unknown signer waits for a listing | refusing it at once | a newcomer's first delta outruns the word that adds them after a reconnect, and refusing it loses that edit on this client until the next open |
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
* **An invitee needs the link at every session.** An open with no link is refused (`SYNC-210`),
  and the pin lives for one session alone. An invitee who signed no entry and holds no copy of
  its own wrapping opens from the document list, or from a share notice, nowhere. No rule yet
  keeps the pin, or the anchor it proved, across sessions.
* **A member's exchange key stands on the relay's word.** The log signs a member's signing key,
  never the exchange key, so the wrapper check reads the exchange key off the relay's row.
* **A writer removed from the log still names keys.** A link counts when its signer held a writing
  role at any point. A removed writer and the relay together can thus name a key of their own.
* **A writer removed or demoted still writes through a forwarding relay.** A delta counts when its
  signer held a writing role at any point (`SYNC-211`). A removed writer holds only the keys before
  the removal, so the delta reads as history written before it. A demoted writer holds the current
  key. No delta carries a place in the log, so no rule yet binds it to the role of its signer then.
* **A held delta waits for a listing that may fail.** A listing that fails asks nothing again, so
  its held deltas wait for the next change of the members (`SYNC-211`).
* **An evicted document reads a summary from any signer.** Its light subscription counts each
  delta and reads its summary with no check of the signer (`RESID-014`).
* **A document of a Murmure channel skips the key checks and the writer check.** Its rows and its
  links are the `meta`'s, and its log is its own. So no log the open walks names its key or the
  channel's writers. A segment's log names its creator alone, and a member who joined holds a copy
  anchored at `meta`'s log. Any member of a channel thus writes its documents through a forwarding
  relay (`SYNC-211`). The open does not name the `meta` yet.
* **A relay can withhold the latest entries from every frame of a member.** An update carries the
  head its author read (`SYNC-207`), and a presence sample carries none. A relay that drops every
  update of the up-to-date members towards one member partitions it, and no rule detects that.
* **A warned member keeps sealing under the old key.** The warning stops nothing: the member's next
  deltas still reach a removed member through the relay, and no rule holds them back.
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
