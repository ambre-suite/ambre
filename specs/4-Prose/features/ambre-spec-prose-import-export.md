# AMBRE -- FEATURE: PROSE IMPORT AND EXPORT

**Type:** Vertical (feature)

> A user opens a document that came from Word, from a web page or from a Markdown file. They hand
> the document back as a file someone without Ambre can read. Every conversion runs on the user's
> own machine.

The native `.ambre` file is the only path that keeps everything. Every other format drops something,
and what it drops is the subject of this document. The paginated PDF is
`features/ambre-spec-prose-pdf-export.md`, and the pixels a picture leaves behind are
`features/ambre-spec-prose-images.md`.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-IO-001` | `INV` | Every conversion MUST run in the Logic thread. | this spec section 3; `ZK-001` | structural -- the reader, the writer, the walk and the builder run inside `ProseState::handle_command`, which only the Logic worker holds; `ambre_prose::logic::interop::tests::a_markdown_file_lands_as_blocks` | none | verified |
| `PROSE-IO-043` | `INV` | No part of a document MUST reach the relay while a conversion runs. | this spec section 3; `ZK-001` | structural -- an import leaves as the one delta its commit produces, which Sync seals like any edit; `ambre_prose::logic::interop::tests::an_export_is_idempotent_and_writes_nothing` -- an export writes no delta | none | verified |
| `PROSE-IO-044` | `INV` | A reader and a writer MUST meet in the intermediate form alone: neither MUST know the other. | this spec section 3 | structural -- `crates/ambre-convert/src/engine/prose/md_read.rs` and `md_write.rs` share `ProseIr`, and neither names the other outside its tests | none | verified |
| `PROSE-IO-009` | `INV` | A reader MUST NOT emit a command. | this spec section 3 | structural -- `md_read::read` returns a `ProseIr` and names no command, document or engine type | none | verified |
| `PROSE-IO-045` | `INV` | A writer MUST NOT read the document. | this spec section 3 | structural -- `md_write::write` takes a `&ProseIr` and nothing else | none | verified |
| `PROSE-IO-046` | `INV` | The walk from the document into the intermediate form MUST be one, shared by every writer. | this spec section 3; `ambre-spec-budgets.md section 1` | structural -- `ProseState::walk_start` and `walk_roots` in `crates/ambre-prose/src/logic/interop.rs` are the one walk, and `start_export` in `logic/sliced.rs` runs it for every format a module writes | none | verified |
| `PROSE-IO-004` | `INV` | Every fidelity loss MUST raise a warning naming the construct that was dropped. | this spec section 3; `INTEROP-003` | `ambre_convert::engine::prose::md_write::tests::what_markdown_lacks_is_reported_once_each`; `ambre_convert::engine::prose::md_write::tests::commonmark_writes_no_table_todo_or_footnote`; `ambre_prose::logic::interop::tests::a_styled_cell_is_reported_on_export` | none | verified |
| `PROSE-IO-005` | `INV` | A reader MUST warn before it drops an element it does not recognise. | this spec section 3; `INTEROP-003` | `ambre_convert::engine::prose::md_read::tests::an_unknown_element_is_reported_and_its_text_kept`; `ambre_convert::engine::prose::html_doc::tests::an_unknown_element_is_reported_and_its_text_kept` | none | verified |
| `PROSE-IO-086` | `INV` | A run effect the Word model has no form for MUST be named in the report: small and all capitals as dropped, hidden text and a double strikethrough as approximated. | this spec section 3; `INTEROP-003` | `ambre_docx::fixtures::a_run_effect_the_model_lacks_is_named_in_the_report` | none | verified |
| `PROSE-IO-087` | `INV` | An import MUST name a look the form flags and carries no value of: a styled run, a styled paragraph and a styled cell. | this spec section 3; `INTEROP-003` | `ambre_prose::logic::interop::tests::word::a_look_the_form_flags_is_named_on_import` | none | verified |
| `PROSE-IO-006` | `INV` | A writer that emits an approximation MUST record it in the report. | this spec section 3; `INTEROP-003` | `ambre_prose_form::word_write::tests::an_approximation_the_writer_emits_is_in_the_report` -- a to-do item, inline code, merged cells and a list past nine levels; `::a_word_round_trip_keeps_every_structure_word_holds` -- the page the form flags is written as A4 and reported | none | verified |
| `PROSE-IO-047` | `INV` | A conversion that finished MUST answer with a report, and one that produced nothing MUST answer with an error: the two MUST NOT be one type. | this spec section 3; `INTEROP-002` | structural -- `ConversionReport` and `InteropError` are two types; `ambre_prose::logic::interop::tests::a_file_that_does_not_read_changes_nothing` | none | verified |
| `PROSE-IO-048` | `INV` | A conversion MUST NOT hold the Logic thread for the length of the file: an import MUST write the document, and an export MUST walk it, one slice per pass. | this spec section 3; `ARCH-030`; `ambre-spec-budgets.md section 1` | `ambre_logic::pass_tests::a_large_prose_import_lands_over_several_passes` -- the view is laid out at the start and at the end alone, since a layout measures the whole document; `ambre_logic::pass_tests::a_large_prose_export_walks_over_several_passes`; `ambre_logic::pass_tests::another_document_is_edited_between_two_prose_slices` -- an edit of another document is served between two slices; `ambre_prose::logic::sliced::tests::a_large_import_lands_one_slice_at_a_time`; `ambre_prose::logic::sliced::tests::every_slice_of_the_write_sends_its_delta`; `ambre_prose::logic::sliced::tests::a_large_export_walks_one_slice_at_a_time` | none | verified |
| `PROSE-IO-099` | `INV` | One slice of an import or of an export MUST hold the `Prose import or export, one slice` row. | this spec section 3; `ambre-spec-budgets.md section 1` | `callgrind` -- the `import_slice` and `export_slice` operations of `crates/ambre-prose/benches/instructions.rs`, a slice past the middle of a file of 2 000 paragraphs, under their ceilings in the CI `instructions` job | none | verified |
| `PROSE-IO-025` | `INV` | The reader an HTML import uses MUST be the reader a rich paste uses: they MUST NOT be two readers. | this spec section 3; `features/ambre-spec-prose-clipboard.md` | structural -- `Conversion::ReadHtmlDocument` and `Conversion::ReadHtml` both run `html_read::read_document` in `crates/ambre-convert/src/engine/prose/html_read.rs`, and a paste keeps its blocks alone; `ambre_prose::logic::interop::tests::html::a_page_imported_lands_as_the_page_pasted` -- the same blocks, types and marks, the colour included | none | verified |
| `PROSE-IO-049` | `INV` | The native file MUST reuse the shared container rather than restate it. | this spec section 3; `ambre-spec-docfile.md section 3` | structural -- `crates/ambre-prose/src/logic/docfile.rs` declares no header of its own and calls the shared writer and reader; `ambre_prose::logic::docfile::tests::the_header_names_prose_and_the_payload_is_the_snapshot` | none | verified |
| `PROSE-IO-012` | `IO` | The native file MUST be the only lossless round trip. | this spec section 3; `ambre-spec-docfile.md section 3` | `ambre_prose::logic::docfile::tests::the_round_trip_loses_nothing` -- a bare engine fed the payload re-exports the same file, and a live state reading it holds the same text and marks | none | verified |
| `PROSE-IO-014` | `IO` | A Word round trip MUST keep every structure the intermediate form represents, or warn for each one it drops. | this spec section 3 | `ambre_prose_form::word_write::tests::a_word_round_trip_keeps_every_structure_word_holds` -- a form of every structure Word holds, written by the codec and read back, equals itself; `::what_word_lacks_is_reported_once_each`; `ambre_prose::logic::interop::tests::word::a_word_export_reads_back_with_its_threads_and_changes` | none | verified |
| `PROSE-IO-073` | `IO` | A document the Word writer writes MUST read back through the Word reader as the same document, its ids numbered again. | this spec section 3; `PROSE-IO-014` | `ambre_docx::roundtrip::a_written_document_reads_back_whole`; `ambre_docx::fixtures::every_fixture_survives_a_round_trip`; `ambre_docx::fixtures::a_reply_and_a_resolved_thread_survive_the_round_trip` | none | verified |
| `PROSE-IO-090` | `IO` | A document the OpenDocument reader reads MUST write and read back through it as the same document, its ids numbered again. | this spec section 3; `PROSE-IO-073` | `ambre_odt::fixtures::every_odt_fixture_survives_a_round_trip` -- each `.odt` input fixture, and the read back names no loss | none | verified |
| `PROSE-IO-097` | `IO` | A document the HTML writer writes MUST read back through the HTML reader as the same blocks, with the same types, marks, colours, sizes and families. | this spec section 3; `PROSE-IO-025` | `ambre_convert::engine::prose::html_write::tests::a_written_form_reads_back_as_itself` -- a heading, every mark, a look, a nested list, a to-do item and a code block, with no loss reported; `ambre_prose::logic::interop::tests::html::an_html_export_reimports_as_the_same_document` | none | verified |

### 1.2 Bring a file into the document

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-IO-027` | `POST` | A user MUST be able to bring a file into the document. | this spec section 4 | `ambre_prose::logic::interop::tests::a_markdown_file_lands_as_blocks`; `ambre_prose::logic::interop::tests::html::an_html_file_lands_as_blocks`; `ambre_prose::commands::tests::the_save_entry_is_declared_once` -- the palette lists the import | none | verified |
| `PROSE-IO-002` | `POST` | An import MUST write through a typed operation list the engine applies as one change. | this spec section 3 | `ambre_prose::logic::interop::tests::a_markdown_file_lands_as_blocks` -- the builder emits typed `ImportOp`s | none | verified |
| `PROSE-IO-050` | `POST` | The commands MUST be built in tree order, parent before child. | this spec section 3 | `ambre_prose::logic::interop::tests::a_markdown_file_lands_as_blocks` -- a nested item and the cells land under their parent; structural -- `build` pushes a parent before it recurses | none | verified |
| `PROSE-IO-051` | `POST` | The whole file MUST apply as one batch: one undo MUST take the import back. | this spec section 3 | `ambre_prose::logic::interop::tests::one_undo_takes_the_import_back`; `ambre_prose::logic::sliced::tests::an_edit_lands_the_import_first_and_undo_removes_it_whole` -- an import of many slices | none | verified |
| `PROSE-IO-026` | `POST` | A construct the block tree refuses MUST be lifted to the nearest place the tree allows, never dropped. | this spec section 3; `ambre-spec-prose-schema.md section 4` | `ambre_prose::logic::interop::tests::a_block_the_item_refuses_is_lifted_and_reported`; structural -- `place` in `crates/ambre-prose/src/logic/interop.rs`, the one builder every import goes through, walks up the parent chain | none | verified |
| `PROSE-IO-052` | `POST` | Every lift MUST be reported. | this spec section 3; `ambre-spec-prose-schema.md section 4` | `ambre_prose::logic::interop::tests::a_block_the_item_refuses_is_lifted_and_reported`; structural -- `place` records each lift in the import's warnings | none | verified |
| `PROSE-IO-053` | `POST` | The report MUST reach the user: a construct changed in silence MUST NOT be the outcome. | this spec section 4; `INTEROP-007` | `ambre_prose::interop::tests::a_report_reads_as_one_sentence`; `ambre_prose::tests::chrome_panel::an_interop_report_reaches_the_user` | none | verified |
| `PROSE-IO-021` | `POST` | An import applied in slices MUST report its progress after every slice but the last, against one total. | this spec section 3; `ambre-spec-budgets.md section 1`; `INTEROP-009` | `ambre_prose::logic::sliced::tests::a_large_import_lands_one_slice_at_a_time` -- each slice short of the end reports further on against one total, and the last one reports the report instead; `ambre_logic::pass_tests::a_large_prose_import_lands_over_several_passes` -- the view is told | none | verified |
| `PROSE-IO-054` | `POST` | An imported picture naming a remote address MUST be reported as unresolved. | this spec section 3; `ZK-001` | `ambre_convert::engine::prose::html_doc::tests::a_remote_picture_is_reported_unresolved` -- an absolute and a relative address read as unresolved, a data address as dropped; `ambre_prose::logic::interop::tests::html::what_a_page_may_not_run_or_fetch_is_named_and_never_lands` | none | verified |
| `PROSE-IO-023` | `FRAME` | An import MUST NOT fetch a remote address a picture names. | this spec section 3; `ZK-001` | structural -- `crates/ambre-convert` declares no network crate, and the walk keeps no attribute of an `img`; `ambre_convert::engine::prose::html_doc::tests::a_remote_picture_is_reported_unresolved` -- the picture lands nowhere | none | verified |
| `PROSE-IO-055` | `FRAME` | An import MUST NOT mutate the document before its reader has finished: the reader and the builder read no document, and the typed operation list is the only writer. | this spec section 3 | structural -- `build` reads no document, and `apply_import` alone writes | none | verified |
| `PROSE-IO-011` | `FAIL` | A failed import MUST leave the document untouched: a partial application MUST NOT be observable. | this spec section 3 | `ambre_prose::logic::interop::tests::a_file_that_does_not_read_changes_nothing`; `ambre_prose::logic::interop::tests::a_refusal_midway_leaves_nothing_for_the_next_commit` | none | verified |
| `PROSE-IO-010` | `FAIL` | A malformed or hostile file MUST fail as a typed error; it MUST NOT panic. | this spec section 3; `ambre-spec-testing.md` | `ambre_prose::interop::markdown::tests::bytes_that_are_not_text_fail_typed`; `ambre_convert::engine::prose::md_read::tests::a_hostile_nesting_is_capped`; cargo-fuzz -- `fuzz/fuzz_targets/markdown.rs`, run by the CI `fuzz` job; `ambre_prose::logic::interop::tests::html::a_hostile_html_file_fails_typed_and_changes_nothing` -- bytes that are not text and a tree nested past its bound | none | verified |
| `PROSE-IO-102` | `FAIL` | A Markdown or HTML file past the bound of the import memory row MUST fail as `TooLarge` before the module reads it. | this spec section 3; `ambre-spec-budgets.md section 6`; `PROSE-IO-010` | `ambre_prose::interop::markdown::tests::a_file_past_the_memory_bound_fails_typed` -- one byte past the bound, and a file at the bound read; structural -- `ambre_prose::interop::html::MAX_SIZE` is the Markdown bound | none | verified |

