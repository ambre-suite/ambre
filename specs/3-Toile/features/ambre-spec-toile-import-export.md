# AMBRE -- FEATURE: TOILE IMPORT AND EXPORT

**Type:** Vertical (feature)
**Implementation:** partial -- a file comes on and the board goes out; the foreign formats and the
showcase bundle are not built.

> A user brings a drawing that came from somewhere else onto the board. They hand the board back as
> a file someone without Ambre can open. Every conversion runs on the user's own machine.

The native `.ambre` file is the only path that keeps everything. Every other format drops something,
and what it drops is the subject of this document. The deck's own page format is
`features/ambre-spec-toile-pdf-export.md`, and the pixels an imported picture leaves behind are
`features/ambre-spec-toile-media.md`.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-IO-068` | `INV` | A conversion MUST run on the user's own machine, never reaching the network. | README pillar "E2EE"; this spec section 5 | structural -- no interop module names a network client, and the one blob that leaves rides `ToileRequest::UploadMedia`, which Sync encrypts before it goes | none | verified |
| `TOILE-IO-010` | `IO` | An `.ambre` export and import round trip MUST preserve the document state, byte for byte on a bare board. | this spec section 3; `ambre-spec-docfile.md section 3` | `ambre_toile::logic::state::docfile::tests::round_trip_is_byte_identical_on_a_bare_document`; `ambre_toile::logic::state::docfile::tests::round_trip_preserves_the_document_state`; `ambre_toile::logic::state::docfile::tests::the_header_names_toile_and_the_document` | none | verified |
| `TOILE-IO-069` | `IO` | A board the export could not read MUST NOT become a file: a valid header over an empty payload MUST NOT be written. | this spec section 3 | none | fails -- a failed snapshot logs an error and writes the header alone, so the file opens as an empty board, section 6; lands with the `A board the export could not read becomes no file` Story (`ambre-roadmap.md` E7) | failing |

### 1.2 Bring a file onto the board

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-IO-020` | `POST` | A user MUST be able to bring a file onto the board, by a drop or by a picker. | this spec section 4 | qa-manual -- DOM glue held by using the application (`1-Foundations/ambre-spec-testing.md section 2`), two canvas listeners and one `<input type="file">`, whose every decidable part is a row of its own: what the board reads (`TOILE-IO-098`), where the file lands (`TOILE-IO-023`) and what the user is told (`TOILE-IO-022`) | none | qa-manual |
| `TOILE-IO-098` | `FAIL` | A file of a type no converter reads MUST be refused with a word, and MUST NOT be read into memory. | this spec section 4 | `ambre_toile::ui::interop::tests::a_format_no_converter_reads_is_refused`; `ambre_toile::ui::interop::tests::every_format_a_converter_reads_is_accepted`; `ambre_toile::ui::interop::tests::the_picker_offers_only_what_the_gate_accepts` -- it holds the picker's filter against the same gate, so a file the dialog offers is never one the board then drops | none | verified |
| `TOILE-IO-001` | `POST` | The whole file MUST become one undoable batch: one undo MUST take the import back. | this spec section 3 | `ambre_toile::logic::state::import::tests::import_is_one_undo_group` -- it undoes a five-element import in one step; `ambre_toile::logic::state::import::tests::import_rebuilds_nested_groups` -- a three-element one carrying two groups; `ambre_toile::logic::state::import::tests::svg_import_takes_the_fast_path` -- it reads the single delta | none | verified |
| `TOILE-IO-002` | `POST` | An import MUST NOT trust foreign geometry: every coordinate MUST be clamped to the world. | this spec section 3 | `ambre_toile::logic::interop::limits::tests::coord_clamp_bounds_and_sanitizes`; `ambre_toile::logic::interop::limits::tests::rect_clamp_keeps_non_negative_extent`; `ambre_toile::logic::interop::tests::coordinates_are_clamped_to_the_world_bound` | none | verified |
| `TOILE-IO-072` | `POST` | A coordinate that is not a number MUST become zero, never poison the geometry it lands in. | this spec section 3 | `ambre_toile::logic::interop::limits::tests::coord_clamp_bounds_and_sanitizes` -- it reads a NaN and an infinity both collapsing to zero | none | verified |
| `TOILE-IO-070` | `POST` | An import MUST say how many elements it created. | this spec section 3 | `ambre_toile::logic::state::import::tests::import_is_one_undo_group` -- it reads the count back against the file; `ambre_toile::logic::state::import::tests::import_rebuilds_nested_groups` -- it reads a count of the three elements and not of the two groups the import rebuilt | none | verified |
| `TOILE-IO-022` | `POST` | An import report MUST reach the user: a construct dropped in silence MUST NOT be the outcome. | this spec section 4 | `ambre_toile::tests::chrome_ui::an_import_report_reaches_the_user` -- it reads the toast a lossy import raises; `ambre_toile::tests::chrome_ui::a_refused_file_reaches_the_user` -- the one a file that never mapped raises; `ambre_toile::ui::interop::message_tests::a_clean_import_reports_its_count` -- the wording; `ambre_toile::ui::interop::message_tests::a_lossy_import_names_what_it_dropped`; `ambre_toile::ui::interop::message_tests::a_long_list_of_dropped_constructs_is_counted` | none | verified |
| `TOILE-IO-023` | `POST` | An imported file MUST land where the user put it, never at a fixed point of the world. | this spec section 4 | `ambre_toile::logic::state::import::tests::an_import_lands_on_the_drop_point`; `ambre_toile::logic::interop::tests::a_batch_lands_with_its_corner_on_the_drop_point` -- the placement itself; `ambre_toile::logic::interop::tests::placing_a_batch_that_creates_nothing_is_a_no_op` | none | verified |
| `TOILE-IO-028` | `POST` | A large import MUST NOT freeze the Logic thread: it MUST apply in bounded batches that yield. | this spec section 3; `ambre-spec-budgets.md` | none | fails -- the whole batch is applied in one synchronous pass, and no progress event exists, section 6; lands with the `Large-file import` Story (`ambre-roadmap.md` E6) | failing |
| `TOILE-IO-071` | `FRAME` | An import MUST NOT change an element the board already held. | this spec section 3 | none | unheld -- every test of the apply pass starts from an empty board, so nothing holds the rule on a populated one, section 6; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-IO-073` | `FAIL` | A file the converter cannot read MUST import nothing, and MUST leave the board as it was. | this spec section 3 | `ambre_toile::logic::state::import::tests::a_broken_file_fails_without_touching_the_board`; `ambre_toile::logic::interop::tests::malformed_svg_fails_to_parse` | none | verified |
| `TOILE-IO-074` | `FAIL` | A file past the count, the depth or the picture-size bound MUST be refused whole, never in part. | this spec section 3 | `ambre_toile::logic::interop::limits::tests::element_count_guard`; `ambre_toile::logic::interop::limits::tests::depth_guard`; `ambre_toile::logic::interop::limits::tests::image_dim_guard`; `ambre_toile::logic::interop::tests::too_many_elements_aborts_atomically`; `ambre_toile::logic::interop::tests::too_deep_nesting_aborts_atomically` | none | verified |
| `TOILE-IO-084` | `FAIL` | A file that maps to no element MUST be refused, never applied as an empty import. | this spec section 3 | `ambre_toile::logic::interop::tests::empty_svg_reports_empty` | none | verified |

### 1.3 Bring an SVG drawing onto the board

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-IO-003` | `POST` | Every shape the drawing carries MUST land as the same kind of element on the board. | this spec section 3 | `ambre_toile::logic::interop::tests::rect_maps_to_shape_rect`; `ambre_toile::logic::interop::tests::ellipse_maps_to_shape_path`; `ambre_toile::logic::interop::tests::path_maps_to_shape_path_with_geometry` | none | verified |
| `TOILE-IO-075` | `POST` | A text node MUST land as text the user can still edit, never as a picture of a word. | this spec section 3 | `ambre_toile::logic::interop::tests::text_maps_to_text_element_preserving_body` | none | verified |
| `TOILE-IO-076` | `POST` | Every node MUST keep the fill and the stroke the drawing gave it. | this spec section 3 | `ambre_toile::logic::interop::tests::rect_import_preserves_fill_and_stroke_style`; `ambre_toile::logic::interop::tests::unstyled_kinds_keep_the_default_style`; `ambre_toile::logic::state::import::tests::import_applies_styles_per_slot` | none | verified |
| `TOILE-IO-077` | `POST` | A group MUST come back as a group, nested as the drawing nested it, never flattened. | this spec section 3 | `ambre_toile::logic::interop::tests::nested_groups_are_rebuilt_innermost_first` -- it reads the two groups and the baked transforms; `ambre_toile::logic::state::import::tests::import_rebuilds_nested_groups` -- it reads both surviving the apply pass | none | verified |
| `TOILE-IO-078` | `POST` | A picture inside the drawing MUST come in with its pixels, never as an empty box. | this spec section 3 | `ambre_toile::logic::interop::tests::embedded_png_maps_to_image_and_media_blob` | none | verified |
| `TOILE-IO-027` | `POST` | An SVG import MUST keep the paint order the drawing had. | this spec section 3 | `ambre_toile::logic::interop::tests::text_lands_in_the_paint_order_the_drawing_had` -- a text node between two shapes, which lands between them | none | verified |
| `TOILE-IO-083` | `POST` | A text node inside a group MUST stay in that group. | this spec section 3 | `ambre_toile::logic::interop::tests::text_inside_a_group_keeps_its_place_and_its_transform` -- the group's member list carries the text's slot | none | verified |
| `TOILE-IO-080` | `POST` | A text node MUST land where the drawing put it, the transforms above it included. | this spec section 3 | `ambre_toile::logic::interop::tests::text_inside_a_group_keeps_its_place_and_its_transform` -- the same text, landing at the translated group's own origin | none | verified |
| `TOILE-IO-082` | `FRAME` | Reading the drawing MUST NOT touch the board: a file refused halfway MUST leave nothing behind. | this spec section 3 | structural -- a converter is a pure function returning a batch, so a refusal carries no command; `ambre_toile::logic::interop::tests::too_many_elements_aborts_atomically` -- it refuses a file whose first nodes had already mapped; `ambre_toile::logic::interop::tests::too_deep_nesting_aborts_atomically` -- it refuses one whose outer nodes had already mapped | none | verified |
| `TOILE-IO-079` | `FAIL` | A group the board cannot rebuild MUST be reported, never dropped without a word. | this spec section 3 | `ambre_toile::logic::interop::tests::a_group_of_one_member_is_reported_rather_than_dropped` -- the member still imports, no group the apply pass would refuse is emitted, and the report names the structure | none | verified |
| `TOILE-IO-081` | `FAIL` | A drawing whose text the reader cannot parse MUST say the text was lost. | this spec section 3 | `ambre_toile::logic::interop::tests::text_the_reader_cannot_parse_is_reported` -- a node whose body the reader finds nothing in, with the shape beside it still importing | none | verified |

