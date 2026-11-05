# AMBRE -- FEATURE: IMAGES

**Type:** Vertical (feature)
**Implementation:** design-only

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
| `PROSE-FEAT-205` | `INV` | The document MUST hold a picture as an address, never as its pixels. | this spec section 3 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-206` | `INV` | The address of a picture MUST be the content hash of the plaintext blob, so two copies of one picture resolve to one address. | this spec section 3 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-052` | `INV` | A picture MUST be a child of the document root: neither a table cell nor a list item MAY hold one. | this spec section 3; `ambre-spec-prose-schema.md section 4` | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-054` | `INV` | The decoded pictures MUST be held under a cap measured in decoded bytes, not in entries. | this spec section 3; `ambre-spec-budgets.md section 6` | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-207` | `INV` | A picture's resolution state MUST NOT be written into the document: it is a fact of the session alone. | this spec section 3 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-057` | `INV` | A picture that is absent, undecryptable or undecodable MUST NOT be fetched again without a reader's gesture. | this spec section 3 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-208` | `CONC` | Two peers resizing one picture at the same moment MUST converge on one display size, and both MUST hold that one. | this spec section 3 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-209` | `IO` | An alignment MUST come back from a round trip as it was written, including the two the layout does not honour. | this spec section 3 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-046` | `OBS` | A picture that failed for good MUST paint a mark distinct from the loading placeholder. | this spec section 4 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-145` | `OBS` | A picture that failed for good MUST offer the gesture that asks for it again. | this spec section 4 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-146` | `OBS` | A picture not yet resolved MUST paint a neutral placeholder carrying its alternative text. | this spec section 4 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Insert a picture

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-210` | `POST` | Inserting MUST write one picture block at the named place in the tree, holding the address and the display size the command carried. | this spec section 3 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-211` | `FAIL` | An insertion naming a parent that is not the document root MUST be refused, and MUST be reported to the author. | this spec section 3 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-212` | `FRAME` | Inserting a picture MUST NOT change the text of any block, the order of the blocks around it, or the decoded cache. | this spec section 3 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Resize a picture

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-058` | `FAIL` | A resize to a width or a height of zero MUST be refused. | this spec section 3 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-059` | `POST` | A display dimension past the maximum MUST be clamped to the maximum. | this spec section 3 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-213` | `POST` | The document MUST hold the clamped value, never the value the gesture asked for. | this spec section 3 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-214` | `POST` | With the aspect locked, the dimension that moved most against the stored size MUST be the one the other is derived from. | this spec section 3 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-215` | `POST` | The derived dimension MUST be taken from the decoded picture's own ratio, or from the stored size when nothing is decoded yet. | this spec section 3 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-216` | `FRAME` | A resize MUST NOT change the picture's address, its alignment, its alternative text or its caption. | this spec section 3 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.4 Align a picture

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-217` | `POST` | Aligning MUST store the alignment the command named, whether or not the layout honours it. | this spec section 3 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-218` | `FRAME` | Aligning MUST NOT change the picture's display size, its address, or the text around it. | this spec section 3 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.5 Put text on a picture

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-219` | `POST` | The text such a command carries MUST be stored on the field it names, on that block alone. | this spec section 3 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-220` | `FRAME` | Setting the alternative text or the caption MUST NOT change the picture's address, its display size or its alignment. | this spec section 3 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.6 Ask for a failed picture again

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-221` | `POST` | Asking again MUST put the entry back to the unasked state and issue the download once more. | this spec section 3 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-222` | `FRAME` | Asking again MUST NOT write to the document, and MUST NOT touch the resolution state of another address. | this spec section 3 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.7 Evict a decoded picture

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-223` | `POST` | Eviction MUST take the least recently painted entries first. | this spec section 3 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-286` | `POST` | Eviction MUST stop as soon as the total fits the cap. | this spec section 3 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-224` | `POST` | Eviction MUST leave one entry standing, even when that entry alone is over the cap. | this spec section 3 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-055` | `FRAME` | A picture whose transfer is in flight MUST NOT be evicted. | this spec section 3 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-056` | `FRAME` | Eviction MUST NOT touch the document: the address MUST stay on the block. | this spec section 3 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.8 Export a document holding pictures

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-IO-007` | `POST` | An export MUST take a picture's pixels from the local cache. | this spec section 3 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-008` | `POST` | A picture the cache does not hold MUST become a placeholder in the file and a line in the report. | this spec section 3 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-040` | `POST` | An export MUST finish without waiting on a transfer, whatever is missing. | this spec section 3 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-041` | `FRAME` | An export MUST NOT issue a fetch, and MUST NOT write to the document it reads. | this spec section 3 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.9 Import a file carrying a picture

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-053` | `POST` | An import carrying a picture in an illegal place MUST put it at the nearest legal ancestor, never drop it. | this spec section 3 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-225` | `POST` | The conversion report MUST carry every picture the import lifted. | this spec section 3 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-226` | `FRAME` | Lifting a picture MUST NOT change its address, its display size, its alternative text or its caption. | this spec section 3 | not built | unbuilt -- lands with the `Images` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** two rows. The PDF export of a hundred pages with pictures
  (`ambre-spec-budgets.md section 1`) counts the blob fetches as its second figure. The decoded
  cache of section 6 is the cap `PROSE-FEAT-054` sets.
* **Boundary.** the picture blobs are Prose's only outbound flow of user content that is not the
  document. The Logic thread makes no network call and holds no key (`PROSE-025`). The Sync worker
  seals every blob, and the document holds an address, not pixels.
* **Verification.** 38 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`. A
  reviewer holds `PROSE-FEAT-056`: eviction touching the document would be a write in a per-session
  cache. `PROSE-FEAT-054`, `-055`, `-223` and `-224` are reached from the cache alone, and Toile
  holds the same pair (`TOILE-IO-007`). The eleven frame conditions each name a field a resize or
  an eviction may not reach, and no assertion covers one.

