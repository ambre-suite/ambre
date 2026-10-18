# AMBRE -- FEATURE: MURMURE MENTIONS
**Type:** Vertical (feature)
**Implementation:** partial -- the mark, the summary and the counts are built; the `@` list is not

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
| `MURMURE-098` | `OBS` | A mention MUST show the current display name of the user it names. | this spec section 3 | `ambre_murmure::mention_tests::a_mention_shows_the_current_name_of_its_member` -- a rename shows in the message, an unknown name keeps the typed text | none | verified |
| `MURMURE-099` | `INV` | A mention of a user who is not a member of the channel MUST NOT notify that user. | this spec section 3 | not built | unbuilt -- lands with the `The mention list` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-233` | `INV` | A mention of the channel MUST count as a mention for every member of the channel. | this spec section 3 | `ambre_murmure::mention_tests::a_mention_counts_for_its_member_and_an_at_channel_for_all` -- an `@channel` counts for two members, never for its author | none | verified |

### 1.2 Mention a member

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-100` | `POST` | The composer MUST write a `mention` mark over the displayed name, carrying the user identifier. | this spec section 3; `MURMURE-008` | `ambre_murmure::mention_tests::a_mention_is_a_mark_that_carries_its_member`; `ambre_murmure::mention_tests::a_mention_outside_the_body_writes_nothing` -- the mark spans the name and a range past the body writes nothing | none | verified |
| `MURMURE-101` | `POST` | The summary of a message MUST list the users it mentions. | this spec section 3; `SYNC-113` | `ambre_murmure::channel_tests::a_sent_message_carries_a_content_summary_with_its_mentions`; `ambre_logic::channels::tests::a_closed_channel_counts_its_mentions_from_the_summaries` -- the summary lists each member once and a closed channel counts it | none | verified |
| `MURMURE-102` | `FRAME` | Mentioning a user MUST NOT change the membership of the channel. | this spec section 3 | `ambre_logic::channels::tests::a_channel_command_reaches_its_engine_and_leaves_as_a_delta_and_an_event` -- a mention sends one update and no membership request | none | verified |

### 1.3 Read the mentions of a message

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-103` | `POST` | A client MUST find the mentions of a message by its marks, never by parsing its text. | this spec section 3 | `ambre_murmure::mention_tests::a_mention_is_a_mark_that_carries_its_member` -- an `@` name typed with no mark reads as no mention | none | verified |
| `MURMURE-104` | `FAIL` | Given a summary that lists a mention the message does not carry, the loaded message MUST correct the count. | this spec section 3; `RESID-011` | `ambre_murmure::mention_tests::a_mention_counts_for_its_member_and_an_at_channel_for_all` -- a forged summary claim counts nothing once the message is loaded | none | verified |
| `MURMURE-152` | `FRAME` | Reading the mentions of a message MUST NOT change the message. | this spec section 3 | `ambre_murmure::mention_tests::reading_the_mentions_changes_nothing` -- the version and the value are unchanged after a read and a count | none | verified |

### 1.4 Mention the whole channel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-229` | `POST` | An `@channel` MUST write a `mention` mark that names the channel and no user. | this spec section 3; `MURMURE-008` | `ambre_murmure::mention_tests::an_at_channel_names_the_channel_and_no_member` -- the mark holds the channel identifier | none | verified |
| `MURMURE-230` | `POST` | The summary of a message that mentions the channel MUST say so, and list no member for it. | this spec section 3; `SYNC-113` | `ambre_murmure::mention_tests::an_at_channel_names_the_channel_and_no_member` -- the summary flags the channel and lists no member | none | verified |
| `MURMURE-231` | `POST` | In a channel past the member threshold of the budget register, the composer MUST ask for confirmation before it sends an `@channel`. | this spec section 3; `ambre-spec-budgets.md section 6` | not built | unbuilt -- lands with the `The mention list` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-232` | `FRAME` | Mentioning the channel MUST NOT write the list of its members into the message. | this spec section 3 | `ambre_murmure::mention_tests::an_at_channel_names_the_channel_and_no_member` -- the message holds no field beyond its five keys | none | verified |

* **Budget.** none of its own: a mention is a mark inside a message.
* **Boundary.** the mentioned users ride in the sealed summary (`SYNC-113`), never in a field the
  relay reads. Push notifications, later, put that list in front of the relay, which they rule.
* **Verification.** 11 `verified`, 2 `unimplemented`. The mention tests of `crates/ambre-murmure`
  hold the mark, the count and the summary. Both open rows need the members of a channel.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The channel holds | Each user sees | Test |
|---|---|---|---|---|
| mentions Bob | has the channel in the background | one message, one mark | a mention badge on the channel (`MURMURE-101`) | `ambre_logic::channels::tests::a_closed_channel_counts_its_mentions_from_the_summaries` |
| mentions Bob | renames himself | the mark, with Bob's identifier | Alice's message shows the new name (`MURMURE-098`) | `ambre_murmure::mention_tests::a_mention_shows_the_current_name_of_its_member` |
| -- | Mallory writes a summary that claims a mention of Bob | a message with no mark | a badge that disappears on open (`MURMURE-104`) | `ambre_murmure::mention_tests::a_mention_counts_for_its_member_and_an_at_channel_for_all` |

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

**The mention list** is a floating panel of the shared shell, placed at the caret. It takes its
placement, its flip and its keyboard model from `ambre-spec-chrome.md` (`CHROME-004`).

**What travels where.** The mark is the truth, and the summary is the hint that lets a client count
with no document loaded. The loaded message always wins (`MURMURE-104`).

**Code.** `crates/ambre-murmure/src/mention.rs`, `ChannelState::send` and `ChannelState::mentions`.
The Logic thread folds the counts into the channel mark.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| `@` in the composer | a list of the members of the channel, filtered as the user types, with `@channel` first |
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

* **The `@` list is not built.** It needs the members of a channel in the Logic thread, which also
  rule a mention of a non-member and the `@channel` threshold.
