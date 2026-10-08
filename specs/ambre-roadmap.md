# AMBRE -- ROADMAP

**Type:** Scheduling (one file for the whole project)
**Scope:** every epic and every story, for every application.

> **Role of this file.** Nothing schedules anywhere else. A spec says what the system does; this file
> says what work is left to reach it. A spec that carries its own backlog is a second schedule, and
> two schedules disagree.

---

## 1. THE MODEL

| Term | What it is | Where it lives |
| :--- | :--- | :--- |
| **Feature** | the smallest thing a user can name, ship alone and specify end to end | one spec document, always current |
| **Story** | one unit of production that moves one or more features forward | one row of this file |
| **Epic** | a group of stories, one increment of value | one heading of this file |
| **Milestone** | a release scope spanning epics | the `Milestone` column |

**Eight rules, and each one exists because it was broken.**

* **A story is one row, and it says what, never how.** A checkbox, a name, its milestone, the
  features it touches. The code says how it works and git says how it got there. A row that explains
  an implementation is a row nobody reads, and it rots on the next commit while looking
  authoritative.
* **A story that needs a paragraph is two stories**, or it is design content sitting in the wrong
  file.
* **Finishing a story is ticking its box. Nothing else.** No note added, no wording changed, no
  account of what was done. The row said what the work was before it started, and it still says it.
* **Closing an epic is cutting it into `ambre-roadmap-journal.md`, verbatim. Nothing else.** Every
  row as written, the heading as written. No summary, no retrospective, no rewrite. An epic is `[x]`
  only when every story under it is `[x]`: the status is derived, never set. While the epic is open a
  finished story stays, ticked, because that is what lets a reader say "eight of eleven, these three
  remain".
* **A story states when it is done.** Its `Touches` cell ends with `Done when` and a check
  a reader can run. The check names rows that read `verified`, a gate that passes or a test. A story whose
  check did not pass is not ticked; the delivered part is renamed and the rest becomes a new row.
* **A milestone tags a story, never a feature.** A feature lives: it has an MVP form, then others.
  `--` means no release scope names the story, which is why transverse work waits.
* **An epic holds no `post-V1` story.** One such row holds an epic open for ever, so section 8
  carries them.
* **A defect is fixed in the milestone that finds it.** A `failing` row is code that breaks its rule
  today. Its story carries the milestone in progress: the earliest one with a story still open. A
  later milestone carries only what is not built. `contracts.py` refuses a `failing` row bound to a
  story of any other milestone.

**An epic key is an identifier, not a rank.** `EF` precedes `E4`, and `E4` is the current objective. A
key is minted once, because a few hundred contract rows cite one. The `Milestone` column orders the
work.

A story is cited by its name, and the name is a join key. `contracts.py` fails on a citation this
file does not answer (`ambre-spec-template.md` section 1, `ambre-spec-architecture.md` section 4).

---

## 2. SHIPPED

Closed epics live in `ambre-roadmap-journal.md`, whole. They are named here so a citation resolves.

| Epic | What it delivered |
| :--- | :--- |
| `E0` | the three-thread stack, the GPU pipeline, the chrome, the crypto primitives |
| `E1` | the Trame vertical end to end: encrypted, synchronized, live through the Blind Relay |
| `E2` | the launcher, the sharing UI, the snapshot client legs |
| `E3` | the Trame MVP: a usable spreadsheet |
| `ET` | CI, the three test tiers, the convergence property test, the first UI belts |
| `E4` | the Toile MVP: an infinite collaborative whiteboard with a presentation mode |
| `EF` | the `.ambre` file, the schema register, and the fixture corpora with their gate |

---

## 3. E5 -- APPLICATION EXPANSION  `[ ]`  *current core objective*

