# AMBRE -- FEATURE: THE SLASH COMMAND

**Type:** Vertical (feature)
**Implementation:** partial -- the twelve items are built; the other features' items are not

> Type a slash at the start of an empty line and a short list appears: a heading, a list, a table,
> a page break. Keep typing to narrow it, press Enter to insert.

The menu inserts nothing itself: it chooses a command and the block features carry out the rest --
`features/ambre-spec-prose-headings.md`, `features/ambre-spec-prose-lists.md`,
`features/ambre-spec-prose-tables.md`, `features/ambre-spec-prose-table-of-contents.md`.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-059` | `INV` | The slash and the text typed after it MUST reach the document like any other text, so a peer reading the block sees what the author typed. | this spec section 3 | `ambre_prose::input_tests::the_slash_and_the_query_reach_the_document_as_text`; `::enter_erases_the_query_then_sends_one_command` | none | verified |
| `PROSE-UI-087` | `INV` | No state of the menu -- its origin, its query, its selection -- MUST be written to the document. | this spec section 3 | `ambre_prose::input_tests::moving_through_the_menu_sends_nothing` | none | verified |
| `PROSE-UI-062` | `INV` | Every item MUST resolve to a command the plane already declares. | this spec section 3; `features/ambre-spec-prose-engine.md section 3` | `ambre_prose::input_tests::every_item_is_a_command_the_plane_carries_out` | none | verified |
| `PROSE-UI-088` | `INV` | The menu MUST NOT carry a command of its own. | this spec section 5 | `ambre_prose::input_tests::every_item_is_a_command_the_plane_carries_out` | none | verified |
| `PROSE-UI-089` | `INV` | The menu MUST send a command only when an item is confirmed; no other key and no pointer gesture inside it MUST send one. | this spec section 3 | `ambre_prose::input_tests::moving_through_the_menu_sends_nothing` | none | verified |
| `PROSE-UI-090` | `OBS` | The menu MUST be drawn only for the author who opened it; a peer MUST see the typed text and nothing else. | this spec section 3 | `ambre_prose::input_tests::a_peer_sees_the_slash_and_no_menu` | none | verified |
| `PROSE-UI-194` | `OBS` | The query MUST keep the items whose label holds it, case folded, in the order of the item table. | this spec section 3 | `ambre_prose::input_tests::the_query_narrows_the_items_in_order` | none | verified |

### 1.2 Open the menu

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-058` | `POST` | The menu MUST open only where a block command is legal: at the start of a block, on an empty block or before its first character. | this spec section 3 | `ambre_prose::input_tests::a_slash_opens_the_menu_only_at_the_start_of_a_block` | none | verified |
| `PROSE-UI-091` | `POST` | Opening the menu MUST record the block the slash was typed in and the offset of the slash. | this spec section 3 | `ambre_prose::input_tests::the_slash_and_the_query_reach_the_document_as_text` | none | verified |
| `PROSE-UI-174` | `POST` | The chosen command MUST be addressed to the block the menu recorded. | this spec section 3 | `ambre_prose::input_tests::enter_erases_the_query_then_sends_one_command` | none | verified |
| `PROSE-UI-092` | `FRAME` | Opening the menu MUST NOT write to the document: the slash reaches it through the ordinary input path, and nothing else does. | derived | `ambre_prose::input_tests::the_slash_and_the_query_reach_the_document_as_text` | none | verified |

### 1.3 Insert the chosen block

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-060` | `POST` | The slash and the query MUST be removed from the block before the chosen block command is sent. | this spec section 3 | `ambre_prose::input_tests::enter_erases_the_query_then_sends_one_command` | none | verified |
| `PROSE-UI-093` | `POST` | The command the chosen item resolves to MUST be sent once. | this spec section 3 | `ambre_prose::input_tests::enter_erases_the_query_then_sends_one_command` | none | verified |
| `PROSE-UI-175` | `POST` | Choosing an item MUST close the menu. | this spec section 3 | `ambre_prose::input_tests::enter_erases_the_query_then_sends_one_command` | none | verified |
| `PROSE-UI-193` | `POST` | A remote edit before the slash MUST move the recorded offset with the text, so the deletion takes the slash and the query and nothing else. | this spec section 3; `PROSE-UI-060` | `ambre_prose::input_tests::a_remote_insertion_before_the_slash_moves_the_origin` | none | verified |
| `PROSE-UI-195` | `POST` | A picture, a footnote, a cross reference, a comment and an equation MUST each be reachable from an item. | `features/ambre-spec-prose-images.md section 4` | not built | unbuilt -- lands with the `The slash menu's other items` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-196` | `POST` | Every heading level the document model carries MUST be reachable from an item. | `ambre-spec-prose-schema.md section 3` | not built | unbuilt -- lands with the `The slash menu's other items` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-094` | `FRAME` | Inserting MUST NOT touch any block other than the origin, and MUST NOT remove text the author typed before the slash. | derived | `ambre_prose::input_tests::an_item_touches_the_origin_block_alone` | none | verified |

