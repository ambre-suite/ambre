# AMBRE -- FEATURE: THE MEMBERS OF A SPACE
**Type:** Vertical (feature)

> A member sees who else belongs to the space, who is online, and writes to any of them in one
> gesture. The list and its search stay on the client: what the user types never leaves it.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-440` | `INV` | The member list of a space MUST hold the members of the space document, and no one else. | this spec section 3; `MURMURE-031`; `SRV-168` | `ambre_murmure::members::tests::the_list_holds_the_member_rows_and_no_one_else` -- a sample from a user outside the rows adds no one; `ambre_logic::spaces::tests::the_member_list_holds_the_rows_of_the_space_document_and_opens_nothing_else` -- the relay's rows become the list; `ambre_logic::spaces::tests::a_change_of_members_while_the_space_is_open_reads_the_rows_again` -- a join and a leave while the space is open; `::a_change_of_members_hands_the_rows_and_costs_the_space_no_request` -- the new rows replace the old, and nothing is asked; `ambre_sync::updates::tests::a_change_of_members_costs_one_listing_whose_rows_reach_logic` -- the rows the relay's word reads reach Logic | none | verified |
| `MURMURE-441` | `OBS` | The member list MUST show each member by display name and avatar. | this spec section 3; `PROFILE-001` | `ambre_app::shell::murmure_regions::the_member_list_shows_each_member_by_name_initials_and_presence` -- browser tier, the list opened from a channel header shows two members by name, each with their initials on their identity colour | none | verified |
| `MURMURE-442` | `OBS` | The member list MUST mark each member as online or offline. | this spec section 3 | `ambre_app::shell::murmure_regions::the_member_list_shows_each_member_by_name_initials_and_presence` -- browser tier, an online and an offline member each carry their mark and its word | none | verified |
| `MURMURE-443` | `OBS` | Each presence state MUST differ from the others in shape as well as in colour. | this spec section 3; `ambre-spec-accessibility.md` | `ambre_app::shell::murmure_regions::the_presence_marks_differ_in_shape_as_well_as_in_colour` -- browser tier, the computed style reads a filled dot online and a hollow ring offline, in two colours | none | verified |
| `MURMURE-519` | `OBS` | The member list opened from a channel MUST mark the members who watch that channel, and only while they watch it. | this spec section 3; `MURMURE-126`; `MURMURE-127` | `ambre_app::shell::murmure_regions::the_member_list_marks_who_watches_the_channel` -- browser tier, a member watching and this user carry the mark, the list of another channel marks this user alone, and the mark leaves with the watcher | none | verified |
| `MURMURE-520` | `OBS` | The mark of a member in the channel MUST differ in shape from the online and offline marks. | this spec section 3; `MURMURE-443` | `ambre_app::shell::murmure_regions::the_mark_of_the_channel_differs_in_shape_from_the_others` -- browser tier, the computed style reads a dot inside a ring, in a colour of its own | none | verified |
| `MURMURE-444` | `INV` | The presence of a member in a space MUST ride the ephemerals of the space document. | this spec section 3; `ambre-spec-presence.md`; `ZK-005` | `ambre_logic::spaces::tests::opening_a_space_sends_this_users_presence_on_its_document_alone` -- the one sample goes on the space document, which the Sync worker seals under its key | none | verified |
| `MURMURE-445` | `INV` | A member MUST show as offline once no connection of theirs refreshes their presence within the presence expiry. | this spec section 3; `ambre-spec-budgets.md section 8` | `ambre_murmure::members::tests::a_member_turns_offline_once_no_sample_refreshes_them_within_the_expiry` -- online at the expiry, offline one millisecond past it; `ambre_logic::spaces::tests::a_member_with_no_sample_within_the_expiry_shows_offline` -- the view's tick turns the member offline; `ambre_app::shell::murmure_regions::a_refresh_reaches_every_space_and_a_space_lost_drops_its_members` -- browser tier, the page's refresh asks it of every space of the sidebar | none | verified |
| `MURMURE-446` | `INV` | The presence in a space MUST NOT show which channel a member reads. | this spec section 3; `MURMURE-127` | `ambre_murmure::members::tests::the_sample_holds_the_state_alone` -- the sample is two bytes, its tag and the state, and nothing else decodes | none | verified |
| `MURMURE-447` | `INV` | A member with no input past the idle delay MUST show as away. | this spec section 3; `ambre-spec-budgets.md section 8` | not built | unbuilt -- lands with the `Presence states` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-459` | `INV` | A member with no connection MUST show as offline, whatever status they set. | this spec section 3 | not built | unbuilt -- lands with the `Presence states` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Open a space

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-448` | `POST` | Opening a space MUST show the member as online to the other members of the space. | this spec section 3 | `ambre_logic::spaces::tests::opening_a_space_sends_this_users_presence_on_its_document_alone` -- the open sends the sample at once; `ambre_logic::spaces::tests::a_sample_shows_its_member_online_and_a_newcomer_is_answered_once` -- the sample turns its member online on the other side | none | verified |
| `MURMURE-449` | `FRAME` | Opening a space MUST NOT show the member as online to a user outside the space. | this spec section 3; `MURMURE-123`; `ZK-005` | `ambre_logic::spaces::tests::opening_a_space_sends_this_users_presence_on_its_document_alone` -- the sample goes on the space document alone, whose key only its members hold | none | verified |
| `MURMURE-472` | `FRAME` | Opening a space MUST NOT ask the relay for its members beyond the readings the document open and its subscription make. | this spec section 3; `SYNC-150`; `SYNC-151` | `ambre_logic::spaces::tests::an_opening_space_asks_for_no_rows_and_shows_the_last_ones_sync_read` -- no listing leaves the space, and the last rows read during the open show once it is open; `ambre_sync::updates::tests::a_subscription_reads_the_members_a_change_during_the_open_left_out` -- the subscription's reading reaches the space | none | verified |

### 1.3 Open the member list

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-450` | `POST` | The member list MUST show the online members first, then the others, each group sorted by display name. | this spec section 4 | `ambre_murmure::members::tests::the_list_shows_the_online_members_first_each_group_by_name` -- four members, two online, names in mixed case | none | verified |
| `MURMURE-451` | `FRAME` | Opening the member list MUST NOT load a document other than the space document. | this spec section 3; `RESID-016` | `ambre_logic::spaces::tests::the_member_list_holds_the_rows_of_the_space_document_and_opens_nothing_else` -- its last assertion: no document opens | none | verified |