### 1.3 Take the document out as a file

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-IO-028` | `POST` | A user MUST be able to take the document out as a file. | this spec section 4 | `ambre_logic::dispatch::tests::a_prose_document_saves_to_a_native_file_and_reopens` -- the save command goes through the real dispatcher; `ambre_prose::commands::tests::the_save_entry_is_declared_once` -- the palette lists the entry | none | verified |
| `PROSE-IO-056` | `POST` | The exported bytes MUST reach the browser download path. | this spec section 4 | `ambre_prose::ui::tests::a_written_file_is_handed_to_the_host_once` -- the view holds the bytes for the host once; `ambre_belt::prose::tests::a_markdown_export_reaches_the_download_directory` -- browser tier: the typed paragraph lands in the download directory as Markdown | none | verified |
| `PROSE-IO-003` | `POST` | An export MUST be idempotent: the same document exported twice MUST yield the same bytes, up to the stamp the format writes. | this spec section 3 | `ambre_prose::logic::interop::tests::an_export_is_idempotent_and_writes_nothing` | none | verified |
| `PROSE-IO-020` | `POST` | An export MUST carry the media type of its bytes, so one download path serves every format. | this spec section 3 | `ambre_logic::dispatch::tests::a_prose_document_saves_to_a_native_file_and_reopens`; structural -- `ProseEvent::ExportReady` carries `media_type`, set from `ExportFormat::media_type` | none | verified |
| `PROSE-IO-022` | `POST` | An export MUST omit the comments and the suggestions the user did not ask for. | this spec section 4; `ZK-001` | `ambre_prose::logic::interop::tests::html::an_html_export_carries_the_review_only_when_asked` -- no thread, no suggestion and the text before the review, then both on the second entry; `ambre_prose::logic::interop::tests::word::a_word_export_without_the_review_carries_none_of_it` | none | verified |
| `PROSE-IO-057` | `POST` | An export walked in slices MUST report its progress after every slice but the last, against one total. | this spec section 3; `ambre-spec-budgets.md section 1`; `INTEROP-009` | `ambre_prose::logic::sliced::tests::a_large_export_walks_one_slice_at_a_time` -- each slice short of the end reports further on against one total, and the last one answers the file the whole walk writes | none | verified |
| `PROSE-IO-017` | `FRAME` | An export to any format but PDF MUST NOT change the document: it MUST emit no CRDT delta. | this spec section 3 | `ambre_prose::logic::docfile::tests::an_export_changes_nothing` -- the version vector is the same after the export; `ambre_prose::logic::interop::tests::an_export_is_idempotent_and_writes_nothing` | none | verified |

### 1.4 Save a native file

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-IO-058` | `POST` | A native file MUST carry Prose's own document-type byte. | this spec section 3; `ambre-spec-docfile.md section 3` | `ambre_prose::logic::docfile::tests::the_header_names_prose_and_the_payload_is_the_snapshot`; `ambre_logic::dispatch::tests::a_saved_file_carries_the_document_id_the_registry_keyed_it_under` | none | verified |
| `PROSE-IO-059` | `FRAME` | A native export MUST NOT route the document through the intermediate form: its payload MUST be the snapshot itself. | this spec section 3; `ambre-spec-docfile.md section 3` | `ambre_prose::logic::docfile::tests::the_header_names_prose_and_the_payload_is_the_snapshot` -- the payload equals the engine's snapshot byte for byte | none | verified |

### 1.5 Open a native file

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-IO-013` | `FAIL` | A native file belonging to another application MUST be re-dispatched, never coerced into Prose. | this spec section 3; `ambre-spec-docfile.md section 3` | `ambre_prose::logic::docfile::tests::a_file_of_another_application_is_handed_back` -- a Trame file comes back as `WrongDocType` and the document is unchanged | none | verified |
| `PROSE-IO-065` | `POST` | A native Prose file MUST open from the home screen as a Prose document. | this spec section 4; `ambre-spec-docfile.md section 3` | `ambre_belt::prose::tests::a_native_prose_file_opens_from_the_home_screen` -- browser tier: a file saved in one browser opens from the home screen of another as a Prose view holding its text | none | verified |
| `PROSE-IO-066` | `FRAME` | Opening a native file MUST NOT write to the document: the opened document MUST export the bytes it was read from. | this spec section 3; `ambre-spec-docfile.md section 3` | `ambre_prose::logic::docfile::tests::the_round_trip_loses_nothing` -- a bare engine fed the payload re-exports the file byte for byte | none | verified |

### 1.6 Import a Word file

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-IO-060` | `POST` | A tracked change and a comment the file carries MUST arrive as a suggestion and a comment thread, never as plain text. | this spec section 3 | `ambre_prose::logic::interop::tests::word::a_word_comment_and_tracked_change_land_as_a_thread_and_a_suggestion` -- the bodies of the comments reach no block; `::a_real_word_file_lands_its_comments_as_threads` | none | verified |
| `PROSE-IO-024` | `FRAME` | A Word import MUST NOT run a macro the file carries. | this spec section 3; `ZK-001` | `ambre_prose::logic::interop::tests::word::a_word_macro_is_reported_and_never_run` -- the project inflates past the bound of a part, so a read of it would fail the import; structural -- `ambre_docx::WordDocument` holds no macro, and the Office module links no interpreter and imports no capability (`ARCH-079`) | none | verified |
| `PROSE-IO-070` | `POST` | The Word reader MUST read each tracked change and each comment a file carries as a revision and a comment, with its author and its date, never as plain text. | this spec section 3; `PROSE-IO-060` | `ambre_docx::fixtures::a_word_tracked_change_reads_as_a_revision_with_its_author`; `ambre_docx::fixtures::a_word_comment_reads_with_its_author_body_and_anchor`; `ambre_docx::fixtures::a_libreoffice_file_reads_every_construct_it_states`; `ambre_docx::fixtures::a_reply_and_a_resolved_thread_read_from_the_extended_part` | none | verified |
| `PROSE-IO-074` | `POST` | The Word reader MUST name in its report each construct it approximates or drops. | this spec section 3; `INTEROP-003` | `ambre_docx::fixtures::a_tracked_move_is_named_as_approximated`; `ambre_docx::fixtures::every_fixture_reads_as_its_expected_dump`; `ambre_docx::fixtures::a_linked_picture_is_reported_and_never_fetched`; `ambre_docx::roundtrip::a_written_document_reads_back_whole` -- the reader names no loss of what the writer writes | none | verified |
| `PROSE-IO-077` | `POST` | The Word reader MUST keep the text under an element it does not recognise. | this spec section 3; `PROSE-IO-005` | `ambre_docx::fixtures::an_unknown_element_is_reported_and_its_text_kept` | none | verified |
| `PROSE-IO-071` | `FRAME` | The Word reader MUST NOT read the bytes of a macro project: it MUST name the project in its report. | this spec section 3; `PROSE-IO-024`; `ZK-001` | `ambre_docx::fixtures::a_macro_project_is_named_and_never_read` -- the project inflates past the bound of a part, so a read of it would fail | none | verified |
| `PROSE-IO-075` | `FRAME` | The Word reader MUST NOT fetch an address a relationship names outside the package: a link keeps its address, and a linked picture is reported. | this spec section 3; `ZK-001` | `ambre_docx::fixtures::a_linked_picture_is_reported_and_never_fetched`; structural -- `crates/ambre-docx` declares no network crate, and `RelTarget::External` holds the address as text that no reader opens | none | verified |
| `PROSE-IO-072` | `FAIL` | A Word file that inflates past its bound, declares a document type or nests past its depth MUST fail as a typed error, before it holds the memory it asks for. | this spec section 3; `PROSE-IO-010` | `ambre_docx::fixtures::a_zip_bomb_fails_typed`; `ambre_docx::fixtures::an_external_entity_is_refused`; `ambre_docx::fixtures::a_hostile_nesting_fails_typed`; `ambre_docx::fixtures::a_truncated_file_fails_typed`; `ambre_docx::fixtures::a_corrupted_file_never_panics`; `ambre_opc::tests::a_part_that_inflates_past_its_bound_is_refused_while_it_inflates`; `::parts_that_inflate_past_the_package_bound_are_refused`; `::a_package_past_its_part_count_is_refused` | none | verified |
| `PROSE-IO-079` | `POST` | A comment and a suggestion a Word import brings MUST carry the author the file names, and the importing user as the one who wrote them into the document. | this spec section 3; `PROSE-IO-060`; `features/ambre-spec-prose-track-changes.md` | `ambre_prose::logic::interop::tests::word::a_word_comment_and_tracked_change_land_as_a_thread_and_a_suggestion` -- the replies name Ada and Bob, the suggestions Bob and Eve, and every suggestion holds the importing user | none | verified |
| `PROSE-IO-080` | `POST` | A Word import in a session with no identity MUST report each comment and each suggestion it drops. | this spec section 3; `PROSE-IO-004` | `ambre_prose::logic::interop::tests::word::without_an_identity_annotations_are_reported_and_their_text_kept` | none | verified |
| `PROSE-IO-082` | `POST` | A Word import that drops a comment or a suggestion MUST keep the text it covers. | this spec section 3; `PROSE-IO-005` | `ambre_prose::logic::interop::tests::word::without_an_identity_annotations_are_reported_and_their_text_kept` -- the anchor of the thread, the insertion and the deletion all land as text | none | verified |
| `PROSE-IO-083` | `POST` | A Word comment with no date MUST land dated at the moment the user chose the file, and the intermediate form MUST mark its date as unknown. | this spec section 3; `PROSE-IO-060` | `ambre_prose::logic::interop::tests::word::an_undated_word_comment_lands_at_the_import_time` -- the form holds no date, and the thread's reply holds the import's | none | verified |
| `PROSE-IO-084` | `POST` | A Word date MUST read as the UTC instant `w16du:dateUtc` gives; a date with the wall time of `w:date` alone MUST read under the rules of the session's time zone on that date, and be reported approximated. | this spec section 3; `PROSE-IO-060` | `ambre_docx::fixtures::a_date_reads_its_utc_instant_when_the_part_gives_one`; `ambre_prose_form::word_write::tests::a_wall_time_reads_in_the_session_zone_and_is_reported`; `ambre_prose_form::word_write::tests::a_word_file_read_in_paris_dates_its_comments_at_their_instants` -- under Europe/Paris, comment 0 of `pandoc/comments.docx` is 2016-05-09T14:13:00Z; `ambre_prose_form::zone::tests::a_wall_time_reads_under_the_rules_of_its_date` -- summer and winter offsets of one zone | none | verified |
| `PROSE-IO-085` | `POST` | A Word export MUST write each date as its wall time under the rules of the session's time zone in `w:date`, and its UTC instant in `w16du:dateUtc`, the namespace declared. | this spec section 3; `PROSE-IO-060` | `ambre_prose_form::word_write::tests::a_written_date_carries_its_wall_time_and_its_utc_instant` | none | verified |

