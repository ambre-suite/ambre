# AMBRE -- FEATURE: PRIVATE CHANNELS AND DIRECT MESSAGES
**Type:** Vertical (feature)
**Implementation:** partial -- a private channel is created, left and deleted. The relay keeps the
key links of a channel under its `meta` document. A direct message is opened, reused and
named. An invite opens a channel.

> A few people talk where nobody else reads: a private channel of a space, or a direct message
> between two or more members. Opening a direct message with the same people finds the one that
> exists.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-046` | `INV` | A direct message MUST belong to no space. | this spec section 3; `ambre-spec-murmure-schema.md section 3` | `ambre_server::routes::channels::tests::a_direct_message_has_no_space_and_no_owner` -- the relay holds the channel under no space | none | verified |
| `MURMURE-047` | `INV` | The relay MUST NOT create a direct message for a set of members that already has one, concurrent requests included. | this spec section 3; `SRV-176` | `ambre_server::routes::channels::tests::racing_requests_create_one_direct_message` -- two racing requests create one; `ambre_server::routes::channels::tests::the_same_members_reach_the_same_direct_message` -- a later request from either side creates none | none | verified |
| `MURMURE-252` | `INV` | A direct message MUST have no owner. | this spec section 3; `SRV-015` | `ambre_server::routes::channels::tests::a_direct_message_has_no_space_and_no_owner` -- every member row is an editor; `ambre_server::routes::channels::tests::a_direct_message_with_an_owner_is_refused` -- a log that keeps an owner is refused | none | verified |
| `MURMURE-264` | `INV` | The relay MUST keep the key links of a channel under its `meta` document, where they serve every document of the channel. | this spec section 3; `SYNC-117` | `ambre_server::routes::channels::tests::a_channel_link_is_kept_under_meta_and_served_for_a_segment` -- database tier | none | verified |
| `MURMURE-333` | `OBS` | An archived channel MUST show as archived, with no composer. | this spec section 3 | not built | unbuilt -- lands with the `Archived channels` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Create a private channel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-048` | `POST` | Creating a private channel MUST make its creator its owner and its only member. | this spec section 3 | `ambre_server::routes::channels::tests::a_private_channel_has_its_creator_alone_and_names_its_space` -- the relay holds the creator as the one member and owner | none | verified |
| `MURMURE-049` | `POST` | A private channel MUST name its space to the relay. | this spec section 3; `ZK-015` | `ambre_server::routes::channels::tests::a_private_channel_has_its_creator_alone_and_names_its_space` -- the relay holds the channel in its space, not public | none | verified |
| `MURMURE-050` | `FRAME` | Creating a private channel MUST NOT write the catalogue of its space. | this spec section 3; `MURMURE-014` | `ambre_logic::spaces::tests::a_private_channel_writes_no_catalogue` -- no update reaches the space document | none | verified |

