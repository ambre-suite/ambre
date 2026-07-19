# AMBRE -- SPEC: PROSE / FEATURES
**Type:** Vertical (Feature/Widget)
**Status:** Draft

> User-facing features of Prose implemented on top of the CRDT engine (`prose-engine`).
> Scope: inline and block styles, lists, headings, tables, table of contents, comments,
> track changes, undo/redo, footnotes, headers/footers, find & replace,
> and spell checker. Functional target: iso Google Docs across all these
> features. Post-MVP features are specified in section O.

---

## 1. OBJECTIVES & CONSTRAINTS

**Scope:**

This component handles:
- Applying and resolving inline and paragraph styles.
- Structural management of lists (ordinals, nesting), headings, and tables.
- Generating and updating the table of contents (TOC).
- The comments system (threads anchored on text ranges).
- Track changes: insert/delete suggestions, accept/reject.
- Local undo/redo (scoped per peer).
- Footnotes and cross references.
- Headers and footers.
- Find & replace.
- Word count.
- Spell checker (post-MVP).
- Post-MVP features listed in section O.

This component delegates:
- CRDT persistence and `LoroDoc` mutations -> `prose-engine` (`ProseState`).
- Visual rendering of features (comment highlights, revision bars) -> `prose-ui`.
- Import/export of corresponding structures -> `prose-interop`.

**Dependencies:**

| Crate | Justification |
|-------|---------------|
| `loro` | `LoroText::mark()`, `LoroMap` for comment threads, `UndoManager`. |
| `ambre-core` | `Uuid` for stable comment, suggestion, footnote IDs (via `ambre-bus` re-export). |
| `ambre-bus` | Typed IPC types for features, carried by `ProseCommand`/`ProseEvent` (-> `AppCommand::Prose`/`AppEvent::Prose`). |
| `unicode-segmentation` | Grapheme counting for Loro offsets (`chars()` Rust ≠ Unicode graphemes). |

> **No `serde` on feature types.** They travel **typed, by move** on `flume`, inside `ProseCommand`/`ProseEvent` (shared memory). `serde`/`postcard` are only used for **network bytes** (Loro CRDT payloads and `Ephemeral`) -- see `ambre-spec-prose-logic.md`.

Post-MVP dependencies:

| Crate | Justification |
|-------|---------------|
| `zspell` | Pure Rust spell checker, WASM-compatible, Hunspell dictionaries. |

**Invariants:**

- A `CommentId` or `SuggestionId` (`Uuid`) is stable for the entire document lifetime: it does not change after partial accept/reject or remote delta import.
- List ordinals are **computed on demand** (not stored in the CRDT): they are derived from the `MovableTree` at read time. Storing ordinals in the CRDT would introduce unnecessary CRDT conflicts on any `ListItem` insertion.
- The TOC is **derived** from the `MovableTree` (not duplicated): no copy of headings is stored in `"meta"` or elsewhere.
- Rejected suggestions (track changes reject) are **removed from the CRDT** -- they leave no trace in the final document.
- No feature short-circuits `ProseState::dispatch()` -- all mutations go through typed `ProseCommand`.

---

## 2. TECHNICAL DESIGN & STATE

### A. Inline Styles

Inline styles are `LoroText` marks defined in `prose-engine`. The features add the
**resolution logic**: when multiple marks overlap or a block style defines a default
value, the effective value is computed via cascade.

#### Complete Inline Marks

```rust
// ambre-prose/src/mark.rs
pub const MARK_BOLD:          &str = "b";
pub const MARK_ITALIC:        &str = "i";
pub const MARK_UNDERLINE:     &str = "u";
pub const MARK_STRIKETHROUGH: &str = "s";
pub const MARK_SUPERSCRIPT:   &str = "sup";
pub const MARK_SUBSCRIPT:     &str = "sub";
pub const MARK_CODE:          &str = "c";    // inline code
pub const MARK_LINK:          &str = "a";    // value = URL (String)
pub const MARK_COLOR:         &str = "fg";   // value = "#rrggbb"
pub const MARK_BG:            &str = "bg";   // value = "#rrggbb"
pub const MARK_FONT_SIZE:     &str = "fs";   // value = f32 (pt)
pub const MARK_FONT_FAMILY:   &str = "ff";   // value = String (font name)
pub const MARK_COMMENT:       &str = "cm";   // value = Uuid (-> "annots")
pub const MARK_SUGGEST_INS:   &str = "si";   // value = Uuid (suggestion)
pub const MARK_SUGGEST_DEL:   &str = "sd";   // value = Uuid (suggestion)
pub const MARK_FOOTNOTE:      &str = "fn";   // value = Uuid (-> "annots")
pub const MARK_CROSS_REF:     &str = "ref";  // value = "{target_block_id}:{kind}"
```

**Per-mark expansion policy:**

| Mark | `ExpandType` | Justification |
|------|-------------|---------------|
| `b`, `i`, `u`, `s`, `sup`, `sub`, `c`, `fg`, `bg`, `fs`, `ff` | `After` | Continue style when typing at the end of the span. Google Docs behavior. |
| `a` (link) | `None` | An adjacent insertion to a link should not become part of it. |
| `cm`, `si`, `sd`, `fn`, `ref` | `None` | Annotations do not auto-extend. |

**Note (P-81):** `superscript` and `subscript` are two INDEPENDENT boolean marks. Both may
legally COEXIST on the same range in the CRDT -- neither write clears the other, so no ordering
rule is required. "Superscript wins" is a STATIC render-time priority applied by `resolve_style`
(fixed priority `superscript` > `subscript`): when both are active it renders as superscript and
ignores subscript. Because the priority is fixed and evaluated at read time -- not a write-order
tie-break -- the resolved style is deterministic and CRDT-stable regardless of the order in which
concurrent `superscript`/`subscript` marks arrive.

#### Style Cascade

```
1. Explicit inline mark on the LoroText      (highest priority)
2. Paragraph style (block's style_id)        -> lookup in "styles" LoroMap
3. Built-in BlockType style                  (heading-1..6, code, quote, body)
4. Document global default                   (stored in "meta.defaults")
```

```rust
// ambre-prose/src/features/style.rs

#[derive(Debug, Clone, Default)]
pub struct ResolvedStyle {
    pub bold:          bool,
    pub italic:        bool,
    pub underline:     bool,
    pub strikethrough: bool,
    pub superscript:   bool,
    pub subscript:     bool,
    pub code_inline:   bool,
    pub color:         Option<u32>,
    pub background:    Option<u32>,
    pub font_size:     f32,          // pt
    pub font_family:   Option<String>,
    pub link:          Option<String>,
}
```

#### Paragraph Styles

```rust
#[derive(Debug, Clone)]
pub struct ParagraphStyle {
    pub align:          TextAlign,
    pub indent_first:   f32,
    pub indent_left:    f32,
    pub indent_right:   f32,
    pub space_before:   f32,
    pub space_after:    f32,
    pub line_height:    LineHeight,
    pub keep_together:  bool,
    pub keep_with_next: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum TextAlign { #[default] Left, Center, Right, Justify }

#[derive(Debug, Clone, Copy)]
pub enum LineHeight { Multiple(f32), Exact(f32), AtLeast(f32) }
```

**Note (P-83):** paragraph style is stored as a per-property `LoroMap` (one key per
`ParagraphStyle` field), not a single opaque blob -- canonical storage lives in Logic §C.
`SetParagraphStyle` writes only the keys that changed, so two concurrent edits to DIFFERENT
properties (e.g. `align` vs `space_after`) merge cleanly via Loro per-key LWW; concurrent edits
to the SAME property remain LWW.

Commands (aligned with `ProseCommand` -- source of truth in `prose-engine`):

```rust
SetParagraphStyle { block_id: TreeID, style: ParagraphStyle },
ApplyMark         { block_id: TreeID, range: (u32, u32), key: String, value: Option<String> },
RemoveMark        { block_id: TreeID, range: (u32, u32), key: String },
ClearInlineMarks  { block_id: TreeID, range: (u32, u32) },
```

**`ApplyMark` value parsing and validation.** `value: Option<String>` is a wire-level
carrier: each mark key defines its own value grammar, validated by the dispatcher **before**
the CRDT is touched. A malformed value is rejected whole (the mark is not applied) and reported
via `ProseEvent::CommandRejected { reason: MarkValueError }` -- the CRDT is never left holding an
unparseable value.

