# AMBRE -- FEATURE: LINKS ON AN ELEMENT

**Type:** Vertical (feature)
**Implementation:** partial -- the register, the command, the chooser and the three navigations. What
is not built is a way back from a jump, and any mark saying a target is gone.

> A user makes an element clickable: a web address, a frame on this board, or another Ambre
> document. The click opens the address, jumps the camera to the frame, or opens the document. A
> legend becomes navigation, and a frame becomes a table of contents.

Following a link changes nothing in the document. It is the one board gesture that writes nothing.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-062` | `INV` | A link MUST be its own register on the element, not part of its style. | this spec section 3 | `ambre_toile::logic::convergence_tests::a_link_is_set_and_cleared` -- it writes and clears a register the format holds as its own field beside `style` (`ambre-spec-toile-schema.md section 3`); `::setting_a_link_touches_nothing_else` -- the other direction | none | verified |
| `TOILE-109` | `INV` | A link MUST outlive its target: deleting the frame a link points at MUST NOT clear the link. | this spec section 3 | `ambre_toile::logic::convergence_tests::deleting_the_target_frame_leaves_the_link` -- it deletes the frame and reads the register back off the painted element | none | verified |
| `TOILE-110` | `IO` | The board MUST NOT fetch an address an element carries. | this spec section 4; `ambre-spec-zero-knowledge.md` | structural -- the absence is the proof: `ambre-toile` declares no HTTP client and no `fetch` binding, its follow path returns a `ShellNav` value rather than performing anything, and the one place an address is touched hands it to `window.open`. The ban is a capability an audit holds, and no test can observe a request that is never made | none | verified |
| `TOILE-UI-405` | `OBS` | The link badge MUST sit outside its element's top-right corner, at one screen size. | this spec section 4 | `ambre_toile::ui::link::tests::the_badge_sits_outside_the_top_right_corner` -- the badge box lies right of and above the element's screen box, at the badge size on both sides | none | verified |
| `TOILE-UI-406` | `OBS` | An element MUST paint a link badge exactly when it carries a link. | this spec section 4 | `ambre_toile::ui::render::tests::a_linked_element_gets_a_badge` -- one badge fill for a linked element beside an unlinked one | none | verified |

### 1.2 Set or clear a link

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-066` | `POST` | Setting or clearing a link MUST be one undo item. | this spec section 3 | `ambre_toile::logic::convergence_tests::a_link_is_set_and_cleared` -- it asserts one delta per write, and the handler makes one `doc.commit()`, which is what one undo item is (`features/ambre-spec-toile-undo.md`); `ambre_toile::logic::convergence_tests::each_link_write_is_one_undo_item` | none | verified |
| `TOILE-111` | `FRAME` | Setting or clearing a link MUST write the link register alone: the element's style, geometry and lock MUST NOT change. | this spec section 3 | `ambre_toile::logic::convergence_tests::setting_a_link_touches_nothing_else` -- it reads the painted element before and after and compares its bounds, its fill, its stroke and its lock | none | verified |
| `TOILE-288` | `FAIL` | Given an element the board does not hold, setting a link MUST answer an error and write nothing. | this spec section 3 | `ambre_toile::logic::convergence_tests::linking_a_missing_element_is_refused` -- no update, and a `NoSuchElement` error in the display events | none | verified |