### 1.3 Open a direct message

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-051` | `POST` | Opening a direct message with a set of members MUST open the existing one when it exists. | this spec section 3 | `ambre_server::routes::channels::tests::the_same_members_reach_the_same_direct_message` -- the relay answers the existing channel; `ambre_logic::direct::tests::an_existing_direct_message_opens_in_place_of_the_new_one` -- the client opens it | none | verified |
| `MURMURE-052` | `POST` | A new direct message MUST wrap its key for each of its members alone. | this spec section 3; `SYNC-010` | `ambre_sync::channels::tests::a_direct_message_makes_every_member_an_editor_under_one_key` -- each member opens one key from their own wrap; `ambre_server::routes::channels::tests::each_member_holds_the_key_wrapped_for_them` -- the relay stores each wrap on its member's row | none | verified |
| `MURMURE-464` | `FRAME` | A new direct message MUST NOT wrap its key for a member under an exchange key the relay changed since the member was chosen. | this spec section 3; `SYNC-010` | `ambre_sync::channels::tests::a_key_other_than_the_pinned_one_is_refused` -- a key the relay serves differently fails the request | none | verified |
| `MURMURE-251` | `POST` | A new direct message MUST give every member, its creator included, the editor role. | this spec section 3; `MURMURE-252` | `ambre_sync::channels::tests::a_direct_message_makes_every_member_an_editor_under_one_key` -- meta's log replays to every member as an editor; `ambre_server::routes::channels::tests::a_direct_message_has_no_space_and_no_owner` -- the relay holds an editor row for each | none | verified |
| `MURMURE-053` | `FRAME` | Opening a direct message MUST NOT add a member to an existing one. | this spec section 3 | `ambre_server::routes::channels::tests::the_same_members_reach_the_same_direct_message` -- the existing channel keeps its members; `ambre_logic::direct::tests::an_existing_direct_message_opens_in_place_of_the_new_one` -- nothing is sent to it | none | verified |

### 1.4 Name a direct message

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-054` | `POST` | A name a member sets MUST be stored in the `meta` document, and read by every member. | this spec section 3 | `ambre_murmure::channel_tests::a_name_one_member_sets_reaches_every_member` -- another member reads it; `ambre_logic::direct::tests::a_name_leaves_on_meta_and_reaches_the_view` -- the name leaves as a delta on `meta` | none | verified |
| `MURMURE-579` | `FAIL` | Given a channel, a direct message, a space or a category named past the budget of a name, the client MUST write nothing and tell the user why. | `ambre-spec-budgets.md section 6`; `SYNC-178` | `ambre_murmure::channel_tests::a_channel_takes_the_longest_name` -- a public and a private channel, the longest name taken, and the engine takes a name only as a `Validated<Name>`, which no text past the cap makes; type-level -- every bus command that names a space, a category or a channel carries a `Validated<Name>`, which no text past the cap makes, so none reaches the Logic thread; `ambre_logic::direct::tests::a_name_leaves_on_meta_and_reaches_the_view` -- a direct message too, and the name sealed is the trimmed one `meta` holds | none | verified |
| `MURMURE-055` | `FRAME` | Clearing a name MUST remove its key, and MUST NOT store an empty or a composed name. | this spec section 3; `MURMURE-016` | `ambre_murmure::channel_tests::clearing_a_name_removes_its_key` -- a cleared or blank name leaves no `name` key | none | verified |
| `MURMURE-491` | `POST` | Every name written into a channel's `meta` MUST also be sealed beside it on the relay, under the channel key. | this spec section 3; `features/ambre-spec-murmure-layout.md`, `MURMURE-490`; `ambre-spec-sync.md`, `SYNC-171` | `ambre_logic::direct::tests::a_name_leaves_on_meta_and_reaches_the_view` -- each name, and its clearing, also leaves to be sealed beside meta; `ambre_logic::spaces::tests::a_private_channel_writes_no_catalogue` -- a private channel's creation carries its name; `ambre_sync::channels::tests::a_channel_name_is_sealed_under_the_channel_key_for_its_meta` -- the seal opens under that key for meta alone | none | verified |

