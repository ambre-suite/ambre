# AMBRE -- FEATURE: INLINE AND PARAGRAPH STYLES

**Type:** Vertical (feature)

> Bold, italic, colour, size, links: formatting applied to a range of text, and the paragraph
> settings underneath it. What a character finally looks like is decided at read time. It reads
> the mark on it, the paragraph it sits in, and the defaults of the document.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-007` | `INV` | An inline mark MUST expand or not expand at its boundaries according to the registry alone (`ambre-spec-prose-schema.md section 3`), never according to the applying feature. | this spec section 3; `PROSE-007` | `ambre_prose::logic::marks::tests::typing_at_a_span_edge_follows_the_registry`; `ambre_prose::mark::tests::a_formatting_mark_swallows_an_insertion_at_its_edge`; `ambre_prose::mark::tests::an_anchor_mark_never_swallows_an_insertion`; structural -- `ambre_prose::mark::expand_type_for` is the one reader of the key, and no handler of `ambre_prose::logic::marks` states a policy | none | verified |
| `PROSE-FEAT-311` | `INV` | The value of a property MUST be taken from the first level that carries it, in the order inline mark, block type, document default. | this spec section 3 | `ambre_prose::style::tests::a_mark_wins_over_the_block_which_wins_over_the_defaults` | none | verified |
| `PROSE-FEAT-020` | `INV` | A named paragraph style MUST sit between the inline mark and the block type: a property it carries MUST win over the block type and lose to the inline mark. | this spec section 3 | not built | unbuilt -- lands with the `The later writing set` Story (`ambre-roadmap.md` POST-V1). No command writes a named paragraph style yet | unimplemented |
| `PROSE-FEAT-177` | `INV` | The cascade MUST terminate: a property no level carries MUST resolve to a decided absence, and a reader of the resolved style MUST NOT look further up. | this spec section 3 | `ambre_prose::style::tests::a_property_no_level_carries_resolves_to_a_decided_absence`; type-level -- every field of `ambre_prose::style::ResolvedStyle` is a decided value, and none of them is a "look further up" | none | verified |
| `PROSE-FEAT-173` | `INV` | Superscript and subscript MUST both be storable on one range: writing either one MUST NOT clear the other. | this spec section 3 | `ambre_prose::logic::convergence_tests::two_peers_raising_and_lowering_one_word_resolve_one_style` | none | verified |
| `PROSE-FEAT-174` | `INV` | Where both are stored on one range, the resolved style MUST be the raised one, by a priority fixed in the reader and not by which mark was written last. | this spec section 3 | `ambre_prose::style::tests::raised_beats_lowered_by_a_fixed_priority` | none | verified |
| `PROSE-FEAT-021` | `CONC` | Two peers holding both marks MUST resolve one style, whichever order the two marks arrived in. | this spec section 3 | `ambre_prose::logic::convergence_tests::two_peers_raising_and_lowering_one_word_resolve_one_style` | none | verified |
| `PROSE-FEAT-008` | `CONC` | Two concurrent applications of the same mark to overlapping ranges MUST converge without producing a torn or duplicated span. | this spec section 2 | `ambre_prose::logic::convergence_tests::two_peers_bolding_overlapping_ranges_keep_one_run` | none | verified |
| `PROSE-FEAT-184` | `CONC` | Two peers writing two values of one mark key over one range MUST converge on one of the two, and both MUST resolve that same one. | this spec section 5 | `ambre_prose::logic::convergence_tests::two_peers_setting_two_sizes_resolve_the_same_one` | none | verified |
| `PROSE-FEAT-337` | `CONC` | Two peers writing value marks over overlapping selections, or one writing while the other types inside the selection, MUST converge on the same text and the same runs. | this spec section 2; `PROSE-FEAT-184` | `ambre_prose::logic::convergence_tests::two_peers_setting_two_fonts_over_overlapping_selections_converge`; `ambre_prose::logic::convergence_tests::a_font_over_a_selection_and_typing_inside_it_converge` | none | verified |
| `PROSE-FEAT-338` | `CONC` | Two peers changing the paragraph style, the paragraph properties or the default fonts over one selection MUST converge, and a property one peer alone set MUST survive. | this spec section 2; `PROSE-FEAT-176` | `ambre_prose::logic::convergence_tests::two_peers_making_one_selection_a_heading_and_a_list_converge` -- one kind per block, a declared tree; `ambre_prose::logic::convergence_tests::two_peers_setting_two_paragraph_properties_of_one_selection_keep_both`; `ambre_prose::logic::convergence_tests::two_peers_setting_the_body_font_converge` | none | verified |
| `PROSE-FEAT-176` | `CONC` | Two authors setting two different properties of one paragraph MUST both keep their property: neither MUST overwrite the other's. | this spec section 3 | `ambre_prose::logic::convergence_tests::two_peers_setting_two_paragraph_properties_both_keep_theirs` | none | verified |
| `PROSE-FEAT-175` | `INV` | Two spellings of one colour MUST be stored as one: the bytes a peer writes for a colour MUST depend on the colour alone. | this spec section 3; `PROSE-008` | `ambre_prose::logic::marks::tests::a_colour_reaches_the_document_in_one_spelling`; `ambre_prose::style::tests::two_spellings_of_one_colour_resolve_to_one_value` | none | verified |
| `PROSE-FEAT-023` | `IO` | Reading a stored colour and writing it back MUST leave the bytes unchanged. | this spec section 3; `PROSE-008` | `ambre_prose::logic::marks::tests::a_stored_colour_written_back_leaves_the_bytes_unchanged` | none | verified |
| `PROSE-FEAT-313` | `INV` | A font size MUST read as at most 1638 points, the word-processing cap: a stored, received or pasted size past it reads as 1638. | this spec section 3; `CANVAS-088`; `PROSE-FEAT-022` | `ambre_prose::style::tests::a_stored_size_past_the_prose_cap_reads_as_the_cap` -- a mark of a million points, and a typed 1639 refused; `ambre_prose::doc::tests::a_default_size_past_the_cap_reads_as_the_cap` -- the document default; `ambre_prose::logic::clipboard::tests::a_crafted_payload_writes_only_the_marks_a_typed_mark_may` -- an own-bytes paste; `ambre_prose::logic::clipboard::html_paste::colours_and_sizes_land_canonical` -- an HTML paste | none | verified |
| `PROSE-FEAT-309` | `OBS` | A paragraph's first line MUST start at its first-line indent, counted from its left indent, and every later line at the left indent. | this spec section 3 | `ambre_prose::layout::run::tests::the_first_line_starts_at_its_indent` | none | verified |
| `PROSE-FEAT-310` | `OBS` | A negative first-line indent MUST start the first line that far left of the left indent. | this spec section 3 | not built | unbuilt -- lands with the `Hanging indents` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Apply a mark to a range

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-178` | `POST` | The mark MUST cover the given range and no character outside it. | this spec section 3 | `ambre_prose::logic::marks::tests::a_mark_covers_the_range_and_no_character_outside_it` | none | verified |
| `PROSE-FEAT-290` | `POST` | The value stored MUST be the canonical form of the value supplied. | this spec section 3 | `ambre_prose::logic::marks::tests::a_colour_reaches_the_document_in_one_spelling`; `ambre_prose::logic::marks::tests::a_size_reaches_the_document_in_its_canonical_spelling` | none | verified |
| `PROSE-FEAT-022` | `FAIL` | A mark value that fails the grammar of its key MUST be refused before the document is written, and MUST be reported to the author. | this spec section 3 | `ambre_prose::logic::marks::tests::a_value_the_grammar_refuses_never_reaches_the_document` | none | verified |
| `PROSE-FEAT-298` | `FAIL` | Applying a mark MUST be refused under an anchor key: a comment, a suggestion, a footnote, a cross reference or a header field. | this spec section 3; `ambre-spec-prose-schema.md section 3` | `ambre_prose::logic::marks::tests::a_style_command_never_writes_an_anchor_it_does_not_own` | none | verified |
| `PROSE-FEAT-179` | `FRAME` | Applying a mark MUST NOT change the text of the block, any other key of the registry over that range, or the block's paragraph settings. | this spec section 3 | `ambre_prose::logic::marks::tests::applying_a_mark_leaves_the_text_and_the_other_keys_alone` | none | verified |

