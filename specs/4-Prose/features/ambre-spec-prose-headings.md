# AMBRE -- FEATURE: HEADINGS

**Type:** Vertical (feature)
**Implementation:** design-only

> Six levels of section title. Each one carries its level and nothing else. The number in front of
> it, and its place in the outline, are computed from the document order when the page is drawn.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-030` | `OBS` | The number drawn in front of a heading MUST be the one its position implies, read from the headings before it in document order. | this spec section 3 | not built | unbuilt -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-090` | `INV` | The document MUST NOT hold a heading's number, nor its place in the outline. | this spec section 3; this spec section 5 | not built | unbuilt -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-029` | `OBS` | A heading whose stored level is outside one to six MUST be numbered as the nearest level inside it. | this spec section 3 | not built | unbuilt -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-091` | `INV` | The stored level MUST stay the level the document carries: no numbering pass and no export MUST rewrite it to the level it was read as. | this spec section 3 | not built | unbuilt -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-031` | `INV` | A level MUST reach a block only through a typed command; it MUST NOT be written as an untyped attribute. | this spec section 3 | not built | unbuilt -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Turn a paragraph into a heading, or change its level

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-094` | `PRE` | The level a command carries MUST be one to six; a level outside that range reaches the document through an import alone. | this spec section 3 | not built | unbuilt -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-092` | `POST` | After the command, the block MUST be a heading of the level asked for, or a paragraph when the level is taken away. | this spec section 4 | not built | unbuilt -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-093` | `FRAME` | Setting a level MUST NOT touch the block's text, its place in document order, or the level of any other block. | derived | not built | unbuilt -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the section numbering, shared with the list ordinals in one row of
  `ambre-spec-budgets.md section 1`: both walk the block tree once per layout.
* **Boundary.** none of its own: a heading is a block like any other, and `ZK-001` seals every
  payload before it leaves the client.
* **Verification.** 8 `unimplemented`, nothing else: `crates/ambre-prose` is a seven-line `lib.rs`
  and `ambre-bus` carries no Prose command, so no row can be held either way. One row has an oracle
  already, the shape a property test reaches. Numbering a document with levels in range and
  numbering it with a level outside one to six must agree (`PROSE-FEAT-029`).
  `PROSE-FEAT-031` is compiler-held the day the command enum exists, since an untyped attribute
  write is a variant that is not there. The rest wait on the gesture: what setting a level leaves
  behind and what it must not touch.

---

## 2. WHAT EACH USER SEES

Two peers over one outline. No test resolves yet: the last column names the row each row rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| inserts a level-two heading in the middle | -- | one more block, no number anywhere | every following heading renumbered, on both screens | none -- `PROSE-FEAT-030` |
| promotes a heading from level three to two | promotes the same one to level one | one level, the later of the two | the same number in front of it on both screens | none -- `PROSE-FEAT-030` |
| opens a document imported with a level of zero | -- | the level as it was imported | the heading numbered as a level one | none -- `PROSE-FEAT-029` |
| exports that document again | -- | the level still as it was imported | the original level in the exported file | none -- `PROSE-FEAT-091` |
| deletes a level-one heading | types under the level-two heading below it | the deletion and the text | the remaining headings renumbered around the hole | none -- `PROSE-FEAT-030` |

---

## 3. MODEL

**Containers.** None of its own. A heading is a block of the movable tree carrying a level
attribute, declared in `ambre-spec-prose-schema.md section 3`.

**The number in front of a heading** is a vector, one component per level: the third
second-level heading under the first first-level one is `1.2.1`. It is derived from the headings
that precede the block in document order (`PROSE-FEAT-030`) and stored nowhere
(`PROSE-FEAT-090`).

```rust
/// The section number of a heading, as one counter per level.
pub fn compute_section_number(tree: &LoroMovableTree, heading_id: &TreeID) -> Vec<u32>;

