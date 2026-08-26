# AMBRE -- FEATURE: WORD COUNT

**Type:** Vertical (feature)
**Implementation:** design-only

> How many words, characters, paragraphs and pages the document holds, shown in the status bar and
> refreshed as it is written.

Nothing here is shared state: the four tallies and the page count are read from the document and
written back nowhere, so there is no two-peer table below. Two authors editing at once each see
the count of the document they both have.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `PROSE-FEAT-018` | The word count MUST be derived on demand and MUST NOT be stored in the document. | this spec section 2 | not built -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-038` | The page count MUST be read from the layout that is applied and refreshed when a new one arrives, never from the delayed text tally. | this spec section 2 | not built -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-039` | The character count MUST be taken over grapheme clusters, MUST exclude whitespace, and MUST be answered beside a second count that includes it; an empty document MUST answer zero everywhere. | this spec section 2 | not built -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-040` | The count MUST be published through the status bar every application shares; Prose MUST NOT carry a status bar of its own. | this spec section 3; `ambre-spec-chrome.md` | not built -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the word count, a row of `ambre-spec-budgets.md section 1`. It is what the
  delay below exists to keep off the keystroke path.
* **Boundary.** none: the count is derived in the Logic thread from the document already there,
  and it reaches the chrome as an event.
* **Verification.** 4 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`.
  `PROSE-FEAT-039` is the row a test reaches with fixtures alone -- an emoji made of several code
  points, a tab, an empty document -- and it is the one most likely to be wrong, because three
  plausible units give three different answers on the same text.

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
requires wherever a position is addressed (`PROSE-FEAT-006`), never over bytes or Rust characters:
an accented letter written as two code points is one character to the reader. The count without
spaces excludes every whitespace cluster, including the break between two paragraphs; the count
with spaces includes them. An empty document answers zero for all five (`PROSE-FEAT-039`).

**Where it goes.** One event, `WordCountUpdated`, declared in
`features/ambre-spec-prose-engine.md section 3`. The view feeds it into the slot the shared status bar
offers each application (`PROSE-FEAT-040`): Trame fills the same slot with its own summary
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

## 4. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The count is derived, never stored | a stored count is a value two peers write concurrently for a document they already agree on, and it is wrong for the moment between an edit and its update. Deriving it makes the count a function of the text, which is what a reader believes it is | storing the tallies in the document, which adds a merge for a number nobody edits |
| The tallies wait half a second | counting the words of a long document per keystroke spends the keystroke budget on a figure the reader glances at, and the figure is stale by one word either way regardless. A delay makes the cost proportional to pauses rather than to typing speed | counting on every change, which puts a document walk inside the keystroke path |
| The page count is not one of the tallies | it comes from the layout, which is recomputed on its own schedule. Tying it to the delayed tally would leave it disagreeing with the pages on screen for as long as the delay lasts, which is exactly when the reader looks at it | folding the page count into the delayed tally, which shows a count the visible pages contradict |
| Characters are grapheme clusters | a user counts what they see. Bytes and code points both split an accented letter or an emoji into several, and the same text then counts differently depending on how it was typed | counting code points, which makes one visible character count as several |
| Two character counts are answered | the two audiences are different -- a limit on a form counts spaces, an author's target usually does not -- and computing one from the other is impossible after the fact | one character count, which is wrong for half of the people who read it |
| The count fills the shared status bar slot | every application has a summary to show in the same place, and a status bar per application means three placements, three styles and three sets of chrome to keep in step | a Prose status bar, which is a second implementation of a shared surface |

---

## 5. NOT BUILT, AND NOT YET RULED

* **What a word is, is not defined.** The tally is named and its unit is not: whether a
  hyphenated compound, a number with a decimal separator or a string of punctuation counts as one
  word decides the figure, and no rule states it.
* **What is counted is not stated either.** The function takes the whole document or a set of
  blocks, and nothing says whether table cells, footnotes, headers and footers are inside the
  whole-document figure. The character rule says they follow the same scope, and the scope is
  undecided.
* **The count over a selection has no trigger.** The function accepts a block list and no gesture
  produces one, so the common "words in the selection" reading is unreachable.
* **The paragraph tally has no rule for empty blocks.** Whether an empty paragraph, an image or a
  page break counts as a paragraph is undecided.
