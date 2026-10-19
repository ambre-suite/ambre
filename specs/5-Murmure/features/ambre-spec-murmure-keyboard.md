# AMBRE -- FEATURE: THE MURMURE KEYBOARD
**Type:** Vertical (feature)
**Implementation:** implemented -- the command table, the palette, the jumps and the find chord.
The chords of the unread feature and the keys inside a conversation ship too.

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
| `MURMURE-170` | `INV` | Every Murmure shortcut MUST be a command with one description in the command table. | this spec section 3; `UI-THREAD-019` | `ambre_murmure::commands::tests::the_table_has_no_defect` -- no defect in the table, and the keymap binds every chord it declares and no other | none | verified |
| `MURMURE-171` | `INV` | No Murmure chord MUST take a chord the browser services. | this spec section 3; `UI-THREAD-011` | `ambre_murmure::commands::tests::no_chord_is_one_the_browser_services` -- the shared clipboard, undo and redo chords, and the chords the browser keeps from the page | none | verified |
| `MURMURE-213` | `INV` | A Murmure chord MUST keep the meaning the other applications give it: the palette chord opens the palette, the find chord opens the search. | this spec section 3; `UI-THREAD-037` | `ambre_murmure::tests::chrome_relay::the_find_chord_opens_the_search_and_its_answer_lists_the_matches` -- the find chord of every application opens the search, from a text field too; `ambre_murmure::tests::chrome_relay::the_mark_all_read_chord_runs_the_command_of_the_unread_feature` -- the palette chord opens the palette; `ambre_app::shell::murmure_regions::the_search_box_asks_lists_and_opens_a_match` -- browser tier, the box opens in the command bar and takes the caret | none | verified |

### 1.2 Switch channel from the palette

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-172` | `POST` | The palette MUST list first the channels and direct messages whose name matches what the user types. | this spec section 3 | `ambre_murmure::tests::chrome_relay::the_palette_lists_the_channels_first_in_sidebar_order` -- the entries in sidebar order, before every command, and a query that keeps two of three | none | verified |
| `MURMURE-173` | `POST` | Choosing a channel in the palette MUST open it and close the palette. | this spec section 3 | `ambre_murmure::tests::chrome_relay::choosing_a_channel_in_the_palette_opens_it` -- the channel shows and opens; `ambre_chrome::chrome_ui::palette_entry_comes_first_and_hands_back_its_key` -- the palette closes | none | verified |
| `MURMURE-174` | `FRAME` | The palette MUST NOT send what the user types outside the client. | this spec section 3; `ZK-007` | type-level -- the query is a signal of the palette alone, and its two sinks take a command id and the key of an entry, never the query | none | verified |

### 1.3 Jump to the first unread

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-175` | `POST` | The command MUST scroll the message list to the first message after the read cursor. | this spec section 3 | `ambre_murmure::view::tests::shift_page_up_brings_the_list_to_the_unread_line` -- Shift+Page Up brings the unread line into view and reads nothing, and a list with no line stays | none | verified |
| `MURMURE-176` | `POST` | The next-unread command MUST open the next channel with activity, in sidebar order. | this spec section 3 | `ambre_murmure::sidebar::tests::a_jump_opens_the_next_channel_with_activity_in_sidebar_order` -- both ways, wrapping, past the quiet channels and the ones to join; `ambre_murmure::tests::chrome_relay::a_jump_opens_the_next_active_channel_and_moves_no_read_cursor` -- through the chords | none | verified |
| `MURMURE-177` | `FRAME` | Jumping MUST NOT move a read cursor. | this spec section 3 | `ambre_murmure::tests::chrome_relay::a_jump_opens_the_next_active_channel_and_moves_no_read_cursor` -- three jumps send two channel openings and nothing else | none | verified |

### 1.4 Edit the last message

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-178` | `POST` | The up arrow in an empty composer MUST open the last message of the user in this channel for edit. | this spec section 3; `features/ambre-spec-murmure-composer.md` | `ambre_murmure::view::tests::up_in_an_empty_composer_edits_the_last_own_message` -- the latest own message opens past a deleted one and an other's, Enter sends its edit, and a draft keeps Up | none | verified |
| `MURMURE-179` | `FRAME` | The up arrow MUST NOT open a message of another member. | this spec section 3; `MURMURE-085` | `ambre_murmure::view::tests::up_never_opens_another_members_message` -- with no own message, or no member signed in, Up opens nothing and sends nothing | none | verified |

### 1.5 Mark everything read

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-180` | `POST` | The chord MUST run the mark-all-read command of the unread feature. | this spec section 3; `MURMURE-118` | `ambre_murmure::tests::chrome_relay::the_mark_all_read_chord_runs_the_command_of_the_unread_feature` -- the chord sends that one command, and a text field keeps it | none | verified |
| `MURMURE-181` | `FRAME` | Marking everything read from the keyboard MUST NOT clear the draft of the composer. | this spec section 3 | `ambre_murmure::view::tests::marking_everything_read_keeps_the_draft` -- the answers of a mark-all-read leave the draft, its focus and the send untouched; `ambre_murmure::tests::chrome_relay::the_mark_all_read_chord_runs_the_command_of_the_unread_feature` -- the chord sends that one command, which names no draft | none | verified |
| `MURMURE-522` | `POST` | Escape in the message list of a channel MUST mark that channel read. | this spec section 3; `MURMURE-115` | `ambre_murmure::view::tests::escape_in_the_list_marks_the_channel_read` -- high up in the list too, while the composer's Escape and the thread's mark nothing | none | verified |

