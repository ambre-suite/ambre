# AMBRE -- FEATURE: WORD COUNT

**Type:** Vertical (feature)
**Implementation:** partial -- the five figures are built; the status bar that shows them is not

> How many words, characters, paragraphs and pages the document holds, shown in the status bar and
> refreshed as it is written.

Nothing here is shared state. The four counts and the page count are read from the document and
written back nowhere, so there is no two-peer table below. Two authors editing at once each see
the count of the document they both have.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-018` | `INV` | The document MUST hold none of the five figures. | this spec section 2; this spec section 4 | `ambre_prose::logic::derived::tests::the_document_holds_none_of_the_five_figures`; `ambre_prose::count::tests::counting_writes_nothing_to_the_document` | none | verified |
| `PROSE-FEAT-119` | `OBS` | The four text counts shown MUST be those of the document's text, once they have settled. | this spec section 2 | `ambre_prose::logic::derived::tests::the_counts_shown_are_those_of_the_text_once_settled` | none | verified |
| `PROSE-FEAT-038` | `OBS` | The page count shown MUST be the number of pages of the layout that is applied. | this spec section 2 | `ambre_prose::logic::derived::tests::the_page_count_is_the_one_of_the_layout_taken` | none | verified |
| `PROSE-FEAT-039` | `INV` | The character count MUST be taken over grapheme clusters, never over bytes or code points. | this spec section 2; `PROSE-FEAT-006` | `ambre_prose::count::tests::a_character_is_a_grapheme_cluster_and_not_a_code_point` | none | verified |
| `PROSE-FEAT-120` | `INV` | The character count MUST exclude every whitespace cluster, including the break between two paragraphs. | this spec section 2 | `ambre_prose::count::tests::whitespace_and_the_break_between_paragraphs_count_only_with_the_spaces` | none | verified |
| `PROSE-FEAT-121` | `OBS` | A second character count, taken over the same clusters and including the whitespace, MUST be shown beside it. | this spec section 2; this spec section 4 | `ambre_prose::count::tests::whitespace_and_the_break_between_paragraphs_count_only_with_the_spaces` | none | verified |
| `PROSE-FEAT-122` | `OBS` | An empty document MUST answer zero for the four text counts. | this spec section 2 | `ambre_prose::count::tests::an_empty_document_counts_zero_for_the_four_counts`; `::an_empty_paragraph_between_two_is_neither_a_paragraph_nor_a_break` | none | verified |
| `PROSE-FEAT-312` | `OBS` | Each of the five figures MUST carry its label, in the order the status bar shows them. | this spec section 3; `ambre-spec-chrome.md` | `ambre_prose::ui::tests::the_status_bar_reads_the_five_figures` | none | verified |
| `PROSE-FEAT-040` | `OBS` | The five figures MUST reach the reader through the status bar slot every application shares. | this spec section 3; `ambre-spec-chrome.md` | `ambre_app::host::prose::tests::the_page_count_reaches_the_shared_status_slot` -- a count the page state holds reaches the mounted shared slot as its five figures after one frame of the page view | none | verified |
| `PROSE-FEAT-123` | `INV` | Prose MUST NOT carry a status bar of its own. | this spec section 3; `ambre-spec-chrome.md` | crate-graph -- `ambre-prose` names neither `leptos` nor a DOM binding, so it cannot build a bar | none | verified |

### 1.2 The text changes

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-124` | `POST` | A run of changes MUST cost one recount of the four text counts, taken after the changes stop, not one per change. | this spec section 2; this spec section 4 | `ambre_prose::ui::tests::a_run_of_changes_costs_one_recount_after_it_stops` | none | verified |
| `PROSE-FEAT-125` | `FRAME` | Recounting MUST NOT touch the document, and MUST NOT restamp the page count. | this spec section 2; derived | `ambre_prose::logic::derived::tests::a_recount_writes_nothing_and_keeps_the_page_count` | none | verified |

