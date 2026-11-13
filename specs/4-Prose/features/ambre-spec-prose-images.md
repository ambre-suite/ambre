# AMBRE -- FEATURE: IMAGES

**Type:** Vertical (feature)

> A picture in the document. The document holds the picture's address and its display size. The
> pixels travel apart, through the shared media flow, and the server that stores them never sees
> them.

The transfer and the encryption are Foundation's (`ambre-spec-message-bus.md`), and
this document restates neither. It owns the block, the resolution of an address into pixels, the
cache that holds them and what an export does when they are missing.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-205` | `INV` | The document MUST hold a picture as an address, never as its pixels. | this spec section 3 | `ambre_prose::logic::pictures::tests::inserting_writes_one_picture_block_with_its_address_and_size` -- the block holds the address, and the document grows by under 2 000 bytes for a picture of 80 000 bytes of samples | none | verified |
| `PROSE-FEAT-206` | `INV` | The address of a picture MUST be the address the media flow gives its plaintext, so two copies of one picture in one document resolve to one address. | this spec section 3; `SYNC-050` | `ambre_prose::logic::picture_interop_tests::an_import_lands_once_every_picture_is_stored` -- two copies of one picture take one upload and land under one address | none | verified |
| `PROSE-FEAT-052` | `INV` | A picture MUST be a child of the document root: neither a table cell nor a list item MAY hold one. | this spec section 3; `ambre-spec-prose-schema.md section 4` | `ambre_prose::logic::pictures::tests::an_insertion_under_a_cell_or_a_list_item_is_refused_and_named` -- an insert under a cell and under a list item each write nothing | none | verified |
| `PROSE-FEAT-054` | `INV` | The decoded pictures MUST be held under a cap measured in decoded bytes, not in entries. | this spec section 3; `ambre-spec-budgets.md section 6` | `ambre_prose::picture::tests::the_cap_counts_decoded_bytes_not_entries` | none | verified |
| `PROSE-FEAT-207` | `INV` | A picture's resolution state MUST NOT be written into the document: it is a fact of the session alone. | this spec section 3 | `ambre_prose::logic::tests::the_picture_cache_and_the_history_never_enter_the_document`; `ambre_prose::logic::pictures::tests::a_failed_download_writes_nothing_and_is_not_asked_again` -- a picture decoded and a picture failed leave the snapshot as it was | none | verified |
| `PROSE-FEAT-057` | `INV` | A picture that is absent, undecryptable or undecodable MUST NOT be fetched again without a reader's gesture. | this spec section 3 | `ambre_prose::picture::tests::a_failed_picture_is_not_asked_again_by_a_paint`; `ambre_prose::logic::pictures::tests::a_failed_download_writes_nothing_and_is_not_asked_again` -- a later layout asks nothing for the failed address | none | verified |
| `PROSE-FEAT-208` | `CONC` | Two peers resizing one picture at the same moment MUST converge on one display size, and both MUST hold that one. | this spec section 3 | `ambre_prose::logic::pictures::tests::two_peers_resizing_one_picture_converge` -- the two clocks differ, and both peers hold one of the two sizes whole | none | verified |
| `PROSE-FEAT-209` | `IO` | An alignment MUST come back from a round trip as it was written, including the two the layout does not honour. | this spec section 3 | `ambre_prose::logic::pictures::tests::every_alignment_survives_a_round_trip` -- each of the four through the document's bytes | none | verified |
| `PROSE-FEAT-317` | `OBS` | A picture wider than the text MUST be drawn at the text's width, its ratio kept. | this spec section 3 | `ambre_prose::picture::tests::a_picture_wider_than_the_column_is_scaled_to_it`; `ambre_prose::view::paint::tests::a_picture_wider_than_the_text_is_drawn_at_its_width` | none | verified |
| `PROSE-IO-108` | `IO` | A picture MUST come back from a Word and an OpenDocument export with its bytes, its alternative text, its size and its centring. | this spec section 3; `features/ambre-spec-prose-import-export.md section 3` | `ambre_prose::logic::picture_interop_tests::a_picture_comes_back_from_a_word_and_an_opendocument_export` | none | verified |
| `PROSE-UI-046` | `OBS` | A picture that failed for good MUST paint a mark distinct from the loading placeholder. | this spec section 4 | `ambre_prose::view::paint::tests::a_failed_picture_paints_a_mark_the_placeholder_does_not` | none | verified |
| `PROSE-UI-145` | `OBS` | A picture that failed for good MUST offer the gesture that asks for it again. | this spec section 4 | `ambre_prose::input_tests::a_press_on_a_failed_picture_asks_for_it_again` -- a press on it sends the retry and moves no caret | none | verified |
| `PROSE-UI-146` | `OBS` | A picture not yet resolved MUST paint a neutral placeholder carrying its alternative text. | this spec section 4 | `ambre_prose::view::paint::tests::a_waiting_picture_paints_a_placeholder_with_its_alternative_text` | none | verified |

### 1.2 Insert a picture

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-210` | `POST` | Inserting MUST write one picture block at the named place in the tree, holding the address and the display size the command carried. | this spec section 3 | `ambre_prose::logic::pictures::tests::inserting_writes_one_picture_block_with_its_address_and_size` | none | verified |
| `PROSE-FEAT-211` | `FAIL` | An insertion naming a parent that is not the document root MUST be refused, and MUST be reported to the author. | this spec section 3 | `ambre_prose::logic::pictures::tests::an_insertion_under_a_cell_or_a_list_item_is_refused_and_named` -- each insert answers the nesting refusal | none | verified |
| `PROSE-FEAT-212` | `FRAME` | Inserting a picture MUST NOT change the text of any block, the order of the blocks around it, or the decoded cache. | this spec section 3 | `ambre_prose::logic::pictures::tests::inserting_a_picture_changes_no_text_and_no_order` | none | verified |
| `PROSE-FEAT-316` | `POST` | Inserting from a file MUST store the bytes through the media flow before it writes anything. | this spec section 3 | `ambre_prose::logic::pictures::tests::a_picture_from_a_file_is_written_once_its_address_comes_back` -- the upload carries the file, and the document holds one block until the answer | none | verified |
| `PROSE-FEAT-324` | `POST` | The block of a picture from a file MUST be written once its address comes back, holding that address. | this spec section 3 | `ambre_prose::logic::pictures::tests::a_picture_from_a_file_is_written_once_its_address_comes_back` -- the answer writes the block, with the address, the native size and the text | none | verified |
| `PROSE-FEAT-318` | `FAIL` | A file the media flow did not store MUST be reported to the author, and MUST write nothing. | this spec section 3; `SYNC-076` | `ambre_prose::logic::pictures::tests::a_file_the_media_flow_did_not_store_writes_nothing_and_is_named` | none | verified |

