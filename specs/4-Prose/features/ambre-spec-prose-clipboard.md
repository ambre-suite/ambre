# AMBRE -- FEATURE: CLIPBOARD

**Type:** Vertical (feature)
**Implementation:** partial -- text, HTML and a plain paste work; tables and pictures do not.

> Copy, cut and paste. Copying inside Ambre keeps the formatting and the structure of what was
> copied. Copying out of it leaves plain text and HTML. A paste from a web page, Google Docs or
> Word keeps its headings, lists, links and marks.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-013` | `INV` | A paste MUST be routed through the Logic thread as a command: the view MUST NOT insert clipboard content into a model of its own. | this spec section 3 | `ambre_prose::input_tests::a_paste_leaves_as_a_command` | none | verified |
| `PROSE-UI-105` | `INV` | The view MUST NOT serialize document content: the payload MUST be built where the document lives. | this spec section 5 | structural -- the payload type is private to `crates/ambre-prose/src/logic/clipboard/mod.rs`, and the view sends the selection's two ends; `ambre_prose::logic::clipboard::tests::a_copy_covers_the_whole_selection_and_writes_nothing` | none | verified |
| `PROSE-UI-041` | `INV` | The view MUST NOT touch the system clipboard: reading and writing it MUST be the host's, inside the window the user's gesture opens. | this spec section 3; `ambre-spec-clipboard.md` | structural -- `web-sys` is a wasm-only dependency of `crates/ambre-prose` given to the reply field, and no Prose source names the clipboard API, which the host in `crates/ambre-app/src/host/mod.rs` alone reads and writes | none | verified |
| `PROSE-UI-106` | `INV` | No format other than the plain-text mirror, the HTML half and this application's own bytes MUST be read from or written to the system clipboard. | this spec section 5 | structural -- `ambre_bus::ClipboardBlob` holds the plain text, the HTML and this application's bytes and nothing else, and the host adds only the token that names its copy | none | verified |
| `PROSE-UI-107` | `IO` | Content copied in Ambre and pasted back into Ambre MUST return with the marks and the block structure it was copied with. | this spec section 3 | `ambre_prose::logic::clipboard::tests::a_rich_paste_keeps_types_and_marks`; `::a_typed_kind_keeps_its_payload_bytes` -- the ordinal style and the code language encode as the bare code and text they replaced | none | verified |
| `PROSE-UI-219` | `IO` | A paste of HTML MUST keep its headings, paragraphs, lists, to-do items, code blocks, links and character formatting, Google Docs and Word fragments included. | this spec section 3 | `ambre_prose::logic::clipboard::tests::each_html_fixture_pastes_as_its_golden_says` -- a Word, a Google Docs, a web page and a PDF viewer fixture, each against a golden text; `::the_html_half_alone_pastes_what_the_rich_half_does` -- Ambre's own HTML; `ambre_prose::logic::clipboard::html_read::tests::a_google_docs_fragment_keeps_its_blocks_and_marks`; `::a_word_fragment_keeps_its_list_and_drops_its_bullet_glyph`; `::white_space_collapses_outside_a_pre_and_stays_inside_one`; `::colours_and_sizes_land_canonical`; `::a_word_clipboard_document_reads_as_its_fragment`; `::a_google_docs_clipboard_fragment_keeps_list_highlight_and_link`; `::a_pdf_text_layer_reads_one_paragraph_per_line` -- the reader, one source at a time; `ambre_prose::input_tests::an_html_half_keeps_a_short_paste_off_the_typing_path` -- the view hands the half on | none | verified |
| `PROSE-UI-220` | `INV` | A paste of HTML MUST insert only text and the formatting this document knows: no script, style, hidden content, embedded object or link outside http, https and mailto MUST reach the document. | this spec section 5 | `ambre_prose::logic::clipboard::html_read::tests::hostile_markup_leaves_only_its_text` | none | verified |
| `PROSE-UI-234` | `INV` | A paste of this application's own bytes MUST write only marks whose key is exactly a formatting root. Each value MUST pass the validation of a typed mark under its key. A link MUST also pass the scheme and length check of an HTML paste. A family name MUST pass the length cap of an HTML paste. A flag MUST land as present, or not at all. Any other mark MUST be dropped. | this spec section 3 | `ambre_prose::logic::clipboard::tests::a_crafted_payload_writes_only_the_marks_a_typed_mark_may` -- a payload built by hand carries an anchor, an unknown key, a number, and a colour, a size and a flag the grammar refuses, and only the two valid marks land; `::a_crafted_link_lands_only_when_a_reader_may_follow_it` -- a `javascript:`, a `data:` and an overlong link are dropped; `::a_crafted_family_name_lands_only_within_the_cap` -- a name of 100 000 characters is dropped; `::a_crafted_key_lands_only_when_it_names_a_root_exactly` -- `b:zz` and `fg:x` are dropped; `::a_crafted_flag_lands_as_present_or_not_at_all` -- `true` lands as the empty string and `false` does not land | none | verified |
| `PROSE-UI-224` | `INV` | A paste of HTML MUST NOT turn a page's theme into marks: the style of an element holding blocks, and a body ink black or near it, MUST set no mark. | this spec section 3 | `ambre_prose::logic::clipboard::html_read::tests::a_web_page_keeps_its_formatting_and_drops_its_theme`; `ambre_prose::logic::clipboard::tests::each_html_fixture_pastes_as_its_golden_says` -- the web page fixture's themed container | none | verified |
| `PROSE-UI-225` | `IO` | A paste of an HTML table MUST land as a table, its rows, columns and merged cells kept. | this spec section 3 | not built | unbuilt -- lands with the `Rich paste: tables and pictures` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-226` | `IO` | A paste of HTML holding a picture the clipboard also carries MUST land the picture at its place in the text. | this spec section 3 | not built | unbuilt -- lands with the `Rich paste: tables and pictures` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Copy

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-040` | `POST` | The copied payload MUST be built from the whole selection, never trimmed to what is on screen. | this spec section 3 | `ambre_prose::logic::clipboard::tests::a_copy_covers_the_whole_selection_and_writes_nothing` | none | verified |
| `PROSE-UI-108` | `POST` | The payload MUST carry a plain-text mirror of the selection, readable by an application that knows nothing of Ambre. | this spec section 3 | `ambre_prose::logic::clipboard::tests::a_copy_covers_the_whole_selection_and_writes_nothing` | none | verified |
| `PROSE-UI-222` | `POST` | A copy MUST also write an HTML half that keeps the block types, the lists, the links and the marks, with every character of the text escaped. | this spec section 3 | `ambre_prose::logic::clipboard::tests::the_html_half_alone_pastes_what_the_rich_half_does` | none | verified |
| `PROSE-UI-235` | `FRAME` | The HTML half of a copy MUST NOT write a link target `PROSE-UI-186` holds back from the host, nor a colour or a size outside the canonical spelling of its typed mark: the run keeps its text, unmarked by that value. | this spec section 3; `PROSE-UI-186` | `ambre_prose::logic::clipboard::tests::a_peer_mark_value_reaches_the_html_half_only_when_safe` -- a peer writes a `javascript:` link, colours carrying a declaration and markup, and a size carrying a declaration straight into the document, and the half holds the text with no `href`, `url(` or `style`; `::a_peer_mark_value_is_written_canonical` -- a valid peer colour and size are written canonical | none | verified |
| `PROSE-UI-109` | `FRAME` | A copy MUST NOT write to the document, and MUST NOT move or shrink the selection it read. | derived | `ambre_prose::logic::clipboard::tests::a_copy_covers_the_whole_selection_and_writes_nothing` | none | verified |

### 1.3 Cut

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-044` | `POST` | A cut MUST delete the source range only after the copy has been asked for. | this spec section 3 | `ambre_prose::input_tests::a_cut_copies_before_it_deletes_the_selection` | none | verified |
| `PROSE-UI-110` | `FAIL` | A cut that produced no payload MUST delete nothing, and MUST leave the document as it was. | this spec section 3 | `ambre_prose::input_tests::a_refused_copy_deletes_nothing` | none | verified |
| `PROSE-UI-111` | `FRAME` | A cut MUST NOT delete outside the source range, and MUST NOT change the formatting of the text that remains around it. | derived | `ambre_prose::input_tests::a_cut_copies_before_it_deletes_the_selection` | none | verified |

### 1.4 Paste

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-042` | `POST` | A paste MUST take the richest half the clipboard holds: this application's own payload, then the HTML half, then the plain-text mirror. | this spec section 3 | `ambre_prose::logic::clipboard::tests::the_richest_half_the_clipboard_holds_wins`; `::foreign_bytes_paste_as_plain_text` -- the plain-text leg | none | verified |
| `PROSE-UI-043` | `POST` | The kept payload MUST be used only while the system clipboard carries this tab's token; otherwise the pasted content MUST come from the system clipboard (`UI-THREAD-007`). | this spec section 3 | `ambre_prose::input_tests::the_kept_payload_serves_only_its_own_copy` | none | verified |
| `PROSE-UI-221` | `FAIL` | HTML nested deeper than the reader's depth cap MUST paste as its plain-text mirror, and MUST NOT be built into a tree. | this spec section 3 | `ambre_prose::logic::clipboard::html_read::tests::a_tree_nested_past_the_cap_reads_as_nothing` | none | verified |
| `PROSE-UI-223` | `POST` | A paste of several blocks, or of one typed block, into an empty paragraph MUST give that paragraph the type of the first pasted block. | this spec section 3 | `ambre_prose::logic::clipboard::tests::the_html_half_alone_pastes_what_the_rich_half_does` | none | verified |
| `PROSE-UI-227` | `POST` | The paste-as-plain-text chord and its context-menu entry MUST insert the plain-text mirror alone, whatever richer half the clipboard holds. | this spec section 4 | `ambre_prose::input_tests::a_plain_paste_carries_the_text_alone`; `ambre_prose::commands::tests::the_context_menu_entries_are_declared_and_left_unbound` | none | verified |
| `PROSE-UI-112` | `FRAME` | A paste MUST NOT change a block outside the range it replaces, and MUST NOT reformat the text that surrounds it. | derived | `ambre_prose::logic::clipboard::tests::a_paste_mid_paragraph_leaves_the_neighbours_alone` | none | verified |

* **Budget.** the HTML read holds the "Prose HTML clipboard import" row of `ambre-spec-budgets.md`.
  A copy walks the selection once and a paste of the other halves is an ordinary insertion; the
  registry budgets neither. What a very large paste costs is in section 6.
* **Boundary.** the system clipboard leaves the application, and it is the one path here that does.
  It carries the user's own selection, already plaintext on their machine; nothing is encrypted,
  nothing is sent, and the host performs the write (`PROSE-UI-041`). The browser's own handling is
  outside Ambre's control, and gates the mirror (`ambre-spec-clipboard.md`).
* **Verification.** 23 `verified`, 2 `unimplemented`. The Logic rows are host tests over a real
  document, a round trip included; the view rows drive the input queue with a recording sender.
  `PROSE-UI-041`, `PROSE-UI-105` and `PROSE-UI-106` are held by where the types live.

---

## 2. WHAT EACH USER SEES

Two peers, one selection. The last column names the row each line rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| copies three formatted paragraphs, then pastes them lower down | -- | the three paragraphs again, with their formatting | the same text twice, styles kept | none -- `PROSE-UI-107` |
| copies a selection larger than the window | -- | nothing yet | the whole selection on the clipboard, not the visible part | none -- `PROSE-UI-040` |
| cuts a sentence | edits the same sentence at that moment | the deletion and Bob's edit, merged | the sentence gone on both screens, Bob's edit kept where it survived | none -- `PROSE-UI-044` |
| pastes into a text editor outside Ambre | -- | nothing | the plain text of what she copied | none -- `PROSE-UI-108` |
| copies in Ambre, copies again in another application, pastes back | -- | the second copy's text | the other application's text, not the earlier rich payload | none -- `PROSE-UI-043` |
| pastes a titled list from a web page | -- | a heading and list items, bold and links kept | the page's structure, in the document's own styles | none -- `PROSE-UI-219` |
| copies in Ambre, pastes into Google Docs | -- | nothing | the headings, lists and formatting she copied | none -- `PROSE-UI-222` |

---

## 3. MODEL

**Containers.** None. A paste writes through the ordinary editing commands, and a copy reads.

**The shared model.** Prose uses the clipboard model of `ambre-spec-clipboard.md`, which is
host-driven and authoritative in the Logic thread. The transport is the application-agnostic
payload of `ambre-bus`:

```rust
/// What crosses to the host, and back. `text` is the plain-text mirror every other
/// application can read, `html` the formatted half other editors read, and `ambre` this
/// application's own bytes, under its private type. `form` says the user asked for the
/// plain text alone.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClipboardBlob {
    pub text:  String,
    pub html:  Option<String>,
    pub ambre: Option<ClipboardBytes>,
    pub form:  PasteForm,
}
```

**The three steps of a copy.**

1. The view asks the Logic thread for the payload (`PROSE-UI-040`); it serializes nothing
   (`PROSE-UI-105`).
2. The Logic thread walks the selection in the document and answers with the three halves:
   the text, the HTML and the bytes.
3. The host writes all three to the system clipboard on the next frame (`PROSE-UI-041`). The view
   keeps the payload in memory for a paste back into the same tab.

```rust
/// Ask the active view for the payload. `ClipboardWrite::Cut` deletes the source range after
/// the copy (`PROSE-UI-044`).
fn clipboard_write(&mut self, write: ClipboardWrite) -> CopyOutcome;
/// The host calls this after each update: the payload to mirror, when one arrived.
pub fn poll_clipboard(&mut self) -> Option<ClipboardBlob>;
/// A paste, with what the host read. `source` says whether the host's token names this
/// tab's copy (`PasteSource::Own`) or not.
fn paste_clipboard(&mut self, blob: ClipboardBlob, source: PasteSource);
```

**The three outcomes of a paste** (`PROSE-UI-042`). The view sends every half it holds in one
`Paste` command, and the Logic thread takes the first that reads:

| What the host read | What is inserted |
|---|---|
| a payload this application wrote, and its token is still there | the rich content (`PROSE-UI-043`) |
| HTML that yields at least one block | the blocks and marks the reader keeps (`PROSE-UI-219`) |
| anything else | the plain text: one paragraph per line, control characters dropped |

**Reading HTML.** The parse runs on the Logic thread, never on the UI thread. A tokenizer pass
first measures the nesting depth. A fragment deeper than 128 elements is not built
(`PROSE-UI-221`), since the tree builder costs the square of the depth. The WHATWG tree builder
of `html5ever` then fills an arena, and one iterative walk keeps an allow-list.

Blocks are headings, paragraphs, list items (bullet, ordered with its numeral style, to-do), and
code with its language. Marks are the usual character formats, inline code, and http, https and
mailto links. The colour, highlight, size and family of an inline style come too. Each value
passes the same validation a typed mark does. Scripts, styles, hidden elements, SVG and MathML,
embedded objects and Word's list glyphs leave nothing (`PROSE-UI-220`).

The style of a container and a near-black ink
are the page's theme, and set no mark (`PROSE-UI-224`). A table reads as one paragraph per cell
and a picture reads as nothing, until `PROSE-UI-225` and `PROSE-UI-226` land. A PDF viewer's
text layer reads as one paragraph per visual line, as its plain text does.

**Writing HTML.** The copy writes one element per block, groups consecutive list items into one
list, and escapes every text and attribute value (`PROSE-UI-222`). An empty paragraph at the
paste point takes the first block's type, as Word and Google Docs do (`PROSE-UI-223`).

A peer writes a mark value straight into the document, unchecked, so the copy checks it again. A
link keeps its target only when the host may open it. A colour or a size is written in its
canonical spelling. A value that fails leaves its text unmarked (`PROSE-UI-235`).

All three leave as commands, so the document is written by the Logic thread and the view keeps no
model of what was pasted (`PROSE-UI-013`). Opening a Word or Markdown *file* is a document
import, not a paste (`features/ambre-spec-prose-import-export.md`).

**Why the mirror is checked.** The kept payload is the only full-fidelity source, and it stays
valid only while nothing else has taken the clipboard. Comparing it against the plain text the
host read is what tells the two apart. A copy in another application replaces the text, and the
kept payload is then stale (`PROSE-UI-043`).

**Cut.** A cut is a copy and then a deletion of the source range, in that order and never the
other one (`PROSE-UI-044`). A copy that produced nothing deletes nothing (`PROSE-UI-110`). The
deletion is the ordinary cross-paragraph one
(`features/ambre-spec-prose-text-editing.md section 3`).

A payload opens with four bytes of its own, so bytes another Ambre application wrote under the
same clipboard type paste as their plain text. Only the formatting marks travel: a comment or a
suggestion anchor stays in the document it was copied from.

Any page can write those bytes, so a paste reads them as untrusted. It keeps a mark only when its
key is exactly a formatting root, as a copy writes it. The value passes the validation of a typed
mark, and the check an HTML paste makes of a link and a family name. A flag set by a boolean lands
as present, and one cleared by a boolean does not land. Any other mark is dropped
(`PROSE-UI-234`).

A paste of several blocks into a band or a note is refused, as a split there is. A paste of one
line with neither a payload nor HTML is typing, and joins the draft.

**Code:** `crates/ambre-prose/src/logic/clipboard/` (the payload, the HTML halves, the copy and
the paste),
`crates/ambre-prose/src/input.rs` (the copy, cut and paste inputs), and
`crates/ambre-app/src/host/prose.rs` (the host entries).

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the copy chord, or the menu entry | asks for the payload and mirrors it |
| the cut chord | the same, then deletes the selection |
| the paste chord | inserts the rich payload, the HTML, or the plain text |
| a paste with no selection | inserts at the caret |
| a paste over a selection | deletes it first (`PROSE-UI-030`) |
| the paste-as-plain-text chord, or its menu entry | inserts the plain text alone, the kept copy ignored too (`PROSE-UI-227`) |
| dropping text into the page | inserts at the drop point (`PROSE-UI-204`) |

The legacy chords and the fold of `Ctrl` and `Cmd` are decided for every application in
`1-Foundations/ambre-spec-commands.md section 2`. Prose takes the shared clipboard identities and
their chord lists, and decodes none of them itself.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The Logic thread builds the payload | serializing the selection in the view | it requires the view to hold the document, which is the one thing `PROSE-UI-001` forbids. The document lives on the Logic thread, and that is the only place the whole selection can be read |
| The payload covers the selection, never the window | building the payload from the display list | the display list is culled by construction, so a user selecting forty pages gets what was built for the frame. The loss is invisible until the paste, and then it is silent |
| The host owns the system clipboard | calling the clipboard from the view | browsers grant clipboard access only inside the window a user gesture opens, and that window is the host's event handler. A worker or a view that asks later is refused, and the refusal cannot be retried or reported usefully |
| The clipboard carries plain text, HTML and this application's own bytes | plain text and the private bytes only | a paste from Google Docs, Word or a web page loses every heading, list and link, and a copy into them does too. HTML is the one rich format every browser exposes; RTF and Markdown are not readable from a web page |
| HTML is read on the Logic thread through an allow-list | sanitizing markup on the UI thread and inserting what survives | the UI thread would parse untrusted content inside the frame budget. An allow-list that emits the document's own blocks and marks keeps nothing it does not know, so no markup survives to be escaped later |
| An arena sink for `html5ever` | `markup5ever_rcdom` | its authors publish it as unofficial and meant for tests, and its reference-counted nodes free a deep tree recursively |
| The kept payload is gated on the mirror | pasting the kept payload whenever there is one | it pastes what the user copied ten minutes ago. The payload in memory is not what the clipboard holds, since another application may have replaced it, and the plain-text mirror is the one field both sides wrote and can compare |
| A cut copies first | deleting and then copying | it loses the content if the copy is refused, with nothing on the clipboard to show for it. The other order makes the failure harmless |
| Importing a file is not a paste | routing a file drop through the paste path | it puts an untrusted document parse on a gesture that has no report. A dropped or opened document needs a parser, a report of what was lost, and a progress path for a large file; a paste has none of those and must not grow them |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A paste from another Ambre application is not ruled on.** The rich half carries this
  application's own shape, so a paste from the whiteboard or the spreadsheet falls back to plain
  text. Making that structure survive is the
  `Cross-app rich paste` Story (`ambre-roadmap.md` POST-V1).
* **Nothing rules on the size of a payload.** A copy of a whole document builds it in memory
  twice, once as text and once as bytes. No rule caps it or says what happens when the system
  clipboard refuses it.
* **Nothing rules on a paste that carries images.** The rich payload is described as runs and
  block structure, and an image is a reference to a blob the receiving document may not have.
* **A dropped file has no design.** Dropped text is `PROSE-UI-204`; a dropped document is an
  import (`features/ambre-spec-prose-import-export.md`).