### 1.3 Remove a mark from a range

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-180` | `POST` | After removal the named key MUST cover no character of the range. | this spec section 3 | `ambre_prose::logic::marks::tests::removing_one_key_clears_the_range_and_keeps_the_rest` | none | verified |
| `PROSE-FEAT-287` | `POST` | After removal the named key MUST still cover every character outside the range that it covered before. | this spec section 3 | `ambre_prose::logic::marks::tests::removing_one_key_clears_the_range_and_keeps_the_rest` | none | verified |
| `PROSE-FEAT-181` | `FRAME` | Removing one key MUST NOT change the text of the block, or any other key of the registry over that range. | this spec section 3 | `ambre_prose::logic::marks::tests::removing_one_key_clears_the_range_and_keeps_the_rest` | none | verified |
| `PROSE-FEAT-299` | `FAIL` | Removing a mark MUST be refused under an anchor key: only the command that owns the anchor takes it off. | this spec section 3; `ambre-spec-prose-schema.md section 3` | `ambre_prose::logic::marks::tests::a_style_command_never_removes_an_anchor_it_does_not_own` | none | verified |

### 1.4 Clear the formatting of a range

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-024` | `POST` | Clearing MUST remove every presentation mark and the link over the range. | this spec section 3 | `ambre_prose::logic::marks::tests::clearing_strips_the_formatting_and_spares_the_anchors` | none | verified |
| `PROSE-FEAT-288` | `POST` | Clearing MUST leave the range's text as it was. | this spec section 3 | `ambre_prose::logic::marks::tests::clearing_strips_the_formatting_and_spares_the_anchors` | none | verified |
| `PROSE-FEAT-182` | `FRAME` | Clearing MUST NOT touch a comment, a suggestion, a footnote, a cross reference or a dynamic field over the range: each carries an identity a formatting reset MUST NOT destroy. | this spec section 3 | `ambre_prose::logic::marks::tests::clearing_strips_the_formatting_and_spares_the_anchors` -- the comment survives the reset; `ambre_prose::logic::marks::tests::clearing_the_formatting_leaves_an_anchor_in_place` | none | verified |