### 1.3 Resize a picture

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-058` | `FAIL` | A resize to a width or a height of zero MUST be refused. | this spec section 3 | `ambre_prose::picture::tests::a_zero_side_is_refused_and_a_huge_one_clamped`; `ambre_prose::logic::pictures::tests::a_resize_to_zero_is_refused_and_a_huge_one_stored_clamped` | none | verified |
| `PROSE-FEAT-059` | `POST` | A display dimension past the maximum MUST be clamped to the maximum. | this spec section 3; `ambre-spec-budgets.md section 6` | `ambre_prose::picture::tests::a_zero_side_is_refused_and_a_huge_one_clamped`; `ambre_prose::logic::pictures::tests::a_resize_to_zero_is_refused_and_a_huge_one_stored_clamped` | none | verified |
| `PROSE-FEAT-213` | `POST` | The document MUST hold the clamped value, never the value the gesture asked for. | this spec section 3 | `ambre_prose::logic::pictures::tests::a_resize_to_zero_is_refused_and_a_huge_one_stored_clamped` | none | verified |
| `PROSE-FEAT-214` | `POST` | With the aspect locked, the dimension that moved most against the stored size MUST be the one the other is derived from. | this spec section 3 | `ambre_prose::picture::tests::a_locked_resize_follows_the_side_that_moved_most`; `ambre_prose::logic::pictures::tests::a_locked_resize_takes_the_decoded_ratio_and_touches_nothing_else` | none | verified |
| `PROSE-FEAT-215` | `POST` | The derived dimension MUST be taken from the decoded picture's own ratio, or from the stored size when nothing is decoded yet. | this spec section 3 | `ambre_prose::picture::tests::a_locked_resize_follows_the_side_that_moved_most`; `ambre_prose::logic::pictures::tests::a_locked_resize_takes_the_decoded_ratio_and_touches_nothing_else` | none | verified |
| `PROSE-FEAT-216` | `FRAME` | A resize MUST NOT change the picture's address, its alignment, its alternative text or its caption. | this spec section 3 | `ambre_prose::logic::pictures::tests::a_locked_resize_takes_the_decoded_ratio_and_touches_nothing_else` | none | verified |

### 1.4 Align a picture

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-217` | `POST` | Aligning MUST store the alignment the command named, whether or not the layout honours it. | this spec section 3 | `ambre_prose::logic::pictures::tests::aligning_stores_the_alignment_and_nothing_else` | none | verified |
| `PROSE-FEAT-218` | `FRAME` | Aligning MUST NOT change the picture's display size, its address, or the text around it. | this spec section 3 | `ambre_prose::logic::pictures::tests::aligning_stores_the_alignment_and_nothing_else` | none | verified |

