# AMBRE — SPEC: PROSE / LOGIC
**Type:** Horizontal (Layer/Engine)
**Status:** Draft

> Prose document engine: CRDT structure for text and blocks, inline annotation model,
> collaborative cursor management, pagination calculation, and interface contract between
> `ambre-prose` and the Logic thread. This file defines the foundations on which
> `prose-features`, `prose-interop`, and `prose-ui` build. These four modules are internal
> to the `ambre-prose` crate.

> **Architecture note.** The client uses the **two-instance model** — a **Main** WASM instance (UI thread + Logic thread sharing one linear memory via `wasm_thread`, talking over typed `flume` values) and a **separate Sync worker** (its own WASM instance/memory, reached over `postMessage` + `postcard`, relayed by the Main thread) — see `ambre-spec-architecture.md §2.B`. Consequences for this spec:
> - **State registry.** The state of a Prose document is `ProseState`, a concrete value stored in the Logic thread's typed registry (`enum DocState { Trame(TrameState), Prose(ProseState), … }`) and dispatched via `match` (no trait object, no `Box<dyn Any>`).
> - **Typed data plane.** `ProseCommand`/`ProseEvent` (+ their payload types: `PageLayout`, `BlockLayout`, …) live in **`ambre-bus`** (re-exported by `ambre-prose`) and travel **by move** on `flume`, inside `AppCommand::Prose(..)` / `AppEvent::Prose(..)`. **No `serde` derivation** on them. `serde`/`postcard` only remain for **network bytes**: Loro CRDT payloads (`export_updates`/`export_snapshot`, already `Vec<u8>`) and the `Ephemeral` presence payload (`PresencePayload`, postcard).
> - **Typed outcome for editing.** A handler that mutates the CRDT **returns** domain bytes (Loro delta); the Logic dispatcher emits `SyncCommand::SendUpdate { document_id, payload }`. `ProseState` holds no transport handle of its own. See §C and `ambre-spec-architecture.md §7 ARCH P2`.
> - **Sync→Logic event routing.** `ProseState` does **not** see `LogicEvent`: the generic dispatcher in `ambre-logic` routes incoming events (`RemoteUpdate`/`RemoteSnapshot`/`RemoteEphemeral`/`SnapshotNeeded`/…) to `ProseState`'s content methods (`apply_remote`, `apply_ephemeral`, `export_snapshot`). `LogicEvent::RemoteEphemeral` carries a **Sync-resolved `from_user_id: UserId`** (spoof-proof — a peer cannot forge another user's id, see `ambre-spec-sync.md §3`); the dispatcher passes it to `apply_ephemeral(from_user_id, payload)`.

---

## 1. OBJECTIVES & CONSTRAINTS

**Scope:**

This component handles:
- The CRDT document structure (block hierarchy + body text).
- The inline annotation model (marks/spans) and the `expand_type_for` function (source of truth).
- Pagination calculation (3 phases: pre-computation, Parley measurement, placement with constraints — manual page breaks, widows/orphans, keep_together, keep_with_next, unbreakable headings, footnotes, header/footer, margin collapse) — Logic thread.
- The `ProseState` contract (typed document state, dispatched via `match` in the Logic thread) and Prose IPC types.
- The complete `ProseEvent` enum — single source of truth for all Logic → UI events (`AppEvent::Prose` payload).
- The format of Prose snapshots and deltas (Loro `Vec<u8>` bytes, opaque on the network path).

This component delegates:
- wgpu/Parley rendering (DrawSurface) → `prose-ui` module.
- User-facing features (TOC, track changes, comments, lists…) → `prose-features` module.
- Multi-format import/export → `prose-interop` module.
- Encryption/transport → `ambre-sync`.

**Dependencies:**

| Crate | Justification |
|-------|---------------|
| `loro` | Native Rust CRDT: `LoroDoc`, `LoroMovableTree`, `LoroText`, `LoroMap`. Single source of collaborative truth. |
| `parley` | Typographic layout used in the Logic thread to compute block heights (pagination). Direct dependency (not only via `ambre-draw`) because `ambre-prose` uses Parley's full API (`Layout`, `FontContext`, line-breaking) — not just the `GlyphRun` types exposed by `DrawSurface`. |
| `ambre-core` | `Uuid`, `DocType`, common types (via `ambre-bus` re-export). |
| `ambre-bus` | Typed IPC types: `ProseCommand`/`ProseEvent` (+ payloads), `AppCommand`/`AppEvent`, `SyncCommand`, `RestCommand`/`RestEvent`. |
| `ambre-draw` | `DrawSurface` trait + kurbo/peniko re-exports. Page rendering functions (`render_block_on`, etc.) are parameterized on this trait — same code for screen (WgpuSurface) and PDF export (KrillaSurface). |
| `postcard` | Binary serialization **only for network payloads**: the `Ephemeral` presence payload (`PresencePayload`). CRDT snapshots/deltas are already `Vec<u8>` produced by Loro. **No postcard on the inter-thread path** (typed values on `flume`). |
| `serde` | `Serialize`/`Deserialize` **only** on postcard-serialized types bound for the network (`PresencePayload`, `CursorPosition`). IPC types (`ProseCommand`/`ProseEvent`, `PageLayout`, `RemoteCursor`, …) do not carry it (move on `flume`). |
| `thiserror` | Typed `ProseError`, mapped to `ProseEvent::Error`. |

No dependency on `ambre-sync` or `ambre-crypto`.

**Invariants:**

- `LoroDoc` is instantiated and confined to the Logic thread. It never crosses any thread boundary.
- **No `LoroMap` for body text.** All editable text is in `LoroText` (Fugue CRDT). `LoroMap`s are reserved for block metadata and out-of-text structures (annotations, styles).
- Deltas exported by `LoroDoc::export_updates()` are opaque `Vec<u8>` on the `ambre-bus` side — `ambre-prose` never inspects them after serialization.
- No `unsafe` in `ambre-prose`.
- Cursor determinism: after CRDT convergence of N peers, the absolute position of a cursor (expressed as `loro::Cursor`) is identical on all peers.
- The depth of a `ListItem` is **derived** from its position in the `MovableTree` (length of the `ListItem` ancestor chain). It is never stored in the CRDT.
- The header and footer each have a stable `TreeID` in `"meta"` (keys `"meta.header.block_id"` and `"meta.footer.block_id"`). Text editing commands (`InsertText`, `ApplyMark`, etc.) target them via this `block_id` like any other block — there is no recursive `SetHeaderContent { ops: Vec<ProseCommand> }` command.

---

## 2. TECHNICAL DESIGN & STATE

### A. Document CRDT Structure

The Prose document is a single `LoroDoc` containing four root containers:

```
LoroDoc
├── "meta"    : LoroMap         — document metadata and configuration
├── "blocks"  : LoroMovableTree — block hierarchy
├── "styles"  : LoroMap         — custom named style dictionary
└── "annots"  : LoroMap         — comments, suggestions, footnotes (prose-features)
```

#### `"meta"` — `LoroMap` (complete MVP schema)

```
meta
├── "title"               : String
├── "created_at"          : u64        — Unix timestamp ms
├── "lang"                : String     — BCP 47, e.g. "fr-FR"
├── "page"                : LoroMap    — page format
│   ├── "width_mm"        : f32
│   ├── "height_mm"       : f32
│   ├── "margin_top_mm"   : f32
│   ├── "margin_bottom_mm": f32
│   ├── "margin_left_mm"  : f32
│   └── "margin_right_mm" : f32
├── "defaults"            : LoroMap    — document default values (4th cascade level)
│   ├── "font_family"     : String     — e.g. "Arial"
│   ├── "font_size_pt"    : f32        — e.g. 11.0
│   └── "line_height"     : f32        — Multiple, e.g. 1.15
├── "header"              : LoroMap    — header (prose-features)
│   ├── "enabled"         : bool
│   ├── "block_id"        : String     — stable TreeID of the header LoroText node
│   │                                    (in "blocks"; holds inline marks + MARK_FIELD_*)
│   ├── "align"           : String     — "left"|"center"|"right"
│   └── "different_first_page" : bool
├── "header_first"        : LoroMap    — first page header (if different_first_page)
│   └── "block_id"        : String     — stable TreeID of the first page header LoroText
├── "footer"              : LoroMap    — footer (same structure as header)
│   └── "block_id"        : String
└── "footer_first"        : LoroMap    — first page footer
    └── "block_id"        : String
```

