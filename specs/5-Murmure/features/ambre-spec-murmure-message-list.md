# AMBRE -- FEATURE: THE MURMURE MESSAGE LIST
**Type:** Vertical (feature)
**Implementation:** design-only

> The conversation of a channel, newest at the bottom. It scrolls through years of history without
> a pause, and shows where the unread messages start.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-182` | `OBS` | Messages MUST be shown oldest at the top and newest at the bottom. | this spec section 3 | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-183` | `OBS` | Consecutive messages of one author inside the grouping span MUST share one header. | this spec section 3; `ambre-spec-budgets.md section 8` | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-184` | `OBS` | The first unread message MUST follow a separator that marks it. | this spec section 3; `features/ambre-spec-murmure-unread.md` | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-185` | `OBS` | A deleted message MUST show as deleted, with no text. | this spec section 3; `MURMURE-087` | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-186` | `OBS` | An edited message MUST show that it was edited. | this spec section 3; `MURMURE-084` | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-187` | `INV` | The list MUST paint only the messages near the viewport. | this spec section 3; `ambre-spec-budgets.md section 2` | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-188` | `INV` | The list MUST be canvas content, its text shaped by the shared text pipeline. | this spec section 3; `ambre-spec-ui.md section 2` | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-207` | `OBS` | The focused message MUST be announced to a screen reader with its author, its time and its text. | this spec section 3; `ambre-spec-accessibility.md section 2`; `CHROME-023` | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-208` | `OBS` | A new message MUST be announced to a screen reader when the user was at the bottom of the list. | this spec section 3; `ambre-spec-accessibility.md section 2` | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 A message arrives

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-189` | `POST` | A new message MUST scroll into view when the user was at the bottom of the list. | this spec section 3 | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-190` | `FRAME` | A new message MUST NOT move the list when the user was reading higher up. | this spec section 3 | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-209` | `FRAME` | Announcing a message MUST NOT move the focus. | this spec section 3; `CHROME-065` | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Scroll up

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-191` | `POST` | Reaching the oldest loaded message MUST ask the engine for the previous segment. | this spec section 3; `MURMURE-023` | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-192` | `FRAME` | Loading older messages MUST NOT move the messages on screen. | this spec section 3 | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |


### 1.4 Move through the messages

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-205` | `POST` | Up and Down in the list MUST move the focus from one message to the next. | this spec section 3 | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-206` | `FRAME` | Moving the focus MUST NOT move a read cursor. | this spec section 3 | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.5 A segment cannot be opened

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-218` | `FAIL` | Given a segment whose key this client does not hold, the list MUST show one band in its place, naming its time span, the cause and the remedy. | this spec section 3; `ambre-spec-murmure-schema.md section 3` | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-219` | `FAIL` | Given a segment older than the member's history boundary, the list MUST show the boundary as the start of the channel, never as a failure. | this spec section 3; `features/ambre-spec-murmure-channels.md` | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-220` | `FAIL` | Given a segment that fails to open under the key it names, the list MUST show it as damaged, never as a missing key. | this spec section 3; `SYNC-045` | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-221` | `FRAME` | A segment the client cannot open MUST NOT stop the list from loading the segments before it. | this spec section 3 | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the frame budget of `ambre-spec-budgets.md section 2` holds the scroll, and the
  memory rows of section 6 hold the loaded segments.
* **Boundary.** none: the list paints what the engine decrypted, and holds no key. A band shows
  only what `meta` already tells every member: the span and the count of a segment.
* **Verification.** 20 `unimplemented`: no Murmure view exists.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | Alice sees | Test |
|---|---|---|---|
| reads the bottom of the channel | posts | Bob's message scrolls into view (`MURMURE-189`) | none |
| reads last week, higher up | posts | the list stays where she reads (`MURMURE-190`) | none |
| scrolls past the oldest loaded message | -- | older messages appear above, and nothing on screen jumps (`MURMURE-192`) | none |
| posts three messages within a minute | -- | one header over the three (`MURMURE-183`) | none |
| opens a channel on a new phone, before its keys arrive | -- | one band per unreadable segment, with its dates and whom to ask (`MURMURE-218`) | none |
| joins a channel invited without history | -- | the boundary at the top, and no error (`MURMURE-219`) | none |

---

## 3. MODEL

**CRDT containers.** none of its own: the list reads the messages of the loaded segments through
the engine.

**Virtualized in one dimension.** The list keeps the height of each message it measured, and paints
the messages that cross the viewport plus a margin. The scroll anchor is a message, not a pixel
offset, so a load above keeps the anchor in place (`MURMURE-192`).

```rust
pub struct ScrollAnchor { pub message: MessageRef, pub offset: PixelOffset }
pub struct PixelOffset(f32);
pub struct RowHeight(f32);
```

**Canvas and DOM.** The messages are canvas content, shaped by the text pipeline every application
uses (`ambre-spec-ui.md section 2`). An image and an animated GIF are painted in place
(`features/ambre-spec-murmure-attachments.md`).

**A screen reader** reaches the list through the mirror of `ambre-spec-accessibility.md section 2`:
the live region carries the focused message, with its author and its time (`MURMURE-207`). A new
message at the bottom is announced there too, politely, and the focus stays put (`MURMURE-209`).

**A segment that cannot be opened** keeps its place. The `segments` list of `meta` gives its time
span and its message count, so the band says which messages it stands for. The engine names the
cause; the list names the remedy.

```rust
pub enum Unreadable {
    KeyNotReceived,         // no key for this segment reached this client
    BeforeHistoryBoundary,  // the member was invited without this history
    Damaged,                // the key it names is held, and the segment does not open
}
```

| Cause | The band says | Remedy |
|---|---|---|
| `KeyNotReceived` | the keys for these messages have not reached this device | ask a member of the channel to share the history |
| `BeforeHistoryBoundary` | the history before this date was not shared with you | none: the band is the start of the channel |
| `Damaged` | these messages could not be verified | retry, which fetches the segment again from the relay |

A bare "unable to decrypt" names no cause and no remedy, so no band shows it (`MURMURE-218`).
A band is one row of the list, whatever the segment holds, so scrolling goes on past it
(`MURMURE-221`).

**Code.** not built.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the wheel, a drag or a key | the list scrolls |
| the pointer over a message | its actions: react, reply, edit and delete when the message is one's own |
| the new-messages separator | shown once, at the first unread message |
| a band of unreadable messages | its cause and its remedy, read out to a screen reader as one row |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| Messages on the canvas | a DOM list | a DOM node per message breaks the frame budget on a channel of years, and the other applications already paint their content on the canvas |
| A message as the scroll anchor | a pixel offset | a load above shifts every offset, and the reader loses the line |
| A band per unreadable segment | one line per message that did not decrypt | the whole segment shares one key, so one cause covers it, and a thousand identical lines bury the readable ones |
| A band that names cause and remedy | a bare "unable to decrypt" | Element users rank that state among their main complaints: it says nothing they can act on |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing is built.** No Murmure view exists.
* **Only the focused message reaches a screen reader.** The list is not mirrored whole, by the rule
  of `ambre-spec-accessibility.md section 2`, so a reader moves through it message by message.
