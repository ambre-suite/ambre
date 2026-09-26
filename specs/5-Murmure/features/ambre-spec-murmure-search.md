# AMBRE -- FEATURE: MURMURE SEARCH IN LOADED SEGMENTS
**Type:** Vertical (feature)
**Implementation:** design-only

> A member finds a message by its words, in the channels they have loaded. The search runs on the
> client, and it says which part of the history it covered.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-147` | `INV` | A search MUST run on the client alone. | this spec section 3; `ZK-007` | not built | unbuilt -- lands with the `Search in loaded segments` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-148` | `INV` | A search MUST cover the active segment and every segment already loaded. | this spec section 3 | not built | unbuilt -- lands with the `Search in loaded segments` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Search

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-149` | `POST` | A result list MUST state which segments it covered. | this spec section 3 | not built | unbuilt -- lands with the `Search in loaded segments` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-150` | `POST` | A result MUST open its message in its channel. | this spec section 3 | not built | unbuilt -- lands with the `Search in loaded segments` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-151` | `FRAME` | A search MUST NOT load a segment the user did not open. | this spec section 3; `RESID-016` | not built | unbuilt -- lands with the `Search in loaded segments` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** a search over the loaded segments reads memory the client holds, inside the rows
  of `ambre-spec-budgets.md section 6`.
* **Boundary.** none: the query never leaves the client, and the relay searches nothing.
* **Verification.** 5 `unimplemented`: no search exists in Murmure.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The channel holds | Each user sees | Test |
|---|---|---|---|---|
| searches "budget" | -- | the loaded segments | the matches, and "searched 2 of 14 segments" (`MURMURE-149`) | none |
| opens a result | -- | the segment of the match | the channel, at the message (`MURMURE-150`) | none |

---

## 3. MODEL

**CRDT containers.** none: a search reads the messages the loaded documents hold.

**The words** are cut with diacritics folded. A search on a new device is the
`Search on a new device` Story (`ambre-roadmap.md` E5).

**Code.** not built.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the search field | the matches, grouped by channel, with the coverage |
| a result | the channel opens at the message |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The client searches | a search on the relay | the relay would learn the query and the matches, which breaks the zero-knowledge pillar |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing is built.** No search exists in Murmure.
* **Older segments are not searched.** The user loads them by scrolling, and the coverage says so.