> **Post-MVP:** `meta` will also host `"version_history"` (N.1), `"line_numbers"` (N.7),
> `"pageless"` (N.11), and `"bibliography"` (N.6 — at the doc root).

#### `"blocks"` — `LoroMovableTree`

Each node represents a **block**. Nodes are identified by their `TreeID` (stable,
UUID-like, generated by Loro). Each node has a `LoroMap` of metadata:

```
Node metadata (LoroMap associated with each TreeID)
├── "t"  : u8          — BlockType discriminant (see enum below)
├── "s"  : String|null — style_id, key in "styles" or built-in style
└── "a"  : LoroMap|null — type-specific attributes (see per type)
```

```rust
// ambre-prose/src/block.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BlockType {
    Paragraph  = 0,
    Heading    = 1,  // a: {"level": u8}  -- 1..=6
    ListItem   = 2,  // a: {"ls": u8, "lo": u8|null}  -- see ListStyle below
    Table      = 3,  // a: {"col_widths": [f32], "merged": [[row,col,rs,cs]]}
    TableRow   = 4,  // no attributes
    TableCell  = 5,  // a: {"bg": str|null, "bt": f32, "bb": f32, "bl": f32, "br": f32,
                     //      "bc": str, "pad": f32, "va": u8}
    CodeBlock  = 6,  // a: {"lang": str}
    Image      = 7,  // a: {"content_hash": str, "alt": str, "w": u32, "h": u32, "align": u8, "caption": str}
    PageBreak  = 8,  // leaf node without text
    HRule      = 9,  // leaf node without text
    Toc        = 10, // a: {"max_level": u8, "numbered": bool}
}
```

**`ListItem` attribute `"a"`:**

The `"ls"` attribute encodes the `ListStyle` discriminant (0 = Bullet, 1 = Ordered, 2 = Todo).
The `"lo"` attribute encodes the `OrderedStyle` discriminant if `ls == 1` (0 = Decimal,
1 = LowerAlpha, 2 = UpperAlpha, 3 = LowerRoman, 4 = UpperRoman). The `"tc"` attribute encodes
the checked state if `ls == 2` (bool).

```
ListItem "a" :
├── "ls" : u8        — ListStyle discriminant: 0=Bullet, 1=Ordered, 2=Todo
├── "lo" : u8|null   — OrderedStyle discriminant (if ls=1)
└── "tc" : bool|null — Todo checked (if ls=2)
```

The **depth** is never stored — it is derived at read time by counting `ListItem`
ancestors in the `MovableTree`:

```rust
// ambre-prose/src/features/list.rs
pub fn compute_depth(tree: &LoroMovableTree, item_id: &TreeID) -> u8 {
    tree.ancestors(item_id)
        .filter(|id| block_type(tree, id) == BlockType::ListItem)
        .count() as u8
}
```

**Nesting rules:**

| Parent type | Allowed children |
|-------------|-------------------|
| (root) | Paragraph, Heading, ListItem, Table, CodeBlock, Image, PageBreak, HRule, Toc |
| Table | TableRow only |
| TableRow | TableCell only |
| TableCell | Paragraph, Heading, ListItem (no nested Table, no CodeBlock) |
| ListItem | ListItem (nesting), Paragraph (degenerate case) |
| All others | — (leaves) |

#### Body Text — `LoroText` (Fugue CRDT)

Each leaf node (`Paragraph`, `Heading`, `ListItem`, `TableCell`, `CodeBlock`) has a
`LoroText` accessible via the key convention `"t:{tree_id}"`:

```rust
// ambre-prose/src/doc.rs
pub fn get_block_text(doc: &LoroDoc, id: &TreeID) -> LoroText {
    doc.get_text(format!("t:{}", id.to_string()))
}
```

**Why not `LoroMap` for body text:** `LoroMap` (LWW) has no concurrent positional insertion
semantics. Two simultaneous insertions at position 5 would be in LWW conflict. `LoroText`
implements Fugue, a sequence CRDT with convergence without pathological interleaving and
stable cursors.

#### Inline Marks — Complete Source of Truth

```rust
// ambre-prose/src/mark.rs

// -- Base styles --
pub const MARK_BOLD:          &str = "b";
pub const MARK_ITALIC:        &str = "i";
pub const MARK_UNDERLINE:     &str = "u";
pub const MARK_STRIKETHROUGH: &str = "s";
pub const MARK_SUPERSCRIPT:   &str = "sup";
pub const MARK_SUBSCRIPT:     &str = "sub";
pub const MARK_CODE:          &str = "c";    // inline code

// -- Typography --
pub const MARK_LINK:          &str = "a";    // value = URL
pub const MARK_COLOR:         &str = "fg";   // value = "#rrggbb"
pub const MARK_BG:            &str = "bg";   // value = "#rrggbb"
pub const MARK_FONT_SIZE:     &str = "fs";   // value = f32 (pt)
pub const MARK_FONT_FAMILY:   &str = "ff";   // value = String

// -- Annotations (prose-features) --
pub const MARK_COMMENT:       &str = "cm";   // value = Uuid -> "annots"
pub const MARK_SUGGEST_INS:   &str = "si";   // value = Uuid -> "annots"
pub const MARK_SUGGEST_DEL:   &str = "sd";   // value = Uuid -> "annots"
pub const MARK_FOOTNOTE:      &str = "fn";   // value = Uuid -> "annots"
pub const MARK_CROSS_REF:     &str = "ref";  // value = "{target_block_id}:{kind}"

// -- Header/footer dynamic fields (prose-features) --
pub const MARK_FIELD_PAGE_NUM:   &str = "f:pn"; // current page number
pub const MARK_FIELD_PAGE_COUNT: &str = "f:pc"; // total page count
pub const MARK_FIELD_DATE:       &str = "f:dt"; // last modified date
pub const MARK_FIELD_TITLE:      &str = "f:ti"; // document title
```

**`expand_type_for` — single source of truth for all `ExpandType` policies:**

```rust
// ambre-prose/src/mark.rs
use loro::ExpandType;

pub fn expand_type_for(mark_key: &str) -> ExpandType {
    match mark_key {
        // Links: an adjacent insertion must not join the link
        MARK_LINK => ExpandType::None,

        // Annotations: never auto-extend
        MARK_COMMENT
        | MARK_SUGGEST_INS
        | MARK_SUGGEST_DEL
        | MARK_FOOTNOTE
        | MARK_CROSS_REF => ExpandType::None,

        // Dynamic fields: fixed value, no expansion
        MARK_FIELD_PAGE_NUM
        | MARK_FIELD_PAGE_COUNT
        | MARK_FIELD_DATE
        | MARK_FIELD_TITLE => ExpandType::None,

        // Base styles and typography: expand to the right
        // (continue style when typing at the end of the span)
        _ => ExpandType::After,
    }
}
```

#### `"styles"` — `LoroMap`

Dictionary of the user's **custom** named styles. Each key is a `style_id`
(`String`), the value is a `LoroMap` of properties.

Built-in styles (hardcoded client-side, not stored in `"styles"`):
`body`, `heading-1`..`heading-6`, `code`, `quote`.

The `quote` style is a `Paragraph` with `style_id = "quote"` — there is no
`BlockType::Quote`. It is a visual style (left indent, colored sidebar),
not a distinct structure.

---

### B. Pagination Model (Logic thread)

Pagination is computed in the Logic thread via Parley. The result — a `PageLayout`
— is sent to the UI thread via `ProseEvent::LayoutUpdated`.

> **Architecture references:** `ambre-spec-canvas.md` §B.2 note — Parley in the Logic
> thread covers: widows/orphans, keep-with-next on headings, image positioning,
> manual page breaks, footnotes. `ambre-spec-architecture.md` §2.D —
> pagination = business logic, Logic thread only.

#### Structures

