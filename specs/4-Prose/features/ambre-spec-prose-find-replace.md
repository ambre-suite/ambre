# AMBRE -- FEATURE: FIND AND REPLACE

**Type:** Vertical (feature)
**Implementation:** design-only

> Searching the document, walking the matches, and replacing one or all of them. Searching changes
> nothing and is one reader's own; replacing is an edit like any other.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `PROSE-FEAT-015` | A replace-all MUST be a single undo item. | this spec section 3; `features/ambre-spec-prose-undo.md` | not built -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-016` | Finding MUST be local view state and MUST NOT change the document. | this spec section 3 | not built -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-033` | A search MUST cover the body blocks in document order and nothing else: comment, footnote, header and footer text MUST NOT be matched. | this spec section 3 | not built -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-034` | A match MUST lie inside one block; a query spanning a block boundary MUST NOT match. | this spec section 3 | not built -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-035` | Matches MUST NOT overlap: after one match, the search MUST resume past its end. | this spec section 3 | not built -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-036` | A replace-all MUST apply its replacements from the end of the document backwards, so that no replacement invalidates a later position. | this spec section 3 | not built -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-037` | The current match MUST be brought on screen: one that falls outside the window MUST be scrolled into view. | this spec section 3 | not built -- lands with the "Headers, footers, find and replace" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the search over the document and the replace-all, two rows of
  `ambre-spec-budgets.md section 1`.
* **Boundary.** none of its own: a search reads the document in the Logic thread and writes
  nothing, and a replacement is an ordinary document write sealed by `ZK-001`.
