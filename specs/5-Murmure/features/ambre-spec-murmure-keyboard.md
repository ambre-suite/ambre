# AMBRE -- FEATURE: THE MURMURE KEYBOARD
**Type:** Vertical (feature)
**Implementation:** design-only

> No frequent action in Murmure needs the mouse. From the keyboard, a member jumps to a channel by
> its name, to the first unread message and to their last message. They mark everything read.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-170` | `INV` | Every Murmure shortcut MUST be a command with one description in the command table. | this spec section 3; `UI-THREAD-019` | not built | unbuilt -- lands with the `The Murmure palette and shortcuts` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-171` | `INV` | No Murmure chord MUST take a chord the browser services. | this spec section 3; `UI-THREAD-011` | not built | unbuilt -- lands with the `The Murmure palette and shortcuts` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-213` | `INV` | A Murmure chord MUST keep the meaning the other applications give it: the palette chord opens the palette, the find chord opens the search. | this spec section 3; `UI-THREAD-037` | not built | unbuilt -- lands with the `The Murmure palette and shortcuts` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Switch channel from the palette

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-172` | `POST` | The palette MUST list first the channels and direct messages whose name matches what the user types. | this spec section 3 | not built | unbuilt -- lands with the `The Murmure palette and shortcuts` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-173` | `POST` | Choosing a channel in the palette MUST open it and close the palette. | this spec section 3 | not built | unbuilt -- lands with the `The Murmure palette and shortcuts` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-174` | `FRAME` | The palette MUST NOT send what the user types outside the client. | this spec section 3; `ZK-007` | not built | unbuilt -- lands with the `The Murmure palette and shortcuts` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Jump to the first unread

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-175` | `POST` | The command MUST scroll the message list to the first message after the read cursor. | this spec section 3 | not built | unbuilt -- lands with the `The keyboard in a conversation` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-176` | `POST` | The next-unread command MUST open the next channel with activity, in sidebar order. | this spec section 3 | not built | unbuilt -- lands with the `The Murmure palette and shortcuts` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-177` | `FRAME` | Jumping MUST NOT move a read cursor. | this spec section 3 | not built | unbuilt -- lands with the `The Murmure palette and shortcuts` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.4 Edit the last message

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-178` | `POST` | The up arrow in an empty composer MUST open the last message of the user in this channel for edit. | this spec section 3; `features/ambre-spec-murmure-composer.md` | not built | unbuilt -- lands with the `The keyboard in a conversation` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-179` | `FRAME` | The up arrow MUST NOT open a message of another member. | this spec section 3; `MURMURE-085` | not built | unbuilt -- lands with the `The keyboard in a conversation` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.5 Mark everything read

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-180` | `POST` | The chord MUST run the mark-all-read command of the unread feature. | this spec section 3; `MURMURE-118` | not built | unbuilt -- lands with the `The Murmure palette and shortcuts` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-181` | `FRAME` | Marking everything read from the keyboard MUST NOT clear the draft of the composer. | this spec section 3 | not built | unbuilt -- lands with the `The keyboard in a conversation` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** none of its own: each shortcut runs a command another feature budgets.
* **Boundary.** none: the palette filters names the client holds, and nothing it types leaves.
* **Verification.** 13 `unimplemented`: no Murmure command exists.

---

## 2. WHAT EACH USER SEES

| Alice does | Alice sees | Test |
|---|---|---|
| presses the palette chord and types "bud" | the "budget" channel and a direct message named "Budget" | none |
| presses the next-unread chord | the next channel with activity, in sidebar order (`MURMURE-176`) | none |
| presses up in an empty composer | her last message, open for edit (`MURMURE-178`) | none |
| presses the mark-all-read chord while drafting | every badge cleared, and her draft kept (`MURMURE-181`) | none |

---

## 3. MODEL

**CRDT containers.** none.

**The commands.** Each is a row of the command table, with a label, an icon, a chord and a group
(`ambre-spec-commands.md`).

| Command | Chord | Where it acts |
|---|---|---|
| open the palette, channels first | Ctrl+K, Cmd+K on a Mac, as in every application | anywhere in Murmure |
| open the search | Ctrl+F, Cmd+F on a Mac, as in every application | anywhere in Murmure |
| jump to the first unread message | Shift+Page Up | the message list |
| open the next channel with activity | Alt+Shift+Down | anywhere in Murmure |
| open the previous channel with activity | Alt+Shift+Up | anywhere in Murmure |
| edit the last own message | Up, in an empty composer | the composer |
| mark the channel read | Escape, in the message list | the message list |
| mark everything read | Shift+Escape | anywhere in Murmure |

```rust
pub struct PaletteQuery(String);         // what the user typed, never sent
pub enum PaletteEntry { Channel { channel: ChannelId, label: String, group: SidebarGroup }, Command(CommandId) }
```

**One palette, two lists.** The channels and the direct messages are entries Murmure supplies to
the palette of the suite, so they come before the commands (`CHROME-093`). A query that starts with
`>` lists the commands alone (`CHROME-094`). The palette takes its size at each width
(`ambre-spec-adaptive-layout.md section 2`). A chord is not shown below the desktop width
(`CHROME-057`).

**Code.** not built.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| a chord of the table above | its command |
| Escape in the palette | the palette closes, and the focus returns |
| Escape while editing a message | the edit is cancelled, and the message is unchanged |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| Channels first in the suite palette | a switcher of its own on Ctrl+K | Ctrl+K opens the palette in every other application, and a second meaning in one of them breaks the habit of the suite |
| Channels first in the suite palette | channels and commands mixed in one list | a channel name among every command makes the frequent jump slower |
| Chords of the command table | key handlers in each view | two views would bind one chord to two actions, which `UI-THREAD-026` refuses |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing is built.** No Murmure command exists.
* **Nothing checks the shared meanings.** `MURMURE-213` has no mechanism, and `UI-THREAD-037`
  reads one application's table at a time.