### 1.7 Export to Markdown

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-IO-018` | `POST` | A Markdown export MUST report every construct the flavour has no syntax for, the page model included. | this spec section 3 | `ambre_convert::engine::prose::md_write::tests::what_markdown_lacks_is_reported_once_each`; `ambre_prose::logic::interop::tests::the_page_model_is_reported` | none | verified |
| `PROSE-IO-061` | `FRAME` | A Markdown export MUST NOT emit a table, a to-do item or a footnote the chosen flavour has no syntax for. | this spec section 3 | `ambre_convert::engine::prose::md_write::tests::commonmark_writes_no_table_todo_or_footnote` | none | verified |
| `PROSE-IO-067` | `FRAME` | A Markdown export MUST NOT write a link whose target `PROSE-UI-186` holds back from the host: the run exports as its text alone. | this spec section 3; `PROSE-UI-186` | `ambre_convert::engine::prose::md_write::tests::a_link_that_could_run_code_exports_as_its_text` -- a `javascript:` and a `data:` link export as text, a `mailto:` link as a link | none | verified |
| `PROSE-IO-068` | `FRAME` | A Markdown export MUST NOT write a link target that ends its destination, its paragraph or its block: a space, a control character, an angle bracket and a backslash are percent-encoded. | this spec section 3 | `ambre_convert::engine::prose::md_write::tests::a_link_target_cannot_break_out_of_its_destination` -- targets carrying a blank line and a link, a blank line and a heading, a paren, angle brackets, a trailing backslash, a tab and a delete | none | verified |
| `PROSE-IO-069` | `FRAME` | A Markdown export MUST NOT write a link target longer than the HTML half of a copy writes: the run exports as its text alone. | this spec section 3; `PROSE-UI-235` | `ambre_convert::engine::prose::md_write::tests::a_link_longer_than_a_copy_writes_exports_as_its_text` -- a target one byte past the cap exports as text, a target at the cap as a link | none | verified |

### 1.8 Import an HTML file

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-IO-016` | `FRAME` | An imported HTML document MUST NOT run a script it carries. | this spec section 3; `ZK-001` | `ambre_convert::engine::prose::html_doc::tests::a_script_a_stylesheet_and_a_frame_are_named_and_never_kept` -- a script element and a handler reach no block and are named; `ambre_prose::logic::interop::tests::html::what_a_page_may_not_run_or_fetch_is_named_and_never_lands`; structural -- the markup module links no script engine, and the walk keeps no attribute but a link target | none | verified |
| `PROSE-IO-062` | `FRAME` | An HTML import MUST NOT resolve a remote reference the document carries: no stylesheet, no font, no frame. | this spec section 3; `ZK-001` | `ambre_convert::engine::prose::html_doc::tests::a_script_a_stylesheet_and_a_frame_are_named_and_never_kept` -- a linked stylesheet, a preloaded font, an import and a font face in a style element, a frame and an object; `ambre_prose::logic::interop::tests::html::what_a_page_may_not_run_or_fetch_is_named_and_never_lands`; structural -- `crates/ambre-convert` declares no network crate | none | verified |
| `PROSE-IO-098` | `POST` | An HTML import MUST name in its report each element it drops or flattens: a script, a stylesheet, a frame, a form control, a picture, a table and a rule. | this spec section 3; `PROSE-IO-005`; `INTEROP-003` | `ambre_convert::engine::prose::html_doc::tests::a_script_a_stylesheet_and_a_frame_are_named_and_never_kept`; `::a_remote_picture_is_reported_unresolved`; `::an_unknown_element_is_reported_and_its_text_kept` -- a table approximated as a paragraph per cell | none | verified |

### 1.9 Export to HTML

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-IO-019` | `POST` | An HTML export MUST report the header and the footer, which the format has no element for. | this spec section 3 | `ambre_convert::engine::prose::html_write::tests::the_bands_and_the_page_are_reported`; `ambre_prose::logic::interop::tests::html::the_bands_are_reported_on_an_html_export` -- a document with a header | none | verified |
| `PROSE-IO-063` | `POST` | An HTML export MUST stand alone: it MUST carry the stylesheet it needs, in the head or inline. | this spec section 3 | `ambre_convert::engine::prose::html_write::tests::the_file_stands_alone_with_its_stylesheet_in_the_head` -- the stylesheet in the head, and no element or attribute that names a file to load | none | verified |
| `PROSE-IO-064` | `FRAME` | An HTML export MUST NOT emit a script or an event handler: the file it writes MUST be inert. | this spec section 3 | `ambre_convert::engine::prose::html_write::tests::the_file_is_inert_whatever_the_document_holds` -- a property over hostile text, links, family names, titles and code, read back through the tree builder a browser runs: no script, no handler, no frame, no loaded address; `::a_hostile_text_and_link_are_written_inert` | none | verified |

### 1.10 Export to Word

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-IO-076` | `POST` | The Word writer MUST write the same bytes for the same document. | this spec section 3; `PROSE-IO-003` | `ambre_docx::fixtures::a_document_writes_the_same_bytes_twice`; `ambre_docx::roundtrip::renumbering_is_idempotent`; `ambre_opc::tests::the_same_parts_write_the_same_bytes` | none | verified |
| `PROSE-IO-078` | `FRAME` | The Word writer MUST NOT change the document it writes. | this spec section 3; `PROSE-IO-017` | structural -- `ambre_docx::write` takes the document by shared reference and numbers its ids on a copy | none | verified |
| `PROSE-IO-081` | `POST` | A Word export MUST omit the comment threads and the suggestions unless the user asks for them, and without them it MUST write the text as it stood before the review. | this spec section 3; section 5; `PROSE-IO-022` | `ambre_prose::logic::interop::tests::word::a_word_export_without_the_review_carries_none_of_it` -- no comment part, no tracked change, the insertion left out and the deletion kept; `::a_word_export_reads_back_with_its_threads_and_changes` -- the second entry carries both | none | verified |

