# AMBRE -- FEATURE: ELEMENT STYLES

**Type:** Vertical (feature)
**Implementation:** partial -- editing ships; an off-screen selection member is read wrong.

> A user picks a colour, a border, a thickness, a font, and the selection takes it. A panel at the
> side has everything; a small bar above the selection has what is used most.

Everything about how an element looks is one value, written whole. Two users restyling the same
element therefore see one of the two styles, never a mixture of both.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. That the style is one value is the
format's rule (`TOILE-018`), and the tear a per-attribute layout suffers is
`ambre-spec-toile-schema.md section 6`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TOILE-UI-019` | An edit from the floating bar MUST touch only the field it edits, and a mixed selection MUST report nothing rather than a wrong shared value. | this spec section 3 | `ambre_toile::ui::floatbar::tests::style_edit_touches_only_its_field`, `::common_fill_is_none_when_mixed`, `::common_fill_survives_when_shared`, `::style_edit_field_mappings`, `ambre_toile::ui::style::tests::differing_fill_flags_only_fill_mixed`, `::identical_elements_are_not_mixed` | verified |
| `TOILE-UI-066` | The panel MUST report a field that differs across the selection as mixed, rather than showing one member's value as shared. | this spec section 3 | `ambre_toile::ui::style::tests::differing_fill_flags_only_fill_mixed`, `::differing_stroke_width_and_opacity_flag_both`, `::differing_text_decorations_flag_underline_and_strikethrough`, `::single_element_shows_its_style_with_no_mixed`, `::identical_elements_are_not_mixed` | verified |
| `TOILE-UI-067` | An style edit MUST be sent as one whole value per element, never one per attribute. | this spec section 3; `TOILE-018` | structural, and the construction is the proof: the only command that changes style carries a whole value, and the format has one register per element, so no attribute has a register of its own. `ambre_toile::logic::convergence_tests::two_peers_converge` drives concurrent edits | verified |
| `TOILE-UI-068` | The floating bar MUST follow the selection on screen, and MUST hide when nothing is selected. | this spec section 4 | `ambre_toile::ui::floatbar::tests::anchor_is_top_edge_midpoint_in_screen_space`, `::dpr_and_offset_apply_to_the_anchor`, `::union_bounds_anchor_a_multi_selection`, `::empty_selection_hides_bar`, `::unresolved_selection_hides_bar` | verified |
| `TOILE-UI-069` | Neither surface MUST hold an authoritative style: both MUST re-derive from the selection and what is painted. | this spec section 3 | `ambre_toile::ui::style::tests::empty_selection_has_no_model`, `::unresolved_selection_has_no_model`, `ambre_toile::ui::floatbar::tests::flags_track_group_and_lock_state` | verified |
| `TOILE-UI-070` | Both surfaces MUST read the whole selection, not the part on screen. | this spec section 3 | none, and **the rule fails today**: both models resolve the selected identities against the culled visible set, so a selected element beyond the edge contributes neither its value nor its difference -- section 6 | unverified |
| `TOILE-UI-071` | An style written by an older build MUST survive a change to the value's layout. | this spec section 3; `TOILE-028` | none, and **the rule fails today**: the value is decoded whole and a decode failure falls back to the default, so any layout change silently resets the style of every element already drawn -- section 6 | unverified |

* **Budget.** the view-state update under 0.5 ms, one row of
  `ambre-spec-budgets.md section 2`: both models are a fold over the selection, and the bar's
  screen position is recomputed every frame inside that figure. Neither has a bench.
* **Boundary.** none: both surfaces are chrome on the UI thread, where `UI-THREAD-004` bans a network,
  crypto or persistence path. The value they emit enters the document and leaves as bytes the Sync
  worker seals (`ZK-001`).
* **Verification.** 5 `verified`, 2 `unverified`. The no-tear rule and the mixed-state reporting are
  held eleven ways between the two surfaces -- the best-covered chrome in the application. Both gaps
  are rules the code answers with something else, and one of them loses user data on an upgrade.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| makes a shape red | makes the same shape blue | one whole style, never a red border on a blue fill | the same colour on both screens (`TOILE-018`) | `logic::convergence_tests::two_peers_converge` |
| makes a shape bold from the bar | changes its fill from the bar at the same moment | both edits, each having touched its own field | both changes, because each edit re-derived the rest live (`TOILE-UI-019`) | `ui::floatbar::tests::style_edit_touches_only_its_field` |
| selects two shapes of different colours | watches | nothing: a selection is not shared | a mixed indicator on the fill, and the shared fields as values (`TOILE-UI-066`) | `ui::style::tests::differing_fill_flags_only_fill_mixed` |
| restyles a selection whose members differ | watches | one style on every member | every selected element identical afterwards, including the fields she did not touch | `ui::style::tests::identical_elements_are_not_mixed` |

---

## 3. MODEL

**Containers.** One binary field per element, holding the whole style
(`ambre-spec-toile-schema.md section 3`). No attribute has a container of its own, which is
`TOILE-018`.

**The value** (`ambre-bus`). Fill and border are optional, so "no fill" is representable; the text
colour is separate from the fill, because on a note the fill is the card:

```rust
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct ElementStyle {
    pub fill:       Option<u32>,   // ARGB; None = no fill
    pub stroke:     Option<u32>,
    pub stroke_w:   f32,
    pub opacity:    f32,           // 0.0 ..= 1.0
    pub corner:     f32,           // corner radius
    pub font:       FontSpec,      // family, size, weight, italic, underline, strikethrough
    pub shadow:     Option<Shadow>,
    pub text_color: Option<u32>,   // None = the kind's default ink
}
```

**Command** (`ambre-bus`): `RestyleElements { ids, style }`, one whole value over a whole selection,
one undo item. There is no per-attribute command and no colour command
(`TOILE-UI-067`).

**The two models.** Both fold the selection into a base -- the first resolved element's whole
style -- plus one difference flag per tracked field. They differ in the write:

| Surface | Write discipline | Consequence |
|---|---|---|
| the panel | edits accumulate on a working copy of the base | a mixed field becomes uniform after any edit (section 2, row 4) |
| the bar | each control emits one field, re-derived live | two quick edits compose instead of clobbering (`TOILE-UI-019`) |

**Steps, when either surface edits a field:**

1. resolve the selected identities against what is painted, and stop when none resolve;
2. fold their styles into a base and a set of difference flags (`TOILE-UI-066`);
3. apply the edited field over the base;
4. send one whole value over every selected identity, debounced on the panel side;
5. the locked members are skipped by the engine, not by the surface (`TOILE-033`).

**Per-kind defaults.** Birth seeds a per-kind style rather than a bare default: everything is
white with no border except a note, which is born a plain yellow card with no border and square
corners, and whose size sentinel makes its body track the card
(`features/ambre-spec-toile-elements.md`, `features/ambre-spec-toile-text-editing.md`).

**Code:**

* `crates/ambre-toile/src/ui/style.rs` -- the panel's fold and its difference flags;
* `ui/floatbar.rs` -- the bar's anchor, its flags and its one-field edits;
* `chrome/style.rs`, `chrome/floatbar.rs` -- the two surfaces themselves;
* `logic/state/elements.rs` -- the whole-value write and its lock refusal.

---

## 4. SURFACE

| Surface | Holds |
|---|---|
| the side panel, shown on a non-empty selection | fill, border, thickness, opacity; a text block with colour, family, size and the four decorations; the four stacking actions and lock (`features/ambre-spec-toile-z-order.md`) |
| the floating bar, above the selection | the four decorations, family, size, text colour and fill, then duplicate, delete, group, ungroup and lock |
| the colour chooser, used by both | a swatch grid on the application palette, plus a custom hue and alpha picker |

**Where the bar sits.** Anchored to the midpoint of the selection's top edge, in screen space,
recomputed from the camera every frame, so it tracks a pan and a zoom without lag. A multi-selection
anchors to the union box (`TOILE-UI-068`).

**The size control's "Auto" entry** maps to the note sentinel, and choosing a number sets an explicit
size that overrides it (`features/ambre-spec-toile-text-editing.md`).

**The two surfaces overlap on purpose**, and the bar is a subset: every action on it exists in the
panel or the menus, so the three can never offer different behaviour for the same name
(`features/ambre-spec-toile-menus.md`).

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| One whole value per element | a per-attribute layout lets two concurrent edits interleave into a style neither user asked for -- a red border on a blue fill. One value makes the tear unrepresentable, at the cost of last-writer-wins on the whole look | a register per attribute, which Trame's cell styles rejected for the same reason |
| A differing field is reported, not resolved | showing one member's value as though it were shared makes the user believe they are seeing the selection. A mixed indicator is the honest answer, and it is what every other suite shows | showing the first member's value |
| The bar re-derives per edit; the panel accumulates | the bar is for quick successive edits, so each one must start from what is on screen now, or the second edit undoes the first. The panel is for a considered pass over one selection, where a working copy is what the user expects | one discipline for both, which breaks one of the two use cases |
| The text colour is separate from the fill | on a note the fill is the card and the body is drawn over it, so one field cannot be both | reusing the fill for the text |
| The fill and the border are optional | "no fill" is a real style on a whiteboard -- an outline over something else -- and a sentinel colour cannot express it | a colour with a zero alpha |
| The surfaces hold no style | two copies of the selection's look drift the moment a remote change arrives, and the drift shows as a control that disagrees with the board | caching the model across selections |
| The lock is enforced by the engine | the surfaces would each need the rule, and a third surface would need it again. One refusal, at the write | filtering the locked members before sending |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A selection past the edge of the screen is read wrong** (`TOILE-UI-070`). Both models resolve the
  selected identities against the culled visible set, so an off-screen member contributes neither its
  value nor its difference: the panel can show a fill as shared when it is not, and an edit then makes
  it shared for real. Same class as the select-all defect (`TOILE-UI-033`), same fix -- the value has to
  come from the engine.
* **A layout change resets every style** (`TOILE-UI-071`). The value is decoded whole and a decode
  failure falls back to the default, so adding or removing a field makes every element already drawn go
  white. The format's answer is the schema version the application does not carry
  (`TOILE-027`, `TOILE-028`); this document's answer is that the value needs a version of its own before
  a first release.
* **Formatting is per element, never per range.** The text controls on both surfaces look like a text
  toolbar and write the element's style, so a word inside a sentence cannot be emphasized
  (`TOILE-UI-065`).
* **A connector has no style to edit** (`TOILE-046`). Both surfaces address elements only, and a
  connector cannot be selected anyway.
* **The bar's hide during a gesture has no test** (`TOILE-UI-068`). It hides while a drag, a resize or a
  rotate is in flight, so it does not follow the preview, and only the empty-selection half of the rule
  is driven.
* **Nothing bounds a thickness or a shadow.** The panel accepts what the control offers, and no rule
  caps a border width or a shadow extent against the element it decorates.