### 1.4 Take the board out as a file

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-IO-021` | `POST` | A user MUST be able to take the board out as a file. | this spec section 4 | `ambre_toile::logic::state::export::tests::the_board_exports_as_an_svg_document` -- it walks the board into the serializer and reads the element back out of the file; `ambre_toile::logic::state::export::tests::an_empty_board_exports_an_empty_document`; `ambre_toile::tests::chrome_ui::a_finished_export_names_its_file` -- it holds the confirmation | none | verified |
| `TOILE-IO-085` | `POST` | The exported bytes MUST reach the browser's download path. | this spec section 4 | qa-manual -- DOM glue held by using the application (`1-Foundations/ambre-spec-testing.md section 2`), a `Blob` behind an object URL, clicked through a detached anchor. The bytes it is handed are `TOILE-IO-021` | none | qa-manual |
| `TOILE-IO-004` | `POST` | An export MUST be deterministic: the same board yields a byte-stable structure. | this spec section 3 | `ambre_toile::logic::interop::svg_export::tests::export_is_deterministic`; `ambre_toile::logic::interop::svg_export::tests::rect_serializes_with_fixed_attrs`; `ambre_toile::logic::interop::svg_export::tests::text_body_is_escaped`; `ambre_toile::logic::interop::svg_export::tests::float_formatter_trims`; `ambre_toile::logic::interop::tests::round_trip_rect_is_stable`; `ambre_toile::logic::interop::tests::round_trip_path_geometry_survives` | none | verified |
| `TOILE-IO-025` | `POST` | An element the SVG serializer cannot express MUST be reported, never dropped in silence. | this spec section 3 | `ambre_toile::logic::interop::svg_export::tests::a_kind_the_writer_cannot_express_is_reported` -- a sticky, a group and a stroke each named, with the shape beside them still written; `ambre_toile::logic::interop::svg_export::tests::a_picture_without_pixels_is_reported` | partial -- a connector is not an element, so the board walk never offers one to the serializer and the file loses every arrow with nothing said; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-IO-026` | `POST` | An exported element MUST carry its rotation. | this spec section 3 | `ambre_toile::logic::interop::svg_export::tests::a_rotated_element_carries_its_angle` -- the transform a rotated element writes, and the wrapper an upright one does not | none | verified |
| `TOILE-IO-086` | `POST` | An export MUST hold every element the board holds: one placed left of or above the origin MUST NOT fall outside the frame. | this spec section 3 | `ambre_toile::logic::interop::svg_export::tests::the_frame_holds_an_element_placed_before_the_origin` -- the frame a board reaching before the origin writes, and the `0 0` one a board in the positive quadrant keeps | none | verified |
| `TOILE-IO-087` | `POST` | An exported picture MUST leave with its pixels, never with an address no reader can resolve. | this spec section 3 | `ambre_toile::logic::interop::svg_export::tests::a_picture_leaves_with_its_pixels` -- the inline data the file carries, and the address it no longer names | none | verified |
| `TOILE-IO-088` | `POST` | Exported text MUST carry the size and the colour it was drawn with. | this spec section 3 | `ambre_toile::logic::interop::svg_export::tests::text_carries_its_size_and_its_ink` | none | verified |
| `TOILE-IO-017` | `POST` | The PNG render-back MUST run offscreen, on the UI thread's own surface. | this spec section 3 | not built | unbuilt -- lands with the `PNG export` Story (`ambre-roadmap.md` E6). It needs a real GPU adapter, so its belt is wasm-tier | unimplemented |
| `TOILE-IO-024` | `FRAME` | An export MUST NOT change the board: it MUST emit no CRDT delta. | this spec section 3 | `ambre_toile::logic::state::export::tests::exporting_twice_yields_the_same_bytes_and_changes_nothing` -- every export in that module reads the empty update on its way through | none | verified |
| `TOILE-IO-097` | `FRAME` | A PNG export MUST NOT block the visible frame. | this spec section 3 | not built | unbuilt -- lands with the `PNG export` Story (`ambre-roadmap.md` E6). It needs a real GPU adapter, so its belt is wasm-tier | unimplemented |