### 1.5 Put text on a picture

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-219` | `POST` | The text such a command carries MUST be stored on the field it names, on that block alone. | this spec section 3 | `ambre_prose::logic::pictures::tests::a_text_lands_on_its_field_alone` | none | verified |
| `PROSE-FEAT-220` | `FRAME` | Setting the alternative text or the caption MUST NOT change the picture's address, its display size or its alignment. | this spec section 3 | `ambre_prose::logic::pictures::tests::a_text_lands_on_its_field_alone` | none | verified |

### 1.6 Ask for a failed picture again

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-221` | `POST` | Asking again MUST put the entry back to the unasked state and issue the download once more. | this spec section 3 | `ambre_prose::logic::pictures::tests::asking_again_puts_the_picture_back_and_downloads_it_once_more` | none | verified |
| `PROSE-FEAT-222` | `FRAME` | Asking again MUST NOT write to the document, and MUST NOT touch the resolution state of another address. | this spec section 3 | `ambre_prose::logic::pictures::tests::asking_again_puts_the_picture_back_and_downloads_it_once_more` | none | verified |

### 1.7 Evict a decoded picture

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-223` | `POST` | Eviction MUST take the least recently painted entries first. | this spec section 3 | `ambre_prose::picture::tests::eviction_takes_the_least_recently_painted_and_stops_when_it_fits` | none | verified |
| `PROSE-FEAT-286` | `POST` | Eviction MUST stop as soon as the total fits the cap. | this spec section 3 | `ambre_prose::picture::tests::eviction_takes_the_least_recently_painted_and_stops_when_it_fits` | none | verified |
| `PROSE-FEAT-224` | `POST` | Eviction MUST leave one entry standing, even when that entry alone is over the cap. | this spec section 3 | `ambre_prose::picture::tests::one_entry_stands_even_over_the_cap` | none | verified |
| `PROSE-FEAT-055` | `FRAME` | A picture whose transfer is in flight MUST NOT be evicted. | this spec section 3 | `ambre_prose::picture::tests::a_transfer_in_flight_is_never_evicted` | none | verified |
| `PROSE-FEAT-056` | `FRAME` | Eviction MUST NOT touch the document: the address MUST stay on the block. | this spec section 3 | `ambre_prose::logic::pictures::tests::an_eviction_writes_nothing_to_the_document` | none | verified |

### 1.8 Export a document holding pictures

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-IO-110` | `POST` | An export MUST ask the media flow once for each address the document holds. | this spec section 3 | `ambre_prose::logic::picture_interop_tests::an_export_waits_for_its_pictures_and_serves_other_commands` -- two blocks of one address ask one download | none | verified |
| `PROSE-IO-007` | `POST` | An export MUST embed the bytes the media flow answered for each picture. | this spec section 3 | `ambre_prose::logic::picture_interop_tests::a_picture_comes_back_from_a_word_and_an_opendocument_export` -- the bytes answered are the bytes embedded | none | verified |
| `PROSE-IO-008` | `POST` | A picture whose bytes did not come back MUST become a placeholder in the file and a line in the report. | this spec section 3 | `ambre_prose::logic::picture_interop_tests::a_picture_that_did_not_come_back_is_a_placeholder_and_a_report_line`; `ambre_prose::logic::print::tests::a_picture_without_its_bytes_is_a_placeholder_in_the_pdf` | none | verified |
| `PROSE-IO-040` | `POST` | An export MUST serve other commands while its pictures are fetched: the fetch MUST NOT block the Logic thread. | this spec section 3; `ARCH-030` | `ambre_prose::logic::picture_interop_tests::an_export_waits_for_its_pictures_and_serves_other_commands` -- a command typed during the wait writes | none | verified |
| `PROSE-IO-041` | `FRAME` | An export MUST NOT write to the document it reads. | this spec section 3 | `ambre_prose::logic::picture_interop_tests::an_export_waits_for_its_pictures_and_serves_other_commands` -- the snapshot is unchanged by the start and the wait | none | verified |
| `PROSE-IO-105` | `POST` | An export MUST walk the document as it stands once its pictures answered, and a picture added during the wait MUST become a placeholder. | this spec section 3 | `ambre_prose::logic::picture_interop_tests::a_picture_added_during_the_wait_is_a_placeholder` | none | verified |
| `PROSE-IO-106` | `POST` | An export MUST hold its picture bytes under the export cap, and a picture past it MUST become a placeholder and a line in the report. | this spec section 3; `ambre-spec-budgets.md section 6` | `ambre_prose::logic::pictures::tests::a_picture_past_the_export_cap_is_let_go` | none | verified |
| `PROSE-IO-107` | `POST` | A PDF export MUST draw each picture whose bytes came back in the box the page gives it. | this spec section 3; `features/ambre-spec-prose-pdf-export.md section 3` | `ambre_prose::logic::print::tests::a_printed_page_draws_a_fetched_picture` -- one encoded picture drawn, its bytes the ones answered, and the written file holds an image; `ambre_prose::logic::print::tests::a_printed_picture_stands_as_it_was_taken` | none | verified |
| `PROSE-IO-109` | `POST` | An HTML export MUST hold each picture inline as a `data:` address, so the file names nothing outside itself. | this spec section 3; `PROSE-IO-064` | `ambre_prose::logic::picture_interop_tests::an_html_export_holds_each_picture_as_a_data_address` | none | verified |

