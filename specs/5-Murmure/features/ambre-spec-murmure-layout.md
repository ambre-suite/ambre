# AMBRE -- FEATURE: THE MURMURE LAYOUT
**Type:** Vertical (feature)
**Implementation:** design-only

> Murmure shows the channels, the conversation and a thread side by side on a desktop, and one of
> them at a time on a phone. The layout follows the three widths every application shares.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-158` | `OBS` | Over the desktop width, the sidebar, the message list and an open thread MUST sit side by side. | this spec section 3; `ambre-spec-adaptive-layout.md section 2` | not built | unbuilt -- lands with the `The Murmure layout and keyboard` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-159` | `OBS` | Between the phone and the desktop widths, an open thread MUST replace the message list, with a way back to it. | this spec section 3; `ambre-spec-adaptive-layout.md section 2` | not built | unbuilt -- lands with the `The Murmure layout and keyboard` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-160` | `OBS` | Under the phone width, Murmure MUST show one view at a time: the channel list, a channel, or a thread. | this spec section 3; `ambre-spec-adaptive-layout.md section 2` | not built | unbuilt -- lands with the `The Murmure layout and keyboard` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-161` | `INV` | A region MUST collapse at a narrow width, never disappear. | this spec section 3; `CHROME-033` | not built | unbuilt -- lands with the `The Murmure layout and keyboard` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-162` | `INV` | The layout MUST read the three widths of the adaptive layout, never a device class. | this spec section 3; `CHROME-034` | not built | unbuilt -- lands with the `The Murmure layout and keyboard` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-163` | `OBS` | The sidebar MUST group the public channels of each space by category, then list the private channels and the direct messages. | this spec section 3 | not built | unbuilt -- lands with the `The Murmure layout and keyboard` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-164` | `OBS` | A channel to join MUST be shown apart from the joined channels. | this spec section 3; `features/ambre-spec-murmure-spaces.md` | not built | unbuilt -- lands with the `The Murmure layout and keyboard` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-165` | `OBS` | A channel with activity MUST show a mark, and its count once the count is known. | this spec section 3; `features/ambre-spec-murmure-unread.md` | not built | unbuilt -- lands with the `The Murmure layout and keyboard` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Select a channel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-166` | `POST` | Selecting a channel MUST show it in the message list and mark it current in the sidebar. | this spec section 3 | not built | unbuilt -- lands with the `The Murmure layout and keyboard` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-167` | `FRAME` | Selecting a channel MUST NOT move the read cursor of the channel left behind. | this spec section 3; `MURMURE-116` | not built | unbuilt -- lands with the `The Murmure layout and keyboard` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Go back on a phone

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-168` | `POST` | Back from a thread MUST show its channel at the root message; back from a channel MUST show the channel list. | this spec section 3 | not built | unbuilt -- lands with the `The Murmure layout and keyboard` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-169` | `FRAME` | Going back MUST NOT move a read cursor. | this spec section 3 | not built | unbuilt -- lands with the `The Murmure layout and keyboard` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** none of its own: the chrome regions are DOM, and the message list holds the frame
  budget of `ambre-spec-budgets.md section 2`.
* **Boundary.** none: the width of a window never leaves the client.
* **Verification.** 12 `unimplemented`: no Murmure view exists.

---

## 2. WHAT EACH USER SEES

| Alice does | At the width | Alice sees | Test |
|---|---|---|---|
| opens a thread | desktop | the sidebar, the channel and the thread, side by side (`MURMURE-158`) | none |
| opens a thread | tablet | the sidebar and the thread, with a way back to the channel (`MURMURE-159`) | none |
| opens a thread, then goes back | phone | the thread alone, then the channel at the root message (`MURMURE-168`) | none |
| widens the window from phone to desktop | phone, then desktop | the same channel, now with the sidebar beside it (`MURMURE-161`) | none |

---

## 3. MODEL

**CRDT containers.** none. The layout holds view state, which no peer shares.

**The three regions, per width.** The widths are those of `ambre-spec-adaptive-layout.md section 2`.

| Region | Phone | Tablet | Desktop |
|---|---|---|---|
| the sidebar | a view of its own | beside the message list, collapsible | persistent |
| the message list | a view of its own | beside the sidebar | the centre column |
| the thread | a view of its own | in place of the message list | a right column, opened on demand |

On a phone, the application switcher of the suite stays the drawer the adaptive layout declares.
The channel list is a Murmure view inside it, not a second drawer.

**The view state.**

```rust
pub enum MurmurePane {
    ChannelList,
    Channel(ChannelId),
    Thread { channel: ChannelId, root: MessageRef },
}
pub struct MessageRef { pub segment: DocumentId, pub message: MessageId }
pub struct SidebarEntry { pub channel: ChannelId, pub group: SidebarGroup, pub activity: Activity }
pub enum SidebarGroup { Category(CategoryId), Private, Direct, ToJoin }
pub enum Activity { None, Some, Count { messages: MessageCount, mentions: MentionCount } }
pub struct MessageCount(u32);
pub struct MentionCount(u32);
```

**Code.** not built.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| a channel in the sidebar | the channel opens in the message list |
| a reply count under a root | the thread opens, in the region its width gives it |
| the back control on a phone or a tablet | the previous view |
| the collapse control of the sidebar on a tablet | the sidebar folds to a strip, and stays reachable |

The sidebar and the thread panel are DOM chrome, and the message list is canvas content
(`ambre-spec-ui.md section 2`).

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| Three columns on a desktop | a rail of space icons before the sidebar | more permanent chrome, against the rule that the messaging view stays minimal |
| Three columns on a desktop | the thread over the message list | the user loses the channel while reading the thread |
| One view at a time on a phone | a drawer that slides the sidebar in | the edge gesture fights the back gesture of the browser |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing is built.** No Murmure view exists, and the adaptive chrome has no sidebar yet
  (`CHROME-033`).
* **The personal order of the sidebar is not here.** Groups and order a user chooses are the
  `The personal sidebar` Story (`ambre-roadmap.md` E5).
