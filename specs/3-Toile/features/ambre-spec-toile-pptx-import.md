# AMBRE -- FEATURE: TOILE POWERPOINT IMPORT

**Type:** Vertical (feature)

> A user receives a PowerPoint deck and opens it on the board. Each slide becomes a frame of the
> deck, with its notes, and looks as PowerPoint shows it. The report names what the board could not
> hold.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `TOILE-IO`, shared with
`features/ambre-spec-toile-import-export.md`.

### 1.1 The file

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-IO-146` | `IO` | The reader MUST read one slide per slide the presentation lists, in its order, at the slide size it declares. | this spec section 3 | `ambre_pptx::fixtures::every_slide_reads_in_deck_order_with_its_size` -- a file python-pptx wrote and LibreOffice's save of it | none | verified |
| `TOILE-IO-151` | `IO` | A placeholder of a slide MUST take the box and the text style its layout and its master give it. | this spec section 3 | `ambre_pptx::fixtures::a_placeholder_takes_the_box_and_the_style_its_layout_and_master_give_it` | none | verified |
| `TOILE-IO-152` | `IO` | A shape the master or the layout draws MUST land on every slide that shows it. | this spec section 3 | `ambre_pptx::fixtures::a_shape_of_the_master_lands_on_every_slide` | none | verified |
| `TOILE-IO-153` | `IO` | A slide MUST keep the background its chain gives it. | this spec section 3 | `ambre_pptx::fixtures::a_slide_background_reaches_the_slide`; `ambre_toile::logic::interop::deck_import::tests::each_slide_is_a_frame_side_by_side_with_its_background_as_its_fill` | none | verified |
| `TOILE-IO-154` | `IO` | A text MUST land as text a user edits, with its size, its ink and its emphasis. | this spec section 3 | `ambre_pptx::fixtures::a_text_box_keeps_its_own_size_ink_and_emphasis`; `ambre_toile::logic::interop::deck_import::tests::a_painted_shape_keeps_its_text_as_a_text_element_above_it`; `::a_text_is_drawn_at_its_slide_size_in_world_units` | none | verified |
| `TOILE-IO-166` | `IO` | A deck the writer of `ambre-pptx` writes MUST read back item by item as it left. | this spec section 3 | `ambre_pptx::read::tests::a_written_deck_reads_back_as_it_left` | none | verified |
| `TOILE-IO-172` | `IO` | Each PowerPoint input fixture, imported, exported and imported again, MUST land as the same deck: frames, titles, notes, sizes, items and bound connectors. | this spec section 3; `TOILE-IO-166`, `TOILE-IO-173` | `ambre_toile::logic::state::import::pptx_tests::every_deck_fixture_lands_the_same_deck_after_an_export_and_an_import` -- the 2 fixtures of the codec, each frame read from the board in paint order, never from the export map | none | verified |

### 1.2 Open the deck on the board

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-IO-147` | `POST` | Each slide MUST land as a frame at the end of the deck, in file order, named by its title. | this spec section 3 | `ambre_toile::logic::state::import::pptx_tests::each_slide_lands_as_a_frame_of_the_deck_in_order` | none | verified |
| `TOILE-IO-148` | `POST` | The speaker notes of a slide MUST reach the notes of its frame, line by line. | this spec section 3 | `ambre_pptx::fixtures::speaker_notes_reach_their_slide_line_by_line`; `ambre_toile::logic::state::import::pptx_tests::the_notes_of_each_slide_reach_its_frame` | none | verified |
| `TOILE-IO-149` | `POST` | A frame MUST hold every item its slide shows. | this spec section 3 | `ambre_toile::logic::state::import::pptx_tests::each_frame_holds_what_its_slide_shows` | none | verified |
| `TOILE-IO-150` | `POST` | An item MUST keep its place on the slide and its turn, a member of a group included. | this spec section 3 | `ambre_pptx::fixtures::a_group_lands_ungrouped_with_its_members_in_place`; `ambre_toile::logic::interop::deck_import::tests::a_turned_item_keeps_its_turn` | none | verified |
| `TOILE-IO-155` | `POST` | A connector bound to two shapes MUST land bound to the same two shapes. | this spec section 3 | `ambre_pptx::fixtures::a_connector_holds_on_to_the_shapes_it_joins`; `ambre_toile::logic::interop::deck_import::tests::a_painted_shape_keeps_its_text_as_a_text_element_above_it` | none | verified |
| `TOILE-IO-156` | `POST` | A PNG or a JPEG picture MUST land as a picture, and a video as its poster frame. | this spec section 3 | `ambre_pptx::fixtures::png_and_jpeg_pictures_land_and_a_video_lands_as_its_poster` | none | verified |
| `TOILE-IO-157` | `POST` | A picture that several slides show MUST land as one blob. | this spec section 3; `features/ambre-spec-toile-media.md section 1` | `ambre_toile::logic::interop::deck_import::tests::a_picture_on_every_slide_is_one_blob` | none | verified |
| `TOILE-IO-158` | `POST` | A SmartArt diagram, a chart, a table, an animation or a video MUST land as a picture or be named in the report. | this spec section 3; `INTEROP-003` | `ambre_pptx::fixtures::what_the_deck_cannot_hold_is_named_in_the_report`; `ambre_toile::logic::state::import::pptx_tests::the_report_names_what_the_board_could_not_hold` | none | verified |
| `TOILE-IO-159` | `POST` | A slide transition MUST fall back to the transition of the deck, and the report MUST name it as approximated. | this spec section 3; `features/ambre-spec-toile-presentation.md section 3` (`TOILE-FEAT-028`) | `ambre_pptx::fixtures::what_the_deck_cannot_hold_is_named_in_the_report`; structural -- the deck model holds no transition per slide and the frame plays the one the deck configures | none | verified |
| `TOILE-IO-160` | `POST` | Each construct the reader names MUST reach the report the user reads. | this spec section 3; `TOILE-IO-022` | `ambre_toile::logic::interop::deck_import::tests::the_report_of_the_reader_joins_the_report_of_the_map` | none | verified |
| `TOILE-IO-168` | `FAIL` | Given a picture the board cannot decode, the import MUST land nothing in its place and name it in the report. | this spec section 3; `INTEROP-003` | `ambre_pptx::read::tests::a_picture_of_another_encoding_is_named_as_dropped`; `ambre_toile::logic::interop::deck_import::tests::a_jpeg_without_its_thumbnail_is_named_as_dropped` | none | verified |
| `TOILE-IO-169` | `POST` | A connector with a free end MUST land as a line between its two ends, named as approximated. | this spec section 3 | `ambre_toile::logic::interop::deck_import::tests::a_connector_with_a_free_end_lands_as_a_line` | none | verified |
| `TOILE-IO-170` | `POST` | A text centred in its box, or anchored below its top, MUST be named as approximated. | this spec section 3 | `ambre_toile::logic::interop::deck_import::tests::a_painted_shape_keeps_its_text_as_a_text_element_above_it` | none | verified |
| `TOILE-IO-164` | `POST` | The whole deck MUST be one undo step. | this spec section 3; `TOILE-IO-001` | `ambre_toile::logic::state::import::pptx_tests::one_undo_takes_the_whole_deck_back` | none | verified |
| `TOILE-IO-165` | `POST` | The deck MUST land where the user dropped the file. | this spec section 4; `TOILE-IO-023` | `ambre_toile::logic::state::import::pptx_tests::the_deck_lands_on_the_drop_point` | none | verified |
| `TOILE-IO-167` | `POST` | The import picker and a drop MUST accept a PowerPoint file. | this spec section 4; `TOILE-IO-098` | `ambre_toile::ui::interop::tests::every_format_a_converter_reads_is_accepted`; `::the_picker_offers_only_what_the_gate_accepts` | none | verified |
| `TOILE-IO-161` | `FAIL` | Given a file the reader cannot read, the import MUST create nothing and leave the board as it was. | this spec section 3; `TOILE-IO-073` | `ambre_toile::logic::state::import::pptx_tests::a_broken_file_imports_nothing`; `ambre_pptx::read::tests::bytes_that_are_no_package_are_refused`; `::a_package_of_another_document_is_refused`; `::a_presentation_of_no_slide_is_refused` | none | verified |
| `TOILE-IO-163` | `FAIL` | Given a part that declares a document type or nests past the bound, the reader MUST refuse the file whole. | this spec section 3; `INTEROP-002` | `ambre_pptx::read::tests::a_part_that_declares_a_document_type_is_refused`; `::a_part_nested_past_the_bound_is_refused`; `::malformed_parts_read_to_an_error` | none | verified |
| `TOILE-IO-162` | `FAIL` | Given a deck whose masters, flattened onto every slide, pass the bound of the reader or of the board, the import MUST refuse the file whole. | this spec section 3; `ambre-spec-budgets.md section 6` | `ambre_pptx::read::tests::a_master_that_flattens_past_the_item_bound_is_refused`; `ambre_toile::logic::interop::deck_import::tests::a_deck_past_the_element_bound_is_refused_whole` | none | verified |
| `TOILE-IO-171` | `FRAME` | An import of a deck MUST NOT change an element, or a slide of the deck, the board already held. | this spec section 3; `TOILE-IO-071` | `ambre_toile::logic::state::import::pptx_tests::a_deck_leaves_the_board_it_lands_on_alone` | none | verified |

