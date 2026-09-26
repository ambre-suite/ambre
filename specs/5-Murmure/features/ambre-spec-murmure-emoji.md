# AMBRE -- FEATURE: MURMURE EMOJI
**Type:** Vertical (feature)
**Implementation:** design-only

> A member writes an emoji in a message, or reacts with one, from a picker. The emoji are drawn in
> colour, and the font that carries them loads only when first needed.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-137` | `OBS` | A Unicode emoji MUST be drawn in colour, in a message and in a reaction. | this spec section 3 | not built | unbuilt -- lands with the `Emoji` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-138` | `INV` | The colour emoji font MUST load on first use, never at start. | this spec section 3; `ambre-spec-budgets.md section 6` | not built | unbuilt -- lands with the `Emoji` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Pick an emoji

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-139` | `POST` | An emoji picked from the composer MUST be inserted at the caret. | this spec section 3 | not built | unbuilt -- lands with the `Emoji` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-140` | `POST` | An emoji picked from a message MUST be added as a reaction. | this spec section 3; `features/ambre-spec-murmure-reactions.md` | not built | unbuilt -- lands with the `Emoji` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-141` | `FRAME` | The picker MUST NOT send a request outside the client. | this spec section 3; `ZK-007` | not built | unbuilt -- lands with the `Emoji` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the colour emoji font row of `ambre-spec-budgets.md section 6`.
* **Boundary.** none: an emoji is text inside a sealed message, and the font ships with the
  client.
* **Verification.** 5 `unimplemented`: no emoji is drawn in colour today.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The channel holds | Each user sees | Test |
|---|---|---|---|---|
| types an emoji | watches | one message | the emoji in colour, on both screens (`MURMURE-137`) | none |
| picks an emoji from a message | watches | one reaction | the reaction, in colour | none |

---

## 3. MODEL

**CRDT containers.** none of its own: an emoji is text in `content`, or a key in `reactions`.

**The font** is a colour emoji font served with the client, loaded on the first emoji to draw. The
glyph pipeline draws it through the colour atlas the budget register already declares.

**Code.** not built.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the emoji control of the composer | the picker, then the emoji at the caret |
| the reaction control of a message | the picker, then the emoji as a reaction |
| `:` followed by a name in the composer | the matching emoji, offered in a list |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The font ships with the client | emoji images from a remote service | each emoji drawn would tell that service what the user reads |
| Loaded on first use | loaded at start | a font of about 10 MB would weigh on every start |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing is built.** No colour emoji is drawn.
* **The font is not chosen.** Its size decides whether the fonts row still holds.