```rust
// ambre-prose/src/pagination.rs

/// Layout computed for an individual block in the document flow.
#[derive(Debug, Clone)]
pub struct BlockLayout {
    pub block_id:    TreeID,
    pub block_type:  BlockType,
    pub height_px:   f32,       // total height (content + space_before + space_after)
    pub page_start:  u32,       // page index (0-based) of the block's start
    pub y_in_page:   f32,       // Y offset of the block top within its start page
    pub is_split:    bool,      // true if the block spans multiple pages
    /// If is_split == true: indices of Parley lines (0-based) where page breaks occur.
    /// split_points[i] = index of the first line of page i+1.
    /// Empty if is_split == false.
    pub split_points: Vec<u32>,
}

/// Global document layout -- source of truth for the UI thread and PDF export.
#[derive(Debug, Clone)]
pub struct PageLayout {
    pub page_width_px:     f32,
    pub page_height_px:    f32,
    pub content_width_px:  f32,   // page_width - margin_left - margin_right
    pub content_height_px: f32,   // page_height - margin_top - margin_bottom (excl. header/footer)
    pub page_count:        u32,
    pub blocks:            Vec<BlockLayout>,
    /// Header height. pages[0] may differ if different_first_page.
    /// 0.0 if header disabled.
    pub header_height_px:  f32,
    pub header_first_height_px: f32,
    /// Footer height.
    pub footer_height_px:  f32,
    pub footer_first_height_px: f32,
    /// Footnotes anchored per page.
    /// Key = page index, value = ordered list of footnotes on that page.
    pub footnotes_per_page: Vec<Vec<FootnoteLayout>>,
    /// Shaped runs of the header for each page. Index = page index.
    /// Shaped in the Logic thread with dynamic field resolution:
    ///   f:pn -> page number (varies per page -- reason for per-page shaping)
    ///   f:pc -> page_count (constant)
    ///   f:dt -> last modified date (constant)
    ///   f:ti -> document title (constant)
    /// pages[0] uses header_first if different_first_page.
    /// Empty if header disabled.
    pub header_runs:  Vec<Vec<ParleyRunData>>,
    /// Shaped runs of the footer for each page. Same logic as header_runs.
    pub footer_runs:  Vec<Vec<ParleyRunData>>,
}

/// Layout of a footnote placed at the bottom of a page.
#[derive(Debug, Clone)]
pub struct FootnoteLayout {
    pub footnote_id:  Uuid,
    pub ordinal:      u32,       // displayed number (sequential across the document)
    pub height_px:    f32,       // height of the footnote text (measured by Parley)
    /// Pre-shaped Parley runs of the footnote text.
    /// Shaped in Pagination Phase 0 (measuring height_px requires shaping --
    /// might as well keep the result). Includes the ordinal prefix (e.g. "1. ").
    pub runs:         Vec<ParleyRunData>,
}

/// Effective available space on a page after deducting reserved zones.
/// Internal structure, not serialized to the UI thread.
struct PageBudget {
    /// content_height_px - header_height - footer_height - footnotes_height
    available_px: f32,
    /// Space consumed by footnotes assigned to this page (grows upward).
    footnotes_px: f32,
}
```

#### Algorithm Inputs

| Source | Data | Role |
|--------|---------|------|
| `"meta.page"` (LoroMap) | `width_mm`, `height_mm`, `margin_*_mm` | Page dimensions → mm → px conversion via 96 DPI |
| `"meta.defaults"` (LoroMap) | `font_family`, `font_size_pt`, `line_height` | Parley defaults if no specific style |
| `"meta.header/footer"` (LoroMap) | `enabled`, `text` (LoroText), `different_first_page` | Reserved zone heights measured by Parley |
| `"blocks"` (LoroMovableTree) | Block hierarchy | DFS pre-order traversal → ordered block list |
| `"styles"` (LoroMap) | Custom named styles | Lookup for each block's `style_id` |
| `"annots"` (LoroMap) | Footnotes (`type: "footnote"`, `text: LoroText`) | Footnote text to measure and place |
| `ParagraphStyle` per block | `space_before`, `space_after`, `line_height`, `keep_together`, `keep_with_next`, `indent_*` | Per-block placement constraints |
| `BlockType` per block | Node type | Type-specific rules (PageBreak, Image, Table, Heading) |

#### Phase 0 — Pre-computations (once per full recomputation)

```
0.1  Convert meta.page to pixels: page_width_px, page_height_px, margins.
     Conversion: 1 mm = 96/25.4 px.
0.2  content_width_px  = page_width_px - margin_left_px - margin_right_px
0.3  content_height_px = page_height_px - margin_top_px - margin_bottom_px
0.4  If header.enabled:
       Measure header.text via Parley(content_width_px) → header_height_px.
       Keep the shaped ParleyRunData (reused in Phase 3).
       If different_first_page and header_first.text exists:
         Measure → header_first_height_px. Keep the runs.
0.5  Same for footer → footer_height_px, footer_first_height_px + runs.
0.6  Collect all footnotes in the document:
       Traverse "annots", filter type == "footnote".
       For each footnote:
         Shape text via Parley(content_width_px) → footnote_height_px + ParleyRunData.
         Keep the runs in FootnoteLayout (measurement requires shaping —
         might as well keep the result).
       Associate each footnote with its anchor_block_id and its offset in the text
       (position of the "fn" mark in the source block's LoroText).
       Sort by order of appearance in the document (DFS block order + offset).
       Assign ordinals (1, 2, 3...) in that order.
       Prefix each footnote's text with its ordinal ("1. ", "2. ", etc.)
       before shaping.
```

#### Phase 1 — Block Measurement (Parley)

```
For each block in DFS pre-order traversal of the MovableTree:

  1.1  Resolve the block's effective style:
       ParagraphStyle = cascade(inline mark, style_id, built-in, meta.defaults)
       → space_before, space_after, line_height, indent_first, indent_left,
         indent_right, keep_together, keep_with_next.

  1.2  Compute the available width for the block's content:
       block_content_width = content_width_px - indent_left - indent_right.

  1.3  Measure height via Parley depending on BlockType:

       Paragraph, Heading, ListItem, CodeBlock:
         Parley::layout(text, block_content_width, font, font_size, line_height)
         → Layout with N lines, each line has a height.
         block_content_height = sum of line heights.
         total_height = space_before + block_content_height + space_after.
         Keep the full Parley Layout (individual lines needed for
         widow/orphan splitting and shaping).

       Image (BlockType::Image, align == Block | Center):
         total_height = space_before + image_height_px + caption_height + space_after.
         caption_height = Parley::layout(caption, block_content_width) if caption non-empty.

       Image (align == Left | Right — floating):
         MVP: floating images are treated as block images.
         Post-MVP: the image reduces block_content_width of adjacent paragraphs
         until its height is exhausted (text wrapping, dedicated spec).

       Table:
         For each TableRow:
           row_height = max(Parley::layout(cell_text, cell_width) for each cell).
         total_height = space_before + sum(row_heights) + space_after.
         Keep individual row_heights (needed for table splitting).

       HRule:
         total_height = space_before + HRULE_HEIGHT (1 px) + space_after.

       PageBreak:
         total_height = 0. Treated as a signal, not a sized block.

       Toc (placeholder):
         Count qualified heading entries (level <= max_level).
         total_height = n_entries × toc_line_height + space_before + space_after.

  1.4  Store: (block_id, block_type, total_height, line_count, line_heights[],
       space_before, space_after, keep_together, keep_with_next,
       footnote_ids[] — footnotes whose mark is in this block).
```

#### Phase 2 — Page Placement (main algorithm)

The algorithm iterates over measured blocks in DFS order and assigns them to pages.
Placement is sequential and non-reversible (no global backtracking — constraints are
resolved locally by moving to the next page).

