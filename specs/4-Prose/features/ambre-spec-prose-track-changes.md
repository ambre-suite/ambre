# AMBRE -- FEATURE: TRACK CHANGES

**Type:** Vertical (feature)
**Implementation:** design-only

> A reviewer edits without changing the document: what they type is marked as a suggested
> insertion, what they delete is marked as a suggested deletion, and the author accepts or rejects
> each change as one unit. A rejected suggestion leaves nothing behind.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The two Stories below are the two halves
of this feature: the suggestion converges under one, and it is painted under the other.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `PROSE-FEAT-004` | A rejected suggestion MUST be removed from the CRDT and MUST leave no trace in the final document. | this spec section 3 | not built -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-010` | Accepting a suggestion MUST apply exactly the recorded change; it MUST NOT re-derive it from the current text. | this spec section 3 | not built -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-011` | Track changes MUST converge under concurrent accept and reject of the same suggestion: one outcome, never a half-applied edit. | this spec section 3 | not built -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-082` | A suggestion's identity MUST be stable for the document's lifetime: a partial accept, a reject or a remote delta MUST NOT change it. | this spec section 3 | not built -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-083` | Accept and reject MUST act on every span carrying the identity, whatever the number of blocks it crosses. | this spec section 3 | not built -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-084` | Accept and reject MUST be idempotent. | this spec section 3 | not built -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-085` | Consecutive edits by one peer MUST share an identity while they stay adjacent and inside the merge window. | this spec section 3 | not built -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-086` | A caret move to a non-adjacent position MUST start a new identity. | this spec section 3 | not built -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-087` | Deleting text covered by the caller's own pending insertion MUST remove that text outright. | this spec section 3 | not built -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-088` | Deleting any other text while suggesting MUST mark it, and MUST NOT remove it. | this spec section 3 | not built -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-089` | Turning suggestion mode off MUST leave every pending suggestion untouched. | this spec section 3 | not built -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-051` | A suggested insertion and a suggested deletion MUST be distinguishable in the text itself, not by an affordance the reader has to open. | this spec section 4 | not built -- lands with the "Collaborative cursors and annotations" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-052` | The accept and reject affordance MUST be DOM chrome, not a canvas overlay. | this spec section 4; `ambre-spec-ui.md section 2` | not built -- lands with the "Collaborative cursors and annotations" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** two rows of `ambre-spec-budgets.md section 1`: the command interception under
  100 us, because it sits on the path of every mutation while suggesting is on, and accept-all over
  a hundred suggestions under 5 ms.
* **Boundary.** none of its own: a suggestion is a mark and an annotation entry in the document, and
  `ZK-001` seals every payload before it leaves the client.