---

## 2. WHAT EACH USER SEES

The address is shared and the pixels are not: each peer fetches for itself. No test resolves yet:
the last column names the row each row rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| inserts a picture | watches | one address, one display size | Alice sees her picture; Bob sees a placeholder until his own fetch lands | none -- `PROSE-FEAT-205`, `PROSE-UI-146` |
| resizes a picture | resizes the same picture at the same moment | one size, the later of the two | the same size on both screens | none -- `PROSE-FEAT-208` |
| scrolls past many pictures and back | -- | every address, unchanged | the pictures again, refetched where the cache dropped them | none -- `PROSE-FEAT-056` |
| opens a document whose picture the server no longer has | watches | the address | a broken picture and an offer to try again, on both screens | none -- `PROSE-UI-046`, `PROSE-UI-145` |
| exports to PDF before a picture has arrived | -- | the address | a placeholder in the file, and a line in the report | none -- `PROSE-IO-008` |
| pastes a picture inside a table cell | -- | the picture, above or below the table | the picture out of the cell, and a line in the report | none -- `PROSE-FEAT-053` |

---

## 3. MODEL

**Containers.** None of its own. A picture is one block type of the movable tree. Its address,
its alternative text, its display size, its alignment and its caption are that node's attributes,
declared in `ambre-spec-prose-schema.md section 3`. There is no media register. The address is the
content hash of the plaintext blob and the key into the media flow, so no second table joins the two
(`ambre-spec-prose-schema.md section 6`).

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

`Left` and `Right` are accepted, kept through a round trip and laid out as `Block`. Text
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
back to `Pending`, and the download is issued once more. Coming back online re-drives the entries
that never resolved, and never a terminal one (`PROSE-FEAT-057`).

**The cache.** Decoded pictures are held in the Logic thread under a cap measured in decoded bytes,
not in entries. It is the bytes that spend the client-memory pillar. On each insert, the
least recently *painted* entries leave until the total fits (`PROSE-FEAT-054`). An entry whose
transfer is in flight is never chosen (`PROSE-FEAT-055`). A single picture larger than the whole
cap is kept, because it is the only way to show it at all.

Eviction frees a decoding, never a fact: the address stays on the block (`PROSE-FEAT-056`), so a
picture scrolled back into view is simply `Pending` again. The cap is a row of
`ambre-spec-budgets.md section 6`.

**The commands** (`ambre_prose::message`, declared in `features/ambre-spec-prose-engine.md section 3`):

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
   driver. Derive the other from the decoded picture's own ratio. When the blob has not been
   decoded yet, derive it from the stored size instead.

**Where a picture may sit.** The root, and only the root: not in a table cell, not in a list item
(`PROSE-FEAT-052`). The rule is the nesting table of `ambre-spec-prose-schema.md section 4`, and
the guard is the one `features/ambre-spec-prose-tables.md section 3` states, shared by both paths.
The import path uses the same predicate and lifts an illegal picture to the nearest legal ancestor
rather than refusing the file. The conversion report carries the lift
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
that failed for good paints a distinct broken mark and the retry affordance. No picture stays on
the loading placeholder for ever (`PROSE-UI-046`).

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
| An export placeholder never blocks the export | fetching the missing blobs inside the export | it makes an offline export hang instead of finishing. An export is a gesture with an end, and waiting on a network fetch inside it turns a five-second action into an unbounded one; the report is what carries the loss (`PROSE-IO-008`) |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing puts a picture into the document in the first place.** Every command above takes an
  address that already exists, and no command carries a file the reader chose, computes its hash or
  uploads it. `features/ambre-spec-prose-import-export.md section 3` reads the local cache and
  nothing fills it. No Prose rule says who uploads a blob or what the reader sees while it
  runs.
* **Nothing says whether a paste follows the refusal or the lift.** Section 2 pastes a picture
  into a table cell and expects it beside the table. `PROSE-FEAT-053` rules on an import and
  `PROSE-FEAT-211` refuses an insertion; a paste is named by neither, and the two answers differ.
* **Two blocks holding one address are not ruled on.** The resolution state is the address's and
  `RetryImageLoad` names a block. Nothing says whether one retry, one failure mark and one
  eviction reach the second block or leave it as it was.
* **The retry has no answer for the reader.** `RetryImageLoad` asks again and nothing says what the
  view shows while it runs, nor what it shows when the second attempt fails the same way.
* **Nothing bounds the source picture.** The display size is clamped and the decoded cache is
  capped. The blob itself is not, so a picture too large to decode fails as
  `CorruptImage` rather than being refused when it is chosen. Toile refuses one at import
  (`TOILE-IO-009`) and Prose states no equivalent.
* **The maximum display dimension is a figure with no home.** It is stated here as a constant of
  this feature and it is not a row of `ambre-spec-budgets.md`, unlike the cache cap
  beside it.
* **Text wrapping has no design**, and the two alignments that need it are accepted and stored. A
  document imported with wrapped pictures keeps an intent nothing honours.
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
