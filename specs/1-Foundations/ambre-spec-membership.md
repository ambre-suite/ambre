# AMBRE -- FEATURE: MEMBERSHIP AND THE WRAPPED KEY

**Type:** Horizontal (framework)
**Implementation:** partial -- the wrap, the unwrap, the directory and the role all ship. A
transient failure of the members request opens the document as if it were new.

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
| `SYNC-093` | `FRAME` | A refused invitation MUST wrap nothing: no request MUST leave beyond the one that fetched the key. | `ambre-spec-key-verification.md`; audit `M14` | `ambre_sync::sharing::tests::invite_pin_mismatch_refuses_and_never_wraps` -- exactly one call was made, the search, so no wrap ran and no share request fired. It is the row that makes the pin comparison worth anything: refusing after the wrap has left tells the attacker the key anyway | none | verified |

### 1.3 A document is opened, and its key unwrapped

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-016` | `POST` | The role carried on the ready signal MUST be this client's own row in the members answer. | this spec section 3; `ambre-spec-message-bus.md`, `BUS-008` | `ambre_sync::sharing::tests::own_role_reads_our_own_row` -- a peer holding owner does not raise ours; `::own_role_reads_writable_roles` -- the writable side; `ambre_sync::updates::tests::document_ready_carries_the_role_on_both_paths` -- both open paths carry it | none | verified |
| `SYNC-017` | `FAIL` | A role that cannot be resolved MUST default to the read-only one, never to a writable one. | this spec section 3 | `ambre_sync::sharing::tests::own_role_falls_back_to_viewer_when_unresolvable` -- both unresolvable cases, no session identity yet and no row for us in the answer, answer the viewer role | none | verified |
| `SYNC-034` | `POST` | A recipient MUST unwrap against the exchange key of whoever wrapped, never against its own. | this spec section 3 | `ambre_crypto::tests::ecdh_symmetric`; `::kek_binds_peer_identity` -- the primitive under the rule; `ambre_sync::updates::tests::a_copy_unwraps_against_its_wrapper_and_never_its_holder` -- a peer's copy opens against the peer and fails against the holder, and a row naming no wrapper reads as self-wrapped | none | verified |
| `SYNC-040` | `FAIL` | A transient failure of the members request MUST NOT be read as a document that does not exist. | this spec section 3 | `ambre_sync::updates::tests::a_failed_members_request_is_not_an_absent_document` -- a transport failure is classified apart from an absence, and the open reports it instead of making the document local; `ambre_sync::updates::tests::a_refused_members_request_keeps_the_relay_refusal` -- a refused request keeps the relay's answer, and the open does not call the relay unreachable | none | verified |
| `SYNC-012` | `FRAME` | A failed unwrap of a document key MUST NOT be replaced by a freshly generated one. | derived (audit `H3`) | `ambre_sync::updates::tests::a_key_that_does_not_unwrap_opens_nothing` -- a tampered key row fails the open, and no key comes out in its place | none | verified |
| `SYNC-173` | `FAIL` | Given an open that fails, the client MUST tell a key it does not hold apart from a failure that a later open can clear. | this spec section 3; `MURMURE-218` | `ambre_sync::updates::tests::a_key_that_does_not_unwrap_opens_nothing` -- a key row that does not unwrap fails as a key not received; `ambre_sync::updates::tests::a_key_the_invitation_does_not_name_is_refused` -- so does a key the invitation does not name; `ambre_sync::websocket::tests::a_first_replay_given_up_fails_the_open` -- a stopped replay fails as transient; `ambre_sync::updates::tests::a_refused_members_request_fails_the_open_as_refused` -- a `4xx` members answer fails as refused, with its code, and a `5xx` stays transient | none | verified |

### 1.4 An ephemeral is attributed

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-014` | `POST` | An ephemeral MUST reach the Logic thread under the identity the member directory maps its author key to. | this spec section 3 | `ambre_sync::ephemeral_tests::author_resolves_to_the_member_behind_the_signing_key`; `ambre_sync::sharing::tests::presence_directory_maps_valid_entries`; `::list_members_refreshes_the_presence_directory` -- the directory the resolution reads | none | verified |
| `SYNC-037` | `POST` | The member directory MUST be rebuilt whenever membership changes. | this spec section 3 | `ambre_sync::sharing::tests::list_members_refreshes_the_presence_directory` -- the members answer replaces it; `ambre_sync::updates::tests::a_change_of_members_reads_the_directory_again_and_a_crossed_answer_changes_nothing` -- the relay's word (`SRV-177`) lists again, and a crossed older answer changes nothing | none | verified |
| `SYNC-150` | `POST` | A change of members committed while a document opens MUST reach the directory that open builds. | this spec section 3; `SRV-177` | `ambre_sync::updates::tests::a_subscription_reads_the_members_a_change_during_the_open_left_out` -- the relay's acknowledgement of a subscription lists the members again, so a newcomer committed before any word could reach this client resolves | none | verified |
| `SYNC-151` | `FRAME` | The relay's word that the members changed MUST NOT cost an open document more than one member request. | this spec section 3; `SRV-177` | `ambre_sync::updates::tests::a_change_of_members_costs_one_listing_whose_rows_reach_logic` -- the word costs one listing, and its fresh answer reaches Logic with the rows; `ambre_logic::spaces::tests::a_change_of_members_hands_the_rows_and_costs_the_space_no_request` -- an open space takes those rows and asks for none | none | verified |
| `SYNC-015` | `FAIL` | An ephemeral whose author key matches no member MUST be dropped, never delivered under a guessed identity. | this spec section 3 | `ambre_sync::ephemeral_tests::unknown_author_key_is_not_resolved`; `ambre_sync::sharing::tests::presence_directory_skips_unparsable_rows_without_inventing_ids` -- a members answer with two rows whose signing key is malformed reads whole, the directory skips both, and only the sound row reaches it | none | verified |
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
| `SYNC-111` | `FRAME` | Checking a member list MUST NOT block the open of the document. | this spec section 3 | `ambre_sync::updates::tests::a_key_the_invitation_does_not_name_is_refused` -- the open reads the log and still completes, the check deciding only which key it holds | none | verified |

