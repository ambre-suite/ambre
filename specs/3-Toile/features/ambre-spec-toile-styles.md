# AMBRE -- FEATURE: ELEMENT STYLES

**Type:** Vertical (feature)
**Implementation:** partial -- editing ships; the bar reports mixed, a restyle keeps hidden fields.

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

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-067` | `INV` | A style edit MUST be sent as one whole value per element, never one per attribute. | this spec section 3; `TOILE-018` | structural -- the only command that changes style carries a whole value, and the format has one register per element, so no attribute has a register of its own | none | verified |
| `TOILE-163` | `CONC` | Two users restyling the same element MUST both end on one of the two styles, never on a mixture. | this spec section 2; `TOILE-018` | `ambre_toile::logic::convergence_tests::two_peers_converge` -- it drives concurrent edits and reads the two peers in agreement | none | verified |
| `TOILE-UI-399` | `INV` | A text size MUST read as at most 4000 points, the presentation cap: a stored or received size past it reads as 4000. | this spec section 3; `CANVAS-088` | `ambre_bus::app::toile::tests::a_size_choice_past_the_cap_reads_as_the_cap` -- a million points and 4001 decode as 4000, and a size in range keeps its bytes; type-level -- `FontSizeChoice` holds a `FontSize`, itself at most 4000, so no local input names more | none | verified |
| `TOILE-UI-069` | `INV` | Neither surface MUST hold an authoritative style: both MUST re-derive from the selection and what is painted. | this spec section 3 | type-level -- both models are functions of the selection and the painted set returning a fresh value, so there is no field a style could survive a selection change in | none | verified |
| `TOILE-UI-066` | `OBS` | The panel MUST report a field that differs across the selection as mixed, rather than showing one member's value as shared. | this spec section 3 | `ambre_toile::ui::style::tests::differing_fill_flags_only_fill_mixed` -- the fill flag; `ambre_toile::ui::style::tests::differing_stroke_width_and_opacity_flag_both` -- two more; `ambre_toile::ui::style::tests::differing_text_decorations_flag_underline_and_strikethrough` -- the two decorations; `ambre_toile::ui::style::tests::a_mixed_field_stays_mixed` -- a later agreeing pair never clears the mix | none | verified |
| `TOILE-UI-161` | `OBS` | The panel MUST report a field that agrees across the selection as a value, never as mixed. | this spec section 3 | `ambre_toile::ui::style::tests::differing_fill_flags_only_fill_mixed` -- the three fields that agree stay unflagged; `ambre_toile::ui::style::tests::identical_elements_are_not_mixed` -- no flag at all on a uniform pair; `ambre_toile::ui::style::tests::two_elements_on_the_default_family_are_not_mixed` -- a fresh element and one the picker reset to the default agree | none | verified |
| `TOILE-UI-162` | `OBS` | A selection of one MUST show that element's style with nothing mixed. | this spec section 3 | `ambre_toile::ui::style::tests::single_element_shows_its_style_with_no_mixed` -- the base is the element's own style and no flag is raised | none | verified |
| `TOILE-UI-397` | `OBS` | An element that names no family MUST show the picker's "Default" entry, and an element that names a family MUST show that family. | this spec section 3 | `ambre_toile::ui::style::tests::an_element_with_no_family_shows_the_default_entry` -- a fresh element and the "Default" pick each read the "Default" label; `::an_element_set_to_sans_shows_sans` -- an explicit Sans reads "Sans", never "Default" | none | verified |
| `TOILE-UI-163` | `OBS` | The panel MUST show nothing when no selected element resolves. | this spec section 3 | `ambre_toile::ui::style::tests::empty_selection_has_no_model` -- no model for an empty selection; `ambre_toile::ui::style::tests::unresolved_selection_has_no_model` -- none for an id the painted set does not hold | none | verified |
| `TOILE-UI-164` | `OBS` | The bar MUST report every field it edits as mixed when it differs across the selection. | this spec section 6 | `ambre_toile::ui::floatbar::tests::bar_reports_every_edited_field_mixed` -- it folds two elements differing on the colour, the family, the size and the four decorations and reads every flag back set | none | verified |
| `TOILE-UI-165` | `OBS` | The bar MUST show the selection's fill when every member shares it. | this spec section 4 | `ambre_toile::ui::floatbar::tests::common_fill_survives_when_shared` -- the shared colour reaches the swatch | none | verified |
| `TOILE-UI-166` | `OBS` | The bar MUST show no fill when the selection's fills differ. | this spec section 4 | `ambre_toile::ui::floatbar::tests::common_fill_is_none_when_mixed` -- the swatch is empty for two different fills | none | verified |
| `TOILE-UI-068` | `OBS` | The bar MUST sit at the midpoint of the selection's top edge. | this spec section 4 | `ambre_toile::ui::floatbar::tests::anchor_is_top_edge_midpoint_in_screen_space` -- both coordinates against a hand-computed midpoint | none | verified |
| `TOILE-UI-167` | `OBS` | A multi-selection MUST anchor the bar to the union of its members' boxes. | this spec section 4 | `ambre_toile::ui::floatbar::tests::union_bounds_anchor_a_multi_selection` -- the anchor of a two-element selection is the union's top-edge midpoint | none | verified |
| `TOILE-UI-168` | `OBS` | The anchor MUST be in the units the page positions with, not the units the canvas draws in. | this spec section 4 | `ambre_toile::ui::floatbar::tests::the_canvas_offset_applies_to_the_anchor` -- the anchor after the canvas offset, the canvas drawing in CSS pixels (`CANVAS-032`) | none | verified |
| `TOILE-UI-169` | `OBS` | The bar MUST hide when no selected element resolves. | this spec section 4 | `ambre_toile::ui::floatbar::tests::empty_selection_hides_bar` -- nothing is published for an empty selection; `ambre_toile::ui::floatbar::tests::unresolved_selection_hides_bar` -- none for an id the painted set does not hold | none | verified |
| `TOILE-UI-170` | `OBS` | The bar MUST hide while a move, a resize or a rotate is in flight, so it does not follow the preview. | this spec section 6 | `ambre_toile::ui::view::view_tests::the_bar_hides_during_a_move` -- it reads the bar the view would publish before, during and after a move drag | none | verified |
| `TOILE-UI-171` | `OBS` | The bar MUST offer group, ungroup, lock and unlock according to what is selected. | this spec section 4 | `ambre_toile::ui::floatbar::tests::flags_track_group_and_lock_state` -- the group flag and both lock flags on a mixed pair | none | verified |
| `TOILE-UI-070` | `OBS` | Both surfaces MUST read the whole selection, not the part on screen. | this spec section 6 | `ambre_toile::ui::style::tests::off_screen_selection_contributes_its_style` -- an off-screen member's differing fill still flags mixed; `ambre_toile::logic::convergence_tests::selection_data_includes_offscreen_elements` -- Logic answers the whole selection | none | verified |
| `TOILE-UI-071` | `IO` | A style written by an older build MUST survive a change to the value's layout. | this spec section 6; `SCHEMA-018` | `ambre_bus::app::toile::tests::a_style_round_trips_through_its_version` -- the register carries its layout version and a style reads back whole; `::a_style_from_an_unknown_layout_is_reported` -- a layout this build does not know is reported, never reset | none | verified |

### 1.2 Restyle the selection from the panel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-172` | `POST` | A panel edit MUST leave every field it did not edit as the selection had it. | this spec section 3 | `ambre_toile::ui::floatbar::tests::a_panel_edit_keeps_every_field_it_did_not_edit` -- each panel control writes its working copy through the one-field edit, and the test reads every other field back, a family the picker cannot name included | none | verified |
| `TOILE-UI-173` | `POST` | A field the panel reported as mixed MUST become uniform after any edit. | this spec section 2 | `ambre_toile::ui::view::view_tests::a_panel_restyle_names_every_member_and_keeps_the_selection` -- a mixed width reads as mixed, and the edit sends one whole value for every member | none | verified |
| `TOILE-UI-174` | `POST` | A restyle MUST NOT overwrite an attribute the surface does not show. | this spec section 6 | `ambre_toile::logic::state::elements::inner_tests::a_restyle_preserves_the_fields_no_surface_shows` -- it restyles two members seeded with differing corner, shadow and weight and reads each back keeping its own | none | verified |
| `TOILE-UI-175` | `POST` | A restyle over a whole selection MUST undo as one step. | this spec section 3 | `ambre_toile::logic::convergence_tests::a_restyle_over_a_selection_is_one_undo_item` | none | verified |
| `TOILE-UI-176` | `FRAME` | A restyle MUST NOT change what is selected. | this spec section 3 | `ambre_toile::ui::view::view_tests::a_panel_restyle_names_every_member_and_keeps_the_selection` | none | verified |
| `TOILE-UI-182` | `FRAME` | A restyle MUST NOT drop the locked members before it is sent: the refusal belongs to the write (`TOILE-033`). | this spec section 5 | `ambre_toile::ui::view::view_tests::a_panel_restyle_names_every_member_and_keeps_the_selection` -- a locked member is named in the command the view builds | none | verified |