```
Mutable state:
  current_page  = 0
  cursor_y      = 0.0   (Y position within the current page)
  page_budgets  = [PageBudget::new(page=0)]

fn available_on_page(page: u32) -> f32:
  budget = content_height_px
  If page == 0: budget -= header_first_height_px + footer_first_height_px
  Else         : budget -= header_height_px + footer_height_px
  budget -= page_budgets[page].footnotes_px
  Return budget

fn new_page():
  current_page += 1
  cursor_y = 0.0
  page_budgets.push(PageBudget::new(page=current_page))

For each measured block B (in DFS order):

  ── Step 2.1: Manual page break ──────────────────────────────────────────

  If B.block_type == PageBreak:
    new_page().
    Emit BlockLayout { block_id, height: 0, page_start: current_page,
                       y_in_page: 0, is_split: false }.
    Continue to next block.

  ── Step 2.2: Footnote reservation ───────────────────────────────────────

  For each footnote_id in B.footnote_ids:
    The footnote MUST appear on the same page as the first line of the block
    that references it (or the current page if the block is split).
    Reserve footnote.height_px in page_budgets[current_page].footnotes_px.
    If the reservation reduces available_on_page() below cursor_y:
      → Conflict: the footnote doesn't fit on this page with the already placed content.
      → new_page(), then reserve the footnote on the new page.

  ── Step 2.3: Vertical margin collapse ───────────────────────────────────

  effective_space_before = if cursor_y == 0.0:
      0.0  (no margin at page top)
    else:
      max(B.space_before, previous_block.space_after)  — CSS-like collapse

  ── Step 2.4: Simple overflow test ────────────────────────────────────────

  needed = effective_space_before + B.total_height_without_spaces + B.space_after
  remaining = available_on_page(current_page) - cursor_y

  If needed <= remaining:
    → The block fits entirely on the current page.
    Go to Step 2.7 (direct placement).

  ── Step 2.5: Unbreakability constraints ─────────────────────────────────

  This block doesn't fit. Before deciding how to split or move it,
  check constraints:

  a) keep_with_next of the PREVIOUS block:
     If the previous block had keep_with_next == true and B is its immediate
     successor AND the previous block is the only thing on this page below
     a reasonable threshold (height < 25% of available_on_page):
       → Move the previous block AND B to a new page.
       → Rewrite the previous block's BlockLayout (page_start = new page).
     Else:
       → keep_with_next is relaxed (geometric impossibility — the previous block
         is too large to move). Continue normally.

  b) Heading (implicitly keep_with_next):
     If B.block_type == Heading:
       → Apply the same logic as keep_with_next: a heading must not remain
         alone at the bottom of a page without its first content block.
       If the heading fits on the current page but the NEXT block won't fit
       (there is less than MIN_CONTENT_AFTER_HEADING = 2 lines of space left):
         → Move the heading to the next page.

  c) keep_together:
     If B.keep_together == true and B.line_count > 1:
       → Do not split this block. Push it entirely to a new page.
       If B.total_height > available_on_page(current_page + 1):
         → The block is larger than an entire page. keep_together is relaxed,
           treated as a normal splittable block (geometric impossibility).

  ── Step 2.6: Block split (widows, orphans, tables) ──────────────────────

  The block overflows and is not protected by keep_together, or keep_together
  was relaxed. Determine the split point.

  a) Widows and orphans (Paragraph, Heading, ListItem, CodeBlock):

     Constants:
       MIN_ORPHAN_LINES = 2   (minimum lines BEFORE the split, on the current page)
       MIN_WIDOW_LINES  = 2   (minimum lines AFTER the split, on the next page)

     Find the optimal split line:
       max_lines_on_current = number of lines that fit in `remaining`.
       If max_lines_on_current < MIN_ORPHAN_LINES:
         → Orphan: fewer than 2 lines would remain at the bottom of the page.
         → Push the entire block to the next page (new_page()).
         → If the block also exceeds the next page, re-split from the start
           of the new page with the same rules.
       Else:
         lines_remaining_after_split = B.line_count - max_lines_on_current
         If lines_remaining_after_split < MIN_WIDOW_LINES:
           → Widow: fewer than 2 lines would end up at the top of the next page.
           → Move the split back: split_line = B.line_count - MIN_WIDOW_LINES.
           → If split_line < MIN_ORPHAN_LINES:
             → Impossible to satisfy both constraints simultaneously.
             → Priority: push the entire block to the next page.
             → If the block exceeds a full page: relax, split at
               max_lines_on_current (the block is too long for the constraints).
         Else:
           split_line = max_lines_on_current.

     Emit BlockLayout with is_split = true, split_points = [split_line].
     Place lines [0..split_line) on current_page.
     new_page().
     Place lines [split_line..] on the new page.
     If the remaining lines still overflow the page: iterate the split
     (loop, each subsequent page splits in turn with the same rules).

  b) Tables:

     Tables are split by TABLE ROW (not by text line).
     No cell is cut in half — the entire row moves to the next page.
     If a single table row exceeds a full page: degenerate case,
     the row is placed and overflows (truncated display with warning).

     If header_row == true (TableStyle): the first row is repeated
     at the top of each page where the table continues (post-MVP).

  c) Images (align == Block):

     Images are never split. If the image doesn't fit on the current
     page, it is pushed to the next page.
     If the image is taller than an entire page: it is placed alone
     on its page, visually overflowing (UI thread clips to page frame).

  ── Step 2.7: Effective placement ─────────────────────────────────────────

  Emit BlockLayout {
      block_id:     B.block_id,
      block_type:   B.block_type,
      height_px:    B.total_height (portion on this page if split),
      page_start:   current_page,
      y_in_page:    cursor_y + effective_space_before,
      is_split:     false (or true if split in step 2.6),
      split_points: [] (or computed indices),
  }
  cursor_y += effective_space_before + B.height_on_this_page.
```

#### Phase 3 — Finalization (footnotes + header/footer runs)

```
3.A  Footnote placement:

Footnotes were reserved in Phase 2.2 on their respective pages.
Their final Y position is computed:

For each page P:
  footnotes_on_P = footnotes_per_page[P], sorted by increasing ordinal.
  footnote_area_top = available_on_page(P) - sum(fn.height_px for fn in footnotes_on_P).
  If footnotes_on_P non-empty:
    Add a horizontal separator (thin line, 1/3 of content_width_px)
    above the first footnote (fixed height = FOOTNOTE_SEP_HEIGHT = 8 px).
  Position each footnote sequentially starting from footnote_area_top + SEP.
  (Each footnote's ParleyRunData is already in FootnoteLayout.runs
   from Phase 0 — nothing to re-shape here.)

3.B  Per-page header/footer shaping:

The header and footer may contain dynamic fields whose value varies per
page (f:pn = page number). Constant fields (f:pc, f:dt, f:ti) are resolved once.

For each page P (0..page_count):
  1. Determine the variant: if P == 0 and different_first_page → header_first / footer_first.
     Otherwise → standard header / footer.
  2. Resolve dynamic fields in the source LoroText:
       f:pn → (P + 1).to_string()
       f:pc → page_count.to_string()
       f:dt → format_date(meta.updated_at)  (resolved once, constant)
       f:ti → meta.title                    (resolved once, constant)
  3. Shape the resolved text via Parley(content_width_px) → Vec<ParleyRunData>.
  4. Store in header_runs[P] and footer_runs[P].

Optimization: if the header/footer does not contain f:pn (the only per-page
variable field), shaping is done once and the result is cloned for all pages.
Total cost: ~10 µs × page_count if f:pn present, ~10 µs if absent.
Negligible within the 20 ms budget for 100 pages.
```

#### Constraint Priority Order

When multiple constraints conflict (a block is too large to satisfy all rules),
the algorithm relaxes constraints in this order (from highest to lowest priority):

| Priority | Constraint | Relaxation |
|----------|-----------|-------------|
| 1 (absolute) | **Manual page break** (`PageBreak`) | Never relaxed — always honored. |
| 2 (absolute) | **Footnotes on the reference page** | If the footnote doesn't fit: it overflows to the next page with a "continued" marker. Post-MVP. MVP: the footnote pushes the content. |
| 3 | **Orphans** (min 2 lines at page bottom) | Relaxed if the block is ≤ 3 lines AND overflows the page (geometric contradiction). |
| 4 | **Widows** (min 2 lines at next page top) | Relaxed if satisfying widows violates orphans and the block doesn't fit on a page. |
| 5 | **keep_together** | Relaxed if the block is larger than an entire page (geometric impossibility). |
| 6 | **keep_with_next** / unbreakable Heading | Relaxed if the previous block (or heading) occupies > 25% of the page (excessive displacement). |
| 7 | **Margin collapse** | Never relaxed — always applied. |