### 1.5 Invite a member

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-056` | `POST` | An invite MUST open the history since the latest history boundary to the invitee, through the key chain. | this spec section 3; `SYNC-119`; `ambre-spec-server.md`, `SRV-190` | `ambre_server::routes::channels::tests::an_invite_on_meta_opens_every_document_of_the_channel` -- the invite on `meta` opens the segment to the invitee, with the links kept under `meta`, database tier; `ambre_sync::sharing::tests::an_invite_posts_its_signed_entry_beside_the_copy` -- the copy wraps the current key and the entry names it; `ambre_sync::updates::tests::an_open_walks_the_chain_to_a_snapshot_older_than_the_cache` -- an open walks back from it | none | verified |
| `MURMURE-057` | `POST` | A third member joining a direct message of two MUST be announced to both members before the invite applies. | this spec section 3; `ambre-spec-membership.md`, `SYNC-161` | `ambre_logic::direct::tests::an_invite_into_a_direct_message_of_two_leaves_behind_its_notice` -- the invite waits for the active segment, then leaves after the notice that names the invitee; `::an_invite_into_a_larger_direct_message_writes_no_notice`; `::an_invite_whose_notice_cannot_be_written_is_refused` -- an invite whose notice cannot be written never leaves; `ambre_murmure::channel_tests::a_notice_names_the_member_who_joins_for_every_member` -- the other member reads it through the authorship check; `::a_notice_of_a_kind_no_build_wrote_is_skipped` | none | verified |
| `MURMURE-058` | `FRAME` | An invite MUST NOT hand the invitee a key older than the latest history boundary. | this spec section 3 | not built | unbuilt -- lands with the `Direct message conversion` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-498` | `POST` | A member MUST invite another into a channel from that channel while it is open. | this spec section 3; `MURMURE-057` | `ambre_app::shell::murmure_regions::the_invite_control_offers_the_other_members_and_fetches_a_key` -- a private channel offers the other members, and a choice fetches the key; `ambre_murmure::tests::chrome_relay::an_invite_fetches_the_key_for_the_shown_channel_and_is_taken_once` -- the key goes to the channel shown when asked; `ambre_app::tofu::tests::a_trusted_key_invites_the_member_into_the_channel` -- the invite leaves past the pin only; `ambre_logic::direct::tests::an_invite_from_the_channel_leaves_on_its_meta_after_its_notice` -- it leaves on `meta`, after the notice | none | verified |
| `MURMURE-501` | `FAIL` | Given a member the open channel lists, the invite control MUST NOT offer them, nor fetch their key. | this spec section 3; `MURMURE-500` | `ambre_murmure::tests::chrome_relay::the_invite_control_offers_no_member_of_the_channel` -- a listed member leaves the list and gets no key fetch, and a channel left forgets its list | none | verified |
| `MURMURE-500` | `FAIL` | Given an invite of a member the open channel lists, the client MUST refuse it and send nothing. | this spec section 3; `MURMURE-498` | `ambre_logic::direct::tests::an_invite_of_a_listed_member_is_refused_and_nothing_leaves` -- a listed member and a channel not open are each refused, and no command reaches Sync | none | verified |

