# AMBRE -- FEATURE: THE MURMURE LAYOUT
**Type:** Vertical (feature)
**Implementation:** partial -- the regions, the back control, the rows of the shell and the
channel list are built. The canvas paints no message.

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
| `MURMURE-158` | `OBS` | Over the desktop width, the sidebar, the message list and an open thread MUST sit side by side. | this spec section 3; `ambre-spec-adaptive-layout.md section 2` | `ambre_app::shell::murmure_regions::a_desktop_shows_the_sidebar_the_channel_and_the_thread_side_by_side` -- the shell mounted on a thread in a frame of 1025 and 1024 pixels, each region's width read off the layout | none | verified |
| `MURMURE-159` | `OBS` | Between the phone and the desktop widths, an open thread MUST replace the message list, with a way back to it. | this spec section 3; `ambre-spec-adaptive-layout.md section 2` | `ambre_app::shell::murmure_regions::a_tablet_thread_replaces_the_channel_with_a_way_back` -- a thread in a frame of 600 and 1024 pixels shows the sidebar and the thread, and the back control shows the channel | none | verified |
| `MURMURE-160` | `OBS` | Under the phone width, Murmure MUST show one view at a time: the channel list, a channel, or a thread. | this spec section 3; `ambre-spec-adaptive-layout.md section 2` | `ambre_app::shell::murmure_regions::a_phone_shows_one_view_at_a_time` -- each pane in a frame of 360 and 599 pixels shows one region, as wide as the frame | none | verified |
| `MURMURE-161` | `INV` | A region MUST collapse at a narrow width, never disappear. | this spec section 3; `CHROME-033` | `ambre_app::shell::murmure_regions::a_region_collapses_and_never_disappears` -- the folded sidebar of a tablet is a strip that unfolds, and a phone reaches the channel list through the back control | none | verified |
| `MURMURE-162` | `INV` | The layout MUST read the three widths of the adaptive layout, never a device class. | this spec section 3; `CHROME-034` | `ambre_app::shell::murmure_regions::the_layout_reads_the_three_widths` -- a channel and a thread in frames of 599, 600, 1024 and 1025 pixels, each region flipping at the two thresholds alone | none | verified |
| `MURMURE-163` | `OBS` | The sidebar MUST group the public channels of each space by category, then list the private channels and the direct messages. | this spec section 3 | `ambre_murmure::sidebar::tests::the_sidebar_groups_public_channels_by_category_then_private_then_direct` -- the sections in order, each channel by catalogue rank; `ambre_app::shell::murmure_regions::the_sidebar_draws_its_sections_in_order` -- the shell draws the section titles and entries in that order | none | verified |
| `MURMURE-164` | `OBS` | A channel to join MUST be shown apart from the joined channels. | this spec section 3; `features/ambre-spec-murmure-spaces.md` | `ambre_murmure::sidebar::tests::a_channel_to_join_sits_apart_from_the_joined_ones` -- a section of its own, left once joined; `ambre_app::shell::murmure_regions::the_sidebar_draws_its_sections_in_order` -- the last section drawn is the one to join | none | verified |
| `MURMURE-165` | `OBS` | A channel with activity MUST show a mark, and its count once the count is known. | this spec section 3; `features/ambre-spec-murmure-unread.md` | `ambre_murmure::sidebar::tests::a_mark_shows_before_the_count_and_the_count_replaces_it` -- the mark, then the counts, kept when absent from an answer; `ambre_app::shell::murmure_regions::a_channel_shows_its_mark_then_its_counts` -- the entry draws the mark, then the message and mention counts | none | verified |
| `MURMURE-382` | `INV` | A split view MUST exist at the desktop width alone. | this spec section 3; `ambre-spec-adaptive-layout.md section 2` | not built | unbuilt -- lands with the `Two channels side by side` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-428` | `OBS` | The command bar of the shell MUST hold the header of the current channel. | this spec section 3; `ambre-spec-chrome.md section 2` | `ambre_app::shell::murmure_regions::the_command_bar_holds_the_channel_header_and_no_status_bar_shows` -- the channel name is read inside the channel header, inside the command bar; `ambre_murmure::tests::chrome_relay::a_channel_named_shows_its_name_in_the_header` -- the name an open channel learns is the one the header reads | none | verified |
| `MURMURE-429` | `OBS` | Murmure MUST show no status bar. | this spec section 3; `ambre-spec-chrome.md section 2` | `ambre_app::shell::murmure_regions::the_command_bar_holds_the_channel_header_and_no_status_bar_shows` -- the mounted shell holds no status bar, formula bar or tab strip | none | verified |
| `MURMURE-486` | `OBS` | A space the user joined no channel of MUST show its channels to join. | this spec section 3; `MURMURE-164`; `ambre-spec-server.md`, `SRV-216` | `ambre_murmure::sidebar::tests::a_space_with_no_channel_of_the_user_shows_its_channels_to_join` -- a listed space with no channel is opened once, and its catalogue fills the section to join; `ambre_app::shell::murmure_regions::a_space_with_no_channel_of_the_user_shows_its_channels_to_join` -- the shell opens the space once and draws its channels to join alone | none | verified |
| `MURMURE-488` | `OBS` | A space the user lost MUST leave the sidebar with every channel of it, and a pane showing one MUST go back to the channel list. | this spec section 3; `features/ambre-spec-murmure-spaces.md`, `MURMURE-487` | `ambre_murmure::sidebar::tests::a_space_lost_leaves_the_sidebar_with_every_channel_of_it` -- its joined, private and listed channels go, where another space and the direct messages stay; `ambre_app::shell::murmure_regions::a_space_lost_leaves_the_sidebar_with_its_channels` -- the shell leaves the channel shown and draws the direct messages alone, and a later listing shows the space again | none | verified |
| `MURMURE-489` | `OBS` | A channel the user lost, opened or not, MUST leave the sidebar of every open client, and a pane showing it MUST go back to the channel list. | this spec section 3; `features/ambre-spec-murmure-channels.md`, `MURMURE-474` | `ambre_sync::channels::tests::the_loss_of_a_document_not_held_as_another_kind_lists_the_users_channels` -- the loss of a held channel document or of one not held lists the channels again, and the loss of a held Prose document does not; `ambre_murmure::sidebar::tests::a_channel_absent_from_a_listing_leaves_the_sidebar` -- the listing replaces the channels whole; `ambre_murmure::tests::chrome_relay::a_channel_lost_while_open_leaves_the_sidebar_and_the_pane` -- the notice of an open channel resets the pane and drops the channel, and a later listing opens it again | none | verified |

### 1.2 Select a channel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-166` | `POST` | Selecting a channel MUST show it in the message list and mark it current in the sidebar. | this spec section 3 | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5). The sidebar marks the channel current, but no message list draws it | unimplemented |
| `MURMURE-167` | `FRAME` | Selecting a channel MUST NOT move the read cursor of the channel left behind. | this spec section 3; `MURMURE-116` | `ambre_app::shell::murmure_regions::selecting_a_channel_sends_nothing_for_the_one_left_behind` -- switching channels sends one open per new channel and no other command | none | verified |