* **Budget.** none of its own yet: the import of a deck rides the sliced apply of
  `TOILE-IO-028`, and no row of `ambre-spec-budgets.md section 1` names a deck import. The reader
  joins the Office module, held to its row of `ambre-spec-budgets.md section 5`.
* **Boundary.** none of its own. The reader runs in the Office module, which imports no
  capability (`ARCH-079`), and each picture leaves as ciphertext (`TOILE-IO-068`).
* **Verification.** 27 `verified`. Two real writers hold the reader, and the board holds the deck
  through the Office engine the host links in place.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| opens a deck of three slides | watches | three frames, at the end of the deck (`TOILE-IO-147`) | the three frames, and the deck panel lists them | `logic::state::import::pptx_tests::each_slide_lands_as_a_frame_of_the_deck_in_order` |
| opens a deck, then undoes | watches | the board as before (`TOILE-IO-164`) | the frames leave in one step, on both boards | `logic::state::import::pptx_tests::one_undo_takes_the_whole_deck_back` |
| opens a broken file | watches | nothing (`TOILE-IO-161`) | an error on Alice's screen and no change on Bob's | `logic::state::import::pptx_tests::a_broken_file_imports_nothing` |

---

## 3. MODEL

**Containers.** This feature owns none. It writes the element, connector, layer, media and
presentation containers through the shared import path (`features/ambre-spec-toile-import-export.md
section 3`).