```rust
// ambre-prose/src/features/style.rs

#[derive(Debug, Clone)]
pub enum MarkValueError {
    /// Color mark ("fg"/"bg") whose value is not exactly "#rrggbb" or "#rrggbbaa".
    BadColor(String),
    /// Font size ("fs") not a finite f32, or outside the accepted 1.0..=1638.0 pt range.
    BadFontSize(String),
    /// A value was required for this key but None was supplied (or vice versa).
    ArityMismatch { key: String },
}

/// Parse a color value. Accepts "#rrggbb" (opaque) and "#rrggbbaa" (explicit alpha).
/// Returns a packed 0xRRGGBBAA u32 -- alpha defaults to 0xFF when the short form is used,
/// so a round-trip of an opaque color is stable ("#3366cc" -> 0x3366ccFF -> "#3366cc").
fn parse_color(raw: &str) -> Result<u32, MarkValueError> {
    let hex = raw.strip_prefix('#').ok_or_else(|| MarkValueError::BadColor(raw.into()))?;
    let (rgb, a) = match hex.len() {
        6 => (u32::from_str_radix(hex, 16), 0xFFu32),
        8 => (
            u32::from_str_radix(&hex[..6], 16),
            u8::from_str_radix(&hex[6..], 16)
                .map_err(|_| MarkValueError::BadColor(raw.into()))? as u32,
        ),
        _ => return Err(MarkValueError::BadColor(raw.into())),
    };
    let rgb = rgb.map_err(|_| MarkValueError::BadColor(raw.into()))?;
    Ok((rgb << 8) | a)
}

/// Parse a font size in points. Rejects NaN/inf and clamps the accepted domain to
/// 1.0..=1638.0 pt (the OOXML half-point ceiling), matching what interop can round-trip.
fn parse_font_size(raw: &str) -> Result<f32, MarkValueError> {
    let v: f32 = raw.parse().map_err(|_| MarkValueError::BadFontSize(raw.into()))?;
    if v.is_finite() && (1.0..=1638.0).contains(&v) {
        Ok(v)
    } else {
        Err(MarkValueError::BadFontSize(raw.into()))
    }
}
```

**Color alpha round-trip (P-58).** `ResolvedStyle::color`/`background` are `Option<u32>` packed
as `0xRRGGBBAA`. The stored CRDT string is the **canonical** form produced by re-serialization:
`0x??????FF` serializes back to the 6-digit `"#rrggbb"`, any other alpha to the 8-digit
`"#rrggbbaa"`. This guarantees import -> resolve -> export is lossless and idempotent.

**`ClearInlineMarks` scope (P-56).** `ClearInlineMarks` strips **formatting** marks only and
**preserves annotation** marks. Concretely, it removes exactly:
`b, i, u, s, sup, sub, c, a, fg, bg, fs, ff`. It never removes the annotation set
`{cm, si, sd, fn, ref}` (comments, suggestions, footnotes, cross references), nor the dynamic
field marks `{f:pn, f:pc, f:dt, f:ti}`: those carry document-model identity (a `Uuid` or a field
binding) that must survive a formatting reset. Removing a comment/suggestion/footnote is done
through its own dedicated command, never as a side effect of clearing styles.

---

### B. Lists

Lists are `BlockType::ListItem` nodes in the `MovableTree`. Nesting is modeled by the
tree's parent/child hierarchy. There is no `List` container node.

```rust
// ambre-prose/src/features/list.rs

#[derive(Debug, Clone)]
pub enum ListStyle {
    Bullet,
    Ordered(OrderedStyle),
    Todo(bool),   // bool = checked
}

#[derive(Debug, Clone, Copy)]
pub enum OrderedStyle {
    Decimal,
    LowerAlpha,
    UpperAlpha,
    LowerRoman,
    UpperRoman,
}

/// Ordinal (1-based) of an ordered list item among its contiguous run of siblings.
///
/// **Restart-vs-continue rule.** Numbering is scoped to a *contiguous run* of
/// `ListItem(Ordered)` siblings under the same parent that share the same `OrderedStyle`.
/// A run is broken by ANY intervening sibling that is not an ordered item of the same style:
/// a non-list block, a bullet/todo item, or an ordered item with a different `OrderedStyle`.
/// When an ordered list is interrupted and later resumes, the resumed run **restarts at 1**
/// (Google Docs behavior). A future "continue previous numbering" affordance would be an
/// explicit per-item override attribute; it is out of scope here and not inferred.
pub fn compute_ordinal(tree: &LoroMovableTree, item_id: &TreeID) -> u32 {
    let parent = tree.parent(item_id);
    let siblings = tree.children(parent.as_ref()); // document order
    let target_style = ordered_style_of(tree, item_id)
        .expect("compute_ordinal called on a non-ordered item");

    let mut ordinal = 0u32;
    for sib in &siblings {
        match ordered_style_of(tree, sib) {
            // Same style: extend the current run.
            Some(style) if style == target_style => ordinal += 1,
            // Any other node type or a different ordered style breaks the run.
            _ => ordinal = 0,
        }
        if sib == item_id {
            // `ordinal` was just incremented for `item_id` itself.
            return ordinal;
        }
    }
    unreachable!("item_id must be a child of its own parent")
}

/// Engine accessor (impl: `ambre-prose/src/features/list.rs`).
/// Reads BlockType + "a".list_style from the node's LoroMap and returns the
/// `OrderedStyle` iff `id` is an ordered `ListItem`, else `None` (bullet/todo items
/// and non-list blocks yield `None` and thus break a run).
fn ordered_style_of(tree: &LoroMovableTree, id: &TreeID) -> Option<OrderedStyle>;
```

Commands:

```rust
InsertListItem { parent_id: Option<TreeID>, after_id: Option<TreeID>, style: ListStyle },
SetListStyle   { block_id: TreeID, style: ListStyle },
IndentList     { block_id: TreeID },
UnindentList   { block_id: TreeID },
```

**`UnindentList` at top level (P-60).** `UnindentList` reparents the item to its
grandparent (moving it one level toward the root) and re-inserts it immediately after its
former parent's subtree. When the item is already a **top-level** list item (its parent is the
document root), it cannot rise further: the command instead **converts the item to a plain
`Paragraph`** (drops `ListStyle`), matching Google Docs where a final outdent exits the list.
`IndentList` on the first item of a run is a no-op (there is no preceding sibling to become its
parent), reported as `CommandRejected { reason: CannotIndentFirstItem }`.

---

### C. Headings

Headings are `BlockType::Heading` nodes with `"a": {level: u8}` (1..=6).

```rust
// ambre-prose/src/features/heading.rs

/// Section number of a heading as a level-depth vector, e.g. `[1, 2, 1]` for "1.2.1".
///
/// **Level validation/clamping (P-61).** Headings carry `"a": {level: u8}` documented as
/// `1..=6`, but an imported (docx/foreign) document may contain `level == 0` or `> 6`.
/// Rather than underflow `counter[level - 1]`, every level is **clamped into `1..=6`**
/// at read time: `0 -> 1`, anything `> 6 -> 6`. Clamping is applied only for numbering
/// (the stored attribute is left untouched, so a lossless re-export keeps the original level).
pub fn compute_section_number(
    tree: &LoroMovableTree,
    heading_id: &TreeID,
) -> Vec<u32> {
    // Per-level counters, index 0 == H1 ... index 5 == H6.
    let mut counter = [0u32; 6];
    // DFS pre-order over the whole tree; stop when we reach the requested heading.
    for node in tree.dfs_pre_order() {
        let Some(raw_level) = heading_level_of(tree, &node) else {
            continue; // not a heading -- headings-only numbering, other blocks ignored
        };
        // Clamp foreign/out-of-range levels into 1..=6 (never index out of bounds).
        let level = raw_level.clamp(1, 6) as usize;
        counter[level - 1] += 1;
        // Reset all deeper levels so the next descent restarts at .1
        for c in counter.iter_mut().skip(level) {
            *c = 0;
        }
        if node == *heading_id {
            // Emit the significant prefix counter[0..level].
            return counter[..level].to_vec();
        }
    }
    Vec::new() // heading_id not found (deleted concurrently) -> empty number
}

/// Engine accessor (impl: `ambre-prose/src/features/heading.rs`).
/// Reads BlockType::Heading + "a".level from the node's LoroMap and returns the raw
/// (unclamped) heading level iff `id` is a `BlockType::Heading`, else `None`.
fn heading_level_of(tree: &LoroMovableTree, id: &TreeID) -> Option<u8>;
```

---

### D. Tables

CRDT structure:

```
Table (BlockType::Table)
└── TableRow (BlockType::TableRow)  ×n
    └── TableCell (BlockType::TableCell)  ×m
        └── LoroText (+ inline marks)
            + inline blocks: Paragraph, Heading, ListItem (no nested Table)
```

Style attributes are stored in each node's `LoroMap` `"a"`:

```
// "a" of the Table node:
{
  "col_widths":    [f32, ...]         -- column widths in pt
  "merged":        { "<anchor_cell_TreeID>": "<extent_cell_TreeID>", ... }
                                      -- each entry is one merged region, keyed by the
                                        top-left (anchor) cell's stable TreeID, valued by
                                        the bottom-right (extent) cell's stable TreeID.
                                        NEVER numeric [row,col,rowspan,colspan] (those are
                                        positions, not identities: a concurrent
                                        InsertTableRow/Col would silently corrupt them --
                                        see the CRDT note below). rowspan/colspan are DERIVED
                                        at layout by locating the two corners in the current
                                        grid; nothing positional is stored.
}

// "a" of the TableCell node:
{
  "bg":            "#rrggbb" | null
  "border_top":    f32                -- thickness in pt (0 = none)
  "border_bottom": f32
  "border_left":   f32
  "border_right":  f32
  "border_color":  "#rrggbb"
  "padding_pt":    f32
  "valign":        "top"|"middle"|"bottom"
}
```

```rust
#[derive(Debug, Clone)]
pub struct TableStyle {
    pub header_row:  bool,   // first row bold / gray background
    pub banded_rows: bool,   // alternating row color
    pub banded_cols: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum VAlign { Top, #[default] Middle, Bottom }

/// Cell borders -- thickness in pt (0.0 = no border).
#[derive(Debug, Clone, Copy, Default)]
pub struct CellBorder {
    pub top:    f32,
    pub bottom: f32,
    pub left:   f32,
    pub right:  f32,
    pub color:  u32,  // RGBA -- applied to all four sides
}
```