### 1.6 Leave a channel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-059` | `POST` | Leaving a private channel or a direct message MUST cause the rotation of its key. | this spec section 3; `ambre-spec-key-rotation.md` | `ambre_server::routes::channels::tests::a_leave_owes_a_rotation_on_a_private_channel_only` -- the relay marks the key of the private channel owed | none | verified |
| `MURMURE-060` | `POST` | A rotation MUST close the active segment, so the successor opens under the new key. | this spec section 3; `features/ambre-spec-murmure-segments.md` | `ambre_murmure::closing_tests::a_rotation_closes_the_active_segment_whatever_it_holds` -- one message below the size asks the close, again after a refusal, and the successor asks none; `ambre_logic::channels::tests::a_rotation_of_meta_closes_the_active_segment_at_the_next_tick` -- the close leaves at the tick, and a rotation of another document asks none; `ambre_sync::updates::tests::each_rotation_this_client_makes_reaches_logic` -- a revoke and an owed rotation each reach Logic | none | verified |
| `MURMURE-262` | `POST` | A rotation MUST lock every segment sealed under the previous key, so a later edit goes to an amendment under the new key. | this spec section 3; `SRV-147` | `ambre_server::routes::channels::tests::a_rotation_locks_the_closing_segment_at_once` -- database tier, the relay locks the closing segment at the rotation; `::a_rotation_locks_the_active_segment_at_its_close` -- and the active one at its close; `ambre_murmure::amending_tests::a_change_of_a_locked_segment_waits_for_its_amendments` -- a later change goes to the amendments | none | verified |
| `MURMURE-461` | `POST` | The last member leaving a public channel MUST leave it listed, with its history, for the next member to join. | this spec section 3; `SRV-173` | `ambre_server::routes::channels::tests::the_last_member_leaves_a_public_channel_for_the_next_to_join` -- the relay keeps the channel public with its documents, and the next member of the space joins it; `ambre_sync::membership_log::tests::a_log_its_last_member_left_admits_the_next_self_join` -- that member's client verifies the join on a log nobody holds | none | verified |
| `MURMURE-467` | `POST` | Leaving a channel MUST drop it from the client of the member who leaves, with its documents and its keys. | this spec section 3; `MURMURE-059` | `ambre_logic::spaces::tests::a_left_channel_leaves_the_client_once_the_relay_answers` -- once the relay answers, the channel goes, the session of each of its documents ends, and the catalogue keeps it; `ambre_logic::spaces::tests::a_refused_leave_keeps_the_channel_whole` -- a refusal keeps it; `ambre_sync::sharing::tests::leaving_a_channel_posts_this_users_removal_on_its_meta` -- the leave is a signed removal on `meta`, with no rotation; `ambre_sync::updates::tests::closing_a_document_forgets_its_keys` -- an ended session takes its keys with it | none | verified |
| `MURMURE-468` | `POST` | Leaving a channel MUST drop it from every other client of the member who leaves. | this spec section 3; `MURMURE-467`; `ambre-spec-server.md`, `SRV-187` | `ambre_logic::spaces::tests::a_leave_from_another_client_drops_the_channel_here` -- the relay's notice to this user ends each session of the channel and tells the view, where a removal of someone else or of a segment drops nothing. A leave asked here ends on the notice, and an answer lost after it shows no failure | none | verified |
| `MURMURE-469` | `FAIL` | Given a channel whose open the relay refuses for want of membership, the client MUST drop it. | this spec section 3; `SRV-047`; `ambre-spec-server.md`, `SRV-103` | `ambre_logic::channels::tests::a_channel_the_relay_refuses_to_open_is_dropped` -- the opening ends and the view learns the channel is gone, not that it failed; `ambre_sync::channels::tests::a_listing_refused_for_want_of_membership_reads_as_refused` -- only a 403 with the forbidden code reads so; `ambre_sync::http::tests::only_a_forbidden_refusal_reads_as_one` | none | verified |
| `MURMURE-061` | `FRAME` | Leaving a public channel MUST NOT rotate its key. | this spec section 3 | `ambre_server::routes::channels::tests::a_leave_owes_a_rotation_on_a_private_channel_only` -- the public channel owes no rotation after the leave | none | verified |
| `MURMURE-257` | `FAIL` | Given the last owner leaving a private channel that keeps other members, the relay MUST refuse the leave. | this spec section 3; `SRV-015` | `ambre_server::routes::channels::tests::the_last_owner_hands_the_role_on_before_leaving` -- refused and kept, then let through once the role is handed on | none | verified |

### 1.7 Delete a channel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-243` | `POST` | Deleting a channel MUST delete every document it groups, with their objects. | this spec section 3; `SRV-142`, `SRV-057` | `ambre_server::routes::channels::tests::deleting_a_channel_deletes_its_documents_and_their_objects` -- no document and no object of the channel is left after the purge | none | verified |
| `MURMURE-244` | `FAIL` | Given a delete from a member who owns neither the channel nor, for a public channel, its space, the relay MUST refuse it. | this spec section 3; `SRV-015`, `SRV-174` | `ambre_server::routes::channels::tests::a_delete_from_anyone_but_an_owner_is_refused` -- an editor of the space, a stranger and an unknown channel get a 403, and nothing goes; `ambre_server::routes::channels::tests::an_owner_of_the_space_deletes_a_public_channel_its_last_owner_left` -- an owner of the space is refused a private channel, database tier | none | verified |
| `MURMURE-245` | `POST` | A private channel or a direct message whose last member leaves MUST be deleted. | this spec section 3 | `ambre_server::routes::channels::tests::the_last_member_leaving_a_private_channel_deletes_it` -- the channel and its documents go with the last leave | none | verified |
| `MURMURE-462` | `POST` | Deleting a channel MUST drop it from the client of its owner, with its documents and its keys. | this spec section 3; `MURMURE-243` | `ambre_logic::spaces::tests::a_deleted_channel_leaves_the_client_and_the_catalogue_once_the_relay_answers` -- once the relay answers, the channel goes and the session of each of its documents ends; `ambre_sync::updates::tests::closing_a_document_forgets_its_keys` -- an ended session takes its keys with it | none | verified |
| `MURMURE-463` | `FAIL` | Given a delete the relay refuses, the client MUST keep the channel as it was. | this spec section 3; `MURMURE-244` | `ambre_logic::spaces::tests::a_refused_delete_keeps_the_channel_whole` -- the view is told, and the channel, its sessions and its catalogue entry stay; `ambre_sync::channels::tests::a_delete_answers_logic_and_leaves_the_sessions_to_it` -- the worker closes no session of its own | none | verified |
| `MURMURE-465` | `POST` | Deleting a channel MUST drop it from the client of every other member that holds it. | this spec section 3; `MURMURE-462`; `ambre-spec-server.md`, `SRV-188` | `ambre_logic::spaces::tests::a_delete_ends_on_the_relays_notice` -- the relay's notice drops a channel another member deleted, and tells the view. A delete asked here ends on it as on its answer | none | verified |
| `MURMURE-246` | `FRAME` | Deleting a channel MUST NOT touch another channel. | this spec section 3 | `ambre_server::routes::channels::tests::deleting_a_channel_deletes_its_documents_and_their_objects` -- the other channel keeps its documents and objects | none | verified |

