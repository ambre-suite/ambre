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

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TOILE-FEAT-007` | A reaction MUST be keyed by `UserId`: the same user reacting from two devices MUST count once. | this spec section 3 | `ambre_toile::logic::convergence_tests::same_user_two_devices_is_one_entry`, `::concurrent_reactions_distinct_users_converge`, `::react_add_change_remove_updates_tally`, `::tally_skips_a_deleted_target`, `::react_broadcasts_delta_and_is_undoable`, `::react_before_session_ready_is_a_noop` | verified |
| `TOILE-FEAT-010` | A reaction tally MUST skip a deleted target rather than surface a dangling badge. | this spec section 3 | `ambre_toile::logic::convergence_tests::derive_viewport_carries_tally_and_skips_deleted_target`, `::tally_skips_a_deleted_target` | verified |
| `TOILE-FEAT-019` | A tally MUST be derived on read, and MUST NOT be stored. | this spec section 3 | `ambre_toile::logic::convergence_tests::react_add_change_remove_updates_tally` reads the tally back after each of the three writes, `::derive_viewport_carries_tally_and_skips_deleted_target`; no container holds a count (`ambre-spec-toile-schema.md section 3`) | verified |
| `TOILE-FEAT-021` | A reaction MUST NOT be written before the user's canonical identity is known. | this spec section 3 | `ambre_toile::logic::convergence_tests::react_before_session_ready_is_a_noop` | verified |
| `TOILE-FEAT-023` | Undoing a reaction MUST remove that user's own entry, and MUST leave every other entry on the target. | this spec section 3; `TOILE-014` | `ambre_toile::logic::convergence_tests::react_broadcasts_delta_and_is_undoable` covers the own entry; the write is one key, so no other entry is reachable | verified |
| `TOILE-UI-079` | Every visible target carrying a tally MUST paint a badge row, and a target with none MUST paint nothing. | this spec section 4 | `ambre_toile::ui::reactions::tests::chip_row_paints_only_tallied_elements`, `::empty_reactions_paint_nothing` | verified |
| `TOILE-UI-081` | One reaction gesture MUST apply to every element of the selection. | this spec section 4 | `ambre_toile::ui::menu::tests::react_emoji_resolves_to_one_react_per_target`, `::react_clear_resolves_to_none_reaction` | verified |
| `TOILE-FEAT-020` | A reaction that changes nothing MUST produce no delta and no undo item. | this spec section 3 | `ambre_toile::logic::convergence_tests::react_broadcasts_delta_and_is_undoable` covers removing an entry that is absent. **Nothing covers the other half**: re-placing the reaction the user already has is skipped by a byte comparison no test drives | unverified |
| `TOILE-UI-080` | A badge MUST keep one screen size through the whole zoom range. | this spec section 4 | none. The chip metrics are logical pixels and only the element box is mapped to screen, so the size is fixed by construction; no test paints the same tally at two zooms | unverified |
| `TOILE-FEAT-022` | A user MUST be able to cast a vote, and their vote MUST replace their emoji on that target. | this spec section 3 | not built -- lands with the "The facilitation layer" Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `TOILE-FEAT-024` | A shared timer MUST show every participant the same remaining time, and a late joiner MUST see it running. | this spec section 3 | not built -- lands with the "The facilitation layer" Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `TOILE-FEAT-025` | A vote session MUST cap each user's votes, and the cap MUST be advisory. | this spec section 3; `features/ambre-spec-toile-elements.md section 3` | not built -- lands with the "The facilitation layer" Story (`ambre-roadmap.md` post-V1) | unimplemented |

* **Budget.** the whole-viewport tally in one scan of the reactions map, the reaction-tally row of
  `ambre-spec-budgets.md section 1`. The derivation is one pass over the reactions, not one
  pass per element, so it costs what the board has reacted to and not what it holds.
* **Boundary.** none of its own: a reaction is written to the document, and `ZK-001` seals every
  payload before it leaves the client. The emoji is document data like any other.
* **Verification.** 7 `verified`, 2 `unverified`, 3 `unimplemented`. The convergence half is the
  best-covered subject of the collaboration surface: two devices, two users and a deleted target are
  each held. The gaps are a no-op the code skips and nobody drives, a chip size nothing measures, and
  the whole facilitation layer, which is a design and not code.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| reacts with a heart on a note | reacts with a heart on the same note | two entries, one per user | one heart chip, counting two (`TOILE-FEAT-007`) | `logic::convergence_tests::concurrent_reactions_distinct_users_converge` |
| reacts from her laptop | -- | one entry | one heart, counting one, however many devices Alice opens (`TOILE-FEAT-007`) | `logic::convergence_tests::same_user_two_devices_is_one_entry` |
| replaces her heart with a check | watches | Alice's one entry, now a check | the heart gone and the check counting one | `logic::convergence_tests::react_add_change_remove_updates_tally` |
| reacts on a note | deletes that note | the note gone, the entry kept | no badge, and no arrow to nowhere (`TOILE-FEAT-010`) | `logic::convergence_tests::tally_skips_a_deleted_target` |
| clears her reaction twice | watches | one delete, then nothing | no second change reaching Bob (`TOILE-FEAT-020`) | `logic::convergence_tests::react_broadcasts_delta_and_is_undoable` |

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
2. build the key from the target and that identity;
3. stop when the write would change nothing, so an unchanged reaction is not an undo item
   (`TOILE-FEAT-020`);
4. write or delete the one key, commit once, and emit the event.

**Steps, when the board is derived:**

1. walk the reactions map once, and split each key on its first separator to recover the target;
2. skip a target the visible set does not hold, which is what drops a deleted one
   (`TOILE-FEAT-010`);
3. fold each entry into its target's counts: one per emoji, and a signed total plus a voter count.

A single-target tally takes the same path over the target's key prefix, and checks that the element
still exists first.

**Code:**

* `crates/ambre-toile/src/logic/state/reactions.rs` -- the write, the two derivations and the tally;
* `ui/reactions.rs` -- the badge pass;
* `ui/menu.rs` -- the picker's command ids and their resolution against the selection.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| one of the four emoji entries of the context menu | places that emoji on every selected element (`TOILE-UI-081`) |
| the clear entry of the context menu | removes this user's own reaction from every selected element |
| any change to a tally | repaints the badge row of that element |

**The badges.** One chip per emoji, the glyph then its count, laid left to right under the element's
bottom edge. Votes collapse into one trailing chip: the signed total, then the number of voters. The
row paints above the board and above the presence overlay
(`features/ambre-spec-toile-presence.md section 4`), in edit mode and in presentation alike.

**The picker is four emoji and a clear.** They are fixed command ids in the registry, so they render,
resolve and are tested like every other menu entry
(`features/ambre-spec-toile-menus.md section 3`). A browser over the whole emoji set is not built.

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| One flat map keyed by target and user together | the root map always exists, so two users reacting at once on a fresh element write two keys and both survive. Ids never contain the separator, so the target is recovered by splitting | a sub-map per target, created by the first reactor -- two rival containers, and the merge keeps one |
| Keyed by `UserId`, never `PeerID` | a reaction is a decision by a person, and it must survive a reload, a key rotation and a second device. A per-session key would let one user count twice and would strand the entry on the next session | keying by the writing peer |
| The tally is derived, never stored | a stored count is a second truth that drifts the first time a write is lost or replayed. Deriving it costs one pass and can be wrong in one place only | a counter per emoji, incremented on write |
| A deleted target keeps its entries | the delete may be undone, and destroying data on a read is how a concurrent edit loses work. The entries are skipped at derivation and reclaimed by compaction | deleting the entries with their target |
| A reaction is an undo item | it is a write to the document like any other, and the history is the one place a mistaken write is corrected | excluding it from the history, as a comment is (`features/ambre-spec-toile-comments.md section 3`) |
| The picker is four fixed emoji | a workshop needs agreement, approval and applause, not a search field. Four ids are four registry commands, tested like the rest of the menu | a full emoji browser |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing casts a vote** (`TOILE-FEAT-022`). The value kind, the signed total, the voter count and
  the trailing badge all exist and are tested; no menu entry, button or chord produces one. The
  roadmap's "Presentation and reactions" Story named votes as shipped, and its emoji half is what
  shipped.
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
* **A badge row is never placed around its neighbours.** Chips paint under the element whatever else
  is there, so two elements a chip-row apart overlap their badges.
* **Nothing bounds the reactions map.** A board reacted to for a year keeps every entry, including
  those of deleted targets, until a compaction nobody triggers.
