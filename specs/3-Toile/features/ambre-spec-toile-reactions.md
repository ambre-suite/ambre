# AMBRE -- FEATURE: REACTIONS AND VOTES

**Type:** Vertical (feature)
**Implementation:** partial -- emoji reactions ship whole; no surface casts a vote.

> A user drops an emoji on a note and everyone sees it, with a count. A workshop reads the board at a
> glance: what the group liked, what it agreed on.

One user has one reaction per element. Reacting again replaces it, so a count is a count of people.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The menu machinery that carries the picker
is `features/ambre-spec-toile-menus.md` and is not restated here.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-FEAT-007` | `CONC` | A reaction MUST be keyed by the person, not the session: the same user reacting from two devices MUST count once. | this spec section 3; section 5 | `ambre_toile::logic::convergence_tests::same_user_two_devices_is_one_entry` -- it reacts from two states carrying one identity, syncs both ways and reads a single voter on each | none | verified |
| `TOILE-FEAT-055` | `CONC` | Two users reacting at once on the same target MUST both keep their reaction, however fresh the target is. | this spec section 3; section 5 | `ambre_toile::logic::convergence_tests::concurrent_reactions_distinct_users_converge` -- it syncs two identities both ways and reads one chip counting two on each | none | verified |
| `TOILE-FEAT-019` | `INV` | A tally MUST be derived on read, and MUST NOT be stored. | this spec section 3; section 5 | structural -- no container holds a count (`ambre-spec-toile-schema.md section 3`) and both derivations fold the entry set on every read | none | verified |
| `TOILE-FEAT-010` | `OBS` | A deleted target MUST NOT carry a badge on the board, rather than surface a dangling one. | this spec section 3; section 5 | `ambre_toile::logic::convergence_tests::derive_viewport_carries_tally_and_skips_deleted_target` -- it deletes a reacted target and reads a derived map that no longer holds it | none | verified |
| `TOILE-FEAT-056` | `OBS` | Asking one deleted target for its tally MUST answer nothing, on the same terms. | this spec section 3 | `ambre_toile::logic::convergence_tests::tally_skips_a_deleted_target` -- it asks the single-target path after the delete. That path is a second guard, on the element map rather than on the visible set, and the only caller it has is that test (section 6) | none | verified |
| `TOILE-FEAT-057` | `OBS` | The board's tallies MUST cover what is on screen, so a reacted target scrolled out of view costs nothing to derive. | this spec section 3; `ambre-spec-budgets.md section 1` | none | unheld -- the whole-board derivation routes each entry by its target and drops the ones the cull left out, so an off-screen target is skipped exactly as a deleted one is, and no test distinguishes the two | unverified |
| `TOILE-FEAT-058` | `INV` | The chips of one target MUST keep the same order from one read to the next, so a count does not shuffle under the reader. | this spec section 3 | none | unheld -- the order is a sorted map flushed into a vector, and no test ever puts two distinct emoji on one target: every reaction test uses a single glyph | unverified |
| `TOILE-FEAT-059` | `OBS` | Votes on a target MUST fold into one signed total and one voter count. | this spec section 3 | `ambre_toile::logic::convergence_tests::same_user_two_devices_is_one_entry` -- it reads the voter count for a single positive weight | partial -- nothing sums two weights, and nothing sums weights of opposite signs, which is the only case the signed total exists for | unverified |
| `TOILE-UI-080` | `OBS` | A badge MUST keep one screen size through the whole zoom range. | this spec section 4 | none | unheld -- the chip metrics are logical pixels and only the element box is mapped to screen, so the size is fixed by construction, but no test paints the same tally at two zooms | unverified |
| `TOILE-UI-299` | `OBS` | A badge row MUST stay legible over whatever it sits on, in edit mode and in presentation alike. | this spec section 4 | none | unheld -- the row is painted after the content and after the presence overlay, and unconditionally on mode, but the order is two call sites and nothing holds it | unverified |
| `TOILE-FEAT-022` | `OBS` | A user MUST be able to cast a vote. | this spec section 3 | not built | unbuilt -- lands with the `The facilitation layer` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `TOILE-FEAT-100` | `OBS` | A vote MUST replace the voter's emoji on that target. | this spec section 3 | not built | unbuilt -- lands with the `The facilitation layer` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `TOILE-FEAT-024` | `CONC` | A shared timer MUST show every participant the same remaining time, and a late joiner MUST see it running. | this spec section 3 | not built | unbuilt -- lands with the `The facilitation layer` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `TOILE-FEAT-025` | `INV` | A vote session MUST cap each user's votes. | this spec section 3; `features/ambre-spec-toile-elements.md section 3` | not built | unbuilt -- lands with the `The facilitation layer` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `TOILE-FEAT-101` | `INV` | The cap on a vote session MUST be advisory: it MUST NOT be enforced against a peer. | this spec section 3; `features/ambre-spec-toile-elements.md section 3` | not built | unbuilt -- lands with the `The facilitation layer` Story (`ambre-roadmap.md` post-V1) | unimplemented |

### 1.2 React to an element

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-FEAT-060` | `PRE` | The caller MUST name a target the board still holds. | this spec section 3 | none | unheld -- nothing checks it: the write takes no existence check, so a reaction on a target that is already gone is stored and never rendered by either derivation. Answered by the `A Toile command that names nothing answers an error` Story (`ambre-roadmap.md` E7) | unverified |
| `TOILE-FEAT-061` | `POST` | Reacting again MUST replace the user's own reaction, so a count stays a count of people. | this spec section 3 | `ambre_toile::logic::convergence_tests::react_add_change_remove_updates_tally` -- it places an emoji, replaces it with a vote and reads the emoji gone and one vote standing, then removes it and reads nothing left | none | verified |
| `TOILE-FEAT-062` | `POST` | A reaction that changes something MUST reach the other users. | this spec section 3 | `ambre_toile::logic::convergence_tests::react_broadcasts_delta_and_is_undoable` -- it reads an outcome carrying an update | none | verified |
| `TOILE-FEAT-067` | `POST` | A reaction that changes something MUST repaint the badges of that target, on this screen too. | this spec section 4 | `ambre_toile::logic::convergence_tests::react_broadcasts_delta_and_is_undoable` -- it reads an outcome announcing that target, and the view queues a fresh board for an announced target | none | verified |
| `TOILE-FEAT-020` | `POST` | Clearing a reaction the user does not have MUST do nothing at all: no delta, and nothing to undo. | this spec section 3 | `ambre_toile::logic::convergence_tests::react_broadcasts_delta_and_is_undoable` -- it clears twice and reads nothing broadcast by the second clear | none | verified |
| `TOILE-FEAT-063` | `POST` | Placing again the reaction the user already has MUST do nothing at all, on the same terms. | this spec section 3 | none | unheld -- the skip is a byte comparison against the stored value and no test places the same reaction twice, so a gesture that would otherwise cost an undo item is unheld | unverified |
| `TOILE-FEAT-021` | `FAIL` | A reaction MUST NOT be written before the user's canonical identity is known. | this spec section 3 | `ambre_toile::logic::convergence_tests::react_before_session_ready_is_a_noop` -- it reacts on a state with no identity and reads nothing broadcast and no tally | none | verified |
| `TOILE-FEAT-064` | `FAIL` | A reaction the client cannot write MUST tell the user, rather than leave the gesture looking applied. | this spec section 3; section 6 | none | fails -- an encode failure logs a warning and returns an empty outcome, so the picker closes on a reaction that was never written and nothing on screen says so. Answered by the `A lost edit is never shown as applied` Story (`ambre-roadmap.md` E7) | unverified |
| `TOILE-FEAT-065` | `FRAME` | Reacting MUST NOT touch anybody else's reaction on that target. | this spec section 3; section 5 | structural -- the write is one insert or one delete under the key built from this user's identity, so no other user's key is reachable | none | verified |