Prose, then Murmure. Prose is the text pilot, and it validates the axis neither Trame's grid nor
Toile's plane touches. It unblocks Toile's inline rich text and Trame's text colours.

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[x]` `ARCH P2` -- the Prose engine core | MVP | the document model: text, blocks, marks, nesting |
| `[x]` A refused command names what it refused | MVP | eight rows across seven features require a refusal to reach the author -- a nesting break, a malformed mark value, a list command, a band field, a footnote, a table operation, a comment -- and the engine declares one untyped error carrying a number and a message, which names neither the command nor a reason. Two features were written against a `CommandRejected` variant no plane declares. A typed error, and one event that carries it |
| `[x]` Pagination | MVP | measured pages: breaks, orphans and widows, kept-together blocks, footnote placement, incremental relayout |
| `[x]` Two peers read the same page number | MVP | the layout is derived from the document and the page format, both shared, and from the faces the device actually loaded: an unknown family resolves to whichever base face is there (`PROSE-034`), so two collaborators measure the same paragraph differently and land on a different page count. A comment naming a page, a cross reference to one and a table of contents entry all rest on an agreement no row states and no decision refuses |
| `[x]` The faces reach the paginator | MVP | the measuring context is sealed (`PROSE-114`), so it shapes nothing until faces are registered into it, and nothing registers any: `ProseState::load_faces` has no caller. The Main thread already fetches the base faces and ships them for the PDF export (`LogicCommand::LoadFonts`), which drops them into the export context alone. Keep the bytes in `LogicState`, hand them to every open Prose document and to each one opened after boot, and route a lazily fetched script face the same way so `PROSE-111` has a trigger outside the tests |
| `[ ]` The same page on two architectures | V1 | two peers pin the same faces and the same shaper, and `PROSE-109` compares two font contexts on one host. Nothing compares two machines, where hinting, curve evaluation and a maths library may not accumulate alike. An `insta` snapshot of a layout measured on the host, replayed under `wasm-bindgen-test` in a browser and compared byte for byte. The Story carries the headless wasm run the repository does not have yet |
| `[x]` Styles, lists and headings | MVP | character and paragraph styles, ordered and bulleted lists, numbered headings |
| `[x]` Tables, table of contents and word count | MVP | tables with merged cells and styles, a generated table of contents, a live word count |
| `[x]` Headers, footers, find and replace | MVP | page headers and footers with dynamic fields, and find and replace |
| `[x]` A deleted block comes back whole | MVP | an undo of a block deletion recreates the block under a new identity, and its body is keyed by the old one: the text comes back empty and a reference to the block stays broken (`PROSE-FEAT-133`). A deletion that keeps the node, hidden rather than removed, would hold both |
| `[x]` Undo, footnotes and cross references | MVP | undo isolated to the local author, footnotes, cross references |
| `[x]` Comments and track changes | MVP | anchored comments, and suggested changes a reviewer accepts or rejects |
| `[ ]` Suggestions beyond typing | V1 | suggestion mode records typing and deleting only, and refuses a split, a join, a block command or a format change while it is on (`PROSE-FEAT-301`). A burst groups by adjacency alone, since the merge window it names rests on an undo interval no document defines (`PROSE-FEAT-085`) |
| `[ ]` A note edit reflows only its page | V1 | every reflow reads the text of every note to decide whether the notes still hold, and an edit to one note's text lays the whole document out again, since its height moved. Past a few hundred notes each keystroke pays for both. Keeping each note's height with its text, and resuming placement at the first page that note sits on, would bound it to the page |
| `[x]` Page rendering | MVP | pages on the canvas with virtual scrolling and cached text |
| `[x]` An edit reaches the layout | MVP | every caller laid out with an empty dirty set and a remote delta dropped the layout whole, so a local edit that kept its heights was never re-shaped and a remote one re-measured the document. A subscription on the document now names the blocks each commit and each import wrote (`PROSE-123`) |
| `[x]` Prose on screen | MVP | the host that mounts the page view on the canvas, fills its palette from the tokens and routes the layout event; the status bar; the drawing of a table |
| `[x]` Styled table cells | MVP | a cell drawn with its fill, its borders, its padding and its vertical alignment |
| `[x]` Input and cursor | MVP | typing, caret navigation, click and drag selection, optimistic display |
| `[x]` Links and bands under the caret | MVP | a press inside a link places the caret and a modified one opens it through the host; the link cursor; the caret entering a header or a footer |
| `[x]` The caret in a footnote | MVP | a note's text lives in its entry, not in a block, and the caret, the selection and every text command address a block. Putting the caret in a note needs an address for that text on the bus, and the edits typed there to reach it |
| `[x]` The format bar | MVP | the contextual format bar over a selection, its mixed state, and its ranges caught up with concurrent edits |
| `[ ]` The format bar's other controls | V1 | the bar carries the five presence marks alone. The link, heading, quote, colour and size controls, superscript, subscript and clearing the formatting reach no control yet |
| `[x]` The slash menu | MVP | the slash menu: where it opens, what it records, the one command it sends and how it closes |
| `[ ]` The slash menu's other items | V1 | the menu reaches nine features through twelve items. A picture, a footnote, a cross reference, a comment, an equation and the heading levels past three have no item |
| `[x]` Input-method composition | MVP | input-method composition through the hidden text field, its preview on the canvas, and double-click word selection |
| `[x]` The draft reads like the text it joins | MVP | the text after the caret moves right by the width of the draft and of the composition until the layout comes back, and the draft is shaped in the style of the block it joins |
| `[x]` Inline marks reach the glyphs | MVP | the layout shapes each span of a block, a note included, in the bold and italic marks it carries, so a mark reaches both screens on the next layout |
| `[x]` The draft takes the marks at the caret | MVP | the draft and the composition take the bold, italic, underline and strike-through of the text before them, as the document will lay them out: each mark expands after its range |
| `[x]` Underline and strike-through are drawn | MVP | the layout shapes the underline and strike-through marks, and the view draws each as a line across its glyph run, at the offset and thickness the font gives |
| `[x]` The layout travels as a delta | MVP | the view takes the layout the Logic thread handed by its pointer, with the flow and page indices built on the Logic thread, and the Logic thread keeps a handle on each layout the view holds so the view never frees one (`PROSE-UI-205`) |
| `[x]` Prose hands the view its visible pages alone | MVP | the Logic thread keeps the whole layout and hands the view the pages it asks for, with a margin, as Trame and Toile hand their viewport. The cost of a layout handed follows the pages shown, not the document (`PROSE-125`, `PROSE-UI-211`, `PROSE-UI-212`) |
| `[x]` A provisional paragraph on the UI thread | MVP | the draft never wraps, and a deletion or the paragraph Enter opens shows nothing until the Logic thread answers, which on a slow device is several frames. A deletion also clears the draft, so a word typed and corrected at once vanishes, then comes back. The view shapes the edited paragraph itself with the Logic thread's measurer and faces, and the layout that answers replaces it. The budget register keeps paragraph shaping off the UI thread (`ambre-spec-budgets.md section 2`), so that row moves with it; `PROSE-UI-006` bans a full layout alone and stands. Shaped with the same code, faces and styles, it breaks where the Logic thread will, so the swap moves nothing. A paragraph that grows pushes the blocks below it down until the pagination answers |
| `[x]` A provisional cell on the UI thread | MVP | text typed into a table cell shows nothing until the Logic thread answers, where a paragraph, a band and a note show it at once. The cell's paragraph is shaped at its column's width, and its row grows until the answer (`PROSE-UI-213`) |
| `[x]` Collaborative cursors and annotations | MVP | remote carets, comment highlights, track-change highlights |
| `[x]` Copy and paste | MVP | copy, cut and paste through the Logic thread: the plain text and this application's own bytes on the system clipboard, the marks and the block types kept on a paste back |
| `[x]` Rich clipboard: HTML in and out | MVP | a copy writes HTML beside the text and the private bytes, and a paste from a web page, Google Docs or Word keeps its blocks, lists, links and marks, read on the Logic thread through an allow-list with a depth cap |
| `[ ]` Rich paste: tables and pictures | V1 | an HTML table lands as a table and a pasted picture lands in the text, where the reader now keeps one paragraph per cell and drops the picture |
| `[x]` Paste as plain text | MVP | a chord and a context-menu entry that insert the plain-text mirror alone, as every word processor offers |
| `[x]` The interactive ruler | MVP | the ruler above the page, its indent handles following the caret's block, and a drag that writes the paragraph once on release |
| `[x]` The first-line indent is laid out | MVP | the layout breaks every line at the left indent and never reads the first-line one, which the document stores and the schema declares. A paragraph's first line starts at its own indent, a hanging one included, and the ruler gains the handle for it |
| `[ ]` Hanging indents | V1 | a negative first-line indent starts the first line left of the rest, as a bibliography or a definition list sets it. The layout carries the indent as a box ahead of the text, which has no negative width, so the value is laid out as none and the ruler's handle stops at the left indent |
| `[x]` A drag that scrolls, and text dropped into the page | MVP | a selection dragged past the edge of the window scrolls and follows the text, and text dropped into the page lands at the drop point |
| `[x]` Inertial scroll | MVP | a flick keeps the column moving, slowing down, and stops at either end of the column |
| `[x]` The native format | MVP | a Prose document saved to and read from an `.ambre` file |
| `[x]` A Prose fixture corpus | MVP | Trame and Toile each carry a fixture generator and Prose carries none, so `fixtures/` holds no Prose document and the schema has reached version 9 with no artifact frozen at any version. Each migration is held by one hand-built document in a unit test, never by a real file. A seeded generator writing a document with styled text, a table, a band, a footnote and a comment gives the current corpus, and the next schema bump its first frozen artifact (`ambre-spec-fixtures.md`) |
| `[x]` Markdown interop | MVP | Markdown import and export, with a report of what was lost |
| `[x]` An import lands whole or not at all | MVP | an import writes block by block into one pending transaction, and a write the engine refuses midway leaves the earlier ones pending for the next commit to take (`PROSE-IO-011`). Apply on a fork and merge on success, or validate every operation before the first write |
| `[ ]` HTML interop | V1 | HTML import and export of whole documents, beside the clipboard reader |
| `[ ]` DOCX interop | V1 | Word import and export, comments and track changes included |
| `[ ]` PDF export | V1 | a paginated PDF with working links and metadata |
| `[ ]` Images | V1 | images in the document, resolved through the shared media flow and embedded on export |
| `[ ]` Operations bound to their signer | MVP | every Loro operation of every application traced to the key that signed its delta |
| `[ ]` Signed membership log | MVP | every invite, revoke and role change signed by its author, so a client checks the member list the relay serves, for documents and channels alike |
| `[x]` The key chain | MVP | every older document key reached from the current one through signed links, so a reload, a new member, a self-join or an eleventh rotation still opens the history. That includes the name and the pictures a revoke leaves under the key they name. The check of a link's signer against the log lands with the `Signed membership log` Story (`ambre-roadmap.md` E5), as `SYNC-121`. Done when `SYNC-042`, `SYNC-116` to `SYNC-120` and `SYNC-122` read `verified` |
| `[ ]` Document residency | MVP | a bounded cache of the documents in memory and on disk, and a light subscription that keeps activity counters for documents not loaded |
| `[ ]` `ARCH P7` -- the Murmure engine core | MVP | the message model, and a typed command surface in place of the opaque byte payload of the channel plane |
| `[ ]` `SYNC P4` / `SRV P1` -- channels | MVP | channel membership and history, and unread tracking |
| `[ ]` Spaces and the channel catalogue | MVP | a space, its catalogue of public channels with their names, categories and keys, and joining a public channel with no other member online |
| `[ ]` Private channels and direct messages | MVP | invite-only channels, and direct messages between two or more members with an optional name |
| `[ ]` Channel segments and finalization | MVP | the chain of segments, closing and finalizing through a relay lease, and the second client that confirms a seal before the deltas go |
| `[ ]` Message authorship | MVP | an edit, a delete or a reaction takes effect only from the member entitled to it |
| `[ ]` Message edits and deletes | MVP | editing and deleting a message, amendments to a final segment, and the re-finalization that turns a delete into an erasure |
| `[ ]` Reactions, mentions and threads | MVP | emoji reactions, `@` mentions, and reply chains with a reply count on the root |
| `[ ]` Typing and presence in a channel | MVP | typing indicators and presence, sealed ephemeral frames on the documents of a channel |
| `[ ]` The Murmure layout and keyboard | MVP | three columns on a desktop, one view at a time on a phone, channels first in the palette, and the Murmure shortcuts |
| `[ ]` The message list and the composer | MVP | the virtualized message list, and a composer built on the Prose editor |
| `[ ]` Notifications in an open tab | MVP | a system notification for a mention or a direct message while a tab is open |
| `[ ]` Search in loaded segments | MVP | local search over the active segment and the final segments already fetched |
| `[ ]` Attachments | MVP | files and images attached to a message through the shared media flow |
| `[ ]` Emoji | MVP | Unicode emoji drawn in messages and reactions, with a picker |
| `[ ]` Animated images | MVP | an animated GIF plays in the message list, and stays still under reduced motion |
| `[ ]` Murmure measured on a phone | MVP | the Murmure budgets measured in WASM on a mid-range phone, in place of the desktop figures and the ratio they assume: segment replay and snapshot import, the JSON authorship check, the envelope open, the rotation of a space leave, the color emoji font size, and the two residency memory budgets. The `segment` bench of `ambre-murmure` is the starting point. It also measures a cold open end to end, relay read included, against the cold start row |
| `[ ]` Moderation | V1 | a moderator hides the message of another member |
| `[ ]` Custom emoji | V1 | emoji a space admin adds for the members of that space |
| `[ ]` Presence settings | V1 | turning off the presence and typing indicators, kept in the user settings |
| `[ ]` Push notifications | V1 | a notification with every tab closed, a level per channel, and mentions routed by the relay |
| `[ ]` Mute and hide | V1 | a channel muted or hidden, kept in the user settings |
| `[ ]` The personal sidebar | V1 | personal groups and order of channels, and bookmarks, kept in the user settings |
| `[ ]` Followed threads | V1 | one view of every followed thread with unread replies, and marking a thread as read |
| `[ ]` Search on a new device | V1 | a signed index per final segment, queried before its segment is fetched |
| `[ ]` Space retention | V1 | one retention duration per space, applied by the relay |
| `[ ]` Direct message conversion | V1 | a direct message becomes a private channel of a space, with its history or from a fresh start |
| `[ ]` Forum channels | V1 | posts with a title and tags, each opening its own thread |
| `[ ]` Chat import | V1 | an archive of Slack or Discord imported into a space |
| `[ ]` Two channels side by side | V1 | a split view that pins and shows two channels at once, each with its own list and composer |
| `[ ]` Pinned messages | V1 | messages pinned to a channel for every member, in a `pinned` list of the channel `meta` document |
| `[ ]` A link to a message | V1 | a link that opens a channel at one message, for a member of the channel |
| `[ ]` Archived channels | V1 | a channel kept readable and closed to writes |

---

## 4. E6 -- V1 COMPLETION  `[ ]`

### Foundations

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[ ]` Large-file import | V1 | a very large import or export stays responsive and reports its progress, in Prose: the conversion is sliced so the Logic thread keeps serving edits between slices. Until then a Prose import past 16 MiB is refused |
| `[ ]` `ARCH P4` -- offline durability | -- | edit while offline and lose nothing on reconnect |
| `[x]` `ARCH P9` -- the Logic thread drains before it answers | MVP | the Logic loop takes one message, applies it and answers the view, so a burst of ten keystrokes costs ten layouts or ten derivations, nine of them stale before they are drawn. On a slow device the answer falls further behind with each keystroke. Every queued command applied first, then one answer per document written, bounds the lag to one pass whatever the typing rate |
| `[x]` Keystroke latency probes | MVP | three budgets govern typing -- the local echo, the confirmation and the moved caret -- and nothing measures any of them. Marks on both threads, run on a slow phone and a slow laptop, over a long document, a burst and a large paste (`ambre-spec-budgets.md`) |
| `[x]` The confirmation budget counted in frames | MVP | the confirmation is now counted in frames: two, the echo frame and the one that applies the answer (`ambre-spec-budgets.md` section 1). The probe counts them, and every key of a release run lands in two. That run drew at 30 Hz, where a slow pass still fits in a frame. At 60 Hz it holds only if the Logic pass that answers a key stays under 4 ms, and it measured 7.3 ms at the median and 19.7 ms at the 95th percentile. Closes with the story below |
| `[x]` A keystroke pass under 4 ms | MVP | the Logic pass that answers one key in a 2 000 character paragraph of a long document takes 7.3 ms at the median and 19.7 ms at the 95th percentile in a release build, against 4 ms. Split the pass first: the CRDT apply, the shaping, the pagination and the events it emits, each stamped. Then bound the shaping to the paragraph an edit touched, or to its line |
| `[x]` No lock on the main thread | MVP | one probe run in three dies on "Atomics.wait cannot be called in this context", and an earlier one on `unreachable`: the main thread blocks on a lock another thread holds, which the browser forbids. Typing then kills the page. Find the lock, and give the main thread a path that never waits |
| `[ ]` `CANVAS P2` -- frame quality | -- | frame profiling, allocation-free painting, dynamic theme, content accessibility, translation and right-to-left |
| `[ ]` Antialiased vector geometry | V1 | every screen pipeline runs at one sample and the solid shader carries no coverage term, so a diagonal or a curve is a hard staircase. Only glyphs escape, through a CPU-rasterized coverage atlas. A high-density display hides it by paying twice the pixels; a 1x monitor does not |
| `[x]` Pictures on the screen | V1 | a picture imported onto a board is painted: the GPU uploads it once, mipped and sampled trilinear under an evicted budget, and the view asks the Logic thread for the pixels of what survived the cull. It paints at thumbnail resolution, which is every pixel this side holds |
| `[ ]` A board of photographs stays fluid | V1 | every upload lands in the frame that first draws it, so opening a board on thirty pictures decodes thirty thumbnails at once; and zoomed far enough out the texture budget evicts what the next frame asks for again, re-uploading pyramids whose top levels that zoom never samples. It wants an upload budget per frame and a chain truncated to the scale drawn at -- which needs the decoded pixels kept to re-upload from (`CANVAS-007`) |
| `[ ]` A web page loads only when asked | V1 | an embed fetches its page the moment a pan brings it on screen, so the site, and anyone on the way to it, learns that this user has this board open -- a flow that leaves the client, which the zero-knowledge boundary rules on the relay alone and has no row for. The user decides instead: ask, always, or never, remembered per site and never shared |
| `[ ]` The icon pipeline | -- | vector icons on the canvas. The last open item of `CANVAS P0` |
| `[ ]` Clipped paths on the canvas | -- | a shape crossing a clip edge is drawn whole today. Clip it, and give a stroke a screen-constant width |
| `[ ]` The adaptive chrome | V1 | the navigation, the side panels, the toolbar overflow and the wide data views adapt at the three declared widths. Today two labels and one bottom sheet move, and the regions the design names exist at no width |
| `[x]` The stylesheet reads the tokens | MVP | fifty literal colours in the shell stylesheet bypass the one token source, so a theme switch moves everything except them. Done when `UI-THREAD-029` reads `verified` |
| `[ ]` A provider account is unlocked or set up | V1 | the home screen has no passphrase field and no setup, so an account that holds its keys cannot open them and one that owes them is never asked. The setup refusal reaches a screen that ignores it. Needs `The provider sign-in reaches the client` first. Done when `CHROME-097` to `CHROME-100` read `verified` |
| `[ ]` User settings | V1 | a sealed store of one user's settings that follows them across devices: Murmure mute and layout first, then macro approvals and secret values |
| `[ ]` The PDF writer is fetched on demand | V1 | `krilla` sits behind a Cargo feature the launcher enables, so every user downloads the writer whether or not they ever export. A feature cannot defer it: the build emits one wasm artifact per instance, so the writer needs a second artifact fetched on the first export, as the budget already prescribes for Prose's interop |