### 1.9 Import a file carrying a picture

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-053` | `POST` | An import carrying a picture in an illegal place MUST put it at the nearest legal ancestor, never drop it. | this spec section 3 | `ambre_prose::logic::picture_interop_tests::a_picture_in_a_cell_lands_beside_the_table_and_is_named` -- the picture of a cell lands at the root | none | verified |
| `PROSE-FEAT-225` | `POST` | The conversion report MUST carry every picture the import lifted. | this spec section 3 | `ambre_prose::logic::picture_interop_tests::a_picture_in_a_cell_lands_beside_the_table_and_is_named` -- one picture moved in the report | none | verified |
| `PROSE-FEAT-226` | `FRAME` | Lifting a picture MUST NOT change its address, its display size, its alternative text or its caption. | this spec section 3 | `ambre_prose::logic::picture_interop_tests::a_picture_in_a_cell_lands_beside_the_table_and_is_named` -- the lifted block holds the address, the size and the text the file gave | none | verified |
| `PROSE-FEAT-319` | `POST` | An import MUST store each distinct picture once. | this spec section 3 | `ambre_prose::logic::picture_interop_tests::an_import_lands_once_every_picture_is_stored` -- three pictures, two of them one file, ask two uploads | none | verified |
| `PROSE-FEAT-325` | `FRAME` | An import MUST NOT write to the document until every upload of its pictures answered. | this spec section 3; `PROSE-IO-011` | `ambre_prose::logic::picture_interop_tests::an_import_lands_once_every_picture_is_stored` -- the snapshot is unchanged after one answer of two | none | verified |
| `PROSE-FEAT-321` | `POST` | A picture whose upload failed MUST be left out and named in the report, and the rest of the file MUST land. | this spec section 3 | `ambre_prose::logic::picture_interop_tests::a_picture_whose_upload_failed_is_left_out_and_named` | none | verified |

### 1.10 Decode a picture that arrived

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-314` | `FAIL` | A source file whose header declares an edge past the source bound MUST be refused before it is decoded. | this spec section 3; `ambre-spec-budgets.md section 6` | `ambre_prose::picture::tests::a_source_past_the_bound_is_refused_and_a_large_one_scaled_down`; `ambre_prose::logic::pictures::tests::a_file_the_media_flow_did_not_store_writes_nothing_and_is_named` -- a file that is no picture is refused, and nothing is uploaded | none | verified |
| `PROSE-FEAT-315` | `POST` | A decoded picture MUST be held at most the cache edge on each side, its ratio kept. | this spec section 3; `ambre-spec-budgets.md section 6` | `ambre_prose::picture::tests::a_source_past_the_bound_is_refused_and_a_large_one_scaled_down` | none | verified |
| `PROSE-FEAT-322` | `FRAME` | A decode MUST NOT write to the document. | this spec section 3; `PROSE-061` | `ambre_prose::logic::tests::a_picture_arriving_writes_nothing_to_the_document` | none | verified |

### 1.11 Hand the view a page that shows a picture

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-320` | `POST` | A picture on a page the view is handed, and not asked for yet, MUST be asked for once. | this spec section 3 | `ambre_prose::logic::pictures::tests::a_layout_asks_once_for_each_picture_it_carries` | none | verified |
| `PROSE-FEAT-323` | `FRAME` | Asking for a picture MUST NOT write to the document. | this spec section 3 | `ambre_prose::logic::pictures::tests::a_layout_asks_once_for_each_picture_it_carries` -- the snapshot is unchanged by the layout that asks | none | verified |

### 1.12 Store a picture

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-326` | `POST` | A stored picture MUST carry none of the metadata its file held, its orientation aside: no location, no device and no date. | this spec section 3; `ZK-001` | `ambre_prose::picture::tests::a_stored_picture_carries_no_exif_location`; `ambre_prose::logic::pictures::tests::a_picture_from_a_file_is_stored_without_its_metadata` -- the upload holds no camera, no XMP and no comment | none | verified |
| `PROSE-FEAT-327` | `FRAME` | Stripping the metadata MUST NOT change the pixels of the picture. | this spec section 3 | `ambre_prose::picture::tests::stripping_leaves_the_pixels_byte_for_byte` -- the JPEG scan is the same bytes, and the PNG decodes to the same samples | none | verified |
| `PROSE-FEAT-328` | `POST` | A stored picture MUST show as it was taken: the orientation its file stated MUST survive the stripping. | this spec section 3 | `ambre_prose::picture::tests::a_stored_picture_keeps_its_orientation`; `ambre_prose::logic::print::tests::a_printed_picture_stands_as_it_was_taken` -- the PDF draws the file turned | none | verified |
| `PROSE-FEAT-329` | `POST` | A picture an import carries MUST be stored without its metadata, as one chosen from a file is. | this spec section 3 | `ambre_prose::logic::picture_interop_tests::an_imported_picture_is_stored_without_its_metadata` | none | verified |

* **Budget.** the PDF export with pictures and one decode are rows of
  `ambre-spec-budgets.md section 1`. The cache, the export bytes and the edges are of section 6.
