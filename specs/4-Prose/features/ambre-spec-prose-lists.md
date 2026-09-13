# AMBRE -- FEATURE: LISTS

**Type:** Vertical (feature)
**Implementation:** design-only

> Bulleted, numbered and checkbox lists, nested to any depth. The numbers are computed when the
> page is drawn, so two authors adding items at once never see them fight over an ordinal.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `PROSE-005` | `INV` | A list item's depth MUST be derived from its ancestors; it MUST NOT be stored in the document. | this spec section 3; `ambre-spec-prose-schema.md section 3` | not built -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-002` | `INV` | List ordinals MUST be derived when the list is read; they MUST NOT be stored in the document. | this spec section 3 | not built -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-026` | `OBS` | The ordinal drawn on an ordered item MUST count only the unbroken run of neighbouring items that share one parent and one numbering style. | this spec section 3 | not built -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-136` | `OBS` | A run that resumes after an interruption MUST be drawn starting at one. | this spec section 5 | not built -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-137` | `CONC` | Two peers whose block trees have converged MUST draw the same ordinal on every item of a list. | this spec section 5 | not built -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Add an item

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `PROSE-FEAT-138` | `POST` | The new item MUST be a child of the given parent, placed after the given sibling, and carry the requested style. | this spec section 3 | not built -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-139` | `FRAME` | Adding an item MUST NOT write to any other block: no ordinal, no depth and no text is written on the items that follow it. | derived | not built -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Change an item's style

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `PROSE-FEAT-140` | `POST` | The item MUST carry the requested style, and a checkbox MUST carry the requested ticked state. | this spec section 3 | not built -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-141` | `FRAME` | Changing a style MUST NOT touch the item's text, its place in the block tree, or the style of any other item. | derived | not built -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.4 Indent an item

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `PROSE-FEAT-142` | `POST` | The indented item MUST become a child of the item that preceded it. | this spec section 3 | not built -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-143` | `POST` | The item's descendants MUST follow it in the same operation, keeping their order and their parent chain below it. | this spec section 5 | not built -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-028` | `FAIL` | Indenting the first item of a run MUST be refused and reported; it MUST NOT be applied. | this spec section 3 | not built -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-144` | `FRAME` | Indenting an item MUST NOT touch its text or its style. | derived | not built -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.5 Outdent an item

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `PROSE-FEAT-027` | `POST` | Outdenting a top-level item MUST turn it into an ordinary paragraph; it MUST NOT leave it a list item. | this spec section 3 | not built -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-145` | `POST` | Outdenting a nested item MUST make it a child of its grandparent, placed after the subtree of its former parent. | this spec section 3 | not built -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-146` | `FRAME` | Outdenting an item MUST NOT touch its text, nor the style of any item it leaves behind. | derived | not built -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the ordinal and the section numbering, one row of
  `ambre-spec-budgets.md section 1`: both are read-time walks over the block tree, and
  pagination pays for them once per layout.
* **Boundary.** none of its own: a list item is a block like any other, and `ZK-001` seals every
  payload before it leaves the client.
* **Verification.** 16 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`.
  `PROSE-005` and `PROSE-FEAT-002` are the two a reviewer can hold without a test, because a
  stored depth or a stored ordinal is a key in the schema table and the schema table has neither.
  `PROSE-FEAT-137` is the one that needs two peers, and it is the row the four commands are
  designed to earn: it fails the moment any of them writes a number.

---

## 2. WHAT EACH USER SEES

Two peers in one list. No test resolves yet: the last column names the row each row rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| adds an item at the end | adds an item at the end at the same moment | both items, in the order the tree converged on | the same two items, numbered one after the other | none -- `PROSE-FEAT-137` |
| inserts an item in the middle | -- | one more child, no number anywhere | every following item renumbered, on both screens | none -- `PROSE-FEAT-139` |
| indents item three | indents item three at the same moment | one move, applied once | one nesting level deeper, and the same depth on both screens | none -- `PROSE-005` |
| indents item four | outdents item four | one of the two moves | the item at one depth, the same for both | none -- `PROSE-005` |
| outdents a top-level item | -- | a paragraph where the item was | the bullet gone, the text kept | none -- `PROSE-FEAT-027` |
| turns item two into a bullet in a numbered list | -- | the item, with its style changed | two runs of numbers, the second restarting at one | none -- `PROSE-FEAT-136` |

---

## 3. MODEL

**Containers.** None of its own. A list item is a block of the movable tree, and its style is an
attribute of that block; both are declared in `ambre-spec-prose-schema.md section 3`. There is no
list container node: nesting is the tree's own parent chain, which is what makes `PROSE-005` a
statement about the format rather than about this feature.

**The styles:**

```rust
pub enum ListStyle {
    Bullet,
    Ordered(OrderedStyle),
    /// The checkbox, and whether it is ticked.
    Todo(bool),
}

pub enum OrderedStyle { Decimal, LowerAlpha, UpperAlpha, LowerRoman, UpperRoman }
```