### Trame

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[x]` One style register per cell | -- | a cell's whole style converges as one value, so no concurrent edit can tear it |
| `[ ]` Paste as plain text in Trame | V1 | the plain-paste chord and a context-menu entry land the plain-text mirror as literal cells, the typed payload and the tab's own copy ignored |
| `[x]` Column and row reorder | MVP | drag a whole column or row to a new position, references following the data |
| `[ ]` Incremental remote apply | -- | a remote change costs the cells it touched, and the UI re-fetches the range that changed instead of the whole viewport |
| `[ ]` Cached sheet indexes | -- | a scroll on one sheet no longer rebuilds every other sheet's index |
| `[ ]` Stable engine sheet indexes | -- | switching a tab no longer resets the engine |
| `[x]` References stored by identity | MVP | a formula stores the rows and columns it names, so an insert widens a range, a delete breaks a reference into `#REF!`, and a reseed or a concurrent edit keeps each one on its cells. Done when `TRAME-008`, `TRAME-171` and `TRAME-226` to `TRAME-229` read `verified` |
| `[ ]` A recompute bounded by what changed | -- | the cycle check and the topological walk cover the cells that changed, not the whole graph |
| `[ ]` Engine completion | V1 | whole-column ranges, reference-preserving functions, the advanced function set, convergent deletion of the last sheet |
| `[ ]` Data tools | V1 | personal filter views, conditional formatting, data validation, and a total that skips filtered rows |
| `[ ]` Grid surface | V1 | frozen rows and columns, function autocompletion, live peer cursors, an auto-fit that measures the whole column instead of the viewport cache |
| `[ ]` xlsx interop | V1 | Excel import and export: values, formulas, styles, merges, column widths |
| `[ ]` Cell comments | V1 | a note anchored to a cell, surviving a sort and a reorder |
| `[ ]` Media in a sheet | V1 | Trame declares images and resolves none. Wire it to the shared media flow |
| `[ ]` Persisted page setup | V1 | paper size, margins and orientation stored with the document instead of asked at export |
| `[ ]` A machine-readable error | V1 | an error carries a code a host can branch on, not only a sentence |
| `[ ]` Bounded float objects | V1 | an absurd image or chart size cannot make layout cost unbounded |
| `[ ]` A fill extrapolates a date series | V1 | a drag over two dates repeats them: the fill extrapolates numbers alone, and a date pattern the corpus promises is detected nowhere |
| `[x]` Trame on the shared kinetic scroll | MVP | the grid keeps its own momentum, apart from the "One kinetic scroll" model (E8): it moves onto it, a finger pans and flicks the grid, and a range dragged past the edge of the grid scrolls and follows. Today a finger selects and nothing scrolls under a drag |
| `[ ]` Trame answers once per pass | V1 | since `ARCH P9` the Logic thread answers each document once per pass, but the grid engine still recomputes its filters, its totals and the visible cells after every command. A burst of typing or a fast fill pays that once per command, all but the last stale. Scheduled only if the Keystroke latency probes Story (`ambre-roadmap.md` E6) shows the grid falling behind |

