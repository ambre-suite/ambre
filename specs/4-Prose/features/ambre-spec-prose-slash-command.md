# AMBRE -- FEATURE: THE SLASH COMMAND

**Type:** Vertical (feature)
**Implementation:** design-only

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

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-059` | `INV` | The slash and the text typed after it MUST reach the document like any other text, so a peer reading the block sees what the author typed. | this spec section 3 | not built | unbuilt -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-087` | `INV` | No state of the menu -- its origin, its query, its selection -- MUST be written to the document. | this spec section 3 | not built | unbuilt -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-062` | `INV` | Every item MUST resolve to a command the plane already declares. | this spec section 3; `features/ambre-spec-prose-engine.md section 3` | not built | unbuilt -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-088` | `INV` | The menu MUST NOT carry a command of its own. | this spec section 5 | not built | unbuilt -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-089` | `INV` | The menu MUST send a command only when an item is confirmed; no other key and no pointer gesture inside it MUST send one. | this spec section 3 | not built | unbuilt -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-090` | `OBS` | The menu MUST be drawn only for the author who opened it; a peer MUST see the typed text and nothing else. | this spec section 3 | not built | unbuilt -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Open the menu

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-058` | `POST` | The menu MUST open only where a block command is legal: at the start of a block, on an empty block or before its first character. | this spec section 3 | not built | unbuilt -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-091` | `POST` | Opening the menu MUST record the block the slash was typed in and the offset of the slash. | this spec section 3 | not built | unbuilt -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-174` | `POST` | The chosen command MUST be addressed to the block the menu recorded. | this spec section 3 | not built | unbuilt -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-092` | `FRAME` | Opening the menu MUST NOT write to the document: the slash reaches it through the ordinary input path, and nothing else does. | derived | not built | unbuilt -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Insert the chosen block

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-060` | `POST` | The slash and the query MUST be removed from the block before the chosen block command is sent. | this spec section 3 | not built | unbuilt -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-093` | `POST` | The command the chosen item resolves to MUST be sent once. | this spec section 3 | not built | unbuilt -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-175` | `POST` | Choosing an item MUST close the menu. | this spec section 3 | not built | unbuilt -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-094` | `FRAME` | Inserting MUST NOT touch any block other than the origin, and MUST NOT remove text the author typed before the slash. | derived | not built | unbuilt -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.4 Close the menu

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-061` | `POST` | The menu MUST close on the deletion of the slash, on a space, on Escape and on a pointer press outside it. | this spec section 3 | not built | unbuilt -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-095` | `FRAME` | Closing the menu MUST NOT write to the document: the slash and the query stay exactly as the author typed them. | this spec section 5 | not built | unbuilt -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** none of its own. Filtering runs over a list of twelve items on a keystroke, inside
  the keystroke budget the draft buffer already owns
  (`features/ambre-spec-prose-text-editing.md`, `ambre-spec-budgets.md section 1`).
* **Boundary.** none: the menu reads view state and emits commands. It holds no key and opens no
  socket, and every mutation it starts is a `ProseCommand` (`PROSE-UI-002`).
* **Verification.** 16 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`.
  `PROSE-UI-058` and the item table behind `PROSE-UI-062` are host-tier facts over the block and
  the query, with no browser. `PROSE-UI-059` and `PROSE-UI-060` are the pair that has to be tested
  together: each alone is satisfied by a design that loses the other. The three frame rows are
  what a document delta answers, and they are the rows a menu that writes its own state fails.

---

## 2. WHAT EACH USER SEES

The menu is local; what the document records is the typed text, then the block the item asked for.
No test resolves yet: the last column names the row each scenario rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| types `/` on an empty block | watches | the slash, as text | Alice's menu; Bob sees a slash | none -- `PROSE-UI-090` |
| types `tab` after it | watches | `/tab` | Alice's menu narrowed; Bob sees `/tab` | none -- `PROSE-UI-059` |
| presses Enter on "Table" | watches | the block emptied, then a table | a table on both screens, and no leftover slash | none -- `PROSE-UI-060` |
| presses Escape | watches | `/tab`, untouched | the menu gone, the text still there | none -- `PROSE-UI-095` |
| types `/` and picks a heading | types in the same block at the same moment | both edits, converged by the text CRDT | a heading holding both authors' text | none -- `PROSE-089` |

---

## 3. MODEL

**Containers.** This feature owns none. It removes a range of one block's text and then sends one
block command.

**The state the view publishes.** The menu is a DOM element on the shared menu primitives, and the
canvas view is what knows where the caret is. The view publishes a state, and the element renders
it (`ambre-spec-ui.md section 2`).

```rust
// ambre-prose/src/ui/slash_menu.rs -- published to the chrome component, never drawn on the canvas