### 1.11 Import an OpenDocument text

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-IO-088` | `POST` | An OpenDocument text import MUST land the same document as the same text imported from Word: the same blocks, threads and suggestions, and the same report. | this spec section 3; `PROSE-IO-060`; `PROSE-IO-074` | `ambre_prose::logic::interop::tests::word::an_odt_file_lands_as_the_document_its_word_twin_lands_as` -- three pairs, each one source written as an `.odt` and a `.docx`, by LibreOffice and by pandoc; `ambre_odt::fixtures::an_odt_import_lands_as_the_word_import_of_the_same_source` -- the form, and the report entry by entry in order | none | verified |
| `PROSE-IO-095` | `POST` | The OpenDocument reader MUST keep the text under an element it does not recognise, and name the element in its report. | this spec section 3; `PROSE-IO-005` | `ambre_odt::fixtures::an_unknown_element_is_reported_and_its_text_kept` | none | verified |
| `PROSE-IO-093` | `FRAME` | The OpenDocument reader MUST NOT read a macro library or a script the package carries: it MUST name it in its report. | this spec section 3; `PROSE-IO-024`; `ZK-001` | `ambre_odt::fixtures::a_macro_library_is_named_and_never_read` -- the library inflates past the bound of a part, so a read of it would fail | none | verified |
| `PROSE-IO-094` | `FRAME` | The OpenDocument reader MUST NOT fetch an address a link or a picture names outside the package. | this spec section 3; `ZK-001` | `ambre_odt::fixtures::a_linked_picture_is_reported_and_never_fetched`; structural -- `crates/ambre-odt` declares no network crate, and a link keeps its address as text that no reader opens | none | verified |
| `PROSE-IO-092` | `FAIL` | An OpenDocument text that inflates past its bound, declares a document type or nests past its depth MUST fail as a typed error, before it holds the memory it asks for. | this spec section 3; `PROSE-IO-010` | `ambre_odt::fixtures::a_zip_bomb_fails_typed`; `ambre_odt::fixtures::an_external_entity_and_a_hostile_nesting_fail_typed`; `ambre_odt::fixtures::a_truncated_or_corrupted_file_never_panics` | none | verified |

### 1.12 Export to OpenDocument

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-IO-089` | `POST` | An OpenDocument text export MUST read back as the Word export of the same document reads back: the same document and the same report, with the review when the user asks for it. | this spec section 3; `PROSE-IO-014`; `PROSE-IO-081` | `ambre_prose::logic::interop::tests::word::an_odt_export_reimports_as_the_word_export_does` -- with and without the review, a thread of two replies and two suggestions; `ambre_odt::fixtures::an_odt_export_reads_back_as_the_word_export_of_the_same_form`; `ambre_odt::fixtures::every_word_fixture_written_as_odt_reads_back` -- the Word test set, written as `.odt` and read back as the same model | none | verified |
| `PROSE-IO-091` | `POST` | The OpenDocument writer MUST write the same bytes for the same document. | this spec section 3; `PROSE-IO-003` | `ambre_odt::fixtures::a_document_writes_the_same_bytes_twice` | none | verified |
| `PROSE-IO-096` | `FRAME` | The OpenDocument writer MUST NOT change the document it writes. | this spec section 3; `PROSE-IO-017` | structural -- `ambre_odt::write` takes the document by shared reference and numbers its ids on a copy | none | verified |

### 1.13 Work on the document while a conversion runs

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-IO-100` | `FRAME` | A command that writes the document MUST land the running conversion first: an edit MUST NOT join the step of an import, and an export MUST hold the document as it stood when the user asked. | this spec section 3; `PROSE-IO-051`; `ARCH-031` | `ambre_prose::logic::sliced::tests::an_edit_lands_the_import_first_and_undo_removes_it_whole`; `ambre_prose::logic::sliced::tests::an_edit_mid_export_leaves_the_file_as_asked` | none | verified |
| `PROSE-IO-101` | `POST` | Between two slices, a command that writes nothing MUST be served, and the conversion MUST keep running. | this spec section 3; `ARCH-030` | `ambre_prose::logic::sliced::tests::a_scroll_between_two_slices_is_served_and_the_import_still_runs` -- a page request and a word count | none | verified |

* **Budget.** seven rows of `ambre-spec-budgets.md section 1`: the Word import and export of a
  hundred pages, and the Markdown export. The other four are the HTML clipboard import, the
  plaintext native export, the shared walk, and one slice of a conversion. The CI `instructions`
  job holds the slice row (`PROSE-IO-099`), and the six others read **none exists**.
* **Boundary.** a local file, in both directions, and nothing else. `PROSE-IO-001` keeps every
  conversion in the Logic thread. `PROSE-IO-043` keeps the document off the relay. `PROSE-IO-016`,
  `PROSE-IO-062` and `PROSE-IO-023` stop an imported file executing or fetching.
* **Verification.** 85 `verified`, 0 `unimplemented`. The native file, the Markdown pair, the
  Word pair and the HTML pair are built, and each runs in slices. The browser tier holds the
  download and the open from the home screen.

---

## 2. WHAT EACH USER SEES

An import is an ordinary write, so it reaches the other peer like any edit; an export writes
nothing. No test resolves yet: the last column names the row each scenario rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| imports a Word file | types elsewhere | the imported blocks and the typing | both, on both screens | none -- `PROSE-IO-002` |
| imports a file that fails to parse | watches | the document as before | an error on Alice's screen and no change on Bob's | none -- `PROSE-IO-011` |
| imports a file, then undoes | watches | the document as before the import | the import leaves in one step, on both screens | none -- `PROSE-IO-051` |
| exports to Markdown | types | the typing alone: an export writes nothing | Bob's typing; Alice's file holds the document she exported | none -- `PROSE-IO-017` |
| exports the same document twice | -- | nothing | the same bytes, twice | none -- `PROSE-IO-003` |
| saves a native file | opens it | the same document state | the same blocks, styles and comments | none -- `PROSE-IO-012` |
| imports a Word table holding a picture | watches | the picture beside the table, never inside it | the picture out of the cell, and a line in the report | `ambre_prose::logic::picture_interop_tests::a_picture_in_a_cell_lands_beside_the_table_and_is_named` |

---

## 3. MODEL

**Containers.** This feature owns none. An import writes the block tree, the style register and the
annotation register the schema declares (`ambre-spec-prose-schema.md section 3`). The native export
serializes the whole document and adds nothing to it.

**The pipeline is symmetric.** A reader turns bytes into the intermediate form, and a writer turns
the intermediate form into bytes. Neither half knows the other (`PROSE-IO-044`), and neither
touches the document (`PROSE-IO-009`, `PROSE-IO-045`).

```text
Import:  bytes -> reader -> ProseIr -> command builder -> Vec<ImportOp> -> the document
Export:  the document -> document walk -> ProseIr -> writer -> bytes
```

The native path is the one exception, and it is deliberate. It carries a Loro snapshot under the
shared header, so it passes through no intermediate form and loses nothing (`PROSE-IO-012`,
`PROSE-IO-059`).

**The intermediate form.** A linear tree, held in memory for the length of one conversion and never
serialized:

```rust
// ambre-prose/src/interop/mod.rs
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProseIr {
    pub meta:      IrMeta,
    pub blocks:    Vec<IrBlock>,
    /// The footnote bodies, in the order of their ordinals.
    pub footnotes: Vec<IrFootnote>,
    /// The comment threads and the suggestions, out of band from the blocks: a run names each
    /// one it lies under by its place here, and the builder mints the document's ids.
    pub comments:    Vec<IrComment>,
    pub suggestions: Vec<IrSuggestion>,
}

/// What the document carries beside its blocks. A flag says a construct is present, for a
/// writer that has no syntax for it to report (`PROSE-IO-018`). `Debug` prints the title's
/// length alone.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct IrMeta {
    pub title: Option<String>,
    pub paged: Paging,   // Flow | Paged
    pub bands: Banding,  // Bare | Banded
}

/// One block. Depth is the nesting, never a number: the document derives it too. `Debug`
/// prints the length of a code block's text, and of an image's alternative text and caption,
/// alone.
#[derive(Clone, PartialEq)]
pub enum IrBlock {
    /// `styled`: a named style, or alignment, indents or spacing of its own.
    Paragraph { styled: OwnStyle, runs: Vec<IrRun> },
    /// The level is the stored one, unclamped (`PROSE-FEAT-091`).
    Heading   { level: StoredLevel, runs: Vec<IrRun> },
    ListItem  { style: IrListStyle, runs: Vec<IrRun>, children: Vec<IrBlock> },
    /// `merged`: two or more cells are merged into one.
    Table     { merged: Merging, rows: Vec<IrTableRow> },
    CodeBlock { lang: Option<Validated<CodeLang>>, text: String },
    /// The bytes ride with a picture a file carried, and with one an export embeds
    /// (`features/ambre-spec-prose-images.md section 3`).
    Image(Box<IrImage>),
    PageBreak,
    HRule,
    Toc,
}

pub struct IrImage {
    pub alt:     String,
    pub caption: String,
    pub align:   IrImageAlign,       // Block | Left | Right | Center
    pub width:   PrintLength<Pt>,    // zero when the file gave none
    pub height:  PrintLength<Pt>,
    /// The address the media flow gave the bytes, once an upload answered.
    pub address: Option<Validated<ContentHash>>,
    pub bytes:   Option<IrImageBytes>,
}

pub struct IrImageBytes { pub format: ImageFormat, pub data: Payload<ImageBytes> }

#[derive(Clone, Debug, Default, PartialEq)]
pub struct IrTableRow { pub cells: Vec<IrTableCell> }

#[derive(Clone, Debug, Default, PartialEq)]
pub struct IrTableCell {
    /// A fill, a border, a padding or a vertical alignment.
    pub styled: OwnStyle,  // Plain | Styled
    /// A reader may put anything here; the command builder lifts what the tree refuses
    /// (`PROSE-IO-026`).
    pub blocks: Vec<IrBlock>,
}

/// One footnote body. A run points at it by ordinal.
#[derive(Clone, Debug, PartialEq)]
pub struct IrFootnote { pub ordinal: Ordinal<FootnoteCall>, pub runs: Vec<IrRun> }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IrListStyle { Bullet, Ordered(IrOrderedStyle), Todo(TodoMark) }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IrOrderedStyle { Decimal, LowerAlpha, UpperAlpha, LowerRoman, UpperRoman }

/// `Debug` prints the text's length alone.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct IrRun {
    pub text:          String,
    pub bold:          Weight,         // Regular | Bold
    pub italic:        Slant,          // Upright | Italic
    pub underline:     Underline,      // Plain | Underlined
    pub strikethrough: Strikethrough,  // Plain | Struck
    pub superscript:   Superscript,    // Level | Raised
    pub subscript:     Subscript,      // Level | Lowered
    pub code:          InlineCode,     // Prose | Code
    pub link:          Option<LinkUrl>,
    /// A colour, a background, a size or a family the form carries no value of: the
    /// Word reader sets it.
    pub styled:        OwnStyle,       // Plain | Styled
    /// The colour, the background, the size and the family the form carries the value of:
    /// the walk and the HTML reader set them, and the HTML writer writes them.
    pub look:          IrLook,
    /// The threads whose anchor covers the run. A Word comment may overlap another.
    pub comments:      Vec<Pos<IrThread>>,
    /// The suggested insertion and the suggested deletion, mirroring the marks `si` and `sd`.
    pub insertion:     Option<Pos<IrChange>>,
    pub deletion:      Option<Pos<IrChange>>,
    pub cross_ref:     Referencing,    // Text | Reference
    /// The ordinal of the footnote this run calls.
    pub footnote_ref:  Option<Ordinal<FootnoteCall>>,
}

/// `Debug` prints the family's length alone. A writer reports a run whose look it cannot
/// write, flagged or valued (`IrRun::carries_look`).
#[derive(Clone, Default, PartialEq, Eq)]
pub struct IrLook {
    pub color:       Option<Rgba>,
    pub background:  Option<Rgba>,
    pub font_size:   Option<FontSize>,
    pub font_family: Option<FontFamily>,
}

/// A thread. The commented range is the runs that name it, never a field here.
pub struct IrComment {
    pub resolved: ThreadState,         // Open | Resolved
    /// The opening body first, then the replies.
    pub replies:  Vec<IrReply>,
}