### Toile

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[ ]` Paste as plain text in Toile | V1 | the plain-paste chord and a context-menu entry land the plain text as one text element at the pointer, where a paste of text today lands nothing |
| `[ ]` Snap to the board grid | V1 | a drag, a resize and a nudge quantize to the grid step the board asks for, which is drawn today and magnetic nowhere |
| `[ ]` Rich text on the board | V1 | a note body carries per-run formatting, on the shared text substrate Prose unblocks. Today it carries one style per element |
| `[ ]` Engine and render optimization | -- | a large board stays fluid: bulk index updates, cheaper layer diffs, memoized routing, atlas eviction |
| `[ ]` Figma and Miro import | V1 | open a board exported from either, reporting every construct it had to drop |
| `[ ]` PNG export | V1 | a board or a frame rendered back to a raster image, without blocking the visible frame |
| `[ ]` An export does not freeze the board | V1 | the deck's PDF export runs to completion inside the Logic thread's command loop, so a large deck stops every local edit and every remote apply for as long as it takes -- the budget says 3 s at 50 slides, and no benchmark has ever measured it. Take the walk off the loop, and measure it |
| `[ ]` A deck exports at full resolution | V1 | a picture reaches the page from its inline thumbnail, capped at a 256 px edge, so a deck prints at screen quality. The full blob is ciphertext on the relay and the Logic thread has no path that fetches and opens one; give it that path, which the export must then be able to await |
| `[ ]` The standalone showcase bundle | V1 | a board exported as a self-contained page that renders without the relay. It leaves the zero-knowledge boundary, and says so |
| `[x]` A frame carries a title | MVP | the title is designed and written by no command, so find can never match a frame and the link chooser lists every one of them as "Untitled frame" |
| `[ ]` Auto-layout reaches a frame | V1 | the solver has no caller outside its own tests and the command has no menu entry, no inspector row and no chord: a layout rule is stored, converges, and moves nothing on screen |
| `[ ]` Boolean operations reach a surface | V1 | union, intersection and difference are built and tested behind a command no menu entry, no button and no chord invokes |
| `[ ]` Every connector shape a user can make | V1 | the format stores path kinds and anchors the tool writes for none of them, so a document can hold an arrow no gesture produces |
| `[x]` Toile on the shared kinetic scroll | MVP | a finger pans the board and a flick keeps it moving, on the "One kinetic scroll" model (E8), wherever the active tool does not draw; and an element or a marquee dragged past the edge of the board pans it. Today a finger acts as the mouse, with no flick, and nothing pans under a drag |
| `[ ]` Toile answers once per pass | V1 | since `ARCH P9` the Logic thread answers each document once per pass, but the board engine still sends its scene changes after every command. A drag or a burst of typing on a note pays that once per command, all but the last stale. Scheduled only if the Keystroke latency probes Story (`ambre-roadmap.md` E6) shows the board falling behind |

---

## 5. E7 -- CORRECTNESS AND SECURITY DEBT  `[ ]`

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[x]` A lost edit is never shown as applied | MVP | a full command queue drops a lossless command in both applications: Toile logs a warning inside its edit send, and Trame returns the error to a call site that logs it and then writes the same text into its own view state, so the user reads an edit the document never received. The undo group boundary is dropped outright beside it (`BUS-011`) |
| `[x]` A dropped lossy request does not leave a stale screen | MVP | the lossless class now holds its commands, and the lossy one still drops silently into a screen that already moved: a column resize writes the local width and sends on the lossy class, whose send returns nothing, so the grid keeps a width the document never took; a status-band refresh drops the same way and leaves the totals of the previous selection on screen; and a row or column insert is the one document mutation sent lossy, so the active cell moves and nothing is inserted (`TRAME-UI-127`, `TRAME-UI-136`, `TRAME-UI-048`) |
| `[x]` One clipboard host | MVP | the asynchronous write replaces the ownership token the copy event wrote, so a same-tab paste reaches the worst rung of the fidelity ladder; and the toolbar Paste button is a second path that skips the ladder and names one application inside the launcher |
| `[x]` The provider sign-in reaches the client | V1 | the provider control navigates the page to the route that answers the provider's address, so the browser shows that answer and never leaves; and nothing reads the return. A deployment without the development mode signs nobody in (`CHROME-095`, `CHROME-096`, `CHROME-101`) |
| `[x]` `SYNC P4` hardening | MVP | a burst from the relay fills the path to the client, and the thread that draws the screen blocks on it. The socket never slows down. Done when `SYNC-059` reads `verified` |
| `[x]` The production sign-in path | V1 | the two provider routes are registered under a fixed provider name while their handlers read one from the path, so every call answers an internal error and the only sign-in a deployment offers has never run; and the rate limiter sits inside the nested router, which strips the prefix its classifier matches on, so no request is ever counted against the brute-force tier |
| `[ ]` A document is compacted as its history grows | MVP | the relay never asks anyone for a snapshot, and that request is the only trigger of the client's whole compaction path. So no document is ever compacted: every cold open replays from the first delta, the delta index is never trimmed, and the cold-start budget is unreachable. The same pass removes for good a Prose block a deletion hid, once the session that hid it has ended (`PROSE-121`); a peer offline but still open may then undo into nothing, the loss a deletion caused before. Murmure needs it first: an active segment holds a bounded tail of deltas past its snapshot (`MURMURE-222`) |
| `[x]` A deleted document leaves nothing | MVP | the delete purges the objects, the ordering state and the rows, but an update or a flush in flight can write after the purge. Nothing fences the document while it goes (`SRV-017`, `SRV-057`). A Murmure channel or space delete needs it first (`MURMURE-243`) |
| `[ ]` A delete completes once it has begun | MVP | the tombstone and the second sweep live in the ephemeral store alone, so a reset during the window loses both and a late object stays for good. A delete whose rows fail to go after the sweep leaves a gutted document that members write to again once the tombstone expires. Done when the delete is marked in the register first, its sweep is scheduled there, and a pass finishes any delete a failure interrupted (`SRV-165`) |
| `[ ]` The hot tail of deltas lives in the index | MVP | one object per delta costs an object request for a few hundred bytes, so a replay of 250 deltas makes 250 requests. A small delta moves into its index row, a capped hot tail stays in the register, and the rest goes to batch objects |
| `[x]` A revoke either completes or changes nothing | MVP | the removal is posted first and the rotation can fail after it, so the member is out of the list and their key still opens new content. A member who leaves posts the removal alone, and the relay refuses the old key until someone rotates |
| `[ ]` A member can leave a document from the screen | MVP | the worker takes the leave command and the relay records the rotation owed, but no screen sends it, so a member cannot leave a document shared with them (`SYNC-127`). Done when the sharing screen offers the departure to a member who does not own the document, and a test drives it to the command |
| `[ ]` A member invited during a revoke gets the new key | MVP | a revoke reads the members, then posts the copies of the new key for them. A member invited between the two gets no copy and keeps the previous key, which the relay now refuses. Done when a test races an invitation against a revoke and every member left holds the current key |
| `[x]` Every sealed object names its key | MVP | the document name, every media blob and every wire message carry no identifier of the key that sealed them, so no reader knows which key opens one. Done when `SYNC-115` and `SYNC-125` read `verified` |
| `[x]` A peer on an older wire is told to update | MVP | a frame of a superseded wire version is dropped as a decode error, so a client a cached page kept alive stops syncing and tells its user nothing. And nothing ties the version byte to the layout, so a later change can move the fields under the same byte. Done when `SYNC-137` and `SRV-166` read `verified` |
| `[x]` A sealed name or blob says its layout | MVP | a name or a blob carries no format version, and the relay keeps a blob across every release. The next change to its header makes each stored picture fail as forged or as sealed under a foreign key, never as an older layout. Done when `SYNC-138` reads `verified` |
| `[ ]` A snapshot follows each rotation | V1 | no snapshot seals the state after a rotation, so the history of a document stays a chain of deltas across the key change |
| `[x]` The media address is not the plaintext | MVP | a blob is addressed by the hash of its plaintext and the relay stores it, so the relay can confirm that a document holds a file it already knows. The media type travels beside it, and a blob is bound to neither its document nor its address. Murmure attachments need it first (`MURMURE-129`) |
| `[x]` The document name is sealed like every other payload | V1 | the sealed name takes the bare cipher, with no author signature and no commitment tag, under one constant string as additional data. So the name of one document opens under any other holding the same key, and nothing says who wrote it (`SYNC-018`, `SYNC-068`) |
| `[x]` A sealed blob is bound to its document and its address | MVP | a blob's additional data is one constant string, so a blob of one document opens under any other holding the same key, and a download takes whatever object storage returns without recomputing the address. Murmure attachments need it first. Done when `SYNC-051`, `SYNC-052` and `SYNC-074` read `verified` |
| `[ ]` The author of a sealed object is a member | V1 | a name or a blob carries its author key, and the open checks the signature against that key without asking whether it belongs to a member. Needs the signed membership log first. Done when `SYNC-135` reads `verified` |
| `[x]` A create fails on a foreign answer | V1 | the client mints the document id and seals the name under it, and only a debug assertion compares the id the relay answers with. Done when `SYNC-136` reads `verified` |
| `[x]` A note body converges character by character | MVP | a sticky or a text body is one whole value, so of two authors typing in the same note one loses their sentence. Done when `TOILE-030` reads `verified` |
| `[x]` An open editor keeps a peer's edit to its note | MVP | the editor closes by diffing its body against the stored one, so a peer's edit that arrived while it was open is deleted. Done when `TOILE-UI-381` reads `verified` |
| `[x]` An open caption editor keeps a peer's edit | MVP | a caption is a shared text, but its close names no base, so a peer's edit that arrived while the editor was open is deleted. Done when `TOILE-263` reads `verified` |
| `[x]` A text close never hides a peer's words | MVP | the local echo paints the typed note body or caption, so a peer's words merged into it leave the screen until the round trip returns them. Done when `TOILE-UI-382` reads `verified` |
| `[ ]` A note close holds its budget | V1 | the close merges through two character diffs, on Logic and in the UI echo, and nothing measures what that costs on a large body. Done when `TOILE-UI-383` reads `verified` |
| `[x]` Two peers migrating one board keep both edits | MVP | each peer migrates its own copy, and a step that creates a container under a map key makes a rival one, so an edit typed into the losing copy is lost. Done when `SCHEMA-020` reads `verified` |
| `[x]` A large board import does not freeze the board | MVP | a Toile import applies its whole batch in one synchronous pass, so a large file holds the Logic thread and every edit waits on it. Done when `TOILE-IO-028` reads `verified` |
| `[x]` A board costs what it shows and what changed | MVP | every connector is routed and shipped at every camera, and dropping, reparenting or reordering one element walks the whole board, as does every remote change. A selected arrow and an open caption must then survive leaving the frame, which today reads as their deletion. Done when `TOILE-045`, `TOILE-056`, `TOILE-061`, `TOILE-091`, `TOILE-092`, `TOILE-207` and `TOILE-UI-386` read `verified` |
| `[x]` A remote change costs the change, not the history | MVP | every peer writes the board scaffolding under its own identity, so a reader that has sent nothing imports each change against the whole history: 48 ms for one move on a board of 5 000 elements. Done when `TOILE-265` reads `verified` |
| `[x]` A remote change costs the change, not the history, in Trame and Prose | MVP | a workbook and a document write their scaffolding under the opening peer's identity, as a board did until `TOILE-265`. A reader that has sent nothing then imports each change against the whole history. Prose must keep the root paragraph a reader grows (`PROSE-055`). Done when `TRAME-231` and `PROSE-129` read `verified` |
| `[x]` Each application announces its own content | MVP | one application announces its focused content and the others are silent, and every surface names itself a spreadsheet cell to assistive technology. Done when `CHROME-023`, `CHROME-024` and `UI-THREAD-048` read `verified` |
| `[x]` One gesture is one undo item on the board | MVP | a frame membership change whose paint-order clamp fires leaves two undo items, and a multi-selection ungroup sends one command per group, so one gesture takes several undos. Done when `TOILE-014`, `TOILE-048` and `TOILE-094` read `verified` |
| `[x]` A selection drops what a peer deleted | MVP | the view keeps every selected id across a board change, so an element a peer deleted stays selected and each later command over the selection names an id the board lost. Done when `TOILE-UI-385` reads `verified` |
| `[x]` An editor closes on what a peer deleted | MVP | an editor open on an element or a caption a peer deletes stays open, and its close names an id the board lost, so the typed text is lost with no word to the user. Done when `TOILE-UI-387`, `TOILE-UI-388` and `TOILE-UI-389` read `verified` |
| `[x]` A title editor survives a pan | MVP | the title editor clears the selection, so it reads its frame from the culled set alone: a pan that carries the frame off screen closes the editor and renames. Done when `TOILE-UI-390` reads `verified` |
| `[x]` A cell edit lands on the cell it opened on | MVP | a commit names a row and a column that Logic resolves on arrival, so a peer's insert above the open cell overwrites a neighbour, and a peer's sheet delete leaves the editor writing to a sheet nobody can reach. Done when `TRAME-075`, `TRAME-UI-160`, `TRAME-UI-162` and `TRAME-UI-176` read `verified` |
| `[x]` A reply draft survives the panel | MVP | each comment card holds its reply field, and the panel rebuilds every card on each change of the list: a scroll empties a half-typed reply, and a thread a peer deletes takes the text along with no word. Done when `PROSE-UI-231` and `PROSE-UI-232` read `verified` |
| `[x]` An ungroup answers the same from the chord and the menu | MVP | `Ctrl+Shift+G` sends the whole selection and the menu only its groups, so over a group and a sticky the chord shows an error the menu never does. Done when `TOILE-264` reads `verified` |
| `[x]` Remove-from-frame targets the element's own frame | MVP | the removal finds its frame by geometry, so an element parented to another frame is removed from the wrong one and nothing changes. Done when `TOILE-UI-149` reads `verified` |
| `[x]` An element outside its frame's box can still leave it | MVP | the remove-from-frame entry shows only when a frame's box contains the selection, so an element dragged out of its frame stays a member with no gesture to undo it. Done when `TOILE-UI-384` reads `verified` |
| `[x]` A board entry that does nothing says why | MVP | the two frame entries with no enclosing frame and a fit to an empty selection return in silence. Done when `TOILE-UI-153` and `TOILE-UI-160` read `verified` |
| `[x]` A speaker note keeps its place under a peer's edit | MVP | the notes panel measures a keystroke against a mirror only a slide change refreshes, so a peer's edit between two keystrokes moves the splice. Done when `TOILE-FEAT-088` reads `verified` |
| `[x]` A hairline stays clickable at every zoom | MVP | the pick floor is a world distance, so a thin stroke grows hard to hit as the user zooms out. Done when `TOILE-UI-225` reads `verified` |
| `[x]` The culling rectangle carries a zoom it can divide by | MVP | the camera clamps its zoom privately, and the culling type the Logic thread divides by accepts any value. Done when `TOILE-UI-281` reads `verified` |
| `[ ]` The relay survives a panic | V1 | a panic in one handler or one socket task has no catch layer, and a lock it poisons stays poisoned for every later request. One request fails, and a test that forces the panic shows the relay still answering (`SRV-101`) |
| `[x]` A frame opens with the key it names | MVP | an inbound frame is opened by trying the current key, then every cached key, then the current key again, so a frame nobody can open costs eleven attempts. Done when `SYNC-045` reads `verified` |
| `[x]` A verified key is pinned under its owner | V1 | the comparison phase falls back to a freshly generated identifier when the server's is malformed, and files the pin under it. The recipient reads as a first contact on every invitation, so a substituted key never raises the alert and the whole comparison is dead (`CHROME-074`) |
| `[x]` A session before its keys is sent to the setup | V1 | the socket closes a session whose keys are not stored with the code of an expired token, so the client renews into another such session and reconnects until the setup. Nothing tells the rest of the client that a setup is owed (`SRV-164`, `SYNC-133`, `SYNC-134`) |
| `[x]` No session is minted under a fallback key | V1 | a new provider account is inserted with an all-zero signing key and its first session is minted under it. The refresh keeps that key after the real one is registered (`SRV-027`) |
| `[x]` A document row is read for an authenticated caller | V1 | the single-document route queries the register for an anonymous caller and answers not-found afterwards, so the register is read on behalf of nobody (`SRV-018`) |
| `[x]` A download link names an object the document holds | V1 | the media link is signed from the address the caller supplies, without reading the media index, so a member obtains a link for any well-formed address inside their own document (`SRV-058`) |
| `[x]` A refusal carries the conflict, never the store's error | V1 | the sequencer wraps a refused clock as an ephemeral-store failure and forwards it verbatim, so the typed conflict is constructed nowhere and the frame the client receives names the store and quotes its message (`SRV-037`, `SRV-081`) |
| `[x]` The quarantine asks once per gap | V1 | the soft threshold is re-tested on every quarantined arrival, so past nine hundred held updates every further one sends another history request and the relay rate-limits the socket (`SYNC-031`) |
| `[x]` The member list shows no invented identity | V1 | a member row whose identifier does not parse is given a freshly generated one, so the list shows a person nothing can act on and a revoke aimed at that row addresses nobody (`SYNC-038`) |
| `[x]` A failed members request is not an absent document | V1 | a network failure and a genuine absence take the same branch, so the open generates a throwaway key, marks the document local and never hydrates (`SYNC-040`) |
| `[x]` Toile reports what its undo engine holds | MVP | two command handlers of thirty emit the undo state and the view drops the event, so the undo and redo affordances of a shipped MVP report nothing (`UNDO-006`) |
| `[x]` The selection reaches the Logic thread whole | MVP | the active-cell report carries one pair of coordinates, and a header band is clamped to the last non-empty cell of the UI cache. So a disjoint selection answers for the rectangle clicked last, and a column longer than the screen answers for its first screen -- in the total, the sort, the style, the delete and the copy alike (`TRAME-UI-129`, `TRAME-UI-033`, `TRAME-UI-104`) |
| `[x]` Every selection change refreshes the aggregates | MVP | the refresh hangs off the active-cell announcement, and extending a header band is the one selection path that does not announce, so sweeping columns A to D leaves the total of A on screen with no sign it is stale (`TRAME-UI-109`) |
| `[x]` A filter converges as one definition | MVP | the shared filter converges field by field, so a concurrent race can join one user's column to another user's operator. Every peer agrees on a definition no user applied (`TRAME-FEAT-025`) |
| `[x]` One viewport request per frame | MVP | the one-slot queue coalesces the scroll path alone, and two other call sites send past it, so a keystroke answered by a data reset in the same frame as a scroll sends two requests (`TRAME-UI-025`) |
| `[x]` An answer names the request that asked for it | MVP | Toile's extent reply carries no correlator and four surfaces ask -- the selection band, `Fit`, zoom-to-selection and the find focus. Whichever answer lands first is spent on whichever question is pending, so a band refresh in flight when a user hits `Fit` frames the selection instead of the board, and a find step centres the camera on the selection (`TOILE-UI-108`, `TOILE-UI-293`, `TOILE-UI-117`) |
| `[x]` Toile reads the board, not the screen | MVP | the culled set is read as though it were the document: a selected element beyond the edge contributes nothing to the style bar, nothing to the menu scope and nothing to select-all, and the inline editor closes the moment a pan carries its element off screen (`TOILE-UI-070`, `TOILE-UI-134`, `TOILE-UI-033`, `TOILE-UI-210`, `TOILE-UI-195`) |
| `[x]` The paint order holds every element | MVP | the visible-set walk reads the order and nothing else, so an element the order omits is invisible on every peer; and a removal rewrites the list, dropping any entry whose tag this version cannot decode (`TOILE-058`, `TOILE-121`, `TOILE-129`) |
| `[x]` The style bar says what it does not know | MVP | the bar folds a common fill and nothing else, so the colour, the family, the size and the decorations are shown as agreed when they differ; a restyle then writes the first member's corner radius, shadow and weight over the whole selection, on attributes no control offered; and a negative size becomes the auto sentinel (`TOILE-UI-164`, `TOILE-UI-174`, `TOILE-UI-180`) |
| `[x]` A style survives a change to its layout | MVP | the style value is decoded whole and a decode failure falls back to the default, so any layout change silently resets the style of every element already drawn (`TOILE-UI-071`) |
| `[x]` The inline editor shows the committed body | MVP | the pinned style carries the ink and the size alone, so entering edit drops the underline and the strikethrough, forces the body opaque and floors a zero size the board draws as nothing; and the deferred open is a bare flag the next created element claims, whichever it is (`TOILE-UI-186`, `TOILE-UI-187`, `TOILE-UI-188`, `TOILE-UI-194`, `TOILE-UI-209`) |
| `[x]` A locked element is locked on every surface | MVP | the lock is read at the write and nowhere else: select-all takes a locked element, the overlay draws its selection box, and the context menu offers it cut, delete, duplicate and the four z-order entries (`TOILE-UI-231`, `TOILE-UI-241`, `TOILE-UI-126`) |
| `[x]` A marquee takes what it was drawn around | MVP | the marquee returns raw members without climbing the group chain, so it selects the parts a click would have selected the group of; and containment is read on the upright box, so a tilted shape is taken by a rectangle that does not cover it (`TOILE-UI-236`, `TOILE-UI-237`) |
| `[x]` A Toile command that names nothing answers an error | MVP | the move, the restyle and the delete skip an id the board does not hold and answer as though it had; the menu resolver falls through to the empty command list a handled intent returns; a group of one and an ungroup of a non-group refuse in silence; and a reaction names a target the board no longer holds with nothing to check it (`TOILE-194`, `TOILE-201`, `TOILE-UI-152`, `TOILE-055`, `TOILE-159`, `TOILE-FEAT-060`) |
| `[x]` A remote change the board cannot apply is announced | MVP | the failure is reported as an error carrying an empty element id and the canvas logs it without surfacing it; in a batch the others keep merging and the import reports success, so a board silently stops receiving a collaborator's work (`TOILE-214`, `TOILE-215`) |
| `[x]` A dropped display request leaves a trace | MVP | Toile's lossy send matches a disconnected queue alone, so a full one drops a camera request with no log; the cull request goes the same way while the dirty flag is cleared in the same frame, and the board paints the previous set until the camera moves again (`TOILE-UI-313`, `TOILE-UI-275`) |
| `[x]` The board tools let go | MVP | the pan flag is raised on `Space` and lowered by nothing, so one press turns every later drag into a pan for the rest of the session; and a press on a transform handle emits a resize or a rotate whatever the pointer did, costing a delta and an undo item for a geometry nobody changed (`TOILE-UI-256`, `TOILE-UI-252`) |
| `[x]` The deck is one list for every peer | MVP | the presentation order and configuration are created by the first write rather than with their parent, so two peers building the deck at the same moment mint rival lists and one is kept (`TOILE-029`) |
| `[x]` Presentation mode holds the screen it promises | MVP | the overlay pass is never told the mode, so the selection box, the handles, the find hits and the presence cursors paint over the slide; the follow affordance names a presenter and is cleared by nothing; a co-author's note edit stays invisible until the slide changes; a fade-out stops when the gliding camera culls its element; and the transition eases over a constant instead of the deck's own configuration (`TOILE-UI-083`, `TOILE-UI-315`, `TOILE-UI-316`, `TOILE-UI-085`, `TOILE-FEAT-028`) |
| `[x]` The SVG reader keeps the drawing's structure | MVP | the vector walk and the text pass are two passes over one file: text is appended after every shape, reads its top-level coordinates alone and can name no group. A group of one member is dropped without a word, and text the reader refuses is lost with no skip recorded (`TOILE-IO-027`, `TOILE-IO-080`, `TOILE-IO-083`, `TOILE-IO-079`, `TOILE-IO-081`) |
| `[x]` The SVG writer holds what the board holds | MVP | a sticky, a connector, a group, an embed and a stroke each serialize to nothing and the export has no report at all; rotation is read and never written; the frame is anchored at the origin, so anything left of or above it falls outside; a picture leaves as an address no reader outside Ambre resolves, and text without its size or its colour (`TOILE-IO-025`, `TOILE-IO-026`, `TOILE-IO-086`, `TOILE-IO-087`, `TOILE-IO-088`) |
| `[x]` A board the export could not read becomes no file | MVP | a failed snapshot logs an error and writes the header alone, so the file is well formed, carries the right document id and opens as an empty board (`TOILE-IO-069`) |
| `[x]` A pasted frame is a frame | MVP | the capture reads a frame flattened, so its speaker notes land as a plain field the first editor mints a rival container for, and a frame copied alone lands with no membership list -- a later add is refused as though the board did not hold it (`TOILE-137`, `TOILE-138`) |
| `[x]` A dangling arrow is announced however it dangled | MVP | the announcement is emitted on the re-aim path alone, so an arrow whose endpoint was deleted by a peer simply stops being painted; and the event the other path does emit reaches a view that drops it, so no surface offers the repair either way (`TOILE-165`, `TOILE-044`) |
| `[x]` A stroke in flight reaches the other users | MVP | nothing in the workspace sends a presence payload for a board: the in-flight sample field is written by tests only and the presence command has no sender, so a freehand stroke appears on the other screens once it is finished (`TOILE-106`) |
| `[x]` A picture is measured before it is expanded | MVP | the decoder expands the whole picture before the dimension bound is read, so the bound protects the document and not the heap (`TOILE-IO-052`) |
| `[x]` Toile's snapshot command answers | -- | the command returns an empty outcome and no caller sends it; the dispatcher answers a Sync request from the document state instead (`TOILE-093`) |
| `[ ]` Shaped runs bounded to the view | -- | every measured block keeps its shaped run for the life of the block, about 150 KB a page (`ambre-spec-budgets.md section 6`). The view could send the pages it shows, and the Logic thread keep runs near them alone, re-shaping on scroll. A click on a block with no run would then wait for it |
| `[ ]` The memory budgets add up | -- | the UI-thread table of `ambre-spec-budgets.md section 6` totals forty-one megabytes over rows that sum to sixty-five: the picture upload budget was added beside the others and charged to no total. Prose then states fifty megabytes of video memory for three pages against a shared wgpu row of twenty, and the table records the arithmetic rather than settling it. A stated total its own rows exceed holds no regression |