/// The stored level of a block, when it is a heading. Raw: the clamp is applied by the
/// numbering, not here (`PROSE-FEAT-029`).
fn heading_level_of(tree: &LoroMovableTree, id: &TreeID) -> Option<u8>;
```

**A level out of range.** The format documents one to six, and an import is the only way in for a
level outside it (`PROSE-FEAT-094`). A document can arrive carrying zero, seven or more. It is
numbered as the nearest level in range (`PROSE-FEAT-029`), and the stored level stays as it came
(`PROSE-FEAT-091`).

| The stored level | Numbering reads | The export writes |
|---|---|---|
| one to six | the level itself | the level itself |
| zero | one | zero |
| above six | six | the stored level |

Without the clamp, a level of zero indexes the counter vector below its first component.

**Creating and changing one** (`ambre-bus`, declared in `features/ambre-spec-prose-engine.md section 3`):

| Shape | Carries |
|---|---|
| `InsertBlock { init: BlockInit::Heading { level }, .. }` | the level, mandatory at creation |
| `SetHeadingLevel { block_id, level }` | the new level of an existing heading |

Both are typed, so no path can create a heading without a level or write a level onto a block that
is not one (`PROSE-FEAT-031`). What each leaves behind is `PROSE-FEAT-092`, and what neither may
touch is `PROSE-FEAT-093`.

**What reads this feature.** The table of contents reads the outline
(`features/ambre-spec-prose-table-of-contents.md`). Pagination reads the heading's own settings for
its keep-with-next rule (`features/ambre-spec-prose-pagination.md`), and the cross references read
the number (`features/ambre-spec-prose-cross-references.md`). None of the three stores what it read.

**Code:** none. `crates/ambre-prose/src/lib.rs` is seven lines.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the style control of the format bar | turns the paragraph into a heading of the chosen level, or back into a paragraph |
| the slash command | inserts a heading of the chosen level |
| a formatting chord | sets the level of the paragraph under the caret |
| an import | keeps the level the file carried, even out of range |

The appearance of each level -- its size, its spacing, its weight -- is the built-in settings of
the block type. The cascade resolves them
(`features/ambre-spec-prose-inline-styles.md section 3`).

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The number is computed from document order | storing the number on the heading | it turns inserting a section into rewriting the outline: a stored number is wrong for every heading after an insertion, so it is rewritten down the document on each one -- a write per heading, per edit, per peer, and a merge of numbers on top. Computed, two peers cannot disagree, because both read the same order |
| An out-of-range level is clamped for numbering and kept as stored | rewriting the level on import | it loses what the file said and makes the round trip lossy. The two duties conflict: numbering needs a level in range or it indexes outside its counters, and a lossless export needs the level the file arrived with, so splitting them by read and write satisfies both |
| The level is part of creating the block | a generic block insert followed by a level write | a heading created without a level needs a default and a follow-up command to correct it -- two writes for one gesture, and a window in which the document holds a heading nobody asked for |
| A heading holds its level and nothing else | storing the outline path on the heading | it is the stored-number defect one level up. The number, the place in the outline and the appearance are all derived from levels and order, so storing any of them is a second copy that a concurrent edit puts out of step |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing rules on a skipped level.** A level-one heading followed by a level-three one has no
  defined number. Whether the missing level counts as zero, is skipped, or is treated as the level
  above is undecided. The table of contents reads whatever the numbering produces.
* **Numbering has no format.** The number is a vector of counters, and no rule says how it is
  rendered. Nothing names the separator, whether a trailing separator is written, or whether a
  level may be lettered rather than numbered.
* **Numbering cannot be turned off per heading.** The table of contents carries a switch for the
  whole document (`features/ambre-spec-prose-table-of-contents.md`); a single unnumbered heading
  has no representation.
* **Nothing rules on a heading inside a table cell or a footnote.** Both hold ordinary blocks, so a
  heading can be created there, and the numbering walk does not say whether it counts.