* **Boundary.** the picture blobs are Prose's only outbound flow of user content that is not the
  document. The Logic thread makes no network call and holds no key (`PROSE-025`). The Sync worker
  seals every blob, and the document holds an address, not pixels. Before a blob leaves, the Logic
  thread strips its location, device and dates (`PROSE-FEAT-326`). The seal hides them from the
  relay, and the stripping keeps them from every member and every exported file.
* **Verification.** 60 `verified`, all on the host: pure functions, `ProseState` with the Sync
  worker's answers driven by hand, and the recording surface.

---

## 2. WHAT EACH USER SEES

The address is shared and the pixels are not: each peer fetches for itself. The last column names
the test each row rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| inserts a picture from a file | watches | one address, one display size | Alice sees her picture at once; Bob sees a placeholder until his own fetch lands | `ambre_prose::logic::pictures::tests::a_picture_from_a_file_is_written_once_its_address_comes_back` |
| resizes a picture | resizes the same picture at the same moment | one size, one of the two | the same size on both screens | `ambre_prose::logic::pictures::tests::two_peers_resizing_one_picture_converge` |
| scrolls past many pictures and back | -- | every address, unchanged | the pictures again, refetched where the cache dropped them | `ambre_prose::logic::pictures::tests::an_eviction_writes_nothing_to_the_document` |
| opens a document whose picture the server no longer has | watches | the address | a broken picture and an offer to try again, on both screens | `ambre_prose::view::paint::tests::a_failed_picture_paints_a_mark_the_placeholder_does_not` |
| exports to PDF while offline | -- | the address | a placeholder in the file, and a line in the report | `ambre_prose::logic::picture_interop_tests::a_picture_that_did_not_come_back_is_a_placeholder_and_a_report_line` |
| imports a Word file with a picture inside a table cell | -- | the picture, beside the table | the picture out of the cell, and a line in the report | `ambre_prose::logic::picture_interop_tests::a_picture_in_a_cell_lands_beside_the_table_and_is_named` |

---

## 3. MODEL

**Containers.** None of its own. A picture is one block type of the movable tree. Its address,
its alternative text, its display size, its alignment and its caption are that node's attributes,
declared in `ambre-spec-prose-schema.md section 3`. There is no media register.

**The freeze.** The image keys joined the version 1 sections of `schema/shapes.txt` and
`schema/readings.txt` in place, and `SCHEMA_VERSION` stayed at 1. Version 1 already declared each
key with its type, so no layout moved (`SCHEMA-009`). No build had written a picture, so no stored
value changed its meaning (`SCHEMA-017`). No release tag exists, so no section was binding yet
(`SCHEMA-019`). The shape belt writes its picture last, so every earlier line keeps its block names.

The address is the key into the media flow: a hash of the plaintext keyed by the document key,
which the Sync worker computes (`SYNC-050`). Two copies of one picture in one document share it, so
no second table joins the two (`ambre-spec-prose-schema.md section 6`).

**The alignments:**

```rust
#[repr(u8)]
pub enum ImageAlign {
    /// A block of its own, at the left edge of the text. The default.
    Block  = 0,
    /// Left, text wrapped to the right. Not built: laid out as `Block`.
    Left   = 1,
    /// Right, text wrapped to the left. Not built: laid out as `Block`.
    Right  = 2,
    /// Centred, no wrapping.
    Center = 3,
}
```

`Left` and `Right` are accepted, kept through a round trip and laid out as `Block`. Text
wrapping needs line breaking with exclusion zones, which the shaper does not offer
(`features/ambre-spec-prose-pagination.md section 5`).

**How an address becomes pixels.** The Logic thread asks, the Sync worker fetches and opens, and
the answer comes back as the plaintext (`ambre-spec-media.md`). The Logic thread makes no network
call and names no key. It decodes the plaintext with the PNG and JPEG decoders Main already links,
and scales it down once to the cache edge. The view receives the decoded samples on the shared heap
and never decodes. What each address shows is a fact of the session, never of the document:

```rust
pub enum PictureShown {
    /// Asked for or not yet asked for: the neutral placeholder.
    Waiting,
    /// Decoded, ready to paint.
    Ready(Arc<DecodedPicture>),
    /// Failed for good. Not asked for again on its own (`PROSE-FEAT-057`).
    Failed(MediaFailure),
}

pub enum MediaFailure {
    /// The Sync worker answered no plaintext: the relay does not hold the
    /// blob, the blob did not open, or the network failed past its retries.
    Unavailable,
    /// The plaintext is no picture this build decodes, or its header
    /// declares an edge past the source bound (`PROSE-FEAT-314`).
    Undecodable,
}
```

Both failures are terminal: asking again on a timer cannot change them. A reader who asks again
puts the address back to unasked, and the download is issued once more. The state is the
address's, so two blocks holding one address show one state, and one retry asks for both.

**When a picture is asked for.** A layout the Logic thread hands the view carries the pictures of
its pages. Each one counts as painted, and one nothing holds yet is asked for once
(`PROSE-FEAT-320`). A failed one is not asked again (`PROSE-FEAT-057`).