### 1.3 Undo a reaction

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-FEAT-023` | `POST` | Undoing a reaction MUST remove that user's own entry. | this spec section 3; `TOILE-014` | `ambre_toile::logic::convergence_tests::react_broadcasts_delta_and_is_undoable` -- it undoes the reaction and reads the tally empty again | none | verified |
| `TOILE-FEAT-066` | `FRAME` | Undoing a reaction MUST NOT touch every other entry on that target. | this spec section 3 | structural -- one reaction is one commit over one key, so the history has one key to put back | none | verified |

### 1.4 Pick an emoji from the menu

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-081` | `POST` | One reaction gesture MUST apply to every element of the selection. | this spec section 4 | `ambre_toile::ui::menu::tests::react_emoji_resolves_to_one_react_per_target` -- it resolves one entry against two selected elements and reads one command per element, each carrying the picker's glyph | none | verified |
| `TOILE-UI-300` | `POST` | The clear entry MUST remove this user's own reaction from every selected element, and only their own. | this spec section 4 | `ambre_toile::ui::menu::tests::react_clear_resolves_to_none_reaction` -- it holds the resolution, over one element only, and the per-user half is held by `TOILE-FEAT-065` | none | verified |
| `TOILE-UI-301` | `FRAME` | The picker MUST NOT place an emoji it does not show: four entries, and a clear. | this spec section 4; section 5 | structural -- the arm matches the four fixed command ids and looks their glyph up in the same table the menu renders from | none | verified |

