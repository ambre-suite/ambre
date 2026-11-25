# AMBRE -- FEATURE: TOILE POWERPOINT EXPORT

**Type:** Vertical (feature)

> A user hands the deck to someone who presents with PowerPoint, Keynote or Google Slides. Each
> slide opens with its text still editable, and the report names what PowerPoint could not hold.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `TOILE-IO`, shared with
`features/ambre-spec-toile-import-export.md`.

### 1.1 The file

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-IO-106` | `IO` | The bytes an export hands back MUST open as a package whose every relationship reaches a part that names its content type. | this spec section 3 | `ambre_pptx::tests::every_relationship_reaches_a_typed_part` -- it also parses every XML part; `ambre_opc::tests::a_written_package_reads_back_with_its_types_and_relationships`; `::a_relationship_to_a_missing_part_is_refused` | none | verified |
| `TOILE-IO-107` | `IO` | A text MUST leave as text a reader edits, never as a picture of a word. | this spec section 3 | `ambre_pptx::tests::text_stays_editable_text_with_its_size_and_colour`; `ambre_toile::logic::interop::pptx_export::tests::a_text_stays_text_with_its_size_on_the_slide` | none | verified |
| `TOILE-IO-108` | `IO` | Any text the user typed MUST read back from the file as it left, line by line. | this spec section 3 | `ambre_pptx::tests::properties::any_text_reads_back_as_it_left` -- a proptest over the slide and its notes | none | verified |
| `TOILE-IO-109` | `IO` | A character the file format cannot hold MUST be left out of the text, never break the slide it lands on. | this spec section 3 | `ambre_pptx::xml::tests::a_character_xml_cannot_hold_is_left_out`; `::a_text_with_markup_characters_writes_a_part_that_parses` | none | verified |
| `TOILE-IO-110` | `IO` | A text MUST keep its face, its weight, its slant, its underline and its strikethrough. | this spec section 3 | `ambre_pptx::tests::a_run_carries_its_face_and_its_emphasis`; `ambre_toile::logic::interop::pptx_export::tests::a_text_keeps_its_face_and_its_emphasis` | none | verified |
| `TOILE-IO-111` | `IO` | A picture MUST leave as one embedded PNG part, however many slides show it. | this spec section 3 | `ambre_pptx::tests::a_picture_leaves_as_one_embedded_png` | none | verified |
| `TOILE-IO-112` | `IO` | An export MUST be deterministic: the same deck yields the same bytes. | this spec section 3 | `ambre_pptx::tests::the_same_deck_writes_the_same_bytes`; `ambre_opc::tests::the_same_parts_write_the_same_bytes` | none | verified |
| `TOILE-IO-113` | `INV` | The file MUST NOT name an author, a date or a document the user did not type. | this spec section 5; `ZK-001` | `ambre_pptx::tests::the_file_names_no_author_no_date_and_no_document` | none | verified |
| `TOILE-IO-114` | `IO` | A shape MUST keep its outline, its fill and its line. | this spec section 3 | `ambre_pptx::tests::each_geometry_writes_its_outline` | none | verified |
| `TOILE-IO-115` | `IO` | An item MUST keep its rotation. | this spec section 3 | `ambre_pptx::tests::a_rotated_item_carries_its_angle` | none | verified |
| `TOILE-IO-116` | `INV` | A file whose parts unpack past the size bound MUST be refused whole, never read in part. | this spec section 3 | `ambre_opc::tests::parts_that_inflate_past_the_package_bound_are_refused` -- the read that passes the bound fails, and the reader answers that error alone; `::a_compound_file_and_noise_are_refused`; `::a_package_without_content_types_is_refused` | none | verified |

### 1.2 Export the deck as a PowerPoint file

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-IO-117` | `POST` | An export MUST produce one slide per slide the deck still resolves. | this spec section 3 | `ambre_toile::logic::state::pptx_export::tests::one_slide_per_frame_in_play_order`; `ambre_pptx::tests::one_slide_per_slide_in_deck_order` -- the slide list of the file, read back | none | verified |
| `TOILE-IO-173` | `POST` | The slide size MUST be the size of the first frame, one point per world unit, unless an edge passes the bounds a presentation accepts. | this spec section 3; `TOILE-IO-172` | `ambre_toile::logic::interop::pptx_export::tests::the_slide_takes_the_size_of_the_first_frame`; `::a_frame_of_another_ratio_lands_centred` -- a frame past the largest edge takes 960 pt at its ratio | none | verified |
| `TOILE-IO-118` | `POST` | The slides MUST follow the play order of the deck. | this spec section 3 | `ambre_toile::logic::state::pptx_export::tests::one_slide_per_frame_in_play_order` -- one deck in both orders, so a sort by creation fails one | none | verified |
| `TOILE-IO-119` | `POST` | A slide whose frame is gone MUST produce no slide. | this spec section 3 | `ambre_toile::logic::state::pptx_export::tests::one_slide_per_frame_in_play_order` -- its last assertion, after the frame is deleted | none | verified |
| `TOILE-IO-120` | `POST` | A slide MUST carry what its frame shows, at the place the frame shows it. | this spec section 3 | `ambre_toile::logic::interop::pptx_export::tests::a_frame_fills_the_slide_and_its_content_lands_in_place`; `::a_frame_of_another_ratio_lands_centred`; `ambre_toile::logic::state::pptx_export::tests::each_slide_holds_what_its_frame_shows` | none | verified |
| `TOILE-IO-121` | `POST` | A board whose deck is empty MUST export as one slide framing its whole content. | this spec section 3 | `ambre_toile::logic::state::pptx_export::tests::an_empty_deck_exports_the_whole_board_as_one_slide` | none | verified |
| `TOILE-IO-122` | `POST` | A connector MUST hold on to the shapes at its ends. | this spec section 3 | `ambre_pptx::tests::a_connector_holds_on_to_the_shapes_at_its_ends`; `ambre_toile::logic::interop::pptx_export::tests::a_connector_holds_on_to_the_shapes_it_joins`; `ambre_toile::logic::state::pptx_export::tests::a_board_opens_back_from_its_file` | none | verified |
| `TOILE-IO-123` | `POST` | The speaker notes of a frame MUST reach the notes page of its slide. | this spec section 3 | `ambre_pptx::tests::speaker_notes_land_on_the_notes_page_of_their_slide`; `ambre_toile::logic::state::pptx_export::tests::the_notes_of_a_frame_reach_its_slide` | none | verified |
| `TOILE-IO-124` | `POST` | A sticky note or a freehand stroke MUST be named in the report as approximated. | this spec section 3; `INTEROP-003` | `ambre_toile::logic::interop::pptx_export::tests::a_sticky_and_a_stroke_are_approximated_and_named`; `ambre_toile::logic::state::pptx_export::tests::a_stroke_leaves_as_a_freeform_line` | none | verified |
| `TOILE-IO-125` | `POST` | A construct the file cannot hold MUST be named in the report, never dropped in silence. | this spec section 3; `INTEROP-003` | `ambre_toile::logic::interop::pptx_export::tests::what_the_file_cannot_hold_is_dropped_and_named` | none | verified |
| `TOILE-IO-126` | `POST` | A user MUST be able to export the deck as a PowerPoint file. | this spec section 4 | `ambre_toile::ui::menu_bar::tests::the_exports_home_in_the_ambre_menu_and_resolve_to_board_commands` -- the entry of the Ambre menu; `ambre_logic::dispatch::tests::a_pptx_export_reaches_the_ui_as_a_file_and_its_report` -- the command loads the Office module and comes back as a named file with its report; `::a_pptx_export_of_an_empty_board_answers_a_reason` | none | verified |
| `TOILE-IO-127` | `FRAME` | A PowerPoint export MUST NOT write to the board it reads, so no peer receives a delta from it. | this spec section 3 | `ambre_toile::logic::state::pptx_export::tests::a_pptx_export_leaves_the_oplog_unchanged` | none | verified |
| `TOILE-IO-128` | `FAIL` | Given a board with no element, the export MUST produce no file. | this spec section 3; `INTEROP-002` | `ambre_toile::logic::state::pptx_export::tests::an_empty_board_has_no_deck_to_export`; `ambre_pptx::tests::a_deck_of_no_slide_is_refused` | none | verified |
| `TOILE-IO-129` | `FAIL` | Given a picture whose bytes are no PNG file, the writer MUST refuse the deck rather than misname a part. | this spec section 3 | `ambre_pptx::tests::bytes_that_are_no_png_are_refused` | none | verified |

