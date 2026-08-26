# AMBRE -- FEATURE: IMAGES

**Type:** Vertical (feature)
**Implementation:** design-only

> A picture in the document. The document holds the picture's address and its display size; the
> pixels travel apart, through the shared media flow, and the server that stores them never sees
> them.

The transfer and the encryption are Foundation's (`ambre-spec-message-bus.md`), and
this document restates neither. It owns the block, the resolution of an address into pixels, the
cache that holds them and what an export does when they are missing.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `PROSE-IO-007` | The document MUST hold a picture as an address, and an export MUST resolve that address from the local cache. | this spec section 3 | not built -- lands with the "Images" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-008` | An export whose pictures are not cached MUST emit a placeholder and report it; it MUST NOT block and MUST NOT fail in silence. | this spec section 3 | not built -- lands with the "Images" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-052` | A picture MUST be a child of the document root: a picture inside a table cell or a list item MUST be refused. | this spec section 3; `ambre-spec-prose-schema.md section 4` | not built -- lands with the "Images" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-053` | An import carrying a picture in an illegal place MUST lift it to the nearest legal place and report it; it MUST NOT drop it. | this spec section 3 | not built -- lands with the "Images" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-054` | The decoded pictures MUST be held under a cap measured in decoded bytes, and the least recently painted MUST leave first. | this spec section 3; `ambre-spec-budgets.md section 6` | not built -- lands with the "Images" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-055` | A picture whose transfer is in flight MUST NOT be evicted. | this spec section 3 | not built -- lands with the "Images" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-056` | Eviction MUST NOT touch the document: the address MUST stay on the block. | this spec section 3 | not built -- lands with the "Images" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-057` | A picture that is absent, undecryptable or undecodable MUST NOT be fetched again on its own; only the reader MUST be able to ask again. | this spec section 3 | not built -- lands with the "Images" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-058` | A resize to a width or a height of zero MUST be refused. | this spec section 3 | not built -- lands with the "Images" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-059` | A display dimension past the maximum MUST be clamped, and the clamped value MUST be what the document holds. | this spec section 3 | not built -- lands with the "Images" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-046` | A picture that failed for good MUST show that it failed and offer to try again; it MUST NOT stay on the loading placeholder. | this spec section 4 | not built -- lands with the "Images" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the PDF export of a hundred pages with pictures, one row of
  `ambre-spec-budgets.md section 1`, whose second figure is the blob fetches this document
  owns; and the decoded cache, one row of `ambre-spec-budgets.md section 6`, which is the cap
  `PROSE-FEAT-054` states.
* **Boundary.** this document owns Prose's only outbound flow of user content that is not the
  document itself: the picture blobs. The Logic thread makes no network call and holds no key
  (`PROSE-025`), the Sync worker seals every blob before it leaves, and the document carries an
  address rather than pixels, so nothing here reaches a service that could read one.