### 1.5 Set a paragraph's settings

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-025` | `POST` | Setting a paragraph's settings MUST write only the properties whose value differs from what is stored. | this spec section 3 | `ambre_prose::logic::marks::tests::setting_a_paragraph_writes_only_what_differs` | none | verified |
| `PROSE-FEAT-183` | `FRAME` | Setting a paragraph's settings MUST NOT change the block's text, the inline marks on it, or the settings of any other block. | this spec section 3 | `ambre_prose::logic::marks::tests::setting_a_paragraph_writes_only_what_differs` | none | verified |

* **Budget.** the resolution of the effective style for a block, a row of
  `ambre-spec-budgets.md section 1`. It runs once per block per layout, so it is on the
  pagination path rather than beside it.
* **Boundary.** none of its own: a mark is a document write, and `ZK-001` seals every payload
  before it leaves the client.
* **Verification.** 29 `verified`, 2 `unimplemented`. `PROSE-FEAT-311` holds the three levels of the
  cascade that resolve today. `PROSE-FEAT-020` is the named paragraph style, the fourth level: no
  command writes one yet. `PROSE-FEAT-310` is the hanging indent. The layout carries the first-line
  indent as a box ahead of the text, and a box has no negative width.

  `PROSE-FEAT-021`, `PROSE-FEAT-008`, `PROSE-FEAT-184` and `PROSE-FEAT-176` are the four the
  two-peer table below is written for. The four `FRAME` rows are each held by the same test as
  their own command: it asserts on what did not move.

---

## 2. WHAT EACH USER SEES

Two peers over one sentence. The last column names the test each row rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| bolds words two to five | bolds words four to seven at the same moment | one bold span over words two to seven | one continuous bold run, no seam `ambre_prose::logic::convergence_tests::two_peers_bolding_overlapping_ranges_keep_one_run` |
| makes a word superscript | makes the same word subscript | both marks, neither cleared | the word raised, on both screens `ambre_prose::logic::convergence_tests::two_peers_raising_and_lowering_one_word_resolve_one_style` |
| sets the paragraph's space before | sets the same paragraph's alignment | both settings | a centred paragraph with the new space above it `ambre_prose::logic::convergence_tests::two_peers_setting_two_paragraph_properties_both_keep_theirs` |
| sets the font size to 12 | sets the same size to 14 | one of the two | one size, the same for both `ambre_prose::logic::convergence_tests::two_peers_setting_two_sizes_resolve_the_same_one` |
| sets a font over two paragraphs | sets another font over the second half of the same range | one font per character | the same fonts on both screens | `ambre_prose::logic::convergence_tests::two_peers_setting_two_fonts_over_overlapping_selections_converge` |
| makes two paragraphs a heading | makes the same two a list at the same moment | one kind per block | the same kind on both screens | `ambre_prose::logic::convergence_tests::two_peers_making_one_selection_a_heading_and_a_list_converge` |
| clears formatting over a commented range | -- | the comment, without the bold | plain text, the comment still anchored `ambre_prose::logic::marks::tests::clearing_strips_the_formatting_and_spares_the_anchors` |
| types at the end of a bold word | -- | the new text inside the bold span | the typing continues bold | `ambre_prose::logic::marks::tests::typing_at_a_span_edge_follows_the_registry` |
| types just after a link | -- | the new text outside the link | plain text after the link | `ambre_prose::logic::marks::tests::typing_at_a_span_edge_follows_the_registry` |

---

## 3. MODEL

**Containers.** None of its own. The mark registry, its keys, its value grammars and its expansion
column are `ambre-spec-prose-schema.md section 3`; the paragraph settings are the per-property map
declared there. This document states the rules whose cause is that format, and restates none of it.

**The cascade.** A property is looked for at four levels and taken from the first that carries it
(`PROSE-FEAT-020`):

1. the inline mark on the character;
2. the paragraph's own settings, through its style identifier;
3. the built-in settings of the block type -- a heading level, a code block;
4. the document defaults.

Levels one, three and four resolve today. Level two reads a store no command writes into, so it
carries nothing: see section 6.

```rust
/// The result of the cascade for one run of text. Every field is a decided value: a caller
/// draws from it and never looks further up (`PROSE-FEAT-020`, `PROSE-FEAT-177`).
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedStyle {
    pub bold:          Weight,         // Regular | Bold
    pub italic:        Slant,          // Upright | Italic
    pub underline:     Underline,      // Plain | Underlined
    pub strikethrough: Strikethrough,  // Plain | Struck
    /// Raised, lowered or neither, never both (`PROSE-FEAT-174`).
    pub script:        Script,         // Baseline | Raised | Lowered
    pub code_inline:   InlineCode,     // Prose | Code
    pub color:         Option<Rgba>,
    pub background:    Option<Rgba>,
    pub font_size:     FontSize,
    pub font_family:   Option<FontFamily>,
    pub link:          Option<LinkUrl>,
}
```

**Raised and lowered text.** Superscript and subscript are two independent marks, and neither
write clears the other, so no write ordering is needed (`PROSE-FEAT-173`). Resolution applies a
fixed priority -- raised wins -- at read time (`PROSE-FEAT-174`). Because the priority is fixed
rather than a tie-break on arrival order, both peers resolve the same style from the same document
(`PROSE-FEAT-021`).

**The paragraph's settings:**

```rust
/// `None` leaves the property alone, never clears it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ParagraphStyle {
    pub align:          Option<Align>,
    pub indent_first:   Option<PrintLength<Pt>>,
    pub indent_left:    Option<PrintLength<Pt>>,
    pub indent_right:   Option<PrintLength<Pt>>,
    pub space_before:   Option<PrintLength<Pt>>,
    pub space_after:    Option<PrintLength<Pt>>,
    pub line_height:    Option<LineHeight>,
    /// Read by pagination: a block that must not be split, and one that must not be
    /// parted from the next (`features/ambre-spec-prose-pagination.md`).
    /// Each one a flag of `define_flag!`, which the store writes as a `bool`.
    pub keep_together:  Option<KeepTogether>,   // Splittable, Together
    pub keep_with_next: Option<KeepWithNext>,   // Free, WithNext
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align { Left = 0, Center = 1, Right = 2, Justify = 3 }