* **Budget.** the key exchange costs one elliptic-curve operation per member at open, charged to
  the crypto rows of `ambre-spec-budgets.md section 1`. The directory and the role are read off an
  answer the open already fetches, so both are free. The log adds one check per entry.
* **Boundary.** the wrapped key crosses the relay and the relay can open none of them: `SYNC-010`
  seals each copy for one exchange key. What it learns is who is a member of what, which is the
  membership metadata `ambre-spec-zero-knowledge.md` accepts.
* **Verification.** 36 `verified`. Every pure transform holds, out of the browser: the wrap, the
  weak key, the pin, the invitee, the role and the log walk. What is loose is the branches that
  only run against a live relay.
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

**The directory is membership, not trust.** It answers "which member wrote this envelope", trusting the
relay's mapping of keys to people. Detecting a relay that substitutes a member's key is the
trusted-key store's job (`ambre-spec-key-verification.md`), and presence does not wait on it.

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
- A new member learns the first entry from its inviter, inside the sealed wrap (`SYNC-104`). So a
  relay cannot hand a new member a log rooted at a creator of its own.
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

**How an entry is appended.** The relay checks the signature against the author's registered key.
It appends at the entry's position alone, under the document lock. Two writers racing for one
position cannot fork the log: the loser is answered `LOG_MOVED`, and signs again after the winner.

**A document older than the log.** Its log is empty, so the relay takes a change with no entry.
It has no first entry, so none of its rows verifies, and the list shows each as unverified. A
document whose log holds an entry refuses a change without one.

**A document the relay does not know is this client's own.** A missing document on open means it
exists on this client alone and has not been created on the relay yet. The local user is then the
only party, and holds the owning role. That inference is right for an absence and wrong for a failure,
which is `SYNC-040`.

**Code.**

* `crates/ambre-sync/src/sharing.rs` -- the wrap, the directory, the role, the invite and the list;
* `crates/ambre-sync/src/members.rs` -- the handlers that gather the state and spawn the requests;
* `crates/ambre-sync/src/updates.rs` -- the unwrap at open, and the wrapper-key choice;
* `crates/ambre-sync/src/membership_log.rs` -- the walk of the log, and the next entry to sign;
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
| Membership is the relay's table | membership as a CRDT container inside the document | adding a member would take a delta, so the relay could not answer "who may read this" without opening the document, and no invite could reach someone who cannot yet decrypt |
| An invite names the key it wraps | trusting the wrap the relay serves | the relay could hand the invitee another document's key, or an old one, and nothing would tell the two apart |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The wrapper-key choice has an untested fallback.** A member row with no wrapper recorded falls
  back to the member's own key, for an older relay. Nothing states how long that fallback lives, and
  no test takes either branch.
* **Nothing states what a role change does to an open document.** The relay answers the request and
  the list refreshes. The role carried on the ready signal was read at open and is not revisited, so
  a demotion takes effect on the next open.
* **No test drives an open.** The whole open path runs in a browser: the members request, the
  unwrap, the directory and the role. The host tier reaches its pure transforms alone.
* **A document created before the log has no first entry.** Its rows stay unverified for ever, and
  no rule states how such a document gains a log.
* **A relay can withhold the latest entries.** Nothing tells a client that its list is late. A
  signed head of the log, pinned by each member, would make the gap visible, and no rule states it.
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
