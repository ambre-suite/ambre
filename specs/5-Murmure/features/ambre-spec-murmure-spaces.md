# AMBRE -- FEATURE: SPACES AND THE CHANNEL CATALOGUE
**Type:** Vertical (feature)

> A team gathers in a space. The space lists its public channels, and any member joins one with no
> one else online. A guest reaches only the channels a member invites them into.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-031` | `INV` | A space MUST be a document, and its members MUST be the members of that document. | this spec section 3 | `ambre_murmure::logic::space_tests::a_new_space_is_one_document_that_holds_no_member`; `ambre_server::routes::channels::tests::a_new_space_has_its_creator_as_its_one_owner` -- database tier, the members are the relay's rows for that document | none | verified |
| `MURMURE-032` | `INV` | A member with the viewer role on the space MUST NOT change the catalogue. | this spec section 3; `SRV-008` | `ambre_logic::spaces::tests::a_viewer_creates_no_channel_and_adds_no_category` -- the client refuses before any write; `ambre_server::routes::channels::tests::a_viewer_or_a_stranger_registers_no_channel` -- database tier | none | verified |
| `MURMURE-033` | `INV` | The catalogue MUST hold the key of each public channel it lists, sealed under the key of the space. | this spec section 3; `ambre-spec-murmure-schema.md section 3` | `ambre_murmure::logic::space_tests::a_listed_channel_carries_its_name_topic_category_position_and_key`; `ambre_sync::channels::tests::a_sealed_channel_key_opens_for_its_own_channel_and_space_only`; `ambre_logic::spaces::tests::a_public_channel_is_listed_with_its_sealed_key_once_the_relay_holds_it` | none | verified |
| `MURMURE-034` | `INV` | A guest of a channel MUST NOT be a member of the space document. | this spec section 3 | `ambre_server::routes::channels::tests::a_self_join_from_outside_the_space_is_refused` -- database tier, a guest holds a row on a channel and none on the space; `ambre_server::routes::channels::tests::a_channel_takes_only_free_documents_of_its_creator` -- database tier, the space document never enters a channel | none | verified |
| `MURMURE-236` | `INV` | A removal from a channel MUST count as verified when the log of its space holds a signed removal of the same member. | this spec section 3; `SYNC-106` | `ambre_sync::membership_log::tests::a_channel_log_heeds_the_word_of_its_space`; `ambre_sync::channels::tests::a_listed_meta_walks_under_the_word_of_its_open_space` -- the channel log walks under the word of the open space | none | verified |
| `MURMURE-254` | `INV` | A key link of a public channel MUST count as verified when an admin of its space signed it. | this spec section 3; `SYNC-121`, `SRV-146` | `ambre_sync::membership_log::tests::a_channel_log_heeds_the_word_of_its_space` -- a link a space admin signed counts as verified; `ambre_sync::channels::tests::a_listed_meta_walks_under_the_word_of_its_open_space` | none | verified |
| `MURMURE-301` | `INV` | A space with no retention set MUST keep every segment. | this spec section 3 | not built | unbuilt -- lands with the `Space retention` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-302` | `OBS` | The interface MUST state the retention as a range: at least its duration, at most that plus the span of a segment. | this spec section 3 | not built | unbuilt -- lands with the `Space retention` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-583` | `OBS` | The sidebar MUST offer a signed-in user a control that creates a space, and the new space MUST show in the sidebar once created. | this spec section 4 | `ambre_murmure::chrome_relay::a_space_created_from_the_sidebar_shows_in_it` -- wasm tier, a blank name sends nothing, a name sends the creation and the space lists once its catalogue lands | none | verified |
| `MURMURE-584` | `OBS` | A space MUST offer its owner and its editors a control that creates a public channel in it, and the new channel MUST open once created. | this spec section 4 | `ambre_murmure::chrome_relay::an_admin_creates_a_public_channel_that_opens_once_created` -- wasm tier, an editor creates the channel last in the catalogue, a viewer is offered no control, and the created channel shows | none | verified |
| `MURMURE-585` | `OBS` | A space MUST offer its owner a control that invites another user into it. | this spec section 4 | `ambre_murmure::chrome_relay::the_owner_of_a_space_invites_another_user_into_it` -- wasm tier, the owner fetches the key of the address on the space document, an editor is offered no control | none | verified |

### 1.2 Create a space

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-035` | `POST` | Creating a space MUST make its creator the owner of the space document. | this spec section 3 | `ambre_server::routes::channels::tests::a_new_space_has_its_creator_as_its_one_owner` -- database tier; `ambre_logic::spaces::tests::a_new_space_shows_at_once_and_uploads_whole_once_its_session_is_ready` -- the client creates the document on the relay | none | verified |
| `MURMURE-036` | `FRAME` | Creating a space MUST NOT add a member other than its creator. | this spec section 3 | `ambre_server::routes::channels::tests::a_new_space_has_its_creator_as_its_one_owner` -- database tier, one member row; `ambre_murmure::logic::space_tests::a_new_space_is_one_document_that_holds_no_member` | none | verified |