**The bounds.** Two edges hold a picture, each a row of `ambre-spec-budgets.md section 6`:

| Edge | Bound | What it stops |
| :--- | :--- | :--- |
| the source edge | 8192 px, each side of the header | a file that expands past the decoder's memory before it is scaled (`PROSE-FEAT-314`) |
| the cache edge | 2048 px, the longer side of the decode | a decoded picture larger than any page shows (`PROSE-FEAT-315`) |
| the display edge | 4096 px, each side of the stored size | a layout cost no page can show (`PROSE-FEAT-059`) |

**The cache.** Decoded pictures are held in the Logic thread under a cap measured in decoded bytes,
not in entries. It is the bytes that spend the client-memory pillar. On each insert, the
least recently *painted* entries leave until the total fits (`PROSE-FEAT-054`). An entry whose
transfer is in flight is never chosen (`PROSE-FEAT-055`). A single picture larger than the whole
cap is kept, because it is the only way to show it at all.

An evicted address tells the view, which lets go of its samples too.

Eviction frees a decoding, never a fact: the address stays on the block (`PROSE-FEAT-056`), so a
picture scrolled back into view is asked for again. The cap is a row of
`ambre-spec-budgets.md section 6`.

**The commands** (`ambre_prose::message`, declared in `features/ambre-spec-prose-engine.md section 3`):

| Shape | Behaviour |
|---|---|
| `InsertImage { parent_id, after_id, content_hash, alt, size }` | a picture block holding an address the media flow already gave |
| `InsertImageFile { parent_id, after_id, data, alt }` | checks and decodes the file, stores it through the media flow, and writes the block once the address comes back. A command that names no place takes the block of the caret |
| `ResizeImage { block_id, size, lock_aspect }` | validates, then writes the display size |
| `SetImageAlign { block_id, align }` | one of the four alignments |
| `SetImageAlt { block_id, alt }` | the alternative text the screen reader announces |
| `SetImageCaption { block_id, caption }` | the caption drawn under the picture |
| `RetryImageLoad { block_id }` | puts the address back to unasked and asks again |

**What an insert from a file does, in order:**

1. refuse a file whose header this build cannot read, or one past the source edge: nothing leaves
   (`PROSE-FEAT-314`);
2. strip its metadata (`PROSE-FEAT-326`), the rule below;
3. decode it, so the author sees the picture with no download;
4. hand the stripped plaintext to the Sync worker and wait for the address (`SYNC-075`);
5. write the block, one image pixel to one page pixel, clamped (`PROSE-FEAT-316`). An upload that
   failed is told to the author, and nothing is written (`PROSE-FEAT-318`).

**What a stored picture keeps.** A photograph carries its location, its camera and its dates beside
its pixels. The Logic thread removes them before the bytes leave, from a file the reader chose and
from a picture an import carries alike (`PROSE-FEAT-326`, `PROSE-FEAT-329`). The pixels are not
touched (`PROSE-FEAT-327`):

| File | Removed | Kept |
| :--- | :--- | :--- |
| JPEG | the EXIF and XMP segments, IPTC, the comment, every other application segment | the scan, the JFIF header, the ICC profile, the colour transform, and a minimal EXIF segment holding the orientation alone |
| PNG | the text chunks, the time chunk and the EXIF chunk | every other chunk, each with its own checksum |
| any other format | -- | nothing: no client decodes it, so it is not stored |

The orientation stays in the file rather than in the block, so the schema keeps its attributes and
no pixel is encoded again. The decode turns the picture by it on the screen, and the PDF draws the
file through the same turn (`PROSE-FEAT-328`).

**What a resize does, in order:**

1. refuse a width or a height of zero: a picture of no area is not a state (`PROSE-FEAT-058`).
2. clamp each dimension to the maximum, and store what the clamp produced (`PROSE-FEAT-059`). Past
   it the layout cost is unbounded and no page can show the picture at that size.
3. when the aspect is locked, take the dimension that moved most against the stored size as the
   driver. Derive the other from the decoded picture's own ratio. When the blob has not been
   decoded yet, derive it from the stored size instead.

**Where a picture may sit.** The root, and only the root: not in a table cell, not in a list item
(`PROSE-FEAT-052`). The rule is the nesting table of `ambre-spec-prose-schema.md section 4`, and
the guard is the one `features/ambre-spec-prose-tables.md section 3` states, shared by both paths.
The import path uses the same predicate and lifts an illegal picture to the nearest legal ancestor
rather than refusing the file. The conversion report carries the lift (`PROSE-FEAT-053`).

**What an import does.** A file that carries pictures stores each distinct one once through the
media flow, and writes nothing until every upload answered (`PROSE-FEAT-319`). The file then lands
whole, as any import does (`PROSE-IO-011`). A picture whose upload failed is left out and named in
the report, and the rest lands (`PROSE-FEAT-321`). The pictures are not decoded on the way in: a
page that shows one asks for it like any other.

