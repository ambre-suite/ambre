# AMBRE — SPEC: PROSE / FEATURES
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
- CRDT persistence and `LoroDoc` mutations → `prose-engine` (`ProseState`).
- Visual rendering of features (comment highlights, revision bars) → `prose-ui`.
- Import/export of corresponding structures → `prose-interop`.

**Dependencies:**

| Crate | Justification |
|-------|---------------|
| `loro` | `LoroText::mark()`, `LoroMap` for comment threads, `UndoManager`. |
| `ambre-core` | `Uuid` for stable comment, suggestion, footnote IDs (via `ambre-bus` re-export). |
| `ambre-bus` | Typed IPC types for features, carried by `ProseCommand`/`ProseEvent` (→ `AppCommand::Prose`/`AppEvent::Prose`). |
| `unicode-segmentation` | Grapheme counting for Loro offsets (`chars()` Rust ≠ Unicode graphemes). |

> **No `serde` on feature types.** They travel **typed, by move** on `flume`, inside `ProseCommand`/`ProseEvent` (shared memory). `serde`/`postcard` are only used for **network bytes** (Loro CRDT payloads and `Ephemeral`) — see `ambre-spec-prose-logic.md`.

Post-MVP dependencies:

| Crate | Justification |
|-------|---------------|
| `zspell` | Pure Rust spell checker, WASM-compatible, Hunspell dictionaries. |

**Invariants:**

- A `CommentId` or `SuggestionId` (`Uuid`) is stable for the entire document lifetime: it does not change after partial accept/reject or remote delta import.
- List ordinals are **computed on demand** (not stored in the CRDT): they are derived from the `MovableTree` at read time. Storing ordinals in the CRDT would introduce unnecessary CRDT conflicts on any `ListItem` insertion.
- The TOC is **derived** from the `MovableTree` (not duplicated): no copy of headings is stored in `"meta"` or elsewhere.
- Rejected suggestions (track changes reject) are **removed from the CRDT** — they leave no trace in the final document.
- No feature short-circuits `ProseState::dispatch()` — all mutations go through typed `ProseCommand`.

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

**Note:** `superscript` and `subscript` are mutually exclusive — `resolve_style` ignores
`subscript` if `superscript` is active on the same range, and vice versa.

#### Style Cascade

```
1. Explicit inline mark on the LoroText      (highest priority)
2. Paragraph style (block's style_id)        → lookup in "styles" LoroMap
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

Commands (aligned with `ProseCommand` — source of truth in `prose-engine`):

```rust
SetParagraphStyle { block_id: TreeID, style: ParagraphStyle },
ApplyMark         { block_id: TreeID, range: (u32, u32), key: String, value: Option<String> },
RemoveMark        { block_id: TreeID, range: (u32, u32), key: String },
ClearInlineMarks  { block_id: TreeID, range: (u32, u32) },
```

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

pub fn compute_ordinal(tree: &LoroMovableTree, item_id: &TreeID) -> u32 {
    // Count preceding siblings of the same ListItem(ordered=true) type
    todo!()
}
```

Commands:

```rust
InsertListItem { parent_id: Option<TreeID>, after_id: Option<TreeID>, style: ListStyle },
SetListStyle   { block_id: TreeID, style: ListStyle },
IndentList     { block_id: TreeID },
UnindentList   { block_id: TreeID },
```

---

### C. Headings

Headings are `BlockType::Heading` nodes with `"a": {level: u8}` (1..=6).

```rust
// ambre-prose/src/features/heading.rs

pub fn compute_section_number(
    tree: &LoroMovableTree,
    heading_id: &TreeID,
) -> Vec<u32> {
    // DFS pre-order, per-level counter [u32; 6], reset lower levels
    todo!()
}
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
  "col_widths":    [f32, ...]         — column widths in pt
  "merged":        [[row,col,rowspan,colspan], ...]
}

// "a" of the TableCell node:
{
  "bg":            "#rrggbb" | null
  "border_top":    f32                — thickness in pt (0 = none)
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
InsertTableCol   { table_id: TreeID, after_col: u32 },
DeleteTableCol   { table_id: TreeID, col_index: u32 },
ResizeTableCol   { table_id: TreeID, col_index: u32, width_pt: f32 },
MergeTableCells  { table_id: TreeID, start: (u32, u32), end: (u32, u32) },
UnmergeTableCell { cell_id: TreeID },
SetCellStyle     { cell_id: TreeID, bg: Option<u32>, valign: Option<VAlign>,
                   border: Option<CellBorder>, padding_pt: Option<f32> },
SetTableStyle    { table_id: TreeID, style: TableStyle },
```