**Two halves, two crates.** The reader lives in `ambre-pptx`, behind its `codec` feature, and runs in
the Office converter module. It answers the deck model the writer reads, plus a report of its own.
The map onto the board lives in `ambre-toile`, beside the PowerPoint export. An OpenDocument reader
answers the same deck and the same report, so it shares the map whole.

```rust
// ambre_pptx, behind the `codec` feature
pub fn read(bytes: &[u8]) -> Result<ImportedDeck, PptxError>;
pub struct ImportedDeck { pub deck: Deck, pub report: DeckReport }

// ambre_toile::logic::interop
pub fn import_deck(read: &ImportedDeck, thumbs: &[Option<Pixels>], format: FileFormat,
                   limits: &Limits) -> Result<ImportBatch, ImportError>;
```

**What each construct becomes:**

| In the file | On the board | Outcome in the report |
|---|---|---|
| a slide | a frame of the slide size, at the end of the deck, its background as its fill | carried |
| the notes page | the notes of its frame | carried |
| a placeholder | its text, in the box and the style of its layout and master | carried |
| a shape of the master or the layout | the same shape on each slide that shows it | carried |
| a rectangle, an ellipse, a triangle, a line, a rounded rectangle | the shape of the same name | carried |
| a custom geometry | a vector path | carried |
| the text of a shape | a text element above the shape | carried |
| a picture, PNG or JPEG | a picture, one blob per distinct picture | carried |
| a connector bound to two shapes | a connector between them | carried |
| a group | its members, ungrouped | approximated |
| a text of several styles | the style of its first run | approximated |
| a centred or a middle text | a text at the top left | approximated |
| a gradient, a pattern or a picture fill | its first colour | approximated |
| a preset outline the model lacks | its box | approximated |
| a video, an embedded object | its poster frame, its preview | approximated |
| a transition | the transition of the deck | approximated |
| a connector with a free end | a line | approximated |
| SmartArt, a chart, a table | the picture of its fallback, or nothing | approximated or dropped |
| an animation, an audio clip, an ink drawing | nothing | dropped |
| a picture of another encoding | nothing | dropped |