### 1.5 Paint the badge row

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-079` | `POST` | Every visible target carrying a tally MUST paint a badge row. | this spec section 4 | `ambre_toile::ui::reactions::tests::chip_row_paints_only_tallied_elements` -- it paints a tallied element and reads a filled chip pill | none | verified |
| `TOILE-UI-302` | `POST` | A visible target with no tally MUST paint nothing, so an untouched element stays untouched. | this spec section 4 | none | unheld -- `ambre_toile::ui::reactions::tests::chip_row_paints_only_tallied_elements` puts a tallied and an untallied element on one board and then reads only that at least one pill was filled: it never counts them, so the untallied half is unheld | unverified |
| `TOILE-UI-303` | `POST` | A board nobody has reacted to MUST cost no drawing at all. | this spec section 4 | `ambre_toile::ui::reactions::tests::empty_reactions_paint_nothing` -- it reads a pass that records no operation whatsoever | none | verified |
| `TOILE-UI-304` | `POST` | Votes on a target MUST collapse into one trailing chip: the signed total, then the number of voters. | this spec section 4 | none | unheld -- the vote branch of the row is reached only by a tally carrying votes and no test builds one, so the format nobody can produce is also the format nobody checks | unverified |
| `TOILE-UI-305` | `FRAME` | Painting badges MUST NOT change the board. | this spec section 4 | type-level -- the pass takes the derived board by shared reference and a drawing surface, so it has nothing to write to | none | verified |

* **Budget.** the whole-viewport tally in one scan of the reactions map, the reaction-tally row of
  `ambre-spec-budgets.md section 1`. The derivation is one pass over the reactions, not one
  pass per element, so it costs what the board has reacted to and not what it holds.
* **Boundary.** none of its own: a reaction is written to the document, and `ZK-001` seals every
  payload before it leaves the client. The emoji is document data like any other.
* **Verification.** 19 `verified`, 10 `unverified`, 5 `unimplemented`. Twelve rows became
  thirty-four over the same eleven tests. The convergence half is still the best-covered subject of
  the collaboration surface -- two devices, two users and a deleted target are each held, and five
  more rows are structural. The gaps are the painting: a chip size nothing measures, an untallied
  element nothing checks, a vote chip nothing can build, and an emoji order no test can shuffle
  because no test ever places two.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| reacts with a heart on a note | reacts with a heart on the same note | two entries, one per user | one heart chip, counting two (`TOILE-FEAT-055`) | `logic::convergence_tests::concurrent_reactions_distinct_users_converge` |
| reacts from her laptop | -- | one entry | one heart, counting one, however many devices Alice opens (`TOILE-FEAT-007`) | `logic::convergence_tests::same_user_two_devices_is_one_entry` |
| replaces her heart with a check | watches | Alice's one entry, now a check | the heart gone and the check counting one (`TOILE-FEAT-061`) | `logic::convergence_tests::react_add_change_remove_updates_tally` |
| reacts on a note | deletes that note | the note gone, the entry kept | no badge, and no arrow to nowhere (`TOILE-FEAT-010`) | `logic::convergence_tests::derive_viewport_carries_tally_and_skips_deleted_target` |
| scrolls the note off screen | -- | the entry kept | the same nothing: the board's tallies are culled with the board, and no test tells the two cases apart (`TOILE-FEAT-057`) | none |
| clears her reaction twice | watches | one delete, then nothing | no second change reaching Bob (`TOILE-FEAT-020`) | `logic::convergence_tests::react_broadcasts_delta_and_is_undoable` |
| places the heart she already has | watches | one entry, unchanged | nothing at all -- and nothing drives that skip (`TOILE-FEAT-063`) | none |

---

## 3. MODEL

**Containers.** The `reactions` map, one entry per target and user under the composite key
`"{target_id}/{user_id}"` (`ambre-spec-toile-schema.md section 3`). The deferred `timer` and
`vote_session` maps are declared there too, and nothing writes them.

**The value** (`ambre-bus`):

```rust
pub struct Reaction {
    pub kind: ReactionKind, // Emoji(String) or Vote(i32)
    pub ts:   i64,          // display only
}