### 1.3 Change one field from the floating bar

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-019` | `POST` | A bar edit MUST overwrite exactly the field it carries. | this spec section 3 | `ambre_toile::ui::floatbar::tests::style_edit_touches_only_its_field` -- a decoration toggle leaves the other three, the fill, the colour and the size intact, and a fill edit leaves the whole text style intact | none | verified |
| `TOILE-UI-177` | `POST` | The bold toggle MUST write the two weights the renderer reads. | this spec section 3 | `ambre_toile::ui::floatbar::tests::style_edit_field_mappings` -- both weights off the toggle | none | verified |
| `TOILE-UI-178` | `POST` | Resetting the family MUST write a style that names no family and draws in the default face. | this spec section 3 | `ambre_toile::ui::floatbar::tests::style_edit_field_mappings` -- the reset writes no family and resolves to the default face, and a chosen family, Sans included, writes its own | none | verified |
| `TOILE-UI-179` | `POST` | Each bar edit MUST start from the selection's style as it stands, so two quick edits compose. | this spec section 3 | `ambre_toile::ui::view::view_tests::two_bar_edits_compose_and_keep_the_selection` -- the second edit carries the first, echoed back with the selection | none | verified |
| `TOILE-UI-180` | `FAIL` | A size the control cannot mean MUST be refused, not turned into the automatic size. | this spec section 6 | `ambre_toile::ui::floatbar::tests::style_edit_field_mappings` -- a negative size does not decode and changes nothing, and Auto lands as the automatic size | none | verified |
| `TOILE-UI-181` | `FRAME` | A bar edit MUST NOT change what is selected. | this spec section 3 | `ambre_toile::ui::view::view_tests::two_bar_edits_compose_and_keep_the_selection` | none | verified |
| `TOILE-UI-419` | `POST` | The floating bar's Delete button MUST report the command id the context menu's Delete entry carries. | this spec section 3 | `ambre_toile::tests::chrome_ui::selection_toolbar_delete_fires_command` -- wasm tier: a click on the mounted bar's Delete button reports `menu::DELETE` | none | verified |

* **Budget.** the view-state update under 0.5 ms, one row of `ambre-spec-budgets.md section 2`. Both models
  are a fold over the selection, and the bar's screen position is recomputed every frame inside that figure.
  Neither has a bench.
* **Boundary.** none: both surfaces are chrome on the UI thread, where `UI-THREAD-004` bans a network,
  crypto or persistence path. The value they emit enters the document and leaves as bytes the Sync
  worker seals (`ZK-001`).
* **Verification.** 33 `verified`. What holds is the read: the base value, the flags, the anchor,
  the one-field mapping, and a selection read whole (`TOILE-UI-070`). The write holds: each surface
  writes one field, a restyle undoes as one step, and two bar edits compose. A style carries its own
  layout version (`TOILE-UI-071`). The lock itself is `TOILE-033`.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| makes a shape red | makes the same shape blue | one whole style, never a red border on a blue fill | the same colour on both screens (`TOILE-018`) | `logic::convergence_tests::two_peers_converge` |
| makes a shape bold from the bar | changes its fill from the bar at the same moment | both edits, each having touched its own field | both changes, because each edit re-derived the rest live (`TOILE-UI-019`) | `ui::floatbar::tests::style_edit_touches_only_its_field` |
| selects two shapes of different colours | watches | nothing: a selection is not shared | a mixed indicator on the fill, and the shared fields as values (`TOILE-UI-066`) | `ui::style::tests::differing_fill_flags_only_fill_mixed` |
| restyles a selection whose members differ | watches | one style on every member | every selected element identical afterwards, including the fields she did not touch (`TOILE-UI-173`) | none: no test drives an edit through either surface |
| restyles two cards differing only in corner radius | watches | one style on every member, the corner kept | each card keeps its own corner radius (`TOILE-UI-174`) | `elements::inner_tests::a_restyle_preserves_the_fields_no_surface_shows` |

---

## 3. MODEL

**Containers.** One binary field per element, holding the whole style
(`ambre-spec-toile-schema.md section 3`). No attribute has a container of its own, which is
`TOILE-018`.

**The value** (`ambre-bus`). Fill and border are optional, so "no fill" is representable. The text colour is
separate from the fill, because on a note the fill is the card:

```rust
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct ElementStyle {
    pub fill:       Option<Rgba>,  // None = no fill
    pub stroke:     Option<Rgba>,
    pub stroke_w:   Px<StoredBoard>,
    pub opacity:    Bounded<Opacity>,    // 0.0 ..= 1.0
    pub corner:     Px<StoredBoard>,   // corner radius
    pub font:       FontSpec,      // family, size (points), weight, italic, underline, strikethrough
    pub shadow:     Option<Shadow>,
    pub text_color: Option<Rgba>,  // None = the kind's default ink
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
flag, and the weight is folded to bold-or-not rather than compared. The write preserves those three: it
merges the shown fields over each element, keeping the element's own corner radius, shadow and non-bold
weight (`TOILE-UI-174`).

**Steps, when either surface edits a field:**

1. resolve the selected identities against what is painted, and stop when none resolve;
2. fold their styles into a base and a set of difference flags (`TOILE-UI-066`);
3. apply the edited field over the base;
4. send one whole value over every selected identity, debounced on the panel side;
5. the locked members are refused by the engine, not filtered by the surface (`TOILE-UI-182`).

**Per-kind defaults.** Birth seeds a per-kind style rather than a bare default. Everything is white with no
border except a note, which is born a plain yellow card with no border and square corners. Its automatic
size makes its body track the card (`features/ambre-spec-toile-elements.md`,
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

**What the bar folds.** Every field it edits: a colour every member shares reaches the swatch, and a
divergence empties it (`TOILE-UI-165`, `TOILE-UI-166`). A field that differs across the selection carries
a mixed flag (`TOILE-UI-164`), so the bar never shows one member's value as the selection's.

**The size control's "Auto" entry** maps to the automatic size, and choosing a number sets an explicit
size that overrides it (`features/ambre-spec-toile-text-editing.md`). A free text has no card, so it
draws an automatic size at the default 12 points. The register stores automatic as zero points, and a
negative number is refused, not clamped to it (`TOILE-UI-180`). The style holds points, the unit of
the control. The board lays them out at 96/72 world px per point, as Trame and Prose do.

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

* **Formatting is per element, never per range.** The text controls on both surfaces look like a text toolbar,
  and they write the element's style. So a word inside a sentence cannot be emphasized (`TOILE-UI-065`).
* **A connector has no style to edit** (`TOILE-046`). Both surfaces address elements only, and a
  connector cannot be selected anyway.
* **Nothing bounds a thickness or a shadow.** The panel accepts what the control offers, and no rule
  caps a border width or a shadow extent against the element it decorates.