/// The author is the name the file gives. `Debug` prints the text's length alone.
pub struct IrReply {
    pub author:     Option<Validated<DisplayName>>,
    /// `None` when the file gives no date: the builder dates the reply at the import.
    pub created_at: Option<Instant<Authored>>,
    pub text:       String,
}

pub struct IrSuggestion {
    pub kind:   SuggestionKind,        // Insertion | Deletion
    pub author: Option<Validated<DisplayName>>,
    /// Prose stores no time for a suggestion, so an export writes none.
    pub date:   Option<Instant<Authored>>,
}
```

The Word mapping fills the threads and the suggestions. The Markdown writer has no syntax for
either, and reports each run that carries one (`PROSE-IO-004`).

**The fields the Word, HTML and PDF filters add.** Not built: they join the intermediate form with
the reader or the writer that reads them, under their Stories (`ambre-roadmap.md` E5). The
Markdown writer has no syntax for any of them, which is why the form above keeps a flag where
these keep the construct. Each struct below lists only what it gains, and a flag gives way to the
field that carries its construct:

```rust
// ambre-prose/src/interop/mod.rs -- not built
pub struct ProseIr {
    // ...
    pub styles:   Vec<IrStyle>,
}

pub struct IrMeta {
    // ...
    /// BCP 47, as the document's `meta.locale` stores it.
    pub lang:         Option<Validated<LangTag>>,
    /// Replaces `paged`.
    pub page:         Option<IrPage>,
    /// Replace `bands`. The `first` pair is set only when the document uses a distinct first
    /// page.
    pub header:       Option<IrHeaderFooter>,
    pub header_first: Option<IrHeaderFooter>,
    pub footer:       Option<IrHeaderFooter>,
    pub footer_first: Option<IrHeaderFooter>,
}

pub struct IrPage {
    pub width:         PrintLength<Mm>,
    pub height:        PrintLength<Mm>,
    pub margin_top:    PrintLength<Mm>,
    pub margin_bottom: PrintLength<Mm>,
    pub margin_left:   PrintLength<Mm>,
    pub margin_right:  PrintLength<Mm>,
    pub orientation:   Orientation,
}

/// One paragraph of runs plus its alignment. A dynamic field is a run, not a marker.
pub struct IrHeaderFooter { pub align: Align, pub runs: Vec<IrRun> }

pub enum IrBlock {
    /// `style_id` and `para_style` replace `styled`.
    Paragraph { style_id: Option<StyleId>, para_style: IrParaStyle, runs: Vec<IrRun> },
    Heading   { level: StoredLevel, numbered: bool, runs: Vec<IrRun> },
    /// `col_widths` and `style` join `rows`; a cell's spans replace `merged`.
    Table     { col_widths: Vec<PrintLength<Pt>>, style: IrTableStyle, rows: Vec<IrTableRow> },
    Image     { content_hash: Option<Validated<ContentHash>>, src_url: Option<LinkUrl>, alt: String,
                width: Option<Px<Image>>, height: Option<Px<Image>>,
                align: Option<IrImageAlign>, caption: Option<String> },
    Toc       { max_level: HeadingLevel, numbered: bool },
    // ...
}

pub struct IrTableStyle {
    /// Promotes the first row to a header row.
    pub header_row:  bool,
    pub banded_rows: bool,
}

/// The fields below replace `styled`.
pub struct IrTableCell {
    pub rowspan:      Len<Row>,
    pub colspan:      Len<Col>,
    pub bg:           Option<Rgb>,
    pub borders:      bool,
    pub border_color: Option<Rgb>,
    pub padding:      Option<PrintLength<Pt>>,
    pub valign:       VAlign,
    // ...
}

pub struct IrRun {
    // ...
    /// Replaces `styled`, once the Word mapping reads a run's look into `look`.
    /// Set inside a header or a footer alone.
    pub field:          Option<HeaderField>,
    /// Replaces `cross_ref`.
    pub cross_ref:      Option<(Uuid, CrossRefKind)>,
}

pub struct IrParaStyle {
    pub align:          Option<Align>,
    pub indent_first:   Option<PrintLength<Pt>>,
    pub indent_left:    Option<PrintLength<Pt>>,
    pub indent_right:   Option<PrintLength<Pt>>,
    pub space_before:   Option<PrintLength<Pt>>,
    pub space_after:    Option<PrintLength<Pt>>,
    /// The three modes the document stores (`features/ambre-spec-prose-inline-styles.md`).
    pub line_height:    Option<LineHeight>,
    pub keep_together:  bool,
    pub keep_with_next: bool,
}

/// The character defaults a named style carries: formatting alone, and none of the
/// content-bearing fields of a run.
pub struct IrRunStyle {
    pub bold:          Option<bool>,
    pub italic:        Option<bool>,
    pub underline:     Option<bool>,
    pub strikethrough: Option<bool>,
    pub superscript:   Option<bool>,
    pub subscript:     Option<bool>,
    pub font_family:   Option<FontFamily>,
    pub font_size:     Option<FontSize>,
    pub color:         Option<Rgb>,
    pub background:    Option<Rgb>,
}

pub struct IrStyle {
    pub id:         StyleId,
    pub name:       String,
    pub para_style: IrParaStyle,
    pub run_style:  IrRunStyle,
}

pub enum IrImageAlign { Block, Left, Right, Center }
```

**The report, and the two error kinds.** The shapes are the shared ones
(`1-Foundations/ambre-spec-interop.md section 3`). Prose names its constructs in three families,
each with its own outcome:

```rust
// ambre_prose::message
pub type ConversionReport = ambre_bus::interop::ConversionReport<ProseConstruct>;

pub enum ProseConstruct {
    /// A construct the format has no syntax for, dropped (`PROSE-IO-004`).
    Feature(UnsupportedFeature),
    /// An element the reader does not recognise, dropped with its text kept (`PROSE-IO-005`).
    Element(DroppedElement),
    /// A block the tree refuses where the file put it, moved (`PROSE-IO-026`, `PROSE-IO-052`).
    Block(LiftedBlock),
}
```

**Steps, on an import:**

1. read the whole file into the intermediate form, and return a typed error carrying no command on
   any parse failure (`PROSE-IO-010`, `PROSE-IO-011`);
2. lift every block the tree refuses to the nearest place it allows, and record each lift
   (`PROSE-IO-026`);
3. build the typed operation list, in tree order, parent before child (`PROSE-IO-002`, `PROSE-IO-050`);
4. write them first into a scratch document, and drop it: a refusal there answers the import and
   writes nothing (`PROSE-IO-011`);
5. apply them as one batch, so the whole file is one undo item (`PROSE-IO-051`). Steps 4 and 5 run
   in slices (`PROSE-IO-048`);
6. hand the report to the view (`PROSE-IO-053`).

Step 2 is a separate pass on purpose. The tree's nesting rule is one predicate
(`ambre-spec-prose-schema.md section 4`), and a reader that applied it inline would need a copy of
it per format.

Step 4 exists because the engine takes no write back once made. A write refused midway would
leave the earlier ones for the next commit. The scratch document holds only the new blocks, so the
rehearsal costs the size of the file, not of the document. The one step that reads the real
document is where the first block lands, and it refuses before anything is written.

**Steps, on an export:** walk the document once into the intermediate form, then hand that form to
the writer. The walk is shared by all four writers (`PROSE-IO-046`). That is what makes it one
budget row rather than four, and what keeps a writer from reading the document (`PROSE-IO-045`).

**Slices.** A module reads or writes a file in one call. The half that sees the document costs the
time, as the measure of the `Prose import or export, one slice` row shows
(`ambre-spec-budgets.md section 1`). So the Logic thread runs that half in slices, one per pass
(`PROSE-IO-048`, `ARCH-030`):

| Conversion | One step | What runs whole |
|---|---|---|
| import | one operation rehearsed, then the same operation written | the module read, the build of the list, and the threads and suggestions the last slice writes |
| export | one top-level block walked, with what it nests | the annotations and the footnotes at the start, and the module write at the end |
| PDF export | one page drawn | the layout at the start, and the module write at the end (`features/ambre-spec-prose-pdf-export.md`) |

`IMPORT_SLICE` and `EXPORT_SLICE` in `crates/ambre-prose/src/logic/sliced.rs` set the steps of one
slice, and the `Prose import or export, one slice` row holds its cost (`PROSE-IO-099`). `PRINT_SLICE`
in `crates/ambre-prose/src/logic/print.rs` sets the pages of one PDF slice, and the
`Prose PDF export, one slice` row holds its cost. An import
counts each operation twice, so its total is known before the first slice. Each slice of the write
commits and sends its delta, and the writes form one step of the history (`PROSE-IO-051`).

Between two slices, the thread serves the other documents and the Sync worker. It also serves the
commands of this document that write nothing: a page request, a caret, a count, a search
(`PROSE-IO-101`). A command that writes lands the conversion first (`PROSE-IO-100`). The edit then
lands after the import, as a step of its own, and an export walks the document as the user asked
for it. A remote delta and a close land it too (`ARCH-031`). A conversion that fits one slice
answers in the pass of its command, and reports no progress.

**The bound of a file.** The import holds the whole file, its form and its list in memory until it
ends. The rehearsal holds a second copy of the blocks. Slices bound how long a pass holds the
thread, not that memory.

An import holds about 45 times its file, and the Markdown bound is derived from that ratio. A file
of 2 MiB peaks near 94 MB, inside the global target (`Prose import, transient` row of
`ambre-spec-budgets.md section 6`). A Markdown or HTML import past it fails as `TooLarge`
(`PROSE-IO-102`). A Word or an OpenDocument package keeps the bound of what the package layer
inflates (`ambre_opc::MAX_INFLATED`). A package carries its pictures, and the form keeps their
bytes until each is stored through the media flow, inside that same bound.

**The commands and the events** (`ambre_prose::message`, declared in
`features/ambre-spec-prose-engine.md section 3`):

| Shape | Direction | Carries |
|---|---|---|
| `ProseCommand::Interop(Import(ProseImport { format, data }))` | UI to Logic | the format and the bytes |
| `ProseCommand::Interop(Export(ExportFormat))` | UI to Logic | the format; the options join it with the first writer that reads one |
| `ProseEvent::Interop(ImportCompleted { report })` | Logic to UI | the report of what the file lost |
| `ProseEvent::Interop(ExportReady { file, report })` | Logic to UI | the file name, which `DOCFILE-019` and `DOCFILE-020` hold, the bytes and their media type, and what the format lost |
| `ProseEvent::Interop(Failed(failure))` | Logic to UI | why the file produced nothing (`PROSE-IO-047`) |
| `ProseEvent::Interop(Progress(progress))` | Logic to UI | the steps done of a total, after every slice but the last (`PROSE-IO-021`, `PROSE-IO-057`) |

```rust
// ambre_prose::message. A reader or a writer adds its variant on the day it lands, so the
// view never offers a format no code reads or writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportFormat { Markdown, Word, OpenDocument, Html }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    /// The native file: the shared header over the document's own snapshot.
    Ambre,
    Markdown(MarkdownFlavor),
    Word(ReviewExport),
    /// The OpenDocument text the Word export's model writes, under the same options.
    OpenDocument(ReviewExport),
    /// A standalone HTML document, with the review only when the user asks (`PROSE-IO-022`).
    Html(ReviewExport),
    /// The pages the engine laid out (`features/ambre-spec-prose-pdf-export.md`).
    Pdf(PdfExport),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkdownFlavor { Gfm, CommonMark }

