# AMBRE -- FEATURE: FRAMES

**Type:** Vertical (feature)
**Implementation:** partial -- membership and ordering ship; nothing on the board joins an element to a frame.

> A frame is a titled region of the board. It holds the elements a user puts in it, paints behind
> them, and doubles as a slide when the board is presented.

A frame is an ordinary element with a membership list. That is why moving one, resizing one and
deleting one need no code of their own.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-011` | `INV` | A frame's children MUST paint after the frame: a child's place in the paint order MUST be later than its frame's. | this spec section 3 | `ambre_toile::logic::convergence_tests::frame_children_paint_after_their_frame` -- it creates both children before the frame so the clamp has to fire, and reads the paint order back. The two membership tests this row also cited never read that order. `ambre_toile::logic::state::groups::tests::the_frame_clamp_ends_on_a_duplicate_entry` -- the clamp ends on a paint order holding a duplicate entry; `ambre_toile::logic::convergence_tests::an_ungroup_into_a_frame_lifts_the_children_above_it` -- a framed group ungrouped: its children paint above the frame on both peers, and one undo restores the order | none | verified |
| `TOILE-049` | `INV` | An element MUST belong to at most one container: joining a frame MUST detach it from its group. | this spec section 3 | `ambre_toile::logic::convergence_tests::set_frame_children_detaches_from_a_group` -- both memberships are read back; `ambre_toile::logic::state::groups::cache_tests::a_member_listed_twice_leaves_its_group_for_a_frame` -- a member two concurrent adds listed twice leaves its group whole | none | verified |
| `TOILE-272` | `INV` | A frame MUST NOT count as its member an element the board no longer holds, even when its list still names it. | this spec section 3; `TOILE-120` | `ambre_toile::logic::convergence_tests::a_frame_entry_for_a_deleted_element_names_no_member` -- a frame add concurrent with the delete, read on both peers and on a fresh load of the stored bytes, each with caches equal to a rebuild | none | verified |
| `TOILE-051` | `CONC` | A frame's speaker-notes container MUST exist from its birth, so two peers editing it first MUST converge. | this spec section 3; `TOILE-029` | `ambre_toile::logic::convergence_tests::concurrent_notes_edits_converge` | none | verified |
| `TOILE-252` | `CONC` | Two users naming one frame at once MUST converge on one writer's whole name. | this spec section 3; `TOILE-018` | `ambre_toile::logic::convergence_tests::concurrent_names_converge_on_one_whole_name` -- two peers naming one frame, converging on one of the two names entire | none | verified |
| `TOILE-253` | `OBS` | A frame MUST show its title on the board, outside its own box so nothing inside it is covered. A frame nobody named MUST show a placeholder in its place. | this spec section 4 | `ambre_toile::ui::render::tests::a_named_frame_reserves_a_title_band_above_itself` -- the band above the frame's own top edge, starting at its corner, widening with the title; `ambre_toile::ui::render::tests::an_unnamed_frame_reads_as_a_placeholder` -- the placeholder standing in, and never empty; `ambre_toile::ui::tool::tests::an_unnamed_frame_can_still_be_pressed_by_its_band` -- the placeholder band takes a press; `ambre_toile::ui::render::tests::a_frame_title_is_drawn_above_the_frame_in_its_ink` -- the glyphs shaped with the shipped face, drawn above the frame's top edge in screen space, a title in the title ink and the placeholder in the secondary ink | none | verified |
| `TOILE-255` | `OBS` | A presentation MUST NOT paint a frame's title: a slide shows the board's content, not its structure. | this spec section 4; `TOILE-UI-083` | type-level -- the title pass has one call site, inside the content pass's `if !present`, so a slide cannot reach it | none | verified |

### 1.2 Add a selection to a frame

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-099` | `POST` | Adding elements to a frame MUST make each one a member. | this spec section 3 | `ambre_toile::logic::convergence_tests::set_frame_children_adds_and_removes_membership` -- the membership is read back after an add of two | none | verified |
| `TOILE-237` | `POST` | Removing elements from a frame MUST take each one out. | this spec section 3 | `ambre_toile::logic::convergence_tests::set_frame_children_adds_and_removes_membership` -- its remove step: one of the two members leaves and the membership is read back without it; `ambre_toile::logic::convergence_tests::a_remove_takes_every_entry_a_concurrent_add_left` -- a member two concurrent adds listed twice leaves on both peers; `ambre_toile::logic::convergence_tests::an_undone_delete_after_a_concurrent_re_add_keeps_the_caches_whole` -- an undone delete and a peer's re-add list the member twice, and one remove takes both | none | verified |
| `TOILE-100` | `POST` | An element removed and added again in one gesture MUST end up last in the frame. | this spec section 3 | `ambre_toile::logic::structure_tests::a_member_removed_and_added_in_one_gesture_ends_last` -- the first of two members removed and added in one command, read back last | none | verified |
| `TOILE-048` | `POST` | A membership change MUST be one undo item, the clamping of the paint order included. | this spec section 3 | `ambre_toile::logic::convergence_tests::a_clamped_membership_change_is_one_undo_item` -- it creates the child before the frame so the clamp fires, presses undo once, and reads both the membership and the paint order back | none | verified |
| `TOILE-101` | `FRAME` | An element the frame cannot take MUST NOT stop the rest: the frame itself, an id the board does not hold and a current member are skipped, and the other elements of the same gesture still join. | this spec section 3 | `ambre_toile::logic::structure_tests::an_ineligible_id_does_not_stop_the_rest` -- the frame itself, an unknown id and a current member in one gesture with an eligible element, which joins with no error | none | verified |
| `TOILE-102` | `FRAME` | A membership change that changes nothing MUST write nothing and broadcast nothing. | this spec section 3 | `ambre_toile::logic::structure_tests::a_membership_change_with_no_effect_writes_nothing` -- a current member added and a stranger removed: no change and no event | none | verified |
| `TOILE-050` | `FAIL` | A membership change aimed at an element that is not a frame MUST be refused, and MUST write nothing. | this spec section 3 | `ambre_toile::logic::structure_tests::a_membership_change_on_a_non_frame_is_refused` -- a shape named as the frame: the kind-mismatch error, no change, no member | none | verified |