### 1.3 Go back on a phone

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-168` | `POST` | Back from a thread MUST show its channel at the root message; back from a channel MUST show the channel list. | this spec section 3 | `ambre_murmure::layout::tests::back_from_a_thread_shows_its_channel_at_the_root_then_the_list` -- the pane before a thread, a channel and the list; `ambre_app::shell::murmure_regions::back_shows_the_channel_at_the_root_then_the_list` -- the back control clicked twice on a phone | none | verified |
| `MURMURE-169` | `FRAME` | Going back MUST NOT move a read cursor. | this spec section 3 | `ambre_app::shell::murmure_regions::back_shows_the_channel_at_the_root_then_the_list` -- the back control writes the pane alone; structural -- going back is a value of the pane, and the shell holding the control holds no command sender | none | verified |

### 1.4 Show two channels side by side

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-383` | `POST` | Opening a channel beside the current one MUST show both, each with its message list and its composer. | this spec section 3 | not built | unbuilt -- lands with the `Two channels side by side` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-384` | `POST` | A command MUST move the focus from one pane to the other. | this spec section 3; `MURMURE-170` | not built | unbuilt -- lands with the `Two channels side by side` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-385` | `POST` | Closing a pane MUST give its width back to the other. | this spec section 3 | not built | unbuilt -- lands with the `Two channels side by side` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-386` | `FRAME` | A split view MUST NOT hold in view more than the documents of the two channels it shows. | this spec section 3; `RESID-003` | not built | unbuilt -- lands with the `Two channels side by side` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** none of its own: the chrome regions are DOM, and the message list holds the frame
  budget of `ambre-spec-budgets.md section 2`.
* **Boundary.** none: the width of a window never leaves the client.
* **Verification.** 16 `verified`, 6 `unimplemented`. The regions, the back control, the rows of
  the shell and the sidebar entries are built. The message list and the split view are not.

---

## 2. WHAT EACH USER SEES

| Alice does | At the width | Alice sees | Test |
|---|---|---|---|
| opens a thread | desktop | the sidebar, the channel and the thread, side by side (`MURMURE-158`) | `ambre_app::shell::murmure_regions::a_desktop_shows_the_sidebar_the_channel_and_the_thread_side_by_side` |
| opens a thread | tablet | the sidebar and the thread, with a way back to the channel (`MURMURE-159`) | `ambre_app::shell::murmure_regions::a_tablet_thread_replaces_the_channel_with_a_way_back` |
| opens a thread, then goes back | phone | the thread alone, then the channel at the root message (`MURMURE-168`) | `ambre_app::shell::murmure_regions::back_shows_the_channel_at_the_root_then_the_list` |
| widens the window from phone to desktop | phone, then desktop | the same channel, now with the sidebar beside it (`MURMURE-161`) | `ambre_app::shell::murmure_regions::the_layout_reads_the_three_widths` |

---

## 3. MODEL

**CRDT containers.** none. The layout holds view state, which no peer shares.

**The three regions, per width.** The widths are those of `ambre-spec-adaptive-layout.md section 2`.

| Region | Phone | Tablet | Desktop |
|---|---|---|---|
| the sidebar | a view of its own | beside the message list, collapsible | persistent |
| the message list | a view of its own | beside the sidebar | the centre column |
| the thread | a view of its own | in place of the message list | a right column, opened on demand |

**Two channels side by side.** Over the desktop width, a second channel opens beside the first,
with its own list and composer (`MURMURE-383`). The window narrows, and the second pane closes
(`MURMURE-382`). Each pane keeps its documents in view, and no more (`MURMURE-386`).

On a phone, the application switcher of the suite stays the drawer the adaptive layout declares.
The channel list is a Murmure view inside it, not a second drawer.

**The rows of the shell.** Murmure fills the grid of `ambre-spec-chrome.md section 2` as below.

| Row | Murmure puts there |
|---|---|
| header | the space, the sync state and the account |
| command bar | the header of the current channel: its name, its topic, the pins, the members, the call and the search (`MURMURE-428`) |
| formula bar | nothing: the row is absent |
| content | one canvas cell per list of messages, each with its composer under it. The sidebar is the sidebar region of the chrome, beside the content |
| tab strip | nothing: the row is absent |
| status bar | nothing: the row is absent (`MURMURE-429`). The frame read-out opens from the palette, in the overlay layer of the list (`CANVAS-016`) |

**The view state.**

```rust
pub enum MurmurePane {
    ChannelList,
    Channel { channel: ChannelId, at: Option<MessageRef> },
    Thread { channel: ChannelId, root: MessageRef },
}
pub struct MessageRef { pub segment: SegmentId, pub message: MessageId }
pub struct SidebarSection { pub group: SidebarGroup, pub title: Option<String>, pub entries: Vec<SidebarEntry> }
pub struct SidebarEntry { pub channel: ChannelId, pub name: Option<String>, pub activity: Activity }
pub enum SidebarGroup { Category { space: SpaceId, category: Option<CategoryId> }, Private, Direct, ToJoin }
pub enum Activity { None, Some, Count { messages: UnreadCount, mentions: MentionCount } }
```

The sidebar is rebuilt from what the Logic thread hands the view. That is the channels of the user,
the catalogue of each space they name, the activity answer and the names of open channels. A space
named for the first time is opened for its catalogue. Drawing the sidebar sends nothing.
Selecting a joined channel opens it once, and selecting one to join joins it in its space.

A channel keeps the message it shows, so back from a thread lands on its root (`MURMURE-168`).
The pane names the region in focus, and the stylesheet alone decides which regions sit beside it.

**Code.** `crates/ambre-murmure/src/layout.rs` holds the pane, and `crates/ambre-murmure/src/sidebar.rs`
the sidebar. `crates/ambre-murmure/src/chrome.rs` maps them onto the regions of
`crates/ambre-chrome/src/app_shell.rs`, and its relay absorbs the events. The launcher mounts them in
`crates/ambre-app/src/shell/app_root.rs`, and `crates/ambre-app/styles.css` lays them out.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| a channel in the sidebar | the channel opens in the message list |
| a reply count under a root | the thread opens, in the region its width gives it |
| the back control on a phone or a tablet | the previous view |
| the collapse control of the sidebar on a tablet | the sidebar folds to a strip, and stays reachable |

The sidebar and the header of each pane are DOM chrome. Every list of messages is canvas content,
in a cell of its own (`MURMURE-421`).

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| Three columns on a desktop | a rail of space icons before the sidebar | more permanent chrome, against the rule that the messaging view stays minimal |
| Three columns on a desktop | the thread over the message list | the user loses the channel while reading the thread |
| No zoom of its own | the zoom model of the editors, with its chip | Murmure is no editor, and a chat has no page or board to fit. The browser zoom scales the chrome and the lists together (`ambre-spec-high-density.md`) |
| The channel header in the command bar | a status bar under the conversation | a chat has no count or mode to report, and the row costs a line of messages on a phone |
| One view at a time on a phone | a drawer that slides the sidebar in | the edge gesture fights the back gesture of the browser |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The canvas paints its backdrop alone.** The messages land with the
  `The message list and the composer` Story (`ambre-roadmap.md` E5).
* **A private channel or a direct message not open shows no name.** Its name is in its `meta`
  document, which learning of it never fetches (`MURMURE-475`). It lands with the
  `The names of channels not open` Story (`ambre-roadmap.md` E5).
* **A space made before the relay knew spaces, and with no channel, is not listed.** Only dev data
  holds one: the relay learns a space at its creation, or from its channels.
* **The id of a Murmure route names nothing yet.** A page opens on the channel list. A link to a
  channel or a message lands with the `A link to a message` Story (`ambre-roadmap.md` E5).
* **The personal order of the sidebar is not here.** Groups, order, mute, hide and bookmarks are
  `features/ambre-spec-murmure-sidebar.md`.