* **Budget.** none of its own: each shortcut runs a command another feature budgets.
* **Boundary.** none: the palette filters names the client holds, and nothing it types leaves.
* **Verification.** 14 `verified`. The table, the palette, the jumps, the find chord and the keys
  inside a conversation hold.

---

## 2. WHAT EACH USER SEES

| Alice does | Alice sees | Test |
|---|---|---|
| presses the palette chord and types "bud" | the "budget" channel and a direct message named "Budget" | none |
| presses the find chord and types "budget" | the matches by channel, under what the search read (`MURMURE-213`) | `ambre_app::shell::murmure_regions::the_search_box_asks_lists_and_opens_a_match` |
| presses the next-unread chord | the next channel with activity, in sidebar order (`MURMURE-176`) | none |
| presses up in an empty composer | her last message, open for edit (`MURMURE-178`) | `ambre_murmure::view::tests::up_in_an_empty_composer_edits_the_last_own_message` |
| presses the mark-all-read chord while drafting | every badge cleared, and her draft kept (`MURMURE-181`) | `ambre_murmure::view::tests::marking_everything_read_keeps_the_draft` |

---

## 3. MODEL

**CRDT containers.** none.

**The commands.** A chord the shell routes is a row of the command table, with a label, an icon, a
chord and a group (`ambre-spec-commands.md`). A key inside a conversation is the region's, like
Page Up. It acts on the list or the composer that holds the focus, which the shell does not see.

| Command | Chord | Where it acts |
|---|---|---|
| open the palette, channels first | Ctrl+K, Cmd+K on a Mac, as in every application | anywhere in Murmure |
| open the search | Ctrl+F, Cmd+F on a Mac, as in every application | anywhere in Murmure |
| jump to the first unread message | Shift+Page Up | the message list |
| open the next channel with activity | Alt+Shift+Down | anywhere in Murmure |
| open the previous channel with activity | Alt+Shift+Up | anywhere in Murmure |
| edit the last own message | Up, in an empty composer | the composer |
| mark the channel read | Escape, in the message list | the message list |
| mark everything read | Alt+Shift+A | anywhere in Murmure |

```rust
pub struct PaletteEntry { pub key: [u8; 16], pub label: String, pub hint: Option<&'static str> }
pub enum PaletteRow<'a> { Entry(&'a PaletteEntry), Command(&'a Command) }
```

**A jump opens a channel as a click does.** It walks the sidebar order, wrapping round, past the
quiet channels and the channels to join, so it never joins one (`MURMURE-176`). It sends the
opening of the channel and nothing else (`MURMURE-177`). A text field keeps the jump chords and
the mark-all-read chord, since Alt+Shift with an arrow extends a selection on a Mac. The find chord
opens the search from any field, or the find of the browser would take it.

**The keys of a conversation.** Shift+Page Up rests the list on the unread line, a quarter down
(`MURMURE-175`). Up in an empty composer loads the member's latest message the edit action takes
(`MURMURE-178`, `MURMURE-179`). Escape in the list marks the channel read (`MURMURE-522`).

**One palette, two lists.** The channels and the direct messages are entries Murmure supplies to
the palette of the suite, so they come before the commands (`CHROME-093`). A query that starts with
`>` lists the commands alone (`CHROME-094`). The palette takes its size at each width
(`ambre-spec-adaptive-layout.md section 2`). A chord is not shown below the desktop width
(`CHROME-057`).

**Code.** `ambre_murmure::commands` holds the table. `Sidebar::next_with_activity` picks the channel
of a jump. `ambre_murmure::chrome::MurmureHandler` runs the commands and `MurmureRelay` supplies
the entries of the palette. `ambre_murmure::chrome::MessageSearch` is the search box the find
chord opens.

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
| Alt+Shift+A marks everything read | Shift+Escape, as a desktop client binds it | Chrome opens its task manager on Shift+Escape, and the page never sees the key (`MURMURE-171`) |
| The find chord opens the Murmure search | the find of the browser | the messages are drawn on a canvas, so the find of the page reads none of them |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The thread's region takes none of these keys.** Its composer waits for the
  `Replying in a thread` Story (`ambre-roadmap.md` E5).
* **Nothing checks the shared meanings across tables.** `UI-THREAD-037` reads one application's
  table at a time.