/// Whether the Word export carries the threads and the suggestions. The Word writer is the
/// first that reads an option, and this is the one it reads (`PROSE-IO-081`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReviewExport { #[default] Omitted, Included }

/// What the PDF writer reads: the language, which a request may leave to the document.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PdfExport { pub language: DocumentLocale }
```

The media type each format declares:

| Format | Media type |
|---|---|
| the native file | `application/octet-stream` |
| Word | `application/vnd.openxmlformats-officedocument.wordprocessingml.document` |
| OpenDocument text | `application/vnd.oasis.opendocument.text` |
| Markdown | `text/markdown; charset=utf-8` |
| PDF | `application/pdf` |
| HTML | `text/html; charset=utf-8` |

A Markdown and an HTML writer produce text, and the bus encodes it as UTF-8 bytes. The view has
one download path and types the download with the media type (`PROSE-IO-020`). The file name
obeys `DOCFILE-019` and `DOCFILE-020`.

**The native file.** Prose owns one pair of entry points over the shared container, and nothing
else. The header layout, the version registry, the migration policy and the at-rest decision are
`ambre-spec-docfile.md section 3`. Prose writes its own document-type byte (`PROSE-IO-058`), and a file
carrying another one is handed back to the caller for re-dispatch rather than read
(`PROSE-IO-013`). A native file on disk is a different destination from the network snapshot. The
snapshot carries no header and is sealed by the Sync worker, and the file is not.

**The Word codec** (`crates/ambre-docx`). Two pure functions over a model of the format, and no
type of Prose. The codec and the Word mapping both run in the Office converter module, and the form
crosses the module boundary:

```text
Import:  bytes -> Office module: ambre_docx::read, then word_read -> ProseIr + report -> Prose
Export:  Prose -> ProseIr -> Office module: word_write, then ambre_docx::write -> bytes + report
```

**Why the mapping runs in the module.** The mapping and the serde code of the Word model took
17 257 bytes of brotli in the Main instance, past its ceiling (`ambre-spec-budgets.md section 5`).
So the intermediate form lives in `crates/ambre-prose-form`, a leaf crate the module may link, and
Prose re-exports it. The mapping sits there behind the `word` feature, which the module alone turns
on. Main links the form and its serde code, and none of the Word mapping. The Markdown mapping
moved to the markup module for the same reason, with the walk that reads the HTML half of a paste.

The model keeps what Word keeps, in Word's shape. A comment anchors on two markers, which may sit
in two paragraphs. A tracked change is a revision on each run it wraps. A reply names its parent
through `commentsExtended.xml`.

The form crosses the module boundary as postcard bytes. It nests a list item and a table cell, and
the reader caps the nesting of a part, so postcard recurses a bounded depth. Neither the codec nor
the mapping names a type of the thread protocol. Each report mirrors the shared one, and Prose
carries each entry onto its own one to one: `ProseConstruct::Word` for the reader's words.

| Layer | Crate or module | Holds |
|---|---|---|
| the package | `ambre-opc` | the ZIP archive, `[Content_Types].xml`, the relationships and the bounds of a hostile package. The xlsx and PPTX codecs share it, and it alone links `zip` |
| one part | `xml` | an owned tree per part, built without recursion. It refuses a document type declaration and resolves the five predefined entities alone |
| the reader | `read` | the parts into the model, and the report of each construct approximated or dropped (`PROSE-IO-074`) |
| the writer | `write` | the model into parts, every id numbered again first (`WordDocument::renumbered`) |

The bounds a hostile package meets, each checked while the bytes inflate (`PROSE-IO-072`). The
three package bounds are one set for DOCX, xlsx and PPTX, the row of `ambre-spec-budgets.md
section 6`:

| Bound | Value | Why |
|---|---|---|
| parts in a package | 4096 | Word writes some twenty, and a picture adds one |
| bytes one part inflates to | 32 MiB | the largest real Word body measured, 12 MB, reads under it |
| bytes every part read inflates to | 64 MiB | about 67 MB, so the model built beside the parts stays under the 100 MB client budget |
| nesting of one part | 256 elements | a nested table costs three levels, so some 80 tables deep |
| a file a Word import hands the module | 64 MiB | the inflated bound: a stored package reaches it with no compression |

What the reader does with a construct the model has no form for:

| Construct | Outcome |
|---|---|
| a computed field, a content control, a tracked move, a floating picture, a section past the first | approximated: the last result, the content, a deletion and an insertion, a picture in the line, the first section |
| a tracked format change | dropped: the format now in force is kept |
| a shape, a chart, a text box, an embedded object, an equation, a bookmark | dropped |
| a header or a footer for a first or an even page | dropped: the default band is kept |
| a picture linked from outside the package | dropped, and never fetched (`PROSE-IO-075`) |
| a macro project | dropped, and never read (`PROSE-IO-071`) |
| an element the reader does not know | dropped, and its text kept (`PROSE-IO-074`) |

A run takes the format set on it and its character style, never its paragraph style's. The role of
the paragraph carries the paragraph style, so a heading's runs do not each turn bold. A style that
draws a structure the model holds, a link or a note call, stays off the run too.

**What a Word file becomes, and what it loses:**

| Block | Word element |
|---|---|
| a paragraph | `w:p` with its style reference |
| a heading | `w:p` under the built-in heading style of that level |
| a list item | `w:p` under the list style, with the numbering the writer manages |
| a table | `w:tbl`, the widths in twips, the header row and the banding on the table properties |
| a code block | `w:p` under a code style the writer injects when the file has none |
| a picture | a drawing plus an image relationship, its bytes the ones the export fetched; its paragraph centred for a centred picture, and its caption the paragraph under it, approximated. One whose bytes did not come back is its alternative text in brackets, approximated (`PROSE-IO-008`) |
| a page break, a rule | a break run, a bottom border |
| a table of contents | the standard field |
| a footnote | an entry in the footnote part |

| Mark | Word element |
|---|---|
| bold, italic, underline, strikethrough | `w:b`, `w:i`, `w:u`, `w:strike` |
| superscript, subscript | `w:vertAlign` |
| inline code | a character style the writer injects |
| colour, size, family | `w:color`, `w:sz` in half-points, `w:rFonts` |
| background | the nearest of the eight highlights, or an exact shading |
| a link | a hyperlink run and an external relationship |
| a suggested insertion or deletion | `w:ins` or `w:del`, carrying the author and the date |
| a comment | a range start and end, plus an entry in the comment part |
| a dynamic field | a field run: page, page count, date or title |

| What is lost | What the reader or the writer does |
|---|---|
| a custom style Word has no name for | falls back to the normal style, and warns (`PROSE-IO-006`) |
| a background colour outside the eight | approximated, and warned |
| the automatic heading numbering | not written: Word numbers through its own fields, and the writer warns |
| a picture in a metafile or a tag image format | skipped and warned: neither decodes in the client |
| a Word checkbox control | read as a to-do item when it is recognisable, warned when it is not |
| a picture inside a cell | lifted beside the table, and the lift is reported (`PROSE-IO-026`) |

A page break and a horizontal rule are distinct in Word, and both survive.

**What the mapping carries today.** The form holds a flag where a style, a colour or a page holds
values. So the mapping writes and reads the structures below, and the report names the rest:

| Construct | Export | Import |
|---|---|---|
| a paragraph, a heading of level 1 to 9, a nested list, a table, a code block, a page break | carried | carried; a title or a quotation reads as a styled paragraph |
| a footnote | carried | carried; an endnote reads as a footnote, approximated |
| a comment thread, a suggestion | carried, when the user asks for the review (`PROSE-IO-081`) | carried as a thread and a suggestion (`PROSE-IO-060`) |
| the page | approximated: an A4 page | the flag alone |
| a to-do item, inline code, merged cells, a list past nine levels | approximated: a bullet, a monospace run, single cells, the ninth level (`PROSE-IO-006`) | -- |
| a style, a colour, a size, a family, a cell format, a header and a footer | dropped | the flag alone |
| a picture | carried, with its bytes, its size and its centring (`PROSE-IO-106`) | carried: stored through the media flow, then written as a picture block (`PROSE-FEAT-319`) |
| a code language, a rule, a table of contents, a cross reference | dropped | -- |

An imported thread and an imported suggestion carry the name the file gives. They also carry the
importing user as the one who wrote them into the document (`PROSE-IO-079`). An annotation typed
by hand carries the session's identity the same way (`PROSE-FEAT-262`). A comment with no date
lands at the time the user chose the file, and the form marks its date as unknown
(`PROSE-IO-083`). A session with no identity stamps none: each annotation is dropped and
reported, and its text kept (`PROSE-IO-080`, `PROSE-IO-082`). A footnote call is not built on
import: the builder reports it.

**Dates.** Word writes `w:date` as the wall time of the author's clock, with a `Z` that does not
mean UTC. A recent Word writes the true instant beside it, in `w16du:dateUtc`. The reader takes
that instant when the part gives one.

Otherwise the reader reads the wall time under the rules of the session's time zone on that date,
summer time included. The report names the date approximated (`PROSE-IO-084`). The writer
writes both: the wall time under those rules, and the instant
(`PROSE-IO-085`).

The zone is an IANA name, such as `Europe/Paris`, and an explicit parameter of the import and of
the export. The session injects it as it injects its identity, and no reader or writer reads it
from the machine. Only the name crosses to the Office module, which holds the database: the Main
instance links none. A name the database does not hold fails the conversion, never read as UTC.

**Run effects.** Small capitals and all capitals have no form, and are reported dropped. Hidden
text keeps its text and a double strikethrough lands as a single one, both reported approximated
(`PROSE-IO-086`). The form flags a styled run, paragraph or cell and carries no value of its look.
So the builder names each on import (`PROSE-IO-087`).

**The review on the way out.** The plain Word export carries no thread and no suggestion. Its text
is the text before the review: a suggested insertion is left out, and a suggested deletion stays.
A second entry exports with the review (`PROSE-IO-081`).

**The OpenDocument text codec** (`crates/ambre-odt`). Two pure functions over the Word model of
`ambre-docx`, on the OpenDocument package of `ambre-opc`. The model is Word's on purpose. An
`.odt` and the same text saved as Word meet one mapping onto the form and one report. So the
second format cannot drift from the first (`PROSE-IO-088`).

The reader names each loss in the words of the Word reader. Both readers take a paragraph's role
from one list of style names, `ParagraphRole::from_style_name`. The list knows the names
LibreOffice writes in both formats.

```text
Import:  bytes -> Office module: ambre_odt::read, then word_read -> ProseIr + report -> Prose
Export:  Prose -> ProseIr -> Office module: word_write, then ambre_odt::write -> bytes + report
```

| OpenDocument | The Word model |
|---|---|
| `text:p`, `text:h` | a paragraph; a heading at its outline level, else the role its style's name gives |
| `text:span` | a run in the format of its text style and the styles it derives from |
| a paragraph's automatic style | the direct format of its runs; a named style stays off the run, as in Word |
| `text:a` | a link: `#name` names a bookmark, any other address is kept as text |
| `text:list` | a list paragraph: its level the nesting, its kind the level of the list style |
| `text:continue-list`, `xml:id` | the same list, its numbering continued |
| `table:table` | a table: a column span, a row span as a vertical merge, the header rows, the widths, a fill |
| `text:note` | a footnote or an endnote, by its class |
| `office:annotation`, `office:annotation-end` | a comment and the range it covers; `loext:parent-name` makes a reply, `loext:resolved` resolves it |
| `text:change-start` to `text:change-end` | a tracked insertion over the range, which may span paragraphs |
| `text:change` | a tracked deletion: the text its region removed, at the mark |
| `fo:break-before`, `fo:break-after` | a page or a column break |
| `draw:frame` holding `draw:image` | a picture in the line; any other anchor is approximated as one |
| the first master page | the page, the default header and the default footer |
| `dc:title` of `meta.xml` | the title |

