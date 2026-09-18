# AMBRE -- FEATURE: ELEMENT STYLES

**Type:** Vertical (feature)
**Implementation:** partial -- editing ships; a restyle overwrites what no control shows.

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

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-067` | `INV` | A style edit MUST be sent as one whole value per element, never one per attribute. | this spec section 3; `TOILE-018` | structural -- the only command that changes style carries a whole value, and the format has one register per element, so no attribute has a register of its own | none | verified |
| `TOILE-163` | `CONC` | Two users restyling the same element MUST both end on one of the two styles, never on a mixture. | this spec section 2; `TOILE-018` | `ambre_toile::logic::convergence_tests::two_peers_converge` -- it drives concurrent edits and reads the two peers in agreement | none | verified |
| `TOILE-UI-069` | `INV` | Neither surface MUST hold an authoritative style: both MUST re-derive from the selection and what is painted. | this spec section 3 | type-level -- both models are functions of the selection and the painted set returning a fresh value, so there is no field a style could survive a selection change in | none | verified |
| `TOILE-UI-066` | `OBS` | The panel MUST report a field that differs across the selection as mixed, rather than showing one member's value as shared. | this spec section 3 | `ambre_toile::ui::style::tests::differing_fill_flags_only_fill_mixed` -- the fill flag; `ambre_toile::ui::style::tests::differing_stroke_width_and_opacity_flag_both` -- two more; `ambre_toile::ui::style::tests::differing_text_decorations_flag_underline_and_strikethrough` -- the two decorations | none | verified |
| `TOILE-UI-161` | `OBS` | The panel MUST report a field that agrees across the selection as a value, never as mixed. | this spec section 3 | `ambre_toile::ui::style::tests::differing_fill_flags_only_fill_mixed` -- the three fields that agree stay unflagged; `ambre_toile::ui::style::tests::identical_elements_are_not_mixed` -- no flag at all on a uniform pair | none | verified |
| `TOILE-UI-162` | `OBS` | A selection of one MUST show that element's style with nothing mixed. | this spec section 3 | `ambre_toile::ui::style::tests::single_element_shows_its_style_with_no_mixed` -- the base is the element's own style and no flag is raised | none | verified |
| `TOILE-UI-163` | `OBS` | The panel MUST show nothing when no selected element resolves. | this spec section 3 | `ambre_toile::ui::style::tests::empty_selection_has_no_model` -- no model for an empty selection; `ambre_toile::ui::style::tests::unresolved_selection_has_no_model` -- none for an id the painted set does not hold | none | verified |
| `TOILE-UI-164` | `OBS` | The bar MUST report every field it edits as mixed when it differs across the selection. | this spec section 6 | none | fails -- the bar folds a common fill and nothing else, so the text colour, the family, the size and the four decorations are shown as though they agreed, section 6; lands with the `The style bar says what it does not know` Story (`ambre-roadmap.md` E7) | failing |
| `TOILE-UI-165` | `OBS` | The bar MUST show the selection's fill when every member shares it. | this spec section 4 | `ambre_toile::ui::floatbar::tests::common_fill_survives_when_shared` -- the shared colour reaches the swatch | none | verified |
| `TOILE-UI-166` | `OBS` | The bar MUST show no fill when the selection's fills differ. | this spec section 4 | `ambre_toile::ui::floatbar::tests::common_fill_is_none_when_mixed` -- the swatch is empty for two different fills | none | verified |
| `TOILE-UI-068` | `OBS` | The bar MUST sit at the midpoint of the selection's top edge. | this spec section 4 | `ambre_toile::ui::floatbar::tests::anchor_is_top_edge_midpoint_in_screen_space` -- both coordinates against a hand-computed midpoint | none | verified |
| `TOILE-UI-167` | `OBS` | A multi-selection MUST anchor the bar to the union of its members' boxes. | this spec section 4 | `ambre_toile::ui::floatbar::tests::union_bounds_anchor_a_multi_selection` -- the anchor of a two-element selection is the union's top-edge midpoint | none | verified |
| `TOILE-UI-168` | `OBS` | The anchor MUST be in the units the page positions with, not the units the canvas draws in. | this spec section 4 | `ambre_toile::ui::floatbar::tests::dpr_and_offset_apply_to_the_anchor` -- the anchor after a pixel-ratio division and a canvas offset | none | verified |
| `TOILE-UI-169` | `OBS` | The bar MUST hide when no selected element resolves. | this spec section 4 | `ambre_toile::ui::floatbar::tests::empty_selection_hides_bar` -- nothing is published for an empty selection; `ambre_toile::ui::floatbar::tests::unresolved_selection_hides_bar` -- none for an id the painted set does not hold | none | verified |
| `TOILE-UI-170` | `OBS` | The bar MUST hide while a move, a resize or a rotate is in flight, so it does not follow the preview. | this spec section 6 | none | unheld -- the view decides the transient hiding at publish time and that seam is wasm-only, so only the empty and unresolved halves are driven; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-171` | `OBS` | The bar MUST offer group, ungroup, lock and unlock according to what is selected. | this spec section 4 | `ambre_toile::ui::floatbar::tests::flags_track_group_and_lock_state` -- the group flag and both lock flags on a mixed pair | none | verified |
| `TOILE-UI-070` | `OBS` | Both surfaces MUST read the whole selection, not the part on screen. | this spec section 6 | none | fails -- both models resolve the selected identities against the culled visible set, so a selected element beyond the edge contributes neither its value nor its difference, section 6; lands with the `Toile reads the board, not the screen` Story (`ambre-roadmap.md` E7) | failing |
| `TOILE-UI-071` | `IO` | A style written by an older build MUST survive a change to the value's layout. | this spec section 6; `TOILE-028` | none | fails -- the value is decoded whole and a decode failure falls back to the default, so any layout change silently resets the style of every element already drawn, section 6; lands with the `A style survives a change to its layout` Story (`ambre-roadmap.md` E7) | failing |