---

## 6. E8 -- THE SHARED MODELS ACROSS THE APPS  `[ ]`

Each row promotes a behaviour shared by nature into one model, so the editors stop reinventing it
apart. Arbitrated in the 2026-08 cross-app review; the target state is in the Foundations spec that
owns the subsystem.

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[x]` One command registry | MVP | one description per command -- title, icon, category, chord -- with the menus, the keyboard and the command palette all derived from it instead of built beside it |
| `[x]` The launcher holds no application logic | MVP | the shell stops deciding what an application's commands do: each application hands the shell one command handler and its own panels and dialogs, and the launcher keeps the integration and the shell |
| `[ ]` One find and replace | V1 | one match session with a live cursor, in every app, instead of Trame's whole-grid re-scan |
| `[ ]` One zoom model | V1 | one zoom engine behind the shared widget: Trame gains keyboard and pinch zoom and zoom-to-selection, Prose gains zoom. Pinch belongs here, in every app, and a finger's pan and flick to the "One kinetic scroll" row |
| `[ ]` One text-editing substrate | V1 | one editing core under both layout drivers: the input pipe, composition, the caret and selection model |
| `[ ]` One interop vocabulary | V1 | the same import and export events, the same conversion report, the same progress model, in every app |
| `[ ]` One document locale | V1 | the document carries its locale, so collaborators on differently configured machines read identical numbers and dates |
| `[ ]` One resize command | V1 | one row and column resize primitive for both table models. No shared table object |
| `[ ]` One media cache | V1 | every decoded image goes through one bounded cache, Toile and Trame included |
| `[ ]` One repaint contract | V1 | Prose and Toile honour the shared partial-repaint modes instead of a parallel mechanism |
| `[x]` One high-density rule | MVP | the shared surface scales for a high-density display once, so no app spec restates it |
| `[x]` The export measures on the shipped faces | V1 | the PDF-export `FontContext` is a `parley::FontContext::new()`, which discovers the faces of the machine. Parley then falls back to one of them for any codepoint the asked family misses. It is the hole `PROSE-114` closed for pagination, still open for export. `PDF-004` pins two exports on one peer, and section 5 of `ambre-spec-pdf-export.md` wants one file out of two peers with no row holding it. Seal the export context the way `ambre_prose::layout::fonts::sealed_context` seals the measuring one, and write the row that says so. The same question is open for the UI context `ambre_canvas::fonts::font_context_from` builds |
| `[ ]` One selectable PDF | V1 | one glyph-to-character mapping in the shared surface, so exported text is selectable and searchable in every app instead of drawn shapes alone |
| `[ ]` One undo bound | V1 | a bounded undo depth in every app. What merges into one step stays per app |
| `[x]` One keymap | MVP | the `Ctrl` and `Cmd` fold, both redo chords, and the legacy copy and paste chords, decided once |
| `[x]` One kinetic scroll | MVP | one momentum model under every app: a flick's release speed, its slowdown and its stop, the slop that tells a tap from a pan, and the speed a drag held near an edge scrolls at. Prose moves onto it. Trame coasts on a trackpad glide where Prose leaves it to the platform, with a different curve: which one holds is decided here, once. Trame and Toile adopt it in their own rows |
| `[ ]` One name for the moving selection end | V1 | `focus` everywhere; the fixed end stays `anchor`. A rename |
| `[ ]` One floating-menu shell | V1 | one placement, flip and keyboard model under every popover and floating bar |
| `[ ]` One performance read-out | V1 | the frame read-out available in every app, not in Trame alone |
| `[ ]` Links in Trame | V1 | clickable per-cell links, on the link and open-URL model shared with Prose and Toile |

---

## 7. E9 -- THE VERIFICATION DEBT  `[ ]`

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[ ]` Hot-path benchmarks and the dependency audit | -- | measured crypto costs, and a machine check that no plaintext crate reaches the boundary |
| `[ ]` The name resolver reads the manifests | -- | five `Cargo.toml` files justify a dependency by naming the spec that needs it, and `spec_refs.py` opens neither a manifest nor any file outside `crates/**/*.rs` and `specs/**/*.md`, so those citations are held by nothing |
| `[ ]` The performance budgets given a mechanism | -- | about thirty budgeted operations across the seven tables of `ambre-spec-budgets.md`, held by one bench. A budget that cannot fail a build is not verified |
| `[ ]` The review status means something | -- | every spec reads `Status: Draft`. The two other values `ambre-spec-template.md` declares are defined by no row and set by no one, so the corpus cannot tell a reviewed document from an unread one |
| `[x]` The test counts leave the specs | -- | section 6 of `ambre-spec-testing.md` stated a test count per crate, kept by hand and stale on every crate that grew: `ambre-toile` read 341 against 514. A count carries no rule, and `cargo test` answers it. Done when no spec states how many tests a crate runs |
| `[x]` The verification figures come from the tables | MVP | each spec's `Verification` line states its counts by hand, and the gate only reports them stale. The `human-review` table of `CONTRIBUTING.md` is kept by hand and already misses `FIXTURE-032`. Both are written from the rows, as the ledger is |
| `[x]` A partial status | MVP | a row whose rule is tested and whose wiring is not reads `untested`, the same as a row with no test at all: the template refuses `verified` beside a `partial` gap. The ledger then cannot tell the two apart |
| `[x]` A shared rule proven in each application | MVP | a rule binding every application is held by one test of the shared crate. `KINETIC-002` read `verified` while Toile broke it, and no gate saw. Such a row names one mechanism per application that runs it |
| `[x]` The Trame and Toile view logic tested off the browser | MVP | the handler logic of `TrameView` and `ToileView` (the scroll glide, the coast, the double-click edit target) as pure functions a native test drives with a clock it sets. Done when `KINETIC-002`, `KINETIC-009` and `TOILE-261` read `verified` |
| `[ ]` Each surface's name read back from the page | -- | each view names the shared text pipe after its content, and a test holds the words. Nothing holds the call: a view that names the wrong content, or an embedded one that renames the pipe, fails no test (`CHROME-024`). Needs `The Trame view built natively` first. Done when a test mounts each application's view, top-level and embedded, and reads the name the pipe carries |
| `[x]` The Toile view built natively | MVP | `ToileView` owned the DOM text pipe and painted on a GPU surface only, so no host test built it and every chord, click and board change reached the view untested. Done when `TOILE-097`, `TOILE-UI-061`, `TOILE-UI-062`, `TOILE-UI-197`, `TOILE-UI-207`, `TOILE-UI-232`, `TOILE-UI-233` and `TOILE-UI-239` read `verified` |
| `[ ]` The Trame view built natively | -- | `TrameView` still reads the browser clock, so the wiring of every handler stays out of reach of a native test. Done when it takes the clock as Prose's view does, and a native test builds it |
| `[ ]` The shared crates cite no application rule | V1 | `ambre-app`, `ambre-logic`, `ambre-draw`, `ambre-canvas`, `ambre-bus` and `ambre-chrome` cite Prose and Toile rules outside a file that names the application: a shared layer that knows one application, or a rule that belongs in the foundations. Done when `spec_code_citations.allow` holds no `owner` line |
| `[x]` The tests a rule does not credit | MVP | a test cites a rule whose `Mechanism` does not name it: a missing mechanism or a false claim. Done when `spec_code_citations.allow` holds no `test` line |
| `[ ]` Visual tests of canvas content | -- | no canvas content is verifiable in CI: headless browsers read back a blank frame |
| `[ ]` The Sync and relay belts | -- | the shipped rows no test would fail on, across the Sync worker, the sealed envelope, the delta order, the membership, the key rotation, the media transfer, the relay, its session, its sequencer and its stores |
| `[ ]` The UI-thread belts | -- | the same debt on the frame, the chrome shell, the clipboard, the input pipe, the launcher, the key verification, the design tokens, the render assets and accessibility |
| `[ ]` The thread-separation belts | -- | the shipped rows no test would fail on across the three-thread stack, the message bus and the zero-knowledge boundary: the crate graph a build shape holds by accident, the envelopes no test drives, and the plaintext a gate reads off a manifest |
| `[ ]` The shared document-tool belts | -- | the same debt across the command dispatch, the undo engine, find and replace, the text editor and the PDF export |
| `[ ]` The Trame belts | -- | the same debt across the grid viewport, the cell surface, the number formats and the data tools |
| `[ ]` The Toile belts | -- | the same debt across the board surface, the connectors, the presentation and the interop converters. |
| `[x]` The zero-knowledge boundary held | MVP | the shipped rows that keep plaintext, keys and tokens from the relay, which no test would fail on: the sealed send, the fields the relay learns, the presigned link, the passphrase, the error a caller reads. A break there leaks in silence and shows nothing on screen. Done when every row bound to this story reads `verified` |
| `[x]` The relay's access checks and the delta order held | MVP | the shipped rows on who the relay serves and how deltas converge, which no test would fail on: a removed member, a replay, a role, a clock, a quarantined or repeated update. A break there serves the wrong peer or splits the document. Done when every row bound to this story reads `verified` |
| `[ ]` Undo and the thread failure paths held | MVP | the shipped rows where an undo or a failing thread could reach another peer's work, which no test would fail on: an undo of a remote edit, an empty undo that emits, two tabs, a dying thread, a dropped command. Done when every row bound to this story reads `verified` |
| `[x]` The container and schema belts | -- | the shipped rows no test would fail on across the `.ambre` container, the schema register and the fixture corpora: the header a wire payload must never carry, the bump obligation a reviewer holds today, and the writers a version freeze binds |
| `[x]` Fuzzing the untrusted readers | MVP | a `cargo-fuzz` target on the wire decode, the `.ambre` container, the formula parser and the Markdown reader, each of which is fed by a relay or by a user's file. A panic in the wire decode is a crash any peer of a shared document can cause remotely |
| `[ ]` Fuzzing the remaining readers | V1 | the sealed path, and the Trame and Toile readers of a user's file, have no fuzz target (`ARCH-029`) |
| `[ ]` No panic path outside the tests | V1 | the panic lints denied in the workspace lint table, crate by crate from the lightest: Prose, the relay and the protocol first, then Sync, the canvas, Toile and Trame. A kept site carries an `expect` attribute with its reason (`ARCH-028`) |
| `[ ]` The suite checked adversarially | -- | `cargo-mutants` over the crates whose tests are generated, so a test that asserts nothing fails |
| `[ ]` The workspace lints | -- | `forbid(unsafe_code)` in every crate instead of two, and a workspace lint table in place of the default clippy set |
| `[ ]` Snapshot tests of the wire and the surfaces | -- | `insta` over the recording surface's operation sequences, the wire encodings and the PDF primitives |
| `[ ]` A coverage gate on added lines | -- | `cargo-llvm-cov` in diff mode, which turns a baseline nobody reads into a check |
| `[ ]` The browser-driven belt | -- | a served build driven by browser automation: the `ambre-app` boot, one command across the canvas and DOM seam, and the dialogs and menus the unit runner cannot drive |
| `[ ]` A real object-store round trip | -- | a presigned upload against a local minio in the database tier, which performs none today |