/// The mode decides the unit of its number, so each mode carries its own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineHeight { Multiple(LineSpacingMultiple), Exact(PrintLength<Pt>), AtLeast(PrintLength<Pt>) }
```

The command carries every property, each optional; the handler compares what it names against
what is stored and writes only the keys that differ (`PROSE-FEAT-025`). The command stays coarse
and the storage stays granular, which is what lets two authors change two properties of one
paragraph at once (`PROSE-FEAT-176`).

**The four commands** (`ambre_prose::message`, declared in `features/ambre-spec-prose-engine.md section 3`):

| Shape | Carries |
|---|---|
| `ApplyMark { block_id, range, key, value }` | one key of the registry, and its value checked by the grammar (`PROSE-FEAT-178`) |
| `RemoveMark { block_id, range, key }` | one key, over a range (`PROSE-FEAT-180`) |
| `ClearInlineMarks { block_id, range }` | nothing but the range (`PROSE-FEAT-024`) |
| `SetParagraphStyle { block_id, style }` | the whole set, diffed by the handler (`PROSE-FEAT-025`) |

**Validating a value.** The value travels as a string, and each key has its own grammar. The
grammar is checked before the document is touched. A malformed value is refused whole and reported,
so no unparseable value ever reaches the format (`PROSE-FEAT-022`).

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarkValueError {
    /// A colour that is neither six nor eight hexadecimal digits behind a hash.
    BadColor(String),
    /// A size that is not finite, or outside one to 1 638 points.
    BadFontSize(String),
    /// A value supplied for a key that takes none, or missing for a key that requires one.
    ArityMismatch { key: MarkKey },
    /// A key the registry does not declare.
    UnknownKey(MarkKey),
}

/// Parse a colour into red, green, blue and alpha packed in that order. The six-digit form
/// is opaque, so the two spellings of one opaque colour parse to one value (`PROSE-FEAT-175`)
/// and a round trip of the stored bytes is stable (`PROSE-FEAT-023`).
pub fn parse_color(raw: &str) -> Result<Rgba, MarkValueError>;
/// Parse a size in points. The ceiling is the one the office formats round-trip.
pub fn parse_font_size(raw: &str) -> Result<FontSize, MarkValueError>;
```