### 1.3 List a public channel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-037` | `POST` | A listed channel MUST carry its name, its topic, its category, its position and its key. | this spec section 3 | `ambre_murmure::logic::space_tests::a_listed_channel_carries_its_name_topic_category_position_and_key`; `ambre_murmure::logic::space_tests::an_unset_topic_and_category_are_absent_keys`; `ambre_logic::spaces::tests::a_public_channel_is_listed_with_its_sealed_key_once_the_relay_holds_it` | none | verified |
| `MURMURE-038` | `FRAME` | Listing a channel MUST NOT change the membership of any channel. | this spec section 3 | `ambre_murmure::logic::space_tests::listing_a_channel_writes_the_space_document_alone`; `ambre_server::routes::channels::tests::an_editor_of_the_space_registers_a_public_channel` -- database tier, no member row moves | none | verified |

### 1.4 Join a public channel alone

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-039` | `POST` | A self-join MUST have the Sync worker unseal the key read from the catalogue and wrap it for the joining member alone. | this spec section 3; `SYNC-010` | `ambre_sync::channels::tests::a_self_join_unseals_the_catalogue_key_and_wraps_it_for_the_joiner_alone`; `ambre_logic::spaces::tests::a_self_join_hands_the_sealed_key_to_sync_and_writes_no_catalogue`; `ambre_server::routes::channels::tests::a_member_of_the_space_joins_a_public_channel_alone` -- database tier | none | verified |
| `MURMURE-040` | `POST` | A self-join MUST add an entry the joining member signs. | this spec section 3; `SYNC-106` | `ambre_server::routes::channels::tests::an_accepted_self_join_stores_the_entry_the_member_signed` -- database tier; `ambre_server::routes::channels::tests::a_self_join_whose_entry_claims_another_role_adds_no_member` | none | verified |
| `MURMURE-041` | `FAIL` | Given a self-join entry for a channel the catalogue does not list, every client MUST show the member as unverified. | this spec section 3; `SYNC-108` | `ambre_sync::sharing::tests::the_member_list_admits_a_self_join_only_in_a_marked_log`; `ambre_logic::channels::tests::a_channel_no_catalogue_lists_admits_no_self_join` | none | verified |
| `MURMURE-042` | `FRAME` | A self-join MUST NOT write the catalogue. | this spec section 3 | `ambre_logic::spaces::tests::a_self_join_hands_the_sealed_key_to_sync_and_writes_no_catalogue`; `ambre_server::routes::channels::tests::a_self_join_changes_no_member_of_the_space` -- database tier | none | verified |
| `MURMURE-635` | `FAIL` | Given a catalogue entry whose channel key no writer of the space signed, a member who joins the channel alone MUST NOT adopt that key. | audit `N16`; this spec section 3; `MURMURE-032` | not built | unbuilt -- lands with the `A channel document accepts no current key the relay chose` Story (`ambre-roadmap.md` E7). The entry is sealed under the space key and signed by no one, and a client applies a delta of any holder of a space key, so a relay that forwards a viewer's entry hands each self-joiner a key and an anchor the viewer chose | unimplemented |

### 1.5 Leave a space

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-043` | `POST` | A member leaving a space MUST cause the rotation of the space key and of the key of each public channel. | this spec section 3; `ambre-spec-key-rotation.md` | `ambre_sync::space_rotation::tests::a_space_debt_is_paid_before_any_channel`; `::the_entries_an_older_space_key_sealed_are_rotated_once`; `::a_new_catalogue_keeps_the_rotations_running`; `ambre_sync::sharing::tests::a_public_channel_is_rekeyed_under_the_current_space_key`; `ambre_logic::spaces::tests::an_admin_hands_the_sealed_catalogue_to_sync` -- the space key first, then each public channel | none | verified |
| `MURMURE-044` | `POST` | The catalogue MUST hold the rotated key of each public channel once the rotation completes. | this spec section 3 | `ambre_logic::spaces::tests::a_resealed_channel_writes_its_new_key_once`; `ambre_murmure::logic::space_tests::a_reseal_replaces_the_key_and_nothing_else` | none | verified |
| `MURMURE-045` | `FRAME` | Leaving a space MUST NOT change a channel of the catalogue beyond its key. | this spec section 3 | `ambre_murmure::logic::space_tests::a_reseal_replaces_the_key_and_nothing_else`; `ambre_logic::spaces::tests::a_resealed_channel_writes_its_new_key_once` -- name, topic, category and place stay | none | verified |
| `MURMURE-234` | `POST` | Leaving a space, or being removed from it, MUST remove the member from every channel of the space, private channels included. | this spec section 3; `SRV-136` | `ambre_server::routes::channels::tests::leaving_a_space_leaves_every_channel_of_it`; `::a_removal_from_a_space_leaves_every_channel_of_it` -- a leave and a removal by an owner, over a public, a shared private and a sole-member private channel | none | verified |
| `MURMURE-235` | `POST` | Each private channel the member leaves that way MUST owe a rotation. | this spec section 3; `SYNC-127` | `ambre_server::routes::channels::tests::leaving_a_space_leaves_every_channel_of_it` -- the shared private channel owes a rotation, the public one owes none | none | verified |
| `MURMURE-237` | `FRAME` | Leaving a space MUST NOT remove the member from a direct message or from a channel of another space. | this spec section 3 | `ambre_server::routes::channels::tests::leaving_a_space_keeps_direct_messages_and_other_spaces` -- a direct message and a channel of a second space keep the member | none | verified |
| `MURMURE-258` | `FAIL` | Given the last owner leaving a space that keeps other members, the client MUST first hand the owner role of the space document to one of them. | this spec section 3; `SRV-015` | `ambre_logic::spaces::tests::the_last_owner_hands_the_role_on_before_leaving`; `::a_leave_another_owner_covers_goes_at_once` -- the role passes before the leave, and a refusal fails the leave | none | verified |
| `MURMURE-479` | `FAIL` | Given a member who joined the space again, a removal of the space log signed before that join MUST NOT verify a channel removal. | this spec section 3; `MURMURE-236` | `ambre_sync::membership_log::tests::a_space_word_follows_its_latest_entries_and_adds_no_member` -- a removal, then a join of the same member, removes no one from the channel | none | verified |
| `MURMURE-480` | `FRAME` | The log of a space MUST NOT verify a member of one of its channels. | this spec section 3; `MURMURE-254` | `ambre_sync::membership_log::tests::a_space_word_follows_its_latest_entries_and_adds_no_member` -- an owner and an editor of the space stay unverified in a channel that holds no entry of theirs | none | verified |
| `MURMURE-487` | `POST` | Given the relay's notice that the user lost a space, by a leave, a removal or a delete, the client MUST close the space and tell the view once. | this spec section 3; `ambre-spec-server.md`, `SRV-218`; `MURMURE-234` | `ambre_logic::spaces::tests::a_space_the_user_lost_closes_and_the_view_learns_it` -- the space closes and the view learns it once, where another user's removal or a document no space holds changes nothing | none | verified |

