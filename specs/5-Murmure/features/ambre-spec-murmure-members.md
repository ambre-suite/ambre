# AMBRE -- FEATURE: THE MEMBERS OF A SPACE
**Type:** Vertical (feature)
**Implementation:** partial -- the member rows, the presence of each member and the filter ship
in the engine and the Logic thread. No view draws them yet.

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
| `MURMURE-440` | `INV` | The member list of a space MUST hold the members of the space document, and no one else. | this spec section 3; `MURMURE-031` | `ambre_murmure::members::tests::the_list_holds_the_member_rows_and_no_one_else` -- a frame from a user outside the rows adds no one; `ambre_logic::spaces::tests::the_member_list_holds_the_rows_of_the_space_document_and_opens_nothing_else` -- the relay's rows become the list | partial -- the rows are read once, at the open: a join or a leave while the space is open lands with the `A live member list` Story (`ambre-roadmap.md` E5) | partial |
| `MURMURE-441` | `OBS` | The member list MUST show each member by display name and avatar. | this spec section 3; `PROFILE-001` | not built | unbuilt -- lands with the `The Murmure layout and keyboard` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-442` | `OBS` | The member list MUST mark each member as online or offline. | this spec section 3 | not built | unbuilt -- lands with the `The Murmure layout and keyboard` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-443` | `OBS` | Each presence state MUST differ from the others in shape as well as in colour. | this spec section 3; `ambre-spec-accessibility.md` | not built | unbuilt -- lands with the `The Murmure layout and keyboard` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-444` | `INV` | The presence of a member in a space MUST ride the ephemeral frames of the space document. | this spec section 3; `ambre-spec-presence.md`; `ZK-005` | `ambre_logic::spaces::tests::opening_a_space_sends_this_users_presence_on_its_document_alone` -- the one frame goes on the space document, which the Sync worker seals under its key | none | verified |
| `MURMURE-445` | `INV` | A member MUST show as offline once no connection of theirs refreshes their presence within the presence expiry. | this spec section 3; `ambre-spec-budgets.md section 8` | `ambre_murmure::members::tests::a_member_turns_offline_once_no_frame_refreshes_them_within_the_expiry` -- online at the expiry, offline one millisecond past it; `ambre_logic::spaces::tests::a_member_with_no_frame_within_the_expiry_shows_offline` -- the view's tick turns the member offline | none | verified |
| `MURMURE-446` | `INV` | The presence in a space MUST NOT show which channel a member reads. | this spec section 3; `MURMURE-127` | `ambre_murmure::members::tests::the_frame_holds_the_state_alone` -- the frame is two bytes, its tag and the state, and nothing else decodes | none | verified |
| `MURMURE-447` | `INV` | A member with no input past the idle delay MUST show as away. | this spec section 3; `ambre-spec-budgets.md section 8` | not built | unbuilt -- lands with the `Presence states` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-459` | `INV` | A member with no connection MUST show as offline, whatever status they set. | this spec section 3 | not built | unbuilt -- lands with the `Presence states` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Open a space

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-448` | `POST` | Opening a space MUST show the member as online to the other members of the space. | this spec section 3 | `ambre_logic::spaces::tests::opening_a_space_sends_this_users_presence_on_its_document_alone` -- the open sends the frame at once; `ambre_logic::spaces::tests::a_frame_shows_its_member_online_and_a_newcomer_is_answered_once` -- the frame turns its member online on the other side | none | verified |
| `MURMURE-449` | `FRAME` | Opening a space MUST NOT show the member as online to a user outside the space. | this spec section 3; `MURMURE-123`; `ZK-005` | `ambre_logic::spaces::tests::opening_a_space_sends_this_users_presence_on_its_document_alone` -- the frame goes on the space document alone, whose key only its members hold | none | verified |

### 1.3 Open the member list

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-450` | `POST` | The member list MUST show the online members first, then the others, each group sorted by display name. | this spec section 4 | `ambre_murmure::members::tests::the_list_shows_the_online_members_first_each_group_by_name` -- four members, two online, names in mixed case | none | verified |
| `MURMURE-451` | `FRAME` | Opening the member list MUST NOT load a document other than the space document. | this spec section 3; `RESID-016` | `ambre_logic::spaces::tests::the_member_list_holds_the_rows_of_the_space_document_and_opens_nothing_else` -- its last assertion: no document opens | none | verified |

### 1.4 Find a member

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-452` | `POST` | The palette MUST list, after the channels, the members of the current space whose display name matches what the user types. | this spec section 4; `MURMURE-172` | not built | unbuilt -- lands with the `The Murmure layout and keyboard` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-453` | `POST` | The filter of the member list MUST keep only the members whose display name matches what the user types. | this spec section 4 | `ambre_murmure::members::tests::the_filter_keeps_the_members_whose_name_matches` -- a substring in any case, in the order of the list | none | verified |
| `MURMURE-454` | `FRAME` | Finding a member MUST NOT send what the user types outside the client. | this spec section 3; `MURMURE-174` | type-level -- the filter takes the list the client holds and the query, and returns a borrow of that list: it holds no channel to the Sync worker | none | verified |