**What a PDF draws.** The page draws each picture as its stored file, never decoded on the Logic
thread. The recorder carries the file, and the PDF module embeds a JPEG whole and decodes a PNG
itself (`PROSE-IO-107`). A file shows at full resolution, and the print slice pays nothing for it.
The shared surface gained one method for it, `draw_encoded_image`, which a screen draws nothing
for (`ambre-spec-drawing-surface.md section 2`). Toile can reuse it:
`A deck exports at full resolution` Story (`ambre-roadmap.md` E6).

**What an export does.** An export asks the media flow for every picture the document addresses,
one download per address, before its walk (`PROSE-IO-110`). The requests are messages: the Logic
thread serves other commands and other documents while they run (`PROSE-IO-040`).

Once the last download answered, the walk starts on the document as it stands then. An edit made
during the wait is in the file. A picture added during the wait has no bytes, and goes out as a
placeholder (`PROSE-IO-105`).

The bytes the export holds count against the export cap (`PROSE-IO-106`). A picture whose bytes
did not come back, or past the cap, becomes a placeholder
and a line in the report. The export finishes (`PROSE-IO-008`). The mapping into each
format is `features/ambre-spec-prose-import-export.md section 3`.

**Where a picture is measured.** A picture is never split across pages
(`features/ambre-spec-prose-pagination.md`, `PROSE-014`), and it is measured as its box plus its
caption. A box wider than the text is drawn at the text's width, its ratio kept, and the document
keeps the stored size (`PROSE-FEAT-317`).

**Code:**

* `crates/ambre-prose/src/picture.rs` -- the bounds, the sizes, the decode and the cache;
* `crates/ambre-prose/src/logic/pictures.rs` -- the commands, the media answers, the import and
  export stages;
* `crates/ambre-prose/src/view/walk.rs` -- the picture, the placeholder and the broken mark;
* `crates/ambre-logic/src/data.rs` -- the lowering of each request and the routing of its answer.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the "Insert a picture" command, from the palette or the file menu | a file chooser, then a picture after the caret's block, once its blob has been stored |
| a press on a picture that failed for good | asks for the blob again, and moves no caret |
| `ResizeImage`, `SetImageAlign`, `SetImageAlt`, `SetImageCaption` | each reaches the engine as a command; the handles and the picture menu that send them are not built |