### 1.2 Restyle the selection from the panel

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-172` | `POST` | A panel edit MUST leave every field it did not edit as the selection had it. | this spec section 3 | none | unheld -- the panel edits a working copy of the base and sends it whole, and no test drives an edit through the surface; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-173` | `POST` | A field the panel reported as mixed MUST become uniform after any edit. | this spec section 2 | none | unheld -- the whole value goes to every member, so the base's value for a mixed field lands on all of them, and no test reads a second field back after an edit; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-174` | `POST` | A restyle MUST NOT overwrite an attribute the surface does not show. | this spec section 6 | none | fails -- the corner radius, the shadow and the exact weight are carried by the value but tracked by no flag and offered by no control, so restyling a mixed selection silently gives every member the first one's; lands with the `The style bar says what it does not know` Story (`ambre-roadmap.md` E7) | failing |
| `TOILE-UI-175` | `POST` | A restyle over a whole selection MUST undo as one step. | this spec section 3 | none | unheld -- one command carries the ids and the value and the undo document owns the grouping, and no test restyles a selection and undoes it; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-176` | `FRAME` | A restyle MUST NOT change what is selected. | this spec section 3 | none | unheld -- the command carries ids and a value and the selection is local to the view, and no test reads the selection back after a restyle; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-182` | `FRAME` | A restyle MUST NOT drop the locked members before it is sent: the refusal belongs to the write (`TOILE-033`). | this spec section 5 | none | unheld -- the panel's model never reads the lock, so it has nothing to filter on, but the surface that builds the command is wasm-only and reached by no test; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.3 Change one field from the floating bar

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-019` | `POST` | A bar edit MUST overwrite exactly the field it carries. | this spec section 3 | `ambre_toile::ui::floatbar::tests::style_edit_touches_only_its_field` -- a decoration toggle leaves the other three, the fill, the colour and the size intact, and a fill edit leaves the whole text style intact | none | verified |
| `TOILE-UI-177` | `POST` | The bold toggle MUST write the two weights the renderer reads. | this spec section 3 | `ambre_toile::ui::floatbar::tests::style_edit_field_mappings` -- both weights off the toggle | none | verified |
| `TOILE-UI-178` | `POST` | Resetting the family MUST write the default face, not an absent one. | this spec section 3 | `ambre_toile::ui::floatbar::tests::style_edit_field_mappings` -- the reset writes the default label, and a chosen family writes its own | none | verified |
| `TOILE-UI-179` | `POST` | Each bar edit MUST start from the selection's style as it stands, so two quick edits compose. | this spec section 3 | none | unheld -- the view re-derives the common style at apply time and that seam is wasm-only, and no test applies two edits in sequence and reads both back; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-180` | `FAIL` | A size the control cannot mean MUST be refused, not turned into the auto sentinel. | this spec section 6 | none | fails -- a negative size is clamped to zero, which is the sentinel that sizes the body to the card, so a bad number silently becomes a different feature; lands with the `The style bar says what it does not know` Story (`ambre-roadmap.md` E7) | failing |
| `TOILE-UI-181` | `FRAME` | A bar edit MUST NOT change what is selected. | this spec section 3 | none | unheld -- the edit carries one field and the view attaches the live ids, and no test reads the selection back after an edit; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |

* **Budget.** the view-state update under 0.5 ms, one row of `ambre-spec-budgets.md section 2`. Both models
  are a fold over the selection, and the bar's screen position is recomputed every frame inside that figure.
  Neither has a bench.
* **Boundary.** none: both surfaces are chrome on the UI thread, where `UI-THREAD-004` bans a network,
  crypto or persistence path. The value they emit enters the document and leaves as bytes the Sync
  worker seals (`ZK-001`).
* **Verification.** 17 `verified`, 5 `failing`, 8 `untested`. What holds is the read: the base value, the
  difference flags, the anchor and the one-field mapping. What is loose is every write, untested through
  either surface: the panel rows and the composition rule rest on reading the code. The five failures are
  rules the code answers otherwise, one losing user data on an upgrade. The lock itself is `TOILE-033`.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| makes a shape red | makes the same shape blue | one whole style, never a red border on a blue fill | the same colour on both screens (`TOILE-018`) | `logic::convergence_tests::two_peers_converge` |
| makes a shape bold from the bar | changes its fill from the bar at the same moment | both edits, each having touched its own field | both changes, because each edit re-derived the rest live (`TOILE-UI-019`) | `ui::floatbar::tests::style_edit_touches_only_its_field` |
| selects two shapes of different colours | watches | nothing: a selection is not shared | a mixed indicator on the fill, and the shared fields as values (`TOILE-UI-066`) | `ui::style::tests::differing_fill_flags_only_fill_mixed` |
| restyles a selection whose members differ | watches | one style on every member | every selected element identical afterwards, including the fields she did not touch (`TOILE-UI-173`) | none: no test drives an edit through either surface |
| restyles two cards differing only in corner radius | watches | one style on every member | both cards squared off, on an attribute no control offered (`TOILE-UI-174`) | none, and the rule fails today -- section 6 |

---

## 3. MODEL

**Containers.** One binary field per element, holding the whole style
(`ambre-spec-toile-schema.md section 3`). No attribute has a container of its own, which is
`TOILE-018`.

**The value** (`ambre-bus`). Fill and border are optional, so "no fill" is representable. The text colour is
separate from the fill, because on a note the fill is the card:

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

**The flags cover what a surface offers, not what the value holds.** `corner` and `shadow` are tracked by no
flag, and the weight is folded to bold-or-not rather than compared. A selection that differs on those three is
silently made uniform by any edit (`TOILE-UI-174`).

**Steps, when either surface edits a field:**

1. resolve the selected identities against what is painted, and stop when none resolve;
2. fold their styles into a base and a set of difference flags (`TOILE-UI-066`);
3. apply the edited field over the base;
4. send one whole value over every selected identity, debounced on the panel side;
5. the locked members are refused by the engine, not filtered by the surface (`TOILE-UI-182`).

**Per-kind defaults.** Birth seeds a per-kind style rather than a bare default. Everything is white with no
border except a note, which is born a plain yellow card with no border and square corners. Its size sentinel
makes its body track the card (`features/ambre-spec-toile-elements.md`,
`features/ambre-spec-toile-text-editing.md`).

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

**Where the bar sits.** Anchored to the midpoint of the selection's top edge, in screen space, and recomputed
from the camera every frame. So it tracks a pan and a zoom without lag. A multi-selection anchors to the union
box (`TOILE-UI-068`, `TOILE-UI-167`).

**What the bar folds.** The fill alone: a colour every member shares reaches the swatch, and a
divergence empties it (`TOILE-UI-165`, `TOILE-UI-166`). The seven other editable fields carry no
flag of their own, so the bar shows them as though the selection agreed (`TOILE-UI-164`).