Commands:

```rust
InsertTableRow   { table_id: TreeID, after_row: Option<TreeID> },
DeleteTableRow   { row_id: TreeID },
InsertTableCol   { table_id: TreeID, after_col: u32, width_pt: Option<f32> },
DeleteTableCol   { table_id: TreeID, col_index: u32 },
ResizeTableCol   { table_id: TreeID, col_index: u32, width_pt: f32 },
MergeTableCells  { table_id: TreeID, start: (u32, u32), end: (u32, u32) },
UnmergeTableCell { cell_id: TreeID },
SetCellStyle     { cell_id: TreeID, bg: Option<u32>, valign: Option<VAlign>,
                   border: Option<CellBorder>, padding_pt: Option<f32> },
SetTableStyle    { table_id: TreeID, style: TableStyle },
```

`MergeTableCells.start`/`end` are the selection corners **at command time** (u32 grid
positions from the current view); the dispatcher resolves them to the anchor and extent
**TreeIDs** and writes one `merged` entry keyed on those stable ids. The command input is
positional; the *stored* merge is never positional.

**Concurrent-edit semantics (CRDT).** This mirrors the resolved Trame merge model
(`trame-logic §12` / §6) so both apps behave identically:

- **Insert inside a merge.** A concurrent `InsertTableRow`/`InsertTableCol` whose new line
  falls between the anchor and extent corners is **absorbed**: the derived rowspan/colspan
  grow because layout recomputes them from the current position of the two stable corners
  (Excel behavior, fully deterministic -- no stored count to go stale).
- **Corner deleted.** If a concurrent `DeleteTableRow`/`DeleteTableCol` tombstones the
  anchor **or** extent cell, the merge is **dropped** at post-import validation (the corner
  TreeID no longer resolves): the region unmerges and the surviving covered cells reappear
  as normal cells. Same philosophy as Trame's `#REF!` on a deleted bound id.
- **Overlapping merges.** Two concurrent merges covering a shared cell are resolved by
  **LWW** on the `merged` map (the entry with the most recent Loro `ListItemID` wins; the
  loser is removed at validation). Covered cells are **masked, not destroyed** -- their
  `LoroText` content survives and reappears verbatim on unmerge. A cell found covered while
  still holding content raises `ProseEvent::MergeConflictDetected { cell_id }` for a UI hint
  (non-blocking), exactly like Trame.

Post-import validation runs `detect_and_handle_merge_conflicts(table_id)` after every remote
delta (drop dangling-corner merges, LWW-resolve overlaps, refresh the masked-cell set); it is
deterministic in `merged` + node liveness alone, so all peers converge.

**`col_widths` / column-count sync (P-62).** `col_widths` MUST always have exactly one entry
per grid column; the dispatcher keeps them in lockstep:

- `InsertTableCol { after_col, width_pt }` inserts a `TableCell` at position `after_col + 1` in
  every row **and** splices one entry into `col_widths` at the same index. `width_pt` defaults to
  the document's default column width (`meta.defaults.col_width_pt`, 72 pt) when `None`; the new
  column does **not** steal width from its neighbours (the table grows wider), matching Docs.
- `DeleteTableCol { col_index }` removes the cell at `col_index` in every row **and** removes the
  `col_widths[col_index]` entry. Deleting the last remaining column deletes the whole `Table`
  node (an empty table is not a valid state).
- `ResizeTableCol` only rewrites `col_widths[col_index]`; it never changes the count.

**Local merge/delete edge cases (P-63)** (the CRDT section above covers *concurrent* races;
these are the *single-peer, command-time* rules, consistent with the TreeID-keyed
`{anchor -> extent}` model):

- **`MergeTableCells` over an already-merged / overlapping region.** The dispatcher first
  resolves the requested corners to the covered cell set. If that set intersects any existing
  merge, all intersecting `merged` entries are **removed first**, then the single new entry is
  written (a merge always supersedes and absorbs the merges it overlaps). A degenerate selection
  of a single cell (`start == end`) is a no-op. A selection whose covered set is not a solid
  rectangle is rejected with `CommandRejected { reason: NonRectangularMerge }`.
- **`UnmergeTableCell` on a non-merged cell.** If `cell_id` is neither an anchor of a `merged`
  entry nor covered by one, the command is a **no-op** (idempotent unmerge). If `cell_id` is a
  *covered* (masked) cell rather than the anchor, the merge that covers it is resolved to its
  anchor and that entry is removed.
- **`DeleteTableRow`/`DeleteTableCol` crossing a merged span.** Deleting a line that contains a
  merge **anchor or extent** corner drops that `merged` entry (the region unmerges; surviving
  covered cells reappear as normal cells) before the line is removed. Deleting an *interior* line
  of a merge (neither corner) shrinks the derived rowspan/colspan automatically, since spans are
  recomputed from the two surviving corners.

---

### E. Images

Images are `BlockType::Image` nodes in the `MovableTree`, inserted via
`ProseCommand::InsertImage`. Their attributes are stored in the node's `LoroMap` `"a"`.

#### CRDT Attributes

```
"a" of the Image node:
├── "content_hash" : String -- content hash addressing the encrypted media blob (resolved via the ambre-sync media flow)
├── "alt"       : String   -- alt text
├── "w"         : u32      -- width in px (display dimension, not original dimension)
├── "h"         : u32      -- height in px
├── "align"     : u8       -- ImageAlign discriminant
└── "caption"   : String   -- optional caption (used by O.8 Table of Figures)
```

```rust
// ambre-prose/src/features/image.rs

#[derive(Debug, Clone, Copy, Default)]
#[repr(u8)]
pub enum ImageAlign {
    /// Independent block, full width or fixed width -- default behavior.
    #[default]
    Block   = 0,
    /// Post-MVP: left-aligned, text wrapped to the right.
    /// MVP: treated as Block (no text wrapping).
    Left    = 1,
    /// Post-MVP: right-aligned, text wrapped to the left.
    /// MVP: treated as Block (no text wrapping).
    Right   = 2,
    /// Centered, no wrapping.
    Center  = 3,
}
```

> **MVP note:** `ImageAlign::Left` and `Right` are accepted in the CRDT and preserved
> on docx import, but the pagination engine (`prose-engine` §2.B Phase 1) treats them
> as `Block` in the MVP. Text wrapping is post-MVP -- it requires a line-breaking
> algorithm with exclusion zones, extension of Parley.

**Nesting rule:** `Image` is a direct child of the root only (not inside a
`TableCell`, not inside a `ListItem`). Images inside tables are post-MVP.

#### Blob Resolution

The `content_hash` addresses an encrypted media blob. Resolution goes through the Foundation
media flow (`ambre-spec-architecture.md §2.D`): the Logic thread issues
`RestCommand::DownloadMedia { document_id, content_hash }`; the **Sync worker** performs the
presigned-URL GET and decryption and replies `RestEvent::MediaDownloaded { content_hash, data }`,
which `ProseState::on_media_downloaded` caches in `media_cache`. The Logic thread makes **no
network call and holds no crypto**. Until the blob is cached (offline or download pending), the
image displays a placeholder (`[image]` + `alt`).

**Resolution state machine and failure path (P-65).** Each referenced `content_hash` has a
resolution state tracked in the Logic thread (never in the CRDT, since it is per-peer/session):

```rust
// ambre-prose/src/features/image.rs

#[derive(Debug, Clone)]
pub enum MediaState {
    /// No download issued yet (image just scrolled into view / doc just opened).
    Pending,
    /// RestCommand::DownloadMedia issued, awaiting the Sync worker reply.
    Loading,
    /// Blob decrypted and cached; ready to decode/paint.
    Ready,
    /// Terminal failure -- see reason. NOT retried automatically.
    Failed(MediaFailure),
}

#[derive(Debug, Clone)]
pub enum MediaFailure {
    /// Blob absent server-side / presigned GET returned 404 (never uploaded, or GC'd).
    Missing,
    /// AEAD decryption failed (wrong key or tampered ciphertext).
    DecryptFailed,
    /// Bytes downloaded and decrypted but not a decodable image.
    CorruptImage,
    /// Network error persisted past the retry budget (below).
    NetworkExhausted,
}
```

The Sync worker reports failures via `RestEvent::MediaDownloadFailed { content_hash, reason }`.
`on_media_download_failed` moves the entry to `MediaState::Failed(..)` and emits
`ProseEvent::ImageFailed { block_id, reason }` so `prose-ui` swaps the neutral placeholder for a
**distinct broken-image glyph plus a "retry" affordance** -- the image is never stuck on the
loading placeholder forever. `NetworkExhausted` is reached after a bounded retry policy owned by
the Sync worker (3 attempts, exponential backoff); `Missing`/`DecryptFailed`/`CorruptImage` are
terminal on the first reply (retrying cannot help). A user-initiated retry (`RetryImageLoad
{ block_id }`) resets the entry to `Pending` and re-issues the download; coming back online does
not auto-retry terminal failures (only `Pending`/`Loading` entries are re-driven on reconnect).

#### Cache eviction policy (P-66)

`media_cache` maps `content_hash -> decoded bytes` in the Logic thread. It is a **bounded
LRU** so a document with many large images cannot grow the Logic-thread heap without limit
(the client-memory pillar). The budget is by **decoded byte size**, not entry count:

