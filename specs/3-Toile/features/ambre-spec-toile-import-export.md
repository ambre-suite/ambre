# AMBRE -- FEATURE: TOILE IMPORT AND EXPORT

**Type:** Vertical (feature)

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
| `TOILE-IO-010` | `IO` | An `.ambre` export and import round trip MUST preserve the document state and its whole history. | this spec section 3; `ambre-spec-docfile.md section 3` | `ambre_toile::logic::state::docfile::tests::a_round_trip_keeps_the_history_whole`; `ambre_toile::logic::state::docfile::tests::round_trip_preserves_the_document_state`; `ambre_toile::logic::state::docfile::tests::the_header_names_toile_and_the_document` | none | verified |
| `TOILE-IO-069` | `IO` | A board the export could not read MUST NOT become a file: a valid header over an empty payload MUST NOT be written. | this spec section 3 | type-level -- the container writer answers `Option<Vec<u8>>` and the header is written inside the branch a read snapshot reaches, so no payload-less file can be constructed. The registry turns the empty answer into the export-failure event, on the plain path and the sealed one alike | none | verified |
| `TOILE-IO-104` | `INV` | A reader MUST keep the SVG and PDF exports: they write nothing, so the role gate MUST NOT withhold them. | this spec section 3; `ambre-spec-architecture.md section 4`; `TRAME-IO-034`; `INTEROP-006` | `ambre_toile::ui::menu_bar::tests::exports_are_not_mutating` -- it pins both commands as non-mutating, the flag the role gate reads | none | verified |
| `TOILE-IO-181` | `IO` | The bytes a PNG export hands back MUST be a PNG image of the size asked, which the page fills edge to edge. | this spec section 3 | `ambre_convert::engine::png::tests::a_page_rasterizes_at_the_size_asked_and_fills_it` -- the PNG signature, the size, and a rect over half the page inking half the raster at twice the scale | none | verified |
| `TOILE-IO-182` | `IO` | A clip in the recorded page MUST keep what it holds, and nothing outside it MUST reach the image. | this spec section 3 | `ambre_convert::engine::png::tests::a_clip_keeps_its_rect_alone_until_it_pops` | none | verified |
| `TOILE-IO-183` | `IO` | Text MUST reach the image as its glyphs, placed where the screen places them: at the pen position, above the baseline. | this spec section 3 | `ambre_convert::engine::png::tests::a_glyph_run_inks_above_its_baseline` -- a glyph of a shipped face, and no ink below its baseline or past its run | none | verified |
| `TOILE-IO-184` | `IO` | A picture MUST reach the image as its pixels, over its destination rect. | this spec section 3 | `ambre_convert::engine::png::tests::a_picture_fills_its_rect_with_its_pixels` | none | verified |
| `TOILE-IO-188` | `IO` | The image of a frame MUST carry that frame alone: it is clipped to the frame's rect. | this spec section 3; `TOILE-IO-030` | `ambre_toile::logic::state::presentation::pdf_tests::a_frame_exports_at_twice_its_size_clipped_to_itself` -- the size and the clip | none | verified |
| `TOILE-IO-190` | `IO` | The image of the board MUST hold its content inside a margin of background. | this spec section 3 | `ambre_toile::logic::state::presentation::pdf_tests::a_board_png_holds_its_shape_inside_its_margin` -- the page recorded by the screen's walk, rasterized by the PDF module, inks the shape's colour in the middle and the background in the margin | none | verified |
| `TOILE-IO-191` | `INV` | A reader MUST keep the PNG export: it writes nothing, so the role gate MUST NOT withhold it. | this spec section 3; `TOILE-IO-104` | `ambre_toile::ui::menu_bar::tests::exports_are_not_mutating` -- it pins the PNG command as non-mutating | none | verified |