**The size control's "Auto" entry** maps to the note sentinel, and choosing a number sets an explicit
size that overrides it (`features/ambre-spec-toile-text-editing.md`). The sentinel is zero, which is
also what a negative number clamps to (`TOILE-UI-180`).

**The two surfaces overlap on purpose**, and the bar is a subset. Every action on it exists in the panel or
the menus, so the three can never offer different behaviour for the same name
(`features/ambre-spec-toile-menus.md`).

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| One whole value per element | a register per attribute | it lets two concurrent edits interleave into a style neither user asked for -- a red border on a blue fill. One value makes the tear unrepresentable, at the cost of last-writer-wins on the whole look, and Trame's cell styles refused the split for the same reason |
| A differing field is reported, not resolved | showing the first member's value | it makes the user believe they are seeing the selection. A mixed indicator is the honest answer, and it is what every other suite shows |
| The bar re-derives per edit; the panel accumulates | one discipline for both | it breaks one of the two use cases. The bar is for quick successive edits, so each must start from what is on screen now, or the second edit undoes the first; the panel is for a considered pass over one selection, where a working copy is what the user expects |
| The text colour is separate from the fill | reusing the fill for the text | on a note the fill is the card and the body is drawn over it, so one field cannot be both |
| The fill and the border are optional | a colour with a zero alpha | "no fill" is a real style on a whiteboard -- an outline over something else -- and a sentinel colour cannot express it |
| The surfaces hold no style | caching the model across selections | two copies of the selection's look drift the moment a remote change arrives, and the drift shows as a control that disagrees with the board |
| The lock is enforced by the engine | filtering the locked members before sending | the surfaces would each need the rule, and a third surface would need it again. One refusal, at the write |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A selection past the edge of the screen is read wrong** (`TOILE-UI-070`). Both models resolve the selected
  identities against the culled visible set, so an off-screen member contributes neither its value nor its
  difference. The panel can show a fill as shared when it is not, and an edit then makes it shared for real.
  Same class as the select-all defect (`TOILE-UI-033`), same fix -- the value has to come from the engine.
* **A layout change resets every style** (`TOILE-UI-071`). The value is decoded whole, and a decode failure
  falls back to the default. Adding or removing a field makes every element already drawn go white. The
  format's answer is the schema version the application does not carry (`TOILE-027`, `TOILE-028`). This
  document's answer is that the value needs a version of its own before a first release.
* **Formatting is per element, never per range.** The text controls on both surfaces look like a text toolbar,
  and they write the element's style. So a word inside a sentence cannot be emphasized (`TOILE-UI-065`).
* **A connector has no style to edit** (`TOILE-046`). Both surfaces address elements only, and a
  connector cannot be selected anyway.
* **The bar's hide during a gesture has no test** (`TOILE-UI-170`). It hides while a drag, a resize or a
  rotate is in flight, so it does not follow the preview. The view decides that at publish time, and that seam
  is wasm-only, so only the empty and unresolved halves are driven.
* **The bar reports one field of eight** (`TOILE-UI-164`). It folds a common fill and nothing else. The text
  colour, the family, the size and the four decorations are drawn from the first resolved member, as though
  the whole selection carried them. The panel, which folds all eleven, is the only honest surface.
* **A restyle overwrites attributes no surface shows** (`TOILE-UI-174`). The corner radius, the shadow and the
  exact weight ride in the value, tracked by no flag and offered by no control. Restyling a mixed selection
  gives every member the first one's. Either the flags cover the whole value, or the write covers only the
  fields a surface can show.
* **A negative size becomes a different feature** (`TOILE-UI-180`). The bar clamps a size to zero, and zero is
  the sentinel that makes a note's body track the card. A number the control cannot mean turns into "fit to
  the card" rather than being refused.
* **No test drives an edit through either surface.** The fold is covered eleven ways; the write is covered by
  the one-field mapping alone. Four things are read from the code (`TOILE-UI-172`, `TOILE-UI-173`,
  `TOILE-UI-175`, `TOILE-UI-179`): what a panel edit leaves alone, and what a mixed field becomes. Also
  whether a restyle undoes as one step, and whether two bar edits compose.
* **Nothing bounds a thickness or a shadow.** The panel accepts what the control offers, and no rule
  caps a border width or a shadow extent against the element it decorates.