---

### E. Images

Images are `BlockType::Image` nodes in the `MovableTree`, inserted via
`ProseCommand::InsertImage`. Their attributes are stored in the node's `LoroMap` `"a"`.

#### CRDT Attributes

```
"a" of the Image node:
├── "content_hash" : String — content hash addressing the encrypted media blob (resolved via the ambre-sync media flow)
├── "alt"       : String   — alt text
├── "w"         : u32      — width in px (display dimension, not original dimension)
├── "h"         : u32      — height in px
├── "align"     : u8       — ImageAlign discriminant
└── "caption"   : String   — optional caption (used by O.8 Table of Figures)
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
> as `Block` in the MVP. Text wrapping is post-MVP — it requires a line-breaking
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

#### Commands

```rust
// InsertImage already in ProseCommand (prose-engine)
// Additional commands:
ResizeImage  { block_id: TreeID, width_px: u32, height_px: u32 },
SetImageAlign { block_id: TreeID, align: ImageAlign },
SetImageAlt  { block_id: TreeID, alt: String },
SetImageCaption { block_id: TreeID, caption: String },
```

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

pub fn build_toc(
    tree: &LoroMovableTree,
    layout: &PageLayout,
    max_level: u8,
    numbered: bool,
) -> TableOfContents { todo!() }
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
configuration plus the stable `block_id` (`TreeID`) that points at the text node — the
text is never duplicated inline (see `prose-logic §2.A`). Editing commands (`InsertText`,
`ApplyMark`, ...) target the header/footer via this `block_id` like any other block.

```
"meta"
├── "header" : LoroMap
│   ├── "enabled"              : bool
│   ├── "block_id"             : String   — stable TreeID of the header LoroText node
│   ├── "align"                : "left"|"center"|"right"
│   └── "different_first_page" : bool
├── "header_first" : LoroMap   — if different_first_page = true
│   └── "block_id" : String    — (same config shape as "header")
├── "footer" : LoroMap
│   ├── "enabled"  : bool
│   ├── "block_id" : String
│   └── "align"    : "left"|"center"|"right"
└── "footer_first" : LoroMap
    └── "block_id" : String
```

Headers/footers content supports **dynamic fields** inserted as special marks. Their
display value is dynamically resolved by `prose-ui` from the current `PageLayout` —
they contain no literal value in the CRDT.

```rust
// ambre-prose/src/mark.rs
pub const MARK_FIELD_PAGE_NUM:   &str = "f:pn";  // current page number
pub const MARK_FIELD_PAGE_COUNT: &str = "f:pc";  // total page count
pub const MARK_FIELD_DATE:       &str = "f:dt";  // last modified date
pub const MARK_FIELD_TITLE:      &str = "f:ti";  // document title
```

These marks all have `ExpandType::None`.

```rust
#[derive(Debug, Clone)]
pub enum HeaderField { PageNumber, PageCount, Date, Title }
```

Commands:

```rust
SetHeaderEnabled      { enabled: bool },
SetFooterEnabled      { enabled: bool },
// The header and footer each have a stable block_id in "meta".
// To edit their content, use InsertText / ApplyMark / etc. targeting that block_id
// directly -- identical to any other block. No dedicated recursive command.
InsertHeaderField     { field: HeaderField, pos: u32 },
InsertFooterField     { field: HeaderField, pos: u32 },
SetDifferentFirstPage { enabled: bool },
```

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
DeleteComment    { comment_id: Uuid },
EditCommentReply { comment_id: Uuid, reply_id: Uuid, new_text: String },
```

---

### I. Track Changes (Suggestions)

Model via `MarkType` on `LoroText`:
- `MARK_SUGGEST_INS` (`"si"`): text present, suggested as insertion.
- `MARK_SUGGEST_DEL` (`"sd"`): text present, suggested as deletion.