### 1.5 Open an `.ambre` file

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-IO-090` | `POST` | Opening a file MUST repaint the board once, not once per element it carries. | this spec section 3; `ambre-spec-docfile.md section 3` | `ambre_toile::logic::state::docfile::tests::round_trip_preserves_the_document_state` -- it reads one repaint event, the rebuilt paint order and the element count | none | verified |
| `TOILE-IO-011` | `FRAME` | A rejected file MUST leave both the input bytes and the current board untouched. | this spec section 3; `ambre-spec-docfile.md section 3` | `ambre_toile::logic::state::docfile::tests::a_rejected_file_leaves_both_the_input_and_the_state_alone` -- it walks five rejection shapes and reads the bytes and the open board back after each | none | verified |
| `TOILE-IO-089` | `FAIL` | A file the engine refuses MUST fail by name, never by a panic. | this spec section 3; `ambre-spec-docfile.md section 3` | `ambre_toile::logic::state::docfile::tests::a_corrupt_payload_under_a_valid_header_is_typed_not_a_panic` | none | verified |
| `TOILE-IO-012` | `FAIL` | A `.ambre` file belonging to another app MUST be re-dispatched to that app, never coerced into Toile. | this spec section 3; `ambre-spec-docfile.md section 3` | `ambre_toile::logic::state::docfile::tests::a_trame_file_is_re_dispatched_not_coerced` | none | verified |

### 1.6 Import a Figma or a Miro document

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-IO-014` | `POST` | A Figma or Miro import MUST map every construct it recognises. | this spec section 3 | not built | unbuilt -- lands with the `Figma and Miro import` Story (`ambre-roadmap.md` E6). `ambre-toile` declares no JSON parser | unimplemented |
| `TOILE-IO-091` | `POST` | A Figma or Miro import MUST list every construct it drops. | this spec section 3 | not built | unbuilt -- lands with the `Figma and Miro import` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TOILE-IO-016` | `POST` | A Figma auto-layout frame MUST import as a plain frame with children at resolved absolute positions. | this spec section 3 | not built | unbuilt -- lands with the `Figma and Miro import` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TOILE-IO-015` | `POST` | A foreign import MUST be as atomic as the SVG path: one batch, one undo. | this spec section 3 | not built | unbuilt -- lands with the `Figma and Miro import` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TOILE-IO-093` | `FRAME` | A foreign import MUST NOT touch the board until the whole file has mapped. | this spec section 3 | not built | unbuilt -- lands with the `Figma and Miro import` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TOILE-IO-092` | `FAIL` | A foreign import MUST refuse a file that breaches a safety bound, exactly as the SVG path does. | this spec section 3 | not built | unbuilt -- lands with the `Figma and Miro import` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TOILE-IO-094` | `FAIL` | A format the converter does not support MUST be refused by name. | this spec section 3 | none | unheld -- both commands answer with a typed refusal today, and no test covers either, section 6; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.7 Export a showcase bundle

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-IO-018` | `POST` | A showcase bundle MUST embed its board snapshot. | this spec section 3 | not built | unbuilt -- lands with the `The standalone showcase bundle` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TOILE-IO-019` | `POST` | A showcase bundle MUST carry plaintext content. | this spec section 3; `ZK-001` | not built | unbuilt -- lands with the `The standalone showcase bundle` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TOILE-IO-096` | `POST` | An export that produces a showcase bundle MUST say, at export time, that the bundle leaves the encryption boundary. | this spec section 3; `ZK-001` | not built | unbuilt -- lands with the `The standalone showcase bundle` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TOILE-IO-095` | `FRAME` | A showcase bundle MUST NOT contact the relay to render. | this spec section 3 | not built | unbuilt -- lands with the `The standalone showcase bundle` Story (`ambre-roadmap.md` E6) | unimplemented |

* **Budget.** the SVG import of 5 000 nodes under 500 ms, a row of
  `ambre-spec-budgets.md section 1`. The conversion has a caller at last, and nothing measures it
  (`TOILE-IO-028`).
* **Boundary.** a local file, in both directions, and nothing else. No converter opens a socket, and
  the pixels an import produces reach the relay as ciphertext alone
  (`features/ambre-spec-toile-media.md section 1`, `TOILE-IO-013`). The showcase bundle is the one
  deliberate exit from the zero-knowledge boundary, and `TOILE-IO-096` makes it say so.
* **Verification.** 34 `verified`, 2 `failing`, 3 `untested`, 12 `unimplemented`, 2 `qa-manual`. What
  holds is the drop point, the reader's structure, the writer's own, and the report, each carrying a
  test of its own. The two `qa-manual` rows are the DOM glue underneath, which no belt reaches. What
  is loose is five rules: two the code answers with something else, three it holds in part.
---

## 2. WHAT EACH USER SEES

An import is an ordinary CRDT write, so it reaches the other peer like any edit. The last column
joins the example to the test that holds it.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| imports an SVG | draws elsewhere on the board | the imported elements and the drawing | both, on both boards | none. No property test drives an import |
| imports a file | imports the same file | two sets of elements, each minted by its own peer | two imports, never one merged drawing | none |
| imports a drawing, then undoes | watches | the board as before the import (`TOILE-IO-001`) | the drawing leaves in one step, on both boards | `logic::state::import::tests::import_is_one_undo_group` |
| imports a broken file | watches | nothing (`TOILE-IO-073`) | an error on Alice's screen and no change on Bob's | `logic::state::import::tests::a_broken_file_fails_without_touching_the_board` |
| exports the board | moves an element | the move alone: the export writes nothing (`TOILE-IO-024`) | Bob's move; Alice's file holds the board she exported | `logic::state::export::tests::exporting_twice_yields_the_same_bytes_and_changes_nothing` |
| drops a drawing on an empty corner | watches | the drawing, at that corner (`TOILE-IO-023`) | the elements where Alice dropped them, on both boards | `logic::state::import::tests::an_import_lands_on_the_drop_point` |
| saves an `.ambre` file | opens it | the same board state (`TOILE-IO-010`) | the same elements, styles and deck | `logic::state::docfile::tests::round_trip_preserves_the_document_state` |

---

## 3. MODEL

**Containers.** This feature owns none. An import writes the element, connector, layer and media containers
the schema declares (`ambre-spec-toile-schema.md section 3`). The native export serializes the whole document
and adds nothing to it.

**Commands and events** (`ambre-bus`):

| Shape | Direction | Carries |
|---|---|---|
| `ToileCommand::ImportSvg` | UI to Logic | the bytes |
| `ToileCommand::ImportImage` | UI to Logic | the bytes and the declared type, which the decoder re-derives |
| `ToileCommand::ImportFigma`, `::ImportMiro` | UI to Logic | the bytes. Both are refused by a typed message |
| `ToileEvent::ImportReport { imported, skipped }` | Logic to UI | how many elements the file yields, and what it drops |
| `ToileEvent::ImportFailed(reason)` | Logic to UI | why the board is untouched |

No command exports. The four import commands carry no drop point (`TOILE-IO-023`), and no event
carries progress (`TOILE-IO-028`).

**The batch a converter produces.** A converter runs offline and cannot name an `ElementId`, which
the engine mints only when a create is applied. So it speaks in slots, and the apply pass resolves
them:

```rust
/// A reference to something the apply pass mints: a create slot, or a group
/// an earlier entry produced. Groups are emitted innermost-first, so a member
/// always names an already-resolved entry.
pub enum ImportRef { Slot(usize), Group(usize) }