### 1.8 Convert a direct message into a private channel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-323` | `POST` | Converting a direct message MUST make it a private channel of the chosen space, keeping its identifier and its segments. | this spec section 3; `SRV-159` | not built | unbuilt -- lands with the `Direct message conversion` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-324` | `FAIL` | Given a conversion by a member who does not belong to the chosen space, the relay MUST refuse it. | this spec section 3 | not built | unbuilt -- lands with the `Direct message conversion` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-325` | `POST` | A conversion MUST warn every member, naming the space and the history choice, before it applies. | this spec section 3 | not built | unbuilt -- lands with the `Direct message conversion` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-326` | `POST` | A conversion that starts fresh MUST rotate the key with no link, so a later member reads nothing before it. | this spec section 3; `SYNC-116` | not built | unbuilt -- lands with the `Direct message conversion` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-329` | `POST` | A conversion MUST make the converting member the owner of the channel. | this spec section 3; `MURMURE-252` | not built | unbuilt -- lands with the `Direct message conversion` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-327` | `FRAME` | Converting a direct message MUST NOT remove any of its members. | this spec section 3 | not built | unbuilt -- lands with the `Direct message conversion` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.9 Invite without history

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-328` | `POST` | An invite into a private channel MUST offer to start the invitee after a new history boundary. | this spec section 3; `MURMURE-326` | not built | unbuilt -- lands with the `Direct message conversion` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-335` | `FRAME` | An invite without history MUST NOT change the keys the current members hold. | this spec section 3 | not built | unbuilt -- lands with the `Direct message conversion` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.10 Archive a channel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-330` | `POST` | Archiving a channel MUST keep it readable, and make the relay refuse every write to it. | this spec section 3; `SRV-160` | not built | unbuilt -- lands with the `Archived channels` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-331` | `FAIL` | Given an archive from a member who does not own the channel, the relay MUST refuse it. | this spec section 3; `SRV-015` | not built | unbuilt -- lands with the `Archived channels` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-332` | `POST` | Unarchiving a channel MUST let it take writes again. | this spec section 3 | not built | unbuilt -- lands with the `Archived channels` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-334` | `FRAME` | Archiving a channel MUST NOT change its keys or its members. | this spec section 3 | not built | unbuilt -- lands with the `Archived channels` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.11 Learn of a channel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-473` | `POST` | Asked for its user's channels, the client MUST learn every channel its user belongs to, private channels and direct messages included, from the relay's listing. | this spec section 3; `ambre-spec-server.md`, `SRV-191` | `ambre_logic::channels::tests::the_users_channels_reach_the_view_and_open_nothing` -- the view's ask reaches Sync and the answer reaches the view whole; `ambre_sync::channels::tests::a_listing_of_the_users_channels_forwards_their_places` -- the relay's listing decodes to each channel's place, and an overtaken answer hands nothing on | none | verified |
| `MURMURE-474` | `POST` | A channel its user is added to, by an invite or a new direct message, MUST reach every open client of that user with no reload. | this spec section 3; `ambre-spec-server.md`, `SRV-192` | `ambre_sync::channels::tests::the_share_of_a_murmure_document_lists_the_users_channels` -- the relay's share of a `meta` lists the channels again, and no other share does | none | verified |
| `MURMURE-475` | `FRAME` | Learning of a channel MUST NOT open it, nor fetch any of its documents. | this spec section 3; `MURMURE-473` | `ambre_logic::channels::tests::the_users_channels_reach_the_view_and_open_nothing` -- the answer sends nothing to Sync and holds no channel state | none | verified |

* **Budget.** an invite wraps one key. The invitee then walks the key chain back to the boundary,
  one small open per rotation, charged to the crypto rows of `ambre-spec-budgets.md section 1`. A
  rotation costs one close and one wrap per remaining member.
* **Boundary.** the relay learns the members of each channel, which `ambre-spec-zero-knowledge.md`
  accepts as the membership graph. The name of a direct message is sealed in `meta`.
* **Verification.** 40 `verified`, 14 `unimplemented`. The relay holds a private channel, its leave,
  its delete and its key links. A client creates, deletes and leaves one, and learns its user's
  channels. A direct message opens, is reused and takes a name; an invite opens a channel, and
  announces a third member. A rotation closes the active segment and locks the ones it leaves;
  conversion and archive are not built.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The channel holds | Each user sees | Test |
|---|---|---|---|---|
| opens a direct message with Bob | opens a direct message with Alice | one direct message | the same conversation on both screens (`MURMURE-051`) | `ambre_server::routes::channels::tests::the_same_members_reach_the_same_direct_message` |
| names the direct message "Budget" | -- | the name in `meta` | "Budget" on both screens (`MURMURE-054`) | `ambre_murmure::channel_tests::a_name_one_member_sets_reaches_every_member` |
| invites Carol into the direct message | -- | a notice, then Carol as a member | both told before Carol joins (`MURMURE-057`) | `ambre_logic::direct::tests::an_invite_into_a_direct_message_of_two_leaves_behind_its_notice` |
| leaves a private channel | writes a message | a new key, and a new active segment | Bob's message unreadable to Alice (`MURMURE-059`) | `ambre_server::routes::channels::tests::a_leave_owes_a_rotation_on_a_private_channel_only`, the rotation owed; `ambre_logic::channels::tests::a_rotation_of_meta_closes_the_active_segment_at_the_next_tick`, the new segment |

---

## 3. MODEL

**CRDT containers.** the `meta` document of `ambre-spec-murmure-schema.md section 3`: its
`channel` map holds the type, the optional name and the channel topic, a `Validated<ChannelTopic>`. A channel
topic is the text a member types, never the relay topic that `ambre-spec-broadcast.md` names.

**A direct message is reused.** The relay holds the members of every channel, so it finds a direct
message whose members are exactly the requested set. It learns nothing new by doing so
(`MURMURE-051`). A request carries the whole new channel: both documents, the key wrapped for each
member, and the log of `meta`. The relay creates it, or answers the existing one and writes
nothing (`MURMURE-053`). One lock per set of members serializes two racing requests, so the second
finds what the first made (`MURMURE-047`, `SRV-176`).

**A direct message, in steps.**

1. The view names the members, each with the exchange key it showed for them.
2. The Sync worker fetches each member's keys from the relay again. An exchange key that changed
   since the view showed it fails the request, and no wrap leaves (`MURMURE-464`). The signing
   key is the one each member's invite names.
3. It draws one key and signs the log of `meta`. The log holds its creation, one editor invite per
   member, then its own role lowered to editor (`MURMURE-251`). No owner is left (`MURMURE-252`).
4. It wraps the key for each member, the log's root inside (`MURMURE-052`).
5. The relay checks the log, and stores the channel with an editor row per member in one
   transaction (`SRV-175`). The segment opens no log: its access reads the rows of `meta`.

**A name is one key.** A member sets the name of a direct message in the `channel` map of `meta`,
and every member reads it from there (`MURMURE-054`). Clearing it removes the key, and a name
composed from the members for display is never stored (`MURMURE-055`, `MURMURE-016`).

**One chain per channel.** Every document of a channel shares its key. So the relay keeps its key
links once, under the `meta` document, and they open any segment (`MURMURE-264`). A link is never
document content: no key reaches the Logic thread.

**No forward secrecy.** A direct message keeps every key, like any channel. Each final segment stays
sealed under the key of its time, and the key chain reaches it. What protects it is the rotation on
a leave.

**An invite, in steps.**

1. The inviting client wraps the current key for the invitee, and its signed entry names that key
   (`SYNC-123`).
2. It posts the wrap and the entry in one request on `meta`, whose rows open every document of the
   channel. The relay refuses an invite on any other document of it (`SRV-190`, `SYNC-105`).
3. The invitee walks the key chain back to the latest history boundary (`SYNC-119`), so it reads
   every segment since.
4. For a direct message of two, both members see the notice before the invite applies.

**The notice of a third member.** An empty message naming who joins, written before the invite
leaves on the same queue. An invite waits while the segment cannot be written. Unlisted members
are announced too: a notice too many is harmless. An invite is asked from the open channel, whose
active segment takes the notice (`MURMURE-498`).

**The invite control.** A private channel or a direct message shows it. It lists the members of the
channel's space, or of every open space for a direct message, less the channel's own members. A
choice fetches the key by member id, so the invite needs no email. The key passes the pin check.

**A member learns of a channel from the relay.** The membership is the channel list, so the relay
lists each channel of a user when the client asks (`MURMURE-473`). An invite or a new direct message
reaches every connection of the member it adds as the share of its `meta`. The client then lists
its channels again (`MURMURE-474`).

**A leave, in steps.** The leaving member adds a signed entry, and the relay removes the row and
marks the rotation owed (`SYNC-127`). The first member client to send rotates the key and closes
the active segment (`SYNC-128`, `MURMURE-060`). The rotation also locks the closing segment, so an
edit written after it never travels under the key the member who left still holds (`MURMURE-262`).

**The leaver's client drops the channel.** It posts the entry alone and picks no key, since the
members who stay would otherwise trust a key the leaver chose (`SYNC-130`). Once the relay answers,
the client drops the channel, its documents and its keys (`MURMURE-467`). A refusal keeps it whole.
The catalogue keeps a public channel listed, for the next member to join (`MURMURE-461`).

**Every client of the leaver drops it.** The relay tells each of them, even when the leave deleted
the channel (`ambre-spec-server.md`, `SRV-187`). Each other client drops it the same way, and tells
the view (`MURMURE-468`). A leave asked here ends on that notice too, so an answer lost after it
keeps nothing.

**A refused open drops the channel.** A leave or a delete whose answer was lost leaves a channel
the user no longer belongs to. Its next open is refused, and the client drops it (`MURMURE-469`).
An open channel whose notice went while offline goes on the reopen (`ambre-spec-sync.md`,
`SYNC-160`).

**A leave signs on the head the relay serves.** The leaver signs after the last entry served, valid
or not, as every author does. A relay that serves a false head can only make the leave fail, which
it could do by refusing it outright.

**A delete is final.** The owner deletes a channel, and every document, file, member row and cursor
goes with it (`MURMURE-243`). The last owner of a private channel hands the role on before leaving
while other members stay (`MURMURE-257`). The last member of a private
channel leaves in the same transaction that deletes it, so no channel is left without a member.

**Each client drops a deleted channel.** The owner's client drops the channel, its documents and
its keys once the relay answers, and keeps it whole on a refusal (`MURMURE-462`, `MURMURE-463`). A
public one leaves the catalogue with it (`MURMURE-247`). Every other member's client drops it too,
on the relay's word to each member (`ambre-spec-server.md`, `SRV-188`, `MURMURE-465`).

**A direct message has no owner.** Every member is an editor (`MURMURE-252`), so nobody deletes it
for the others. It goes once its last member leaves, as a private channel does (`MURMURE-245`). A
public channel needs no owner either: the space reads its key in the catalogue. Its last owner
leaves while others stay, and an owner of the space deletes it (`SRV-174`).

**An archive closes a channel and keeps it.** The owner archives it: every member still reads it,
and the relay refuses every write (`MURMURE-330`, `SRV-160`). Unarchiving opens it again.

**A direct message becomes a private channel.** A member of the direct message who belongs to the
space converts it, in one act (`MURMURE-323`). The identifier and the segments stay, and the
converting member becomes the owner (`MURMURE-329`). Every member is warned first, with the space
and the history choice (`MURMURE-325`). A member who is not in the space stays in the channel
(`MURMURE-327`).

**Keep, or start fresh.** Keep is the default: the key stays, and a later member reads the whole
history. Start fresh rotates the key and leaves no link, so the chain stops there
(`MURMURE-326`). The members of the direct message keep the older keys. The boundary protects the
history from later members only: a member can still copy the text. Any invite into a private
channel can use the same boundary, as an invite without history (`MURMURE-328`).

A direct message becomes private, never public: making it public is a second act, by a space admin,
who writes its key into the catalogue.

**Code.**

* `ambre-server/src/routes/channels.rs` -- the `public` flag at creation, the owner's delete, and
  the direct message route with the log it checks.
* `ambre-server/src/routes/share.rs` -- an invite into a channel, taken on its `meta` alone.
* `ambre-server/src/db/channels.rs` -- deleting a channel with every document it groups, and
  finding or creating a direct message under its lock.
* `ambre-server/src/db/members.rs` -- a leave: the owed rotation, and the delete of an emptied
  private channel.
* `ambre-logic/src/spaces.rs` -- creating a private channel, with no catalogue write; the owner's
  delete, which drops the channel and its catalogue entry once the relay answers.
* `ambre-logic/src/direct.rs` -- a direct message request, and the created or existing channel it
  opens.
* `ambre-murmure/src/channel.rs` -- the name a member sets on a direct message.
* `ambre-sync/src/channels.rs` -- registering a channel, with a sealed key for a public one only.
  It also holds the log, the wraps and the key check of a direct message.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| new private channel | a channel with its creator alone, in the chosen space |
| message on a member's name | the existing direct message, or a new one |
| rename on a direct message | a name for every member, or none |
| invite into a channel | the invitee joins, with the history since the latest boundary |
| leave a channel | the channel leaves the sidebar, and a private one rotates its key |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A direct message is reused | a new one for each open | two conversations between the same people split one history |
| A rotation closes the active segment | rotating inside the segment | a segment would hold deltas under two keys, and its final snapshot under one |
| A public channel keeps its key on a leave | rotating on every leave | every member of the space reads the key in the catalogue anyway |
| A direct message is a channel | a separate model for direct messages | a conversion would copy the history into a new document, with new identifiers and a new key |
| No forward secrecy | forward secrecy for direct messages | a full history on a new device, the final snapshots and the search index all need every key |
| The channel list is the membership | a sealed document listing the channels of a user | a second list drifts after a revoke, and shows a channel whose key is gone |
| A channel is deleted, not archived, in its first form | archiving first | an archive keeps every byte, so it answers neither an erasure request nor the storage a dead channel holds |
| A direct message converts to private | straight into a public channel | the whole space would read a history written for a few members |
| Keep or start fresh | a history duration chosen at conversion | keys change at a rotation, not on a date: a cut inside a segment would split it under two keys |
| Archive keeps a channel | archive deletes it | a closed project still holds decisions its members come back to |
| An owner of the space stands in for the last owner of a public channel | refusing the leave, as for a document | a space leave would answer a conflict for each public channel the member owns, an admin could not remove such a member, and a vanished owner would leave a channel nobody deletes |
| An owner of the space stands in | every admin of the space, editors included | an editor writes the catalogue, and deleting a channel is the act an owner of the whole space already holds |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The invite list before the channel lists its members.** Until the Logic thread lists them, the
  control offers every member of the space. The Logic thread then refuses a listed one
  (`MURMURE-500`).