### 1.3 Name a frame

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-247` | `POST` | Naming a frame MUST make the board hold that name, so every surface reading a frame's title reads what the user typed. | this spec section 3 | `ambre_toile::logic::convergence_tests::a_named_frame_is_found_by_its_title` -- the name read back off the board, trimmed, and the search finding the frame by it; `ambre_toile::tests::chrome_ui::deck_panel_names_a_frame` -- the panel's field emitting that name for a deck frame and for one off it | none | verified |
| `TOILE-248` | `POST` | Naming a frame MUST be one undo item. | this spec section 3 | `ambre_toile::logic::convergence_tests::naming_a_frame_is_bounded_undoable_and_refused_off_a_frame` -- one undo takes the name and leaves the frame, and the redo brings it back; `ambre_toile::tests::chrome_ui::deck_panel_names_a_frame` -- one edit leaves the panel as one intent | none | verified |
| `TOILE-249` | `POST` | A name MUST be bounded, so no frame can carry a title the document cannot hold. | this spec section 3 | `ambre_toile::logic::convergence_tests::naming_a_frame_is_bounded_undoable_and_refused_off_a_frame` -- a title past the bound, read back cut to it | none | verified |
| `TOILE-250` | `FRAME` | A naming that changes nothing MUST write nothing and broadcast nothing. | this spec section 3 | `ambre_toile::logic::convergence_tests::naming_a_frame_is_bounded_undoable_and_refused_off_a_frame` -- the same name again, answering no delta and no event | none | verified |
| `TOILE-251` | `FAIL` | A naming aimed at an element that is not a frame MUST be refused, and MUST write nothing. | this spec section 3 | `ambre_toile::logic::convergence_tests::naming_a_frame_is_bounded_undoable_and_refused_off_a_frame` -- a sticky answering the kind-mismatch error, with no delta and its own body untouched | none | verified |

### 1.4 Press a frame's title

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-254` | `POST` | A press on a frame's title MUST select that frame. | this spec section 4 | `ambre_toile::ui::tool::tests::a_press_on_a_frames_title_selects_the_frame` -- the board holding nothing under that point, and the press selecting the frame anyway; `ambre_toile::ui::tool::tests::an_unnamed_frame_can_still_be_pressed_by_its_band` -- the same press over a placeholder | none | verified |
| `TOILE-256` | `FRAME` | Selecting a frame by its title MUST NOT change the board: it MUST write nothing. | this spec section 4 | structural -- the press answers a selection change and the title branch reaches no command sink, so no write can leave it | none | verified |