```rust
// ambre-prose/src/features/image.rs

/// Soft cap on the total decoded bytes held in `media_cache`.
/// 64 MiB: comfortably covers the images visible across a few pages while staying
/// well inside the <100 MB client working-set pillar.
const MEDIA_CACHE_BUDGET_BYTES: usize = 64 * 1024 * 1024;
```

Eviction rule: on each insert, if the running total would exceed `MEDIA_CACHE_BUDGET_BYTES`,
least-recently-*painted* entries are dropped until it fits. Eviction only frees the decoded
cache (a per-session view); it never touches the CRDT (`content_hash` stays in the node),
so an evicted image whose `content_hash` scrolls back into view simply re-enters
`MediaState::Pending` and is re-fetched. An entry currently `Loading` is never evicted (its
download is in flight); a single image larger than the whole budget is kept (it is the only
way to display it) but logged.

#### Commands

```rust
// InsertImage already in ProseCommand (prose-engine)
// Additional commands:
ResizeImage  { block_id: TreeID, width_px: u32, height_px: u32, lock_aspect: bool },
SetImageAlign { block_id: TreeID, align: ImageAlign },
SetImageAlt  { block_id: TreeID, alt: String },
SetImageCaption { block_id: TreeID, caption: String },
```

**`ResizeImage` validation (P-67).** Dimensions are validated by the dispatcher before the
CRDT is touched:

- **Zero / non-positive.** A `width_px == 0` or `height_px == 0` is rejected with
  `CommandRejected { reason: DegenerateImageSize }` (a zero-area image is not a valid state).
- **Absurdly large.** Each dimension is clamped to `MAX_IMAGE_DIM_PX = 10_000` px (past this
  the layout cost is unbounded and no page can show it at native size); the clamped value is
  what gets stored.
- **Aspect lock.** When `lock_aspect` is `true`, the dispatcher preserves the intrinsic
  ratio: it takes the dimension that changed the most relative to the current `w`/`h` as the
  driver and recomputes the other from the intrinsic (decoded) ratio, so a corner-drag keeps
  proportions. When the intrinsic ratio is unknown (blob not yet decoded), the current
  stored `w:h` ratio is used as the fallback.

**Nesting validation (P-64).** The structural rules ("no nested `Table`", `Image` is a
**root child only**) are **enforced at dispatch time**, not merely documented:
`ProseState::dispatch()` rejects any command whose resulting parent/child relation would
violate them -- an `InsertTableRow`/cell-content command that would place a `Table` inside a
`TableCell`, or an `InsertImage`/move that would parent an `Image` under a `TableCell` or
`ListItem` -- with `CommandRejected { reason: IllegalNesting { child, parent } }`. The same
predicate is reused by `prose-interop` during docx/HTML import: a violating imported node is
**hoisted to the nearest legal ancestor** (the image is lifted out of the cell to a
following root block) and a `ConversionReport` warning is emitted, so an illegal structure
can never enter the CRDT from either the command path or the import path.

---

### F. Table of Contents (TOC)

Derived view of the `MovableTree`, recomputed on each `LayoutUpdated`.

```rust
// ambre-prose/src/features/toc.rs

#[derive(Debug, Clone)]
pub struct TocEntry {
    pub block_id:       TreeID,
    pub level:          u8,
    pub text:           String,
    pub section_number: Option<Vec<u32>>,
    pub page:           u32,
}

#[derive(Debug, Clone)]
pub struct TableOfContents {
    pub entries:   Vec<TocEntry>,
    pub max_level: u8,
}

// Impl: `ambre-prose/src/features/toc.rs`.
// Walks the tree in document order, keeps every `BlockType::Heading` with level
// <= max_level, resolves each heading's start page from `layout`, and (when
// `numbered`) prefixes the `compute_section_number` counter. See the 3-pass TOC
// stabilization in `ambre-spec-prose-logic.md` §2.B.
pub fn build_toc(
    tree: &LoroMovableTree,
    layout: &PageLayout,
    max_level: u8,
    numbered: bool,
) -> TableOfContents;
```

Commands:

```rust
InsertToc      { parent_id: Option<TreeID>, after_id: Option<TreeID>, max_level: u8, numbered: bool },
UpdateTocConfig { block_id: TreeID, max_level: u8, numbered: bool },
```

---

### G. Headers and Footers

Headers and footers are `LoroText` nodes in the `"blocks"` tree, independent from the
document body and repeated on each page. `"meta"` of the `LoroDoc` holds only their
configuration plus the stable `block_id` (`TreeID`) that points at the text node -- the
text is never duplicated inline (see `prose-logic §2.A`). Editing commands (`InsertText`,
`ApplyMark`, ...) target the header/footer via this `block_id` like any other block.

```
"meta"
├── "different_first_page" : bool    -- document-level flag; governs BOTH header_first
│                                     and footer_first at once (Google Docs semantics:
│                                     a single "Different first page" checkbox).
├── "header" : LoroMap
│   ├── "enabled"  : bool
│   ├── "block_id" : String   -- stable TreeID of the header LoroText node
│   └── "align"    : "left"|"center"|"right"
├── "header_first" : LoroMap   -- content used on page 0 iff different_first_page = true
│   ├── "block_id" : String    -- (same config shape as "header")
│   └── "align"    : "left"|"center"|"right"
├── "footer" : LoroMap
│   ├── "enabled"  : bool
│   ├── "block_id" : String
│   └── "align"    : "left"|"center"|"right"
└── "footer_first" : LoroMap    -- content used on page 0 iff different_first_page = true
    ├── "block_id" : String
    └── "align"    : "left"|"center"|"right"
```

**First-page header/footer activation (P-53).** `different_first_page` is a **single
document-level flag** (hoisted out of `"header"`), not a per-side one: it activates
`header_first` **and** `footer_first` together, exactly like the single Google Docs
checkbox. When it is `false`, `header_first`/`footer_first` are inert (their `block_id`
nodes are retained but never rendered, so toggling the flag back on is lossless). The
pagination engine reads this one flag to decide, on page 0 only, whether to render the
`*_first` block instead of the regular one (`prose-logic §2.B`); `header.enabled` /
`footer.enabled` still gate whether the side is shown at all.

Headers/footers content supports **dynamic fields** inserted as special marks. Their
display value is dynamically resolved by `prose-ui` from the current `PageLayout` --
they contain no literal value in the CRDT.

```rust
// ambre-prose/src/mark.rs
pub const MARK_FIELD_PAGE_NUM:   &str = "f:pn";  // current page number
pub const MARK_FIELD_PAGE_COUNT: &str = "f:pc";  // total page count
pub const MARK_FIELD_DATE:       &str = "f:dt";  // last modified date
pub const MARK_FIELD_TITLE:      &str = "f:ti";  // document title
```

These marks all have `ExpandType::None`.

**Date field format/locale (P-55).** The `f:dt` mark stores **no literal date** in the
CRDT (like every dynamic field). Its value carries only the desired *format*, resolved by
`prose-ui` at paint time against `meta.updated_at` and the document locale:

```rust
// ambre-prose/src/mark.rs -- value grammar of the "f:dt" mark.
/// Format selector stored as the mark value (an ASCII tag, not a rendered string).
/// The rendered text is produced by prose-ui from `meta.updated_at` (a u64 epoch-ms
/// timestamp) and the document locale (`meta.lang`, falling back to the export locale
/// used elsewhere), so it re-localizes without a CRDT rewrite and stays convergent.
pub enum DateFieldFormat {
    /// Locale short date, e.g. en-US "7/19/26", fr-FR "19/07/2026".
    Short,
    /// Locale long date, e.g. en-US "July 19, 2026", fr-FR "19 juillet 2026".
    Long,
    /// ISO-8601 calendar date, locale-independent: "2026-07-19".
    Iso,
}
```

The source timestamp is `meta.updated_at` (the last-modified time); resolving from a
per-peer wall clock is forbidden (it would render differently on each peer and break
convergence of what is displayed). `Short`/`Long` follow `meta.lang`; `Iso` is stable
regardless of locale.

```rust
#[derive(Debug, Clone)]
pub enum HeaderField { PageNumber, PageCount, Date(DateFieldFormat), Title }

/// Which of the two header/footer variants a field-insertion targets.
/// The regular block, or the first-page block (only meaningful when
/// `different_first_page` is on -- see P-53).
#[derive(Debug, Clone, Copy)]
pub enum HfTarget { Regular, FirstPage }
```

Commands:

```rust
SetHeaderEnabled      { enabled: bool },
SetFooterEnabled      { enabled: bool },
// The header and footer each have a stable block_id in "meta".
// To edit their content, use InsertText / ApplyMark / etc. targeting that block_id
// directly -- identical to any other block. No dedicated recursive command.
//
// `target` (P-54) disambiguates the regular vs first-page block when
// different_first_page is on. With the flag off, `HfTarget::FirstPage` is rejected
// (CommandRejected { reason: FirstPageInactive }); Regular is always valid.
InsertHeaderField     { field: HeaderField, target: HfTarget, pos: u32 },
InsertFooterField     { field: HeaderField, target: HfTarget, pos: u32 },
SetDifferentFirstPage { enabled: bool },
```

**Field-insertion target (P-54).** `InsertHeaderField`/`InsertFooterField` route to the
`block_id` of the block named by `target`: `HfTarget::Regular` -> `meta.header`/`meta.footer`,
`HfTarget::FirstPage` -> `meta.header_first`/`meta.footer_first`. This resolves the former
ambiguity where `{field, pos}` could not say which of the two variants (regular vs
first-page) a field landed in once `different_first_page` was on. A `pos` past the current
length of the target `LoroText` is clamped to its end.