### 1.6 Delete a public channel or a space

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-247` | `POST` | Deleting a public channel MUST remove its entry from the catalogue. | this spec section 3; `MURMURE-243` | `ambre_murmure::logic::space_tests::unlisting_a_channel_removes_its_entry_and_leaves_the_others` -- the entry and its sealed key go, a peer replaying the delta lists it no more; `ambre_logic::spaces::tests::a_deleted_channel_leaves_the_client_and_the_catalogue_once_the_relay_answers` -- the owner's client writes that delta once the relay answers | none | verified |
| `MURMURE-466` | `FAIL` | Given a catalogue entry whose channel the relay no longer holds, the client of a space admin MUST remove the entry. | this spec section 3; `MURMURE-247`, `SRV-189` | `ambre_logic::spaces::tests::an_admin_unlists_an_entry_whose_channel_the_relay_no_longer_holds` -- an entry the answer lacks goes, one listed since the ask stays, and a viewer asks nothing; `ambre_sync::channels::tests::a_listing_of_a_space_forwards_its_channels_and_a_failure_nothing` -- the answer reaches the Logic thread | none | verified |
| `MURMURE-248` | `POST` | Deleting a space MUST delete every channel of the space, then the space document. | this spec section 3; `SRV-143` | `ambre_server::routes::channels::tests::deleting_a_space_deletes_every_channel_then_the_space` -- every channel document is gone, then the space document | none | verified |
| `MURMURE-249` | `FAIL` | Given a space delete from a member who does not own the space document, the relay MUST refuse it. | this spec section 3; `SRV-015` | `ambre_server::routes::channels::tests::a_space_delete_from_anyone_but_its_owner_is_refused` -- an editor and a stranger are refused, and the space and its channels stay | none | verified |
| `MURMURE-250` | `FRAME` | Deleting a space MUST NOT delete a direct message. | this spec section 3; `MURMURE-046` | `ambre_server::routes::channels::tests::deleting_a_space_deletes_every_channel_then_the_space` -- a direct message between the same members stays | none | verified |

### 1.7 Set the retention of a space

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-303` | `POST` | Setting the retention MUST be the act of a space admin, recorded on the relay for the whole space. | this spec section 3; `SRV-111`, `ZK-015` | not built | unbuilt -- lands with the `Space retention` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-304` | `POST` | A client that finds an expired segment MUST remove it from the `segments` list, and show where the history now starts. | this spec section 3; `MURMURE-219` | not built | unbuilt -- lands with the `Space retention` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-317` | `FRAME` | Setting the retention MUST NOT change a key or a membership. | this spec section 3 | not built | unbuilt -- lands with the `Space retention` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** a leave costs one rotation per public channel, bounded by the space leave row of
  `ambre-spec-budgets.md section 1`.