---

## 8. POST-V1 -- NOT SCHEDULED

### Foundations

| Story | Touches |
| :--- | :--- |
| `[ ]` Split-secret organization recovery | the escrow has a release path and no deposit path, and the quorum is clamped to one so the register advertises no guarantee the relay cannot keep. Threshold reconstruction, and a deposit, are what make the clamp liftable |
| `[ ]` Rendez-Vous | end-to-end encrypted video, and its signalling on an endpoint of its own, and a call started from a Murmure channel |
| `[ ]` Murmure bots and webhooks | bots holding their own keys, slash commands, interactive messages, and inbound webhooks through an encrypting bridge |
| `[ ]` Link previews | a preview of a web page fetched once by the sender and sealed in the message, and the name of a linked Ambre document for its members |
| `[ ]` Murmure export | a channel written to one `.ambre` file, through a container version that holds several documents |
| `[ ]` Cross-app rich paste | a paste from one app into another keeps its structure instead of falling back to plain text |
| `[ ]` Machine members | a program joins a document as a member, marked as a machine, with its declared host, and its removal rotates the key |
| `[ ]` Write-only inboxes | a program posts into a document through an asymmetric inbox it cannot read, and a member client merges each post once |
| `[ ]` Addressed machines | a program reads only the frames a user addresses to it, under a key derived from the document key, and answers through the inbox |
| `[ ]` Document macros | a user saves a signed macro in a document, approves each version, and runs it as one undoable change |
| `[ ]` Macro triggers | a macro runs after a kind of local edit, in the editing client only |
| `[ ]` Macro network access | a macro calls a domain its user approved, held by the browser's content policy |
| `[ ]` Macro marketplace | a publisher signs a macro package, an editor installs it into a document, and an update waits for a new installation |
| `[ ]` Organization trust anchor | an organization signing key held by its owners, admin certificates it signs, and a fingerprint members verify |
| `[ ]` Organization macro catalog | a catalog document of macros an organization approves, and the policy its members' clients apply. Needs the trust anchor |