* **Verification.** 13 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`.
  `PROSE-FEAT-010` and `PROSE-FEAT-011` are what the planned property covers -- any interleaving of
  insert and delete suggestions, then an accept-all, leaves a self-consistent document -- and
  `PROSE-FEAT-011` is the row two peers are needed for. `PROSE-FEAT-085` is the row with no
  mechanism in sight, because the window it names is undefined: section 6.

---

## 2. WHAT EACH USER SEES

Two peers over one set of suggestions. No test resolves yet: the last column names the row each row
rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| suggests a sentence | -- | the sentence, marked as an insertion | the sentence in the insertion style on both screens | none -- `PROSE-FEAT-088` |
| suggests deleting a paragraph | -- | the paragraph, marked as a deletion | the paragraph struck through, and still there | none -- `PROSE-FEAT-088` |
| accepts a suggestion | rejects the same one | one outcome, whichever converged | the same document on both screens, never half of the change | none -- `PROSE-FEAT-011` |
| rejects her own suggested insertion | -- | nothing of it | the text gone on both screens, and no mark left | none -- `PROSE-FEAT-004` |
| suggests across a paragraph break | accepts it | both halves applied | one change, not two, on both screens | none -- `PROSE-FEAT-083` |
| accepts a suggestion | accepts the same one | one applied change | the same document on both screens | none -- `PROSE-FEAT-084` |
| deletes text she suggested a moment ago | -- | nothing of that text | the text simply gone, with no strike-through | none -- `PROSE-FEAT-087` |
| turns suggestion mode off with changes pending | -- | every pending suggestion | the suggestions still marked, still acceptable | none -- `PROSE-FEAT-089` |

---

## 3. MODEL

**Containers.** Two inline marks carry the suggestion's identity on the text -- one for a suggested
insertion, one for a suggested deletion -- and one annotation entry carries the kind, the author,
the peer and the stamp. Both are declared in `ambre-spec-prose-schema.md section 3`.

**The text is never two documents.** A suggestion is the real text plus a mark, so a suggested
insertion is already in the block and a suggested deletion is still in it. That is what makes the
four outcomes cheap, and each one is idempotent (`PROSE-FEAT-084`) and reads the record rather than
the current text (`PROSE-FEAT-010`).

| | Accept | Reject |
|---|---|---|
| a suggested insertion | strip the mark, the text stays | remove the text, and the mark with it |
| a suggested deletion | remove the covered text | strip the mark, the text stays |

**Grouping, and what a unit is.** An identity groups the marks that accept and reject together. The
grouping is **per authoring burst**: the engine keeps the current identity per peer and reuses it
while consecutive edits from that peer stay adjacent to the group's span and fall inside a merge
window (`PROSE-FEAT-085`). A caret move to a non-adjacent position starts a new one
(`PROSE-FEAT-086`), and so does an idle gap or a switch between inserting and deleting at disjoint
ranges. The unit the reviewer accepts is then "the sentence I just typed", rather than one character
or a whole session.

**A delete is classified before it is applied.** While suggesting, a delete range is cut into
maximal segments by what covers each one, and the two kinds are treated in opposite ways:

| Segment | Treatment |
|---|---|
| covered by a pending insertion this peer owns | the text is removed outright (`PROSE-FEAT-087`) |
| original text, or a suggestion any other peer owns | the text is marked as a suggested deletion (`PROSE-FEAT-088`) |

Ownership is read from the annotation entry the mark points at, so a foreign insertion nested inside
this peer's own is split out correctly rather than swallowed.

**One suggestion, several spans.** A change that crossed a block boundary is stored as several
marked spans sharing one identity, and accept and reject collect all of them and act atomically
(`PROSE-FEAT-083`).

**Turning the mode off decides nothing about what is pending.** The flag stops *new* edits from
being intercepted, and nothing else (`PROSE-FEAT-089`).

**The commands** (`ambre-bus`, declared in `features/ambre-spec-prose-engine.md section 3`):

| Shape | Behaviour |
|---|---|
| `EnableTrackChanges { enabled }` | intercepts new edits, or stops intercepting them |
| `AcceptSuggestion { suggestion_id }` | applies every span of that identity |
| `RejectSuggestion { suggestion_id }` | cancels every span of that identity |
| `AcceptAllSuggestions` | applies every pending suggestion |
| `RejectAllSuggestions` | cancels every pending suggestion |

**The three queries the engine answers**, each a pure read of the marks and the entries: the
segments a delete range cuts into, every span carrying one identity, and the set of identities
currently pending.

**Code:** none. `crates/ambre-prose/src/lib.rs` is seven lines.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the suggestion-mode control | new edits become suggestions, or stop being them |
| typing while suggesting | a suggested insertion at the caret |
| deleting while suggesting | a suggested deletion, or a plain delete over this peer's own pending insertion |
| a hover over a suggestion | the accept and reject affordance, on that suggestion |
| the accept control | that suggestion applied, whatever its span count |
| the reject control | that suggestion cancelled, leaving nothing |
| the accept-all and reject-all controls | every pending suggestion at once |

The two states are read in the text itself and not from an affordance (`PROSE-UI-051`): a suggested
insertion is underlined on a light ground, a suggested deletion is struck through on a light ground,
both in theme colours the palette owns
(`features/ambre-spec-prose-page-rendering.md section 3`). Those are canvas content, painted in the
page pass. The accept and reject affordance is a DOM Leptos popover, transform-synced to the hovered
suggestion through a signal the view raises (`PROSE-UI-052`).

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| A suggestion is a mark on the real text | the text a reviewer types has to merge with everything else being typed, so it belongs in the same sequence as the rest. A mark on it costs one attribute per span, and accept and reject become a strip or a delete rather than a replay | a fork of the document per reviewer, whose memory is the document size times the number of suggestions, and whose merge back is a second convergence problem |
| Two marks, not one with a kind | an insertion and a deletion are opposite operations on the same range, and a reader has to tell them apart before doing anything. Two keys make an overlap of the two representable, which one key with a value cannot | one mark carrying the kind in its value, which cannot hold a suggested deletion of a suggested insertion |
| The unit is an authoring burst | one character is too small to judge and a session is too large to reject, and the reviewer thinks in sentences. Adjacency plus a time window reproduces that boundary with no gesture to learn (`PROSE-FEAT-085`) | one identity per keystroke, which makes a sentence forty decisions, or one per session, which makes a typo and a rewrite one decision |
| A delete is classified by ownership | deleting text that has not been accepted by anyone yet is a reviewer correcting themselves, not a change anyone needs to approve. Marking it would create a suggested deletion of a suggested insertion, which is a pair with no meaning (`PROSE-FEAT-087`) | marking every delete, which leaves the reviewer's own typos on the screen until the author rules on them |
| Accept and reject act on the whole identity | a change that crossed a paragraph break is one change to the person who made it and to the person judging it. Acting per span would put a document in a state where half a sentence moved (`PROSE-FEAT-083`) | acting per span, which makes the block count visible to the reviewer and lets an accept stop halfway |
| Both are idempotent | two peers may accept the same suggestion at the same moment, and a delta may arrive twice. Idempotence is what makes those cases converge with no ordering rule at all (`PROSE-FEAT-084`, `PROSE-FEAT-011`) | a guard that refuses the second accept, which needs the two peers to agree on which one was first |
| A rejected suggestion leaves nothing | a rejected change that stays visible as a tombstone means the document nobody approved is still readable in the file, which is the opposite of what rejecting is for (`PROSE-FEAT-004`) | tombstoning the rejection, which keeps the reviewer's text in the document for good |
| Turning the mode off resolves nothing | the flag is about what happens next, and the pending suggestions are work someone did. Silently accepting them would apply changes nobody approved; silently dropping them would lose the review (`PROSE-FEAT-089`) | flushing the pending set on the flag, which makes one toggle either an approval or a deletion of everyone's review |
| The styles are in the text, the buttons are in the DOM | a reader scanning a page must see which words are proposed without hovering anything, so the two states are typographic. The buttons are two controls on one hovered span, which the browser positions, focuses and labels for free (`PROSE-UI-051`, `PROSE-UI-052`) | a gutter bar per changed line, which says a line changed and never which words, or canvas buttons, which reimplement focus and labelling |

---

## 6. NOT BUILT, AND NOT YET RULED

* **No gesture enters suggestion mode.** `EnableTrackChanges` is declared, and no surface sends it:
  neither chrome surface names the mode (`features/ambre-spec-prose-toolbar.md section 4`,
  `features/ambre-spec-prose-slash-command.md section 4`). The feature is unreachable, and every
  rule above is about what happens once it is on.
* **The merge window is set from an interval no document defines.** The grouping window is stated as
  two seconds, "aligned with the undo merge interval", and
  `features/ambre-spec-prose-undo.md section 4` records that the undo interval is undefined. One
  undefined number is the source of two.
* **The engine names two module paths for one feature.** The interception and the accept and reject
  entry points were written under `features/track_changes.rs`, and the three queries beside them
  under `features/suggest.rs`. Nothing says which is the feature's home, and the crate exists in
  neither.
* **The view type is named and never declared.** `features/ambre-spec-prose-engine.md section 3`
  gives the view a `Vec<UiSuggestion>` field, and no document says what a `UiSuggestion` holds. The
  comments counterpart is declared; this one never was.
* **The annotation entry discriminates its kind by a field of two types.** A suggestion's kind is a
  small integer and a footnote's is a string in the same map, which is `PROSE-031`
  (`ambre-spec-prose-schema.md section 1`). The query that lists pending suggestions is specified as
  a filter over that field, so this feature is the one that pays for the defect.
* **Nothing rules on suggestion mode against undo.** Undo reverts the local peer's operations
  (`ambre-spec-undo.md`, `UNDO-001`), and an interception turns one typed operation into a marked
  one. Whether undo
  reverts the mark, the text or both is unstated.
* **Nothing rules on who may accept.** Any collaborator may accept or reject any suggestion,
  including their own, and no rule says whether that is intended.
