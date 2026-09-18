# AMBRE -- FEATURE: WORD COUNT

**Type:** Vertical (feature)
**Implementation:** design-only

> How many words, characters, paragraphs and pages the document holds, shown in the status bar and
> refreshed as it is written.

Nothing here is shared state. The four tallies and the page count are read from the document and
written back nowhere, so there is no two-peer table below. Two authors editing at once each see
the count of the document they both have.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-018` | `INV` | The document MUST hold none of the five figures. | this spec section 2; this spec section 4 | not built | unbuilt -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-119` | `OBS` | The four text tallies shown MUST be those of the document's text, once they have settled. | this spec section 2 | not built | unbuilt -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-038` | `OBS` | The page count shown MUST be the number of pages of the layout that is applied. | this spec section 2 | not built | unbuilt -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-039` | `INV` | The character count MUST be taken over grapheme clusters, never over bytes or code points. | this spec section 2; `PROSE-FEAT-006` | not built | unbuilt -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-120` | `INV` | The character count MUST exclude every whitespace cluster, including the break between two paragraphs. | this spec section 2 | not built | unbuilt -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-121` | `OBS` | A second character count, taken over the same clusters and including the whitespace, MUST be shown beside it. | this spec section 2; this spec section 4 | not built | unbuilt -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-122` | `OBS` | An empty document MUST answer zero for the four text tallies. | this spec section 2 | not built | unbuilt -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-040` | `OBS` | The five figures MUST reach the reader through the status bar slot every application shares. | this spec section 3; `ambre-spec-chrome.md` | not built | unbuilt -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-123` | `INV` | Prose MUST NOT carry a status bar of its own. | this spec section 3; `ambre-spec-chrome.md` | not built | unbuilt -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 The text changes

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-124` | `POST` | A run of changes MUST cost one recount of the four text tallies, taken after the changes stop, not one per change. | this spec section 2; this spec section 4 | not built | unbuilt -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-125` | `FRAME` | Recounting MUST NOT touch the document, and MUST NOT restamp the page count. | this spec section 2; derived | not built | unbuilt -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 A layout arrives

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-126` | `POST` | The page count MUST become that layout's, on the frame it is applied. | this spec section 2 | not built | unbuilt -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-127` | `FRAME` | A new layout MUST NOT change any of the four text tallies. | this spec section 2; derived | not built | unbuilt -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the word count, a row of `ambre-spec-budgets.md section 1`. It is what the
  delay below exists to keep off the keystroke path.
* **Boundary.** none: the count is derived in the Logic thread from the document already there,
  and it reaches the chrome as an event.
* **Verification.** 13 `unimplemented`, nothing else: `crates/ambre-prose` is a seven-line
  `lib.rs`. The rule most likely to be wrong is the unit, where three plausible answers exist
  (`PROSE-FEAT-039`). The next is the character count, where the break between two paragraphs is
  whitespace the tree does not store (`PROSE-FEAT-120`). Nine rows are host tests over fixtures -- an emoji
  made of several code points, a tab, an empty document. The four that read a layout land in the
  wasm tier (`ambre-spec-testing.md section 6`).

---

## 2. MODEL

**Containers.** None. Nothing here is persisted (`PROSE-FEAT-018`).

**The five figures, and the two triggers.** Four are text tallies, recomputed after a change and
delayed by half a second so a fast typist does not pay for them per keystroke. The fifth is not a
tally at all: the page count is read from the layout, and refreshed when a layout arrives
(`PROSE-FEAT-038`). Both triggers write into one cached result, so the status bar shows the latest
of each.

```rust
pub struct WordCount {
    pub words:                  u32,
    /// Grapheme clusters that are not whitespace (`PROSE-FEAT-039`).
    pub characters:             u32,
    pub characters_with_spaces: u32,
    pub paragraphs:             u32,
    /// From the applied layout, not from the tally (`PROSE-FEAT-038`).
    pub pages:                  u32,
}

/// Recompute the four text tallies. `pages` is left at zero: the caller stamps it from the
/// layout. `blocks` is `None` for the whole document, or the selected blocks.
pub fn compute_word_count(doc: &LoroDoc, tree: &LoroMovableTree, layout: &PageLayout,
                          blocks: Option<&[TreeID]>) -> WordCount;
```