* **Budget.** the export of 50 slides under 500 ms, a row of `ambre-spec-budgets.md section 1`.
  No benchmark runs in CI, and the 18 ms of the writer alone is a single host reading.
* **Boundary.** none of its own: the writer is a pure function, and the file leaves on the shared
  export path (`TOILE-IO-068`). The file carries no author and no date (`TOILE-IO-113`).
* **Verification.** 25 `verified`. The File menu reaches the export, and the Office converter
  module runs the writer.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| reorders the deck | exports it | one order, converged | Bob's file follows Alice's order (`TOILE-IO-118`) | `logic::state::pptx_export::tests::one_slide_per_frame_in_play_order` |
| edits a note while Bob exports | exports | Alice's edit alone: the export writes nothing (`TOILE-IO-127`) | Bob's file holds the board he exported | `logic::state::pptx_export::tests::a_pptx_export_leaves_the_oplog_unchanged` |

---

## 3. MODEL

**Containers.** This feature owns none. It reads the deck order, the frames, their notes, the
elements and the connectors (`ambre-spec-toile-schema.md section 3`), and writes nothing.

**Two halves, two crates.** The board walk and the map live in `ambre-toile`, beside the SVG and the
PDF exports. The model and the writer live in `ambre-pptx`, a crate of pure functions. Its `codec`
feature holds the writer, so the Main instance links the model alone (`ARCH-078`).