* **Boundary.** the catalogue and every key in it are sealed with the space document (`ZK-001`).
  The relay holds the public flag and the space of each channel (`ZK-015`).
* **Verification.** 32 `verified`, 6 `unimplemented`. The engine, the Logic thread, the Sync worker
  and the relay's database tier hold a new space, the catalogue and a signed self-join. The relay's
  database tier holds the leave and the delete, and the wasm tier the controls of the sidebar. The
  Sync worker and the Logic thread hold the key rotation on a leave. The guest and the retention are
  unbuilt.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The space holds | Each user sees | Test |
|---|---|---|---|---|
| lists a public channel | is a member of the space | the channel in the catalogue | Bob sees a channel to join | none |
| is offline | joins the public channel alone | a new member row, signed by Bob | Bob reads the channel at once (`MURMURE-039`) | none |
| -- | the relay flips a private channel to public and adds Mallory | a row the catalogue does not back | Mallory listed as unverified (`MURMURE-041`) | none |
| invites Carol into one channel as a guest | -- | Carol in that channel, not in the space | Carol sees that channel alone (`MURMURE-034`) | none |
| leaves the space | stays | rotated keys in the catalogue | Bob reads on, and Alice reads nothing new (`MURMURE-043`) | none |

---

## 3. MODEL

**CRDT containers.** the `space` document of `ambre-spec-murmure-schema.md section 3`: `space`,
`categories` and `channels`.

**Who writes the catalogue.** A space admin is an owner or an editor of the space document. A
member with the viewer role reads it, and the relay refuses a write from them (`SRV-008`).

**An entry can outlive its channel.** The owner's client unlists a deleted channel only once the
relay answers (`MURMURE-247`). A closed tab loses that write, and so does an owner who is a viewer
of the space. So each time the space goes live, a space admin's client asks the relay which
channels it holds (`SRV-189`). It removes an entry listed at the ask and absent from the answer
(`MURMURE-466`).

**A self-join, in steps.**

1. The Logic thread reads the sealed key of the channel in the catalogue, a `SealedChannelKey`, and
   hands it to the Sync worker. No key material reaches the Logic thread (`MURMURE-157`).
2. The Sync worker unseals it with the space key, wraps it for its own exchange key (`SYNC-010`),
   and signs a join entry (`SYNC-106`). It reads the head of the log from the relay first, since a
   member-to-be cannot read the log itself.
3. The relay creates the row only for a member of the space and a channel it holds as public
   (`SRV-105`, `SRV-106`). It stores the signed entry beside the row (`SRV-107`).