pub struct SlashMenuState {
    /// The caret at the moment the menu opened, in canvas space.
    pub anchor:   Vec2,
    /// The block the slash was typed in, and the offset of the slash itself.
    pub origin:   (TreeID, u32),
    pub query:    String,
    pub items:    Vec<SlashItem>,
    pub selected: usize,
}

pub struct SlashItem {
    pub label:   &'static str,
    pub icon:    Icon,
    pub command: SlashCommand,
}

pub enum SlashCommand {
    InsertBlock(BlockInit),
    InsertTable { rows: u32, cols: u32 },
    InsertToc,
    SetHeading(u8),
    SetCodeBlock { lang: Option<String> },
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
4. Enter deletes the slash and the query from the origin block, in one `DeleteText` over the range
   the menu spans (`PROSE-UI-060`);
5. the item's command is sent, addressed to the origin block (`PROSE-UI-091`).

**Where it may open.** A slash in the middle of a sentence is a slash, not a menu. The menu opens
only where inserting a block is meaningful: on an empty block, or before the block's first
character (`PROSE-UI-058`).

**What closes it.** The Backspace that erases the slash, a space, Escape, and any pointer press
outside the menu. Each of the four leaves the typed text exactly as it is: closing the menu is not
an undo (`PROSE-UI-095`).

**The twelve items, and the command each one resolves to** (`features/ambre-spec-prose-engine.md section 3`):

| Item | Command |
|---|---|
| Text | `InsertBlock` with the paragraph initializer |
| Heading 1, Heading 2, Heading 3 | `SetHeadingLevel`, on the origin block |
| Bullet list, Numbered list, Task list | `InsertListItem`, with the chosen style |
| Table | `InsertTable`, three by three |
| Code | `InsertBlock` with the code initializer, no language |
| Separator, Page break | `InsertBlock` with the rule and the break initializers |
| Table of contents | `InsertToc` |

**Code:** none. `crates/ambre-prose/src/lib.rs` is seven lines.

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

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing exists.** `crates/ambre-prose/src/lib.rs` is seven lines, so the menu, its filter and
  its origin are a design, and every row above is `unimplemented`.
* **The deletion and the command are two operations, and nothing binds them.** Step 4 and step 5
  are a `DeleteText` and a block command, and a peer can insert between them. The corpus declares
  no way to send the pair as one. The failure is a table appearing after a `/tab` that a remote
  edit rescued.
* **The origin is not reanchored.** The menu remembers a block and an offset, and a remote edit
  before that offset moves the text under it. `features/ambre-spec-prose-text-editing.md` reanchors
  the draft over a remote delta; nothing does the same for the menu's range.
* **The filter is undefined.** "Filtered by the query" is the whole of it. No rule says whether the
  match is a prefix, a substring or fuzzy. Nothing says whether it is case-folded, or what order
  the survivors are listed in.
* **The item list is fixed and reaches only nine features.** Twelve items name the paragraph, the
  heading, the three list styles, the table, the code block, the two rules and the table of
  contents. A picture, a footnote, a cross reference, a comment and an equation have no item, and
  `features/ambre-spec-prose-images.md section 4` names this menu as their trigger.
* **The three heading items stop at level three.** The document model carries six
  (`ambre-spec-prose-schema.md section 3`). Half the levels are reachable from the bar and the
  menu alike only through a control neither one declares.