* **Verification.** 11 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`.
  `PROSE-FEAT-056` is the row a reviewer can hold without a test, because eviction touching the
  document would be a write in a cache the crate keeps per session. `PROSE-FEAT-054` and `-055` are
  the two a test reaches with no network, from the cache alone, and Toile holds the same pair with
  `TOILE-IO-007`.

---

## 2. WHAT EACH USER SEES

The address is shared and the pixels are not: each peer fetches for itself. No test resolves yet:
the last column names the row each row rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| inserts a picture | watches | one address, one display size | Alice sees her picture; Bob sees a placeholder until his own fetch lands | none -- `PROSE-IO-007` |
| resizes a picture | resizes the same picture at the same moment | one size, the later of the two | the same size on both screens | none -- `PROSE-FEAT-059` |
| scrolls past many pictures and back | -- | every address, unchanged | the pictures again, refetched where the cache dropped them | none -- `PROSE-FEAT-056` |
| opens a document whose picture the server no longer has | watches | the address | a broken picture and an offer to try again, on both screens | none -- `PROSE-UI-046` |
| exports to PDF before a picture has arrived | -- | the address | a placeholder in the file, and a line in the report | none -- `PROSE-IO-008` |
| pastes a picture inside a table cell | -- | the picture, above or below the table | the picture out of the cell, and a line in the report | none -- `PROSE-FEAT-053` |

---

## 3. MODEL

**Containers.** None of its own. A picture is one block type of the movable tree, and its address,
its alternative text, its display size, its alignment and its caption are that node's attributes,
declared in `ambre-spec-prose-schema.md section 3`. There is no media register: the address is the
content hash of the plaintext blob, and it is also the key into the media flow, which is why no
second table joins the two (`ambre-spec-prose-schema.md section 6`).

**The alignments:**

```rust
#[repr(u8)]
pub enum ImageAlign {
    /// A block of its own, the default.
    Block  = 0,
    /// Left, text wrapped to the right. Not built: laid out as `Block`.
    Left   = 1,
    /// Right, text wrapped to the left. Not built: laid out as `Block`.
    Right  = 2,
    /// Centred, no wrapping.
    Center = 3,
}
```

`Left` and `Right` are accepted, kept through a round trip and laid out as `Block`, because text
wrapping needs line breaking with exclusion zones, which the shaper does not offer
(`features/ambre-spec-prose-pagination.md section 5`).

**How an address becomes pixels.** The Logic thread asks, the Sync worker fetches and decrypts, and
the answer comes back as bytes (`ambre-spec-message-bus.md`). The Logic thread makes
no network call and names no key. Each address the document holds carries a resolution state, kept
per session and never in the document:

```rust
pub enum MediaState {
    /// Nothing asked for yet.
    Pending,
    /// Asked for, waiting on the Sync worker.
    Loading,
    /// Decrypted and cached, ready to paint.
    Ready,
    /// Failed for good. Not asked for again on its own (`PROSE-FEAT-057`).
    Failed(MediaFailure),
}

