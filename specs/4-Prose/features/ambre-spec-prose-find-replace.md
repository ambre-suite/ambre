# AMBRE -- FEATURE: FIND AND REPLACE

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

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `PROSE-FEAT-033` | A search MUST cover the body blocks in document order and nothing else: comment, footnote, header and footer text MUST NOT be matched. | this spec section 3 | not built -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-034` | A match MUST lie inside one block; a query spanning a block boundary MUST NOT match. | this spec section 3 | not built -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-035` | Matches MUST NOT overlap: after one match, the search MUST resume past its end. | this spec section 3 | not built -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-036` | A replace-all MUST apply its replacements from the end of the document backwards, so that no replacement invalidates a later position. | this spec section 3 | not built -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the search over the document and the replace-all, two rows of
  `ambre-spec-budgets.md section 1`.
* **Boundary.** none of its own: a search reads the document in the Logic thread and writes
  nothing, and a replacement is an ordinary document write sealed by `ZK-001`.
* **Verification.** 4 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`.
  `PROSE-FEAT-035` and `PROSE-FEAT-036` are the two with an oracle: matching is a pure function of
  one string and one query, and a replace-all is idempotent -- after it, the query matches nothing,
  which is the property that fails when a replacement shifted a later position. The shared rules and
  their coverage are in `ambre-spec-find.md`.

---

## 2. WHAT EACH USER SEES

The shared scenario -- a search writes nothing, and a replace-all is one undo item -- is
`ambre-spec-find.md section 2`. Prose's own turn on the scope and the concurrent replace:

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| replaces all forty matches | types in the middle of the document | forty replacements and Bob's text | the replacements made, Bob's sentence untouched | none -- `PROSE-FEAT-036` |
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
  their own stores, each reached its own way, and are out of scope by decision (`PROSE-FEAT-033`).
* **Why a match stays inside a block** -- blocks are not concatenated for the search, so a match is
  one range inside one block and needs no second identifier to be addressed (`PROSE-FEAT-034`); a
  query that spans a paragraph break therefore finds nothing.
* **Why replacements run backwards** -- each replacement shifts every position after it in its block.
  Applying them from the end of the document towards the start leaves every position not yet applied
  still valid, so the whole set is computed once and sent as one run (`PROSE-FEAT-036`), which is also
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
"One find and replace" Story.

---

## 5. DECISIONS

The shared decisions -- the scan in the Logic thread, the search that writes nothing, the one-undo
replace-all -- are `ambre-spec-find.md section 5`. Prose's own:

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The search covers body text only | comments, footnotes and headers are separate stores with their own navigation, and folding them into one result list makes a match the user cannot see without leaving the page they are on. The editors this one is measured against draw the same line | searching every store, which returns matches with no place to show them |
| A match does not cross a block boundary | concatenating the document to search it requires a second position space and a mapping back to blocks for every match, and the mapping is what breaks under a concurrent edit. Keeping matches inside a block makes a match one identifier and one range | concatenating the blocks, which buys a rare match and costs a position mapping |
| Matches do not overlap | overlapping matches make replace-all ambiguous -- replacing the first destroys the second -- and no user expects two hits on the same characters | reporting every occurrence including overlaps, which makes replace-all's outcome depend on the order |
| Replacements are applied backwards | the alternative is to recompute the positions after each replacement, which is a re-scan per match and turns replace-all into a loop over the document. Running backwards keeps one scan and makes the whole set sendable at once | replacing forwards and re-searching, which is quadratic and re-enters text already replaced |

---

## 6. NOT BUILT, AND NOT YET RULED

The shared gaps -- the live shared session, the stale highlight, the undebounced walk, the undefined
whole-word and case fold -- are in `ambre-spec-find.md section 6`. Prose's own:

* **Replace-one has no stated interaction with the current index.** After a replacement the replaced
  text may itself match the query, and nothing says whether the walk can loop on it.
* **Regular-expression search is a switch with no design.** The query carries the flag, no rule
  covers what a pattern may express, what it costs or what a malformed one reports, and the Story
  that would build it is "The later writing set" (`ambre-roadmap.md`).