The refusal has to reach the author, and the event that carries it is not declared: see section 6.

**What clearing removes.** Clearing formatting removes the eleven presentation marks and the link.
It removes no comment, suggestion, footnote or cross reference, and no dynamic field. Each of those
carries an identity or a binding that a formatting reset must not destroy (`PROSE-FEAT-182`).
Removing one of them is its own command.

**Code:** `crates/ambre-prose/src/style.rs` for the cascade, `crates/ambre-prose/src/mark.rs` for
the registry, `crates/ambre-prose/src/message/mark.rs` for the value grammars, and
`crates/ambre-prose/src/logic/marks.rs` for the write path.

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

* **A refusal travels, and nothing shows it.** `PROSE-FEAT-022` requires the author to be told.
  The engine names the command and the reason (`PROSE-105`, `PROSE-106`), and a malformed mark
  value is one of the reasons it states. What is missing is the view: nothing rules where a
  refusal appears or how long it stays.
* **Nothing says which side of the registry a new key falls on.** Clearing removes the
  presentation marks and the link, and spares the annotations. The split is a sentence of
  section 3 rather than a column of the registry. A key added to
  `ambre-spec-prose-schema.md section 3` falls on neither side until someone edits this document.
* **The font family is resolved against what the device loaded**, and pagination depends on the
  result (`features/ambre-spec-prose-pagination.md`). No rule says what an unknown family name
  resolves to for the reader who does not have it, only that measurement may then differ.
* **A named paragraph style carries nothing yet.** Level two of the cascade reads the `styles`
  store, and no command writes an entry into it. Every property therefore falls through to the
  block type and the document defaults, which is why `PROSE-FEAT-020` is `unimplemented`. The store and
  the command that fills it land with "The later writing set" (`ambre-roadmap.md` POST-V1).
* **A named paragraph style cannot inherit from another.** The style store is flat and the cascade
  above has no level for a base style. The deferred design gives a style a `based_on` key and walks
  the chain, capped at five levels with a cycle guard. No container carries the key
  (`ambre-spec-prose-schema.md section 3`), and the Story is
  "The later writing set" (`ambre-roadmap.md` POST-V1).
* **Nothing rules on a link's target.** The link mark carries a string, and no rule constrains the
  scheme, the length or what happens on an empty one. Opening it is a click rule
  (`PROSE-UI-070`, `PROSE-UI-071`, `features/ambre-spec-prose-selection.md`), and the shared
  open-URL affordance is Foundations'.