**What a character is.** Counting is over grapheme clusters, the same unit the feature layer
requires wherever a position is addressed (`PROSE-FEAT-006`), never over bytes or Rust characters.
An accented letter written as two code points is one character to the reader. The count without
spaces excludes every whitespace cluster, including the break between two paragraphs
(`PROSE-FEAT-120`); the count with spaces includes them (`PROSE-FEAT-121`). An empty document
answers zero for the four text tallies (`PROSE-FEAT-122`). What it answers for the fifth is the
applied layout's, and section 5 states that nothing decides it.

**Where it goes.** One event, `WordCountUpdated`, declared in
`features/ambre-spec-prose-engine.md section 3`. The view feeds it into the slot the shared status bar
offers each application (`PROSE-FEAT-040`). Trame fills the same slot with its own summary
(`2-Trame/features/ambre-spec-trame-status-bar.md`), and neither application builds a bar.

**Code:** none. `crates/ambre-prose/src/lib.rs` is seven lines.

---

## 3. SURFACE

| Trigger | Behaviour |
|---|---|
| typing, and any other change to the text | recomputes the four tallies, half a second after the last one |
| a new layout | restamps the page count alone |
| a selection | not ruled on: the count over selected blocks is possible and nothing asks for it |
| opening the document | the tallies of the whole document, once the first layout has arrived |

The status bar itself, its slots and its placement are the shared chrome's
(`ambre-spec-chrome.md`).

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The count is derived, never stored | storing the tallies in the document | it adds a merge for a number nobody edits: two peers write it concurrently for a document they already agree on, and it is wrong for the moment between an edit and its update. Derived, the count is a function of the text, which is what a reader believes it is |
| The tallies wait half a second | counting on every change | it puts a document walk inside the keystroke path, spending the keystroke budget on a figure the reader glances at and that is stale by one word either way regardless. A delay makes the cost proportional to pauses rather than to typing speed |
| The page count is not one of the tallies | folding the page count into the delayed tally | it shows a count the visible pages contradict. The page count comes from the layout, which is recomputed on its own schedule, so the delay leaves the two disagreeing exactly when the reader looks |
| Characters are grapheme clusters | counting code points | it makes one visible character count as several. Bytes and code points both split an accented letter or an emoji, so the same text counts differently depending on how it was typed, and a user counts what they see |
| Two character counts are answered | one character count | it is wrong for half of the people who read it. A limit on a form counts spaces and an author's target usually does not, and computing one from the other is impossible after the fact |
| The count fills the shared status bar slot | a Prose status bar | it is a second implementation of a shared surface. Every application has a summary to show in the same place, and one bar per application means three placements, three styles and three sets of chrome to keep in step |

---

## 5. NOT BUILT, AND NOT YET RULED

* **What a word is, is not defined.** The tally is named and its unit is not. Whether a
  hyphenated compound, a number with a decimal separator or a string of punctuation counts as one
  word decides the figure. No rule states it.
* **What is counted is not stated either.** The function takes the whole document or a set of
  blocks. Nothing says whether table cells, footnotes, headers and footers are inside the
  whole-document figure. The character rule says they follow the same scope, and the scope is
  undecided.
* **The count over a selection has no trigger.** The function accepts a block list and no gesture
  produces one, so the common "words in the selection" reading is unreachable.
* **The paragraph tally has no rule for empty blocks.** Whether an empty paragraph, an image or a
  page break counts as a paragraph is undecided, and it decides `PROSE-FEAT-122` too. A document
  an author has not typed into holds one empty paragraph, not nothing.
* **Nothing says how many pages an empty document has.** `PROSE-FEAT-038` reads the page count off
  the applied layout, and `features/ambre-spec-prose-pagination.md` has no rule for a document with
  no content. One page or zero is undecided, so `PROSE-FEAT-122` is stated over the four text
  tallies alone rather than contradicting the layout.
