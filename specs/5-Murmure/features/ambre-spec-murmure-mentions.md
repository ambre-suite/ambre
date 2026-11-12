# AMBRE -- FEATURE: MURMURE MENTIONS
**Type:** Vertical (feature)

> A member names another in a message with `@`. The one named is told, even when the channel is
> not loaded, and the name follows a later rename.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-098` | `OBS` | A mention MUST show the current display name of the user it names. | this spec section 3 | `ambre_murmure::logic::mention_tests::a_mention_shows_the_current_name_of_its_member` -- a rename shows in the message, an unknown name keeps the typed text | none | verified |
| `MURMURE-099` | `INV` | A mention of a user who is not a member of the channel MUST NOT notify that user. | this spec section 3 | `ambre_murmure::logic::mention_tests::a_mention_of_a_non_member_is_no_mark`; `ambre_logic::channels::tests::the_members_of_a_channel_reach_the_view_and_screen_its_mentions` -- the summary lists the member alone and the text stays | none | verified |
| `MURMURE-233` | `INV` | A mention of the channel MUST count as a mention for every member of the channel. | this spec section 3 | `ambre_murmure::logic::mention_tests::a_mention_counts_for_its_member_and_an_at_channel_for_all` -- an `@channel` counts for two members, never for its author | none | verified |

### 1.2 Mention a member

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-100` | `POST` | The composer MUST write a `mention` mark over the displayed name, carrying the user identifier. | this spec section 3; `MURMURE-008` | `ambre_murmure::logic::mention_tests::a_mention_is_a_mark_that_carries_its_member`; `ambre_murmure::logic::mention_tests::a_mention_outside_the_body_writes_nothing` -- the mark spans the name and a range past the body writes nothing | none | verified |
| `MURMURE-101` | `POST` | The summary of a message MUST list the users it mentions. | this spec section 3; `SYNC-113` | `ambre_murmure::logic::channel_tests::a_sent_message_carries_a_content_summary_with_its_mentions`; `ambre_logic::channels::tests::a_closed_channel_counts_its_mentions_from_the_summaries` -- the summary lists each member once and a closed channel counts it | none | verified |
| `MURMURE-102` | `FRAME` | Mentioning a user MUST NOT change the membership of the channel. | this spec section 3 | `ambre_logic::channels::tests::a_channel_command_reaches_its_engine_and_leaves_as_a_delta_and_an_event` -- a mention sends one update and no membership request | none | verified |

### 1.3 Read the mentions of a message

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-103` | `POST` | A client MUST find the mentions of a message by its marks, never by parsing its text. | this spec section 3 | `ambre_murmure::logic::mention_tests::a_mention_is_a_mark_that_carries_its_member` -- an `@` name typed with no mark reads as no mention | none | verified |
| `MURMURE-104` | `FAIL` | Given a summary that lists a mention the message does not carry, the loaded message MUST correct the count. | this spec section 3; `RESID-011` | `ambre_murmure::logic::mention_tests::a_mention_counts_for_its_member_and_an_at_channel_for_all` -- a forged summary claim counts nothing once the message is loaded | none | verified |
| `MURMURE-152` | `FRAME` | Reading the mentions of a message MUST NOT change the message. | this spec section 3 | `ambre_murmure::logic::mention_tests::reading_the_mentions_changes_nothing` -- the version and the value are unchanged after a read and a count | none | verified |

### 1.4 Mention the whole channel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-229` | `POST` | An `@channel` MUST write a `mention` mark that names the channel and no user. | this spec section 3; `MURMURE-008` | `ambre_murmure::logic::mention_tests::an_at_channel_names_the_channel_and_no_member` -- the mark holds the channel identifier | none | verified |
| `MURMURE-230` | `POST` | The summary of a message that mentions the channel MUST say so, and list no member for it. | this spec section 3; `SYNC-113` | `ambre_murmure::logic::mention_tests::an_at_channel_names_the_channel_and_no_member` -- the summary flags the channel and lists no member | none | verified |
| `MURMURE-231` | `POST` | In a channel past the member threshold of the budget register, the composer MUST ask for confirmation before it sends an `@channel`. | this spec section 3; `ambre-spec-budgets.md section 6` | `ambre_murmure::logic::mention_tests::an_at_channel_past_the_threshold_waits_for_confirmation`; `ambre_logic::channels::tests::an_at_channel_past_the_threshold_asks_before_it_sends` -- eleven members ask first and nothing leaves before the confirmed resend | none | verified |
| `MURMURE-232` | `FRAME` | Mentioning the channel MUST NOT write the list of its members into the message. | this spec section 3 | `ambre_murmure::logic::mention_tests::an_at_channel_names_the_channel_and_no_member` -- the message holds no field beyond its five keys | none | verified |