### 1.3 Follow a link

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-112` | `PRE` | Following MUST be asked for by a gesture of its own -- a modified click, the link glyph, or a plain click where editing is off -- so a plain click with the select tool selects instead. | this spec section 4 | `ambre_toile::ui::link::tests::a_plain_click_never_follows_while_editing` -- it holds the rule as a function of the two inputs; `ambre_toile::ui::view::view_tests::a_plain_click_selects_a_linked_element_and_a_modified_click_follows` -- the view routes the click by it | none | verified |
| `TOILE-113` | `POST` | Following a link to a frame MUST fit the camera to that frame's box. | this spec section 3 | `ambre_toile::ui::link::tests::each_target_means_one_navigation` -- it holds that a frame target resolves to a jump and to nothing else; `ambre_toile::ui::view::view_tests::a_jump_fits_the_camera_to_the_frames_box` | none | verified |
| `TOILE-065` | `POST` | A web address MUST be opened by the shell, in a sandboxed tab. | this spec section 4; `ambre-spec-zero-knowledge.md` | `ambre_toile::ui::link::tests::only_the_outbound_halves_reach_the_shell` -- it holds that the board hands the address out rather than acting on it; `ambre_app::shell::app_root::tests::a_web_address_opens_in_a_fresh_tab_with_no_opener_and_no_referrer` | none | verified |
| `TOILE-114` | `POST` | A link to another Ambre document MUST be opened by the shell, never by the board. | this spec section 4 | `ambre_toile::ui::link::tests::only_the_outbound_halves_reach_the_shell` -- a document target leaves the board as a value; `::each_target_means_one_navigation` -- it resolves to that one arm; `ambre_app::shell::app_root::tests::a_document_link_is_opened_by_the_shells_router` | none | verified |
| `TOILE-063` | `FRAME` | Following a link MUST NOT write to the document. | this spec section 3 | structural -- the types are the proof: following resolves to a `LinkAction`, whose three arms are a camera request, an address and a document reference. The one board command it can produce is `RequestSelectionStats`, which the engine answers without a commit | none | verified |
| `TOILE-115` | `FRAME` | Following a link MUST NOT enter the undo history: a `Ctrl+Z` after a jump MUST revert the last edit, not the jump. | this spec section 3 | `ambre_toile::logic::convergence_tests::a_jump_is_not_an_undo_item` -- the Logic half; `ambre_toile::ui::view::view_tests::an_undo_after_a_jump_reverts_the_edit_and_not_the_jump` -- the view half | none | verified |
| `TOILE-064` | `FAIL` | Following a link to a frame the board no longer holds MUST do nothing, and MUST NOT report an error. | this spec section 3 | `ambre_toile::ui::view::view_tests::a_jump_to_a_frame_nobody_holds_does_nothing_and_says_nothing` | none | verified |
| `TOILE-UI-404` | `POST` | A follow MUST reach a linked element under the pointer, even where an unlinked element lies over it. | this spec section 4 | `ambre_toile::ui::link::tests::the_click_finds_the_top_most_linked_element` -- an unlinked box over a linked one resolves to the linked one, and a point off both to nothing | none | verified |

* **Budget.** none of its own once built: setting a link is one register write, under the
  transform-command row of `ambre-spec-budgets.md section 1`. A jump to a frame is a camera change,
  budgeted by `features/ambre-spec-toile-viewport.md`.
* **Boundary.** the one board feature that names an outbound path: the view hands a web address to
  the shell, which opens it in a sandboxed tab. The board never fetches it, and the address is
  document data, sealed like the rest (`TOILE-065`).
* **Verification.** 16 `verified`. Host tests read the register off the painted element and drive
  the view through each follow. Browser tests hold the shell's two outbound opens. The fetch ban is
  an audit, because the view hands a value out rather than acting on it (`TOILE-110`).

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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkTarget {
    Url(LinkUrl),     // a web address, opened sandboxed by the shell
    Frame(ElementId), // a frame on this board: the camera fits to it
    Document { document_id: DocumentId, doc_type: DocType },
}
```

The document arm carries its type beside its id, where the design carried an id alone. The shell
cannot build a route without the type, and asking the server for it turns a follow into a round trip
that can fail. An embed's reference takes the same shape, for the same reason
(`features/ambre-spec-toile-embeds.md`).

**Command** (`ambre-bus`): `SetLink { id, link }`, where an absent link clears the register. One
register write and one undo item, and nothing else on the element moves (`TOILE-066`,
`TOILE-111`). A locked element refuses it like it refuses every other edit.