### 1.2 Bring a file onto the board

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-IO-020` | `POST` | A user MUST be able to bring a file onto the board, by a drop or by a picker. | this spec section 4 | `ambre_belt::toile::tests::a_dropped_file_and_a_picked_file_both_land` -- browser tier: an SVG dropped on the canvas and one chosen through the picker each land as a shape | none | verified |
| `TOILE-IO-098` | `FAIL` | A file of a type no converter reads MUST be refused with a word, and MUST NOT be read into memory. | this spec section 4 | `ambre_toile::ui::interop::tests::a_format_no_converter_reads_is_refused`; `ambre_toile::ui::interop::tests::every_format_a_converter_reads_is_accepted`; `ambre_toile::ui::interop::tests::the_picker_offers_only_what_the_gate_accepts` -- it holds the picker's filter against the same gate, so a file the dialog offers is never one the board then drops | none | verified |
| `TOILE-IO-001` | `POST` | The whole file MUST become one undoable batch: one undo MUST take the import back. | this spec section 3 | `ambre_toile::logic::state::import::tests::import_is_one_undo_group` -- it undoes a five-element import in one step; `ambre_toile::logic::state::import::tests::import_rebuilds_nested_groups` -- a three-element one carrying two groups; `ambre_toile::logic::state::import::tests::svg_import_takes_the_fast_path` -- it reads the single delta; `ambre_toile::logic::state::import::tests::undo_removes_a_sliced_import_whole` -- a file of several slices | none | verified |
| `TOILE-IO-002` | `POST` | An import MUST NOT trust foreign geometry: every coordinate MUST be clamped to the world. | this spec section 3 | `ambre_toile::logic::interop::limits::tests::coord_clamp_bounds_and_sanitizes`; `ambre_toile::logic::interop::limits::tests::rect_clamp_keeps_non_negative_extent`; `ambre_toile::logic::interop::limits::tests::a_coord_bound_is_finite_and_not_negative`; `ambre_toile::logic::interop::tests::coordinates_are_clamped_to_the_world_bound` | none | verified |
| `TOILE-IO-072` | `POST` | A coordinate that is not a number MUST become zero, never poison the geometry it lands in. | this spec section 3 | `ambre_toile::logic::interop::limits::tests::coord_clamp_bounds_and_sanitizes` -- it reads a NaN and an infinity both collapsing to zero | none | verified |
| `TOILE-IO-070` | `POST` | An import MUST say how many elements it created. | this spec section 3 | `ambre_toile::logic::state::import::tests::import_is_one_undo_group` -- it reads the count back against the file; `ambre_toile::logic::state::import::tests::import_rebuilds_nested_groups` -- it reads a count of the three elements and not of the two groups the import rebuilt | none | verified |
| `TOILE-IO-022` | `POST` | An import report MUST reach the user: a construct dropped in silence MUST NOT be the outcome. | this spec section 4; `INTEROP-007` | `ambre_toile::tests::chrome_ui::an_import_report_reaches_the_user` -- it reads the toast a lossy import raises; `ambre_toile::tests::chrome_ui::a_refused_file_reaches_the_user` -- the one a file that never mapped raises; `ambre_toile::ui::interop::message_tests::a_clean_import_reports_its_count` -- the wording; `ambre_toile::ui::interop::message_tests::a_lossy_import_names_what_it_dropped`; `ambre_toile::ui::interop::message_tests::a_long_list_of_dropped_constructs_is_counted` | none | verified |
| `TOILE-IO-023` | `POST` | An imported file MUST land where the user put it, never at a fixed point of the world. | this spec section 4 | `ambre_toile::logic::state::import::tests::an_import_lands_on_the_drop_point`; `ambre_toile::logic::interop::tests::a_batch_lands_with_its_corner_on_the_drop_point` -- the placement itself; `ambre_toile::logic::interop::tests::placing_a_batch_that_creates_nothing_is_a_no_op` | none | verified |
| `TOILE-IO-028` | `POST` | A large import MUST NOT freeze the Logic thread: it MUST apply in bounded batches that yield. | this spec section 3; `ambre-spec-budgets.md`; `ARCH-030`; `ARCH-031` | `ambre_toile::logic::state::import::tests::a_large_import_lands_one_slice_at_a_time` -- one slice per call, a delta each; `ambre_toile::logic::state::import::tests::a_camera_move_does_not_wait_for_the_import`; `ambre_toile::logic::state::import::tests::an_edit_lands_the_import_first_in_one_delta`; `ambre_toile::logic::state::import::tests::undo_removes_a_sliced_import_whole`; `ambre_toile::logic::state::import::tests::undo_mid_import_removes_it_whole` | none | verified |
| `TOILE-IO-071` | `FRAME` | An import MUST NOT change an element the board already held. | this spec section 3 | `ambre_toile::logic::state::import::tests::an_import_leaves_the_board_it_lands_on_alone` -- a grouped file lands on a board holding a shape, and that shape's stored map is unchanged | none | verified |
| `TOILE-IO-073` | `FAIL` | A file the converter cannot read MUST import nothing, and MUST leave the board as it was. | this spec section 3 | `ambre_toile::logic::state::import::tests::a_broken_file_fails_without_touching_the_board`; `ambre_toile::logic::interop::tests::malformed_svg_fails_to_parse` | none | verified |
| `TOILE-IO-074` | `FAIL` | A file past the count, the depth or the picture-size bound MUST be refused whole, never in part. | this spec section 3 | `ambre_toile::logic::interop::limits::tests::element_count_guard`; `ambre_toile::logic::interop::limits::tests::depth_guard`; `ambre_toile::logic::interop::limits::tests::image_dim_guard`; `ambre_toile::logic::interop::tests::too_many_elements_aborts_atomically`; `ambre_toile::logic::interop::tests::too_deep_nesting_aborts_atomically` | none | verified |
| `TOILE-IO-084` | `FAIL` | A file that maps to no element MUST be refused, never applied as an empty import. | this spec section 3; `INTEROP-011` | `ambre_toile::logic::interop::tests::empty_svg_reports_empty`; `ambre_toile::logic::interop::board_import::tests::a_board_of_nothing_the_board_holds_is_refused` | none | verified |

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
| `TOILE-IO-079` | `FAIL` | A group the board cannot rebuild MUST be reported, never dropped without a word. | this spec section 3; `INTEROP-003` | `ambre_toile::logic::interop::tests::a_group_of_one_member_is_reported_rather_than_dropped` -- the member still imports, no group the apply pass would refuse is emitted, and the report names the structure | none | verified |
| `TOILE-IO-081` | `FAIL` | A drawing whose text the reader cannot parse MUST say the text was lost. | this spec section 3; `INTEROP-003` | `ambre_toile::logic::interop::tests::text_the_reader_cannot_parse_is_reported` -- a node whose body the reader finds nothing in, with the shape beside it still importing | none | verified |

### 1.4 Take the board out as a file

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-IO-021` | `POST` | A user MUST be able to take the board out as a file. | this spec section 4 | `ambre_toile::logic::state::export::tests::the_board_exports_as_an_svg_document` -- it walks the board into the serializer and reads the element back out of the file; `ambre_toile::logic::state::export::tests::an_empty_board_exports_an_empty_document`; `ambre_toile::tests::chrome_ui::a_finished_export_names_its_file` -- it holds the confirmation | none | verified |
| `TOILE-IO-085` | `POST` | The exported bytes MUST reach the browser's download path. | this spec section 4 | `ambre_belt::toile::tests::an_svg_export_reaches_the_download_directory` -- browser tier: the file lands in the download directory as an SVG of the board's one rectangle | none | verified |
| `TOILE-IO-004` | `POST` | An export MUST be deterministic: the same board yields a byte-stable structure. | this spec section 3 | `ambre_toile::logic::interop::svg_export::tests::export_is_deterministic`; `ambre_toile::logic::interop::svg_export::tests::rect_serializes_with_fixed_attrs`; `ambre_toile::logic::interop::svg_export::tests::text_body_is_escaped`; `ambre_toile::logic::interop::svg_export::tests::float_formatter_trims`; `ambre_toile::logic::interop::tests::round_trip_rect_is_stable`; `ambre_toile::logic::interop::tests::round_trip_path_geometry_survives`; `ambre_toile::logic::state::export::tests::exporting_twice_yields_the_same_bytes_and_changes_nothing` -- through the Logic handler | none | verified |
| `TOILE-IO-025` | `POST` | An element the SVG serializer cannot express MUST be reported, never dropped in silence. | this spec section 3; `INTEROP-003` | `ambre_toile::logic::interop::svg_export::tests::a_kind_the_writer_cannot_express_is_reported` -- a sticky, a group and a stroke each named, with the shape beside them still written; `ambre_toile::logic::interop::svg_export::tests::a_picture_without_pixels_is_reported`; `ambre_toile::logic::state::export::tests::a_connector_is_named_in_the_report` -- the board walk names each connector, which never reaches the serializer | none | verified |
| `TOILE-IO-026` | `POST` | An exported element MUST carry its rotation. | this spec section 3 | `ambre_toile::logic::interop::svg_export::tests::a_rotated_element_carries_its_angle` -- the transform a rotated element writes, and the wrapper an upright one does not | none | verified |
| `TOILE-IO-086` | `POST` | An export MUST hold every element the board holds: one placed left of or above the origin MUST NOT fall outside the frame. | this spec section 3 | `ambre_toile::logic::interop::svg_export::tests::the_frame_holds_an_element_placed_before_the_origin` -- the frame a board reaching before the origin writes, and the `0 0` one a board in the positive quadrant keeps | none | verified |
| `TOILE-IO-087` | `POST` | An exported picture MUST leave with its pixels, never with an address no reader can resolve. | this spec section 3 | `ambre_toile::logic::interop::svg_export::tests::a_picture_leaves_with_its_pixels` -- the inline data the file carries, and the address it no longer names; `::a_picture_without_pixels_is_reported` -- the picture whose pixels never arrived | none | verified |
| `TOILE-IO-088` | `POST` | Exported text MUST carry the size and the colour it was drawn with. | this spec section 3 | `ambre_toile::logic::interop::svg_export::tests::text_carries_its_size_and_its_ink`; `ambre_toile::logic::interop::svg_export::tests::auto_sized_text_leaves_at_the_size_it_was_drawn_with` -- an automatic size leaves at the default the board draws | none | verified |
| `TOILE-IO-017` | `POST` | The PNG raster and its encoding MUST run on the converter thread, never on the UI thread or the Logic thread. | this spec section 3; `ambre-spec-architecture.md section 2`, the converter thread | `ambre_logic::dispatch::tests::a_png_export_rasterizes_on_the_converter_thread_and_the_logic_thread_never_waits` -- the command returns with the job in flight and nothing answered, and the file comes back once the converter thread serves the job; ci-gate -- `scripts/converter_boundary.py`, run by `just converter-boundary` in the CI `lint` job, refuses `ambre-pdf` in either instance, and the raster compiles behind the same `pdf` feature of `ambre-convert`, so no thread of Main can rasterize (`ARCH-078`) | none | verified |
| `TOILE-IO-024` | `FRAME` | An export MUST NOT change the board: it MUST emit no CRDT delta. | this spec section 3 | `ambre_toile::logic::state::export::tests::exporting_twice_yields_the_same_bytes_and_changes_nothing` -- every export in that module reads the empty update on its way through; `::a_media_request_writes_nothing`; `ambre_logic::dispatch::tests::a_png_export_rasterizes_on_the_converter_thread_and_the_logic_thread_never_waits` -- the PNG export sends nothing to Sync | none | verified |
| `TOILE-IO-097` | `FRAME` | A PNG export MUST NOT block the visible frame. | this spec section 3; `ambre-spec-budgets.md section 2` | structural -- the UI thread sends the command and hands the answered bytes to the download path, as for the SVG export (`TOILE-IO-085`), and draws no pixel of the image; `ambre_logic::dispatch::tests::a_png_export_rasterizes_on_the_converter_thread_and_the_logic_thread_never_waits` -- the raster runs on the converter thread, and the UI thread receives the file alone | none | verified |
| `TOILE-IO-192` | `POST` | A user MUST be able to take the whole board, or one frame, out as a PNG image. | this spec section 4 | `ambre_toile::ui::menu_bar::tests::the_file_menu_offers_the_board_as_png` -- the File menu entry and the board command it sends; `ambre_toile::ui::menu::tests::a_lone_frame_offers_its_png_and_nothing_else_does` -- the frame's own entry; `ambre_logic::dispatch::tests::a_png_export_rasterizes_on_the_converter_thread_and_the_logic_thread_never_waits` -- the command routed to a live board answers `board.png` | none | verified |
| `TOILE-IO-185` | `FAIL` | A raster request past the edge bound, with an empty edge, or of other than one page MUST be refused before any pixel is allocated. | this spec section 3; `ambre-spec-budgets.md section 6` | `ambre_convert::engine::png::tests::a_size_past_the_bound_or_a_document_not_of_one_page_is_refused` | none | verified |
| `TOILE-IO-186` | `POST` | An image MUST show its region at two pixels per board unit, the region's corners on the image's corners. | this spec section 3 | `ambre_toile::logic::interop::png_export::tests::a_region_rasterizes_at_twice_its_size_corner_to_corner` | none | verified |
| `TOILE-IO-187` | `POST` | A region whose image would pass the edge bound MUST be scaled down until its longest edge meets the bound, keeping its aspect. | this spec section 3; `ambre-spec-budgets.md section 6` | `ambre_toile::logic::interop::png_export::tests::a_large_region_is_scaled_down_to_the_bound` | none | verified |
| `TOILE-IO-189` | `FAIL` | A PNG export of a board with no element, or of an element that is no frame, MUST refuse and say so, and hand the converter thread no job. | this spec section 3 | `ambre_toile::logic::state::presentation::pdf_tests::an_empty_board_or_a_shape_has_no_png`; `ambre_logic::dispatch::tests::a_png_export_of_an_empty_board_answers_with_a_reason_and_no_job`; `ambre_toile::ui::menu::tests::a_lone_frame_offers_its_png_and_nothing_else_does` -- a shape selected sends nothing | none | verified |