What the reader does with a construct the model has no form for, in the Word reader's words:

| Construct | Outcome |
|---|---|
| a field, an index or a table of contents | approximated: the result it last showed |
| a section | approximated: its content, in the flow |
| a picture anchored to the page, the paragraph or a character | approximated: a picture in the line |
| a numbering format outside the five of the model | approximated: decimal |
| a deletion of several paragraphs | approximated: their text in the paragraph of the mark |
| a tracked format change | dropped: the format now in force is kept |
| a shape, a text box, an embedded object, a bookmark | dropped |
| a header or a footer for a first or a left page | dropped: the default band is kept |
| a picture linked from outside the package | dropped, and never fetched (`PROSE-IO-094`) |
| a macro library or a script | dropped, and never read (`PROSE-IO-093`) |
| an element the reader does not know | dropped, and its text kept (`PROSE-IO-095`) |

ODF collapses white space in a paragraph: a run of spaces, tabs and line ends reads as one space,
and none at the start. `text:s`, `text:tab` and `text:line-break` carry the rest, and the writer
writes them. LibreOffice writes a date with no zone, which is a wall time. It reads in the session's
zone and is reported approximated, as a Word `w:date` is. A date with a zone names the
instant.

The reader bounds a hostile file with the package bounds of `ambre-spec-budgets.md section 6`
and the nesting bound of the Word reader (`PROSE-IO-092`). A repeat or a span attribute counts
1024 at most.

The writer writes the parts LibreOffice opens: `content.xml`, `styles.xml`, `meta.xml` and the
pictures. Each automatic style takes its name in the order the walk first needs it, so the same
document writes the same bytes (`PROSE-IO-091`). A comment and a changed region go by a name the
writer derives, so a read back numbers the ids again (`PROSE-IO-090`). A page break inside a
paragraph splits it, and the second half breaks before itself. Read back, the form holds the same
blocks.

**The pairs the rule is held on.** Each pair is one source written twice. LibreOffice writes
`features.odt` and `features.docx` from one flat ODF source. pandoc writes `inline.odt` and
`structure.odt` from Markdown, and LibreOffice saves each as Word. pandoc's own Word writer is not
the twin: it writes no page and no band where its OpenDocument writer writes both.

The comparison reads the form and the report. It joins two neighbour runs of the same marks. It
drops one leading tab of a Word note body, which LibreOffice writes after the note mark and the
source does not hold.

**What a Markdown file becomes.** The flavour decides the table, the to-do item and the footnote;
everything else is common:

| Block | Markdown |
|---|---|
| a paragraph, a heading | a paragraph, a hash run |
| a list item | a dash or an ordinal, nested by two spaces |
| a to-do item | a dash and a bracket, in the extended flavour alone |
| a table | a pipe table, in the extended flavour alone |
| a code block | a fence carrying the language |
| a picture | nothing, reported: the file could name no address |
| a page break | a rule, and a warning: the format has no page (`PROSE-IO-018`) |
| a table of contents | a comment marker, which no reader renders |
| a footnote | a reference and a definition, in the extended flavour alone |

Markdown has no superscript, subscript, colour, size, family, comment, suggestion or cross
reference, and a picture's width, height, alignment and caption have no syntax either. Each one is
a warning at the drop site, never a silent omission (`PROSE-IO-018`, `PROSE-IO-004`).

A peer writes a link's target straight into the document, unchecked. The writer keeps a link only
when the host may open its target, and exports any other as its text alone (`PROSE-IO-067`). A
target longer than a copy writes is exported as its text alone too (`PROSE-IO-069`). A
target it keeps is percent-encoded where Markdown would read it as syntax, so it cannot end its
destination, its paragraph or its block (`PROSE-IO-068`).

**What an HTML file becomes.** A standalone document, with the stylesheet in the head or inline
(`PROSE-IO-063`), and no script in it at all (`PROSE-IO-064`):

| Block | Element |
|---|---|
| a paragraph, a heading | `p`, `h1` to `h6` |
| a list item | `li`, under `ul` or `ol`, and under a to-do list with a checkbox |
| a table | `table`, with a head section when the table has a header row |
| a code block | `pre` and `code`, carrying the language as a class |
| a picture | `figure` holding `img`, its bytes inline as a `data:` address (`PROSE-IO-107`), and its caption as `figcaption` |
| a page break, a rule | a rule carrying a page-break class, `hr` |
| a table of contents | a labelled `nav` holding an ordered list of the headings, approximated |
| a footnote | a raised anchor to an entry of a closing list |

A mark becomes its element: strong emphasis, emphasis, underline, strike-through, superscript,
subscript, code, anchor. A colour, a background, a size or a family becomes an inline style. A
suggestion becomes an insertion or a deletion carrying its identifier and its author, and a
comment a span carrying the identifiers of its threads. The threads follow the text, each body
with its author. Both are gated by the export options (`PROSE-IO-022`). The header and the footer
have no element at all, so the writer reports them (`PROSE-IO-019`).

The head holds a content security policy that refuses every script and every load. The writer
writes none, so the policy only backs up the escaping should it fail (`PROSE-IO-064`).

**HTML import is the paste path.** The same reader serves a file the user chose and the rich half
of a paste. A paste from a web page and an import of the same page produce the same blocks
(`PROSE-IO-025`, `features/ambre-spec-prose-clipboard.md`). It is the one leg of this feature a
user waits on, which is why it carries its own budget row.

The walk reads three things more for an import, and a paste ignores them. It reads the list
nesting of each block, so a list inside an item nests under it. It names each element it drops or
flattens (`PROSE-IO-098`). It refuses a tree nested past its bound, where a paste reads it as
nothing (`PROSE-IO-010`). The builder checks each look value as a paste checks it, so the two
paths write the same marks.

| HTML | What lands, and what the report names |
|---|---|
| a script, a handler | nothing; a script, dropped (`PROSE-IO-016`) |
| a style element, a linked stylesheet, a preloaded font | nothing; a stylesheet or a font, dropped (`PROSE-IO-062`) |
| a frame, an object, an embed, a canvas, a media element, a drawing | nothing; embedded content, dropped |
| a button, a text area, a list box | nothing; a form control, dropped |
| a picture | nothing; unresolved when it names an address, dropped otherwise (`PROSE-IO-054`) |
| a table | a paragraph per cell; a table, approximated |
| a rule, a block quote | nothing, and a paragraph; each dropped |
| an element the walk does not know | its text; HTML, dropped (`PROSE-IO-005`) |

**A remote address is not fetched.** A Markdown or an HTML picture may name a URL rather than a
content hash. The Logic thread never opens a socket. A fetch on the import path would turn
opening a file into a signal to a third party. So the picture lands unresolved and the report names
it (`PROSE-IO-023`).

**The crates each path uses:**

| Crate | Role | Declared |
|---|---|---|
| `ambre-docx` | reads and writes the Word format, in pure Rust: the codec above | yes, in the Office module |
| `ambre-prose-form` | the intermediate form, and the Word mapping behind its `word` feature | yes; Prose links the form, and the Office module the mapping |
| `ambre-opc` | the package layer the three OOXML codecs share | yes, in the Office module |
| `ambre-odt` | reads and writes an OpenDocument text onto the Word model: the codec above | yes, in the Office module |
| `jiff`, its IANA database bundled | the rules of the session's time zone, for a Word date | yes, behind the `word` feature, in the Office module |
| `zip`, `quick-xml` | the package, under `ambre-opc` alone, and the XML of each part | yes |
| `pulldown-cmark` | reads Markdown, tables, footnotes and strikethrough included, in the markup converter module | in `ambre-convert` |
| `pulldown-cmark-to-cmark` | writes an event stream back as Markdown | no |
| `html5ever` | reads HTML that may be malformed, for a paste and an import, in the markup converter module | in `ambre-convert` |
| `markup5ever_rcdom` | the document object the HTML reader walks | no: the module builds its own arena |
| `ambre-core` | the identifier and the document-type byte | yes |
| `postcard`, `serde` | the fixtures, and nothing on the bus | yes |

`crates/ambre-prose` declares no reader and no writer. The markup converter module runs the
Markdown reader and writer, with their mapping onto the intermediate form. It also runs the walk
that reads the HTML half of a paste into the blocks a copy writes. The Office module runs the Word
codec and the Word mapping.

The Main instance bounds the file, hands the bytes over and builds what comes back. It links no
parser and no mapping, which holds its download under its ceiling (`ambre-spec-architecture.md
section 2`, the converter modules). Each module has its own row in `ambre-spec-budgets.md section
5`.

**Code:** the Word codec is `crates/ambre-docx`, over the package layer `crates/ambre-opc`. The
form and the Word mapping are `crates/ambre-prose-form`. The Markdown mapping is
`crates/ambre-convert/src/engine/prose/md_read.rs` and `md_write.rs`.

The HTML pair sits beside them: `html_read.rs`, the walk a paste shares, then `html_doc.rs` and
`html_write.rs`. `crates/ambre-prose/src/interop/word.rs`, `interop/markdown.rs`,
`interop/html.rs` and `logic/conversion.rs` route each import and export through its module.
`logic/sliced.rs` runs the half that sees the document one slice per pass. The native pair is
`crates/ambre-prose/src/logic/docfile.rs`. The dispatcher in
`crates/ambre-logic/src/data.rs` serves the save, because the header needs the document id the
registry holds.

---

## 4. SURFACE