### 1.3 A layout arrives

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-126` | `POST` | The page count MUST become that layout's once that layout is taken. | this spec section 2 | `ambre_prose::logic::derived::tests::the_page_count_is_the_one_of_the_layout_taken` | none | verified |
| `PROSE-FEAT-127` | `FRAME` | A new layout MUST NOT change any of the four text counts. | this spec section 2; derived | `ambre_prose::logic::derived::tests::a_new_layout_leaves_the_four_counts_alone` | none | verified |

* **Budget.** the word count, a row of `ambre-spec-budgets.md section 1`. It is what the
  delay below exists to keep off the keystroke path.
* **Boundary.** none: the count is derived in the Logic thread from the document already there,
  and it reaches the chrome as an event.
* **Verification.** 14 `verified`: host tests in `crates/ambre-prose`. `PROSE-FEAT-123` holds by
  the crate graph. `PROSE-FEAT-040` is held in the browser tier, where the page view publishes
  into the mounted slot.

---

## 2. MODEL

**Containers.** None. Nothing here is persisted (`PROSE-FEAT-018`).

**The five figures.** Four are text counts, recounted half a second after the last change. The
page count is read from the layout the engine took last (`PROSE-FEAT-038`).

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WordCount {
    pub words:                  Count<Words>,
    /// Grapheme clusters that are not whitespace (`PROSE-FEAT-039`).
    pub characters:             Count<Graphemes>,
    pub characters_with_spaces: Count<Graphemes>,
    pub paragraphs:             Count<Paragraphs>,
    /// From the layout taken last, not from the count (`PROSE-FEAT-038`).
    pub pages:                  Len<Page>,
}

/// The four text counts of the whole document. `pages` is left at zero: the engine stamps it
/// from the layout it took last.
pub fn compute_word_count(document: &LoroDoc) -> WordCount;
```

**The half second.** The view holds it, and sends `CountWords` once the changes stop
(`PROSE-FEAT-124`).

**The units.** Every text block of the document flow is counted, a table cell's included.

| Unit | What it is |
|---|---|
| character | a grapheme cluster, never a byte or a code point (`PROSE-FEAT-006`, `PROSE-FEAT-039`) |
| character, without spaces | a cluster that is not whitespace; the break between two paragraphs is whitespace (`PROSE-FEAT-120`) |
| character, with spaces | every cluster, and one per break between two paragraphs (`PROSE-FEAT-121`) |
| word | a UAX #29 word segment holding a letter or a digit: `well-known` is two, `3.14` is one |
| paragraph | a text block holding a visible character; an empty one, an image and a page break are none |

An empty document answers zero for the four counts (`PROSE-FEAT-122`); section 5 leaves its page
count open.

**Where it goes.** One event, `WordCountUpdated`, declared in
`features/ambre-spec-prose-engine.md section 3`. The view feeds it into the slot the shared status bar
offers each application (`PROSE-FEAT-040`). Trame fills the same slot with its own summary
(`2-Trame/features/ambre-spec-trame-status-bar.md`), and neither application builds a bar.

**Code:** `crates/ambre-prose/src/count.rs` holds the counts; `logic/derived.rs` holds the command
and the page stamp; `ui.rs` holds the delay.

---

## 3. SURFACE

| Trigger | Behaviour |
|---|---|
| typing, and any other change to the text | recomputes the four counts, half a second after the last one |
| a new layout | restamps the page count alone |
| a selection | nothing: the count over a selection is not built |
| opening the document | the counts of the whole document, once the first layout has arrived |

The status bar itself, its slots and its placement are the shared chrome's
(`ambre-spec-chrome.md`).

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The count is derived, never stored | storing the counts in the document | it adds a merge for a number nobody edits: two peers write it concurrently for a document they already agree on, and it is wrong for the moment between an edit and its update. Derived, the count is a function of the text, which is what a reader believes it is |
| The counts wait half a second | counting on every change | it puts a document walk inside the keystroke path, spending the keystroke budget on a figure the reader glances at and that is stale by one word either way regardless. A delay makes the cost proportional to pauses rather than to typing speed |
| The page count is not one of the counts | folding the page count into the delayed count | it shows a count the visible pages contradict. The page count comes from the layout, which is recomputed on its own schedule, so the delay leaves the two disagreeing exactly when the reader looks |
| Characters are grapheme clusters | counting code points | it makes one visible character count as several. Bytes and code points both split an accented letter or an emoji, so the same text counts differently depending on how it was typed, and a user counts what they see |
| Two character counts are answered | one character count | it is wrong for half of the people who read it. A limit on a form counts spaces and an author's target usually does not, and computing one from the other is impossible after the fact |
| The count fills the shared status bar slot | a Prose status bar | it is a second implementation of a shared surface. Every application has a summary to show in the same place, and one bar per application means three placements, three styles and three sets of chrome to keep in step |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The count over a selection is not built.** No command carries a set of blocks, and no gesture
  would produce one, so the common "words in the selection" reading is unreachable.
* **Nothing says how many pages an empty document has.** `PROSE-FEAT-038` reads the page count off
  the applied layout, and `features/ambre-spec-prose-pagination.md` has no rule for a document with
  no content. One page or zero is undecided, so `PROSE-FEAT-122` is stated over the four text
  counts alone rather than contradicting the layout.