### Prose

| Story | Touches |
| :--- | :--- |
| `[ ]` The later writing set | regular-expression find, bookmarks, version history, linked styles, pageless mode, line numbers, tables of figures, an index, a bibliography, section breaks and columns, protected zones, an equation editor |
| `[ ]` The spell checker | misspelled words underlined, with suggestions |

### Trame

| Story | Touches |
| :--- | :--- |
| `[ ]` Range protection | a range only its owner may edit, and the refusal the others see |
| `[ ]` Data tools, second pass | colour scales, data bars, top-N formats, and validation by range, date or pattern |
| `[ ]` Sheet PDF export | print a sheet to a vector PDF |
| `[ ]` Post-V1 surface | sparklines, charts, reference autocompletion, smooth scroll |
| `[ ]` Post-V1 interop | OpenDocument import, and the Google Sheets bridge |
| `[ ]` Version history | named snapshots a user can return to, in place of an undo stack lost on close |
| `[ ]` Engine optimization | the indexes and parallelism a million-cell sheet needs, measured against benchmarks |
| `[ ]` Recompute on mobile hardware | measure the recompute on a target device. Past 50 ms at P95 it must yield instead of blocking |
| `[ ]` The shared style dictionary | the style indirection the schema prescribes and no code writes. Gated on a benchmark: it needs a reclamation policy first |
| `[ ]` Named formulas | `LAMBDA` and `LET`, and a formula saved under a name that any cell of the workbook calls as a function |

### Toile

| Story | Touches |
| :--- | :--- |
| `[ ]` The facilitation layer | template boards, cards with tags, a shared timer, a voting session and the surface that casts a vote, a minimap |

---

## 9. HOW TO RESUME WITH NO PRIOR CONTEXT

Read section 1, then the epic marked *current core objective*, and work its open rows. Open a spec
only when you start the row that names it. `just spec-context-story "<NAME>"` prints what the rows
of one story need, and says when that story is cheaper read whole. No other file schedules anything.
