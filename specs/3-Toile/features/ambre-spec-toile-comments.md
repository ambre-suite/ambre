# AMBRE -- FEATURE: COMMENTS

**Type:** Vertical (feature)
**Implementation:** design-only -- no container, no command, no surface.

> A user pins a remark on an element or on empty board, and a teammate answers it hours later. The
> thread is resolved when the point is settled, and it stays readable afterwards.

Comments are the asynchronous half of collaborating on a board. Cursors and reactions are the
synchronous half, and they vanish; a thread must not.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. Nothing below is built: the rows are the
design, checkable against itself and against the format. One rule of this feature lives with the
machine it belongs to and is not restated: the comment tool drops a thread where it clicks
(`TOILE-UI-042`, `features/ambre-spec-toile-tools.md section 1`).

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TOILE-024` | A comment thread MUST be anchored either to a stable `ElementId` or to a free world point, never to a viewport coordinate. | this spec section 3 | not built -- lands with the "Comments" Story (`ambre-roadmap.md` E4) | unimplemented |
| `TOILE-025` | A pin whose anchor element is deleted MUST go dangling like a connector: it MUST NOT be rendered and MUST NOT be destroyed. | this spec section 3; `features/ambre-spec-toile-connectors.md section 3` | not built -- lands with the "Comments" Story (`ambre-roadmap.md` E4) | unimplemented |
| `TOILE-026` | The `DisplayViewport` MUST carry pins and their badges, and MUST NOT stream thread bodies: those are fetched on demand. | this spec section 3; `features/ambre-spec-toile-engine.md section 3` | not built -- lands with the "Comments" Story (`ambre-roadmap.md` E4) | unimplemented |
| `TOILE-FEAT-011` | Two peers dropping a comment on the same spot MUST mint distinct threads; neither MUST overwrite the other. | this spec section 3 | not built -- lands with the "Comments" Story (`ambre-roadmap.md` E4) | unimplemented |
| `TOILE-FEAT-012` | Message order in a thread MUST be the `LoroMovableList` order, never the `ts` register: clocks are untrusted. | this spec section 3 | not built -- lands with the "Comments" Story (`ambre-roadmap.md` E4) | unimplemented |
| `TOILE-FEAT-013` | A message body MUST be a `LoroText`, so a self-edit converges character-wise rather than by whole-string LWW. | this spec section 3 | not built -- lands with the "Comments" Story (`ambre-roadmap.md` E4) | unimplemented |
| `TOILE-FEAT-014` | A message author MUST be the canonical `UserId` resolved by `ambre-sync`, never a `PeerID`. | this spec section 3; `features/ambre-spec-toile-reactions.md section 5` | not built -- lands with the "Comments" Story (`ambre-roadmap.md` E4) | unimplemented |
| `TOILE-FEAT-015` | Resolving and reopening a thread MUST be idempotent and convergent, and MUST NOT destroy the thread or its messages. | this spec section 3 | not built -- lands with the "Comments" Story (`ambre-roadmap.md` E4) | unimplemented |
| `TOILE-FEAT-016` | A thread anchored to a deleted element MUST stay valid and simply not render; it MUST NOT be auto-deleted. | this spec section 3 | not built -- lands with the "Comments" Story (`ambre-roadmap.md` E4) | unimplemented |
| `TOILE-FEAT-017` | A comment command MUST NOT enter the undo stack: `Ctrl+Z` on the board MUST NOT retract a teammate-visible message. | this spec section 3; `features/ambre-spec-toile-undo.md section 1` | not built -- lands with the "Comments" Story (`ambre-roadmap.md` E4) | unimplemented |
| `TOILE-UI-022` | A comment pin MUST be painted in screen space and MUST NOT be occluded by board content, whatever the z-order. | this spec section 4; `features/ambre-spec-toile-engine.md section 3` | not built -- lands with the "Comments" Story (`ambre-roadmap.md` E4) | unimplemented |
| `TOILE-UI-023` | The comments panel MUST hold no authoritative state: it re-reads a thread on `CommentsUpdated` and emits commands for every change. | this spec section 4 | not built -- lands with the "Comments" Story (`ambre-roadmap.md` E4) | unimplemented |
| `TOILE-UI-024` | A pin badge MUST come from `DisplayViewport.comments` and MUST NOT require reading the panel or the thread bodies. | this spec section 4 | not built -- lands with the "Comments" Story (`ambre-roadmap.md` E4) | unimplemented |
| `TOILE-UI-082` | A click on a pin MUST open that pin's thread, and MUST NOT select the element under it. | this spec section 4 | not built -- lands with the "Comments" Story (`ambre-roadmap.md` E4) | unimplemented |

* **Budget.** none of its own yet. A pin is one more node the viewport carries, so it lands under the
  visible-set row of `ambre-spec-budgets.md section 1`; the panel reads a thread on demand, so
  no figure covers the board-wide list.
* **Boundary.** none of its own: a thread is written to the document, and `ZK-001` seals every payload
  before it leaves the client. A message body is document content like any other, so the relay reads
  none of it.
* **Verification.** 0 `verified`, 0 `unverified`, 14 `unimplemented`. Nothing here exists: no
  container is written, no command name appears in the workspace, and the display viewport carries no
  pin field. Three sections of three specs described the feature as designed and none said it was
  unbuilt until the rows did.

---

## 2. WHAT EACH USER SEES

The rows describe what the design owes, and each names the rule the Story must bring a test for.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| drops a comment on a note | drops one on the same note | two threads, distinct | both pins, neither overwritten (`TOILE-FEAT-011`) | none -- unbuilt |
| replies to a thread | replies to it at the same moment | both replies, in list order | the same two replies in the same order on both screens (`TOILE-FEAT-012`) | none -- unbuilt |
| edits her own message | reads it | one body, merged character-wise | Alice's correction, without losing Bob's read position (`TOILE-FEAT-013`) | none -- unbuilt |
| resolves a thread | resolves it at the same moment | one resolved flag | one resolved thread, its messages intact (`TOILE-FEAT-015`) | none -- unbuilt |
| deletes the element a thread is pinned to | reads that thread in the panel | the thread, its anchor pointing at nothing | no pin on the board, and the thread still in the list (`TOILE-FEAT-016`, `TOILE-025`) | none -- unbuilt |
| posts a message, then presses `Ctrl+Z` | reads the message | the message | the message on both screens, and Alice's last drawing action reverted (`TOILE-FEAT-017`) | none -- unbuilt |

---

## 3. MODEL

**Containers.** The `comments` map, one entry per thread under a minted thread id, each holding its
anchor, its resolved flag and its ordered messages, every message body a text container
(`ambre-spec-toile-schema.md section 3`). All five rows of the format are declared `design`.

**The anchor** (`ambre-bus`, to be added):

```rust
pub enum CommentAnchor {
    Element(ElementId),        // the pin tracks the element as it moves
    Point { x: f64, y: f64 },  // a free world point, for a remark on empty board
}
```

**Commands** (`ambre-bus`, to be added): `CreateComment { anchor, body }`, `ReplyComment
{ thread, body }`, `EditComment { thread, message, body }` and `ResolveThread { thread, resolved }`.
None enters the history (`TOILE-FEAT-017`).

**Events** (`ambre-bus`, to be added): `CommentsUpdated(thread)` and `CommentThreadOpened(thread)`.

**Steps, when a thread is created:**

1. mint a thread id, so two peers on the same spot cannot collide (`TOILE-FEAT-011`);
2. write the anchor, the resolved flag and the first message under that id, seeding the message list
   and the body container in the same write, so no container is created lazily
   (`TOILE-029`);
3. commit outside the history scope, and emit the two events.

**Steps, when the viewport is derived:**

1. take each thread whose anchor resolves: an element in the visible set, or a point inside the
   camera;
2. carry its pin position, its message count and its resolved flag, and no body (`TOILE-026`);
3. skip a thread whose anchor element is gone, without touching it (`TOILE-025`).

**Why the author is a `UserId`.** A message must carry the person, not the session: the same argument
as a reaction, and it is written once in
`features/ambre-spec-toile-reactions.md section 5` (`TOILE-FEAT-014`).

**Why the timestamp orders nothing.** A clock belongs to the machine that wrote it, and a board is
open on machines nobody controls. The list order is the order the CRDT converges on, and the
timestamp is shown to the reader and used for nothing else (`TOILE-FEAT-012`).

**Code:** none. No container, no command, no event and no field.

---

## 4. SURFACE

What the Story owes, stated so the rows above have a section to cite:

| Trigger | Behaviour |
|---|---|
| the comment tool, then a click on the board | drops a thread at that point or on the element under it, and opens the composer (`TOILE-UI-042`) |
| a click on a pin | opens that thread in the panel (`TOILE-UI-082`) |
| the comments entry of the view menu | opens the panel on the whole board, filterable by open or resolved |
| a click on a thread in the board-wide list | recentres the camera on that thread's anchor |
| the resolve toggle of an open thread | resolves it, or reopens it (`TOILE-FEAT-015`) |

**The pins.** A pin paints in screen space above all content, so it keeps one size through the zoom
and no element hides it (`TOILE-UI-022`). It carries its message count and its resolved state, both
read from the derived viewport and never from the panel (`TOILE-UI-024`). The pick tests the pins
before the elements, so a pin over a note takes the click (`TOILE-UI-082`,
`features/ambre-spec-toile-selection.md section 3`).

**The panel** is an `ambre-chrome` side panel over the command channel, the same plumbing as the find
panel (`features/ambre-spec-toile-find.md section 4`). It holds no authoritative state: it re-reads a
thread when the board says the thread changed (`TOILE-UI-023`).

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| A thread is its own entry under a minted id | two peers commenting on the same spot must both be heard, and a minted id makes the collision unrepresentable rather than resolved | one thread per element, keyed by the element id |
| The anchor is an element or a point, never a screen position | a pin must follow the note it is about, through every pan, zoom and move. A viewport coordinate is meaningless on the second screen | a pin at a place in the current view |
| A dangling pin is kept and not painted | the element may come back through an undo or a late change, and deleting the discussion because the drawing changed destroys the record of why it changed | deleting the thread with its anchor |
| A message body is a text container | two people editing the same message is rare, and a whole-string register loses one of them silently when it happens | one register per message |
| Order is the list order, not the timestamp | a clock is untrusted and unsynchronized, so ordering by it lets a wrong clock rewrite a conversation | sorting by the author's timestamp |
| A comment is outside the history | `Ctrl+Z` reverts what the user drew. Withdrawing a message a teammate has already read is not an undo, it is a deletion nobody asked for | commenting like any other write |
| Mentions and notifications are out | a mention needs a notification channel, which is Murmure's subject. Routing between the two applications does not exist | a mention that mails or notifies |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The whole feature is unbuilt.** Three specs held its design -- the anchoring, the thread schema
  and the panel -- and one of them listed a mandatory test belt for it. No command name of it appears
  in the workspace.
* **The precedent the body rule cites does not exist** (`TOILE-FEAT-013`). A sticky body is a
  whole-string register today, not a text container, so building comments on "the sticky pattern"
  would reproduce the defect rather than the pattern (`TOILE-030`,
  `ambre-spec-toile-text-editing.md`).
* **Nothing says who may comment.** Whether a viewer may open a thread, and whether a member may
  resolve someone else's, is a permission question no row covers
  (`ambre-spec-membership.md`).
* **Nothing says what a resolved thread costs.** A board of a thousand resolved threads carries them
  all in the document and in every export, and no rule prunes or archives them.
* **A comment reaches no export.** Neither the file round trip nor the image and page exports name a
  thread, so a resolved discussion exists on the board alone.
* **Mentions are deferred.** They need a notification path to Murmure, which does not exist
  (`README.md`, "Applications").