### 1.5 Open an `.ambre` file

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-IO-090` | `POST` | Opening a file MUST repaint the board once, not once per element it carries. | this spec section 3; `ambre-spec-docfile.md section 3` | `ambre_toile::logic::state::docfile::tests::round_trip_preserves_the_document_state` -- it reads one repaint event, the rebuilt paint order and the element count | none | verified |
| `TOILE-IO-011` | `FRAME` | A rejected file MUST leave both the input bytes and the current board untouched. | this spec section 3; `ambre-spec-docfile.md section 3` | `ambre_toile::logic::state::docfile::tests::a_rejected_file_leaves_both_the_input_and_the_state_alone` -- it walks five rejection shapes and reads the bytes and the open board back after each | none | verified |
| `TOILE-IO-089` | `FAIL` | A file the engine refuses MUST fail by name, never by a panic. | this spec section 3; `ambre-spec-docfile.md section 3` | `ambre_toile::logic::state::docfile::tests::a_corrupt_payload_under_a_valid_header_is_typed_not_a_panic`; cargo-fuzz -- `fuzz/fuzz_targets/toile_file.rs` wraps each input in a valid header, run by the CI `fuzz` job | none | verified |
| `TOILE-IO-012` | `FAIL` | A `.ambre` file belonging to another app MUST be re-dispatched to that app, never coerced into Toile. | this spec section 3; `ambre-spec-docfile.md section 3` | `ambre_toile::logic::state::docfile::tests::a_trame_file_is_re_dispatched_not_coerced` | none | verified |

### 1.6 Import a Figma or a Miro board

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-IO-014` | `POST` | A Figma or Miro import MUST land each shape, text and sticky note as the same kind of element, with its words, its size and its paint. | this spec section 3 | `ambre_toile::logic::interop::board_import::tests::each_figma_node_lands_as_its_own_kind`; `ambre_toile::logic::interop::board_import::tests::each_miro_item_lands_as_its_kind_and_each_loss_is_named` -- both read the authored fixtures through the images engine | none | verified |
| `TOILE-IO-091` | `POST` | A Figma or Miro import MUST name in its report every construct it drops or changes, once per occurrence. | this spec section 3; `INTEROP-003` | `ambre_toile::logic::interop::board_import::tests::every_figma_construct_dropped_is_named`; `ambre_toile::logic::interop::board_import::tests::each_miro_item_lands_as_its_kind_and_each_loss_is_named`; `ambre_toile::logic::interop::board_import::tests::a_connector_lands_between_mapped_nodes_and_a_loose_one_is_reported` | none | verified |
| `TOILE-IO-016` | `POST` | A Figma auto-layout frame MUST import as a plain frame with children at resolved absolute positions. | this spec section 3 | `ambre_toile::logic::interop::board_import::tests::an_auto_layout_frame_lands_with_its_children_where_figma_put_them` -- the export carries each child's absolute box, so nothing is resolved twice | none | verified |
| `TOILE-IO-140` | `POST` | A frame of the board MUST land as a frame, with its title, holding the nodes it held. | this spec section 3 | `ambre_toile::logic::interop::board_import::tests::a_frame_holds_its_members_and_a_labelled_shape_is_grouped`; `ambre_toile::logic::state::import::tests::a_figma_board_lands_with_its_frames_and_connectors_in_one_undo` -- it reads the title and the five members on the board | none | verified |
| `TOILE-IO-141` | `POST` | A Figma group MUST land as a group of the same members. | this spec section 3 | `ambre_toile::logic::interop::board_import::tests::a_frame_holds_its_members_and_a_labelled_shape_is_grouped` | none | verified |
| `TOILE-IO-142` | `POST` | A shape that carries words MUST land as the shape and an editable text over it, grouped. | this spec section 3 | `ambre_toile::logic::interop::board_import::tests::a_frame_holds_its_members_and_a_labelled_shape_is_grouped`; `ambre_toile::logic::interop::board_import::tests::each_figma_node_lands_as_its_own_kind` -- the label `Decide` | none | verified |
| `TOILE-IO-143` | `POST` | A connector whose two ends land MUST land as a connector between them; any other MUST be reported. | this spec section 3 | `ambre_toile::logic::interop::board_import::tests::a_connector_lands_between_mapped_nodes_and_a_loose_one_is_reported`; `ambre_toile::logic::state::import::tests::a_figma_board_lands_with_its_frames_and_connectors_in_one_undo` | none | verified |
| `TOILE-IO-144` | `POST` | A Miro item MUST keep its rotation, and a child of a frame MUST land at the frame's corner plus its own offset. | this spec section 3 | `ambre_toile::logic::interop::board_import::tests::each_miro_item_lands_as_its_kind_and_each_loss_is_named`; `ambre_toile::logic::state::import::tests::a_miro_board_keeps_its_rotation_and_its_frame` | none | verified |
| `TOILE-IO-145` | `POST` | Each Figma page after the first MUST land right of the page before it, never on top of it. | this spec section 3 | `ambre_toile::logic::interop::board_import::tests::a_second_figma_page_lands_beside_the_first` | none | verified |
| `TOILE-IO-015` | `POST` | A foreign import MUST be as atomic as the SVG path: one batch, one undo. | this spec section 3 | `ambre_toile::logic::state::import::tests::a_figma_board_lands_with_its_frames_and_connectors_in_one_undo` -- one undo takes back every element and the connector | none | verified |
| `TOILE-IO-093` | `FRAME` | A foreign import MUST NOT touch the board until the whole file has mapped. | this spec section 3 | structural -- the map is a pure function returning a batch, as the SVG path's is (`TOILE-IO-082`); `ambre_toile::logic::interop::board_import::tests::a_board_past_a_bound_is_refused_whole` -- it refuses a file whose first nodes had already mapped | none | verified |
| `TOILE-IO-092` | `FAIL` | A foreign import MUST refuse a file that breaches a safety bound, exactly as the SVG path does. | this spec section 3 | `ambre_toile::logic::interop::board_import::tests::a_board_past_a_bound_is_refused_whole` -- the element bound and the depth bound; `ambre_toile::logic::interop::board_import::tests::a_board_of_nothing_the_board_holds_is_refused` | none | verified |
| `TOILE-IO-094` | `FAIL` | JSON that is neither a Figma nor a Miro export MUST be refused by name, and MUST leave the board as it was. | this spec section 3 | `ambre_toile::logic::state::import::tests::json_of_neither_tool_is_refused_by_name` | none | verified |