**Why the link is not part of the style.** The style register is one value written whole. Folding
a link into it would make every restyle of a selection overwrite the links of its members. A link is
navigation, not a visual attribute, and the lock flag sits outside the style for that same reason
(`features/ambre-spec-toile-styles.md`).

**Steps, when the view follows a link:**

1. read the target from the painted element, which carries the register;
2. for a frame, **ask the board for that frame's box** and fit the camera to the answer
   (`TOILE-113`). A frame nobody holds answers no box, so the fit does nothing and raises no
   error (`TOILE-064`);
3. for a web address, hand it to the shell, which opens it sandboxed (`TOILE-065`);
4. for a document, hand it to the shell too (`TOILE-114`).

The view asks the board for the frame rather than reading it off the screen. The design read it from
the painted element, and that fails the case links exist to serve. A link to a frame is how a user
reaches one that sits **off screen**, and the culled frame holds no off-screen element.

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

**Where the chooser lives.** In the shell, not in the style panel the design named. Only the shell
can offer two of the three targets. It validates an address where the user can still read the
refusal, and it knows the documents this user may open. The frame list is the deck's, which already
holds every frame on the board -- including the off-screen ones a link exists to reach. The shell
hands the board a target that is already well formed, and the embed entries take the same inversion
(`features/ambre-spec-toile-embeds.md`).

**What a linked element looks like.** A small badge outside its top-right corner, painted in screen
space so it keeps its size through the zoom. Always painted, not shown on hover: nobody follows a
link nobody can see. The painted element carries the register, so the badge costs a walk of the
culled frame and no state.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The link is a register of its own | a field of the style register | the style is written whole, so a link inside it would be erased by any restyle of a selection that included the element |
| Following writes nothing | recording the jump, so a presenter could replay it | a jump is one user's camera, and a camera is per user; writing it would put one user's navigation into the other users' history |
| A dead frame target is inert | clearing the register when the frame is gone | it destroys data on a read, and the target may come back through a late change or an undo |
| A web address is opened by the shell, never fetched | fetching a preview of the address | the board holds document content, so a fetch from the board would put a request next to plaintext and give an attacker a path to it. The shell opens a sandboxed tab and the board learns nothing |
| Three target kinds and no more | a general action target | an address, a place on this board and another document are the three navigations a whiteboard needs; anything else is a command, and a command belongs to a menu |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A dead link and an unclicked one are the same event** (`TOILE-064`). A follow the board
  cannot resolve does nothing and reports nothing.
  * A user therefore cannot tell a target someone deleted from a glyph they missed. The board keeps
    the register, because the frame may come back through a late change or an undo. Nothing on the
    screen says it is waiting.
* **A jump has no way back** (`TOILE-115`). Following writes nothing, so it enters no history:
  `Ctrl+Z` after a jump reverts the last edit and leaves the camera where the link put it. No rule
  says whether a board owes a back gesture, and the presentation case below is the same hole one
  document wider.
* **A link on a connector is not built.** A user can now select an arrow, and an arrow carries a
  style of its own (`features/ambre-spec-toile-connectors.md`), so the reason for the deferral is
  gone.
  * What is left is that the register lives on an element map and an arrow is not one. Nothing
    offers it, and the chooser addresses the element selection only.
* **A link covers one element at a time.** The chooser answers with one target and the view
  applies it to a single selection. It drops a multi-selection rather than fanning out, because a
  chooser cannot mean "these five all point here" without saying so. Nothing on screen says why the
  entry did nothing.
* **The frame list has no names.** The design names a frame title, and nobody has written it
  (`features/ambre-spec-toile-frames.md`), so the chooser lists every frame as "Untitled frame".
  On a board with four frames, choosing the right one is guesswork until that lands.
* **Nothing validates an address.** Two answers stay open: the write refuses a malformed address,
  or the follow ignores it. The shared open-URL model the roadmap names is where one answer belongs
  (`ambre-roadmap.md`, "Links in Trame").
* **A link out of a presentation has no back.** Following a document link during a presentation leaves
  the board, and nothing returns the presenter to the slide they left.
