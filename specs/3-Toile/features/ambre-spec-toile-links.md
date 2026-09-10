# AMBRE -- FEATURE: LINKS ON AN ELEMENT

**Type:** Vertical (feature)
**Implementation:** built -- the register, the command, the chooser and the three navigations. What
is not built is a way back from a jump, and any mark saying a target is gone.

> A user makes an element clickable: it opens a web address, jumps the camera to a frame on this
> board, or opens another Ambre document. A legend becomes navigation, and a frame becomes a table of
> contents.

Following a link changes nothing in the document. It is the one board gesture that writes nothing.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-062` | `INV` | A link MUST be its own register on the element, not part of its style. | this spec section 3 | `ambre_toile::logic::convergence_tests::a_link_is_set_and_cleared` writes and clears it; the format holds it as its own field beside `style` (`ambre-spec-toile-schema.md section 3`), and `::setting_a_link_touches_nothing_else` holds the other direction | verified |
| `TOILE-109` | `INV` | A link MUST outlive its target: deleting the frame a link points at MUST NOT clear the link. | this spec section 3 | `ambre_toile::logic::convergence_tests::deleting_the_target_frame_leaves_the_link` deletes the frame and reads the register back off the painted element | verified |
| `TOILE-110` | `IO` | The board MUST NOT fetch an address an element carries. | this spec section 4; `ambre-spec-zero-knowledge.md` | structural, and the absence is the proof: `ambre-toile` declares no HTTP client and no `fetch` binding, its follow path returns a `ShellNav` value rather than performing anything, and the one place an address is touched hands it to `window.open`. The ban is a capability an audit holds; no test can observe a request that is never made | verified |

### 1.2 Set or clear a link

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-066` | `POST` | Setting or clearing a link MUST be one undo item. | this spec section 3 | `ambre_toile::logic::convergence_tests::a_link_is_set_and_cleared` asserts one delta per write; the handler makes one `doc.commit()`, which is what one undo item is (`features/ambre-spec-toile-undo.md`). No test walks the undo stack over a link | unverified |
| `TOILE-111` | `FRAME` | Setting or clearing a link MUST write the link register alone: the element's style, geometry and lock MUST NOT change. | this spec section 3 | `ambre_toile::logic::convergence_tests::setting_a_link_touches_nothing_else` reads the painted element before and after and compares its bounds, its fill, its stroke and its lock | verified |

