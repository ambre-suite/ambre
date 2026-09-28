# AMBRE -- FEATURE: MURMURE MENTIONS
**Type:** Vertical (feature)
**Implementation:** design-only

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
| `MURMURE-098` | `OBS` | A mention MUST show the current display name of the user it names. | this spec section 3 | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-099` | `INV` | A mention of a user who is not a member of the channel MUST NOT notify that user. | this spec section 3 | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-233` | `INV` | A mention of the channel MUST count as a mention for every member of the channel. | this spec section 3 | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Mention a member

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-100` | `POST` | The composer MUST write a `mention` mark over the displayed name, carrying the user identifier. | this spec section 3; `MURMURE-008` | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-101` | `POST` | The summary of a message MUST list the users it mentions. | this spec section 3; `SYNC-113` | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-102` | `FRAME` | Mentioning a user MUST NOT change the membership of the channel. | this spec section 3 | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Read the mentions of a message

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-103` | `POST` | A client MUST find the mentions of a message by its marks, never by parsing its text. | this spec section 3 | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-104` | `FAIL` | Given a summary that lists a mention the message does not carry, the loaded message MUST correct the count. | this spec section 3; `RESID-011` | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-152` | `FRAME` | Reading the mentions of a message MUST NOT change the message. | this spec section 3 | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.4 Mention the whole channel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-229` | `POST` | An `@channel` MUST write a `mention` mark that names the channel and no user. | this spec section 3; `MURMURE-008` | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-230` | `POST` | The summary of a message that mentions the channel MUST say so, and list no member for it. | this spec section 3; `SYNC-113` | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-231` | `POST` | In a channel past the member threshold of the budget register, the composer MUST ask for confirmation before it sends an `@channel`. | this spec section 3; `ambre-spec-budgets.md section 6` | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-232` | `FRAME` | Mentioning the channel MUST NOT write the list of its members into the message. | this spec section 3 | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** none of its own: a mention is a mark inside a message.
* **Boundary.** the mentioned users ride in the sealed summary (`SYNC-113`), never in a field the
  relay reads. Push notifications, later, put that list in front of the relay, which they rule.
* **Verification.** 13 `unimplemented`: no mention exists in the code.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The channel holds | Each user sees | Test |
|---|---|---|---|---|
| mentions Bob | has the channel in the background | one message, one mark | a mention badge on the channel (`MURMURE-101`) | none |
| mentions Bob | renames himself | the mark, with Bob's identifier | Alice's message shows the new name (`MURMURE-098`) | none |
| -- | Mallory writes a summary that claims a mention of Bob | a message with no mark | a badge that disappears on open (`MURMURE-104`) | none |

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

**Code.** not built.

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

* **Nothing is built.** No mention exists.
