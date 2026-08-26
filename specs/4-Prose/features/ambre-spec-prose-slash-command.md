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

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `PROSE-UI-058` | The menu MUST open only where a block command is legal: at the start of a block, on an empty block or before its first character. | this spec section 3 | not built -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-059` | The slash and the text typed after it MUST reach the document like any other text, so a peer reading the block sees what the author typed. | this spec section 3 | not built -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-060` | The slash and the query MUST be removed from the block before the chosen block command is sent. | this spec section 3 | not built -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-061` | The menu MUST close on the deletion of the slash, on a space, on Escape and on a pointer press outside it, and closing it MUST write nothing. | this spec section 3 | not built -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-062` | Every item MUST resolve to a command the plane already declares; the menu MUST NOT carry a command of its own. | this spec section 3; `features/ambre-spec-prose-engine.md section 3` | not built -- lands with the "Toolbar, slash menu and composition" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** none of its own: filtering runs over a list of twelve items on a keystroke, inside
  the keystroke budget the draft buffer already owns
  (`features/ambre-spec-prose-text-editing.md`, `ambre-spec-budgets.md section 1`).
* **Boundary.** none: the menu reads view state and emits commands. It holds no key and opens no
  socket, and every mutation it starts is a `ProseCommand` (`PROSE-UI-002`).
* **Verification.** 5 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`.
  `PROSE-UI-058` and the filter behind `PROSE-UI-062` are host-tier facts over the block and the
  query, with no browser. `PROSE-UI-059` and `PROSE-UI-060` are the pair that has to be tested
  together: each alone is satisfied by a design that loses the other.

---

## 2. WHAT EACH USER SEES

The menu is local; what the document records is the typed text, then the block the item asked for.
No test resolves yet: the last column names the row each scenario rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| types `/` on an empty block | watches | the slash, as text | Alice's menu; Bob sees a slash | none -- `PROSE-UI-059` |
| types `tab` after it | watches | `/tab` | Alice's menu narrowed; Bob sees `/tab` | none -- `PROSE-UI-059` |
| presses Enter on "Table" | watches | the block emptied, then a table | a table on both screens, and no leftover slash | none -- `PROSE-UI-060` |
| presses Escape | watches | `/tab`, untouched | the menu gone, the text still there | none -- `PROSE-UI-061` |
| types `/` and picks a heading | types in the same block at the same moment | both edits, converged by the text CRDT | a heading holding both authors' text | none -- `PROSE-002` |

---

## 3. MODEL

**Containers.** This feature owns none. It removes a range of one block's text and then sends one
block command.

**The state the view publishes.** The menu is a DOM element on the shared menu primitives, and the
canvas view is what knows where the caret is, so the view publishes a state and the element renders
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
is in the document, so a peer sees a slash being typed and then erased -- which is what happened.

**Steps, from the slash to the block:**

1. a `/` typed at the start of a block opens the menu, and the block and offset are remembered as
   the origin (`PROSE-UI-058`);
2. every character after it appends to the query and to the block alike, and the item list is
   filtered on the query;
3. the arrow keys move the selection, and nothing is sent (`PROSE-UI-061`);
4. Enter deletes the slash and the query from the origin block, in one `DeleteText` over the range
   the menu spans (`PROSE-UI-060`);
5. the item's command is sent, addressed to the origin block (`PROSE-UI-062`).

**Where it may open.** A slash in the middle of a sentence is a slash, not a menu, so the menu opens
only where inserting a block is meaningful: on an empty block, or before the block's first
character (`PROSE-UI-058`).

**What closes it.** The Backspace that erases the slash, a space, Escape, and any pointer press
outside the menu. Each of the four leaves the typed text exactly as it is: closing the menu is not
an undo (`PROSE-UI-061`).

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

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The typed text reaches the document | the input pipe is one path, and a menu that swallows keystrokes is a second one that has to reimplement composition, undo and the draft buffer. Letting the text through costs a visible slash for a moment and keeps one editing path | intercepting the keystrokes at the menu, which forks the input pipeline for one feature |
| The slash and the query are deleted before the command | the author asked for a table, not for `/tab` followed by a table, and deleting first means the block command lands on a block whose text is what the author wanted | leaving the query and letting the block command overwrite it, which depends on the two arriving in order |
| The menu opens only at the start of a block | a slash is a legitimate character everywhere else -- a date, a path, a fraction -- and a menu that opens on all of them is a menu the author has to dismiss all day | opening on any slash, which is what makes the feature an irritation rather than a shortcut |
| Closing writes nothing | four gestures close the menu and none of them is a request to change the document. Making one of them delete text would make Escape destructive, which no other Escape in the application is | erasing the query on close, which loses text the author typed on purpose |
| Every item is an existing command | the menu is a surface, and a surface that mints its own command is a second dispatch path that convergence rules never covered | giving the menu a command of its own for the composite items, which puts document logic in the chrome |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing exists.** `crates/ambre-prose/src/lib.rs` is seven lines, so the menu, its filter and
  its origin are a design, and every row above is `unimplemented`.
* **The deletion and the command are two operations, and nothing binds them.** Step 4 and step 5
  are a `DeleteText` and a block command, and a peer can insert between them. The corpus declares
  no way to send the pair as one, so the failure is a table appearing after a `/tab` that a remote
  edit rescued.
* **The origin is not reanchored.** The menu remembers a block and an offset, and a remote edit
  before that offset moves the text under it. `features/ambre-spec-prose-text-editing.md` reanchors
  the draft over a remote delta; nothing does the same for the menu's range.
* **The filter is undefined.** "Filtered by the query" is the whole of it: no rule says whether the
  match is a prefix, a substring or fuzzy, whether it is case-folded, or what order the survivors
  are listed in.
* **The item list is fixed and reaches only nine features.** Twelve items name the paragraph, the
  heading, the three list styles, the table, the code block, the two rules and the table of
  contents. A picture, a footnote, a cross reference, a comment and an equation have no item, and
  `features/ambre-spec-prose-images.md section 4` names this menu as their trigger.
* **The three heading items stop at level three.** The document model carries six
  (`ambre-spec-prose-schema.md section 3`), so half the levels are reachable from the bar and the
  menu alike only through a control neither one declares.
