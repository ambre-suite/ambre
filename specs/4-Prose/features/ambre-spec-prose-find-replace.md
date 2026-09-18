# AMBRE -- FEATURE: PROSE FIND AND REPLACE

**Type:** Vertical (feature)
**Implementation:** design-only

> Searching the document, walking the matches, and replacing one or all of them. Searching changes
> nothing and is one reader's own; replacing is an edit like any other.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The shared rules are `ambre-spec-find.md`,
`FIND-001` through `FIND-007`: the search writes nothing, wraps, reveals its match, and a replace-all
is one undo item. Prose's own turn is what a document match is, and how a replace-all stays correct.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-148` | `OBS` | The matches of a search MUST be highlighted for the peer that started it and for no other peer. | this spec section 5 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-033` | `INV` | A search MUST match text in the body blocks and nothing else: comment and reply bodies, suggestion metadata, footnote bodies and the text of headers and footers MUST NOT be matched. | this spec section 3 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-034` | `INV` | A match MUST lie inside one block; a query spanning a block boundary MUST NOT match. | this spec section 3 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-035` | `INV` | Matches MUST NOT overlap: after one match, the search MUST resume past its end. | this spec section 3 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Open a search

The walk itself is `ambre-spec-find.md section 1.1`. Document order is Prose's own.

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-150` | `POST` | A search MUST report every match in the document, in document order. | this spec section 3 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-151` | `FRAME` | Opening a search MUST NOT write to the document, MUST NOT move the caret and MUST NOT change the selection: the selection is read to seed the query, and is not written. | derived | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Walk to another match

Where the walk lands, and that the match is brought on screen, are `ambre-spec-find.md`. What the
walk may not disturb is Prose's.

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-152` | `FRAME` | Walking to another match MUST NOT write to the document, MUST NOT change the selection and MUST NOT change the match list: only the current index moves. | derived | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.4 Replace the current match

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-153` | `POST` | Replacing the current match MUST replace exactly the range the search reported for it, with the text the gesture carries. | this spec section 4 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-155` | `FRAME` | Replacing one match MUST NOT change the text of any other match, and MUST NOT change the query or its case and whole-word switches. | derived | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.5 Replace every match

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-156` | `POST` | A replace-all MUST replace every match the search reported, each with the text the gesture carries. | this spec section 3 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-036` | `POST` | A replace-all MUST apply its replacements from the end of the document backwards, so that no replacement invalidates a later position. | this spec section 3 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-157` | `FRAME` | A replace-all MUST NOT change any character outside the ranges the search reported, and MUST NOT change the query or its switches. | derived | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.6 Close the search

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-158` | `POST` | Closing the search MUST end the session: the query, the match list and the index MUST be dropped, and no match MUST remain highlighted. | this spec section 4 | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-159` | `FRAME` | Closing the search MUST NOT write to the document, MUST NOT move the caret and MUST NOT change the selection. | derived | not built | unbuilt -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the search over the document and the replace-all, two rows of
  `ambre-spec-budgets.md section 1`.
* **Boundary.** none of its own. A search reads the document in the Logic thread and writes
  nothing. A replacement is an ordinary document write sealed by `ZK-001`.
* **Verification.** 14 `unimplemented`, nothing else, since `crates/ambre-prose` is a `lib.rs`; the
  shared rules and their coverage are `ambre-spec-find.md`. Counted here is what a Prose match is
  and what each gesture may not disturb. `PROSE-FEAT-035` and `PROSE-FEAT-156` are the two with an
  oracle: matching is a pure function of one string and one query. A replace-all is checked
  against the list it was handed: every reported range carries the replacement, and nothing
  outside them moved. A replace-all is not idempotent, no row claims it is, and the pair fails
  when a replacement shifted a later position.

---

## 2. WHAT EACH USER SEES

The shared scenario -- a search writes nothing, and a replace-all is one undo item -- is
`ambre-spec-find.md section 2`. Prose's own turn on the scope and the concurrent replace:

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| searches for a word | -- | nothing: no write | her own matches highlighted; Bob sees nothing of her search | none -- `PROSE-FEAT-148` |
| replaces all forty matches | types in the middle of the document | forty replacements and Bob's text | the replacements made, Bob's sentence untouched | none -- `PROSE-FEAT-157` |
| searches for a word that only exists in a comment | -- | nothing | no match, and the comment untouched | none -- `PROSE-FEAT-033` |
| searches for a phrase split across two paragraphs | -- | nothing | no match | none -- `PROSE-FEAT-034` |

---

## 3. MODEL

**Containers.** None. The query, the matches and the current index are session state on the Logic
thread, and nothing about a search is persisted (`ambre-spec-find.md`, `FIND-001`).

**The query and a match:**