### 1.4 Close the menu

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-061` | `POST` | The menu MUST close on the deletion of the slash, on a space, on Escape and on a pointer press outside it. | this spec section 3 | `ambre_prose::input_tests::closing_the_menu_leaves_the_text_as_typed` | none | verified |
| `PROSE-UI-095` | `FRAME` | Closing the menu MUST NOT write to the document: the slash and the query stay exactly as the author typed them. | this spec section 5 | `ambre_prose::input_tests::closing_the_menu_leaves_the_text_as_typed` | none | verified |

* **Budget.** none of its own. Filtering runs over a list of twelve items on a keystroke, inside
  the keystroke budget the draft buffer already owns
  (`features/ambre-spec-prose-text-editing.md`, `ambre-spec-budgets.md section 1`).
* **Boundary.** none: the menu reads view state and emits commands. It holds no key and opens no
  socket, and every mutation it starts is a `ProseCommand` (`PROSE-UI-002`).
* **Verification.** 18 `verified`, 0 `untested`, 2 `unimplemented`.
  The two open rows are items the menu does not carry yet. Every built row runs on the host
  against the view state and the Logic thread, with no browser.
  `PROSE-UI-059` and `PROSE-UI-060` share a test, since each alone is met by a design that
  loses the other. The DOM element renders `SlashMenuState` and holds no rule of its own.
---

## 2. WHAT EACH USER SEES

The menu is local; what the document records is the typed text, then the block the item asked for.
The last column names the test, or the row a scenario rests on when no test resolves it.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| types `/` on an empty block | watches | the slash, as text | Alice's menu; Bob sees a slash | `ambre_prose::input_tests::a_peer_sees_the_slash_and_no_menu` |
| types `tab` after it | watches | `/tab` | Alice's menu narrowed; Bob sees `/tab` | `ambre_prose::input_tests::the_slash_and_the_query_reach_the_document_as_text` |
| presses Enter on "Table" | watches | the block emptied, then a table | a table on both screens, and no leftover slash | `ambre_prose::input_tests::enter_erases_the_query_then_sends_one_command` |
| presses Escape | watches | `/tab`, untouched | the menu gone, the text still there | `ambre_prose::input_tests::closing_the_menu_leaves_the_text_as_typed` |
| types `/` and picks a heading | types in the same block at the same moment | both edits, converged by the text CRDT | a heading holding both authors' text | none -- `PROSE-089` |

---

## 3. MODEL

**Containers.** This feature owns none. It removes a range of one block's text and then sends one
block command.

**The state the view publishes.** The menu is a DOM element on the shared menu primitives, and the
canvas view is what knows where the caret is. The view publishes a state, and the element renders
it (`ambre-spec-ui.md section 2`).

```rust
// ambre-prose/src/view/slash.rs -- published to the chrome component, never drawn on the canvas

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SlashMenuState {
    /// Under the slash, in canvas space. `None` hides the menu.
    pub anchor:   Option<ViewPoint>,
    pub query:    String,
    /// The labels the query keeps (`PROSE-UI-194`).
    pub items:    SlashChoiceVec<&'static str>,
    pub selected: SlashChoice,
}