A picture not yet resolved paints a neutral placeholder carrying its alternative text. A picture
that failed for good paints a distinct broken mark, a cross over its box, and a press on it asks
again. No picture stays on the loading placeholder for ever (`PROSE-UI-046`).

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The document holds an address, never pixels | embedding the bytes in the block | it makes the document grow with the pictures and the history grow with the edits to them. A document is synchronized on every keystroke and a picture is megabytes: embedding it would put those megabytes on the delta path, in the history, and in every peer's copy whether they look at the picture or not. An address is 32 bytes and deduplicates two copies of one picture for free |
| The address is the content hash of the plaintext | a minted identifier | it stores the same picture twice and lets an address point at bytes that are not the ones it named. Two identical pictures then resolve to one blob without anything having to notice they are the same, and the address is verifiable against what came back |
| The Logic thread never fetches | fetching from the Logic thread | it puts network and crypto into the crate the whole application's rules say holds neither. The fetch needs the presigned URL and the document key, and the boundary that keeps those in the Sync worker is the one `PROSE-025` states; asking through a command keeps the Logic thread a pure function of what it is given |
| The cache is capped by decoded bytes | a cap on the number of entries | it is a memory figure only when every picture is the same size. A decoded picture is its pixels, and an entry count says nothing about them: ten thumbnails and ten photographs are the same count and two orders of magnitude apart, so the cap has to be the memory figure the pillar names |
| Eviction frees the decoding and never the document | clearing the address | it turns a local eviction into a remote deletion of the picture. The address is the fact and the pixels are a view of it, so dropping the pixels costs a refetch and nothing else; writing to the document to free memory would put a local memory decision on the synchronization path |
| An entry in flight is never evicted | evicting by recency alone | it drops the picture that is loading in favour of one already on screen. The in-flight entry's bytes have not arrived, so evicting it frees nothing and cancels work already paid for; it is also the entry the reader is most likely waiting on (`PROSE-FEAT-055`) |
| A terminal failure is not retried on its own | retrying on a timer | it never ends for a blob that was garbage-collected, and never tells the reader why. A picture the server does not have, or that does not decrypt, answers the same on every attempt: retrying spends requests to learn nothing and hides the failure behind a spinner, while asking the reader turns it into one visible state with one gesture out of it |
| A failure shows a distinct mark, not the loading placeholder | keeping the neutral placeholder | it is what a reader stares at with no way to know it will never resolve. The two mean opposite things -- one is waiting, the other is over -- and one glyph for both makes every failure look like a slow network for ever (`PROSE-UI-046`) |
| A zero dimension is refused and a large one is clamped | clamping the zero too | it invents a size the author never asked for. Zero has no correct interpretation, so refusing is the honest answer |
| A zero dimension is refused and a large one is clamped | refusing the large one | it fails a drag rather than stopping it. A number too large has one interpretation, the largest the layout can carry, so clamping keeps the gesture working |
| The clamped value is what is stored | storing the requested size and clamping when painting | it leaves the document holding a size nothing renders, and makes two builds with different maxima disagree about one document. Storing the clamped value makes the document say what the reader sees, so every later reader agrees with it |
| Wrapped text is laid out as a block | shipping an approximation of the wrap | it is wrong in a way the author has to fix by hand on every page. The shaper offers no exclusion zones, so a wrap would be an approximation that puts glyphs under the picture; a block is visibly not the wrap the author asked for, and it is never wrong |
| A picture sits at the root alone | allowing it everywhere | it makes the pagination cases and the merge cases grow together. A picture inside a cell or a list item multiplies the layout cases -- a cell that must grow, an ordinal beside a box -- for a document shape the MVP does not need, and refusing it once keeps every measurement rule about text |
| An illegal import is lifted, never dropped | dropping the node with a warning | it loses content the author still has in the source file and cannot see is gone. A file the reader opens is not a command they typed, so refusing part of it in silence loses their content; lifting keeps the picture and the report says where it went (`PROSE-FEAT-053`) |
| A missing picture never blocks the export | waiting for every blob before answering anything, the export held until the network returns | it makes an offline export hang instead of finishing. An export is a gesture with an end: a download that fails is a placeholder and a line in the report, and the export finishes (`PROSE-IO-008`) |
| An export fetches every picture before its walk | exporting from the decoded cache alone | it writes holes. The cache holds about three pictures at the cache edge, and nothing of a page never shown, so a document with more pictures than the cap, or with pages no reader scrolled to, would export with placeholders where the pictures are. The fetch is messages to the Sync worker, so it costs the Logic thread nothing while it waits (`PROSE-IO-040`) |
| The document holds the address and no thumbnail | Toile's inline thumbnail in the document | it puts pixels in the document, which `PROSE-FEAT-205` forbids, and grows every snapshot and every peer's copy with them. A Prose page paints the blob or a placeholder, so a thumbnail has no surface |
| The Logic thread decodes, at the cache edge | decoding in the images converter module | it moves the JPEG decoder out of Main, which is its own story: `The JPEG decoder moves to the images module` Story (`ambre-roadmap.md` E5) makes the decode asynchronous. Main already links the PNG and JPEG decoders for Toile and Murmure, so decoding here adds no byte to the download |
| An import stores its pictures, then lands whole | writing the blocks first and their addresses when the uploads answer | it writes the document in two moves, and a reader sees blocks with no picture behind them. The address comes from the Sync worker, so the file waits for it and then lands in one step of the history (`PROSE-IO-011`) |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing resizes or aligns a picture by hand.** The commands reach the engine, and the corner
  handles, the edge handles and the picture menu that would send them are not built. The
  `Picture handles and menu` Story (`ambre-roadmap.md` E5) builds them.
* **Nothing says whether a paste follows the refusal or the lift.** A picture pasted into a table
  cell meets `PROSE-FEAT-211` on an insert and `PROSE-FEAT-053` on an import, and the two answers
  differ. The `Rich paste: tables and pictures` Story (`ambre-roadmap.md` E5) rules on it.
* **The retry has no answer for the reader while it runs.** `RetryImageLoad` puts the picture back
  to the placeholder, and a second failure shows the mark again. Nothing says how many times a
  reader may ask.
* **Coming back online re-drives nothing.** A picture that failed while the network was down stays
  failed until a reader asks again.
* **An export waits on the Sync worker's answer.** A download the worker never answers holds the
  export, and a later export replaces it.
* **Text wrapping has no design**, and the two alignments that need it are accepted and stored. A
  document imported with wrapped pictures keeps an intent nothing honours. The
  `Text wrapped around a picture` Story (`ambre-roadmap.md` E5) designs it.
* **A Markdown export drops each picture.** The file could name no address, and the report says so.
  The `Pictures in a Markdown export` Story (`ambre-roadmap.md` E5) rules on a way out.
* **A caption leaves a Word or an OpenDocument file as the paragraph under the picture.** The
  model of the codec holds no caption, so a round trip turns it into text, and the report names it.
* **An equation is the second block whose content is not text, and it has no home.** The deferred
  design stores a formula as a mark on a range and as a block type, both holding a source string.
  It is rendered on the Logic thread into a drawing the page paints the way it paints a picture.
  Neither the mark nor the block type is in `ambre-spec-prose-schema.md section 3`, no event
  carries the rendering, and the renderer is a choice nobody has made. It is placed
  here because the path from a Logic-thread product to a painted block is this feature's. The
  Story is "The later writing set" (`ambre-roadmap.md` POST-V1).
* **A Word table holding a picture loses the cell it was in.** The pipeline lifts it beside the
  table and reports the lift (`PROSE-IO-026`, `PROSE-FEAT-053`), which is settled. What is not
  settled is the way back. An export writes the lifted picture where the document now holds it. A
  round trip through Prose then moves the picture out of the cell permanently.
