# AMBRE -- FEATURE: LINKS ON AN ELEMENT

**Type:** Vertical (feature)
**Implementation:** design-only -- no command, no field, no surface.

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
| `TOILE-062` | `INV` | A link MUST be its own register on the element, not part of its style. | this spec section 3 | not built -- lands with the "Hyperlinks and go-to-frame" Story (`ambre-roadmap.md` E4) | unimplemented |
| `TOILE-109` | `INV` | A link MUST outlive its target: deleting the frame a link points at MUST NOT clear the link. | this spec section 3 | not built -- lands with the "Hyperlinks and go-to-frame" Story (`ambre-roadmap.md` E4); it is the half `TOILE-064` was carrying, and the two are falsifiable apart -- one is what a click does, the other is what a deletion elsewhere on the board may not do to a register it never names | unimplemented |
| `TOILE-110` | `IO` | The board MUST NOT fetch an address an element carries. | this spec section 4; `ambre-spec-zero-knowledge.md` | not built -- lands with the "Hyperlinks and go-to-frame" Story (`ambre-roadmap.md` E4); it is the half `TOILE-065` was carrying, and it is not held by the surface that opens the tab. A capability ban is held by the absence of a fetch path, so the Story owes an audit of the crate and not a test of the click | unimplemented |

### 1.2 Set or clear a link

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-066` | `POST` | Setting or clearing a link MUST be one undo item. | this spec section 3 | not built -- lands with the "Hyperlinks and go-to-frame" Story (`ambre-roadmap.md` E4) | unimplemented |
| `TOILE-111` | `FRAME` | Setting or clearing a link MUST write the link register alone: the element's style, geometry and lock MUST NOT change. | this spec section 3 | not built -- lands with the "Hyperlinks and go-to-frame" Story (`ambre-roadmap.md` E4); it is the half `TOILE-066` was carrying, and it is what `TOILE-062` states of the schema and nothing stated of the gesture -- a register of its own is worth exactly what the write leaves alone | unimplemented |

### 1.3 Follow a link

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-112` | `PRE` | Following MUST be asked for by a gesture of its own -- a modified click, the link glyph, or a plain click where editing is off -- so a plain click with the select tool selects instead. | this spec section 4 | not built -- lands with the "Hyperlinks and go-to-frame" Story (`ambre-roadmap.md` E4); unwritten until now, and nothing can check it from inside the following: the caller is the surface that routes the click, and a link that swallowed a plain click would make a linked element uneditable with no refusal to observe | unimplemented |
| `TOILE-113` | `POST` | Following a link to a frame MUST fit the camera to that frame's box. | this spec section 3 | not built -- lands with the "Hyperlinks and go-to-frame" Story (`ambre-roadmap.md` E4); unwritten until now: section 3 carries the step and no row stated what the gesture owes when it succeeds | unimplemented |
| `TOILE-065` | `POST` | A web address MUST be opened by the shell, in a sandboxed tab. | this spec section 4; `ambre-spec-zero-knowledge.md` | not built -- lands with the "Hyperlinks and go-to-frame" Story (`ambre-roadmap.md` E4) | unimplemented |
| `TOILE-114` | `POST` | A link to another Ambre document MUST be opened by the shell, never by the board. | this spec section 4 | not built -- lands with the "Hyperlinks and go-to-frame" Story (`ambre-roadmap.md` E4); unwritten until now: the target type carries three kinds and only two had a rule, so the third outbound path is the one the Boundary bullet did not name | unimplemented |
| `TOILE-063` | `FRAME` | Following a link MUST NOT write to the document. | this spec section 3 | not built -- lands with the "Hyperlinks and go-to-frame" Story (`ambre-roadmap.md` E4) | unimplemented |
| `TOILE-115` | `FRAME` | Following a link MUST NOT enter the undo history: a `Ctrl+Z` after a jump MUST revert the last edit, not the jump. | this spec section 3 | not built -- lands with the "Hyperlinks and go-to-frame" Story (`ambre-roadmap.md` E4); it is the half `TOILE-063` was carrying, and the two break apart -- a follow recording where the camera came from would write nothing and still push an item, which is the shape a back gesture would take -- section 6 | unimplemented |
| `TOILE-064` | `FAIL` | Following a link to a frame the board no longer holds MUST do nothing, and MUST NOT report an error. | this spec section 3 | not built -- lands with the "Hyperlinks and go-to-frame" Story (`ambre-roadmap.md` E4); the surviving half of the row, now the refusal alone -- section 6 on what a silent refusal costs the user | unimplemented |

* **Budget.** none of its own once built: setting a link is one register write, which lands under the
  transform-command row of `ambre-spec-budgets.md section 1`. A jump to a frame is a
  camera change, budgeted by `features/ambre-spec-toile-viewport.md`.
* **Boundary.** the one board feature that names an outbound path. A web address is handed to the
  shell and opened in a sandboxed tab; the board never fetches it, so no request carries document
  content and `ZK-001` is untouched. The address itself is document data, sealed like the rest
  (`TOILE-065`).
