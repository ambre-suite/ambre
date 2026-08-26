# AMBRE -- FEATURE: INLINE AND PARAGRAPH STYLES

**Type:** Vertical (feature)
**Implementation:** design-only

> Bold, italic, colour, size, links: formatting applied to a range of text, and the paragraph
> settings underneath it. What a character finally looks like is decided at read time, from the
> mark on it, the paragraph it sits in, and the defaults of the document.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `PROSE-FEAT-007` | An inline mark MUST expand or not expand at its boundaries according to the registry alone (`ambre-spec-prose-schema.md section 3`), never according to the applying feature. | this spec section 3; `PROSE-007` | not built -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-008` | Two concurrent applications of the same mark to overlapping ranges MUST converge without producing a torn or duplicated span. | this spec section 2 | not built -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-020` | The value of a property MUST be taken from the first level that carries it, in the order inline mark, paragraph style, block type, document default. | this spec section 3 | not built -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-021` | Superscript and subscript MUST both be storable on one range, and the resolved style MUST be the same whichever order the two marks arrived in. | this spec section 3 | not built -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-022` | A mark value that fails the grammar of its key MUST be refused before the document is written, and MUST be reported to the author. | this spec section 3 | not built -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-023` | A colour MUST be stored in one canonical form, so that reading it and writing it back MUST leave the bytes unchanged. | this spec section 3; `PROSE-008` | not built -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-024` | Clearing formatting MUST remove formatting marks only, and MUST leave every comment, suggestion, footnote, cross reference and dynamic field in place. | this spec section 3 | not built -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-025` | Setting a paragraph's settings MUST write only the properties that changed, so two authors changing different properties MUST NOT overwrite each other. | this spec section 3 | not built -- lands with the "Styles, lists and headings" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the resolution of the effective style for a block, a row of
  `ambre-spec-budgets.md section 1`: it runs once per block per layout, so it is on the
  pagination path rather than beside it.
* **Boundary.** none of its own: a mark is a document write, and `ZK-001` seals every payload
  before it leaves the client.
* **Verification.** 8 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`.
  `PROSE-FEAT-023` and `PROSE-FEAT-021` are the two with an oracle: a colour is a round trip
  through one function, and the resolved style of two marks is a comparison of two arrival orders.
  `PROSE-FEAT-008` is the convergence row, and it is what the two-peer table below is written for.

---

## 2. WHAT EACH USER SEES

Two peers over one sentence. No test resolves yet: the last column names the row each row rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| bolds words two to five | bolds words four to seven at the same moment | one bold span over words two to seven | one continuous bold run, no seam | none -- `PROSE-FEAT-008` |
| makes a word superscript | makes the same word subscript | both marks, neither cleared | the word raised, on both screens | none -- `PROSE-FEAT-021` |
| sets the paragraph's space before | sets the same paragraph's alignment | both settings | a centred paragraph with the new space above it | none -- `PROSE-FEAT-025` |
| sets the font size to 12 | sets the same size to 14 | the later of the two | one size, the same for both | none -- `PROSE-FEAT-025` |
| clears formatting over a commented range | -- | the comment, without the bold | plain text, the comment still anchored | none -- `PROSE-FEAT-024` |
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
/// draws from it and never looks further up (`PROSE-FEAT-020`).
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
write clears the other, so no write ordering is needed. Resolution applies a fixed priority --
raised wins -- at read time. Because the priority is fixed rather than a tie-break on arrival
order, both peers resolve the same style from the same document (`PROSE-FEAT-021`).

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
which is what lets two authors change two properties of one paragraph at once.

**The four commands** (`ambre-bus`, declared in `features/ambre-spec-prose-engine.md section 3`):

| Shape | Carries |
|---|---|
| `ApplyMark { block_id, range, key, value }` | one key of the registry, and its value as a string |
| `RemoveMark { block_id, range, key }` | one key, over a range |
| `ClearInlineMarks { block_id, range }` | nothing but the range (`PROSE-FEAT-024`) |
| `SetParagraphStyle { block_id, style }` | the whole set, diffed by the handler (`PROSE-FEAT-025`) |

