# AMBRE -- FEATURE: HEADINGS

**Type:** Vertical (feature)
**Implementation:** design-only

> Six levels of section title. Each one carries its level and nothing else: the number in front of
> it, and its place in the outline, are computed from the document order when the page is drawn.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `PROSE-FEAT-029` | A heading level outside one to six MUST be brought into that range for numbering, and the stored level MUST be left as it was. | this spec section 3 | not built -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-030` | A heading's number MUST be derived from the headings before it when the document is read; it MUST NOT be stored. | this spec section 3 | not built -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-031` | A block MUST become a heading only through a typed level, at creation or through the command that sets one; a level MUST NOT be written as an untyped attribute. | this spec section 3 | not built -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the section numbering, shared with the list ordinals in one row of
  `ambre-spec-budgets.md section 1`: both walk the block tree once per layout.
* **Boundary.** none of its own: a heading is a block like any other, and `ZK-001` seals every
  payload before it leaves the client.
* **Verification.** 3 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`.
  `PROSE-FEAT-029` has an oracle already: numbering a document whose levels are all in range and
  numbering the same document with an out-of-range level are two paths that must agree, which is
  the shape a property test reaches. `PROSE-FEAT-031` is compiler-held the day the command enum
  exists, because an untyped attribute write is a variant that is not there.

---

## 2. WHAT EACH USER SEES

Two peers over one outline. No test resolves yet: the last column names the row each row rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| inserts a level-two heading in the middle | -- | one more block, no number anywhere | every following heading renumbered, on both screens | none -- `PROSE-FEAT-030` |
| promotes a heading from level three to two | promotes the same one to level one | one level, the later of the two | the same number in front of it on both screens | none -- `PROSE-FEAT-030` |
| opens a document imported with a level of zero | -- | the level as it was imported | the heading numbered as a level one | none -- `PROSE-FEAT-029` |
| exports that document again | -- | the level still as it was imported | the original level in the exported file | none -- `PROSE-FEAT-029` |
| deletes a level-one heading | types under the level-two heading below it | the deletion and the text | the remaining headings renumbered around the hole | none -- `PROSE-FEAT-030` |

---

## 3. MODEL

**Containers.** None of its own. A heading is a block of the movable tree carrying a level
attribute, declared in `ambre-spec-prose-schema.md section 3`.

**The number in front of a heading** is a vector, one component per level: the third
second-level heading under the first first-level one is `1.2.1`. It is derived from the headings
that precede the block in document order and stored nowhere (`PROSE-FEAT-030`).

```rust
/// The section number of a heading, as one counter per level.
pub fn compute_section_number(tree: &LoroMovableTree, heading_id: &TreeID) -> Vec<u32>;

/// The stored level of a block, when it is a heading. Raw: the clamp is applied by the
/// numbering, not here (`PROSE-FEAT-029`).
fn heading_level_of(tree: &LoroMovableTree, id: &TreeID) -> Option<u8>;
```

**A level out of range.** The format documents one to six, and an imported document can carry
zero, seven or more (`PROSE-FEAT-029`):

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
is not one (`PROSE-FEAT-031`).

**What reads this feature.** The table of contents reads the outline
(`features/ambre-spec-prose-table-of-contents.md`), pagination reads the heading's own settings for
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
the block type, resolved by the cascade
(`features/ambre-spec-prose-inline-styles.md section 3`).

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The number is computed from document order | a stored number is wrong for every heading after an insertion, so it would be rewritten down the document on each one -- a write per heading, per edit, per peer, and a merge of numbers on top. Reading them cannot disagree between two peers, because both read the same order | storing the number on the heading, which turns inserting a section into rewriting the outline |
| An out-of-range level is clamped for numbering and kept as stored | the two duties conflict: numbering needs a level in range or it indexes outside its counters, and a lossless export needs the level the file arrived with. Splitting them by read and write satisfies both, and neither has to know about the other | rewriting the level on import, which loses what the file said and makes the round trip lossy |
| The level is part of creating the block | a heading created without one needs a default, and a follow-up command to correct it -- two writes for one gesture, and a window in which the document holds a heading nobody asked for. Carrying the level in the initializer removes both | a generic block insert followed by a level write |
| A heading holds its level and nothing else | its number, its place in the outline and its appearance are all derived from levels and order, so storing any of them is a second copy that a concurrent edit puts out of step | storing the outline path on the heading, which is the same defect as the stored number, one level up |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing rules on a skipped level.** A level-one heading followed by a level-three one has no
  defined number: whether the missing level counts as zero, is skipped, or is treated as the level
  above is undecided, and the table of contents reads whatever the numbering produces.
* **Numbering has no format.** The number is a vector of counters, and no rule says how it is
  rendered -- the separator, whether a trailing separator is written, or whether a level may be
  lettered rather than numbered.
* **Numbering cannot be turned off per heading.** The table of contents carries a switch for the
  whole document (`features/ambre-spec-prose-table-of-contents.md`); a single unnumbered heading
  has no representation.
* **Nothing rules on a heading inside a table cell or a footnote.** Both hold ordinary blocks, so a
  heading can be created there, and the numbering walk does not say whether it counts.