### 1.5 Pick a member from the `@` list

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-551` | `POST` | An `@` that starts a word in the composer MUST list `@channel` first, then the members of the channel whose name has a word that starts with the query, never the writer, until a space ends the query. | this spec section 3 | `ambre_murmure::ui::mention_list::tests::an_at_that_starts_a_word_opens_a_query_up_to_the_caret` -- an `@` inside a word opens nothing; `ambre_murmure::ui::view::tests::an_at_opens_the_members_with_the_channel_first` -- the list filters as the user types and a space closes it | none | verified |
| `MURMURE-552` | `POST` | A pick MUST write the name over the query, then a space, and the message sent MUST carry a `mention` mark over that name. | this spec section 3; `MURMURE-100` | `ambre_murmure::ui::view::tests::a_pick_writes_the_name_and_sends_its_mention` -- two picks send two marks over their names, and Tab picks without sending | none | verified |
| `MURMURE-553` | `POST` | An edit of the draft MUST move each mention it leaves whole, and drop each mention whose name it cuts. | this spec section 3 | `ambre_murmure::ui::view::tests::an_edit_of_the_draft_keeps_the_mentions_it_leaves_whole` -- text typed before a name moves its mark, and a key inside a name drops it | none | verified |
| `MURMURE-554` | `POST` | While the list shows, Up and Down MUST move its choice, Enter and Tab MUST pick it, and Escape MUST close it until its query ends. | this spec section 4 | `ambre_murmure::ui::view::tests::the_list_takes_its_keys_and_escape_closes_it_until_the_query_ends` -- the choice wraps round, Escape keeps the draft, and a new `@` opens it again | none | verified |
| `MURMURE-555` | `POST` | A press on an entry MUST pick it in the region whose list shows it, and leave the focus in the composer. | this spec section 4 | `ambre_murmure::ui::view::tests::a_click_on_an_entry_picks_it_in_its_region` -- a pick for the thread leaves the channel's draft; `ambre_murmure::tests::chrome_relay::the_mention_list_shows_its_entries_and_a_press_picks_one` -- the options show the choice, and a press is taken once by its region | none | verified |
| `MURMURE-556` | `FRAME` | Moving through the `@` list or closing it MUST NOT change the draft, and MUST NOT send it. | this spec section 4 | `ambre_murmure::ui::view::tests::the_list_takes_its_keys_and_escape_closes_it_until_the_query_ends` -- the draft reads `@` after Down, Up and Escape, and nothing left | none | verified |

### 1.6 Answer the `@channel` question

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-557` | `POST` | The question MUST name the number of members it notifies, and Send MUST send the held draft once, confirmed, to its channel or its thread. | this spec section 3; `MURMURE-231` | `ambre_murmure::ui::commands::tests::a_confirmed_at_channel_sends_the_held_draft_again` -- a message and a reply leave confirmed, the reply with its box; `ambre_murmure::tests::chrome_relay::the_at_channel_question_sends_on_send_and_gives_back_on_cancel` -- one confirmed send after Send | none | verified |
| `MURMURE-558` | `POST` | Cancel, Escape and the backdrop MUST give the draft back to the composer it left, with its mentions. A draft typed there since MUST stay. | this spec section 3 | `ambre_murmure::ui::view::tests::a_cancelled_at_channel_gives_its_draft_back` -- the next Enter sends the draft and its two marks, and a typed draft stays; `ambre_murmure::tests::chrome_relay::the_at_channel_question_sends_on_send_and_gives_back_on_cancel` -- the thread's region alone takes a reply back, once | none | verified |
| `MURMURE-559` | `FRAME` | Asking the question, and cancelling it, MUST NOT send the message. | this spec section 3 | `ambre_murmure::tests::chrome_relay::the_at_channel_question_sends_on_send_and_gives_back_on_cancel` -- nothing is sent while the dialog is open, nor after Cancel | none | verified |
| `MURMURE-560` | `FRAME` | A reply held by the question MUST NOT open its thread before Send. | this spec section 3 | `ambre_logic::channels::tests::an_at_channel_past_the_threshold_asks_before_it_sends` -- the first reply on a message asks, and Sync gets no thread to create | none | verified |