#### Special Cases

**Headers and footers:** the header and footer reduce the available space on each
page. The first page may have different values (`different_first_page`).
Headers/footers are not blocks in the `MovableTree` — their height is measured
once in Phase 0 and subtracted from each page's budget. Header/footer rendering
is handled by `prose-ui` directly from `"meta.header/footer"` data.

**TOC (placeholder):** the `Toc` block is a placeholder whose height is estimated
from the number of qualified headings × `toc_line_height`. After placement, if the
page count changes (which modifies page numbers in the TOC), a second stabilization
pass is triggered (max 3 iterations for convergence — beyond that, accept the
approximation).

**Floating images (Left/Right) — MVP:** treated as block images. Text wrapping is
deferred post-MVP (requires a line-breaking algorithm with exclusion zones,
extension of Parley).

**Nested lists:** `ListItem` with `ListItem` children are measured recursively.
Each sub-item is an independent block for pagination — a parent `ListItem` has no
implicit keep_together on all its children (identical to Google Docs).

**Merged tables:** merged cells (`merged`) are attributed to the first row of the
merge. The row height is the max between normal cells and the visible portion of
the merged cell for that row.

#### Incremental Invalidation

Full recomputation of 100 pages takes < 20 ms (P95 mobile target). Incremental
invalidation optimizes frequent cases (keystroke, insertion, deletion):

```
1. Identify the modified block (CRDT delta → impacted block_id).
2. Re-measure only this block via Parley (Phase 1, ~1 ms).
3. If the block height hasn't changed:
     → No placement recalculation needed. Emit RunsUpdated only.
4. If the height changed:
     → Recalculate Phase 2 from this block to the end of the document.
     → Blocks preceding the modification keep their BlockLayout unchanged.
5. Emit LayoutUpdated with first_dirty_page = page of the modified block.
```

**Cases forcing a full recomputation (Phase 0 + 1 + 2):**
- Page dimension change (`meta.page`).
- Header or footer enable/disable.
- `meta.defaults` change (font_family, font_size, line_height).
- Full snapshot import (`RemoteSnapshot`).

---

### C. Interface Contract (API)

#### `ProseState` — typed document state

`ProseState` is a concrete state, stored in `DocState::Prose(ProseState)` and dispatched
via `match` in the Logic thread (`ambre-logic`). Its public surface mirrors `TrameState`:

- `handle_command(ProseCommand) -> ProseOutcome` — content command (UI → Logic).
- `apply_remote(&[u8]) -> Vec<ProseEvent>` — remote CRDT delta/snapshot to import.
- `apply_ephemeral(from_user_id: UserId, &[u8]) -> Vec<ProseEvent>` — remote `Ephemeral` (cursor) payload; the author identity is the **Sync-resolved `from_user_id`**, never trusted from the payload.
- `export_snapshot() -> Vec<u8>` — on `SnapshotNeeded`.

The generic dispatcher in `ambre-logic` routes incoming `LogicEvent`s to these
methods and **explodes the outcome** (display → `UiEvent::Viewport`, `update`/`snapshot`
→ `SyncCommand::SendUpdate`/`SendSnapshot`). `ProseState` never names `SyncCommand`,
`LogicEvent`, or any `flume` channel.

```rust
// ambre-prose/src/logic.rs
use ambre_bus::{AppEvent, ProseCommand, ProseEvent};
use loro::LoroDoc;

pub struct ProseState {
    doc:              LoroDoc,
    layout:           Option<PageLayout>,
    layout_version:   u64,               // incremented on each pagination recomputation
    tracking:         bool,              // track changes active
    media_cache:      HashMap<String, Vec<u8>>,  // content_hash -> decrypted image bytes
}

/// Result of a content command. The Logic dispatcher explodes it:
/// `display` -> AppEvent::Prose -> UiEvent::Viewport; `update` -> SyncCommand::SendUpdate;
/// `ephemeral` -> SyncCommand::SendEphemeral (adding the document_id held by Logic).
pub struct ProseOutcome {
    pub display:   Vec<ProseEvent>,
    pub update:    Option<Vec<u8>>,   // raw Loro delta (export_updates)
    pub ephemeral: Option<Vec<u8>>,   // PresencePayload postcard (local presence)
}

impl ProseState {
    /// Content command from the UI thread (already typed -- no deserialization).
    /// A single `ProseCommand` can produce multiple display events
    /// (e.g. `DocUpdated` + `LayoutUpdated` + `RunsUpdated` after a keystroke) AND a delta.
    pub fn handle_command(&mut self, command: ProseCommand) -> ProseOutcome {
        let display = self.dispatch(command);   // Loro mutation + display events
        ProseOutcome {
            display,
            update: Some(self.doc.export_updates()), // delta of the mutation -> broadcast by Logic
            ephemeral: None,
        }
    }

    /// Remote CRDT delta OR snapshot (the Logic dispatcher calls this on
    /// `RemoteUpdate`/`RemoteSnapshot` after routing by `document_id`).
    pub fn apply_remote(&mut self, payload: &[u8]) -> Vec<ProseEvent> {
        self.doc.import(payload).ok();
        self.invalidate_layout_from_delta(payload);
        self.produce_layout_events() // -> [DocUpdated, LayoutUpdated, RunsUpdated]
    }

    /// Remote `Ephemeral` payload (already decrypted by `ambre-sync`). The author
    /// identity is the **Sync-resolved `from_user_id`** (spoof-proof, `ambre-spec-sync.md §3`):
    /// the peer-authored payload carries only the cursor position and a self-asserted,
    /// cosmetic `display_name` -- never a `user_id`. The canonical id and the peer color
    /// are derived here from `from_user_id`.
    pub fn apply_ephemeral(&mut self, from_user_id: UserId, payload: &[u8]) -> Vec<ProseEvent> {
        match postcard::from_bytes::<PresencePayload>(payload) {
            Ok(p) => vec![ProseEvent::RemoteCursors { cursors: vec![RemoteCursor {
                user_id:      from_user_id,
                display_name: p.display_name,
                color:        ambre_draw::peer::peer_color(&from_user_id), // shared cross-app helper (canvas §D)
                position:     p.position,
            }] }],
            Err(_) => vec![], // non-Prose Ephemeral payload (ignored)
        }
    }

    /// On `SnapshotNeeded`: the Logic dispatcher then emits `SyncCommand::SendSnapshot`.
    pub fn export_snapshot(&self) -> Vec<u8> {
        self.doc.export_snapshot()
    }

    /// Decrypted media arrived (dedicated Logic route): local cache + visual invalidation.
    pub fn on_media_downloaded(&mut self, content_hash: String, data: Vec<u8>) -> Vec<ProseEvent> {
        self.media_cache.insert(content_hash, data);
        vec![ProseEvent::DocUpdated { delta: vec![] }] // empty delta = invalidation without mutation
    }
}
```

> **`ConnectionStatus`, `SyncWarning`, `MemberRevoked`, sharing, uploaded media**: are not Prose content — the `ambre-logic` dispatcher routes them directly as `UiEvent` (online/offline indicator, `UiEvent::SyncWarning`, etc.) without going through `ProseState`.

```rust
impl ProseState {
    /// Produces the 3 standard events after any LoroDoc mutation (local or remote).
    fn produce_layout_events(&mut self) -> Vec<ProseEvent> {
        let delta = self.doc.export_updates();  // raw Loro delta
        let layout = self.recalculate_pagination();
        let runs = self.shape_dirty_blocks();
        self.layout_version += 1;
        vec![
            ProseEvent::DocUpdated { delta },
            ProseEvent::LayoutUpdated {
                layout,
                first_dirty_page: 0, // full recomputation for RemoteUpdate/Snapshot
                version: self.layout_version,
            },
            ProseEvent::RunsUpdated {
                runs,
                version: self.layout_version,
            },
        ]
    }
}
```

#### `ProseCommand` — IPC Types (UI thread → Logic thread)