### 1.4 Find a member

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-452` | `POST` | The palette MUST list, after the channels, the members of the current space whose display name matches what the user types. | this spec section 4; `MURMURE-172` | `ambre_murmure::tests::chrome_relay::the_palette_lists_the_other_members_of_the_space_after_the_channels` -- the members of the shown channel's space after its channels, this user left out, and a query that keeps one | none | verified |
| `MURMURE-453` | `POST` | The filter of the member list MUST keep only the members whose display name matches what the user types. | this spec section 4 | `ambre_murmure::members::tests::the_filter_keeps_the_members_whose_name_matches` -- a substring in any case, in the order of the list | none | verified |
| `MURMURE-454` | `FRAME` | Finding a member MUST NOT send what the user types outside the client. | this spec section 3; `MURMURE-174` | type-level -- the filter takes the list the client holds and the query, and returns a borrow of that list: it holds no channel to the Sync worker | none | verified |

### 1.5 Write to a member

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-455` | `POST` | Choosing a member MUST open the direct message between the user and that member alone. | this spec section 4; `MURMURE-051` | `ambre_murmure::tests::chrome_relay::choosing_a_member_fetches_their_key_then_shows_the_direct_message` -- the choice fetches the key, and the answered channel shows; `ambre_app::shell::murmure_regions::a_click_on_another_member_fetches_their_key` -- the same from the list; `ambre_sync::sharing::tests::a_member_key_is_fetched_by_user_id_and_wraps_nothing`; `ambre_app::tofu::tests::a_trusted_member_key_opens_the_direct_message_of_the_two` -- the request names that member alone, with the pinned key | none | verified |

**Frame:** `MURMURE-053`