**The save entry is in the command palette** (`PROSE-IO-028`), declared in
`crates/ambre-prose/src/commands.rs`. A native file opens from the home screen, like a Trame or a
Toile file (`PROSE-IO-065`). The palette imports and exports Markdown, Word, OpenDocument
text and HTML. It exports Word, OpenDocument and HTML with the review from an entry of their own
(`PROSE-IO-027`, `PROSE-IO-081`, `PROSE-IO-022`).

| Trigger, once it exists | What must happen |
|---|---|
| the open entry, or a file dropped on the page | the file is read and applied as one batch (`PROSE-IO-051`) |
| the save entry | the Logic thread serializes and answers with the bytes and their media type |
| the export entry, per format | a dialog collects the options, then the same path runs |
| the answer | the view hands the bytes to the browser; the document does not change (`PROSE-IO-017`) |
| the report | what the conversion lost, named construct by construct (`PROSE-IO-004`) |

**The two options a reader must see before an export.** The comments and the suggestions travel in
a Word and in an HTML file. A document handed to a third party carries the review with it
unless the export leaves it out. The dialog states both, and the writer honours both
(`PROSE-IO-022`).

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| Every conversion runs on the user's machine | converting on the server | it is faster to build and hands the relay the whole document. The pillar is that the server holds ciphertext; a conversion is the one moment a document exists as plaintext outside the editor, so it is the one that must not leave the device |
| One intermediate form between every reader and every writer | writing commands straight out of each reader | it gives no place to validate, no place to report, and nothing to roll back. Four formats read and written directly would be twelve converters and twelve places to forget a warning; one form makes a format a reader plus a writer, and the loss report a property of the form rather than of a pair |
| The reader emits no command, and the writer reads no document | letting each writer read the document | it multiplies the walk by the format count and couples every writer to the CRDT. A reader is bytes to a value, a writer is a value to bytes, and both are pure functions on the host, which is what makes a conversion testable with no engine and keeps the walk one budget row for four formats |
| The native file is the only lossless path | promising fidelity on Word | Word has no place for a Prose custom style and numbers its own headings. Every other format has a model of its own, so a round trip through it is a conversion twice; the native payload is the document itself |
| The native path skips the intermediate form | routing the native file through the form too | it trades the one lossless path for symmetry. The payload is a snapshot, and a snapshot through an intermediate form would be a conversion of the document into a subset of itself; the form exists to lose things safely, and this path loses nothing |
| The container is Foundation's, not Prose's | a Prose header | it is what the corpus had, and it made each new application restate the layout. The header carries a document-type byte and re-dispatches across applications, so it is a contract between three applications and belongs to none of them |
| A picture is an address, and an import never fetches one | fetching a remote picture during the import | it puts a network call in the thread whose whole contract is that it makes none. The Logic thread holds no key and opens no socket, and a fetch on the import path would tell a third party which document a user just opened, before the user has seen it |
| A block the tree refuses is lifted, never dropped | refusing the file | it fails on an ordinary Word document. A file the reader opens is not a command they typed, so losing part of it in silence loses content they still have and cannot see is gone; lifting keeps it, and the report says where it went |
| A block the tree refuses is lifted, never dropped | dropping the block | it is the silent loss this pipeline exists to make impossible. A file the reader opens is not a command they typed, so losing part of it in silence loses content they still have and cannot see is gone; lifting keeps it, and the report says where it went |
| A wildcard arm warns before it drops | matching the rest and moving on | it is what makes a lossy import look like a faithful one. A reader meets constructs its author never saw, and the honest answer is to name them; an arm that drops in silence turns every future format extension into an invisible data loss |
| An export is idempotent | writing in map order | it is stable within one run and different across builds. A user who exports twice and gets two different files cannot tell a real change from a serialization detail, and neither can a test; fixing the order of every attribute is what makes the round trip provable at all |
| The comments and the suggestions are opt-in on the way out | exporting them always | it publishes the review with the document and is discovered by the reader on the other side. A review is internal by default: a document sent to a client should not carry who doubted which sentence, so the safe direction is the one that needs an explicit act to publish |
| A report accompanies a conversion that finished, and an error replaces one that did not | one error type | it makes "the file opened with three warnings" and "the file did not open" the same value. The two are different facts and a caller acts on them differently: one is shown beside a document, the other instead of it; merging them forces every caller to ask which case it is holding |
| A progress event after every slice but the last | reporting progress always | it spends an event per small import. A conversion that fits one slice answers in the pass of its command, and needs no progress bar |
| A progress event after every slice but the last | reporting progress never | it is the freeze. A conversion that takes ten seconds with no sign of life reads as a frozen document |
| A progress event after every slice but the last | a threshold of one megabyte of file | an export learns its size only once it is written. A slice is what the thread holds, so it is the unit the user waits on in both directions |
| The module reads and writes a file in one call, and the half that sees the document runs in slices | a reader and a writer that yield between slices | the module call is a few percent of a large conversion, by the measure of the `Prose import or export, one slice` row (`ambre-spec-budgets.md section 1`). A reader that yields would rewrite every codec for those few percent |
| A command that writes lands the running import first | an edit between two slices of the import | the history groups the commits of one command, never two interleaved ones. The edit would join the step of the import, or split it in two, and one undo would no longer take the file back (`PROSE-IO-051`) |
| A command that writes lands the walk of an export first | walking a fork of the document | a fork copies the whole document in one call, the cost the slices remove |
| Word and the clipboard are the two Google Docs paths | calling the Google API from the client | it trades a metadata leak for a convenience the download already gives. There is no way to read a Google document from the client without an authenticated network call to Google, which leaks which document is opened and when; a download and a paste both arrive as formats already read |
| The Word codec is its own crate over `zip` and `quick-xml` | `docx-rs` 0.4.22, for both directions or for the writer alone | measured on 2026-11-09 over 17 real files from Word and LibreOffice. Its writer panics on 6 of them, on an `unreachable!` for a field instruction, and a panic kills the worker. Its public model hides the text of a deletion, and a rewrite doubles a comment. It links `image`: 505 KB of brotli wasm for a read and a write, against 147 KB for this codec. Its reader names no loss, so the report could not count what it skips |
| An OpenDocument text reads into the Word model | a model of ODF of its own, mapped onto the form | it is a second mapping and a second report, and `PROSE-IO-088` would hold two pieces of code to one answer. The constructs Prose reads are the same in both formats, and each ODF construct the Word model lacks is one the Word reader already names |
| The OpenDocument text codec is its own crate over the shared package | `ferrodoc-odt` 1.0.1, `odox-core` 0.16.0 or `unfair-codec-odf` 0.1.0 | measured on 2026-11-11, read and write in one test cdylib on wasm32 at opt-level "z" with LTO: 188 KB, 154 KB and 157 KB of brotli, against 182 KB for this codec alone and less once it shares the Office module's zip, XML and Word mapping. Each brings a zip of its own version, and each reads a 70-page text in 9 to 12 ms, as this codec does. None carries a comment or a tracked change: `ferrodoc-odt` drops both, `unfair-codec-odf` has no form for either, and `odox-core` is an editable tree with no model of either. None names what it drops |
| The Word codec is its own crate over `zip` and `quick-xml` | the codec inside `ambre-prose` | Prose links into the Main instance, and the codec belongs in the converter module fetched on demand. A crate of pure functions moves there whole |
| A run takes its own format and its character style | resolving the paragraph style into each run | every run of a heading would carry the heading's bold and size, and every run of the body the default font. The role carries the paragraph style once |
| A part with a document type declaration is refused | expanding its entities | an entity may name a file outside the package or expand without bound. Word refuses one too, so no file Word wrote carries it |
| The HTML reader is the paste reader | a paste-specific reader | it is a second HTML implementation with a second set of losses. A paste from a web page and an import of that page are the same bytes in the same format, so two readers would be two results for one input, and the second one to drift would be the one nobody tested |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A Word note body opens on a separator the reader keeps.** LibreOffice writes a tab after the
  note mark of a Word note, and Word writes a space. The Word reader keeps either as text, so a note
  imported from Word starts with it, and one imported from OpenDocument does not. The oracle of
  `PROSE-IO-088` drops it on the Word side, and no row states which reading is right.
* **A deletion that spans paragraphs splits on its way to OpenDocument.** ODF writes one region per
  run of removed text, so one Word deletion over two paragraphs leaves as two, and reads back as
  two suggestions. No fixture holds one, and no row reports the split.
* **A header row below a body row is written as a body row.** ODF holds the header rows at the top
  of a table, and Word does not. The writer reports nothing.
* **No dialog collects the export options.** The palette offers one entry per choice, and the
  Word export with its review is the second entry. The PDF entry exports in the document's
  language.
* **Where an import lands is not ruled.** A failed import leaves the document untouched
  (`PROSE-IO-011`). Nothing says what a successful one does to a document that already has
  content: replace it, append to it, or insert at the caret. Toile answers the same question with
  `TOILE-IO-023` and Prose states nothing, so the batch of step 5 has no defined destination.
* **A slice counts steps, not their cost.** A table is one operation, whatever its cells, and so is
  a paragraph of a hundred thousand characters. One such step holds its pass past the slice row, and
  no rule splits it.
* **An edit waits for the rest of the import.** A command that writes this document lands the
  whole remainder first (`PROSE-IO-100`), so a key typed during a large import waits for it. Only the
  other documents and the commands that write nothing are served between two slices.
* **No running conversion can be cancelled.** No row asks for it, and no surface offers it.
* **The Markdown bound is small.** An import holds about 45 times its file until it ends, and the
  rehearsal's scratch document holds about half of that. The bound is derived from that ratio, so a
  Markdown or HTML file past 2 MiB is refused. A long novel in Markdown is about 1 MB.
* **A named style's character defaults have no home in the document.** The intermediate form
  carries them, because a Word style carries them, and the style register of
  `ambre-spec-prose-schema.md section 3` holds paragraph fields alone. A Word file whose "Quote"
  style is italic therefore imports as a style that is not italic, and no warning covers it. The
  loss is in the command builder, not in the reader.
* **A comment's author survives an export and not a rename.** The document stamps a display name
  and a document peer, and the Word format carries an author string. The round trip is faithful
  to what Prose stores. It stores no stable user identity, which
  `features/ambre-spec-prose-comments.md section 6` records, and an export freezes whatever name
  the author had that day.
* **A cross reference crosses formats as a bookmark and comes back by name.** The document holds a
  block identifier, and Word holds a generated bookmark. Nothing states how the two are joined on a
  round trip through a file that a third party edited in between.
* **Nothing bounds an import but the Word package.** Toile clamps foreign geometry, bounds the
  element count and bounds the nesting depth (`TOILE-IO-002`). The Word reader bounds the parts,
  their size and their nesting (`PROSE-IO-072`), and a Markdown file is bounded by its size alone. A
  file with a million blocks has no stated refusal. The atomicity rule means the whole batch is
  held in memory before any of it is applied.
* **Three reader crates are a plan.** `crates/ambre-prose` declares none of them, so the
  dependency table above marks each one absent rather than reading as a description of the
  manifest. The Word codec is built, and no gesture reaches it.