### 1.5 Rename a frame in place

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-261` | `POST` | A double-click on a frame's title MUST open the editor on that title, carrying the title it replaces. | this spec section 4 | `ambre_toile::ui::tool::tests::a_double_click_on_a_frames_title_edits_that_title` -- the band routes the gesture to the title; `ambre_toile::ui::edit::tests::the_editor_opens_on_a_frames_title` -- the editor targets the title, not the frame's body | none | verified |
| `TOILE-262` | `FRAME` | An edit that types nothing over a frame nobody named MUST rename nothing: the placeholder is never written. | this spec section 4 | `ambre_toile::ui::edit::tests::an_untouched_placeholder_commits_nothing` | none | verified |
| `TOILE-UI-390` | `FAIL` | Given a pan that carries a frame off screen while its title is edited, the editor MUST NOT close: scrolling is not leaving. | this spec section 4; `features/ambre-spec-toile-text-editing.md` (`TOILE-UI-210`) | `ambre_toile::ui::view::view_tests::a_title_editor_outlives_a_pan_and_leaves_with_the_board` -- the editor fetches its frame, a pan writes no rename, and the board's word that the frame is gone closes it with no write and one notice | none | verified |

* **Budget.** the membership change under 1 ms, the transform-command row of
  `ambre-spec-budgets.md section 1`. The clamp re-materializes the whole paint order per moved
  child, so the figure is per child out of order and no bench measures it.
* **Boundary.** none of its own: a frame and its membership live in the Logic thread's document and
  leave as bytes the Sync worker seals (`ZK-001`).
* **Verification.** 24 `verified`. What holds is membership as data, the title beside it, and what a
  press or a double-click on it does. An element joins one frame, detaches from its group and paints
  after it in one undo item, and the notes and the name converge. The membership gesture is driven
  through what it refuses and skips, and the title's glyphs are read back.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| adds a shape to a frame | adds the same shape to another frame | the shape in one of the two frames, never in both | the shape in the frame that won, and no duplicate | `logic::convergence_tests::set_frame_children_detaches_from_a_group` |
| adds a shape to a frame | brings that shape to the back | the membership, and a paint order the clamp corrected | the shape above its frame on both screens (`TOILE-011`) | `logic::convergence_tests::frame_children_paint_after_their_frame` |
| edits the frame's speaker notes | edits the same notes at the same moment | both sentences, character by character | one merged text, with neither sentence lost | `logic::convergence_tests::concurrent_notes_edits_converge` |
| deletes the frame | moves a child of it | the frame gone; the child moved and no longer a member | the child alone on the board | `logic::convergence_tests::caches_match_a_full_rebuild` |
| names the frame "Agenda" | names it "Roadmap" at the same moment | one of the two names, whole | the same title on both screens, never a blend (`TOILE-252`) | `logic::convergence_tests::concurrent_names_converge_on_one_whole_name` |

Deleting a frame does not delete its members: only a group cascades
(`features/ambre-spec-toile-groups.md`).

---

## 3. MODEL

**Containers.** A frame is an element of the `elements` map whose kind is the frame kind. It owns two
nested containers, both declared in `ambre-spec-toile-schema.md section 3`: a membership list, and a
speaker-notes text. Its optional layout register belongs to
`features/ambre-spec-toile-auto-layout.md`.

**Commands** (`ambre-bus`): `SetFrameChildren { frame, add, remove }`, `RemoveFromFrames { ids }` and
`RenameFrame { frame, name }`, each meant as one undo item. `RemoveFromFrames` names no frame: the
handler takes each id out of every frame whose list holds it (`TOILE-UI-149`). Creating, moving,
resizing, rotating, reordering and deleting a frame use the ordinary element commands
(`features/ambre-spec-toile-elements.md`).

**Steps, when membership changes:**

1. resolve the frame and refuse anything that is not one (`TOILE-050`). A missing frame answers
   the unknown-element error, and an element of another kind answers the kind-mismatch error;
2. remove the listed ids from the membership list first, so a remove and an add of one id re-appends
   it at the end (`TOILE-099`, `TOILE-100`);
3. for each added id, skip the frame itself, an id the board does not hold, and a current member,
   without stopping the gesture (`TOILE-101`);
4. detach each survivor from its enclosing group, so it belongs to one container
   (`TOILE-049`);
5. stop and write nothing when neither list changed (`TOILE-102`);
6. prune a group the move emptied;
7. clamp every child that sits below the frame in the paint order to just above it (`TOILE-011`);
8. commit once, so the membership and the clamp undo together (`TOILE-048`).

**A list entry can outlive its element.** A delete takes the id off every list that names it
(`TOILE-271`). A frame add concurrent with that delete still leaves the entry in the merged list.
A board stored before `TOILE-271` holds such entries too, since its delete wrote no frame list. The
read skips the entry, as the paint order skips a dead entry (`TOILE-120`), and no peer repairs it
(`TOILE-272`).

**An ungroup can hand children to a frame.** When no group encloses the group, the frame with the
smallest id that holds it takes the children (`TOILE-157`). The clamp then lifts them above it
(`TOILE-011`). Two peers that saw different frames holding the group can each pick their own, so the
children land in two frames. A concurrent frame add opens the same breach of `TOILE-049`.

**Steps, when a frame is named:**

1. resolve the frame and refuse anything that is not one (`TOILE-251`);
2. trim the name and cut it to the bound, so no title outgrows what the document carries
   (`TOILE-249`);
3. stop and write nothing when the name did not change (`TOILE-250`);
4. write it as one register and commit once (`TOILE-247`, `TOILE-248`).

**The name is one register, never a shared text.** A title is short and written by one person at a
time. The whole value converging on one writer is what a user expects, where a character-wise text
would interleave two titles into a third nobody typed. That is the opposite choice from the
speaker notes beside it, which two people do write together (`TOILE-051`).

**Why the containers are seeded at birth.** The notes text and the membership list are created in the
frame's own creation write, not on the first edit. A lazy per-peer seed would mint two rival
containers under one key, and the merge would keep one and drop the other user's sentences. The rule
is the format's (`TOILE-029`), and the frame is the place the code applies it (`TOILE-051`).

**Code:**

* `crates/ambre-toile/src/logic/state/groups.rs` -- the membership handler, the clamp, and the tree
  helpers both frames and groups share;
* `logic/state/elements.rs` -- the birth seed of the two containers;
* `logic/state/derive.rs` -- the frame's paint node, and the snapshot the presentation pass takes of
  its declared children.

---

## 4. SURFACE

| Trigger | Sends |
|---|---|
| the frame tool, dragged on the board | `CreateElement` with the frame kind (`features/ambre-spec-toile-tools.md`) |
| a press on a frame's title | nothing: the selection is the view's own (`TOILE-254`, `TOILE-256`) |
| a double-click on a frame's title | `RenameFrame`, once the edit closes (`TOILE-261`, `TOILE-262`) |
| a pan that carries the frame off screen, its title under edit | nothing: the editor stays open on the frame it fetched (`TOILE-UI-390`) |
| "Add to frame", in the context menu over a selection inside a frame | `SetFrameChildren` adding the selection (`features/ambre-spec-toile-menus.md`) |
| "Remove from frame", in the same menu | `RemoveFromFrames` naming the selection (`TOILE-UI-149`), offered when a frame holds part of it (`TOILE-UI-384`) |
| typing in a deck row's name field | `RenameFrame` for that frame (`features/ambre-spec-toile-presentation.md section 4`) |

**What a frame looks like.** A rectangle painted from its own style register, before its members,
so it reads as a background rather than a box drawn over them. Its title sits above its top-left
corner, outside the box, in screen space so it keeps one size through the zoom (`TOILE-253`). A
press on it selects the frame, so a user grabs one without clicking through what is inside
(`TOILE-254`). A double-click renames it in place (`TOILE-261`), and a presentation drops the titles
with the grid (`TOILE-255`).

**A frame nobody named reads as a placeholder**, in the secondary ink, so every frame has something
to read and something to double-click (`TOILE-253`). The placeholder names no rank. The deck panel
numbers a frame by its place in the deck, and a second numbering here would call one frame two
different things. It is never written to the document, and an edit that types nothing over it
renames nothing (`TOILE-262`).

**What no surface offers.** Removing an element from a frame, and dropping an element into a frame by
dragging it there. The context menu's one entry adds; nothing removes, and membership is never
implied by position.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A frame is an element | a parallel container map, with a duplicated lifecycle and a per-frame stack | as an element, move, resize, rotate, delete and reorder apply for free and there is one paint order rather than one per frame; only the frame-specific content needs handlers |
| Membership is declared, never spatial | membership by containment, recomputed on every move | a user drags a shape across a frame constantly, so every drag would change the document, and two overlapping frames would leave the membership ambiguous |
| A frame's name is one register | a character-wise text, like the speaker notes | a title is short and written by one person at a time. Two writers would interleave into a third title nobody typed, where a register converges on one of the two whole |
| The ordering rule is enforced, not assumed | trusting the writers | a concurrent reorder can put a child under its frame, and the frame would then paint over its own contents. Clamping is cheap and makes the nesting a property rather than a convention |
| The ordering rule is enforced, not assumed | painting the frame in a pass of its own | a frame painted in its own pass is always in front or always behind, so the paint order stops saying anything about two frames overlapping, and the author loses a control they had |
| An element belongs to one container | multiple membership, resolved at derivation | a shape in a frame and in a group has two owners for one position, and the two would fight over it on every transform |
| A member entry for a deleted element is skipped, not repaired | pruning the entry when a peer reads it | it turns a read into a write, and every peer reading the board at once would write the same removal. The skip costs one lookup per asked id |
| Deleting a frame keeps its members | cascading like a group | it makes a frame dangerous to delete. A frame is a region, so removing the region should not remove what was inside it; a group is the opposite, because a group is a binding |
| The notes and membership containers are born with the frame | seeding them on first use | the concurrent-first-write collision is silent and loses user text, and it costs two empty containers per frame to make impossible |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing puts an element into a frame by dragging it.** The only trigger is a context-menu entry
  that requires the selection to already sit inside the frame's box, and nothing takes an element out
  again. A frame is therefore a region a user can draw and can barely fill.
* **The membership list is seeded beside the notes and nothing proves it.** Both containers are born
  with the frame, and only the notes have a concurrent-first-write test (`TOILE-051`). Two peers each
  adding the first child to a fresh frame is the same collision, untested.
* **The title's click target is an estimate.** The band's width is measured at six tenths of an em
  per character. The exact advance is known only once the text is shaped, and the target has to
  exist on a host with no font at all. A title of wide glyphs therefore reaches past its own band,
  and one of narrow glyphs stops short of it.
* **Two titles at the same corner overlap.** Nothing staggers the bands of two frames whose
  top-left corners meet, and the one painted last wins the click (`TOILE-254`).
* **Two unnamed frames read alike.** The placeholder carries no rank, so a board of unnamed frames
  shows the same words above each of them. What tells them apart is where they are.
* **The edited title does not hold its size through a zoom.** The editor's size is pinned when it
  opens, from the camera of that moment, while the painted band is screen space. Zooming mid-edit
  grows or shrinks the text being typed, and the committed title returns to the band's own size.
* **A resized frame does not carry its members.** The members keep their world positions, so shrinking
  a frame leaves them outside it and nothing says whether they are still members. They are.
* **Nesting a frame in a frame is neither allowed nor refused.** The handler refuses only the frame
  joining itself, so a frame may be added to another frame's membership, and the clamp then orders
  them. What that means for the presentation deck is not ruled
  (`features/ambre-spec-toile-presentation.md`).
