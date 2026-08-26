# AMBRE -- FEATURE: IMPORT AND EXPORT

**Type:** Vertical (feature)
**Implementation:** partial -- the converters and the native round trip ship; no gesture reaches any of them.

> A user brings a drawing that came from somewhere else onto the board, and hands the board back as
> a file someone without Ambre can open. Every conversion runs on the user's own machine.

The native `.ambre` file is the only path that keeps everything. Every other format drops something,
and what it drops is the subject of this document. The deck's own page format is
`features/ambre-spec-toile-pdf-export.md`, and the pixels an imported picture leaves behind are
`features/ambre-spec-toile-media.md`.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TOILE-IO-001` | An import MUST be atomic: the whole file becomes one undoable batch, and a parse failure MUST import nothing. | this spec section 3 | `ambre_toile::logic::state::import::tests::import_is_one_undo_group`, `::a_broken_file_fails_without_touching_the_board`, `::svg_import_takes_the_fast_path`, `ambre_toile::logic::interop::tests::malformed_svg_fails_to_parse`, `::empty_svg_reports_empty`, `::too_many_elements_aborts_atomically`, `::too_deep_nesting_aborts_atomically` | verified |
| `TOILE-IO-002` | An import MUST NOT trust foreign geometry: coordinates MUST be clamped, and counts, depth and image dimensions MUST be bounded. | this spec section 3 | `ambre_toile::logic::interop::limits::tests::coord_clamp_bounds_and_sanitizes`, `::rect_clamp_keeps_non_negative_extent`, `::element_count_guard`, `::depth_guard`, `::image_dim_guard`, `ambre_toile::logic::interop::tests::coordinates_are_clamped_to_the_world_bound` | verified |
| `TOILE-IO-003` | Every mapped SVG node MUST land as its element kind, keeping text bodies editable and per-slot styles. | this spec section 3 | `ambre_toile::logic::interop::tests::rect_maps_to_shape_rect`, `::ellipse_maps_to_shape_path`, `::path_maps_to_shape_path_with_geometry`, `::text_maps_to_text_element_preserving_body`, `::nested_groups_are_rebuilt_innermost_first`, `::rect_import_preserves_fill_and_stroke_style`, `::unstyled_kinds_keep_the_default_style`, `ambre_toile::logic::state::import::tests::import_applies_styles_per_slot`, `::import_rebuilds_nested_groups` | verified |
| `TOILE-IO-004` | An export MUST be deterministic: the same board yields a byte-stable structure. | this spec section 3 | `ambre_toile::logic::interop::svg_export::tests::export_is_deterministic`, `::rect_serializes_with_fixed_attrs`, `::text_body_is_escaped`, `::float_formatter_trims`, `ambre_toile::logic::interop::tests::round_trip_rect_is_stable`, `::round_trip_path_geometry_survives` | verified |
| `TOILE-IO-010` | An `.ambre` export and import round trip MUST preserve the document state, byte for byte on a bare board. | this spec section 3; `ambre-spec-docfile.md section 3` | `ambre_toile::logic::state::docfile::tests::round_trip_is_byte_identical_on_a_bare_document`, `::round_trip_preserves_the_document_state`, `::the_header_names_toile_and_the_document` | verified |
| `TOILE-IO-011` | A rejected file MUST leave both the input bytes and the current board untouched, and MUST fail typed rather than panic. | this spec section 3; `ambre-spec-docfile.md section 3` | `ambre_toile::logic::state::docfile::tests::a_rejected_file_leaves_both_the_input_and_the_state_alone`, `::a_corrupt_payload_under_a_valid_header_is_typed_not_a_panic` | verified |
| `TOILE-IO-012` | A `.ambre` file belonging to another app MUST be re-dispatched to that app, never coerced into Toile. | this spec section 3; `ambre-spec-docfile.md section 3` | `ambre_toile::logic::state::docfile::tests::a_trame_file_is_re_dispatched_not_coerced` | verified |
| `TOILE-IO-022` | An import report MUST reach the user: a construct dropped in silence MUST NOT be the outcome. | this spec section 4 | none, and **the rule fails today**: the event carries the report and the view writes it to the diagnostic log, where no user reads it -- section 6 | unverified |
| `TOILE-IO-023` | An imported file MUST land where the user put it, never at a fixed point of the world. | this spec section 4 | none, and **the rule fails today**: a raster lands at the world origin and an SVG at its own coordinates, and no command carries a drop point -- section 6 | unverified |
| `TOILE-IO-024` | An export MUST NOT change the board: it MUST emit no CRDT delta. | this spec section 3 | none. Both serializers take a read-only view and neither is driven from a live board, so no test can assert the empty outcome yet | unverified |
| `TOILE-IO-025` | An element the SVG serializer cannot express MUST be reported, never dropped in silence. | this spec section 3 | none, and **the rule fails today**: a sticky, a connector, a group, an embed and a stroke each serialize to nothing, and the export has no report at all -- section 6 | unverified |
| `TOILE-IO-026` | An exported element MUST carry its rotation. | this spec section 3 | none, and **the rule fails today**: the serializer reads the rotation field and writes no transform, so every element leaves axis-aligned -- section 6 | unverified |
| `TOILE-IO-027` | An SVG import MUST keep the source paint order, and a text node inside a group MUST stay in that group. | this spec section 3 | none, and **the rule fails today**: text is read in a second pass and appended after every vector node -- section 6 | unverified |
| `TOILE-IO-028` | A large import MUST NOT freeze the Logic thread: it MUST apply in bounded batches that yield. | this spec section 3; `ambre-spec-budgets.md` | none, and **the rule fails today**: the whole batch is applied in one synchronous pass, and no progress event exists -- section 6 | unverified |
| `TOILE-IO-020` | A user MUST be able to bring a file onto the board, by a drop or by a picker. | this spec section 4 | not built -- lands with the "The Toile interop surface" Story (`ambre-roadmap.md` E4); the commands exist and nothing emits them | unimplemented |
| `TOILE-IO-021` | A user MUST be able to take the board out as a file, and the bytes MUST reach the browser download path. | this spec section 4 | not built -- lands with the "The Toile interop surface" Story (`ambre-roadmap.md` E4); no export command exists on the bus | unimplemented |
| `TOILE-IO-014` | A Figma or Miro import MUST map every construct it recognises, and MUST list every construct it drops. | this spec section 3 | not built -- lands with the "Figma and Miro import" Story (`ambre-roadmap.md` E6); `ambre-toile` declares no JSON parser | unimplemented |
| `TOILE-IO-015` | A foreign import MUST obey the same safety limits and the same atomicity as the SVG path. | this spec section 3 | not built -- lands with the "Figma and Miro import" Story (`ambre-roadmap.md` E6) | unimplemented |
| `TOILE-IO-016` | A Figma auto-layout frame MUST import as a plain frame with children at resolved absolute positions. | this spec section 3 | not built -- lands with the "Figma and Miro import" Story (`ambre-roadmap.md` E6) | unimplemented |
| `TOILE-IO-017` | The PNG render-back MUST run offscreen on the UI thread's own surface and MUST NOT block the visible frame. | this spec section 3 | not built -- lands with the "PNG export" Story (`ambre-roadmap.md` E6); it needs a real GPU adapter, so its belt is wasm-tier | unimplemented |
| `TOILE-IO-018` | A showcase bundle MUST embed its board snapshot and MUST NOT contact the relay to render. | this spec section 3 | not built -- lands with the "The standalone showcase bundle" Story (`ambre-roadmap.md` E6) | unimplemented |
| `TOILE-IO-019` | A showcase bundle MUST carry plaintext content and MUST say so at export time. | this spec section 3; `ZK-001` | not built -- lands with the "The standalone showcase bundle" Story (`ambre-roadmap.md` E6) | unimplemented |

* **Budget.** the SVG import of 5 000 nodes under 500 ms, a row of
  `ambre-spec-budgets.md section 1`. It is a budget for a conversion no user can start
  (`TOILE-IO-020`).
* **Boundary.** a local file, in both directions, and nothing else. No converter opens a socket, and
  the pixels an import produces reach the relay as ciphertext alone
  (`features/ambre-spec-toile-media.md section 1`, `TOILE-IO-013`). The showcase bundle is the one
  deliberate exit from the zero-knowledge boundary, and `TOILE-IO-019` makes it say so.
* **Verification.** 7 `verified`, 7 `unverified`, 8 `unimplemented`. What ships is held where it is
  read: the native round trip, the SVG mapping, the safety limits and the determinism of the output
  all have a host test. Six of the seven gaps state a rule the code answers with something else, and
  they share one cause -- the converters were built as pure functions and never joined to a board or
  to a user. The eight others describe paths with no code.

---

## 2. WHAT EACH USER SEES

An import is an ordinary CRDT write, so it reaches the other peer like any edit. The last column
joins the example to the test that holds it.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| imports an SVG | draws elsewhere on the board | the imported elements and the drawing | both, on both boards | none. No property test drives an import |
| imports a file | imports the same file | two sets of elements, each minted by its own peer | two imports, never one merged drawing | none |
| imports a drawing, then undoes | watches | the board as before the import (`TOILE-IO-001`) | the drawing leaves in one step, on both boards | `logic::state::import::tests::import_is_one_undo_group` |
| imports a broken file | watches | nothing (`TOILE-IO-001`) | an error on Alice's screen and no change on Bob's | `logic::state::import::tests::a_broken_file_fails_without_touching_the_board` |
| exports the board | moves an element | the move alone: the export writes nothing (`TOILE-IO-024`) | Bob's move; Alice's file holds the board she exported | none |
| saves an `.ambre` file | opens it | the same board state (`TOILE-IO-010`) | the same elements, styles and deck | `logic::state::docfile::tests::round_trip_preserves_the_document_state` |

---

## 3. MODEL

**Containers.** This feature owns none. An import writes the element, connector, layer and media
containers the schema declares (`ambre-spec-toile-schema.md section 3`); the native export
serializes the whole document and adds nothing to it.

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
   failure or breached bound (`TOILE-IO-001`, `TOILE-IO-002`);
2. hold the batch and upload each media blob when the file carries one, applying nothing until every
   real content hash returns (`features/ambre-spec-toile-media.md section 1`);
3. create each slot, restyle it, build the groups innermost-first, then the connectors, then seed
   the media map;
4. commit once, so the whole file is one undo item, and rebuild the derived caches;
5. report how many elements the file yields, and what it drops.

Step 3 is ordered on purpose: a style is an independent register that cannot ride a create, and a
group needs the ids the creates mint.

**Steps, on an SVG export:** map each element back to its SVG counterpart, write the attributes in a
fixed order through a single float formatter, and anchor the view box at the origin. A view box
anchored at the content's corner would translate the whole document on re-import.

**What an SVG file becomes:**

| SVG node | What the import creates |
|---|---|
| `<rect>`, `<circle>`, `<ellipse>`, `<line>`, `<polygon>` | a shape, with its fill and its stroke |
| `<path>` | a vector node carrying the exact geometry |
| `<text>` | a text element, read from the source and left editable |
| `<image>` | a picture, and one blob for the media pipeline |
| `<g>` | a group, rebuilt after the creates land |
| a gradient or a filter | a raster fallback, reported as lossy |

**Text is read from the source, never from the parser.** The parser exposes a text node only by
flattening it to glyph outlines, which needs a system font database the browser does not have and
loses the editable string. So the font stack is disabled and the text nodes are read from the XML
directly.

**What an SVG export writes:** shapes, vector nodes, text and pictures. A sticky, a connector, a
group, an embed and a freehand stroke write nothing at all (`TOILE-IO-025`).

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

**There is none** (`TOILE-IO-020`, `TOILE-IO-021`). No drop handler, no file picker and no menu entry
emits an import command: the File menu carries one row, and it starts the presentation
(`features/ambre-spec-toile-menus.md section 4`). Nothing serializes a live board either, because no
code maps the visible document into the view model the SVG serializer reads.

| Trigger, once it exists | What must happen |
|---|---|
| a file dropped on the board | the file is converted and applied at the drop point (`TOILE-IO-023`) |
| the File menu import entry | a picker opens, and the chosen file takes the same path |
| the File menu export entry | the Logic thread serializes and answers with the bytes |
| the answer | the UI thread hands the bytes to the browser; no board state changes (`TOILE-IO-024`) |

**What the user is told today.** The report of a lossy import reaches the view and becomes a line in
the diagnostic log (`TOILE-IO-022`). A failure becomes a warning in the same log. Neither is a
surface a user can read.

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| Every conversion runs on the user's machine | the pillar is that the server holds ciphertext. A conversion is the one moment a document exists as plaintext outside the editor, so it is the one that must not leave the device | converting on the server, which is faster to build and hands the relay the whole document |
| An import is one atomic batch | a mid-file failure would leave a board half built from a file the user cannot see, and no undo would take it back in one step | streaming the elements as they parse, which is cheaper in memory and corrupts the board on any failure |
| A converter speaks in slots, not in ids | an offline converter cannot know an id the engine mints on apply, and rewriting the batch afterwards would need a second pass over every reference | minting ids in the converter, which needs the engine's counter outside the engine |
| A group is rebuilt after the creates | a group is born from the ids of its members, so it cannot exist before them. Emitting innermost-first makes every nested member already resolved | flattening a group on import, which loses the author's structure silently |
| Text is read from the XML source | the parser can only give text as outlines, which needs system fonts the browser lacks and destroys the string the user typed | letting the parser flatten text, which imports a picture of a word |
| Foreign geometry is clamped, not refused | a merely large drawing is not an attack, and refusing it would reject honest files. A clamp bounds the world and keeps the import | rejecting any coordinate outside the world, which fails on ordinary files |
| The native file is the only lossless path | every other format has a model of its own, so a round trip through it is a conversion twice. The native payload is the document itself | promising fidelity on SVG, which drops connector semantics and the deck |
| A showcase bundle is an explicit act | it produces plaintext outside the encryption boundary, so it may never happen as a side effect of another action | publishing automatically on export, which declassifies without asking |

---

## 6. NOT BUILT, AND NOT YET RULED

* **No gesture reaches any of it** (`TOILE-IO-020`, `TOILE-IO-021`). Four import commands are on the
  bus and two of them are handled; their only callers are tests. There is no export command at all,
  and the serializer's input type has no producer: `ExportElement` is a view model nothing builds
  from a board. The engine half of the feature is complete and unreachable.
* **The report is a log line** (`TOILE-IO-022`). The event names every construct the file lost, and
  the view writes two counts to the diagnostic log. A user who imports a file with gradients is told
  nothing.
* **An import ignores where the user is** (`TOILE-IO-023`). A raster lands at the world origin,
  wherever the camera happens to be; an SVG lands at its own coordinates. Neither command carries a
  drop point, so the surface that will exist has nowhere to put one.
* **The export drops five kinds and says nothing** (`TOILE-IO-025`). A sticky, a connector, a group,
  an embed and a freehand stroke serialize to nothing. The import path has a report and the export
  path has no equivalent, so the loss is invisible on the way out.
* **A rotated element exports upright** (`TOILE-IO-026`). The view model carries a rotation and the
  serializer never writes a transform. A board of rotated shapes exports as a board of unrotated
  ones, and the determinism rows still pass.
* **Text loses its place** (`TOILE-IO-027`). The vector walk and the text pass are two passes over
  one file, and the text pass appends. So every text node lands above every shape whatever the source
  said, and a text node inside a group is not a member of that group. The group's transform is not
  applied to it either.
* **The responsiveness design has no code** (`TOILE-IO-028`). The corpus described bounded batches
  that yield between chunks and an `ImportProgress` event, and contrasted them with Trame's single
  synchronous pass. Toile's apply pass is a single synchronous pass, and no such event exists on the
  bus. The claim was a plan written in the present tense.
* **The dependency table was a plan.** Two crates were declared for paths that do not exist and are
  absent from the manifest. The table above marks each one, because a dependency list is the most
  mechanically checkable paragraph in a spec.
* **Figma and Miro are refused by name.** Both commands answer with a typed message and touch
  nothing, which is the honest behaviour; the four rows that describe the mapping are the
  specification the converters will be built against.
* **Templates have no owner.** A template is a saved batch applied through the import path, so it
  needs no new concept and no new row. It needs a store, a gallery and curation, and none of the
  three is scheduled.
* **Nothing opens or saves an `.ambre` file.** The round trip is verified on both applications and
  its only production caller is the fixture generator. A user cannot save a board to disk or open one
  from it, in Toile or in Trame.
* **No property test drives an import.** An import creates as many elements as a paste and the
  two-peer convergence property drives neither it nor the serializers, which is why every row of
  section 2 that names a concurrent case says `none`.
* **The decoder has no fuzz target.** It is fed by user files and it is the first target
  `ambre-spec-testing.md` names, and none exists yet.
