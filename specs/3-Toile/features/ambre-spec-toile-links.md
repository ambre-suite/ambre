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

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TOILE-062` | A link MUST be its own register on the element, not part of its style. | this spec section 3 | not built -- lands with the "Hyperlinks and go-to-frame" Story (`ambre-roadmap.md` E4) | unimplemented |
| `TOILE-063` | Following a link MUST be local: it MUST NOT write to the document and MUST NOT be an undo item. | this spec section 3 | not built -- lands with the "Hyperlinks and go-to-frame" Story (`ambre-roadmap.md` E4) | unimplemented |
| `TOILE-064` | A link to a frame the board no longer holds MUST be inert, and MUST NOT be cleared. | this spec section 3 | not built -- lands with the "Hyperlinks and go-to-frame" Story (`ambre-roadmap.md` E4) | unimplemented |
| `TOILE-065` | A web address MUST be opened by the shell, sandboxed, and MUST NOT be fetched by the board. | this spec section 4; `ambre-spec-zero-knowledge.md` | not built -- lands with the "Hyperlinks and go-to-frame" Story (`ambre-roadmap.md` E4) | unimplemented |
| `TOILE-066` | Setting or clearing a link MUST be one register write and one undo item. | this spec section 3 | not built -- lands with the "Hyperlinks and go-to-frame" Story (`ambre-roadmap.md` E4) | unimplemented |

* **Budget.** none of its own once built: setting a link is one register write, which lands under the
  transform-command row of `ambre-spec-budgets.md section 1`. A jump to a frame is a
  camera change, budgeted by `features/ambre-spec-toile-viewport.md`.
* **Boundary.** the one board feature that names an outbound path. A web address is handed to the
  shell and opened in a sandboxed tab; the board never fetches it, so no request carries document
  content and `ZK-001` is untouched. The address itself is document data, sealed like the rest
  (`TOILE-065`).
* **Verification.** 0 `verified`, 0 `unverified`, 5 `unimplemented`. Nothing here exists: the field is
  `design` in the format, the command appears nowhere in the workspace, and no surface can set or
  follow a link. The rows are the design, checkable against itself.

---

## 2. WHAT EACH USER SEES

The rows describe what the design owes, and each names the test the Story must bring.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| sets a link on a shape | sets a different link on the same shape | one of the two links, whole | the same target on both screens (`TOILE-062`) | none -- unbuilt |
| follows a link to a frame | watches | nothing new | Alice's camera on the frame, and Bob's camera where it was (`TOILE-063`) | none -- unbuilt |
| deletes the target frame | follows the link to it | the link, pointing at an id nobody holds | nothing happens on Bob's click, and the link stays (`TOILE-064`) | none -- unbuilt |
| clears the link | follows it at the same moment | the cleared register | nothing happens, and no error | none -- unbuilt |

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
register. One register write, one undo item (`TOILE-066`).

**Why the link is not part of the style.** The style register is one value written whole, so
folding a link into it would make every restyle of a selection overwrite the links of its members.
A link is navigation, not a visual attribute, which is the same reason the lock flag sits outside it
(`features/ambre-spec-toile-styles.md`).

**Steps, when a link is followed:**

1. read the target from the painted element, which carries the register;
2. for a frame, fit the camera to that frame's box, or do nothing when the frame is gone
   (`TOILE-064`);
3. for a web address, hand it to the shell, which opens it sandboxed (`TOILE-065`);
4. for a document, ask the shell to open it.

None of those steps sends a command, which is what `TOILE-063` states.

**Code:** none. The field, the command and the following are all unbuilt.

---

## 4. SURFACE

What the Story owes, stated so the rows above have a section to cite:

| Trigger | Behaviour |
|---|---|
| the link control on a single selection, in the style panel | opens a small chooser: an address, a frame from a list, or a document |
| `Ctrl/Cmd`-click on a linked element, with the select tool | follows the link |
| a click on the link glyph | follows the link |
| a plain click, with the select tool | selects, as always: a link never blocks editing |
| a plain click, in presentation mode | follows the link, because editing is off (`features/ambre-spec-toile-presentation.md`) |

**What a linked element looks like.** A small glyph on hover, painted in screen space so it stays the
same size through the zoom. A painted element would have to carry a flag saying it is linked, and it
does not.

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The link is a register of its own | the style is written whole, so a link inside it would be erased by any restyle of a selection that included the element | a field of the style register |
| Following writes nothing | a jump is one user's camera, and a camera is per user. Writing it would put one user's navigation into the other users' history | recording the jump, so a presenter could replay it |
| A dead frame target is inert | the target may come back through a late change or an undo, and clearing it would destroy data on a read | clearing the register when the frame is gone |
| A web address is opened by the shell, never fetched | the board holds document content, so a fetch from the board would put a request next to plaintext and give an attacker a channel. The shell opens a sandboxed tab and the board learns nothing | fetching a preview of the address |
| Three target kinds and no more | an address, a place on this board and another document are the three navigations a whiteboard needs. Anything else is a command, and a command belongs to a menu | a general action target |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The whole feature is unbuilt** (`TOILE-062` to `TOILE-066`). The field is declared `design`, the
  command and the target type appear nowhere in the workspace, and no painted element says whether it
  is linked. Three sections of the corpus described it as delivered before these rows said
  otherwise.
* **A link on a connector is not designed.** A connector carries no style and cannot be selected
  (`features/ambre-spec-toile-connectors.md`), so captioning it comes first.
* **Nothing validates an address.** Whether a malformed address is refused at the write or ignored at
  the follow is open, and the shared open-URL model the roadmap names is where it should be settled
  once (`ambre-roadmap.md`, "Links in Trame").
* **A link out of a presentation has no back.** Following a document link during a presentation leaves
  the board, and nothing returns the presenter to the slide they left.