* **Verification.** 0 `verified`, 0 `unverified`, 12 `unimplemented`. Nothing here exists: the
  field is `design` in the format, the command appears nowhere in the workspace, and no surface can
  set or follow a link. The rows are the design, checkable against itself, and five rows checked
  against themselves did not hold. `TOILE-065` banned a fetch and named a shell in one verdict, and
  the ban is a capability an audit holds rather than a click (`TOILE-110`). `TOILE-064` refused a
  dead target and preserved a register in one row, and a deletion elsewhere on the board is what
  breaks the second (`TOILE-109`). `TOILE-066` and `TOILE-063` each carried a write rule and an undo
  rule (`TOILE-111`, `TOILE-115`). And the design owed three rules it never wrote: what the camera
  does on a successful jump (`TOILE-113`), what a document target does (`TOILE-114`), and the plain
  click a link may not swallow (`TOILE-112`).

---

## 2. WHAT EACH USER SEES

The rows describe what the design owes, and each names the test the Story must bring.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| sets a link on a shape | sets a different link on the same shape | one of the two links, whole | the same target on both screens (`TOILE-062`) | none -- unbuilt |
| follows a link to a frame | watches | nothing new | Alice's camera on the frame, and Bob's camera where it was (`TOILE-113`, `TOILE-063`) | none -- unbuilt |
| deletes the target frame | follows the link to it | the link, pointing at an id nobody holds | nothing happens on Bob's click, and the link stays (`TOILE-064`, `TOILE-109`) | none -- unbuilt |
| clears the link | follows it at the same moment | the cleared register | nothing happens, and no error, because a follow reads and never writes (`TOILE-063`) | none -- unbuilt |

---

## 3. MODEL

**Containers.** One optional binary field on any element, declared `design` in
`ambre-spec-toile-schema.md section 3`. Absent means no link.

**The target shape** (`ambre-bus`, to be added):

```rust
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub enum LinkTarget {
    Url(String),      // a web address, opened sandboxed by the shell
    Frame(ElementId), // a frame on this board: the camera flies to it
    Document(Uuid),   // another Ambre document, opened by the shell
}
```

**Command** (`ambre-bus`, to be added): `SetLink { id, link }`, where an absent link clears the
register. One register write and one undo item, and nothing else on the element moves
(`TOILE-066`, `TOILE-111`).

**Why the link is not part of the style.** The style register is one value written whole, so
folding a link into it would make every restyle of a selection overwrite the links of its members.
A link is navigation, not a visual attribute, which is the same reason the lock flag sits outside it
(`features/ambre-spec-toile-styles.md`).

**Steps, when a link is followed:**

1. read the target from the painted element, which carries the register;
2. for a frame, fit the camera to that frame's box (`TOILE-113`), or do nothing when the frame
   is gone (`TOILE-064`);
3. for a web address, hand it to the shell, which opens it sandboxed (`TOILE-065`);
4. for a document, ask the shell to open it (`TOILE-114`).

None of those steps sends a command, which is what `TOILE-063` states, and none of them
reaches the undo history, which is what `TOILE-115` states.

**Code:** none. The field, the command and the following are all unbuilt.

---

## 4. SURFACE

What the Story owes, stated so the rows above have a section to cite:

| Trigger | Behaviour |
|---|---|
| the link control on a single selection, in the style panel | opens a small chooser: an address, a frame from a list, or a document (`TOILE-066`) |
| `Ctrl/Cmd`-click on a linked element, with the select tool | follows the link (`TOILE-112`) |
| a click on the link glyph | follows the link |
| a plain click, with the select tool | selects, as always: a link never blocks editing (`TOILE-112`) |
| a plain click, in presentation mode | follows the link, because editing is off (`features/ambre-spec-toile-presentation.md`) |

**What a linked element looks like.** A small glyph on hover, painted in screen space so it stays the
same size through the zoom. A painted element would have to carry a flag saying it is linked, and it
does not.

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

* **The whole feature is unbuilt** (`TOILE-062` to `TOILE-066`). The field is declared `design`, the
  command and the target type appear nowhere in the workspace, and no painted element says whether it
  is linked. Three sections of the corpus described it as delivered before these rows said
  otherwise.
* **A dead link and an unclicked one are the same event** (`TOILE-064`). A follow the board
  cannot resolve does nothing and reports nothing, so a user cannot tell a target that was deleted
  from a glyph they missed. The register is kept because the frame may come back through a late
  change or an undo, and nothing on the screen says it is waiting.
* **A jump has no way back** (`TOILE-115`). Following writes nothing, so it enters no history:
  `Ctrl+Z` after a jump reverts the last edit and leaves the camera where the link put it. Whether
  a board owes a back gesture is unruled, and the presentation case below is the same hole one
  document wider.
* **A link on a connector is not designed.** A connector carries no style and cannot be selected
  (`features/ambre-spec-toile-connectors.md`), so captioning it comes first.
* **Nothing validates an address.** Whether a malformed address is refused at the write or ignored at
  the follow is open, and the shared open-URL model the roadmap names is where it should be settled
  once (`ambre-roadmap.md`, "Links in Trame").
* **A link out of a presentation has no back.** Following a document link during a presentation leaves
  the board, and nothing returns the presenter to the slide they left.
