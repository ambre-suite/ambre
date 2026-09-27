# AMBRE -- FEATURE: MURMURE REACTIONS
**Type:** Vertical (feature)
**Implementation:** design-only

> A member answers a message with an emoji. Many members can react with the same one, and each
> member removes only their own. A reaction never counts as an unread message.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-091` | `INV` | A reaction MUST NOT count as an unread message. | this spec section 3; `features/ambre-spec-murmure-unread.md` | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 React to a message

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-092` | `POST` | A reaction MUST add one entry keyed by its emoji and the reacting member. | this spec section 3; `MURMURE-006` | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-093` | `POST` | A reaction MUST carry an activity summary of kind reaction. | this spec section 3; `SYNC-113` | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-094` | `FAIL` | Given a delta that writes a reaction under another user's key, every client MUST refuse the delta. | this spec section 3; `features/ambre-spec-murmure-messages.md` | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-095` | `FRAME` | Reacting MUST NOT change the content of the message. | this spec section 3 | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Remove a reaction

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-096` | `POST` | Removing a reaction MUST delete the member's own entry for that emoji. | this spec section 3 | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-097` | `FRAME` | Removing a reaction MUST NOT remove the entry of another member. | this spec section 3; `MURMURE-007` | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** none of its own: a reaction is one small delta, charged to the send rows of
  `ambre-spec-budgets.md section 1`.
* **Boundary.** a reaction is sealed like any delta (`ZK-001`), and its kind rides in the sealed
  summary, so the relay cannot tell it from a message.
* **Verification.** 7 `unimplemented`: no reaction exists in the code.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The channel holds | Each user sees | Test |
|---|---|---|---|---|
| reacts with a thumbs-up | reacts with a thumbs-up at the same instant | two entries under one emoji | one thumbs-up counted twice (`MURMURE-007`) | none |
| removes her thumbs-up | keeps his | Bob's entry alone | one thumbs-up, Bob's (`MURMURE-097`) | none |
| reacts to a message | has the channel in the background | one reaction | no unread badge moves (`MURMURE-091`) | none |

---

## 3. MODEL

**CRDT containers.** the `reactions` map of a message, in `ambre-spec-murmure-schema.md section 3`:
one flat map, one entry per emoji and user. The message creates it empty (`MURMURE-256`), so no
reaction ever creates a container.

**Why one entry per user.** Each member writes and removes only their own key, so two concurrent
reactions never race for one value. The authorship check refuses an entry under another key
(`MURMURE-094`).

**Code.** not built.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the reaction control on a message | the picker, then the emoji as a reaction |
| a click on an existing reaction | adds or removes one's own entry under that emoji |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| One entry per user | a list of the users who reacted | two concurrent removals of one slot remove the wrong member |
| One entry per user | a counter per emoji | a counter cannot say who reacted, and nobody could remove their own reaction alone |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing is built.** No reaction exists.
* **Custom emoji are not here.** They are the `Custom emoji` Story (`ambre-roadmap.md` E5).