**Validating a value.** The value travels as a string, and each key has its own grammar. The
grammar is checked before the document is touched: a malformed value is refused whole and reported,
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
/// is opaque, so a round trip of an opaque colour is stable (`PROSE-FEAT-023`).
fn parse_color(raw: &str) -> Result<u32, MarkValueError>;
/// Parse a size in points. The ceiling is the one the office formats round-trip.
fn parse_font_size(raw: &str) -> Result<f32, MarkValueError>;
```

The refusal has to reach the author, and the event that carries it is not declared: see section 6.

**What clearing removes.** Clearing formatting removes the eleven presentation marks and the link.
It removes no comment, suggestion, footnote or cross reference, and no dynamic field: each of those
carries an identity or a binding that a formatting reset must not destroy (`PROSE-FEAT-024`).
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

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The effective style is resolved at read time | a stored resolved style is a second copy of every level above it, and it is wrong the moment a paragraph style or a document default changes. Resolving on read makes a change to a style visible everywhere it applies, with no walk over the document | storing the resolved values on the text, which turns editing a style into rewriting every character it covers |
| Expansion is a property of the mark key, not of the caller | whether typing at the edge of a span continues it is the same question for every path that can type there -- the keyboard, a paste, an import. Answering it per caller gives one answer per caller, and the user meets all of them | letting each feature decide, which is how a link grows by a word when text is pasted after it |
| Raised beats lowered by a fixed priority | both marks can legally coexist, and a tie-break on arrival order is not a tie-break at all: two peers can see the same document and pick different winners. A fixed priority is decided by the reader, not by the network | clearing one mark when the other is written, which makes the outcome depend on the order two concurrent writes happened to arrive in |
| A value is validated before the document is written | the format has no type per key -- every value is a string -- so the only place a bad value can be stopped is the command handler. Past it, the value is in the history of every peer and no reader can do anything with it | validating at read time, which puts the error in the reader and keeps the bad value forever |
| A colour has one canonical spelling | two peers writing the same colour must produce the same bytes, or last-writer-wins arbitrates between two spellings of one value and the document flickers between them. The eight-digit form carries alpha, the six-digit one is opaque, and the shorter one wins whenever alpha is full | accepting any spelling as written, which makes `#3366cc` and `#3366ccff` two racing values |
| Paragraph settings are stored per property | two authors changing the alignment and the spacing of one paragraph is an ordinary collision, and a single opaque value makes one of them lose work that did not conflict. Per-key storage makes the merge automatic and leaves only same-property collisions to arbitrate | one opaque settings value per paragraph, which loses the half that did not conflict |
| Clearing formatting spares annotations | a comment, a suggestion and a footnote are content, not presentation: they carry an identity that a second store points at. Removing them as a side effect of a formatting reset destroys that link silently, and the reset is a gesture users reach for often | clearing every mark on the range, which deletes comments nobody asked to delete |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The event a refusal travels on does not exist.** `PROSE-FEAT-022` requires the author to be
  told, and the refusal was written as `CommandRejected`, a variant
  `features/ambre-spec-prose-engine.md section 3` never declares -- the lists feature reports
  through the same missing variant. The enum carries one generic error instead, which names no
  command and no reason.
* **The font family is resolved against what the device loaded**, and pagination depends on the
  result (`features/ambre-spec-prose-pagination.md`). No rule says what an unknown family name
  resolves to for the reader who does not have it, only that measurement may then differ.
* **A named paragraph style cannot inherit from another.** The style store is flat and the cascade
  above has no level for a base style. The deferred design gives a style a `based_on` key and walks
  the chain, capped at five levels with a cycle guard; no container carries the key
  (`ambre-spec-prose-schema.md section 3`), and the Story is "The later writing set"
  (`ambre-roadmap.md`).
* **Nothing rules on a link's target.** The link mark carries a string, and no rule constrains the
  scheme, the length or what happens on an empty one. Opening it is a click rule
  (`PROSE-UI-070`, `PROSE-UI-071`, `features/ambre-spec-prose-selection.md`), and the shared
  open-URL affordance is Foundations'.