```rust
// ambre_pptx::model, in points, from the top-left corner of the slide
pub struct Deck { pub size: SlideSize, pub slides: Vec<Slide> }
pub struct Slide { pub name: String, pub items: Vec<Item>, pub notes: String }
pub enum Item { Shape(Shape), Picture(Picture), Connector(Connector) }
pub enum Geometry { Rect, RoundRect { radius: PrintLength<Pt> }, Ellipse, Triangle, Line, Freeform(Vec<PathSegment>) }

// ambre_pptx, behind the `codec` feature
pub fn write(deck: &Deck) -> Result<Payload<FileBytes>, PptxError>;
```

**What each construct becomes:**

| On the board | In the file | Outcome in the report |
|---|---|---|
| a rectangle, an ellipse, a triangle, a line | the preset of the same name, with its fill, line and rotation | carried |
| a vector path | a custom geometry of the same path | carried |
| a text | a text box: its face, size, ink and emphasis, left editable | carried |
| a picture with its pixels | an embedded PNG, one part per distinct picture | carried |
| a straight connector | a connection shape bound to the shapes at its ends | carried |
| the notes of a frame | the notes page of its slide | carried |
| a sticky note | a rounded rectangle in the note colour, holding its text | approximated |
| a freehand stroke | a freeform line of one width: the pressure is lost | approximated |
| an elbow or a curved connector | the bent or curved connector PowerPoint routes itself | approximated |
| a connector caption | a text box on the middle of the line, no longer bound to it | approximated |
| a group | its members, ungrouped | approximated |
| an embed, a document embed, a frame off the deck | nothing | dropped |
| a picture whose pixels never arrived | nothing | dropped |

**The package.** One master, one blank layout, one theme, and a notes master when a slide has
notes. Each slide carries its items itself, so a reader sees them where the board put them. The
core properties hold a fixed title alone. A face the board bundles takes the common face of its
kind: the theme sans, Times New Roman, Courier New.

**Steps, on an export:**

1. the board walk lists the frames of the deck in play order, and skips a frame that is gone. An
   empty deck gives one region framing the whole board, and an empty board gives nothing;
2. per region, it lists every element that meets the frame in paint order, and every connector
   whose line meets it. A frame of the deck is a slide of its own, never an item of another;