4. Every client checks the entry against the catalogue, never against the relay flag. The Logic
   thread tells the Sync worker whether a catalogue it holds lists the channel (`MURMURE-041`).
5. The joining client walks the key chain back from the key of the catalogue (`SYNC-119`). It
   reads the whole history of the channel, before a rotation too.

**A leave takes every channel of the space with it** (`MURMURE-234`). The relay removes the member
from each channel of the space, and each private one owes a rotation (`SRV-136`). The removal from
the space, signed in its log, is what verifies each channel removal (`MURMURE-236`). A guest is
never a member of the space, so a guest's channels are not touched. A space leave skips the
last-owner lockout on each channel: the member leaves the space, so no channel may keep them.

**A self-join re-checks the space.** The join takes a share lock on the space document, then reads
the membership again. A leave that commits first therefore refuses the join, and never races it.

**A space delete removes its channels first.** Each channel goes with its documents and its members
are told. The space document goes last, so a failure halfway leaves a space to delete again.

**Retention is one setting of the space.** A space admin sets one duration for every channel of the
space (`MURMURE-303`). The relay applies it: a policy in a client would erase nothing while no admin
is online, and the erasure promise would be false. A final segment older than the duration goes,
with its index, its amendments, its thread documents and its files (`SRV-111`, `SRV-156`). A message
therefore stays at least the duration, and at most the duration plus the span of its segment. The
interface states that range, never the duration alone (`MURMURE-302`).

**The keys, on a leave.** An admin need not be a member of a public channel to re-key it
(`SRV-146`). The link it signs is verified by its role in the space, not in the channel
(`MURMURE-254`). The first admin client online after the leave rotates the space key, then the key of
each public channel. Its Sync worker seals each new key under the new space key, and the catalogue
takes the sealed value.

**The space log speaks from its latest entries.** A removal counts only while no later entry adds
the member back (`MURMURE-479`). Its writers count at any point of its history. A link seals a key
its signer already held, and the relay takes a rotation from a current admin alone. The space log
adds link signers and removals, and never a member of a channel (`MURMURE-480`).

**A sealed entry names its space key.** It starts with the id of the space key that sealed it. An
admin therefore re-keys exactly the channels whose entry names an older space key. A last owner who
leaves first hands the owner role to the first editor by id, or else the first viewer
(`MURMURE-258`).

**Code.** `ambre-murmure` holds the space document and its catalogue. `ambre-logic` opens a space
and lists a channel. `ambre-sync` seals, unseals and wraps the keys. The relay registers a channel
and accepts a self-join.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| create a space, from the sidebar | a new space, with its creator as owner, listed in the sidebar |
| create a public channel, from the space in the sidebar | an owner or an editor lists a new channel, which opens at once |
| invite a user into a space, from the space in the sidebar | the owner names an email, and the user joins the space document once the key check passes |
| the catalogue of a space | the public channels, grouped by category, joined ones marked |
| join on a public channel | the channel opens, and joins the sidebar |
| invite a guest into a channel | the guest reaches that channel alone |
| leave a space | the space leaves the sidebar, and its keys rotate |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A self-join is checked against the catalogue | trusting the public flag of the relay | the relay could flip a private channel to public and show a verified member |
| A guest stays out of the space document | letting a guest read the catalogue | the catalogue holds every public key, so the guest would read every public channel |
| One key per public channel | one key for every public channel of a space | a guest or a bot of one channel would read them all |
| The catalogue lists public channels only | private channels in the catalogue | every member of the space reads the catalogue, so it would show each private channel |
| The catalogue names a public channel | the name in the channel `meta` document | a member who has not joined holds no key for `meta`, so the channel to join would have no name |
| The catalogue carries the sealed key | a member online to wrap the key | a join would wait on another member, and a space leave pays for the catalogue with a rotation |
| A space leave takes its private channels | keeping them, and offering to leave | a member an admin removed from the space would keep reading its private channels |
| The relay applies retention | a client applies it | no erasure happens while no admin is online |
| One retention per space | one per channel | two settings for one space, and a member cannot tell which one holds |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Built.** A new space, the catalogue, the public channel it lists, the signed self-join, the
  leave with its key rotation, and the delete of a space. The sidebar creates a space and a public
  channel, and invites a user into a space.
* **Not built.** The guest and the retention.
