# AMBRE -- FEATURE: INLINE AND PARAGRAPH STYLES

**Type:** Vertical (feature)
**Implementation:** design-only

> Bold, italic, colour, size, links: formatting applied to a range of text, and the paragraph
> settings underneath it. What a character finally looks like is decided at read time. It reads
> the mark on it, the paragraph it sits in, and the defaults of the document.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-007` | `INV` | An inline mark MUST expand or not expand at its boundaries according to the registry alone (`ambre-spec-prose-schema.md section 3`), never according to the applying feature. | this spec section 3; `PROSE-007` | not built | unbuilt -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-020` | `INV` | The value of a property MUST be taken from the first level that carries it, in the order inline mark, paragraph style, block type, document default. | this spec section 3 | not built | unbuilt -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-177` | `INV` | The cascade MUST terminate: a property no level carries MUST resolve to a decided absence, and a reader of the resolved style MUST NOT look further up. | this spec section 3 | not built | unbuilt -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-173` | `INV` | Superscript and subscript MUST both be storable on one range: writing either one MUST NOT clear the other. | this spec section 3 | not built | unbuilt -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-174` | `INV` | Where both are stored on one range, the resolved style MUST be the raised one, by a priority fixed in the reader and not by which mark was written last. | this spec section 3 | not built | unbuilt -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-021` | `CONC` | Two peers holding both marks MUST resolve one style, whichever order the two marks arrived in. | this spec section 3 | not built | unbuilt -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-008` | `CONC` | Two concurrent applications of the same mark to overlapping ranges MUST converge without producing a torn or duplicated span. | this spec section 2 | not built | unbuilt -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-184` | `CONC` | Two peers writing two values of one mark key over one range MUST converge on one of the two, and both MUST resolve that same one. | this spec section 5 | not built | unbuilt -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-176` | `CONC` | Two authors setting two different properties of one paragraph MUST both keep their property: neither MUST overwrite the other's. | this spec section 3 | not built | unbuilt -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-175` | `INV` | Two spellings of one colour MUST be stored as one: the bytes a peer writes for a colour MUST depend on the colour alone. | this spec section 3; `PROSE-008` | not built | unbuilt -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-023` | `IO` | Reading a stored colour and writing it back MUST leave the bytes unchanged. | this spec section 3; `PROSE-008` | not built | unbuilt -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Apply a mark to a range

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-178` | `POST` | The mark MUST cover the given range and no character outside it, and the value stored MUST be the canonical form of the value supplied. | this spec section 3 | not built | unbuilt -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-022` | `FAIL` | A mark value that fails the grammar of its key MUST be refused before the document is written, and MUST be reported to the author. | this spec section 3 | not built | unbuilt -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-179` | `FRAME` | Applying a mark MUST NOT change the text of the block, any other key of the registry over that range, or the block's paragraph settings. | this spec section 3 | not built | unbuilt -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Remove a mark from a range

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-180` | `POST` | After removal the named key MUST cover no character of the range. | this spec section 3 | not built | unbuilt -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-287` | `POST` | After removal the named key MUST still cover every character outside the range that it covered before. | this spec section 3 | not built | unbuilt -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-181` | `FRAME` | Removing one key MUST NOT change the text of the block, or any other key of the registry over that range. | this spec section 3 | not built | unbuilt -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.4 Clear the formatting of a range

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-024` | `POST` | Clearing MUST remove every presentation mark and the link over the range. | this spec section 3 | not built | unbuilt -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-288` | `POST` | Clearing MUST leave the range's text as it was. | this spec section 3 | not built | unbuilt -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-182` | `FRAME` | Clearing MUST NOT touch a comment, a suggestion, a footnote, a cross reference or a dynamic field over the range: each carries an identity a formatting reset MUST NOT destroy. | this spec section 3 | not built | unbuilt -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.5 Set a paragraph's settings

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-025` | `POST` | Setting a paragraph's settings MUST write only the properties whose value differs from what is stored. | this spec section 3 | not built | unbuilt -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-183` | `FRAME` | Setting a paragraph's settings MUST NOT change the block's text, the inline marks on it, or the settings of any other block. | this spec section 3 | not built | unbuilt -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the resolution of the effective style for a block, a row of
  `ambre-spec-budgets.md section 1`. It runs once per block per layout, so it is on the
  pagination path rather than beside it.
* **Boundary.** none of its own: a mark is a document write, and `ZK-001` seals every payload
  before it leaves the client.