pub struct ImportBatch {
    pub commands:   Vec<ToileCommand>,       // creates only, parent before child
    pub styles:     Vec<Option<ElementStyle>>, // index-aligned with commands
    pub groups:     Vec<ImportGroup>,        // rebuilt after the creates land
    pub connectors: Vec<ImportConnector>,    // endpoints named by slot
    pub media:      Vec<MediaBlob>,          // plaintext, for the Sync layer
    pub report:     ImportReport,
}
```

**Steps, on an import:**

1. parse and convert the whole file in memory, and return an error carrying no command on any parse
   failure or breached bound (`TOILE-IO-073`, `TOILE-IO-074`);
2. hold the batch and upload each media blob when the file carries one, applying nothing until every
   real content hash returns (`features/ambre-spec-toile-media.md section 1`);
3. create each slot, restyle it, build the groups innermost-first, then the connectors, then seed
   the media map;
4. commit once, so the whole file is one undo item, and rebuild the derived caches;
5. report how many elements the file created, and what it drops (`TOILE-IO-070`).

Step 3 is ordered on purpose: a style is an independent register that cannot ride a create. A group needs the
ids the creates mint.

**Steps, on an SVG export:** map each element back to its SVG counterpart, wrapping a rotated one in its own
transform (`TOILE-IO-026`). Write the attributes in a fixed order through a single float formatter. Anchor the
view box at the origin, or at the content's own corner when the board reaches before it (`TOILE-IO-086`). A
board in the positive quadrant therefore keeps a `0 0` origin, which is the one a reader leaves alone. Name in
the report every element no SVG shape expresses (`TOILE-IO-025`).

**What an SVG file becomes:**

| SVG node | What the import creates |
|---|---|
| `<rect>`, `<circle>`, `<ellipse>`, `<line>`, `<polygon>` | a shape, with its fill and its stroke |
| `<path>` | a vector node carrying the exact geometry |
| `<text>` | a text element, read from the source and left editable |
| `<image>` | a picture, and one blob for the media pipeline |
| `<g>` | a group, rebuilt after the creates land |
| a gradient or a filter | a raster fallback, reported as lossy |

**Text is read from the source, never from the parser.** The parser exposes a text node only by flattening it
to glyph outlines, which needs a system font database the browser does not have. That flattening also loses
the editable string. So the font stack is disabled and the text nodes are read from the XML directly.

**The two trees are walked together.** The normalized tree carries no text node at all, so the source is
stepped beside it. Each normalized child consumes the next drawn source child. Every `<text>` passed on the
way is imported at that point, inside the group the walk is in and under its transform (`TOILE-IO-027`,
`TOILE-IO-080`, `TOILE-IO-083`).

**What an SVG export writes:** shapes, vector nodes, text with its size and its ink (`TOILE-IO-088`), and
pictures carrying their pixels inline (`TOILE-IO-087`). A sticky, a group, a frame, an embed and a freehand
stroke write nothing, and each is named in the report (`TOILE-IO-025`). A connector is not an element, so the
board walk never offers one.

**The crates each path uses:**

| Crate | Role | Declared |
|---|---|---|
| `usvg` | parses SVG to a normalized tree, with no font stack | yes |
| `roxmltree` | reads the editable text bodies from the source | yes |
| `image` | decodes and encodes raster | yes |
| `postcard` | the snapshot a showcase bundle would embed | yes |
| `serde_json` | would parse a Figma or a Miro document | no |
| `lyon` | would rasterize the vector export | no |

**Code:**

* `crates/ambre-toile/src/logic/interop/svg_import.rs` -- the walk, the text pass and the groups;
* `logic/interop/svg_export.rs` -- the serializer and its float formatter;
* `logic/interop/raster.rs` -- the decode, the bound and the thumbnail;
* `logic/interop/limits.rs` -- the four safety bounds;
* `logic/interop/docfile.rs` and `logic/state/docfile.rs` -- the native pair, over the container
  `ambre-spec-docfile.md` owns;
* `logic/state/import.rs` -- the apply pass and the media round trip.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| a file dropped on the board | it is converted and applied at the drop point (`TOILE-IO-020`, `TOILE-IO-023`) |
| the File menu's import entry | a picker opens on the formats the board reads, and the chosen file takes the same path, landing in the middle of the viewport (`TOILE-IO-098`) |
| a file of a type no converter reads | it is refused before its bytes are read, by name (`TOILE-IO-098`) |
| the File menu's save entry | the whole board leaves as a native `.ambre` file, on the same answer path, and the entry's own label says the file is not encrypted (`ambre-spec-docfile.md`, `DOCFILE-001`, `DOCFILE-009`) |
| the File menu's SVG export entry | the Logic thread serializes the board and answers with the bytes (`TOILE-IO-021`) |
| the File menu's PDF export entry | the deck leaves as a page per slide, on the same answer path (`features/ambre-spec-toile-pdf-export.md`, `TOILE-IO-029`) |
| the answer | the UI thread hands the bytes to the browser, and no board state changes (`TOILE-IO-085`, `TOILE-IO-024`) |
| an import that landed | a notice says how many elements arrived, and what was dropped (`TOILE-IO-022`) |

**The shell owns the file, the board owns the point.** The canvas belongs to the launcher, so the two
listeners and the picker live there. The camera belongs to the view, so it is the view that turns a drop point
into a world point. What the launcher must not hold is a list of formats: it asks the board what it reads
(`TOILE-IO-098`). That is why the picker's filter and the drop gate can be held against each other by a test
rather than by a reviewer.

**The native file is not an export, and the menu says so by grouping.** The save sits above the separator, the
two lossy paths below it: one keeps everything and the others drop something. Opening a file is not here at
all. It belongs to the launcher, because the file's header names the application. At the moment of picking, no
board is open (`ambre-spec-docfile.md section 4`).

**A refusal is a sentence, not a log line.** Every outcome a user must see reaches the same transient notice
(`TOILE-IO-022`). Three come from an import: the count it brought in, the constructs it dropped, and a file no
converter reads. Two come from an export: the file it produced, and the reason it produced none. The
diagnostic log keeps only what a developer reads: a blob the browser refused, a media type that reached the
view past its own gate.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| Every conversion runs on the user's machine | converting on the server | it is faster to build and it hands the relay the whole document. The pillar is that the server holds ciphertext, and a conversion is the one moment a document exists as plaintext outside the editor, so it is the one that must not leave the device |
| An import is one atomic batch | streaming the elements as they parse | it is cheaper in memory and corrupts the board on any failure: a mid-file error leaves a board half built from a file the user cannot see, and no undo takes it back in one step |
| A converter speaks in slots, not in ids | minting ids in the converter | it needs the engine's counter outside the engine. An offline converter cannot know an id the engine mints on apply, and rewriting the batch afterwards needs a second pass over every reference |
| A group is rebuilt after the creates | flattening a group on import | it loses the author's structure silently. A group is born from the ids of its members, so it cannot exist before them, and emitting innermost-first makes every nested member already resolved |
| Text is read from the XML source | letting the parser flatten text | it imports a picture of a word: the parser can only give text as outlines, which needs system fonts the browser lacks and destroys the string the user typed |
| Foreign geometry is clamped, not refused | rejecting any coordinate outside the world | it fails on ordinary files. A merely large drawing is not an attack, and a clamp bounds the world and keeps the import |
| The native file is the only lossless path | promising fidelity on SVG | it drops connector semantics and the deck. Every other format has a model of its own, so a round trip through it is a conversion twice, while the native payload is the document itself |
| A showcase bundle is an explicit act | publishing automatically on export | it declassifies without asking. The bundle produces plaintext outside the encryption boundary, so it may never happen as a side effect of another action |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Two foreign formats are still refused by name** (`TOILE-IO-014`, `TOILE-IO-094`). The Figma and Miro
  commands are on the bus and answer with a typed refusal. The surface now offers neither, so a user meets the
  gap at the picker rather than at the board.
* **The drop and the download are held by a person** (`TOILE-IO-020`, `TOILE-IO-085`). Two canvas listeners,
  one `<input type="file">` and a click on a detached anchor. No belt of this workspace drives a
  `DataTransfer` or watches a file leave a browser. Everything decidable around them is a row with a test: the
  formats, the drop point, the notice. What is left is the glue itself.
* **A connector leaves with nothing said** (`TOILE-IO-025`). The report names what the serializer
  refuses, and the board walk offers it elements alone. An arrow is therefore lost before the
  serializer sees it, so no line of the report covers one.
* **An exported picture is the thumbnail, not the blob** (`TOILE-IO-100`). The full pixels are
  ciphertext on the relay and no path here fetches them, so a file leaves at the resolution the
  board itself paints.
* **The two trees are paired by position, not by identity.** Each normalized child takes the next
  drawn source child. A normalization that drops or splits a node steps the two walks apart: a
  `<use>`, a `<switch>`, a clip path. The text after that point then lands in the wrong group.
  Nothing detects the drift, and no rule covers it.
* **A text node reads its own `x`, `y` and `font-size` alone.** A `tspan` offset and an inherited
  font size are not followed, so text built from spans lands at the parent's coordinates.
* **A board the export could not read still becomes a file** (`TOILE-IO-069`). The snapshot is taken
  with a fallback: on failure it logs an error and writes the header over an empty payload. The file
  is well formed, carries the right document id, and opens as an empty board.
* **A board reaching before the origin loses its absolute place on re-import** (`TOILE-IO-086`). The
  frame then carries a non-zero origin, which a reader normalizes away. The geometry survives whole
  and the board lands at the origin. Nothing rules on where a re-imported drawing belongs.
* **No test of the apply pass starts from a populated board** (`TOILE-IO-071`). Every one begins on a
  fresh state, so the rule that an import leaves alone what the board already held rests on reading
  the code.
* **The responsiveness design has no code** (`TOILE-IO-028`). The corpus described bounded batches
  that yield between chunks and an `ImportProgress` event, and contrasted them with Trame's single
  synchronous pass. Toile's apply pass is a single synchronous pass, and no such event exists on the
  bus. The claim was a plan written in the present tense.
* **The dependency table was a plan.** Two crates were declared for paths that do not exist and are
  absent from the manifest. The table above marks each one, because a dependency list is the most
  mechanically checkable paragraph in a spec.
* **Figma and Miro are refused by name** (`TOILE-IO-094`). Both commands answer with a typed
  message and touch nothing, which is the honest behaviour, and no test asserts it. The four rows
  that describe the mapping are the specification the converters will be built against.
* **Templates have no owner.** A template is a saved batch applied through the import path, so it
  needs no new concept and no new row. It needs a store, a gallery and curation, and none of the
  three is scheduled.
* **Nothing opens or saves an `.ambre` file.** The round trip is verified on both applications and
  its only production caller is the fixture generator. A user cannot save a board to disk or open one
  from it, in Toile or in Trame.
* **No property test drives an import.** An import creates as many elements as a paste, and the two-peer
  convergence property drives neither it nor the serializers. That is why every row of section 2 that names a
  concurrent case says `none`.
* **The decoder has no fuzz target.** It is fed by user files and it is the first target
  `ambre-spec-testing.md` names, and none exists yet.