```rust
// ambre-prose/src/events.rs

// IPC content plane (ambre-bus) -- move on flume, no serde.
#[derive(Debug, Clone)]
pub enum ProseCommand {
    Noop,

    // -- Text editing --
    InsertText    { block_id: TreeID, pos: u32, text: String },
    DeleteText    { block_id: TreeID, range: (u32, u32) },
    ApplyMark     { block_id: TreeID, range: (u32, u32), key: String, value: Option<String> },
    RemoveMark    { block_id: TreeID, range: (u32, u32), key: String },

    // -- Block structure --
    InsertBlock   { parent_id: Option<TreeID>, after_id: Option<TreeID>,
                    block_type: BlockType, style_id: Option<String> },
    DeleteBlock   { block_id: TreeID },
    MoveBlock     { block_id: TreeID, new_parent: Option<TreeID>, after_id: Option<TreeID> },
    SetBlockStyle { block_id: TreeID, style_id: String },

    // -- Paragraph styles --
    SetParagraphStyle { block_id: TreeID, style: ParagraphStyle },
    ClearInlineMarks  { block_id: TreeID, range: (u32, u32) },

    // -- Lists --
    InsertListItem { parent_id: Option<TreeID>, after_id: Option<TreeID>, style: ListStyle },
    SetListStyle   { block_id: TreeID, style: ListStyle },
    IndentList     { block_id: TreeID },
    UnindentList   { block_id: TreeID },

    // -- Tables --
    InsertTableRow   { table_id: TreeID, after_row: Option<TreeID> },
    DeleteTableRow   { row_id: TreeID },
    InsertTableCol   { table_id: TreeID, after_col: u32 },
    DeleteTableCol   { table_id: TreeID, col_index: u32 },
    ResizeTableCol   { table_id: TreeID, col_index: u32, width_pt: f32 },
    MergeTableCells  { table_id: TreeID, start: (u32, u32), end: (u32, u32) },
    UnmergeTableCell { cell_id: TreeID },
    SetCellStyle     { cell_id: TreeID, bg: Option<u32>, valign: Option<VAlign>,
                       border: Option<CellBorder>, padding_pt: Option<f32> },
    SetTableStyle    { table_id: TreeID, style: TableStyle },

    // -- TOC --
    InsertToc       { parent_id: Option<TreeID>, after_id: Option<TreeID>,
                      max_level: u8, numbered: bool },
    UpdateTocConfig { block_id: TreeID, max_level: u8, numbered: bool },

    // -- Images --
    /// Insert an image. Attributes are typed -- no recourse to stringly-typed SetBlockAttr.
    InsertImage      { parent_id: Option<TreeID>, after_id: Option<TreeID>,
                       content_hash: String, alt: String, width_px: u32, height_px: u32 },
    ResizeImage      { block_id: TreeID, width_px: u32, height_px: u32 },
    SetImageAlign    { block_id: TreeID, align: ImageAlign },
    SetImageAlt      { block_id: TreeID, alt: String },
    SetImageCaption  { block_id: TreeID, caption: String },

    // -- Headers / footers --
    /// The header and footer each have a stable TreeID in "meta".
    /// All text editing commands (InsertText, ApplyMark, etc.)
    /// target them directly via this block_id -- no special variant.
    SetHeaderEnabled      { enabled: bool },
    SetFooterEnabled      { enabled: bool },
    InsertHeaderField     { field: HeaderField, pos: u32 },
    InsertFooterField     { field: HeaderField, pos: u32 },
    SetDifferentFirstPage { enabled: bool },

    // -- Comments --
    AddComment       { block_id: TreeID, range: (u32, u32), text: String },
    ReplyToComment   { comment_id: Uuid, text: String },
    ResolveComment   { comment_id: Uuid },
    DeleteComment    { comment_id: Uuid },
    EditCommentReply { comment_id: Uuid, reply_id: Uuid, new_text: String },

    // -- Track Changes --
    EnableTrackChanges   { enabled: bool },
    AcceptSuggestion     { suggestion_id: Uuid },
    RejectSuggestion     { suggestion_id: Uuid },
    AcceptAllSuggestions,
    RejectAllSuggestions,

    // -- Footnotes & cross references --
    InsertFootnote  { block_id: TreeID, pos: u32, note_text: String },
    EditFootnote    { footnote_id: Uuid, new_text: String },
    DeleteFootnote  { footnote_id: Uuid },
    InsertCrossRef  { block_id: TreeID, pos: u32, target_block_id: TreeID, kind: CrossRefKind },

    // -- Find & Replace --
    FindStart  { query: FindQuery },
    FindNext,
    FindPrev,
    FindClose,
    ReplaceOne { replacement: String },
    ReplaceAll { replacement: String },

    // -- Clipboard (host-driven, Logic-authoritative -- see prose-ui K / ambre-spec-ui.md 2.E) --
    /// Build the full-fidelity clipboard payload from the CRDT selection.
    /// The Logic dispatcher replies with ProseEvent::Clipboard { text, ambre }.
    CopySelection { anchor: (TreeID, u32), focus: (TreeID, u32) },
    /// Paste an opaque x-ambre payload (postcard of a typed Prose clipboard payload).
    PasteRich     { block_id: TreeID, pos: u32, payload: Vec<u8> },

    // -- Import / Export --
    ImportDocument { format: ImportFormat, data: Vec<u8> },
    ExportDocument { format: ExportFormat, options: ExportOptions },

    // -- Collaborative cursor (ephemeral) --
    /// Local presence. The handler builds the wire `PresencePayload { position: pos,
    /// display_name: <local session name> }` and returns it as `outcome.ephemeral`
    /// (postcard) -> SyncCommand::SendEphemeral. No `user_id` is ever put on the wire.
    UpdateCursor { pos: CursorPosition },

    // -- Undo / Redo --
    Undo,
    Redo,
}
```

#### `ProseEvent` — Single Source of Truth (Logic thread → UI Thread)

```rust
// ambre-prose/src/events.rs

// IPC content plane (ambre-bus) -- move on flume, no serde.
#[derive(Debug, Clone)]
pub enum ProseEvent {
    // -- Engine --
    /// Confirmed CRDT delta (local or imported remote).
    DocUpdated    { delta: Vec<u8> },
    /// Layout recomputed after mutation or remote import.
    /// `version` allows the UI thread to drop obsolete layouts (anti-flickering).
    LayoutUpdated { layout: PageLayout, first_dirty_page: u32, version: u64 },
    /// Pre-shaped Parley runs for modified blocks -- cached in RunCache (prose-ui).
    /// `version` aligned with the corresponding LayoutUpdated.
    RunsUpdated   { runs: Vec<(TreeID, Vec<ParleyRunData>)>, version: u64 },
    /// Decrypted remote cursors (from Ephemeral).
    RemoteCursors { cursors: Vec<RemoteCursor> },
    /// Full-fidelity clipboard payload built from the CRDT selection (reply to CopySelection).
    /// `text` = text/plain mirror; `ambre` = opaque x-ambre bytes. The UI stashes it and the
    /// host mirrors it to the OS clipboard (poll_clipboard, ambre-spec-ui.md 2.E).
    Clipboard { text: String, ambre: Vec<u8> },

    // -- Features --
    /// Recalculated TOC (triggered by LayoutUpdated).
    TocUpdated    { toc: TableOfContents },
    /// Word count updated (debounced 500ms).
    WordCountUpdated { count: WordCount },
    /// Find search results.
    FindResults   { matches: Vec<FindMatch>, current: usize },
    /// Navigation within results (FindNext/FindPrev).
    FindUpdated   { current: usize },
    /// Spelling errors for a block (post-MVP).
    SpellErrors   { block_id: TreeID, errors: Vec<SpellError> },

    // -- Interop --
    /// Import completed (success or with warnings).
    ImportCompleted { report: ConversionReport },
    /// Progress of an import (large documents > 1 MB).
    ImportProgress  { percent: u8 },
    /// Export completed -- data ready for download.
    ExportReady     { format: ExportFormat, data: Vec<u8>, report: ConversionReport },
    /// Progress of an export.
    ExportProgress  { percent: u8 },