**Steps, on an import:**

1. the Office module opens the package under the bounds of `ambre-opc`, and reads the slide list
   of the presentation part;
2. per slide, it follows the layout, then the master, then the theme. It resolves each colour
   through the colour map of the master and the scheme of the theme;
3. it lays the shapes of the master, then of the layout, then of the slide, unless one of them hides
   the master's. A placeholder takes each property from the first level of its chain that sets it;
4. it reads the notes page, and records each construct the deck cannot hold. It decodes the
   thumbnail of each JPEG picture, since the Main instance links no JPEG decoder (`ARCH-078`);
5. the map lays the frames side by side, one point to one world unit, and turns each item into a
   create. The frames, their members, their titles and their notes are units of the sliced apply.

**Code:**

* `crates/ambre-pptx/src/read/mod.rs` -- the reader and its flattening;
* `crates/ambre-pptx/src/read/style.rs` -- the theme, the colour map and the text style chain;
* `crates/ambre-pptx/src/read/xml.rs` -- the XML tree of one part;
* `crates/ambre-pptx/src/report.rs` -- the report every presentation reader writes;
* `crates/ambre-convert/src/engine/mod.rs` -- the `ReadPptx` conversion of the Office module;
* `crates/ambre-toile/src/logic/interop/deck_import.rs` -- the map onto the board;
* `crates/ambre-toile/src/logic/state/import.rs` -- the frame units of the apply.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| a PowerPoint file dropped on the board | the Office module reads it, and the deck lands at the drop point (`TOILE-IO-165`) |
| the File menu's import entry | the picker offers `.pptx`, and the deck lands in the middle of the viewport (`TOILE-IO-167`) |
| the first import of a session | waits for the Office module, which the Sync worker fetches once (`ARCH-081`) |
| an import that landed | the shared notice says how many elements arrived, and what was dropped (`TOILE-IO-022`) |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The reader flattens the master onto each slide | a master of the board, shared by its frames | the board has no master. A frame holds its own elements, so an inherited shape must exist on each frame to be seen there |
| The reader answers the deck model the writer reads | a reader model of its own | the OpenDocument reader would need a third model, and the map onto the board could not be shared |
| A background becomes the fill of the frame | a rectangle under every item | a rectangle that fills the frame takes every click on the slide, and the frame already paints a fill |
| The Office module decodes the JPEG thumbnails | a decode on the converter thread per picture | each picture would cost a round trip of its own, and the import would hold until every answer came back |
| One point is one world unit | the slide fitted to a fixed frame width | the export writes a frame at one point per world unit (`TOILE-IO-173`), so the round trip keeps its geometry |
| Each input fixture is imported, exported and imported again, and the two decks are compared as stored data (`TOILE-IO-172`) | comparing the bytes of the file read with the bytes of the file written | the zip order, the part names, the ids and the namespace prefixes differ with no defect. A byte comparison fails on each of them and holds nothing a user reads |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A group lands ungrouped.** The board rebuilds groups for SVG, and the reader flattens them.
* **A text carries one style.** A body of several runs takes the first run's style, and the
  alignment of a text is lost, as the board's text has none.
* **A table, a chart and a SmartArt diagram land as nothing** unless the file carries a fallback
  picture. Each is named.
* **A picture crop is not read.** The whole picture fills the box of the cropped one.
* **The deck import has no budget row.** No bench reads it.
* **The reader owns a third XML tree.** The Word and OpenDocument codecs each own one.