```
"annots"
└── "{suggestion_id}" : LoroMap
    ├── "type"       : u8  — 0 = insertion, 1 = deletion
    ├── "author"     : String
    ├── "peer_id"    : u64
    └── "created_at" : u64
```

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
        ProseCommand::DeleteText { block_id, range } => vec![
            ProseCommand::ApplyMark {
                block_id,
                range,
                key:   MARK_SUGGEST_DEL.to_string(),
                value: Some(suggestion_id.to_string()),
            },
        ],
        other => vec![other],
    }
}

pub fn accept_suggestion(doc: &mut LoroDoc, suggestion_id: Uuid) { todo!() }
pub fn reject_suggestion(doc: &mut LoroDoc, suggestion_id: Uuid) { todo!() }
pub fn accept_all(doc: &mut LoroDoc) { todo!() }
pub fn reject_all(doc: &mut LoroDoc) { todo!() }
```

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

**Documented limitation:** collaborative cross-peer undo not supported — identical to Google Docs.

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

#### Cross References

Mark `"ref"` on text, value = `"{target_block_id}:{kind}"`. Resolved at display time
from the current `PageLayout`.

```rust
#[derive(Debug, Clone)]
pub enum CrossRefKind {
    PageNumber,
    SectionNumber,
    CustomText(String),
}
```

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
order. Search never mutates the CRDT — only `Replace` sends `ProseCommand`.

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
) -> Vec<FindMatch> { todo!() }

/// Returns commands in reverse document order
/// so that offsets remain valid during application.
pub fn replace_all(
    doc: &LoroDoc,
    tree: &LoroMovableTree,
    query: &FindQuery,
    replacement: &str,
) -> Vec<ProseCommand> { todo!() }
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

Computed in the Logic thread on each `DocUpdated` (debounced 500ms).

```rust
// ambre-prose/src/features/word_count.rs

#[derive(Debug, Clone)]
pub struct WordCount {
    pub words:                  u32,
    pub characters:             u32,   // without spaces
    pub characters_with_spaces: u32,
    pub paragraphs:             u32,
    pub pages:                  u32,   // from PageLayout
}

pub fn compute_word_count(
    doc: &LoroDoc,
    tree: &LoroMovableTree,
    layout: &PageLayout,
    blocks: Option<&[TreeID]>,   // None = entire document
) -> WordCount { todo!() }
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

The dictionary is lazy-loaded from a separate WASM asset — not in the main binary.

Commands:

```rust
SetSpellCheckLang     { lang: String },
AcceptSpellSuggestion { block_id: TreeID, range: (u32, u32), replacement: String },
AddToDictionary       { word: String },
```

---

### O. Post-MVP Features

The features below are **specified for ISO Google Docs completeness** but are not part
of the MVP. Each can be implemented independently on top of the existing CRDT foundations.

---

#### O.1 Version History

Named document snapshots at a point in time, navigable and restorable. Distinct from
track changes (which is intra-document).

**Model:** `"meta.version_history"` (`LoroList<LoroMap>`), each entry `{name, created_at, snapshot_id}`. The Loro snapshot is stored server-side (S3), referenced by `snapshot_id`, encrypted with the current `DocumentKey` before upload — identical to the normal SecSync pipeline.

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
in the CRDT. Rendering is produced in the Logic thread (LaTeX engine → SVG, to be evaluated
between `typst` WASM and a custom MathML subset). The SVG is transmitted to the UI thread via
`LogicEnvelope`. Lazy WASM module to avoid impacting startup.

---

#### O.5 Bookmarks

Freely named anchors in the text, distinct from block `TreeID`s.

**Model:** mark `MARK_BOOKMARK` (`"bk"`) on a position or range, value = bookmark name.
Entry in `"annots"` for navigation.

**Commands:** `InsertBookmark { block_id, pos, name }`, `DeleteBookmark { name }`,
`GoToBookmark { name }`.

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

**Documented limitation:** application-level protection only — a malicious peer
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
- Collaborative cursors are encrypted in `ambre-sync` → `Ephemeral.ciphertext`.
- `zspell` is local — no network call for the spell checker.
- Protected zones (O.12) are an application-level rule — their security limitation is documented.