---

### H. Comments

Comments are discussion threads anchored on a text range via the `"cm"` mark.

```
"annots"
└── "{comment_id}" : LoroMap
    ├── "anchor_block" : String
    ├── "anchor_start" : u32
    ├── "anchor_end"   : u32
    ├── "resolved"     : bool
    ├── "created_at"   : u64
    └── "replies"      : LoroList<LoroMap>
        └── LoroMap
            ├── "id"         : String (Uuid)
            ├── "author"     : String
            ├── "peer_id"    : u64
            ├── "created_at" : u64
            └── "text"       : LoroText
```

Commands:

```rust
AddComment       { block_id: TreeID, range: (u32, u32), text: String },
ReplyToComment   { comment_id: Uuid, text: String },
ResolveComment   { comment_id: Uuid },
ReopenComment    { comment_id: Uuid },
DeleteComment    { comment_id: Uuid },
DeleteReply      { comment_id: Uuid, reply_id: Uuid },
EditCommentReply { comment_id: Uuid, reply_id: Uuid, new_text: String },
```

**Author / peer_id source (P-30).** The commands carry only `text`: the dispatcher fills
`author` and `peer_id` from the **current session identity**, never from the client-supplied
payload. `peer_id` is the local `LoroDoc` peer id; `author` is the session `display_name`
(the same Sync-resolved identity used for presence -- see `prose-logic §2`). A client cannot
forge another user's name on a comment or reply because the field is stamped server-of-record
side (the local Logic thread), not read off the wire.

**Anchor reconciliation -- the mark is authoritative (P-68).** A comment has two positional
records: the live `"cm"` mark on the `LoroText` and the static `anchor_block`/`anchor_start`/
`anchor_end` in `"annots"`. The **mark is the single source of truth** for where the comment
currently sits; the stored `anchor_*` triple is a **cache** used only to (a) render the thread
before the block's runs are shaped and (b) survive as a last-known position if the mark is
gone. On every edit that touches an annotated block, `reconcile_comment_anchors(block_id)`
rewrites the `anchor_*` cache from the current span of the `"cm"` mark. If the two ever
disagree (e.g. after a remote delta), the mark wins and the cache is refreshed; the `anchor_*`
values never drive rendering when a live mark exists.

**Orphaning and overlap (P-69).**

- **Orphan on full delete.** When the entire marked range is deleted, the `"cm"` mark
  disappears with the text. The comment is **not** deleted from `"annots"`: it becomes an
  **orphan** (`resolved` untouched, `anchor_*` frozen at the last-known position) and
  `prose-ui` lists it in an "orphaned comments" affordance rather than dropping the
  discussion. A later `DeleteComment` is the only thing that removes the thread. This matches
  the test-table expectation ("Add -> Reply -> Resolve -> orphan").
- **Overlapping comments.** A single `"cm"` mark is single-valued, so overlapping comments on
  the same characters cannot both be stored as one mark. Overlap is modeled by **layering**:
  each comment owns its own `"cm"` span, and where spans overlap the newer mark's value wins
  the raw mark read, but both `annots` entries remain live and both highlights are rendered
  (stacked) by `prose-ui`. Resolving/deleting one comment only rewrites its own span; the
  other's span is untouched.

**Replies and permissions (P-70).**

- **`DeleteReply`** removes a single reply from the thread's `"replies"` list by `reply_id`.
  Deleting the **first** reply (the opening comment) is equivalent to `DeleteComment` (the
  thread has no head left) and is treated as such.
- **Reply to a resolved comment.** `ReplyToComment` on a `resolved == true` thread is allowed
  and **auto-reopens** it (`resolved -> false`), matching Google Docs; `ReopenComment` exists
  for an explicit reopen without adding a reply.
- **Editing/deleting others' replies.** `EditCommentReply` and `DeleteReply` are permitted
  **only on the caller's own reply** (`reply.peer_id`/`author` == session identity); a command
  targeting another peer's reply is rejected with `CommandRejected { reason: NotReplyAuthor }`.
  `ResolveComment`/`ReopenComment`/`DeleteComment` (thread-level) are not so restricted -- any
  collaborator may resolve or delete a thread, as in Docs.

---

### I. Track Changes (Suggestions)

Model via `MarkType` on `LoroText`:
- `MARK_SUGGEST_INS` (`"si"`): text present, suggested as insertion.
- `MARK_SUGGEST_DEL` (`"sd"`): text present, suggested as deletion.

```
"annots"
└── "{suggestion_id}" : LoroMap
    ├── "type"       : u8  -- 0 = insertion, 1 = deletion
    ├── "author"     : String
    ├── "peer_id"    : u64
    └── "created_at" : u64
```

#### Suggestion grouping granularity (P-71)