3. the map sets the slide size from the first frame, one point per world unit (`TOILE-IO-173`).
   A frame past the edges a presentation accepts takes 960 pt wide, at its ratio. Each frame then
   fits that slide with one scale, centred;
4. the map turns each element into an item and records each approximation and drop;
5. the writer adds the static parts, then each slide with its pictures and its notes page. It
   finishes the zip with a fixed time and a fixed part order (`TOILE-IO-112`).

**The package layer** is `ambre-opc`, which the Word and the spreadsheet codecs share. It adds a
part, relates two parts and finishes the zip, and refuses a relationship to a part the package
does not hold. It reads a package back under the bounds of `ambre-spec-budgets.md section 6`
(`TOILE-IO-116`).

**Code:**

* `crates/ambre-pptx/src/model.rs` -- the deck;
* `crates/ambre-pptx/src/pml.rs` -- the writer of the presentation parts;
* `crates/ambre-opc/src/lib.rs` -- the package layer;
* `crates/ambre-pptx/src/xml.rs` -- the escaping writer every part goes through;
* `crates/ambre-toile/src/logic/interop/pptx_export.rs` -- the map and the report;
* `crates/ambre-toile/src/logic/state/pptx_export.rs` -- the board walk, and the call into the
  converter module;
* `crates/ambre-convert/src/engine/mod.rs` -- the `WritePptx` conversion of the Office module.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the File menu's PowerPoint export entry | the board walk and the map run on the Logic thread; the deck crosses to the Office converter module, which writes the package on the converter thread (`TOILE-IO-126`, `ARCH-129`) |
| the answer | the file, `deck.pptx`, and the map's report take the shared export path, as the PDF export does. A board with no element answers a failure and no file (`TOILE-IO-128`) |
| the first export of a session | waits for the Office module, which the Sync worker fetches once (`ARCH-081`) |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| One blank layout, each slide carries its items | a title and a body placeholder per slide | the layout would decide where the frame title sits, and move content away from where the board put it |
| The writer is a crate of pure functions | the writer inside `ambre-toile` | it links a zip and an XML writer into the Main instance, and the Word and spreadsheet codecs could not share the package layer |
| A sticky note leaves as a rounded rectangle holding its text | a sticky note as a picture | the text would stop being text, and a reader could not edit or search it |
| A freehand stroke leaves as a freeform line | a stroke as a picture | it loses the edit, and a picture of a line blurs at projector size |
| Every text goes through the escaping writer of `quick-xml` | strings formatted by hand | one missed escape on one text breaks the whole slide, and a text the user typed reaches every part |
| The zip keeps deflate alone | the default features of `zip` | they add AES, bzip2, lzma, zstd and a clock, none of which a PowerPoint file uses, to the converter module |
| One slide size for the deck, from the first frame | one slide size per frame | a presentation holds one size; a frame of another ratio is centred instead |
| The core properties hold a fixed title | the name of the user and the export time | the file leaves the encryption boundary, and a fact the user did not type must not leave with it |
| The deck crosses to the Office module as postcard bytes | the deck as a typed field of the request | every converter module decodes the request, so each would link the serde code of the deck |
| The map runs on the Logic thread, beside the board walk | the map in the Office module | the map weighs about 1 KB of brotli in Main, and the module would need the Toile element types in a crate of their own |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A picture leaves at thumbnail resolution.** The board holds a thumbnail in plaintext and the
  full blob is ciphertext on the relay, as `TOILE-IO-100` states for the PDF.
* **A frame of another ratio shows its neighbours.** The bands beside a centred frame are slide
  area, so an element that meets the frame and spills into a band stays visible there.
* **A group leaves ungrouped.** PresentationML has group shapes, and the map does not write them.
* **A text carries one style.** A body with runs of several styles waits on one story:
  the `Rich text on the board` Story (`ambre-roadmap.md` E6).
* **The LibreOffice check is manual.** A headless LibreOffice converts a sample to PDF with every
  construct in place, and no CI job runs it.
* **The board walk is not measured.** The budget row reads the writer alone.