* **Budget.** none of its own: a mention is a mark inside a message.
* **Boundary.** the mentioned users ride in the sealed summary (`SYNC-113`), never in a field the
  relay reads. Push notifications, later, put that list in front of the relay, which they rule.
* **Verification.** 23 `verified`. The mention tests of `crates/ambre-murmure` hold the mark, the
  count, the summary and the screen. The Logic tests hold the members that feed it. The view tests
  hold the `@` list and the draft given back, and browser tests hold the panel and the question.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The channel holds | Each user sees | Test |
|---|---|---|---|---|
| mentions Bob | has the channel in the background | one message, one mark | a mention badge on the channel (`MURMURE-101`) | `ambre_logic::channels::tests::a_closed_channel_counts_its_mentions_from_the_summaries` |
| mentions Bob | renames himself | the mark, with Bob's identifier | Alice's message shows the new name (`MURMURE-098`) | `ambre_murmure::logic::mention_tests::a_mention_shows_the_current_name_of_its_member` |
| -- | Mallory writes a summary that claims a mention of Bob | a message with no mark | a badge that disappears on open (`MURMURE-104`) | `ambre_murmure::logic::mention_tests::a_mention_counts_for_its_member_and_an_at_channel_for_all` |

---

## 3. MODEL

**CRDT containers.** the `mention` mark on a message's `content`, in
`ambre-spec-murmure-schema.md section 3`. Its value is the user identifier, and it expands to
nothing.

**An `@channel`** is a mark that names the channel (`MURMURE-229`). The summary carries one flag,
not a list of members (`MURMURE-230`), and each client counts it as its own mention
(`MURMURE-233`). The list would grow with the channel, and the relay would learn nothing more from
it anyway. Past a member threshold, the composer asks first (`MURMURE-231`). Only moderation, a
later Story, can restrict it by role.

**The question** is a modal dialog. The Logic thread holds no draft: it hands the send back in its
event, and the view keeps it. Send sends it again, confirmed (`MURMURE-557`). Cancel gives it back
to the composer it left (`MURMURE-558`).

A draft typed there in the round trip wins, and the held one is lost. The dialog is modal, so that
window is a few milliseconds. Logic asks before it opens the thread of a first reply, so a cancel
leaves none (`MURMURE-560`).

**The mention list** is a floating panel hung above the caret. An `@` that starts a word opens a
query (`MURMURE-551`), and a pick writes the name with its mark (`MURMURE-552`). The pipe keeps the
focus, so the composer takes the list's keys (`MURMURE-554`). A press on an entry picks it and
moves no focus (`MURMURE-555`). The panel is a listbox in the page, so a screen reader reads it.

**The marks of a draft** count chars of the text they were laid on. Each edit maps them onto the
new text, and drops each one whose name it cut (`MURMURE-553`).

**The screen.** The Logic thread drops each mark of a non-member before a draft leaves
(`MURMURE-099`). Members unlisted yet, every mark stays.

**What travels where.** The mark is the truth, and the summary is the hint that lets a client count
with no document loaded. The loaded message always wins (`MURMURE-104`).

**Code.** `crates/ambre-murmure/src/logic/mention.rs`, `ChannelState::send` and `ChannelState::mentions`.
The list is `crates/ambre-murmure/src/ui/mention_list.rs`, the composer and `MentionPanel`.
The Logic thread folds the counts into the channel mark.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| `@` in the composer | a list of the members of the channel, filtered as the user types, with `@channel` first |
| Up, Down in the list | the choice moves round the list, and a screen reader says it |
| Enter, Tab in the list | the choice is written into the draft |
| Escape in the list | the list closes until its query ends |
| a press on an entry | that entry is written into the draft |
| Enter on an `@channel`, past the threshold | a dialog: "Notify all N members of this channel?", Cancel first, then Send |
| Cancel, Escape or the backdrop of that dialog | the draft is back in the composer, with its mentions |
| a click on a mention | the profile of the member named |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A mark carrying the user identifier | the name as plain text | a rename would break every mention, and a parse would find names that are not mentions |
| The mark is the truth | trusting the summary | a member could claim a mention of anyone, and the claim would stay |
| The summary carries one flag | the list of every member | it grows with the channel, and the mark already says everything |
| A confirmation past a threshold | no `@channel` before the moderation roles | a team channel needs to reach everyone from the first day |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The panel has a shell of its own.** It takes no placement nor flip from the shared one
  (`CHROME-004`), which waits for the `One floating-menu shell` Story (`ambre-roadmap.md` E8).
