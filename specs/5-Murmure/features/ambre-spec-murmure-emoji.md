# AMBRE -- FEATURE: MURMURE EMOJI
**Type:** Vertical (feature)
**Implementation:** partial

> A member writes an emoji in a message, or reacts with one, from a picker. The emoji are drawn in
> colour, and the font that carries them loads only when first needed.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-137` | `OBS` | A Unicode emoji MUST be drawn in colour, in a message and in a reaction. | this spec section 3; `CANVAS-019` | `ambre_murmure::paint::tests::the_reactions_paint_as_chips_under_the_body` -- a reaction paints as a chip with its emoji; `ambre_murmure::paint::tests::the_emoji_face_is_asked_for_on_first_use_only` -- a body and a chip shape through the canvas text path, which draws colour by `CANVAS-019` | none | verified |
| `MURMURE-138` | `INV` | The colour emoji font MUST load on first use, never at start. | this spec section 3; `ambre-spec-budgets.md section 6` | `ambre_murmure::paint::tests::the_emoji_face_is_asked_for_on_first_use_only` -- a list with no emoji asks for nothing, an emoji in a body or on a chip asks once | none | verified |
| `MURMURE-374` | `INV` | A custom emoji MUST belong to one space, listed in its space document. | this spec section 3; `ambre-spec-murmure-schema.md section 3` | not built | unbuilt -- lands with the `Custom emoji` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-375` | `INV` | The image of a custom emoji MUST be sealed under the key of its space. | this spec section 3; `ambre-spec-media.md` | not built | unbuilt -- lands with the `Custom emoji` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-569` | `INV` | A custom emoji in a body MUST be an `emoji` mark beside the style marks, carrying its `CustomEmojiId`. | this spec section 3; `MURMURE-194`; `ambre-spec-murmure-schema.md section 3` | not built | unbuilt -- lands with the `Custom emoji` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Pick an emoji

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-139` | `POST` | An emoji picked from the composer MUST be inserted at the caret. | this spec section 3 | `ambre_murmure::view::tests::a_picked_emoji_goes_in_at_the_caret` -- the chord asks for the picker, and a pick lands at the caret of its region alone; `ambre_murmure::tests::chrome_relay::a_pick_reacts_to_the_message_or_goes_to_the_composer` -- browser tier, the region takes the pick once | none | verified |
| `MURMURE-140` | `POST` | An emoji picked from a message MUST be added as a reaction. | this spec section 3; `features/ambre-spec-murmure-reactions.md` | `ambre_murmure::tests::chrome_relay::a_pick_reacts_to_the_message_or_goes_to_the_composer` -- browser tier, the action opens the picker and a pick sends the reaction to the message; `ambre_murmure::view::tests::a_click_on_a_chip_adds_or_takes_away_the_members_reaction` -- a chip adds the member's reaction, or takes it away | none | verified |
| `MURMURE-141` | `FRAME` | The picker MUST NOT send a request outside the client. | this spec section 3; `ZK-007` | `ambre_murmure::tests::chrome_relay::a_pick_reacts_to_the_message_or_goes_to_the_composer` -- browser tier, no resource is fetched while the picker opens, searches and picks; `ambre_murmure::emoji::tests::every_emoji_of_the_table_is_one_reaction_key` -- the table is compiled in | none | verified |

### 1.4 Add a custom emoji

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-376` | `POST` | Adding a custom emoji MUST be the act of a space admin, its image uploaded through the shared media flow. | this spec section 3; `MURMURE-032` | not built | unbuilt -- lands with the `Custom emoji` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-377` | `FAIL` | Given a name the space already uses, the client MUST refuse the new emoji. | this spec section 3 | not built | unbuilt -- lands with the `Custom emoji` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-378` | `FRAME` | Adding a custom emoji MUST NOT change a message. | this spec section 3 | not built | unbuilt -- lands with the `Custom emoji` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.5 Use a custom emoji

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-379` | `POST` | A custom emoji MUST be offered in the channels of its space alone. | this spec section 3 | not built | unbuilt -- lands with the `Custom emoji` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-380` | `FAIL` | Given a custom emoji the reader cannot resolve, the client MUST draw its name as text. | this spec section 3 | not built | unbuilt -- lands with the `Custom emoji` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-381` | `FRAME` | Using a custom emoji MUST NOT copy its image into the message. | this spec section 3; `MURMURE-017` | not built | unbuilt -- lands with the `Custom emoji` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the colour emoji font row of `ambre-spec-budgets.md section 6`.
* **Boundary.** none: an emoji is text inside a sealed message, and the font ships with the
  client.
* **Verification.** 5 `verified`, 9 `unimplemented`. The picker, the chips and the lazy face are
  built; the custom emoji are not.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The channel holds | Each user sees | Test |
|---|---|---|---|---|
| types an emoji | watches | one message | the emoji in colour, on both screens (`MURMURE-137`) | `ambre_murmure::paint::tests::the_emoji_face_is_asked_for_on_first_use_only` |
| picks an emoji from a message | watches | one reaction | the reaction, in colour | `ambre_murmure::tests::chrome_relay::a_pick_reacts_to_the_message_or_goes_to_the_composer` |

---

## 3. MODEL

**CRDT containers.** none of its own: an emoji is text in `content`, or a key in `reactions`.

**The font** is the colour emoji face of `ambre-spec-render-assets.md`, loaded on the first emoji
to draw (`CANVAS-019`). The picker is a panel of the shared shell (`CHROME-004`).

**Custom emoji belong to a space.** A space admin adds one: a name, and an image through the shared
media flow, sealed under the space key (`MURMURE-374` to `MURMURE-376`). The `emoji` map of the space
document lists them. A message holds the `emoji` mark with the identifier, never the image
(`MURMURE-381`). A reaction keys it as `ReactionEmoji::Custom`.

**Who can draw it.** Every member of the space reads the space document, so every member draws the
emoji. A guest, a direct message, or an emoji the admin removed later: the reader cannot resolve it,
and draws `:name:` as text (`MURMURE-380`).

**The picker** searches a table compiled into the client, by name (`MURMURE-141`). A pick for the
composer waits for its region, which takes it once at the caret; a pick for a message is the
reaction. A reaction paints as a chip under the body, the member's own in the accent. A click on a
chip adds the member's reaction, or takes it away (`MURMURE-096`).

**Code.** `ambre-murmure`: `emoji` (the table), `chrome::EmojiPicker`, `paint` (the chips) and
`view` (the chord and the chip click).

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| Ctrl and `;` in the composer | the picker, then the emoji at the caret |
| "Add reaction" in the actions of a message | the picker, then the emoji as a reaction |
| a click on a reaction chip | the member's reaction added, or taken away |
| `:` followed by a name in the composer | the matching emoji, offered in a list |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The font ships with the client | emoji images from a remote service | each emoji drawn would tell that service what the user reads |
| Loaded on first use | loaded at start | a font of about 10 MB would weigh on every start |
| Custom emoji belong to a space | custom emoji per user | a member's emoji would draw for nobody else, since nobody else holds the key of the image |
| An unresolved emoji draws its name | a broken image | the name still says what the author meant |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Custom emoji are not built.** Neither is the `:` list of the composer.
* **The composer has no emoji control on a phone.** The system keyboard writes emoji through the
  input pipe; the chord is for a keyboard.
* **The weight of the emoji face is not measured.** Its size decides whether the fonts row of
  `ambre-spec-budgets.md section 6` still holds.