* **Verification.** 22 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`.
  `PROSE-FEAT-023` and `PROSE-FEAT-175` are the two an oracle reaches with one function: a byte
  round trip, and two colour spellings parsing to one value. `PROSE-FEAT-174`
  is decided by a comparison rather than by two arrival orders, so it holds before any peer exists.
  `PROSE-FEAT-021`, `PROSE-FEAT-008`, `PROSE-FEAT-184` and `PROSE-FEAT-176` are the four the
  two-peer table below is written for. The four `FRAME` rows are the ones nothing reaches until
  a handler exists to leave something alone.

---

## 2. WHAT EACH USER SEES

Two peers over one sentence. No test resolves yet: the last column names the row each row rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| bolds words two to five | bolds words four to seven at the same moment | one bold span over words two to seven | one continuous bold run, no seam | none -- `PROSE-FEAT-008` |
| makes a word superscript | makes the same word subscript | both marks, neither cleared | the word raised, on both screens | none -- `PROSE-FEAT-173`, `PROSE-FEAT-174` |
| sets the paragraph's space before | sets the same paragraph's alignment | both settings | a centred paragraph with the new space above it | none -- `PROSE-FEAT-176` |
| sets the font size to 12 | sets the same size to 14 | one of the two | one size, the same for both | none -- `PROSE-FEAT-184` |
| clears formatting over a commented range | -- | the comment, without the bold | plain text, the comment still anchored | none -- `PROSE-FEAT-182` |
| types at the end of a bold word | -- | the new text inside the bold span | the typing continues bold | none -- `PROSE-FEAT-007` |
| types just after a link | -- | the new text outside the link | plain text after the link | none -- `PROSE-FEAT-007` |

---

## 3. MODEL

**Containers.** None of its own. The mark registry, its keys, its value grammars and its expansion
column are `ambre-spec-prose-schema.md section 3`; the paragraph settings are the per-property map
declared there. This document states the rules whose cause is that format, and restates none of it.

**The cascade.** A property is looked for at four levels and taken from the first that carries it
(`PROSE-FEAT-020`):

1. the inline mark on the character;
2. the paragraph's own settings, through its style identifier;
3. the built-in settings of the block type -- a heading level, a code block, a quote;
4. the document defaults.

```rust
/// The result of the cascade for one run of text. Every field is a decided value: a caller
/// draws from it and never looks further up (`PROSE-FEAT-020`, `PROSE-FEAT-177`).
pub struct ResolvedStyle {
    pub bold:          bool,
    pub italic:        bool,
    pub underline:     bool,
    pub strikethrough: bool,
    pub superscript:   bool,
    pub subscript:     bool,
    pub code_inline:   bool,
    pub color:         Option<u32>,
    pub background:    Option<u32>,
    /// In points.
    pub font_size:     f32,
    pub font_family:   Option<String>,
    pub link:          Option<String>,
}
```

**Raised and lowered text.** Superscript and subscript are two independent marks, and neither
write clears the other, so no write ordering is needed (`PROSE-FEAT-173`). Resolution applies a
fixed priority -- raised wins -- at read time (`PROSE-FEAT-174`). Because the priority is fixed
rather than a tie-break on arrival order, both peers resolve the same style from the same document
(`PROSE-FEAT-021`).

**The paragraph's settings:**

```rust
pub struct ParagraphStyle {
    pub align:          TextAlign,
    pub indent_first:   f32,
    pub indent_left:    f32,
    pub indent_right:   f32,
    pub space_before:   f32,
    pub space_after:    f32,
    pub line_height:    LineHeight,
    /// Read by pagination: a block that must not be split, and one that must not be
    /// parted from the next (`features/ambre-spec-prose-pagination.md`).
    pub keep_together:  bool,
    pub keep_with_next: bool,
}

pub enum TextAlign { Left, Center, Right, Justify }
pub enum LineHeight { Multiple(f32), Exact(f32), AtLeast(f32) }
```

The command carries the whole set; the handler compares it against what is stored and writes only
the keys that differ (`PROSE-FEAT-025`). The command stays coarse and the storage stays granular,
which is what lets two authors change two properties of one paragraph at once
(`PROSE-FEAT-176`).

**The four commands** (`ambre-bus`, declared in `features/ambre-spec-prose-engine.md section 3`):

| Shape | Carries |
|---|---|
| `ApplyMark { block_id, range, key, value }` | one key of the registry, and its value as a string (`PROSE-FEAT-178`) |
| `RemoveMark { block_id, range, key }` | one key, over a range (`PROSE-FEAT-180`) |
| `ClearInlineMarks { block_id, range }` | nothing but the range (`PROSE-FEAT-024`) |
| `SetParagraphStyle { block_id, style }` | the whole set, diffed by the handler (`PROSE-FEAT-025`) |

**Validating a value.** The value travels as a string, and each key has its own grammar. The
grammar is checked before the document is touched. A malformed value is refused whole and reported,
so no unparseable value ever reaches the format (`PROSE-FEAT-022`).

```rust
pub enum MarkValueError {
    /// A colour that is neither six nor eight hexadecimal digits behind a hash.
    BadColor(String),
    /// A size that is not finite, or outside one to 1 638 points.
    BadFontSize(String),
    /// A value supplied for a key that takes none, or missing for a key that requires one.
    ArityMismatch { key: String },
}