    // -- Error --
    Error { code: u16, message: String },
}
```

> **Note:** `ParleyRunData` is a serializable type that encapsulates the Parley run data
> needed for rendering (glyphs, positions, metrics via Parley). It is defined in `prose-ui`
> but referenced here in `ProseEvent` — accepted because all these modules are in `ambre-prose`.
> The types `TableOfContents`, `WordCount`, `FindMatch`, `SpellError`, `ConversionReport`,
> `ExportFormat` are defined in their respective modules (`prose-features`, `prose-interop`)
> and imported here.

#### `PresencePayload`, `CursorPosition`, and `RemoteCursor`

> ⚠️ **`PresencePayload` and `CursorPosition` keep `serde`**: they are the **postcard-serialized** `Ephemeral` payload that transits over the network (encrypted by `ambre-sync`) — real network bytes. **`RemoteCursor` does NOT** carry `serde`: it is a Logic → UI event payload (content plane, moved on `flume`), built from the Sync-resolved `from_user_id`.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorPosition {
    pub block_id: TreeID,
    pub offset:   u32,
    pub anchor:   Option<(TreeID, u32)>, // Some if selection
}

/// Peer-authored `Ephemeral` wire payload. Carries ONLY position + a self-asserted,
/// cosmetic display name. It deliberately does NOT carry a `user_id`: the canonical
/// identity is the Sync-resolved `from_user_id` delivered out-of-band on
/// `LogicEvent::RemoteEphemeral` (a peer cannot forge another user's id).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresencePayload {
    pub position:     CursorPosition,
    pub display_name: String,   // self-asserted, cosmetic (never trusted for identity)
}

/// Logic -> UI event payload (NO serde -- moved by value on `flume`).
/// Assembled in `apply_ephemeral` from the trusted `from_user_id` + the wire payload.
#[derive(Debug, Clone)]
pub struct RemoteCursor {
    /// Canonical user identifier -- stable, per-user, not per-device.
    /// Set to the Sync-resolved `from_user_id`; NEVER read from the peer payload.
    pub user_id:      UserId,
    pub display_name: String,
    pub color:        peniko::Color, // shared cross-app peer color (ambre_draw::peer::peer_color)
    pub position:     CursorPosition,
}
```

#### Data Flow

Inter-thread transport: **`flume` channels** (typed Rust values by move, shared memory — no `postMessage`, no postcard on this path). The `ambre-logic` dispatcher routes by `DocumentId` to `ProseState` and explodes the typed returns.

```
UI Thread
  │  LogicCommand::App { document_id, command: AppCommand::Prose(ProseCommand) }   (flume, typed)
  │  ───────────────────────────────────────────────────────────────────────────────────────────►
  │                                                                                  Logic Thread
  │                                                    match doc_state {
  │                                                      DocState::Prose(s) => {
  │                                                        let out = s.handle_command(cmd);
  │                                                        emit AppEvent::Prose(out.display);
  │                                                        if let Some(delta) = out.update {
  │                                                          emit SyncCommand::SendUpdate { .. };
  │                                                        }
  │                                                        if let Some(eph) = out.ephemeral {
  │                                                          emit SyncCommand::SendEphemeral { .. };
  │                                                        }
  │                                                      }}
  │                                                    }
  │  ◄───────────────────────────────────────────────────────────────────────────────────────────
  │  UiEvent::Viewport { event: AppEvent::Prose(ProseEvent) }   (flume, typed)

Sync Thread
  │  LogicEvent::RemoteUpdate { document_id, payload }   (flume, typed)
  │  ───────────────────────────────────────────────────────────────────────────────────────────►
  │                                                                                  Logic Thread
  │                                                    match doc_state {
  │                                                      DocState::Prose(s) => {
  │                                                        emit AppEvent::Prose(
  │                                                          s.apply_remote(&payload)
  │                                                        );
  │                                                      }}
  │  ◄───────────────────────────────────────────────────────────────────────────────────────────
  │  SyncCommand::SendUpdate { document_id, payload }   (flume, typed)
```

---

### D. Persistence Schema

```rust
// ambre-prose/src/doc.rs
impl ProseState {
    /// Raw Loro snapshot -- zstd compression + XChaCha20 encryption applied by ambre-sync.
    pub fn export_snapshot(&self) -> Vec<u8> {
        self.doc.export_snapshot()
    }

    pub fn import_snapshot(&mut self, snapshot: &[u8]) -> Result<(), ProseError> {
        self.doc.import(snapshot).map_err(ProseError::LoroCrdt)
    }
}
```

---

## 3. SECURITY & CONFIDENTIALITY (ZERO-KNOWLEDGE)

| Flow | Data | Encryption |
|------|---------|-------------|
| Logic thread → Sync thread (`SyncCommand::SendUpdate`) | Raw Loro delta plaintext | XChaCha20-Poly1305 + Ed25519 in `ambre-sync` |
| Logic thread → Sync thread (`SyncCommand::SendEphemeral`) | Serialized `PresencePayload` (position + self-asserted name; no `user_id`) | Encrypted in `ambre-sync` → `Ephemeral.ciphertext`; author identity added by Sync as `from_user_id` on the inbound path |
| Logic thread → UI thread (`ProseEvent`) | `PageLayout`, Parley runs, TOC — shared WASM memory | No exposure outside the browser process |

`ambre-prose` never contacts the network, never imports `ambre-crypto`, and never sends
plaintext to `ambre-server`.

---

## 4. QUALITY & TESTABILITY

**Unit tests (`ambre-prose/tests/`)** — native (`cfg(not(target_arch = "wasm32"))`):

| Case | Method |
|-----|---------|
| CRDT convergence: 2 peers edit the same `LoroText` | Cross `LoroDoc::import()`, assertion on identical `to_string()` |
| Cursor stability after remote edit | `loro::Cursor` encode + decode after delta import |
| Marks: overlap, expansion, partial removal | `get_text().iter_rich_text()` |
| `expand_type_for`: coverage of all marks | Exhaustive test on each constant |
| `compute_depth`: ListItem nested 3 levels | Assertions on mock MovableTree |
| `ListItem` "a": round-trip `ListStyle` → CRDT → `ListStyle` | Loro JSON serialization |
| MovableTree nesting: insert, move, delete | DFS traversal + structure check |
| Nesting rule: TableCell does not contain a Table | Assertion on rejected `InsertBlock` |
| Pagination: block that fits on a page | Mock Parley fixed heights, assertion page_start + y_in_page |
| Pagination: manual page break (`PageBreak`) | Block after PageBreak → page_start == previous page + 1 |
| Pagination: orphans (< 2 lines at bottom) | Block 5 lines, remaining = 1 line → entire block on next page |
| Pagination: widows (< 2 lines at top) | Block 10 lines, remaining = 8 → split_line = 8, not 9 (keep 2 at next page top) |
| Pagination: keep_together respected | Block 15 lines with keep_together, remaining = 10 → entire block next page |
| Pagination: keep_together relaxed (block > page) | Block 80 lines keep_together, page = 50 lines → normal split |
| Pagination: keep_with_next (heading + paragraph) | Heading at page bottom, following paragraph overflows → heading moved |
| Pagination: implicit unbreakable heading | Heading with < 2 lines of space after → heading to next page |
| Pagination: footnote reserves bottom-of-page space | Block with "fn" mark, footnote height = 30px → available reduced by 30 + SEP |
| Pagination: footnote forces new_page if conflict | Footnote doesn't fit on current page → new_page + reservation |
| Pagination: header/footer reduce budget | Header 40px + footer 20px → available = content_height - 60 |
| Pagination: different_first_page | Page 0 header_first 60px, pages 1+ header 40px → different budgets |
| Pagination: table split by row | Table 20 rows, page = 15 rows → 15 rows page 1, 5 rows page 2 |
| Pagination: block image not split | Image 800px, remaining = 200px → entire image on next page |
| Pagination: vertical margin collapse | Block A space_after=20, block B space_before=30 → effective space = 30 (max) |
| Pagination: no margin at page top | First block of a page → space_before ignored |
| Pagination: incremental invalidation (unchanged height) | Modify text without changing height → no placement recalculation |
| Pagination: incremental invalidation (changed height) | Add line → recalculate from this block, previous blocks unchanged |
| Pagination: TOC placeholder estimated height | 10 headings × toc_line_height → correct estimated height |
| `BlockType::Toc`: allowed at root | Nesting rule |
| Round-trip postcard of the Prose **network payload** (`PresencePayload` / `CursorPosition`) | `assert_eq!` after double serialization (these are the only postcard types — `ProseCommand`/`ProseEvent`/`RemoteCursor` move typed on `flume`, nothing to serialize) |