### 1.7 Export a showcase bundle

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-IO-018` | `POST` | A showcase bundle MUST carry the picture of its board alone: it MUST NOT embed the board state. | this spec section 3; `ZK-037` | `ambre_toile::logic::state::export::showcase_tests::a_showcase_page_embeds_no_board_state` -- the body of the page is the SVG export and nothing else, and no template or script carries any state | none | verified |
| `TOILE-IO-019` | `POST` | A showcase bundle MUST carry plaintext content. | this spec section 3; `ZK-037` | `ambre_toile::logic::state::export::showcase_tests::a_showcase_page_draws_the_board_in_the_clear_with_no_script` -- the text of an element reads in the page as written | none | verified |
| `TOILE-IO-138` | `POST` | A showcase bundle MUST draw the board with no script: its picture MUST be the SVG export of the same board. | this spec section 3; `TOILE-IO-021` | `ambre_toile::logic::state::export::showcase_tests::a_showcase_page_draws_the_board_in_the_clear_with_no_script` -- the page holds the SVG export whole and no script element | none | verified |
| `TOILE-IO-096` | `POST` | An export that produces a showcase bundle MUST say, at export time, that the bundle leaves the encryption boundary. | this spec section 4; `ZK-037` | `ambre_toile::ui::commands::tests::the_showcase_label_says_the_page_leaves_the_encryption` -- asserted on the command's label, the one string every menu and the palette show before the click; `ambre_toile::ui::menu_bar::tests::the_file_menu_offers_the_showcase_page_as_its_own_gesture` | none | verified |
| `TOILE-IO-139` | `POST` | A showcase bundle MUST report each element its picture cannot draw. | this spec section 3; `TOILE-IO-025`; `INTEROP-003` | `ambre_toile::logic::state::export::showcase_tests::a_showcase_page_names_what_it_cannot_draw` -- a sticky note named, under the HTML format | none | verified |
| `TOILE-IO-095` | `FRAME` | A showcase bundle MUST NOT contact the relay to render. | this spec section 3 | `ambre_toile::logic::state::export::showcase_tests::a_showcase_page_loads_nothing_from_the_network` -- every address the page names is a `data:` address, it holds no element that loads a file, and its content policy refuses every fetch before any other element of the head; `::the_address_scan_finds_a_remote_picture` -- the scan the first test runs finds a remote address | none | verified |
| `TOILE-IO-137` | `FRAME` | A showcase bundle MUST NOT carry a comment, a reaction or an element the board deleted. | this spec section 3; `ZK-037` | `ambre_toile::logic::state::export::showcase_tests::a_showcase_page_embeds_no_board_state` -- a deleted element, a comment and a reaction leave no word in the page, while the board's own snapshot holds the deleted text in the clear | none | verified |

* **Budget.** the SVG import of 5 000 nodes under 500 ms, a row of
  `ambre-spec-budgets.md section 1`. The `import` bench of `ambre-toile` reads the whole import and
  its longest slice (`TOILE-IO-028`).
* **Boundary.** a local file, in both directions, and nothing else. No converter opens a socket, and
  the pixels an import produces reach the relay as ciphertext alone
  (`features/ambre-spec-toile-media.md section 1`, `TOILE-IO-013`). The showcase bundle is the one
  deliberate exit from the zero-knowledge boundary (`ZK-037`), and `TOILE-IO-096` makes it say so.
* **Verification.** 75 `verified`. What holds carries a test of its own: the
  drop point, the reader's structure, the writer's own and the report. So do the sliced apply on a
  populated board and the two foreign boards, each read from an authored fixture. The browser tier
  drives the glue underneath: a drop, a picked file and a download. No browser opens a showcase
  page in CI.
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
| `ToileCommand::Interop(Import(ToileImport { format, data, at }))` | UI to Logic | the format, the bytes and the drop point (`TOILE-IO-023`). A picture names its declared type, which the decoder re-derives. A JSON file is a board, Figma or Miro, which the boards module tells apart |
| `ToileCommand::Interop(Export(ToileExport))` | UI to Logic | SVG, PDF, a showcase page, the native file or the sealed native file |
| `ToileEvent::Interop(ImportCompleted { report })` | Logic to UI | how many elements the file yields, and what it drops |
| `ToileEvent::Interop(Progress(progress))` | Logic to UI | the units applied of the batch, after each slice but the last (`INTEROP-008`) |
| `ToileEvent::Interop(ExportReady { file, report })` | Logic to UI | the file, and what the format cannot express (`TOILE-IO-025`) |
| `ToileEvent::Interop(Failed(failure))` | Logic to UI | why the board is untouched, or why no file left |

The shapes and the report are the shared ones (`1-Foundations/ambre-spec-interop.md section 3`).
Toile names its constructs: an element carried, and a kind of element, a text, a picture, a nested
SVG or a gradient paint dropped. Each slice of a large import also repaints the board, and the
drawing grows on screen (`TOILE-IO-028`).

**The batch a converter produces.** A converter runs offline and cannot name an `Id<Element>`, which
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
    pub rotations:  Vec<Radians>,            // index-aligned, shorter when upright
    pub groups:     Vec<ImportGroup>,        // rebuilt after the creates land
    pub frames:     Vec<ImportFrame>,        // titled and filled after the groups
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
3. create each slot and turn it, then restyle it. Build the groups innermost-first, then title and
   fill the frames innermost-first. Then draw the connectors, and seed the media map;
4. apply step 3 in slices of a fixed count of units, one commit and one delta each. One Loro undo
   group spans them, so the whole file is one undo item. The Logic thread applies one slice per
   pass, and comes back without a message while one remains (`TOILE-IO-028`);
5. land the rest at once before anything that writes or reads the board: an edit, an undo, an
   export, a remote delta, or a close. Only a camera move, a cursor and slide navigation pass;
6. rebuild the derived caches once the last slice lands, and report how many elements the file
   created, and what it drops (`TOILE-IO-070`).

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

**Which Figma and Miro file.** Neither tool exports a documented board file: a Figma `.fig` and a
Miro `.rtb` backup are private binary formats. Each documents the JSON of its REST interface, and a
user saves it to a file with a personal token, outside Ambre. Ambre reads that file and opens no
socket (`TOILE-IO-068`):

| Tool | The file |
|---|---|
| Figma | the answer of `GET /v1/files/:key`: a `document` node and its tree, each node with its `absoluteBoundingBox` |
| Miro | the answer of `GET /v2/boards/:id/items`: a `data` list of items. The connectors answer joins it, in the same list or beside it as `connectors` |

The drop gate takes any `.json` file. The boards module tells the two tools apart by the top-level
keys, and refuses JSON that is neither (`TOILE-IO-094`). It answers a flat board, and Toile maps it.

**What a board becomes:**

| Figma node, Miro item | What the import creates | Report |
|---|---|---|
| `FRAME`, `SECTION`, `COMPONENT`; `frame` | a frame, titled, holding its children (`TOILE-IO-140`) | none |
| `INSTANCE` | a frame | component link dropped |
| `GROUP` | a group (`TOILE-IO-141`) | none |
| `RECTANGLE`, `ELLIPSE`, `LINE`; `shape` of a rectangle, a circle or a triangle | the same shape | none |
| a rounded rectangle | a rectangle whose corner is a tenth of its shorter side | none |
| a diamond, a rhombus | a vector node of the four corners | none |
| any other outline: a star, a cloud, a callout | a rectangle | shape with no board counterpart, approximated |
| `SHAPE_WITH_TEXT`; a `shape` with content | the shape and a text over it, grouped (`TOILE-IO-142`) | none |
| `TEXT`; `text` | a text, at its size and in its ink | text formatting approximated, when runs differ |
| `STICKY`; `sticky_note` | a sticky note, in its colour | text formatting approximated, when runs differ |
| `CONNECTOR`; `connector` | a connector, when both ends land (`TOILE-IO-143`) | connector dropped otherwise; connector label dropped |
| `VECTOR`, `BOOLEAN_OPERATION` | nothing: the outline sits in the file only on request | vector path dropped |
| a picture fill; `image` | nothing: the pixels sit behind an address no converter fetches | picture dropped |
| `EMBED`, `LINK_UNFURL`; `embed`, `preview`, `document`, `app_card` | nothing | embed dropped |
| `TABLE`; `card`; `WIDGET`, `STAMP`; a type the reader does not know | nothing | table, card, widget or unknown element dropped |
| a node Figma hides | nothing: it shows nothing in Figma | none |

A gradient fill, a Figma shadow and a Figma blur are dropped and reported, as on the SVG path. A
Figma rotation lands upright in the node's bounding box and is reported approximated: the
box the export writes is already turned. A Miro item keeps its rotation, and a child of a frame
lands at the frame's corner plus its own offset (`TOILE-IO-144`). Each Figma page holds its own
plane, so each page after the first lands 200 units right of the one before it (`TOILE-IO-145`).
A frame under an auto layout needs nothing more: the export writes each child's resolved box
(`TOILE-IO-016`).

A Miro text reads its HTML as plain lines; a bold word or a link is reported.

**What an SVG export writes:** shapes, vector nodes, text with its size and its ink (`TOILE-IO-088`), and
pictures carrying their pixels inline (`TOILE-IO-087`). A sticky, a group, a frame, an embed and a freehand
stroke write nothing, and each is named in the report (`TOILE-IO-025`). A connector is not an element, so the
board walk never offers one to the serializer and names it in the report itself.

**Steps, on a showcase export:**

1. write the SVG export of the board, its report included (`TOILE-IO-138`, `TOILE-IO-139`);
2. write one HTML page. The content policy comes first in the head, and the SVG document inline is
   the whole body (`TOILE-IO-018`, `TOILE-IO-137`);
3. answer the page as the one display event, which the UI thread hands to the browser
   (`ZK-037`).

**What a showcase page holds:** a policy of `default-src 'none'; img-src data:; style-src
'unsafe-inline'`, so the browser loads nothing and runs nothing (`TOILE-IO-095`). The page names
no address beyond the `data:` address of each picture. It embeds no board state: a reader learns
what the picture shows, and nothing the picture leaves out.

**What a PNG export draws.** The image of the board or of one frame is the page the screen's walk
draws, the walk the PDF export records (`features/ambre-spec-toile-pdf-export.md section 3`). The
board grid is dropped, and pictures come at the resolution the board holds (`TOILE-IO-100`).

1. on the Logic thread, read the region. It is the board's content with a margin of 16 board units
   (`TOILE-IO-190`), or one frame's rect, which also clips the page (`TOILE-IO-188`). A board with no
   element or an element that is no frame refuses at once (`TOILE-IO-189`);
2. set the page camera at two raster pixels per board unit (`TOILE-IO-186`). Scale it down when the
   longest edge would pass the bound of `ambre-spec-budgets.md section 6` (`TOILE-IO-187`);
3. record the page through the walk in one step. Hand it to the converter thread as one job, and
   never wait for it (`TOILE-IO-017`, `ARCH-098`);
4. on the converter thread, the PDF module replays the ops onto premultiplied samples and writes
   the PNG file (`TOILE-IO-181` to `TOILE-IO-185`). It flattens each path and accumulates the
   area each line covers, as a font rasterizer does. krilla links the stroker, the face parser
   and the zlib stream already. The raster adds its own small code and the glyph outlines;
5. the answer reaches the board's view as `board.png`, on the path every export takes
   (`TOILE-IO-192`, `TOILE-IO-085`).

**The crates each path uses:**

| Crate | Role | Declared |
|---|---|---|
| `usvg` | parses SVG to a normalized tree, with no font stack, in the images converter module | in `ambre-convert` |
| `roxmltree` | reads the editable text bodies from the source, in the images converter module | in `ambre-convert` |
| `image` | decodes and encodes raster | yes |
| `base64` | the pictures an SVG export writes inline | yes |
| `serde_json` | was the plan for a Figma or a Miro export. The boards module reads the JSON with a reader of its own, smaller than a general JSON crate in a module that holds a size ceiling | no |
| `tiny-skia-path` | widens a stroke of the PNG export into its outline, the copy krilla links | in `ambre-convert` |
| `skrifa` | reads the glyph outlines the PNG raster fills, the copy krilla links | in `ambre-convert` |
| `flate2` | writes the zlib stream of the PNG file, the copy krilla links | in `ambre-convert` |

**Code:**

* `crates/ambre-convert/src/engine/svg.rs` -- the parse and the text pass, in the images converter
  module;
* `crates/ambre-toile/src/logic/interop/svg_import.rs` -- the walk of the drawing the module answers,
  and the groups;
* `crates/ambre-convert/src/engine/board.rs` -- the Figma and Miro reader, in the boards converter
  module, over the JSON reader of `engine/json.rs`;
* `logic/interop/board_import.rs` -- the walk of the board the module answers, its frames, groups
  and connectors;
* `logic/interop/svg_export.rs` -- the serializer and its float formatter;
* `logic/interop/showcase.rs` -- the showcase page around the SVG export;
* `logic/interop/png_export.rs` -- the region, the page camera and the bound of a PNG export;
* `crates/ambre-convert/src/engine/png.rs` -- the replay, the coverage rasterizer and the PNG
  writer, in the PDF converter module;
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
| the File menu's showcase entry | the Logic thread writes the page and answers with the bytes. The entry sits in a group of its own, and its label says the page is not encrypted and readable by anyone (`TOILE-IO-018`, `TOILE-IO-096`) |
| the File menu's PNG export entry | the Logic thread records the whole board as one page, and the converter thread answers the image (`TOILE-IO-192`, `TOILE-IO-017`) |
| the context menu of a frame selected alone | its PNG export entry takes the frame out as an image of itself (`TOILE-IO-192`, `TOILE-IO-188`) |
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
| A showcase page draws the SVG export | a page that boots the Ambre wasm from inline bytes | it is the whole suite inline, past 10 MB, and a renderer that waits for a GPU adapter the reader may lack. The SVG export already draws the board in any browser, with no script to trust |
| A showcase page is the picture alone | embedding the board state beside the picture | nothing reads it back, and it hands every reader what the picture leaves out: the sticky notes, the frames, the speaker notes. The page leaves the boundary for good, so it carries what the user shows and nothing more |
| A Figma or Miro import reads the REST JSON a user saved | reading the `.fig` file or the Miro `.rtb` backup | both are private binary formats with no published schema. A reader of either follows the tool's changes by trial, and a board the tool rewrites breaks it without a word |
| A Figma or Miro import reads a file | calling the tool's REST interface from Ambre with the user's token | it hands a third party's token to the client and opens a socket from a conversion. The pillar is that a conversion runs on the device and reaches nothing (`TOILE-IO-068`) |
| The PNG raster runs in the PDF module, on the converter thread | rendering the page offscreen on the UI thread's GPU surface, then reading it back | the read back stalls the frame it must not block (`TOILE-IO-097`): WebGL2, the default backend, reads a buffer back synchronously. The UI thread would then also encode the PNG, which costs tens of milliseconds for a large board. A GPU raster is testable only with a real adapter, where the CPU raster is a host test over pixels |
| The PNG raster runs in the PDF module, on the converter thread | a rasterizer of its own module, or of the images module | the PDF module already links the face parser, the stroker and the zlib stream the raster needs. A new family adds a fetch, a digest and a size budget for one export |
| The PDF module rasterizes the paths itself | `tiny-skia`, a full 2D renderer, or `zeno`, a path rasterizer | they cost the module about 250 KB and 125 KB of raw wasm more than its own raster, measured on 2026-11-15 (`ambre-spec-budgets.md section 5`). The page needs a solid fill, a rect clip and a picture alone, which a coverage rasterizer and a bilinear sample hold in a few hundred lines |
| A PNG export records its page in one step | recording it in bands over several passes | one page is the step a deck export takes, and the slice row holds it for a slide (`TOILE-IO-099`). A whole board can be denser than a slide, section 6 |
| A showcase bundle is an explicit act | publishing automatically on export | it declassifies without asking. The bundle produces plaintext outside the encryption boundary, so it may never happen as a side effect of another action |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A showcase page draws what the SVG export draws** (`TOILE-IO-139`). A sticky note, a frame,
  a freehand stroke and a connector do not leave. The report names each one.
  The page is a picture: no pan, no zoom beyond the browser's own, no deck.
* **The board reader runs in a converter module of its own.** It grew the images module past its
  ceiling, so it left for the boards module (`ambre-spec-budgets.md section 5`). The first Figma or
  Miro import of a session therefore waits on one more fetch.
* **A Figma or Miro board loses its pictures** (`TOILE-IO-091`). The export names each picture by an
  address on the tool's servers, and no converter fetches it. The report names each one.
* **A Figma rotation lands upright** (`TOILE-IO-091`). The box the export writes is the turned
  node's bounding box. The node's own size and angle need the transform the file carries only on
  request, and no row says which one Ambre reads.
* **The board readers were written against the documented schemas, not against live exports.** The
  fixtures are authored from the public reference. A key a real export spells differently lands as
  a construct dropped and named, never as a failure.
* **A shape with words lands as two elements.** A board shape carries no text of its own, so the
  text rides over it in a group. Editing the group's text works; resizing the shape leaves the text
  box where it was.
* **A connector does not leave** (`TOILE-IO-025`). The report names each one, and the file holds
  no arrow. Writing them needs the routed path the solver derives, which the export does not read.
* **An exported picture is the thumbnail, not the blob** (`TOILE-IO-100`). The full pixels are
  ciphertext on the relay and no path here fetches them, so a file leaves at the resolution the
  board itself paints.
* **The two trees are paired by position, not by identity.** Each normalized child takes the next
  drawn source child. A normalization that drops or splits a node steps the two walks apart: a
  `<use>`, a `<switch>`, a clip path. The text after that point then lands in the wrong group.
  Nothing detects the drift, and no rule covers it.
* **A text node reads its own `x`, `y` and `font-size` alone.** A `tspan` offset and an inherited
  font size are not followed, so text built from spans lands at the parent's coordinates.
* **A board reaching before the origin loses its absolute place on re-import** (`TOILE-IO-086`). The
  frame then carries a non-zero origin, which a reader normalizes away. The geometry survives whole
  and the board lands at the origin. Nothing rules on where a re-imported drawing belongs.
* **An edit made during an import waits for the rest of it** (`TOILE-IO-028`). The slices keep the
  camera and the cursors live, not the pen. Interleaving an edit would put it inside the import's
  undo group, or split the import into several undo items. The wait is bounded by the whole-import
  budget row.
* **A crash mid-import leaves part of the file on the board.** Each slice is saved and broadcast as
  it lands, so the board a reload restores holds the slices applied before the crash. A close, an
  export and a remote delta land the rest first, so only a crash shows it.
* **The conversion is one synchronous step.** The slices cover the apply pass, not the parse. The
  `import` bench reads both, and the slices hold most of the whole.
* **The dependency table was a plan.** Two crates were declared for paths that do not exist and are
  absent from the manifest. The table above marks each one, because a dependency list is the most
  mechanically checkable paragraph in a spec.
* **Templates have no owner.** A template is a saved batch applied through the import path, so it
  needs no new concept and no new row. It needs a store, a gallery and curation, and none of the
  three is scheduled.
* **Nothing opens or saves an `.ambre` file.** The round trip is verified on both applications and
  its only production caller is the fixture generator. A user cannot save a board to disk or open one
  from it, in Toile or in Trame.
* **No property test drives an import.** An import creates as many elements as a paste, and the two-peer
  convergence property drives neither it nor the serializers. That is why every row of section 2 that names a
  concurrent case says `none`.
* **No CI ceiling holds the PNG page step** (`TOILE-IO-097`). The Logic thread records a PNG export's
  page in one step, as it records a page of the deck. The instruction ceiling of
  `scripts/instructions.py` holds a slide of the deck, and a whole board can be denser. No bench
  measures the raster either: it runs on the converter thread, under the answer deadline of
  `ARCH-108`.
* **An encoded picture draws nothing on the PNG page.** A page of Toile carries decoded thumbnails
  alone, so the PDF module links no JPEG decoder for the raster. A page that recorded an encoded
  picture, as a Prose page does, would leave its box empty. No Prose or Trame PNG export exists.
* **The PNG export has no browser test.** The routing, the raster and the pixels are host tests, and
  the download path is the SVG export's (`TOILE-IO-085`). No belt test drives the entry in the served
  build.