`suggestion_id` groups the marks that accept/reject as **one unit**. The grouping is
**per contiguous authoring burst**, not per keystroke and not per whole session: the Logic
thread keeps a `current_suggestion: Option<(Uuid, Instant)>` per peer and reuses the same
`suggestion_id` while consecutive edits from that peer are **adjacent** (touching or
overlapping the group's current span) **and** within a coalescing window
(`SUGGESTION_MERGE_MS = 2000`, aligned with the undo merge interval). A caret jump to a
non-adjacent position, a 2 s idle gap, or a switch between insertion and deletion at
disjoint ranges starts a **new** id. This makes accept/reject operate on a human-meaningful
change ("the sentence I just typed") rather than one character or the entire session.

#### Mutations in Suggestion Mode

```rust
// ambre-prose/src/features/track_changes.rs

pub fn intercept_command(
    cmd: ProseCommand,
    peer_id: u64,
    suggestion_id: Uuid,
) -> Vec<ProseCommand> {
    match cmd {
        ProseCommand::InsertText { block_id, pos, text } => vec![
            ProseCommand::InsertText { block_id: block_id.clone(), pos, text: text.clone() },
            ProseCommand::ApplyMark {
                block_id,
                range: (pos, pos + text.chars().count() as u32),
                key:   MARK_SUGGEST_INS.to_string(),
                value: Some(suggestion_id.to_string()),
            },
        ],
        // Deleting text in suggestion mode. Per P-72, a deletion over the author's own
        // still-pending SUGGEST_INS is a real removal (retracting an unaccepted insertion),
        // NOT a "suggest deletion of my own suggestion". We split the requested range into:
        //   - sub-ranges carrying this peer's own pending "si": hard-deleted outright,
        //   - everything else (original text, or other peers' suggestions): marked "sd".
        ProseCommand::DeleteText { block_id, range } => {
            let mut out = Vec::new();
            // Reverse order so earlier offsets stay valid as we emit edits.
            for seg in segment_delete_range(&block_id, range, peer_id).into_iter().rev() {
                match seg.kind {
                    // Own un-accepted insertion -> physically remove it (and its "si" mark
                    // goes with the text). Overlap/nesting handled because we segment first.
                    SegKind::OwnPendingIns => out.push(ProseCommand::DeleteText {
                        block_id: block_id.clone(),
                        range: seg.range,
                    }),
                    // Original text or another peer's content -> mark as suggested deletion.
                    SegKind::Other => out.push(ProseCommand::ApplyMark {
                        block_id: block_id.clone(),
                        range: seg.range,
                        key:   MARK_SUGGEST_DEL.to_string(),
                        value: Some(suggestion_id.to_string()),
                    }),
                }
            }
            out
        }
        other => vec![other],
    }
}

/// A slice of the requested delete range classified by what suggestion (if any) covers it.
struct DeleteSegment { range: (u32, u32), kind: SegKind }
enum SegKind {
    /// Covered by a still-pending MARK_SUGGEST_INS whose value == a suggestion this peer owns.
    OwnPendingIns,
    /// Original text, or a suggestion (ins or del) owned by another peer.
    Other,
}

/// Walk the "si" marks intersecting `range` and cut it into maximal same-classification
/// segments. Contiguous runs of the same kind are coalesced. Reads authorship from the
/// suggestion's `annots.{id}.peer_id`, so a nested/overlapping foreign insertion inside an
/// own one is correctly split out as `Other`.
/// Engine query (impl: `ambre-prose/src/features/suggest.rs`) -- pure classification,
/// no mutation.
fn segment_delete_range(
    block_id: &TreeID,
    range: (u32, u32),
    peer_id: u64,
) -> Vec<DeleteSegment>;

/// Accept a single suggestion: an insertion becomes permanent (strip "si"), a deletion is
/// applied for real (remove the covered text, which drops "sd" with it). Idempotent.
pub fn accept_suggestion(doc: &mut LoroDoc, suggestion_id: Uuid) {
    let sid = suggestion_id.to_string();
    // A suggestion may span multiple blocks (P-73): resolve every "si"/"sd" span whose
    // value == sid across the whole tree, not just one block.
    for (block_id, range, is_insertion) in suggestion_spans(doc, &sid) {
        let text = doc.get_text(&block_id);
        if is_insertion {
            // Accept insertion: the text stays, the tracking mark is removed.
            text.unmark(range.0, range.1, MARK_SUGGEST_INS);
        } else {
            // Accept deletion: remove the text (the "sd" mark disappears with it).
            text.delete(range.0, range.1);
        }
    }
    // Drop the metadata entry once no span references it any more.
    doc.get_map("annots").delete(&sid);
    doc.commit();
}

/// Reject a single suggestion: an insertion is removed from the doc (leaves no trace --
/// invariant "rejected suggestions removed from the CRDT"), a deletion is cancelled
/// (strip "sd", text stays). Idempotent.
pub fn reject_suggestion(doc: &mut LoroDoc, suggestion_id: Uuid) {
    let sid = suggestion_id.to_string();
    for (block_id, range, is_insertion) in suggestion_spans(doc, &sid) {
        let text = doc.get_text(&block_id);
        if is_insertion {
            // Reject insertion: physically remove the suggested text.
            text.delete(range.0, range.1);
        } else {
            // Reject deletion: keep the text, drop the tracking mark.
            text.unmark(range.0, range.1, MARK_SUGGEST_DEL);
        }
    }
    doc.get_map("annots").delete(&sid);
    doc.commit();
}

pub fn accept_all(doc: &mut LoroDoc) {
    for sid in all_suggestion_ids(doc) {
        accept_suggestion(doc, sid);
    }
}

pub fn reject_all(doc: &mut LoroDoc) {
    for sid in all_suggestion_ids(doc) {
        reject_suggestion(doc, sid);
    }
}

/// All (block_id, range, is_insertion) spans across the whole tree whose "si"/"sd" mark
/// value equals `sid`. A single suggestion can produce several spans (multi-block, P-73).
/// Engine query (impl: `ambre-prose/src/features/suggest.rs`): scan si/sd marks over
/// all blocks, filter value == sid.
fn suggestion_spans(doc: &LoroDoc, sid: &str) -> Vec<(TreeID, (u32, u32), bool)>;

/// The set of distinct suggestion ids currently present as "annots" entries.
/// Engine query (impl: `ambre-prose/src/features/suggest.rs`): keys of `annots`
/// whose entry has a `type` in {0,1}.
fn all_suggestion_ids(doc: &LoroDoc) -> Vec<Uuid>;
```

**Multi-block suggestions and toggling off (P-73).** A single suggestion whose span was
split across a block boundary (e.g. an insertion that included a paragraph break) is stored
as several `si`/`sd` spans sharing one `suggestion_id`; `suggestion_spans` collects **all**
of them, so accept/reject act atomically on the whole change regardless of block count.
`EnableTrackChanges { enabled: false }` **while suggestions are pending** does **not**
silently accept or drop them: the flag only stops *new* edits from being intercepted;
existing `si`/`sd` marks survive and remain individually acceptable/rejectable. Turning
tracking off is orthogonal to resolving what is already tracked -- the pending suggestions
stay until an explicit accept/reject (or `AcceptAll`/`RejectAll`), matching Google Docs
where "Editing" mode still shows the outstanding suggestions.

Commands:

```rust
EnableTrackChanges   { enabled: bool },
AcceptSuggestion     { suggestion_id: Uuid },
RejectSuggestion     { suggestion_id: Uuid },
AcceptAllSuggestions,
RejectAllSuggestions,
```

---

### J. Undo / Redo

```rust
impl ProseState {
    pub fn init_undo_manager(doc: &LoroDoc) {
        let mut um = doc.undo_manager(PeerID::new_local());
        um.set_max_undo_steps(200);
        um.set_merge_interval(Duration::from_millis(500));
    }
}
```

Commands: `Undo`, `Redo` (in `ProseCommand`).

**Documented limitation:** collaborative cross-peer undo not supported -- identical to Google Docs.

---

### K. Footnotes & Cross References

#### Footnotes

Mark `"fn"` on the main `LoroText`, value = `FootnoteId`. Content in `"annots"`:

```
"annots"
└── "{footnote_id}" : LoroMap
    ├── "type"    : "footnote"
    └── "text"    : LoroText
```

The ordinal is computed on demand (`MovableTree` traversal + position in `LoroText`).

**Footnote mark deletion and placement (P-75).**

- **Orphan on delete.** The footnote reference lives as an `"fn"` mark on the body text. When
  that marked character is deleted, the mark disappears with it and the `"annots"` entry is
  **orphaned** (same policy as comments, P-69): it is not auto-purged. `prose-ui` surfaces
  orphaned footnotes for review; only `DeleteFootnote` removes the entry (and it also strips
  any surviving `"fn"` mark carrying that id). Deleting the reference does not renumber via a
  stored ordinal because ordinals are computed on demand -- the remaining notes simply
  renumber at the next read.
- **Where footnotes are allowed.** A footnote reference may sit in **body text and table-cell
  text**, but **not** in header/footer text (a repeated-on-every-page region cannot own a
  document-tail note). `InsertFootnote` targeting a header/footer `block_id` is rejected with
  `CommandRejected { reason: FootnoteNotAllowedHere }`. A footnote referenced from a table
  cell still renders in the page footnote area like any other (its ordinal follows document
  order of the reference position).

#### Cross References

Mark `"ref"` on text. Its value is a **structured** encoding, not a `':'`-joined string:
the former `"{target_block_id}:{kind}"` form was ambiguous because `CrossRefKind::CustomText`
may itself contain `':'`. It is stored as a small tagged record (postcard bytes, hex-encoded
into the mark's `String` value) so an arbitrary custom label round-trips losslessly (P-74):

```rust
/// Wire form of a "ref" mark value. Encoded with postcard, hex in the mark String.
/// The target is a TreeID (stable id), never the target's rendered text, so a moved
/// target keeps resolving.
#[derive(Debug, Clone)]
pub struct CrossRefValue {
    pub target_block_id: TreeID,
    pub kind:            CrossRefKind,
}

#[derive(Debug, Clone)]
pub enum CrossRefKind {
    PageNumber,
    SectionNumber,
    /// Free text; may contain ':' or any Unicode -- safe because it is length-prefixed
    /// by postcard, not delimiter-split.
    CustomText(String),
}
```

Resolved at display time from the current `PageLayout`.

**Dangling reference rule (P-74).** When `target_block_id` no longer resolves to a live node
(the target was deleted), the reference is **not** an error and is **not** removed from the
CRDT: `prose-ui` renders it as a neutral broken-reference placeholder (Google Docs style,
e.g. a struck-through "?" or the last-known label for `CustomText`), and the mark is retained
so that a concurrent undo/re-insert of the target restores resolution. `PageNumber`/
`SectionNumber` against a dead target render the placeholder; `CustomText` still shows its
literal label (it carries its own text and does not depend on the target existing).

Commands:

```rust
InsertFootnote  { block_id: TreeID, pos: u32, note_text: String },
EditFootnote    { footnote_id: Uuid, new_text: String },
DeleteFootnote  { footnote_id: Uuid },
InsertCrossRef  { block_id: TreeID, pos: u32, target_block_id: TreeID, kind: CrossRefKind },
```

---

### L. Find & Replace

Find & Replace operates on the `LoroText::to_string()` of each block in `MovableTree` traversal
order. Search never mutates the CRDT -- only `Replace` sends `ProseCommand`.

**Search scope (P-76).** Find scans **body text only** -- the `LoroText` of every block in
`MovableTree` document order. Text that lives **outside the tree** is **out of scope by
design** and documented as such: comment/reply bodies and suggestion metadata (`"annots"`),
header/footer field content and their `LoroText` (`"meta"`), and footnote note bodies
(`"annots"`). Rationale: these are separate stores with their own navigation surfaces, and
Google Docs likewise does not fold them into the main find pass. Matches are **per-block**;
a query straddling a block boundary does not match (there is no cross-block concatenation),
which keeps `FindMatch.range` a single `(u32, u32)` inside one `block_id`.

```rust
// ambre-prose/src/features/find_replace.rs

#[derive(Debug, Clone)]
pub struct FindQuery {
    pub text:           String,
    pub case_sensitive: bool,
    pub whole_word:     bool,
    pub use_regex:      bool,   // post-MVP
}

#[derive(Debug, Clone)]
pub struct FindMatch {
    pub block_id: TreeID,
    pub range:    (u32, u32),
}

pub fn find_all(
    doc: &LoroDoc,
    tree: &LoroMovableTree,
    query: &FindQuery,
) -> Vec<FindMatch> {
    // Empty query matches nothing (P-77): guard against a zero-length match that would
    // otherwise loop forever / select every gap.
    if query.text.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for block_id in tree.dfs_pre_order() {
        let Some(text) = doc.try_get_text(&block_id) else { continue }; // leaf/non-text
        let hay = text.to_string();
        for (start, end) in match_offsets(&hay, query) {
            out.push(FindMatch { block_id: block_id.clone(), range: (start, end) });
        }
    }
    out
}

/// All match spans of `query` within one block string, honoring case_sensitive and
/// whole_word (regex is post-MVP). Offsets are in the same grapheme unit as ProseCommand
/// positions (P-11), not raw bytes.
/// Engine query (impl: `ambre-prose/src/features/find.rs`): scan `hay` for `query.text`,
/// non-overlapping, with a word-boundary filter when `whole_word`.
fn match_offsets(hay: &str, query: &FindQuery) -> Vec<(u32, u32)>;

/// Returns commands in reverse document order
/// so that offsets remain valid during application.
pub fn replace_all(
    doc: &LoroDoc,
    tree: &LoroMovableTree,
    query: &FindQuery,
    replacement: &str,
) -> Vec<ProseCommand> {
    if query.text.is_empty() {
        return Vec::new(); // empty query never replaces (P-77)
    }
    // Reverse document order so each edit's offsets stay valid as later edits apply.
    let mut cmds = Vec::new();
    for m in find_all(doc, tree, query).into_iter().rev() {
        // Model a replace as delete-then-insert at the match start. Doing both as one
        // range operation means the span's *inline marks are dropped* on the replaced
        // text (P-77): the new text inherits only the mark set active at the insertion
        // point (ExpandType::After marks continue, annotation marks do not), so bold over
        // an old word does not silently coat an unrelated replacement. Annotation marks
        // (cm/si/sd/fn/ref) covering the exact replaced span are intentionally not
        // reapplied -- replacing the anchored text orphans the annotation per P-69/P-75.
        cmds.push(ProseCommand::DeleteText { block_id: m.block_id.clone(), range: m.range });
        cmds.push(ProseCommand::InsertText {
            block_id: m.block_id,
            pos:      m.range.0,
            text:     replacement.to_string(),
        });
    }
    cmds
}
```

Current state (occurrences, selected index) is maintained in the Logic thread and sent
to the UI thread via `ProseEvent` for highlighting.

Commands:

```rust
FindStart  { query: FindQuery },
FindNext,
FindPrev,
FindClose,
ReplaceOne { replacement: String },
ReplaceAll { replacement: String },
```

Events:

```rust
FindResults { matches: Vec<FindMatch>, current: usize },
FindUpdated { current: usize },
```

---

### M. Word Count

The text tallies (words, characters, paragraphs) are recomputed in the Logic thread on each
`DocUpdated` (debounced 500 ms). The **page count is not** a text tally: it is read from the
`PageLayout` and refreshed on each `LayoutUpdated` (P-78). Coupling `pages` to the debounced
`DocUpdated` would leave it stale relative to the current layout, so `WordCount.pages` is
sourced from `layout.page_count` and updated on the layout event; the two triggers write into
one cached `WordCount` and the status bar always shows the latest of each.

```rust
// ambre-prose/src/features/word_count.rs

#[derive(Debug, Clone)]
pub struct WordCount {
    pub words:                  u32,
    pub characters:             u32,   // graphemes excluding whitespace -- see below
    pub characters_with_spaces: u32,   // all graphemes including whitespace
    pub paragraphs:             u32,
    pub pages:                  u32,   // from PageLayout.page_count (LayoutUpdated, P-78)
}

/// Recompute the text tallies. `pages` is filled separately from PageLayout (P-78); this
/// function leaves it at 0 and the caller stamps the layout's page_count.
///
/// **"characters without spaces" definition (P-28).** Counting is over Unicode **grapheme
/// clusters** (`unicode-segmentation`), consistent with ProseCommand offsets -- never
/// `char`/`u8`. `characters` excludes any grapheme that is Unicode-whitespace
/// (`char::is_whitespace` on the cluster's base scalar: spaces, tabs, and the synthetic
/// newline between blocks); `characters_with_spaces` includes them. An empty document
/// yields all-zero. Table/list/footnote/header text follows the same `blocks` scope rule.
pub fn compute_word_count(
    doc: &LoroDoc,
    tree: &LoroMovableTree,
    layout: &PageLayout,
    blocks: Option<&[TreeID]>,   // None = entire document (body tree order)
) -> WordCount {
    use unicode_segmentation::UnicodeSegmentation;

    let ids: Vec<TreeID> = match blocks {
        Some(sel) => sel.to_vec(),
        None => tree.dfs_pre_order().collect(),
    };

    let mut wc = WordCount {
        words: 0, characters: 0, characters_with_spaces: 0,
        paragraphs: 0, pages: 0,
    };
    for block_id in &ids {
        let Some(text) = doc.try_get_text(block_id) else { continue };
        let s = text.to_string();
        // Words: unicode word segmentation, keeping only alphanumeric-bearing tokens.
        wc.words += s.unicode_words().count() as u32;
        for g in s.graphemes(true) {
            wc.characters_with_spaces += 1;
            if !g.chars().next().is_some_and(char::is_whitespace) {
                wc.characters += 1;
            }
        }
        // Each text-bearing block counts as one paragraph (headings/list items included,
        // matching Docs' paragraph tally).
        wc.paragraphs += 1;
    }
    // `pages` is stamped by the caller from `layout.page_count` on LayoutUpdated (P-78).
    let _ = layout;
    wc
}
```

Event: `ProseEvent::WordCountUpdated { count: WordCount }`.
Displayed in the status bar by `prose-ui`.

---

### N. Spell Checker *(post-MVP)*

Priority P4 in the Logic thread, never blocking P1-P3.

```rust
// ambre-prose/src/features/spellcheck.rs

pub struct SpellChecker {
    dict: zspell::Dictionary,
    lang: String,
}

#[derive(Debug, Clone)]
pub struct SpellError {
    pub range:       (u32, u32),
    pub word:        String,
    pub suggestions: Vec<String>,  // max 5
}
```

The dictionary is lazy-loaded from a separate WASM asset -- not in the main binary.

Commands:

```rust
SetSpellCheckLang     { lang: String },
AcceptSpellSuggestion { block_id: TreeID, range: (u32, u32), replacement: String },
AddToDictionary       { word: String },
```

**Note (P-84):** the custom dictionary written by `AddToDictionary` is PER-USER, not per-doc.
It lives in the encrypted user-settings store (under the user's OWN key), NEVER in the document
`LoroDoc` -- this preserves the zero-knowledge boundary: nothing new crosses the relay. For MVP it
is local-only (browser persistent storage), with no cross-device sync; cross-device sync via the
user-settings channel is post-MVP.

---

### O. Post-MVP Features

The features below are **specified for ISO Google Docs completeness** but are not part
of the MVP. Each can be implemented independently on top of the existing CRDT foundations.

---

#### O.1 Version History

Named document snapshots at a point in time, navigable and restorable. Distinct from
track changes (which is intra-document).

**Model:** `"meta.version_history"` (`LoroList<LoroMap>`), each entry `{name, created_at, snapshot_id}`. The Loro snapshot is stored server-side (S3), referenced by `snapshot_id`, encrypted with the current `DocumentKey` before upload -- identical to the normal SecSync pipeline.

**Commands:** `SaveVersion { name }`, `RestoreVersion { snapshot_id }`, `ListVersions`,
`DeleteVersion { snapshot_id }`.

---

#### O.2 Multi-Column Layout

2 or 3 text columns in a section, newspaper style.

**Model:** attribute `"columns": {count: u8, gap_pt: f32, equal_width: bool}` on a
`BlockType::Section` node (new type). The pagination calculation in `prose-engine` is
extended to distribute child blocks across multiple columns.

---

#### O.3 Section Breaks

Changes orientation, margins, or column count from a point in the document.

**Model:** `BlockType::SectionBreak` with attributes `{orientation: Portrait|Landscape, margins: Option<IrPage>, columns: Option<u8>}`. Pagination in `prose-engine` re-reads parameters at each `SectionBreak`.

---

#### O.4 Equation Editor

Inline and block mathematical formulas, input via LaTeX or visual editor.

**Model:** `BlockType::Equation` and mark `MARK_EQUATION` (`"eq"`), value = LaTeX string
in the CRDT. Rendering is produced in the Logic thread (LaTeX engine -> SVG, to be evaluated
between `typst` WASM and a custom MathML subset). The SVG is transmitted to the UI thread via
`LogicEnvelope`. Lazy WASM module to avoid impacting startup.

---

#### O.5 Bookmarks

Freely named anchors in the text, distinct from block `TreeID`s.

**Model:** mark `MARK_BOOKMARK` (`"bk"`) on a position or range, value = a stable
`BookmarkId` (`Uuid`), **not** the display name. The human-facing name lives in the
`"annots"` entry keyed by that id (`{ "name": String, "block_id": ..., "range": ... }`).
Keying the mark on a `Uuid` rather than the name makes `DeleteBookmark`/`GoToBookmark`
unambiguous even when two bookmarks share a display name (P-80).

**Name uniqueness (P-80).** Display names are **not** required to be globally unique (Docs
allows duplicates), but the navigation commands take the `BookmarkId`, so they never
disambiguate by name. `InsertBookmark` mints a fresh `Uuid`; if the requested name collides
with an existing one, `prose-ui` may suffix it for display (" (2)") but the identity is the
`Uuid`. The picker lists `(name, BookmarkId)` pairs so a duplicate name still resolves to one
target.

**Commands:** `InsertBookmark { block_id, pos, name }` (returns the new `BookmarkId` via
`ProseEvent`), `DeleteBookmark { id: Uuid }`, `GoToBookmark { id: Uuid }`.

---

#### O.6 Bibliography & Citations

Management of bibliographic sources and citation insertion in the text.

**Model:** `"bibliography"` (`LoroMap`) at the root of the `LoroDoc`. Each source is a
`LoroMap` of CSL (Citation Style Language) fields. Citations are `MARK_CITATION`
(`"cit"`) marks, value = `source_id`. Formatting (APA, IEEE, Chicago, Vancouver) is
computed on demand from CSL data.

**Commands:** `AddSource { source: CslSource }`, `InsertCitation { block_id, pos, source_id }`,
`SetCitationStyle { style: CitationStyle }`, `GenerateBibliography`.

---

#### O.7 Line Numbers

Automatic line numbering in the left margin, typical of legal documents.

**Model:** `"meta.line_numbers": {enabled: bool, start: u32, step: u32, restart_each_page: bool}`.
Numbers are computed from `PageLayout` (Parley lines per block) and rendered by
`prose-ui` in the left margin.

---

#### O.8 Table of Figures & Table of Tables

Derived blocks similar to TOC but for captioned images and tables.

**Model:** `BlockType::TableOfFigures` and `BlockType::TableOfTables` (placeholders). Entries
are extracted from `Image` and `Table` blocks with a `"caption"` attribute.

---

#### O.9 End of Document Index

Alphabetical index of marked terms in the text, with page references.

**Model:** mark `MARK_INDEX_ENTRY` (`"ix"`), value = normalized term. `BlockType::Index`
is a placeholder whose content is computed from `PageLayout` on demand.

**Commands:** `MarkIndexEntry { block_id, range, term }`, `InsertIndex`.

---

#### O.10 Linked Styles (Style Inheritance)

A custom style can inherit from a base style and only override certain properties.

**Model:** `"based_on": String` attribute in the style's `LoroMap`. The `resolve_style`
cascade follows the inheritance chain (max depth: 5 levels, cycle protection via loop
detection).

---

#### O.11 Pageless Mode

Continuous scrolling without page breaks or gutters, adaptive width.

**Model:** `"meta.pageless": bool`. When active, `prose-engine` returns a linear layout
without `page_count`. `prose-ui` removes gutters and adapts width to the viewport.

---

#### O.12 Protected Zones

Read-only ranges for certain peers (e.g. templates with delimited editable zones).

**Model:** mark `MARK_PROTECTED` (`"prot"`), value = list of authorized `peer_id`s.
`ProseState::dispatch()` checks the current peer before applying any mutation on a
protected range.

**Documented limitation:** application-level protection only -- a malicious peer
manipulating the CRDT directly can bypass it. Explicitly documented in the UI.

---

#### O.13 Regex in Find & Replace

Extension of `FindQuery::use_regex` (field already planned, flagged post-MVP). Uses the
`regex` crate (WASM-compatible) for matching.

---

#### O.14 Headers/Footers per Section

Extension of section F for different header/footer content per section.
Depends on O.3 (section breaks).

---

## 3. SECURITY & CONFIDENTIALITY (ZERO-KNOWLEDGE)

All feature data (comment text, suggestions, notes, headers) is stored in the Logic
thread's `LoroDoc` and encrypted by `ambre-sync` before transport. No feature has a
direct path to the network.

- `display_name` in comments and suggestions travels in the encrypted CRDT.
- Collaborative cursors are encrypted in `ambre-sync` -> `Ephemeral.ciphertext`.
- `zspell` is local -- no network call for the spell checker.
- Protected zones (O.12) are an application-level rule -- their security limitation is documented.

---

## 4. QUALITY & TESTABILITY

**Unit tests:**

| Feature | Tested cases |
|---------|-------------|
| Inline styles | Cascade mark > block style > built-in > default. Superscript/subscript mutually exclusive. Partial clear. |
| Lists | Ordinals after insert/delete/move. Reset on unindent. Todo checked/unchecked. |
| Headings | `compute_section_number`: H1->H2->H2->H3 = [1],[1,1],[1,2],[1,2,1]. Reset after H1. |
| Tables | Row/column CRUD. Merge/split. Cell styles. CRDT conflict simultaneous delete+insert. |
| TOC | `build_toc`: 10 mixed headings, section_number + page. |
| Headers/footers | Dynamic fields resolved from mocked `PageLayout`. Different first page. |
| Comments | Add -> Reply -> Resolve -> orphan (annotated text deleted). |
| Track changes | Insert accepted/rejected, Delete accepted/rejected. AcceptAll. 2-peer convergence in suggestion mode. |
| Undo/Redo | 10 keystrokes -> undo -> redo. Local peer isolation. |
| Footnotes | Ordinal recomputation after insert/delete. |
| Find & Replace | Exact match, case insensitive, whole word. ReplaceAll reverse order. No match. |
| Word count | Entire document, partial selection, empty document. |

**Prop-tests:**

```rust
proptest! {
    #[test]
    fn track_changes_roundtrip(ops in vec(arb_insert_delete(), 1..20)) {
        let mut doc = make_doc_with_tracking();
        apply_ops(&mut doc, &ops);
        accept_all(&mut doc);
        assert!(doc.check_state_correctness_slow());
    }
}

proptest! {
    #[test]
    fn find_replace_idempotent(text in "\\PC{1,200}", query in "\\PC{1,10}") {
        let doc = make_doc_with_text(&text);
        let cmds = replace_all(&doc, &tree, &FindQuery::exact(&query), "__REPLACED__");
        let doc2 = apply_commands(doc, cmds);
        assert!(find_all(&doc2, &tree, &FindQuery::exact(&query)).is_empty());
    }
}
```

---

## 5. PERFORMANCE & RESOURCES

| Operation | Target | Thread |
|-----------|--------|--------|
| `build_toc()` -- 50 headings | < 1 ms | Logic |
| `compute_ordinal()` -- 100 items | < 500 µs | Logic |
| `compute_section_number()` -- 50 headings | < 500 µs | Logic |
| `intercept_command()` track changes | < 100 µs | Logic |
| `accept_all()` -- 100 suggestions | < 5 ms | Logic |
| `find_all()` -- 100 pages, query 10 chars | < 20 ms | Logic |
| `replace_all()` -- 50 occurrences | < 30 ms | Logic |
| `compute_word_count()` -- 100 pages | < 5 ms | Logic |
| `check_block()` spell -- 200 words | < 10 ms | Logic P4 (post-MVP) |

**WASM size:**

| Component | Raw estimated | Gzip estimated |
|-----------|-----------|-------------|
| `prose-features` (without zspell) | ~35 KB | ~11 KB |
| `unicode-segmentation` | ~40 KB | ~15 KB |
| `zspell` + dict fr-FR (post-MVP, lazy) | ~550 KB | ~210 KB |

---

## 6. REJECTED ARCHITECTURAL DECISIONS

| Alternative | Rejected because |
|-------------|-------------|
| **Ordinals stored in the CRDT** | Guaranteed CRDT conflicts on any insertion. O(siblings) on demand is sufficient. |
| **TOC as CRDT structure** | Guaranteed desynchronization. Derived view only. |
| **Loro branches for track changes** | Memory overhead O(doc_size × suggestions). Inline MarkType is O(1). |
| **Comments in a separate LoroDoc** | Double synchronization, CRDT consistency impossible with the main doc. |
| **LanguageTool server** | Violation of Zero-Knowledge: plaintext to third-party server. |
| **LanguageTool WASM (grammar)** | Model > 100 MB. Incompatible with binary budget. |
| **Collaborative undo** | Prohibitive algorithmic complexity. Identical to Google Docs choice. |
| **Find & Replace in the UI thread** | The UI thread does not own the CRDT content. |
| **Page numbers stored in the CRDT** | Invalidated as soon as any layout change occurs. Computed from `PageLayout` only. |

---

## 7. INTEGRATION PLAN (ROADMAP)

**Phase 0 -- Styles, Lists, Headings (Week 1)**
- [ ] Complete inline marks (+ superscript/subscript), `resolve_style` cascade, `ParagraphStyle`.
- [ ] Lists: `compute_ordinal()`, `IndentList`/`UnindentList`, all `OrderedStyle`.
- [ ] Headings: `compute_section_number()`.
- [ ] Unit tests styles, lists, headings.

**Phase 1 -- Tables, TOC, Word Count (Week 2)**
- [ ] Tables: row/column CRUD, merge, cell styles (`bg`, border, valign), `TableStyle`.
- [ ] TOC: `build_toc()`, `BlockType::Toc` placeholder.
- Word count: `compute_word_count()`, debounce 500ms, `ProseEvent::WordCountUpdated`.

**Phase 2 -- Headers, Footers, Find & Replace (Week 2-3)**
- [ ] Headers/footers: `"meta.header/footer"` structure, dynamic fields, different first page.
- Find & Replace: `find_all()`, `replace_one()`, `replace_all()` (exact + case insensitive + whole word).

**Phase 3 -- Comments & Track Changes (Week 3)**
- [ ] Comments: `"annots"` structure, `"cm"` marks, Add/Reply/Resolve commands.
- [ ] Track changes: `intercept_command()`, accept/reject, 2-peer CRDT convergence.
- [ ] Prop-tests track changes.

**Phase 4 -- Undo, Footnotes, Cross Refs (Week 4)**
- [ ] Undo/Redo: `init_undo_manager()`, merge_interval, local peer isolation.
- [ ] Footnotes and cross references.

**Phase 5 -- Post-MVP (order suggested by increasing effort)**
- [ ] Regex find & replace -- minimal effort, `FindQuery` already planned.
- [ ] Bookmarks -- simple mark, minimal effort.
- [ ] Version history -- S3 pipeline already in place.
- [ ] Linked styles -- `resolve_style` extension.
- [ ] Pageless mode -- disable pagination in `prose-engine`.
- [ ] Line numbers, Table of figures, O.9 Index -- derived from `PageLayout`.
- [ ] Bibliography -- new CSL subsystem.
- [ ] Section breaks + Multi-columns -- pagination extension.
- [ ] Protected zones -- `ProseState::dispatch()` extension.
- [ ] Headers/footers per section -- depends O.3.
- [ ] Equation editor -- new rendering engine, maximum effort.
- [ ] Spell checker -- `zspell` + lazy asset.