/// Parse a colour into red, green, blue and alpha packed in that order. The six-digit form
/// is opaque, so the two spellings of one opaque colour parse to one value (`PROSE-FEAT-175`)
/// and a round trip of the stored bytes is stable (`PROSE-FEAT-023`).
fn parse_color(raw: &str) -> Result<u32, MarkValueError>;
/// Parse a size in points. The ceiling is the one the office formats round-trip.
fn parse_font_size(raw: &str) -> Result<f32, MarkValueError>;
```

The refusal has to reach the author, and the event that carries it is not declared: see section 6.

**What clearing removes.** Clearing formatting removes the eleven presentation marks and the link.
It removes no comment, suggestion, footnote or cross reference, and no dynamic field. Each of those
carries an identity or a binding that a formatting reset must not destroy (`PROSE-FEAT-182`).
Removing one of them is its own command.

**Code:** none. `crates/ambre-prose/src/lib.rs` is seven lines.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the format bar, over a selection | applies or removes one mark on the selected range |
| a formatting chord | the same, from the keyboard |
| the colour and size controls | applies the mark whose value the control produced, refused if malformed |
| the paragraph controls: alignment, spacing, line height | writes the properties that changed |
| the ruler handles | writes the three indents, on release (`features/ambre-spec-prose-ruler.md`) |
| clearing formatting | strips presentation, keeps annotations |
| typing at a span boundary | continues the span, or does not, per the registry |

Which control sits where, and what the format bar shows for the current selection, is chrome
(`features/ambre-spec-prose-toolbar.md`).

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The effective style is resolved at read time | storing the resolved values on the text | it turns editing a style into rewriting every character it covers: a second copy of every level above, wrong the moment a paragraph style or a document default changes. Resolved on read, a change to a style is visible everywhere it applies with no walk over the document |
| Expansion is a property of the mark key, not of the caller | letting each feature decide | it is how a link grows by a word when text is pasted after it. Whether typing at the edge of a span continues it is the same question for the keyboard, a paste and an import, and one answer per caller means the user meets all of them |
| Raised beats lowered by a fixed priority | clearing one mark when the other is written | it makes the outcome depend on the order two concurrent writes happened to arrive in, so two peers seeing the same document pick different winners. Both marks can legally coexist, and a fixed priority is decided by the reader rather than by the network |
| A value is validated before the document is written | validating at read time | it puts the error in the reader and keeps the bad value forever. The format has no type per key -- every value is a string -- so the command handler is the only place a bad value can be stopped; past it, the value is in the history of every peer |
| A colour has one canonical spelling | accepting any spelling as written | it makes `#3366cc` and `#3366ccff` two racing values, and last-writer-wins then arbitrates between two spellings of one colour while the document flickers between them. Canonical, the eight-digit form carries alpha and the shorter one wins whenever alpha is full |
| Paragraph settings are stored per property | one opaque settings value per paragraph | it loses the half that did not conflict. Two authors changing the alignment and the spacing of one paragraph is an ordinary collision, and per-key storage merges it automatically, leaving only same-property collisions to arbitrate |
| Clearing formatting spares annotations | clearing every mark on the range | it deletes comments nobody asked to delete. A comment, a suggestion and a footnote are content and carry an identity a second store points at, so a formatting reset -- a gesture users reach for often -- would break that link silently |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The event a refusal travels on does not exist.** `PROSE-FEAT-022` requires the author to be
  told, and the refusal was written as `CommandRejected`, a variant
  `features/ambre-spec-prose-engine.md section 3` never declares. The lists feature reports
  through the same missing variant. The enum carries one generic error instead, which names no
  command and no reason. It lands with the
  `A refused command names what it refused` Story (`ambre-roadmap.md` E5).
* **Nothing says which side of the registry a new key falls on.** Clearing removes the
  presentation marks and the link, and spares the annotations. The split is a sentence of
  section 3 rather than a column of the registry. A key added to
  `ambre-spec-prose-schema.md section 3` falls on neither side until someone edits this document.
* **The font family is resolved against what the device loaded**, and pagination depends on the
  result (`features/ambre-spec-prose-pagination.md`). No rule says what an unknown family name
  resolves to for the reader who does not have it, only that measurement may then differ.
* **A named paragraph style cannot inherit from another.** The style store is flat and the cascade
  above has no level for a base style. The deferred design gives a style a `based_on` key and walks
  the chain, capped at five levels with a cycle guard. No container carries the key
  (`ambre-spec-prose-schema.md section 3`), and the Story is
  "The later writing set" (`ambre-roadmap.md` POST-V1).
* **Nothing rules on a link's target.** The link mark carries a string, and no rule constrains the
  scheme, the length or what happens on an empty one. Opening it is a click rule
  (`PROSE-UI-070`, `PROSE-UI-071`, `features/ambre-spec-prose-selection.md`), and the shared
  open-URL affordance is Foundations'.