pub struct ReactionTally {
    pub emoji:      Vec<(String, u32)>, // sorted, so the chip order is stable
    pub vote_total: i64,
    pub vote_count: u32,
}
```

**Command** (`ambre-bus`): `React { target, reaction }`, where an absent reaction removes this user's
own entry. One commit, so one undo item (`TOILE-FEAT-023`).

**Event** (`ambre-bus`): `ReactionsUpdated(target)`.

**Steps, when a user reacts:**

1. stop when no canonical identity is known: there is no key to write under (`TOILE-FEAT-021`);
2. build the key from the target and that identity, and take the target on trust: a target that is
   already gone is written to, and skipped only at derivation (`TOILE-FEAT-060`);
3. stop when the write would change nothing, so an unchanged reaction is not an undo item -- a
   clear with no entry to clear (`TOILE-FEAT-020`), or a byte-identical re-write (`TOILE-FEAT-063`);
4. write or delete the one key, commit once, and emit the event (`TOILE-FEAT-062`,
   `TOILE-FEAT-067`).

An encode failure between 3 and 4 logs a warning and returns the same empty outcome as step 1, so
the two refusals are indistinguishable to the caller and neither reaches the user
(`TOILE-FEAT-064`).

**Steps, when the board is derived:**

1. walk the reactions map once, and split each key on its first separator to recover the target;
2. skip a target the visible set does not hold, which is what drops a deleted one
   (`TOILE-FEAT-010`);
3. fold each entry into its target's counts: one per emoji, and a signed total plus a voter count.

A single-target tally takes the same path over the target's key prefix, and checks that the element
still exists first (`TOILE-FEAT-056`) -- a second guard, on a different condition, and no caller
outside its test (section 6).

**Code:**

* `crates/ambre-toile/src/logic/state/reactions.rs` -- the write, the two derivations and the tally;
* `ui/reactions.rs` -- the badge pass;
* `ui/menu.rs` -- the picker's command ids and their resolution against the selection.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| one of the four emoji entries of the context menu | places that emoji on every selected element (`TOILE-UI-081`) |
| the clear entry of the context menu | removes this user's own reaction from every selected element (`TOILE-UI-300`) |
| any change to a tally | repaints the badge row of that element (`TOILE-FEAT-067`) |

**The badges.** One chip per emoji, the glyph then its count, laid left to right under the element's
bottom edge. Votes collapse into one trailing chip: the signed total, then the number of voters. The
row paints above the board and above the presence overlay
(`features/ambre-spec-toile-presence.md section 4`), in edit mode and in presentation alike.

**The picker is four emoji and a clear.** They are fixed command ids in the registry, so they render,
resolve and are tested like every other menu entry
(`features/ambre-spec-toile-menus.md section 3`). A browser over the whole emoji set is not built.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| One flat map keyed by target and user together | a sub-map per target, created by the first reactor | two users reacting at once on a fresh element create two rival containers and the merge keeps one. The root map always exists, so the flat shape writes two keys and both survive; ids never contain the separator, so the target is recovered by splitting |
| Keyed by `UserId`, never `PeerID` | keying by the writing peer | a per-session key would let one user count twice and would strand the entry on the next session. A reaction is a decision by a person, and it must survive a reload, a key rotation and a second device |
| The tally is derived, never stored | a counter per emoji, incremented on write | a stored count is a second truth that drifts the first time a write is lost or replayed. Deriving it costs one pass and can be wrong in one place only |
| A deleted target keeps its entries | deleting the entries with their target | the delete may be undone, and destroying data on a read is how a concurrent edit loses work. The entries are skipped at derivation and reclaimed by compaction |
| A reaction is an undo item | excluding it from the history, as a comment is (`features/ambre-spec-toile-comments.md section 3`) | a reaction is a write to the document like any other, and the history is the one place a mistaken write is corrected |
| The picker is four fixed emoji | a full emoji browser | a workshop needs agreement, approval and applause, not a search field. Four ids are four registry commands, tested like the rest of the menu |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing casts a vote** (`TOILE-FEAT-022`). The value kind, the signed total, the voter count and
  the trailing badge all exist and are tested; no menu entry, button or chord produces one. The
  emoji half of this document has a surface, the vote half has none, and no open Story carries the
  difference.
* **The facilitation layer is a design** (`TOILE-FEAT-024`, `TOILE-FEAT-025`). A shared timer and a
  facilitated vote session are shared intent, so they are CRDT and their containers are declared. The
  Story that would build them is the last one of its epic. A vote budget can only be advisory, like
  a lock: a modified client can
  overspend, and the relay cannot arbitrate (`features/ambre-spec-toile-engine.md section 3`).
* **A reaction carries a timestamp nothing writes** (`ts`). The picker sends zero, because the UI
  thread has no time source wired to it, and no reader uses the field. Eight bytes per entry carrying
  nothing.
* **Undo retracts a reaction, and the same argument says it should not.** A comment is excluded from
  the history because `Ctrl+Z` must not withdraw something a teammate has read (`TOILE-FEAT-017`); a
  reaction is as visible and is undoable. One of the two rules is wrong, and no document says which.
* **Two public read paths, and neither has a caller.** The single-target tally and the accessor
  that answers "what did I put here" are both `pub`, both tested, and named nowhere else in the
  workspace. The badge pass reads the derived map instead, so the guard on element existence that
  only the single-target path carries protects nothing that renders (`TOILE-FEAT-056`).
* **A reaction that will not encode is lost in the logs** (`TOILE-FEAT-064`), and an entry that
  will not decode is skipped at derivation on the same terms. Both are one line of `tracing` or
  less, and the user sees a gesture that did nothing.
* **Culled and deleted are the same answer** (`TOILE-FEAT-057`). The board's derivation drops a
  target the cull left out and a target that no longer exists through one condition, so no test can
  tell them apart and a bug in the cull would read as a correct delete.
* **A badge row is never placed around its neighbours.** Chips paint under the element whatever else
  is there, so two elements a chip-row apart overlap their badges.
* **Nothing bounds the reactions map.** A board reacted to for a year keeps every entry, including
  those of deleted targets, until a compaction nobody triggers.