**The ordinal, and where a run ends.** Numbering is scoped to an unbroken run of neighbouring
ordered items under one parent that share one numbering style. Any other neighbour breaks the run:
a paragraph, a bullet, a checkbox, or an ordered item numbered differently. A list that is
interrupted and resumes starts again at one (`PROSE-FEAT-026`).

```rust
/// The one-based ordinal of an ordered item within its run (`PROSE-FEAT-002`).
pub fn compute_ordinal(tree: &LoroMovableTree, item_id: &TreeID) -> u32;

/// The numbering style of a block, when it is an ordered item. A bullet, a checkbox and a
/// block that is not a list item all answer `None`, and each of those breaks a run.
fn ordered_style_of(tree: &LoroMovableTree, id: &TreeID) -> Option<OrderedStyle>;
```

**The four commands** (`ambre-bus`, declared in `features/ambre-spec-prose-engine.md section 3`):

| Shape | Behaviour |
|---|---|
| `InsertListItem { parent_id, after_id, style }` | a new item, at a place in the tree |
| `SetListStyle { block_id, style }` | changes the style, which may break a run in two |
| `IndentList { block_id }` | reparents the item under its previous neighbour |
| `UnindentList { block_id }` | reparents the item under its grandparent, after its former parent's subtree |

**The two edges of nesting.** Indenting the first item of a run has no previous neighbour to
become its parent, so it is refused and reported rather than silently ignored
(`PROSE-FEAT-028`). Outdenting a top-level item has no grandparent to rise to, so it becomes an
ordinary paragraph and loses its style (`PROSE-FEAT-027`): that is the gesture by which a user
leaves a list from the keyboard.

**Code:** none. `crates/ambre-prose/src/lib.rs` is seven lines.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the list controls of the format bar | starts a list, or changes the style of the items in the selection |
| the slash command | inserts an item of the chosen style |
| Tab at the start of an item | indents it, refused on the first item of a run |
| Shift with Tab | outdents it, leaving the list at the top level |
| Enter on an item | a new sibling item of the same style, through the ordinary paragraph split |
| a click on a checkbox | ticks or unticks that item |

Whether Enter on an empty item outdents rather than adding another empty one is not ruled on:
see section 6.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| Nesting is the block tree, with no list node | a list node holding its items | it turns one move into two writes that can converge apart: a second structure over the same blocks, kept in step on every insert, move and delete. The tree already moves a subtree atomically, which is exactly what indenting an item with children is |
| Depth is read from the ancestors | a depth attribute on the item | it is correct until the first concurrent indent, after which the item is stored as one number and shaped as another. Reading the chain cannot disagree with the tree, because it is the tree |
| Ordinals are computed when the list is read | storing the ordinal on each item | it makes an insertion a rewrite of the list and a merge of numbers: a stored number is wrong for every following item the moment one is inserted -- a write per item, per insertion, per peer. Reading them costs one walk of the run and converges by construction |
| A run breaks on any different neighbour | continuing the numbering across an interruption | it needs a stored intent per item to be predictable, because there is no correct answer for what a list "is" across the paragraph between two of its parts: the same document reads as one interrupted list to one author and as two lists to another |
| A resumed run restarts at one | inferring a continuation from the surrounding blocks | it guesses, and guesses differently on either side of an edit. Continuing needs to know it is a continuation, which is an explicit per-item override; restarting is what the popular editors do, and it explains itself in one sentence |
| Outdenting at the top level leaves the list | ignoring the command at the top level | it leaves a user pressing Shift and Tab with no way out of the list, at the one place they press it most: the end of a list they want to stop writing |
| Indenting the first item is refused, not ignored | reparenting to the item above the list | it moves the item out of the list it was in. The first item has no previous neighbour, so there is no correct target, and reporting the refusal lets the chrome answer instead of leaving a silent no-op that reads as a lost keystroke |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The event a refusal travels on does not exist.** `PROSE-FEAT-028` requires the refusal to be
  reported, and it was written as `CommandRejected`, a variant
  `features/ambre-spec-prose-engine.md section 3` never declares. The styles feature reports
  through the same missing variant, and it lands with the `A refused command names what it refused`
  Story (`ambre-roadmap.md` E5).
* **Nothing rules on Enter in an empty item.** Every editor of this kind ends a list that way, and
  no rule here says whether it outdents, leaves the list, or adds another empty item.
* **Continuing a previous numbering has no design.** It is named as a future per-item override and
  nothing carries it, so a document imported with a continued list loses that intent.
* **The checkbox state is a style, not a property.** Ticking an item rewrites its style value,
  so two peers ticking and restyling one item race over one key rather than over two.
* **Nothing rules on how deep a list may nest**, nor on what the marker is past the styles the
  numbering offers.
* **Nothing states that two concurrent moves settle on one shape.** Two rows of section 2 rest on
  it -- two peers indenting the same item, and an indent racing an outdent -- and they cite
  `PROSE-005`, which says only where the depth is read from. It is a property of the movable tree,
  so it belongs to `ambre-spec-prose-schema.md section 1`, and no row there carries it either.
