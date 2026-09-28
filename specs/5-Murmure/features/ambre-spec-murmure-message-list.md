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
| `MURMURE-417` | `INV` | The list MUST scroll through the shared kinetic model, never through a speed or a decay of its own. | this spec section 3; `KINETIC-001` | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-418` | `OBS` | The time of a message MUST show in the reader's own language and time zone. | this spec section 3 | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-419` | `INV` | Every action on a message MUST be a command of the command table. | this spec section 3; `UI-THREAD-019` | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-420` | `INV` | A thread MUST show its messages through the same list as a channel. | this spec section 3; `features/ambre-spec-murmure-threads.md` | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-421` | `INV` | Each region that shows messages MUST be a canvas of its own, one cell of the chrome layout. | this spec section 3; `CHROME-009`; `CANVAS-070` | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |

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

### 1.6 Act on a message

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-422` | `POST` | A right-click on a message MUST open its actions in the shared floating menu. | this spec section 3; `CHROME-004` | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-423` | `POST` | A long press on a message MUST open its actions on a touch screen. | this spec section 3; `KINETIC-003` | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-424` | `FRAME` | Opening the actions of a message MUST NOT change the message. | this spec section 3 | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.7 Select and copy text

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-425` | `POST` | A mouse drag across the messages MUST select their text, from one character to several messages. | this spec section 3; `CANVAS-037` | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-426` | `POST` | Copying a selection MUST put its text on the clipboard, each message under its author and its time. | this spec section 3; `UI-THREAD-006` | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-427` | `FRAME` | Selecting text MUST NOT move a read cursor. | this spec section 3; `MURMURE-206` | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |

**Frame:** `UI-THREAD-044`

* **Budget.** the frame budget of `ambre-spec-budgets.md section 2` holds the scroll, and the
  memory rows of section 6 hold the loaded segments.
* **Boundary.** none: the list paints what the engine decrypted, and holds no key. A band shows
  only what `meta` already tells every member: the span and the count of a segment.
* **Verification.** 31 `unimplemented`: no Murmure view exists.

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
(`features/ambre-spec-murmure-attachments.md`). The fonts, the colour emoji and the decoded images
come from `ambre-spec-render-assets.md`. The list paints through the drawing surface of
`ambre-spec-drawing-surface.md`, inside the frame loop of `ambre-spec-frame.md`.

**One list, in every place a message shows.** A channel, a thread and the second pane of a split
view each show one list (`MURMURE-420`). Each list is a canvas of its own, in its own cell of the
chrome layout (`MURMURE-421`). A list owns its scroll, its hit-test and its repaint plan. A closed
list costs nothing. The DOM around a list holds its header and its controls, never a message.

**The scroll.** The list takes its pan, its flick, its wheel momentum and its edge scroll from
`ambre-spec-kinetic-scroll.md` (`MURMURE-417`). The list owns step 4 of that model alone: it moves
its anchor by the distance of each tick.

**The time of a message.** The `ts` of a message is Unix milliseconds. The list formats it in the
language and the time zone of the reader (`MURMURE-418`). A channel carries no locale: two members
in two countries read one message at two local times.

**The actions of a message.** Each action is a command of the table of `ambre-spec-commands.md`,
with its label, its icon and its chord where it has one (`MURMURE-419`). The hover bar, the context
menu, the long press and the palette read the same description. A long press is a press held
inside the tap slop of `ambre-spec-kinetic-scroll.md`, past the delay of the platform.

| Command | Offered on |
|---|---|
| react, reply, copy link, pin, bookmark, mark unread | every message the member can read |
| edit, delete | a message of the member |
| remove | a message of another member, for a moderator |
| copy text | a message with text |

**Selection.** A mouse drag across the list selects text through the selection model of
`ambre-spec-text-editor.md` (`MURMURE-425`). On a touch screen a drag scrolls, so the copy text
action takes the whole message. The text is read-only, so the model runs with no caret.
A copy writes the `text/plain` mirror and the private type of `ambre-spec-clipboard.md`
(`MURMURE-426`). The mirror puts each message under its author and its time.

**A screen reader** reaches the list through the mirror of `ambre-spec-accessibility.md section 2`:
the live region carries the focused message, with its author and its time (`MURMURE-207`). A new
message at the bottom is announced there too, politely, and the focus stays put (`MURMURE-209`).

**A segment that cannot be opened** keeps its place. The `segments` list of `meta` gives its time
span and its message count, so the band says which messages it stands for. The engine names the
cause; the list names the remedy.

```rust
pub enum Unreadable {
    KeyNotReceived,         // the key chain did not reach the key of this segment
    BeforeHistoryBoundary,  // the member was invited without this history
    Damaged,                // the key it names is held, and the segment does not open
}
```

| Cause | The band says | Remedy |
|---|---|---|
| `KeyNotReceived` | the keys for these messages have not reached this device | retry, which fetches the key chain again from the relay |
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
| the wheel, a finger or a key | the list scrolls, and coasts after a flick |
| the pointer over a message | its main actions, in a bar over the message |
| a right-click on a message | every action of the message, in the floating menu |
| a long press on a message, on a touch screen | every action of the message, in the floating menu |
| a mouse drag across the text | a selection, and the list scrolls when the drag reaches an edge |
| the copy chord over a selection | the text on the clipboard, each message under its author and its time |
| the new-messages separator | shown once, at the first unread message |
| a band of unreadable messages | its cause and its remedy, read out to a screen reader as one row |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| Messages on the canvas | a DOM list | a DOM node per message breaks the frame budget on a channel of years, and the other applications already paint their content on the canvas |
| Messages of a thread in the canvas list | a DOM thread panel | two renderers of one message drift, and each feature of a message would be written twice |
| One canvas per list | one canvas for every list, cut into views | a canvas per cell keeps the chrome layout whole, and gives each list its own scroll and hit-test |
| Text selection drawn by the list | a transparent DOM text layer over the list | two layouts of one text drift apart, and the layer costs the DOM node per message the canvas avoids |
| A message as the scroll anchor | a pixel offset | a load above shifts every offset, and the reader loses the line |
| A band per unreadable segment | one line per message that did not decrypt | the whole segment shares one key, so one cause covers it, and a thousand identical lines bury the readable ones |
| A band that names cause and remedy | a bare "unable to decrypt" | Element users rank that state among their main complaints: it says nothing they can act on |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing is built.** No Murmure view exists.
* **Only the focused message reaches a screen reader.** The list is not mirrored whole, by the rule
  of `ambre-spec-accessibility.md section 2`, so a reader moves through it message by message.