* **Verification.** 7 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`.
  `PROSE-FEAT-035` and `PROSE-FEAT-036` are the two with an oracle: matching is a pure function of
  one string and one query, and a replace-all is idempotent -- after it, the query matches
  nothing, which is the property that fails when a replacement shifted a later position.

---

## 2. WHAT EACH USER SEES

Two peers, one search. No test resolves yet: the last column names the row each row rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| searches for a word | -- | nothing: no write | her own matches highlighted; Bob sees nothing of her search | none -- `PROSE-FEAT-016` |
| walks to the next match, off screen | -- | nothing | the page scrolled to the match | none -- `PROSE-FEAT-037` |
| replaces all forty matches | types in the middle of the document | forty replacements and Bob's text | the replacements made, Bob's sentence untouched | none -- `PROSE-FEAT-036` |
| presses undo after that replace-all | -- | the forty replacements taken back | the original words, in one step | none -- `PROSE-FEAT-015` |
| searches for a word that only exists in a comment | -- | nothing | no match, and the comment untouched | none -- `PROSE-FEAT-033` |
| searches for a phrase split across two paragraphs | -- | nothing | no match | none -- `PROSE-FEAT-034` |

---

## 3. MODEL

**Containers.** None. The query, the matches and the current index are session state on the Logic
thread, and nothing about a search is persisted (`PROSE-FEAT-016`).

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

**What is searched.** The body blocks of the movable tree, in document order, and nothing else.
Comment and reply bodies, suggestion metadata, footnote bodies and the text of headers and footers
live in their own stores, each with its own way of being reached, and they are out of scope by
decision (`PROSE-FEAT-033`).

**Why a match stays inside a block.** Blocks are not concatenated for the search, so a match is
one range inside one block and needs no second identifier to be addressed
(`PROSE-FEAT-034`). A query that spans a paragraph break therefore finds nothing.

**Why replacements run backwards.** Each replacement shifts every position after it in its block.
Applying them from the end of the document towards the start leaves every position not yet applied
still valid, so the whole set can be computed once and sent as one run (`PROSE-FEAT-036`), which
is also what makes it one undo item (`PROSE-FEAT-015`).

**The commands and the events** (`ambre-bus`, declared in `features/ambre-spec-prose-engine.md section 3`):

| Shape | Direction | Carries |
|---|---|---|
| `FindStart { query }` | view to engine | the query; the engine answers with every match |
| `FindNext`, `FindPrev` | view to engine | nothing: the engine moves its own index |
| `FindClose` | view to engine | nothing: the session ends |
| `ReplaceOne { replacement }` | view to engine | the text replacing the current match |
| `ReplaceAll { replacement }` | view to engine | the text replacing every match |
| `FindResults { matches, current }` | engine to view | the matches to highlight |
| `FindUpdated { current }` | engine to view | the new index alone |

**Revealing the current match.** On both events the view highlights and, when the current match
lies outside the window, scrolls the least that brings it into view (`PROSE-FEAT-037`) -- the same
reveal every caret move uses (`features/ambre-spec-prose-selection.md section 2`). Trame reveals
its match cell the same way (`2-Trame/features/ambre-spec-trame-find-replace.md`).

**Code:** none. `crates/ambre-prose/src/lib.rs` is seven lines.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the find chord, or the menu entry | opens the search, with the selection as the initial query |
| typing in the search field | restarts the search and highlights every match |
| Enter, Shift with Enter | walks to the next and the previous match, revealing it |
| the case and whole-word switches | restart the search |
| replace | replaces the current match and walks to the next |
| replace all | replaces every match, as one undo item |
| Escape, or closing the panel | ends the session and drops the highlights |

The search panel is chrome (`ambre-spec-chrome.md`). One match session with a live
cursor, in every application, is the "One find and replace" Story.

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| A search writes nothing | a search is one reader's attention, and two readers searching for different words in one document must not see each other's highlights, nor undo each other's searches. Keeping it out of the document makes both free | recording the session in the document, which shares one reader's search with every peer |
| The search covers body text only | comments, footnotes and headers are separate stores with their own navigation, and folding them into one result list makes a match the user cannot see without leaving the page they are on. The editors this one is measured against draw the same line | searching every store, which returns matches with no place to show them |
| A match does not cross a block boundary | concatenating the document to search it requires a second position space and a mapping back to blocks for every match, and the mapping is what breaks under a concurrent edit. Keeping matches inside a block makes a match one identifier and one range | concatenating the blocks, which buys a rare match and costs a position mapping |
| The search runs in the Logic thread | the document lives there, and it is the only side that can read every block. Searching in the view would need a copy of the text on the UI thread, which is the one thing `PROSE-UI-001` forbids, and the copy would be stale by a keystroke besides | searching in the view, which requires a second model of the document |
| Matches do not overlap | overlapping matches make replace-all ambiguous -- replacing the first destroys the second -- and no user expects two hits on the same characters | reporting every occurrence including overlaps, which makes replace-all's outcome depend on the order |
| Replacements are applied backwards | the alternative is to recompute the positions after each replacement, which is a re-scan per match and turns replace-all into a loop over the document. Running backwards keeps one scan and makes the whole set sendable at once | replacing forwards and re-searching, which is quadratic and re-enters text already replaced |
| Replace-all is one undo item | a user who replaces forty matches and regrets it presses undo once, not forty times. It is also the only grouping rule this application writes down today | one undo item per replacement, which makes an undo of the gesture forty keystrokes |
| The current match is revealed | walking matches with the keyboard is the whole gesture, and a match off screen makes the next press feel like nothing happened. The reveal is the caret's own, so there is one scroll rule in the application | highlighting without scrolling, which is the version where a search of a long document appears to do nothing |

---

## 6. NOT BUILT, AND NOT YET RULED

* **What a whole word is, is not defined**, and neither is what the case-insensitive comparison
  is: an ASCII fold, a Unicode one, or a locale-aware one. The three disagree on ordinary text.
* **Nothing rules on a document edited during a search.** The matches are a snapshot, and no rule
  says whether an edit -- local or remote -- re-runs the search, invalidates the list, or leaves
  the index pointing at a range that has moved.
* **Nothing rules on the cost of a search.** Every search is a full walk of the document, and no
  rule caps the match count or says what happens on a query matching a very large number of them.
* **Replace-one has no stated interaction with the current index.** After a replacement the
  replaced text may itself match the query, and nothing says whether the walk can loop on it.
* **Regular-expression search is a switch with no design.** The query carries the flag, no rule
  covers what a pattern may express, what it costs or what a malformed one reports, and the Story
  that would build it is "The later writing set" (`ambre-roadmap.md`).