### 1.6 Set a status

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-456` | `POST` | Setting busy MUST show the member as busy to every member of the space. | this spec section 3 | not built | unbuilt -- lands with the `Presence states` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-457` | `POST` | A status the member sets MUST be the same on every device of that member. | this spec section 3; `SETTINGS-012` | not built | unbuilt -- lands with the `Presence states` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-458` | `FRAME` | Setting a status MUST NOT change the notification level of any channel. | this spec section 3; `MURMURE-268` | not built | unbuilt -- lands with the `Presence states` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the presence write of `ambre-spec-budgets.md section 3`, once per refresh period
  (`ambre-spec-budgets.md section 8`). Each picture costs the avatar row of its section 6.
* **Boundary.** the presence sample is sealed under the key of the space document (`ZK-005`). The
  relay learns the member list and the display names it already holds (`ZK-018`).
* **Verification.** 18 `verified`, 5 `unimplemented`. The engine and the Logic thread hold the
  list, the presence and the filter. The browser tier draws the list beside a channel, with its
  watchers marked. A member chosen there or in the palette opens a direct message.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The space holds | Each user sees | Test |
|---|---|---|---|---|
| opens the space | has it open | Alice and Bob | Bob sees Alice online (`MURMURE-448`) | `ambre_logic::spaces::tests::a_sample_shows_its_member_online_and_a_newcomer_is_answered_once` |
| closes every tab | has the space open | Alice and Bob | Alice offline once her presence expires (`MURMURE-445`) | `ambre_logic::spaces::tests::a_member_with_no_sample_within_the_expiry_shows_offline` |
| reads the "budget" channel | reads "design" | nothing more | Alice online, never where (`MURMURE-446`) | `ambre_murmure::members::tests::the_sample_holds_the_state_alone` |
| types "bo" in the palette | -- | Bob among the members | Bob under the channels; choosing him opens their direct message (`MURMURE-455`) | `ambre_murmure::tests::chrome_relay::choosing_a_member_fetches_their_key_then_shows_the_direct_message` |
| sets busy | has the space open | nothing durable | Alice busy (`MURMURE-456`) | none |

---

## 3. MODEL

**CRDT containers.** none. The member rows of the space document are the relay's table
(`MURMURE-031`, `ambre-spec-membership.md`). The display name comes with each row, and the avatar
follows `ambre-spec-profile.md`.

The relay says when the rows change (`SRV-177`). Sync reads them
once and hands them to an open space, which asks for nothing more (`SYNC-151`). Of two readings
that cross, the later one stands.

The space asks for no rows of its own (`MURMURE-472`): it keeps the reading the subscription
makes (`SYNC-150`), even while it opens.

**Presence in a space.** Each client that holds a space open sends one presence sample on the space
document, once per refresh period of `ambre-spec-budgets.md section 8`. The sample is an ephemeral
that holds the state alone, sealed like every ephemeral (`ZK-005`). A member whose sample expires
shows as offline. The sample carries no channel: the presence of a channel stays in
`ambre-spec-murmure-typing-presence.md`.

**The clock is the view's.** The Logic thread reads no clock. The view ticks once per refresh
period: the tick sends the sample and dates every sample heard since the tick before. A sample is
thus dated to within one period, so it expires after the presence expiry of
`ambre-spec-budgets.md section 8`, three periods, and not after one.

**A newcomer is answered.** The relay keeps no presence a client can read. So a client answers
the first sample of a member with its own, and a newcomer sees who is online at once. The answer
goes once per member: two clients never answer each other twice.

**A departure.** The relay's word that a member left turns them offline at once
(`PRESENCE-014`). When the departing connection is another tab of this user, this tab answers
with its sample, and the other members see the user online again.

**The states, and how each one reads.**

| State | Set by | Mark |
|---|---|---|
| online | a connection holds the space open | a filled green dot |
| away | no input past the idle delay of `ambre-spec-budgets.md section 8` | an amber half-filled dot |
| busy | the member | a red dot with a bar |
| offline | no connection, whatever the member set | a hollow grey ring |
| in the channel | a connection watches the channel the list opened from | an accent dot inside a ring |

The last mark takes the place of the others in the list opened from that channel (`MURMURE-519`).
This user carries it too, since the channel is on their screen.
The colours come from the design tokens (`ambre-spec-design-tokens.md`). The shape carries the
state for a user who does not see the colour.

**The status a member sets** lives in the `presence` map of the user settings, so every device of
the member reads it (`ambre-spec-user-settings.md`). A connection copies it into its sample.

**Code.** `ambre-murmure` holds the sample, the roster with its expiry, the order and the filter.
`ambre-logic` sends the sample, takes the member rows Sync reads, and hands the view the list.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the members button of the header of a channel in a space | the member list of the space opens in the right column, and closes on a second press (`ambre-spec-murmure-layout.md`) |
| typing in the filter of the member list | the list keeps the members whose display name matches |
| typing in the palette | the matching members follow the channels and the direct messages |
| choosing a member, in the list or in the palette | the direct message with that member opens, created on first use |
| the status menu of the user | online, or busy |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The client filters the list it holds | a member search route on the relay | the relay would read every query, and a route that answers names invites a search by a non-member |
| The member search lives in the palette | one search field for members and messages | the two searches share no mechanism: one filters names in the clear, the other reads decrypted segments |
| Presence rides the space document | a presence per channel, merged on the client | a member would show as offline in a space whose channels they do not watch |
| One sample per refresh period | a sample on every change of input | the cost grows with the members times the samples, and a list only needs a state per minute |
| Offline wins over a status | a status that outlives the connection | a member marked busy while away from every device reads as present |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A private channel or a direct message lists no members.** The list shows the members of the
  space, so its button shows on a channel in a space alone. The members of a channel wait on the
  `The members of a channel` Story (`ambre-roadmap.md` E5).
* **Do not disturb is not ruled.** It changes who a notification wakes, so it waits on the
  notification design (`ambre-spec-murmure-notifications.md`).
* **A space of thousands of members is not ruled.** The list and the presence scale with the
  members, and no budget row bounds a space.