pub enum MediaFailure {
    /// The server does not have it.
    Missing,
    /// The bytes did not decrypt.
    DecryptFailed,
    /// The bytes decrypted and are not a picture this build can decode.
    CorruptImage,
    /// The network failed past the retry budget the Sync worker owns.
    NetworkExhausted,
}
```

The first three are terminal on the first answer, because asking again cannot change them. The
fourth is what the Sync worker's own bounded retry ends on. A reader who asks again puts the entry
back to `Pending` and the download is issued once more; coming back online re-drives the entries
that never resolved, and never a terminal one (`PROSE-FEAT-057`).

**The cache.** Decoded pictures are held in the Logic thread under a cap measured in decoded bytes,
not in entries, because it is the bytes that spend the client-memory pillar. On each insert, the
least recently *painted* entries leave until the total fits (`PROSE-FEAT-054`). An entry whose
transfer is in flight is never chosen (`PROSE-FEAT-055`). A single picture larger than the whole
cap is kept, because it is the only way to show it at all.

Eviction frees a decoding, never a fact: the address stays on the block (`PROSE-FEAT-056`), so a
picture scrolled back into view is simply `Pending` again. The cap is a row of
`ambre-spec-budgets.md section 6`.

**The commands** (`ambre-bus`, declared in `features/ambre-spec-prose-engine.md section 3`):

| Shape | Behaviour |
|---|---|
| `InsertImage { parent_id, after_id, content_hash, alt, width_px, height_px }` | a picture block, at a place in the tree |
| `ResizeImage { block_id, width_px, height_px, lock_aspect }` | validates, then writes the display size |
| `SetImageAlign { block_id, align }` | one of the four alignments |
| `SetImageAlt { block_id, alt }` | the alternative text the screen reader announces |
| `SetImageCaption { block_id, caption }` | the caption drawn under the picture |
| `RetryImageLoad { block_id }` | puts a failed entry back to `Pending` and asks again |

**What a resize does, in order:**

1. refuse a width or a height of zero: a picture of no area is not a state (`PROSE-FEAT-058`).
2. clamp each dimension to the maximum, and store what the clamp produced (`PROSE-FEAT-059`). Past
   it the layout cost is unbounded and no page can show the picture at that size.
3. when the aspect is locked, take the dimension that moved most against the stored size as the
   driver, and derive the other from the decoded picture's own ratio. When the blob has not been
   decoded yet, derive it from the stored size instead.

**Where a picture may sit.** The root, and only the root: not in a table cell, not in a list item
(`PROSE-FEAT-052`). The rule is the nesting table of `ambre-spec-prose-schema.md section 4`, and
the guard is the one `features/ambre-spec-prose-tables.md section 3` states, shared by both paths.
The import path uses the same predicate and lifts an illegal picture to the nearest legal ancestor
rather than refusing the file, and the conversion report carries the lift
(`PROSE-FEAT-053`).

**What an export does.** An export reads the bytes from the cache, never from the network
(`PROSE-IO-007`): the fetch happens before the export runs, through the ordinary media flow. A
picture still missing when the export runs becomes a placeholder and a line in the report, and the
export finishes (`PROSE-IO-008`). The mapping into each format is
`features/ambre-spec-prose-import-export.md section 3`.

**Where a picture is measured.** A picture is never split across pages
(`features/ambre-spec-prose-pagination.md`, `PROSE-014`), and it is measured as its box plus its
caption.

**Code:** none. `crates/ambre-prose/src/lib.rs` is seven lines.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the slash command, or the insert menu | a picture at the caret, once its blob has been stored |
| a drag on a corner handle | resizes with the aspect locked, on release |
| a drag on an edge handle | resizes that dimension alone |
| the alignment entries of the picture menu | one of the four alignments |
| the alternative-text entry | the text a screen reader announces |
| the caption entry | the caption under the picture |
| a click on the retry affordance | asks for the blob again |

A picture not yet resolved paints a neutral placeholder carrying its alternative text. A picture
that failed for good paints a distinct broken mark and the retry affordance, so no picture stays on
the loading placeholder for ever (`PROSE-UI-046`).

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The document holds an address, never pixels | a document is synchronized on every keystroke and a picture is megabytes: embedding it would put those megabytes on the delta path, in the history, and in every peer's copy of the document whether they look at the picture or not. An address is 32 bytes and deduplicates two copies of one picture for free | embedding the bytes in the block, which makes the document grow with the pictures and the history grow with the edits to them |
| The address is the content hash of the plaintext | two identical pictures then resolve to one blob without anything having to notice they are the same, and the address is verifiable against what came back | a minted identifier, which stores the same picture twice and lets an address point at bytes that are not the ones it named |
| The Logic thread never fetches | the fetch needs the presigned URL and the document key, and the boundary that keeps those in the Sync worker is the one `PROSE-025` states. Asking through a command keeps the Logic thread a pure function of what it is given | fetching from the Logic thread, which puts network and crypto into the crate the whole application's rules say holds neither |
| The cache is capped by decoded bytes | a decoded picture is its pixels, and an entry count says nothing about them: ten thumbnails and ten photographs are the same count and two orders of magnitude apart. The pillar is a memory figure, so the cap has to be one | a cap on the number of entries, which is a memory figure only when every picture is the same size |
| Eviction frees the decoding and never the document | the address is the fact and the pixels are a view of it, so dropping the pixels costs a refetch and nothing else. Writing to the document to free memory would put a local memory decision on the synchronization path | clearing the address, which turns a local eviction into a remote deletion of the picture |
| An entry in flight is never evicted | its bytes have not arrived, so evicting it frees nothing and cancels work already paid for. It is also the entry the reader is most likely waiting on (`PROSE-FEAT-055`) | evicting by recency alone, which drops the picture that is loading in favour of one already on screen |
| A terminal failure is not retried on its own | a picture the server does not have, or that does not decrypt, answers the same on every attempt: retrying spends requests to learn nothing, and hides the failure behind a spinner. Asking the reader turns it into one visible state with one gesture out of it | retrying on a timer, which never ends for a blob that was garbage-collected and never tells the reader why |
| A failure shows a distinct mark, not the loading placeholder | the two mean opposite things -- one is waiting, the other is over -- and one glyph for both makes every failure look like a slow network for ever (`PROSE-UI-046`) | keeping the neutral placeholder, which is what a reader stares at with no way to know it will never resolve |
| A zero dimension is refused and a large one is clamped | zero has no correct interpretation, so refusing is the honest answer. A number too large has one, the largest the layout can carry, so clamping keeps the gesture working instead of failing it | clamping the zero too, which invents a size the author never asked for; or refusing the large one, which fails a drag rather than stopping it |
| The clamped value is what is stored | the document then says what the reader sees, and every later reader agrees with it. Storing the request and clamping at paint time makes two builds with different maxima disagree about one document | storing the requested size and clamping when painting, which leaves the document holding a size nothing renders |
| Wrapped text is laid out as a block | the shaper offers no exclusion zones, so a wrap would be an approximation that puts glyphs under the picture. A block is visibly not the wrap the author asked for, and it is never wrong | shipping an approximation, which is wrong in a way the author has to fix by hand on every page |
| A picture sits at the root alone | a picture inside a cell or a list item multiplies the layout cases -- a cell that must grow, an ordinal beside a box -- for a document shape the MVP does not need. Refusing it once keeps every measurement rule about text | allowing it everywhere, which makes the pagination cases and the merge cases grow together |
| An illegal import is lifted, never dropped | a file the reader opens is not a command they typed: refusing part of it silently loses their content. Lifting keeps the picture and the report says where it went (`PROSE-FEAT-053`) | dropping the node with a warning, which loses content the author still has in the source file and cannot see is gone |
| An export placeholder never blocks the export | an export is a gesture with an end, and waiting on a network fetch inside it turns a five-second action into an unbounded one. The report is what carries the loss (`PROSE-IO-008`) | fetching the missing blobs inside the export, which makes an offline export hang instead of finishing |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing puts a picture into the document in the first place.** Every command above takes an
  address that already exists, and no command carries a file the reader chose, computes its hash or
  uploads it. `features/ambre-spec-prose-import-export.md section 3` reads the local cache and
  nothing fills it, and no Prose rule says who uploads a blob or what the reader sees while it
  runs.
* **The retry has no answer for the reader.** `RetryImageLoad` asks again and nothing says what the
  view shows while it runs, nor what it shows when the second attempt fails the same way.
* **Nothing bounds the source picture.** The display size is clamped and the decoded cache is
  capped; the blob itself is not, so a picture too large to decode fails as
  `CorruptImage` rather than being refused when it is chosen. Toile refuses one at import
  (`TOILE-IO-009`) and Prose states no equivalent.
* **The maximum display dimension is a figure with no home.** It is stated here as a constant of
  this feature and it is not a row of `ambre-spec-budgets.md`, unlike the cache cap
  beside it.
* **Text wrapping has no design**, and the two alignments that need it are accepted and stored, so
  a document imported with wrapped pictures keeps an intent nothing honours.
* **An equation is the second block whose content is not text, and it has no home.** The deferred
  design stores a formula as a mark on a range and as a block type, both holding a source string,
  and renders it on the Logic thread into a drawing the page paints the way it paints a picture.
  Neither the mark nor the block type is in `ambre-spec-prose-schema.md section 3`, no event
  carries the rendering, and the renderer itself is named as a choice nobody has made. It is placed
  here because the path from a Logic-thread product to a painted block is this feature's, and the
  Story is "The later writing set" (`ambre-roadmap.md`).
* **A Word table holding a picture loses the cell it was in.** The pipeline lifts it beside the
  table and reports the lift (`PROSE-IO-026`, `PROSE-FEAT-053`), which is settled. What is not
  settled is the way back: an export writes the lifted picture where the document now holds it, so
  a round trip through Prose moves the picture out of the cell permanently.