### 1.3 Follow a link

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-112` | `PRE` | Following MUST be asked for by a gesture of its own -- a modified click, the link glyph, or a plain click where editing is off -- so a plain click with the select tool selects instead. | this spec section 4 | `ambre_toile::ui::link::tests::a_plain_click_never_follows_while_editing` holds the rule as a function of the two inputs. What routes the click is the view, which is not host-constructible, so nothing runs the two together | unverified |
| `TOILE-113` | `POST` | Following a link to a frame MUST fit the camera to that frame's box. | this spec section 3 | `ambre_toile::ui::link::tests::each_target_means_one_navigation` holds that a frame target resolves to a jump and to nothing else. The fit itself rides the camera path the zoom-to-selection entry uses, which no test drives | unverified |
| `TOILE-065` | `POST` | A web address MUST be opened by the shell, in a sandboxed tab. | this spec section 4; `ambre-spec-zero-knowledge.md` | `ambre_toile::ui::link::tests::only_the_outbound_halves_reach_the_shell` holds that the board hands the address out rather than acting on it. The tab's own terms -- a fresh target, `noopener,noreferrer` -- are one `window.open` call in the shell, which the host tier cannot run | unverified |
| `TOILE-114` | `POST` | A link to another Ambre document MUST be opened by the shell, never by the board. | this spec section 4 | `ambre_toile::ui::link::tests::only_the_outbound_halves_reach_the_shell` and `::each_target_means_one_navigation` hold that a document target leaves the board as a value. The shell's own open path is the launcher's, which no test drives | unverified |
| `TOILE-063` | `FRAME` | Following a link MUST NOT write to the document. | this spec section 3 | structural, and the types are the proof: following resolves to a `LinkAction`, whose three arms are a camera request, an address and a document reference. The one board command it can produce is `RequestSelectionStats`, which the engine answers without a commit | verified |
| `TOILE-115` | `FRAME` | Following a link MUST NOT enter the undo history: a `Ctrl+Z` after a jump MUST revert the last edit, not the jump. | this spec section 3 | structural, on the row above: the undo manager records commits, a follow makes none, and nothing records where the camera came from. No test types `Ctrl+Z` after a jump | unverified |
| `TOILE-064` | `FAIL` | Following a link to a frame the board no longer holds MUST do nothing, and MUST NOT report an error. | this spec section 3 | structural: a jump asks the board for the frame's bounds, and an id nobody holds answers `None` bounds, on which the fit does nothing and no error is raised. Nothing drives the pair, and section 6 says what the silence costs | unverified |

* **Budget.** none of its own once built: setting a link is one register write, which lands under the
  transform-command row of `ambre-spec-budgets.md section 1`. A jump to a frame is a
  camera change, budgeted by `features/ambre-spec-toile-viewport.md`.
* **Boundary.** the one board feature that names an outbound path. A web address is handed to the
  shell and opened in a sandboxed tab; the board never fetches it, so no request carries document
  content and `ZK-001` is untouched. The address itself is document data, sealed like the rest
  (`TOILE-065`).
* **Verification.** 5 `verified`, 7 `unverified`, 0 `unimplemented`, over four tests and three
  structural readings. The half that lands in the document is held: the register is its own, it
  survives its target's deletion, and setting it moves nothing else -- that last one reads the
  painted element before and after rather than trusting the handler. The half that leaves the
  document is held by its shape instead of by a run: following resolves to a value with three arms,
  and the two outbound ones are handed out rather than acted on, which is what makes the fetch ban
  an audit (`TOILE-110`) rather than a test of a request nobody makes. What stays loose is every
  rule whose subject is the surface -- which click follows, where the camera lands, what the shell
  does with the value -- because the view is not host-constructible and the shell is a browser. Two
  of those (`TOILE-064`, `TOILE-115`) are held only by the absence of code, which is the weakest
  reading in the ladder and the honest one here.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| sets a link on a shape | sets a different link on the same shape | one of the two links, whole | the same target on both screens (`TOILE-062`) | none: the register is one LWW value, and no test holds two peers over it |
| follows a link to a frame | watches | nothing new | Alice's camera on the frame, and Bob's camera where it was (`TOILE-113`, `TOILE-063`) | none: the follow makes no commit, so there is nothing for a two-peer run to compare |
| deletes the target frame | follows the link to it | the link, pointing at an id nobody holds | nothing happens on Bob's click, and the link stays (`TOILE-064`, `TOILE-109`) | `logic::convergence_tests::deleting_the_target_frame_leaves_the_link` holds the half that is a document fact |
| clears the link | follows it at the same moment | the cleared register | nothing happens, and no error, because a follow reads and never writes (`TOILE-063`) | `logic::convergence_tests::a_link_is_set_and_cleared` holds the clear |

---

## 3. MODEL

**Containers.** One optional binary field on any element (`ambre-spec-toile-schema.md section 3`).
Absent means no link, and clearing removes the key rather than writing an empty one (`TOILE-230`).

**The target shape** (`ambre-bus`):

```rust
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub enum LinkTarget {
    Url(String),      // a web address, opened sandboxed by the shell
    Frame(ElementId), // a frame on this board: the camera fits to it
    Document { document_id: DocumentId, doc_type: DocType },
}
```

The document arm carries its type beside its id, where the design carried an id alone: a route
cannot be built without the type, and asking the server for it turns a follow into a round trip that
can fail. An embed's reference is shaped the same way, for the same reason
(`features/ambre-spec-toile-embeds.md`).

**Command** (`ambre-bus`): `SetLink { id, link }`, where an absent link clears the register. One
register write and one undo item, and nothing else on the element moves (`TOILE-066`,
`TOILE-111`). A locked element refuses it like it refuses every other edit.

**Why the link is not part of the style.** The style register is one value written whole, so
folding a link into it would make every restyle of a selection overwrite the links of its members.
A link is navigation, not a visual attribute, which is the same reason the lock flag sits outside it
(`features/ambre-spec-toile-styles.md`).

**Steps, when a link is followed:**

1. read the target from the painted element, which carries the register;
2. for a frame, **ask the board for that frame's box** and fit the camera to the answer
   (`TOILE-113`); a frame nobody holds answers no box, and the fit then does nothing, with no
   error (`TOILE-064`);
3. for a web address, hand it to the shell, which opens it sandboxed (`TOILE-065`);
4. for a document, hand it to the shell too (`TOILE-114`).

The frame is asked for rather than read off the screen. The design had it read from the painted
element, and that is wrong for the case links exist to serve: a link to a frame is how a user reaches
one that is **off screen**, and an off-screen element is not in the culled frame. The board answers
the bounds of an id it holds, which makes the dead-target refusal fall out of the same path instead
of needing a check of its own.

Step 2 sends `RequestSelectionStats`, which the engine answers without a commit; the other two send
nothing at all. So no step writes (`TOILE-063`) and none reaches the undo history (`TOILE-115`).

**Code:**

* `crates/ambre-bus/src/app/toile.rs` -- the target type, the command, the register on the painted
  element;
* `crates/ambre-toile/src/logic/state/elements.rs` -- the one write;
* `crates/ambre-toile/src/ui/link.rs` -- which click follows, which element it means, and what
  following that element resolves to;
* `crates/ambre-app/src/shell/app_root.rs` -- the chooser and the two outbound opens.

---

## 4. SURFACE

What the Story owes, stated so the rows above have a section to cite:

| Trigger | Behaviour |
|---|---|
| the Object menu's link entry, on a single selection | opens the chooser: an address, a frame from the board's own list, or a document (`TOILE-066`) |
| the Object menu's remove-link entry | clears the register, with no dialog |
| `Ctrl/Cmd`-click on a linked element, with the select tool | follows the link (`TOILE-112`) |
| a click on the link badge | follows the link, whatever the modifier |
| a plain click, with the select tool | selects, as always: a link never blocks editing (`TOILE-112`) |
| a plain click, in presentation mode | follows the link, because editing is off (`features/ambre-spec-toile-presentation.md`) |

**Where the chooser lives.** In the shell, not in the style panel the design named. Two of the three
targets are things only the shell can offer: an address it validates where the user can still read
the refusal, and the documents this user may open. The frame list is the deck's, which already holds
every frame on the board -- including the off-screen ones a link exists to reach. The board is
handed a target that is already well formed, which is the same inversion the embed entries take
(`features/ambre-spec-toile-embeds.md`).

**What a linked element looks like.** A small badge outside its top-right corner, painted in screen
space so it keeps its size through the zoom. Always painted, not shown on hover: a link nobody can
see is a link nobody follows, and the painted element carries the register, so the badge costs a
walk of the culled frame and no state.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The link is a register of its own | a field of the style register | the style is written whole, so a link inside it would be erased by any restyle of a selection that included the element |
| Following writes nothing | recording the jump, so a presenter could replay it | a jump is one user's camera, and a camera is per user; writing it would put one user's navigation into the other users' history |
| A dead frame target is inert | clearing the register when the frame is gone | it destroys data on a read, and the target may come back through a late change or an undo |
| A web address is opened by the shell, never fetched | fetching a preview of the address | the board holds document content, so a fetch from the board would put a request next to plaintext and give an attacker a channel. The shell opens a sandboxed tab and the board learns nothing |
| Three target kinds and no more | a general action target | an address, a place on this board and another document are the three navigations a whiteboard needs; anything else is a command, and a command belongs to a menu |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A dead link and an unclicked one are the same event** (`TOILE-064`). A follow the board
  cannot resolve does nothing and reports nothing, so a user cannot tell a target that was deleted
  from a glyph they missed. The register is kept because the frame may come back through a late
  change or an undo, and nothing on the screen says it is waiting.
* **A jump has no way back** (`TOILE-115`). Following writes nothing, so it enters no history:
  `Ctrl+Z` after a jump reverts the last edit and leaves the camera where the link put it. Whether
  a board owes a back gesture is unruled, and the presentation case below is the same hole one
  document wider.
* **A link on a connector is not built.** An arrow can now be selected and carries a style of its
  own (`features/ambre-spec-toile-connectors.md`), so the reason this was deferred is gone; what is
  left is that the register lives on an element map and an arrow is not one. Nothing offers it, and
  the chooser addresses the element selection only.
* **A link is set on one element at a time.** The chooser answers with one target and the view
  applies it to a single selection; a multi-selection is ignored rather than fanned out, because a
  chooser cannot mean "these five all point here" without saying so. Nothing on screen says why the
  entry did nothing.
* **The frame list has no names.** A frame's title is designed and unwritten
  (`features/ambre-spec-toile-frames.md`), so the chooser lists every frame as "Untitled frame".
  On a board with four frames, choosing the right one is guesswork until that lands.
* **Nothing validates an address.** Whether a malformed address is refused at the write or ignored at
  the follow is open, and the shared open-URL model the roadmap names is where it should be settled
  once (`ambre-roadmap.md`, "Links in Trame").
* **A link out of a presentation has no back.** Following a document link during a presentation leaves
  the board, and nothing returns the presenter to the slide they left.