---

## 4. QUALITY & TESTABILITY

**Unit tests:**

| Feature | Tested cases |
|---------|-------------|
| Inline styles | Cascade mark > block style > built-in > default. Superscript/subscript mutually exclusive. Partial clear. |
| Lists | Ordinals after insert/delete/move. Reset on unindent. Todo checked/unchecked. |
| Headings | `compute_section_number`: H1→H2→H2→H3 = [1],[1,1],[1,2],[1,2,1]. Reset after H1. |
| Tables | Row/column CRUD. Merge/split. Cell styles. CRDT conflict simultaneous delete+insert. |
| TOC | `build_toc`: 10 mixed headings, section_number + page. |
| Headers/footers | Dynamic fields resolved from mocked `PageLayout`. Different first page. |
| Comments | Add → Reply → Resolve → orphan (annotated text deleted). |
| Track changes | Insert accepted/rejected, Delete accepted/rejected. AcceptAll. 2-peer convergence in suggestion mode. |
| Undo/Redo | 10 keystrokes → undo → redo. Local peer isolation. |
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
| `build_toc()` — 50 headings | < 1 ms | Logic |
| `compute_ordinal()` — 100 items | < 500 µs | Logic |
| `compute_section_number()` — 50 headings | < 500 µs | Logic |
| `intercept_command()` track changes | < 100 µs | Logic |
| `accept_all()` — 100 suggestions | < 5 ms | Logic |
| `find_all()` — 100 pages, query 10 chars | < 20 ms | Logic |
| `replace_all()` — 50 occurrences | < 30 ms | Logic |
| `compute_word_count()` — 100 pages | < 5 ms | Logic |
| `check_block()` spell — 200 words | < 10 ms | Logic P4 (post-MVP) |

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

**Phase 0 — Styles, Lists, Headings (Week 1)**
- [ ] Complete inline marks (+ superscript/subscript), `resolve_style` cascade, `ParagraphStyle`.
- [ ] Lists: `compute_ordinal()`, `IndentList`/`UnindentList`, all `OrderedStyle`.
- [ ] Headings: `compute_section_number()`.
- [ ] Unit tests styles, lists, headings.

**Phase 1 — Tables, TOC, Word Count (Week 2)**
- [ ] Tables: row/column CRUD, merge, cell styles (`bg`, border, valign), `TableStyle`.
- [ ] TOC: `build_toc()`, `BlockType::Toc` placeholder.
- Word count: `compute_word_count()`, debounce 500ms, `ProseEvent::WordCountUpdated`.

**Phase 2 — Headers, Footers, Find & Replace (Week 2-3)**
- [ ] Headers/footers: `"meta.header/footer"` structure, dynamic fields, different first page.
- Find & Replace: `find_all()`, `replace_one()`, `replace_all()` (exact + case insensitive + whole word).

**Phase 3 — Comments & Track Changes (Week 3)**
- [ ] Comments: `"annots"` structure, `"cm"` marks, Add/Reply/Resolve commands.
- [ ] Track changes: `intercept_command()`, accept/reject, 2-peer CRDT convergence.
- [ ] Prop-tests track changes.

**Phase 4 — Undo, Footnotes, Cross Refs (Week 4)**
- [ ] Undo/Redo: `init_undo_manager()`, merge_interval, local peer isolation.
- [ ] Footnotes and cross references.

**Phase 5 — Post-MVP (order suggested by increasing effort)**
- [ ] Regex find & replace — minimal effort, `FindQuery` already planned.
- [ ] Bookmarks — simple mark, minimal effort.
- [ ] Version history — S3 pipeline already in place.
- [ ] Linked styles — `resolve_style` extension.
- [ ] Pageless mode — disable pagination in `prose-engine`.
- [ ] Line numbers, Table of figures, O.9 Index — derived from `PageLayout`.
- [ ] Bibliography — new CSL subsystem.
- [ ] Section breaks + Multi-columns — pagination extension.
- [ ] Protected zones — `ProseState::dispatch()` extension.
- [ ] Headers/footers per section — depends O.3.
- [ ] Equation editor — new rendering engine, maximum effort.
- [ ] Spell checker — `zspell` + lazy asset.