/// What the view keeps while the menu is open. Never published, never written.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SlashMenu {
    /// The block the slash was typed in, and the offset of the slash itself. A remote edit
    /// moves the offset with the text (`PROSE-UI-193`).
    origin:   Spot,
    query:    String,
    selected: SlashChoice,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlashItem {
    pub label:   &'static str,
    pub command: SlashCommand,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlashCommand {
    Paragraph,
    Heading(HeadingLevel),
    List(ListStyle),
    Table,
    Code,
    Rule,
    PageBreak,
    Toc,
}
```

**The menu is optimistic, and the text is authoritative.** The slash opens the menu and still flows
down the ordinary input pipe, so the document records it (`PROSE-UI-059`). Nothing about the menu
is in the document (`PROSE-UI-087`), so a peer sees a slash being typed and then erased -- which is
what happened.

**Steps, from the slash to the block:**

1. a `/` typed at the start of a block opens the menu, and the block and offset are remembered as
   the origin (`PROSE-UI-058`);
2. every character after it appends to the query and to the block alike, and the item list is
   filtered on the query;
3. the arrow keys move the selection, and nothing is sent (`PROSE-UI-089`);
4. Enter, or a press on an item, deletes the slash and the query from the origin block. One
   `DeleteText` covers the range the menu spans (`PROSE-UI-060`);
5. the item's command is sent, addressed to the origin block (`PROSE-UI-174`).

**The filter.** An item survives when its label holds the query, case folded. The survivors keep
the order of the item table below (`PROSE-UI-194`). A query no item holds leaves the list empty,
and Enter then splits the block as it would with no menu.

**Where it may open.** A slash in the middle of a sentence is a slash, not a menu. The menu opens
only where inserting a block is meaningful: on an empty block, or before the block's first
character (`PROSE-UI-058`). That block is a paragraph or a heading of the flow. A band, a note, a
table cell and a list item open none.

**What closes it.** The Backspace that erases the slash, a space, Escape, and any pointer press
outside the menu. Each of the four leaves the typed text exactly as it is: closing the menu is not
an undo (`PROSE-UI-095`).

**The twelve items, and the command each one resolves to** (`features/ambre-spec-prose-engine.md section 3`):

| Item | Command |
|---|---|
| Text | `InsertBlock` with the paragraph initializer, after the origin block |
| Heading 1, Heading 2, Heading 3 | `SetHeadingLevel`, on the origin block |
| Bullet list, Numbered list, Task list | `InsertBlock` with the list-item initializer and the chosen style, after the origin block |
| Table | `InsertTable`, three by three, after the origin block |
| Code | `InsertBlock` with the code initializer, no language, after the origin block |
| Separator, Page break | `InsertBlock` with the rule and the break initializers, after the origin block |
| Table of contents | `InsertToc`, three levels, unnumbered, after the origin block |

**Code:** `crates/ambre-prose/src/view/slash.rs` holds the items, the filter and the state.
`crates/ambre-prose/src/input.rs` holds the keys and the text the menu watches. `ProseSlashMenu`
in `crates/ambre-prose/src/chrome/mod.rs` is the element.

---

## 4. SURFACE

| Trigger | What happens |
|---|---|
| `/` at the start of a block | the menu opens under the caret |
| a character after it | the list narrows on the query |
| the arrow keys | move the selection in the list |
| Enter | the slash and the query leave the block, then the item's command is sent |
| Escape, a space, the Backspace over the slash, a press outside | the menu closes and the text stays |

The element, its keyboard order and its flip at the edge of the window are the shared chrome
toolkit's, and are not respecified here (`ambre-spec-chrome.md`).

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The typed text reaches the document | intercepting the keystrokes at the menu | it forks the input pipeline for one feature: a menu that swallows keystrokes has to reimplement composition, undo and the draft buffer. Letting the text through costs a visible slash for a moment and keeps one editing path |
| The slash and the query are deleted before the command | leaving the query and letting the block command overwrite it | it depends on the two arriving in order. The author asked for a table, not for `/tab` followed by a table, and deleting first means the block command lands on a block whose text is what the author wanted |
| The menu opens only at the start of a block | opening on any slash | it is what makes the feature an irritation rather than a shortcut. A slash is a legitimate character everywhere else -- a date, a path, a fraction -- and the author has to dismiss the menu all day |
| Closing writes nothing | erasing the query on close | it loses text the author typed on purpose. Four gestures close the menu and none of them is a request to change the document, and deleting on one of them makes Escape destructive, which no other Escape in the application is |
| Every item is an existing command | giving the menu a command of its own for the composite items | it puts document logic in the chrome. The menu is a surface, and a surface that mints its own command is a second dispatch path the convergence rules never covered |
| The deletion and the command are two commands | one command that erases the query and inserts the block | it is a command of the menu's own (`PROSE-UI-088`). A peer's text typed into the query between the two is kept, as every concurrent insertion is: the author loses nothing and sees what the peer typed |

---

## 6. NOT BUILT, AND NOT YET RULED

Nothing. The items the menu does not reach yet are rows of section 1, bound to their Story.