### 1.5 Write to a member

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-455` | `POST` | Choosing a member MUST open the direct message between the user and that member alone. | this spec section 4; `MURMURE-051` | not built | unbuilt -- lands with the `The Murmure layout and keyboard` Story (`ambre-roadmap.md` E5) | unimplemented |

**Frame:** `MURMURE-053`

### 1.6 Set a status

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-456` | `POST` | Setting busy MUST show the member as busy to every member of the space. | this spec section 3 | not built | unbuilt -- lands with the `Presence states` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-457` | `POST` | A status the member sets MUST be the same on every device of that member. | this spec section 3; `SETTINGS-012` | not built | unbuilt -- lands with the `Presence states` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-458` | `FRAME` | Setting a status MUST NOT change the notification level of any channel. | this spec section 3; `MURMURE-268` | not built | unbuilt -- lands with the `Presence states` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the presence write of `ambre-spec-budgets.md section 3`, once per refresh period
  (`ambre-spec-budgets.md section 8`). Each picture costs the avatar row of its section 6.
* **Boundary.** the presence frame is sealed under the key of the space document (`ZK-005`). The
  relay learns the member list and the display names it already holds (`ZK-018`).
* **Verification.** 9 `verified`, 1 `partial`, 10 `unimplemented`. The engine and the Logic
  thread hold the list, the presence and the filter. The view rows wait on the Murmure layout.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The space holds | Each user sees | Test |
|---|---|---|---|---|
| opens the space | has it open | Alice and Bob | Bob sees Alice online (`MURMURE-448`) | `ambre_logic::spaces::tests::a_frame_shows_its_member_online_and_a_newcomer_is_answered_once` |
| closes every tab | has the space open | Alice and Bob | Alice offline once her presence expires (`MURMURE-445`) | `ambre_logic::spaces::tests::a_member_with_no_frame_within_the_expiry_shows_offline` |
| reads the "budget" channel | reads "design" | nothing more | Alice online, never where (`MURMURE-446`) | `ambre_murmure::members::tests::the_frame_holds_the_state_alone` |
| types "bo" in the palette | -- | Bob among the members | Bob under the channels; choosing him opens their direct message (`MURMURE-455`) | none |
| sets busy | has the space open | nothing durable | Alice busy (`MURMURE-456`) | none |

---

## 3. MODEL

**CRDT containers.** none. The member rows of the space document are the relay's table
(`MURMURE-031`, `ambre-spec-membership.md`). The display name comes with each row, and the avatar
follows `ambre-spec-profile.md`.

**Presence in a space.** Each client that holds a space open sends one ephemeral frame on the space
document, once per refresh period of `ambre-spec-budgets.md section 8`. The frame holds the state
alone, sealed like every ephemeral frame (`ZK-005`). A member whose frame expires shows as offline.
The frame carries no channel: the presence of a channel stays in `ambre-spec-murmure-typing-presence.md`.

**The clock is the view's.** The Logic thread reads no clock. The view ticks once per refresh
period: the tick sends the frame and dates every frame heard since the tick before. A frame is
thus dated to within one period, so it expires after the presence expiry of
`ambre-spec-budgets.md section 8`, three periods, and not after one.

**A newcomer is answered.** The relay keeps no presence a client can read. So a client answers
the first frame of a member with its own, and a newcomer sees who is online at once. The answer
goes once per member: two clients never answer each other twice.

**A departure.** The relay's word that a member left turns them offline at once
(`PRESENCE-014`). When the departing connection is another tab of this user, this tab answers
with its frame, and the other members see the user online again.

**The states, and how each one reads.**

| State | Set by | Mark |
|---|---|---|
| online | a connection holds the space open | a filled green dot |
| away | no input past the idle delay of `ambre-spec-budgets.md section 8` | an amber half-filled dot |
| busy | the member | a red dot with a bar |
| offline | no connection, whatever the member set | a hollow grey ring |

The colours come from the design tokens (`ambre-spec-design-tokens.md`). The shape carries the
state for a user who does not see the colour.

**The status a member sets** lives in the `presence` map of the user settings, so every device of
the member reads it (`ambre-spec-user-settings.md`). A connection copies it into its frame.

**Code.** `ambre-murmure` holds the frame, the roster with its expiry, the order and the filter.
`ambre-logic` sends the frame, asks the relay for the member rows, and hands the view the list.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the members button of the space header | the member list opens in the right column (`ambre-spec-murmure-layout.md`) |
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
| One frame per refresh period | a frame on every change of input | the cost grows with the members times the frames, and a list only needs a state per minute |
| Offline wins over a status | a status that outlives the connection | a member marked busy while away from every device reads as present |

---

## 6. NOT BUILT, AND NOT YET RULED

* **No view draws the list.** The names, the avatars, the marks, the palette and the choice of a
  member wait on the `The Murmure layout and keyboard` Story (`ambre-roadmap.md` E5). The choice
  needs the key check of `ambre-spec-key-verification.md` before a direct message wraps a key.
* **The list is read once.** The relay pushes no change of the members of a document, so a join
  or a leave shows at the next open. The `A live member list` Story (`ambre-roadmap.md` E5)
  closes it.
* **Do not disturb is not ruled.** It changes who a notification wakes, so it waits on the
  notification design (`ambre-spec-murmure-notifications.md`).
* **A space of thousands of members is not ruled.** The list and the presence scale with the
  members, and no budget row bounds a space.
