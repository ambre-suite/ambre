# AMBRE -- FEATURE: THE PERSONAL SIDEBAR
**Type:** Vertical (feature)
**Implementation:** design-only

> Each member arranges the channel list for themselves: groups and order of their own, channels
> muted or hidden, and messages bookmarked. None of it touches what the other members see, and all
> of it follows the member to every device.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-275` | `INV` | A muted channel MUST show no unread count. | this spec section 3 | not built | unbuilt -- lands with the `Mute and hide` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-298` | `INV` | A muted channel MUST still show the mentions of the user. | this spec section 3 | not built | unbuilt -- lands with the `Mute and hide` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-276` | `INV` | A hidden channel MUST stay a channel the user belongs to. | this spec section 3 | not built | unbuilt -- lands with the `Mute and hide` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-277` | `INV` | The personal groups, the order and the bookmarks MUST be the same on every device of the user. | this spec section 3; `SETTINGS-012` | not built | unbuilt -- lands with the `The personal sidebar` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-282` | `OBS` | The hide action MUST say that the user stays a member, and offer to leave instead. | this spec section 3 | not built | unbuilt -- lands with the `Mute and hide` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Mute a channel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-278` | `POST` | Muting a channel MUST set its notification level to mentions. | this spec section 3; `MURMURE-268` | not built | unbuilt -- lands with the `Mute and hide` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-279` | `FRAME` | Muting a channel MUST NOT change its membership. | this spec section 3 | not built | unbuilt -- lands with the `Mute and hide` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Hide a channel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-280` | `POST` | Hiding a channel MUST remove it from the sidebar. | this spec section 3 | not built | unbuilt -- lands with the `Mute and hide` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-281` | `POST` | A mention of the user in a hidden channel MUST show the channel again. | this spec section 3 | not built | unbuilt -- lands with the `Mute and hide` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-283` | `FRAME` | Hiding a channel MUST NOT change its membership. | this spec section 3 | not built | unbuilt -- lands with the `Mute and hide` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.4 Arrange the sidebar

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-284` | `POST` | Moving a channel MUST store its personal group and its position in the user settings. | this spec section 3; `SETTINGS-004` | not built | unbuilt -- lands with the `The personal sidebar` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-285` | `POST` | A personal group MUST be created, renamed and removed by its user alone. | this spec section 3 | not built | unbuilt -- lands with the `The personal sidebar` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-286` | `FRAME` | Arranging the sidebar MUST NOT change the catalogue of a space. | this spec section 3; `MURMURE-032` | not built | unbuilt -- lands with the `The personal sidebar` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.5 Bookmark a message

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-287` | `POST` | Bookmarking a message MUST store its channel, its segment and its identifier in the user settings. | this spec section 3 | not built | unbuilt -- lands with the `The personal sidebar` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-288` | `FAIL` | Given a bookmark whose message was deleted or expired, or whose channel the user lost, the list MUST show it as gone. | this spec section 3 | not built | unbuilt -- lands with the `The personal sidebar` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-289` | `FRAME` | Bookmarking a message MUST NOT change the message. | this spec section 3 | not built | unbuilt -- lands with the `The personal sidebar` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** none of its own: every entry is a small value in the settings document, which the
  row of `ambre-spec-budgets.md section 6` bounds.
* **Boundary.** every choice here lives in the sealed settings document (`SETTINGS-001`). The one
  that leaves it is the notification level of a muted channel, whose copy `ZK-016` bounds.
* **Verification.** 16 `unimplemented`: no settings document exists.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The settings of Alice hold | Alice sees | Test |
|---|---|---|---|---|
| mutes a busy channel | posts in it | the level mentions | no count, no push (`MURMURE-275`, `MURMURE-278`) | none |
| mutes that channel | mentions her in it | the level mentions | the mention badge, and a push (`MURMURE-298`) | none |
| hides a channel | mentions her in it | the channel hidden | the channel back in the sidebar (`MURMURE-281`) | none |
| moves a channel to a group "Clients" on her laptop | -- | the group and the position | the same group on her phone (`MURMURE-277`) | none |
| bookmarks a message | deletes that message | the bookmark | the bookmark marked gone (`MURMURE-288`) | none |

---

## 3. MODEL

**CRDT containers.** the `murmure` section of the user settings, in
`ambre-spec-murmure-schema.md section 3`: `layout`, `groups` and `bookmarks`. The level of a muted
channel lives in `notify`, owned by `features/ambre-spec-murmure-notifications.md`.

**Mute, hide and leave are three acts.**

| Act | The channel | The key | Traffic | Where it lives |
|---|---|---|---|---|
| mute | stays in the sidebar, dimmed, with its mentions | kept | received | `notify`, set to mentions |
| hide | leaves the sidebar until a mention | kept | received | `layout`, `hidden` |
| leave | leaves every list | rotated for the others | none | the membership log |

A user who hides a channel and believes they left still receives its traffic, so the hide action
says so and offers to leave (`MURMURE-282`).

**Personal groups.** A group is the user's own, named by them, and orders the channels they put in
it. It sits beside the categories of a space, which the space admins own. Arranging the sidebar never
writes the catalogue (`MURMURE-286`).

```rust
pub struct PersonalGroupId(Uuid);       // minted by the user's client
pub enum SidebarGroup { Category(CategoryId), Personal(PersonalGroupId), Private, Direct, ToJoin }
```

**A bookmark is personal, and a pin is shared.** A bookmark names the channel, the segment and the
message, so the client loads that segment alone to show it (`MURMURE-287`). A pin is the
`Pinned messages` Story (`ambre-roadmap.md` E5).

**Code.** not built.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| mute, in the channel menu | the channel dims and stops counting |
| hide, in the channel menu | a notice that the user stays a member, with leave as the other choice |
| a drag in the sidebar | the channel moves, within a group or to another |
| new group, in the sidebar menu | a group named by the user |
| bookmark, on a message | the message joins the bookmarks view |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The personal state lives in the user settings | local storage in the browser | a new device starts empty, and a new device is where the user needs the arrangement most |
| The personal state lives in the user settings | a Murmure document of its own | one user would hold two sealed stores and two sources of truth |
| Mute keeps the mentions | mute silences everything | a user who mutes a busy room still wants to know when someone asks them directly |
| Hide says the user stays a member | hide and leave as one action | a hidden channel keeps its key and its traffic, which a user who meant to leave would not expect |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing is built.** No settings document exists, so no choice here is kept.

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| How the settings document is found and shared out | `ambre-spec-user-settings.md` |
| What a notification level does | `features/ambre-spec-murmure-notifications.md` |
| How a member leaves a channel | `features/ambre-spec-murmure-channels.md` |
| Where the sidebar sits at each width | `features/ambre-spec-murmure-layout.md` |