```rust
pub struct FindQuery {
    pub text:           String,
    pub case_sensitive: bool,
    pub whole_word:     bool,
    /// The regular-expression search, which is a Story of its own and unbuilt.
    pub use_regex:      bool,
}

pub struct FindMatch { pub block_id: TreeID, pub range: (u32, u32) }

/// Every match in the document, in block order (`PROSE-FEAT-033`, `PROSE-FEAT-034`).
pub fn find_all(doc: &LoroDoc, tree: &LoroMovableTree, query: &FindQuery) -> Vec<FindMatch>;

/// The match spans inside one block's text, honouring case and whole-word, and never
/// overlapping (`PROSE-FEAT-035`).
fn match_offsets(hay: &str, query: &FindQuery) -> Vec<(u32, u32)>;

/// The replacements, from the end of the document backwards (`PROSE-FEAT-036`).
pub fn replace_all(doc: &LoroDoc, tree: &LoroMovableTree, query: &FindQuery,
                   replacement: &str) -> Vec<ProseCommand>;
```

* **What is searched** -- the body blocks of the movable tree, in document order, and nothing else.
  Comment and reply bodies, suggestion metadata, footnote bodies and header and footer text live in
  their own stores, each reached its own way. All are out of scope by decision (`PROSE-FEAT-033`).
* **Why a match stays inside a block** -- blocks are not concatenated for the search. A match is
  one range inside one block and needs no second identifier to be addressed (`PROSE-FEAT-034`). A
  query that spans a paragraph break therefore finds nothing.
* **Why replacements run backwards** -- each replacement shifts every position after it in its block.
  Applying them from the end of the document towards the start leaves every position not yet applied
  still valid. The whole set is computed once and sent as one run (`PROSE-FEAT-036`), which is also
  what makes it one undo item (`ambre-spec-find.md`, `FIND-006`).

**The commands and the events** (`ambre-bus`, the shared shape is `ambre-spec-find.md section 3`,
declared in `features/ambre-spec-prose-engine.md section 3`):

| Shape | Direction | Carries |
|---|---|---|
| `FindStart { query }` | view to engine | the query; the engine answers with every match |
| `FindNext`, `FindPrev` | view to engine | nothing: the engine moves its own index |
| `FindClose` | view to engine | nothing: the session ends |
| `ReplaceOne { replacement }` | view to engine | the text replacing the current match |
| `ReplaceAll { replacement }` | view to engine | the text replacing every match |
| `FindResults { matches, current }` | engine to view | the matches to highlight |
| `FindUpdated { current }` | engine to view | the new index alone |

**Code:** none. `crates/ambre-prose/src/lib.rs` is seven lines.

---

## 4. SURFACE

The shared chords, the empty-query clear and the reveal are `ambre-spec-find.md section 4`; the
reveal is the caret's own (`features/ambre-spec-prose-selection.md section 2`). The search panel is
chrome (`ambre-spec-chrome.md`). One match session with a live cursor, in every application, is the
"One find and replace" Story (`ambre-roadmap.md` E8).

---

## 5. REJECTED ALTERNATIVES

The shared refusals -- the scan in the Logic thread, the search that writes nothing, the one-undo
replace-all -- are `ambre-spec-find.md section 5`. Prose's own:

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The search covers body text only | searching every store | it returns matches with no place to show them: comments, footnotes and headers are separate stores with their own navigation, so a folded result list holds matches the user cannot see without leaving the page they are on. The editors this one is measured against draw the same line |
| A match does not cross a block boundary | concatenating the blocks | it buys a rare match and costs a position mapping: a second position space, a mapping back to blocks for every match, and that mapping is what breaks under a concurrent edit. Inside a block, a match is one identifier and one range |
| Matches do not overlap | reporting every occurrence including overlaps | it makes replace-all's outcome depend on the order, since replacing the first destroys the second, and no user expects two hits on the same characters |
| Replacements are applied backwards | replacing forwards and re-searching | it is quadratic and re-enters text already replaced: recomputing the positions after each replacement is a re-scan per match. Backwards keeps one scan and makes the whole set sendable at once |

---

## 6. NOT BUILT, AND NOT YET RULED

The shared gaps -- the live shared session, the stale highlight, the undebounced walk, the undefined
whole-word and case fold -- are in `ambre-spec-find.md section 6`. Prose's own:

* **Replace-one has no stated interaction with the current index.** After a replacement the replaced
  text may itself match the query, and nothing says whether the walk can loop on it.
* **Nothing rules on a selection that seeds the query across a block boundary.** Section 4 opens
  the search with the selection as the initial query, and `PROSE-FEAT-034` says a query spanning a
  block boundary never matches. Selecting two paragraphs and pressing the find chord opens a
  search that cannot hit. No rule says whether the seed is truncated to the first block, dropped,
  or taken whole.
* **Regular-expression search is a switch with no design.** The query carries the flag. No rule
  covers what a pattern may express, what it costs or what a malformed one reports. The Story
  that would build it is "The later writing set" (`ambre-roadmap.md` POST-V1).