**Prop-tests (proptest):**

```rust
proptest! {
    #[test]
    fn crdt_convergence(ops in vec(arb_prose_command(), 1..50)) {
        let mut doc_a = make_doc("peer-a");
        let mut doc_b = make_doc("peer-b");
        apply_and_sync(&mut doc_a, &mut doc_b, &ops);
        assert_eq!(
            doc_a.get_text("t:root").to_string(),
            doc_b.get_text("t:root").to_string()
        );
    }
}

proptest! {
    #[test]
    fn expand_type_no_panic(key in "\\PC{1,20}") {
        // expand_type_for never panics regardless of the key
        let _ = expand_type_for(&key);
    }
}
```

**Benchmarks (criterion):**

| Benchmark | Target |
|-----------|-------|
| `LoroDoc::export_updates()` — doc 100 pages | < 5 ms |
| `LoroDoc::import()` — delta 1 KB | < 2 ms |
| `recalculate_pagination()` — 100 pages, mocked heights | < 20 ms |
| `recalculate_pagination()` — 100 pages, 20 footnotes, header+footer | < 25 ms |
| `recalculate_pagination()` — partial invalidation 10 pages | < 3 ms |
| `postcard::to_allocvec(CursorPosition)` — network `Ephemeral` payload | < 10 µs |

---

## 5. PERFORMANCE & RESOURCES

| Operation | Target | Thread |
|-----------|-------|--------|
| `InsertText` local: Loro txn + export delta | < 2 ms | Logic |
| `import()` remote delta 1 KB | < 2 ms | Logic |
| `recalculate_pagination()` total invalidation 100 pages | < 20 ms | Logic |
| `recalculate_pagination()` partial invalidation (1 block, 10 pages) | < 3 ms | Logic |
| `shape_dirty_blocks()` — 10 modified blocks | < 5 ms | Logic |
| `PageLayout` handoff 100 pages (`flume` by move, no serialization) | < 1 ms | Logic |
| `export_snapshot()` doc 100 pages | < 10 ms | Logic |

**Memory:**
- `LoroDoc` Prose: < 10 MB for a typical document (~100 pages, ~50,000 words).
- `PageLayout` (100 pages, ~300 blocks, ~20 footnotes, header/footer runs): < 200 KB.

**WASM size (`ambre-prose` excluding `loro` and `parley`):**

| Component | Raw estimated | Gzip estimated |
|-----------|-----------|-------------|
| `ambre-prose` (engine + features + interop + ui) | ~230 KB | ~72 KB |
| `loro` (shared) | ~400 KB | ~120 KB |
| `parley` (shared with other apps) | ~200 KB | ~60 KB |

---

## 6. REJECTED ARCHITECTURAL DECISIONS

| Alternative | Rejected because |
|-------------|-------------|
| **`LoroMap` for body text** | LWW: two concurrent insertions at position N → conflict. `LoroText` (Fugue) guarantees positional convergence. |
| **`LoroList<char>`** | Correct convergence but `LoroText` exposes marks natively. Reinventing rich text on `LoroList` is unnecessary. |
| **`LoroMovableTree` for inline marks** | Overlapping marks cannot be modeled in a tree. `LoroText::mark()` is the intended API. |
| **Block hierarchy in `LoroList` (flat)** | `MoveBlock` without recreation impossible. `LoroMovableTree` guarantees stable `TreeID` after `move_to()`. |
| **`depth` stored in the CRDT** | Redundant with position in the `MovableTree`, possible divergence after concurrent `MoveBlock` CRDT. Derived at read time. |
| **`SetBlockAttr { key: String, value: String }`** | Stringly-typed, impossible to validate statically. Each block type with attributes now has a dedicated typed command (`InsertImage`, `SetCellStyle`, `SetTableStyle`, `SetParagraphStyle`). |
| **`ordered: bool` in `ListItem` CRDT** | Cannot encode `Todo` or `LowerRoman`. Replaced by the full `ListStyle` discriminant. |
| **`ProseEvent` spread across multiple enums** | `ambre-prose` is a single crate — centralizing in a single enum is simpler and avoids any module visibility issues. |
| **`BlockType::Quote`** | Quote is a visual style (indentation, sidebar), not a distinct structure. `Paragraph { style_id: "quote" }` is consistent with the Google Docs model. |
| **Pagination in the UI thread** | `requestAnimationFrame` (16ms). A full recomputation (>20ms) would block rendering. Logic thread mandatory. |
| **Pagination in the Sync thread** | Does not know Parley, reserved for crypto/network. |
| **`contentEditable` / Slate.js / ProseMirror** | Rejected globally: outside Rust-first + wgpu/DrawSurface vision. |
| **Loro branches for track changes** | Full `LoroDoc` fork = memory overhead O(doc_size × suggestions). `MarkType si/sd` is O(1). |

---

## 7. INTEGRATION PLAN (ROADMAP)

**Phase 0 — Core CRDT (Week 1)**
- [ ] Definition of `BlockType` (with `Toc`), `ProseCommand`, `ProseEvent` (complete enum), `CursorPosition`.
- [ ] `LoroDoc` initialization with the four root containers and complete `meta` schema.
- [ ] `expand_type_for` covering all marks.
- [ ] Text operations: `InsertText`, `DeleteText`, `ApplyMark`, `RemoveMark`.
- [ ] Block operations: `InsertBlock`, `DeleteBlock`, `MoveBlock`, nesting rule.
- [ ] `compute_depth` for `ListItem`s.
- [ ] Unit tests CRDT convergence, exhaustive `expand_type_for`, `compute_depth`.
- [ ] Round-trip postcard of the Prose **network payload** (`PresencePayload` in `Ephemeral`). (`ProseCommand`/`ProseEvent`/`RemoteCursor` travel typed on `flume` — nothing to serialize.)

**Phase 1 — State & Pagination (Week 2)**
- [ ] `ProseState`: typed surface `handle_command` / `apply_remote` / `apply_ephemeral` / `export_snapshot` (+ `ProseOutcome`), stored in `DocState::Prose`.
- [ ] Wiring to the `ambre-logic` dispatcher (`AppCommand::Prose` variant, `match`; outcome exploded to UI/Sync).
- [ ] Pagination Phase 0: pre-computations (dimensions, header/footer measured by Parley).
- [ ] Pagination Phase 1: measurement of all BlockTypes via Parley (paragraph, heading, table, image, code, toc placeholder).
- [ ] Pagination Phase 2: sequential placement with constraints (PageBreak, orphans, widows, keep_together, keep_with_next, unbreakable heading, footnotes, margin collapse).
- [ ] Pagination Phase 3: footnote placement at page bottom.
- [ ] Incremental layout invalidation (re-measure modified block, recalculate from that block).
- [ ] `shape_dirty_blocks()` → `ProseEvent::RunsUpdated`.
- [ ] Host tests of `ProseState` with mock `flume` channels (pure dispatch, x86_64 — like `ambre-logic` tests).
- [ ] Exhaustive pagination unit tests (19 cases — see §4).
- [ ] Criterion benchmarks (100 pages < 20 ms, incremental < 3 ms).

**Phase 2 — Collaborative Cursors & Undo (Week 3)**
- [ ] `UpdateCursor` → `SyncCommand::SendEphemeral` → `ProseEvent::RemoteCursors`.
- [ ] `LoroDoc::undo_manager()`: `Undo`/`Redo` scoped by local peer.

**Phase 3 — UI Integration & Stabilization (Week 4)**
- [ ] Connection `PageLayout` + `RunsUpdated` → `prose-ui` (virtual scroll, RunCache).
- [ ] End-to-end headless tests.
- [ ] Perf budget validation on target mobile hardware.
