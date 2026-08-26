# AMBRE -- FEATURE: LISTS

**Type:** Vertical (feature)
**Implementation:** design-only

> Bulleted, numbered and checkbox lists, nested to any depth. The numbers are computed when the
> page is drawn, so two authors adding items at once never see them fight over an ordinal.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `PROSE-005` | A list item's depth MUST be derived from its ancestors; it MUST NOT be stored in the document. | this spec section 3; `ambre-spec-prose-schema.md section 3` | not built -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-002` | List ordinals MUST be derived when the list is read; they MUST NOT be stored in the document. | this spec section 3 | not built -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-026` | An ordinal MUST count only the unbroken run of neighbouring items that share one parent and one numbering style; an interrupted list MUST restart at one when it resumes. | this spec section 3 | not built -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-027` | Outdenting a top-level item MUST turn it into an ordinary paragraph; it MUST NOT leave it a list item. | this spec section 3 | not built -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-028` | Indenting the first item of a run MUST be refused and reported; it MUST NOT be applied. | this spec section 3 | not built -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the ordinal and the section numbering, one row of
  `ambre-spec-budgets.md section 1`: both are read-time walks over the block tree, and
  pagination pays for them once per layout.
* **Boundary.** none of its own: a list item is a block like any other, and `ZK-001` seals every
  payload before it leaves the client.
* **Verification.** 5 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`.
  `PROSE-005` and `PROSE-FEAT-002` are the two a reviewer can hold without a test, because a
  stored depth or a stored ordinal is a key in the schema table and the schema table has neither.

---

## 2. WHAT EACH USER SEES

Two peers in one list. No test resolves yet: the last column names the row each row rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| adds an item at the end | adds an item at the end at the same moment | both items, in the order the tree converged on | the same two items, numbered one after the other | none -- `PROSE-FEAT-002` |
| inserts an item in the middle | -- | one more child, no number anywhere | every following item renumbered, on both screens | none -- `PROSE-FEAT-002` |
| indents item three | indents item three at the same moment | one move, applied once | one nesting level deeper, and the same depth on both screens | none -- `PROSE-005` |
| indents item four | outdents item four | one of the two moves | the item at one depth, the same for both | none -- `PROSE-005` |
| outdents a top-level item | -- | a paragraph where the item was | the bullet gone, the text kept | none -- `PROSE-FEAT-027` |
| turns item two into a bullet in a numbered list | -- | the item, with its style changed | two runs of numbers, the second restarting at one | none -- `PROSE-FEAT-026` |

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

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| Nesting is the block tree, with no list node | a list container would be a second structure over the same blocks, and every insert, move and delete would have to keep the two in step. The tree already moves a subtree atomically, which is exactly what indenting an item with children is | a list node holding its items, which turns one move into two writes that can converge apart |
| Depth is read from the ancestors | a stored depth is a second truth, and two peers indenting the same item concurrently leave it stored as one number and shaped as another. Reading the chain cannot disagree with the tree, because it is the tree | a depth attribute on the item, correct until the first concurrent indent |
| Ordinals are computed when the list is read | a stored number is wrong for every following item the moment one is inserted, so it would have to be rewritten down the whole list -- a write per item, per insertion, per peer. Reading them costs one walk of the run and converges by construction | storing the ordinal on each item, which makes an insertion a rewrite of the list and a merge of numbers |
| A run breaks on any different neighbour | the alternative is to decide what a list "is" across the paragraph between two of its parts, and there is no correct answer: the same document reads as one interrupted list to one author and as two lists to another. Breaking on difference is the rule the user can see | continuing the numbering across an interruption, which needs a stored intent per item to be predictable |
| A resumed run restarts at one | continuing needs to know it is a continuation, which is an explicit per-item override, not something to infer. Restarting is what the popular editors do, and it is the behaviour that can be explained in one sentence | inferring a continuation from the surrounding blocks, which guesses, and guesses differently on either side of an edit |
| Outdenting at the top level leaves the list | there is nowhere further to rise, and the alternative is a gesture that does nothing at the one place users press it most: the end of a list they want to stop writing | ignoring the command at the top level, which leaves a user pressing Shift and Tab with no way out of the list |
| Indenting the first item is refused, not ignored | it has no previous neighbour, so there is no correct target. Reporting it lets the chrome answer the user, and keeps the silent no-op from reading as a lost keystroke | reparenting to the item above the list, which moves the item out of the list it was in |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The event a refusal travels on does not exist.** `PROSE-FEAT-028` requires the refusal to be
  reported, and it was written as `CommandRejected`, a variant
  `features/ambre-spec-prose-engine.md section 3` never declares. The styles feature reports
  through the same missing variant.
* **Nothing rules on Enter in an empty item.** Every editor of this kind ends a list that way, and
  no rule here says whether it outdents, leaves the list, or adds another empty item.
* **Continuing a previous numbering has no design.** It is named as a future per-item override and
  nothing carries it, so a document imported with a continued list loses that intent.
* **The checkbox state is a style, not a property.** Ticking an item rewrites its style value,
  so two peers ticking and restyling one item race over one key rather than over two.
* **Nothing rules on how deep a list may nest**, nor on what the marker is past the styles the
  numbering offers.
