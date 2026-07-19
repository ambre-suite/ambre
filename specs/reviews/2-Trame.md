# Spec review log — specs/2-Trame

Coverage log for the `ambre-spec-review` skill. Data file, updated per pass.
See `.claude/skills/ambre-spec-review/` for the procedure and taxonomy.

- Target folder: `specs/2-Trame/` (logic, features, interop, ui)
- Specs in folder: 4 -> pass cap = max(6, 2*4) = **8**
- Termination: `CONVERGED` (2 quiet passes, different entry points) or `BUDGET-EXHAUSTED` at pass 8.
- Status: **IN PROGRESS** — 2 passes done, not converged.

---

## Corrections applied (2026-07-18)

All findings were re-verified against the current `ambre-trame` / `ambre-bus` code before editing; the code is the ground truth and the specs were realigned to it.

**Root causes fixed:**
- **Stale `TrameCommand`/`TrameEvent`/`DisplayCell` in logic §9.** The bus (`ambre-bus::app::trame`) is now the single source of truth; §9 was rewritten to mirror it exactly (positional grid coords, `SetCellText`/`SetCellsBatch` (no `EditCell`/`ClearCell`), parameterless intents, real `FilterApplied {hidden_row_count,col}` / `SelectionStats` (u64) / `FindNext` / `ReplaceAll` / `MergeConflictDetected` (positional) / `SheetList {..,active}` / `ScrollViewport` (no `velocity`/`sheet_id`)). **No `serde`** on the enums (only the value types). `ImportXlsx`/`ExportXlsx`/`ExportPdf`/presence/`RequestSnapshot` are labelled "planned, not yet on the bus". Resolves T-01, T-03, T-04, T-06..T-15, T-39, T-54, T-60.
- **VM op:** `Op::PropagateError` → `Op::PushError(FormulaError)` (T-23). **EntireAxis** reserved/never parsed — §10 test + roadmap corrected (T-25). **Deletion** reseeds (no `collect_dirty_for_deleted`) (T-40). **P3** documents the as-built full reseed (T-35). **NamedRange/Name** eliminated by `resolve_refs`, `debug_assert!`+`PushEmpty` fallback (T-41).
- **Interop:** CSV §4 rewritten to the real `TrameState` methods (positional `write_cell`, `used_extent`, `evaluated_value`, no `len-1` underflow, no borrow clash, "first kilobyte" comment, no unwrap) — T-19, T-20, T-42, T-45. xlsx/PDF §2/§3/§5 flagged as unimplemented Phase 2/3 sketches with a reconciliation banner (`TrameState` not `TrameController`; positional `CellRef`; flat cells; one-way `TrameOutcome`; `#id`↔name mapping; synchronous, no `ImportProgress`) + concrete bug fixes (arity, if/else type, date-as-serial, unwrap) — T-16, T-17, T-18, T-21, T-46..T-52, T-56, T-57, T-59.
- **UI:** §1/§2.B rewritten to the typed `flume` plane (no `LogicEnvelope`/postcard), exhaustive `TrameEvent` dispatch, `FormulaError`/`EditAck`/`ImportProgress` are not variants — T-02, T-14, T-53, T-54. All vello `Scene` calls → `DrawSurface` (`fill_rect`/`stroke_rect`/`stroke_line`) — T-55. `col_offsets` underflow guarded (T-58), `velocity_px_ms` dropped (T-60), autocompletion slice/`letter_to_index` guarded (T-38), `color_for_user` → shared `ambre_draw::peer::peer_color` (T-62), PDF clipping no-ellipsis + Logic-thread base-font layer (T-36, T-37).
- **Features:** `sort_range` rewritten to the real active-sheet/`realize_sort` path (T-26, T-27), `compare_cell_values` `Error<Empty` comment fixed + descending-blanks-to-top documented (T-28, T-29), undo/redo = `apply_history` + full `after_structural` reseed (rebuilds `filter_hidden`, no `doc.diff`) (T-31, T-32), `apply_remote` Post-MVP `from_user_id` param + no-unwrap (T-30), `FilterApplied {..,col}` / `ProtectionViolationWarning {from_user_id: UserId}` (T-06).

**Design questions (resolved by the user):** T-34 → deterministic re-insertion of the first-sorting `sheet_id` on concurrent last-sheet deletion (impl pending a 2-peer test); T-61 → xlsx numeric-vs-text export typed by the evaluated `CellValue` (`evaluated_value`), a single typing source of truth.

---

## Corrections applied (2026-07-19, cross-app presence follow-up)

Surfaced while pinning the canonical peer-color helper for the Prose review (`specs/reviews/4-Prose.md`, P-04/P-49).

- **`UserId` newtype erased to raw `Uuid` in the presence + protection paths (NEW, extends T-62).** `trame-ui` declared `remote_cursors: HashMap<Uuid, RemoteCursor>` and `RemoteCursor.user_id: Uuid`, and `color_for_user(&Uuid)` — directly contradicting `trame-logic §9` (`RemoteCursor.user_id: UserId`, `apply_ephemeral(from_user_id: UserId)`) and the spec's own "`UserId` not `pub_key`" rationale. Also `trame-logic` `my_user_id: Uuid` and the range-protection enforcement (`allowed: Vec<Uuid>`, features §) used raw `Uuid`. Fixed: all in-memory user identity is now `UserId` end-to-end (CRDT still serializes `owner_user_id`/`allowed_user_ids` as Uuid-hex strings). `SessionReady` already emits `user_id: UserId`, so `my_user_id: UserId` also removes a latent mismatch.
- **Canonical `peer_color` (finalizes T-62).** The shared helper is now defined in Foundation: `ambre_draw::peer::peer_color(user_id: &UserId) -> peniko::Color` (`ambre-spec-canvas.md §D`), seeded on the canonical `UserId`; `ambre-draw` takes a scoped `ambre-core` dep (`architecture.md §2` invariant updated). Trame's `color_for_user` is now a `&UserId` delegate to it.

---

## Pass 1

- Entry point: anchor `trame-logic`, all six taxonomy categories.
- Generator: 45 candidates. Verifier (separate agent): 37 CONFIRMED, 3 NEEDS-HUMAN, 5 REJECTED.
- Rejected (not logged as findings): T-05 (context-menu UI variants, not commands), T-24 (compile() invariant prevents underflow), T-33 (composition is implied), T-43 (freeze is a design sketch, roadmap-consistent), T-44 (generic Error event covers the warning).
- Sparse categories this pass: (4) Rust/WASM implementability, (6) ripple effects. Next pass must change entry point toward these (anchor `interop`/`ui`, pull in Foundations sync/architecture specs).

### Findings

| id | spec | section | category | severity | status | pass | verdict | note |
|---|---|---|---|---|---|---|---|---|
| T-01 | logic/features | §9 vs §7 | internal-consistency | blocker | fixed | 1 | CONFIRMED | TrameCommand/Event derive serde in logic; features §7 says "no serde". |
| T-02 | ui/logic | ui§1 vs §9 | external-consistency | blocker | fixed | 1 | CONFIRMED | UI uses postcard over postMessage; logic says shared-memory flume, no serialization. |
| T-03 | logic/ui | §9 vs ui§5.A | internal-consistency | major | fixed | 1 | CONFIRMED | ClearCell command exists in enum; ui says empty SetCellText clears. |
| T-04 | logic/ui/features | §9 vs ui/features | external-consistency | blocker | fixed | 1 | CONFIRMED | SetCellText used everywhere; canonical enum only has EditCell. |
| T-06 | logic/features/ui | §9 vs §7 | internal-consistency | major | fixed | 1 | CONFIRMED | FilterApplied field set diverges three ways. |
| T-07 | logic/features | §9 vs §1 | internal-consistency | major | fixed | 1 | CONFIRMED | ApplyFilter carries kind:FilterKind AND redundant values/condition. |
| T-08 | logic/ui | §9 vs ui§5.G | internal-consistency | major | fixed | 1 | CONFIRMED | FindNext field mismatch (sheet_id/from/find vs query). |
| T-09 | logic/ui | §9 vs ui§5.G | internal-consistency | major | fixed | 1 | CONFIRMED | ReplaceAll field mismatch (version/scope vs query/all_sheets). |
| T-10 | logic/ui | §9 vs ui§13bis | internal-consistency | minor | fixed | 1 | CONFIRMED | RequestSelectionStats params vs parameterless. |
| T-11 | logic/ui | §9 vs ui§5.C | internal-consistency | major | fixed | 1 | CONFIRMED | SortRange/SortRangeMulti lack version; ui passes version. |
| T-12 | logic/ui | §9 vs ui§13 | internal-consistency | major | fixed | 1 | CONFIRMED | SheetList lacks active field; ui/logic expect it. |
| T-13 | logic/ui | §9 vs ui§2 | internal-consistency | minor | fixed | 1 | CONFIRMED | MergeConflictDetected field names differ. |
| T-14 | ui | §2 vs §18 | internal-consistency | major | fixed | 1 | CONFIRMED | Handler matches FormulaError/ExportComplete but §18 says not variants. |
| T-15 | logic | §9 DisplayCell | completeness | major | fixed | 1 | CONFIRMED | DisplayCell lacks is_filtered/is_error/sparkline used elsewhere. |
| T-16 | interop | §2.C-E vs logic§2.B | external-consistency | major | fixed | 1 | CONFIRMED | Import writes nested {v,t,s}+styles map; logic says flat, not implemented. |
| T-17 | interop | §7 vs logic§8 | internal-consistency | major | fixed | 1 | CONFIRMED | Import batches + ImportProgress mid-op; logic mandates synchronous single outcome. |
| T-18 | interop | §2.A vs logic§8 | external-consistency | major | fixed | 1 | CONFIRMED | Import calls SyncCommand directly; contradicts one-way TrameOutcome. |
| T-19 | interop | §4.A import_csv | rust-implementability | major | fixed | 1 | CONFIRMED | Overlapping &mut controller borrows; won't compile. |
| T-20 | interop | §4.B export_csv | edge-case | major | fixed | 1 | CONFIRMED | len - 1 underflow/panic on empty sheet. |
| T-21 | interop | §2.C dates | edge-case | major | fixed | 1 | CONFIRMED | Dates as ISO strings typed "n"; engine uses Excel serials. |
| T-22 | interop/features | §6/§2 | error-path | major | fixed | 1 | CONFIRMED | import().unwrap()/save_to_buffer().unwrap() panic on malformed input. |
| T-23 | logic | §4.A/§4.B | correctness | major | fixed | 1 | CONFIRMED | Op::PropagateError defined but compile() never emits it. |
| T-25 | logic | §3.A vs §10 | internal-consistency | major | fixed | 1 | CONFIRMED | EntireAxis "never emitted" yet SUM(A:A) is MVP + a test. |
| T-26 | features | §2 sort_range | correctness | major | fixed | 1 | CONFIRMED | skip(row_end+1) on range-only vec always empty; rows dropped. |
| T-27 | features | §2 sort_range | rust-implementability | minor | fixed | 1 | CONFIRMED | cmd.sheet_idx used but field is sheet_id. |
| T-28 | features | §2 compare | consistency | minor | fixed | 1 | CONFIRMED | Comment vs impl: Error<Empty ordering mismatch. |
| T-29 | features | §2 sort | edge-case | minor | fixed | 1 | CONFIRMED | Descending reverses Empty too; blanks to top (Excel keeps last). |
| T-30 | features | §6 apply_remote | completeness | major | fixed | 1 | CONFIRMED | Uses &from_user_id but apply_remote has no such param. |
| T-31 | features | §4 undo | completeness | major | fixed | 1 | NEEDS-HUMAN | doc.diff needs stored prev version absent from TrameState; underspecified. |
| T-32 | features | §4 undo + §1 | completeness | major | fixed | 1 | CONFIRMED | rebuild_after_undo_redo never recomputes filter_hidden; stale after undo. |
| T-34 | logic | §2.D delete | edge-case | major | fixed | 1 | NEEDS-HUMAN | Concurrent last-sheet deletion restores emptied sheet; CRDT semantics decision. **Decided (user): deterministic re-insertion of the first-sorting `sheet_id`; §2.D + roadmap updated. Impl pending a 2-peer convergence test.** |
| T-35 | logic | §8 vs §11bisA | perf-budget | major | fixed | 1 | CONFIRMED | Full O(document) reseed per batch vs incremental promise; >50ms at 100k. |
| T-36 | ui | §12 PDF vs §4bisC | rust-implementability | major | fixed | 1 | CONFIRMED | Logic-thread PDF text needs fonts; fonts are UI/gpu-side only. |
| T-37 | ui | §12 clipping | consistency | minor | fixed | 1 | CONFIRMED | Screen no-ellipsis vs KrillaSurface ellipsis; "same output" broken. |
| T-38 | ui/logic | §7 autocompletion | edge-case | minor | fixed | 1 | CONFIRMED | content[1..cursor] panics cursor==0; letter_to_index underflow. |
| T-39 | logic | §7 vs §9 | completeness | minor | fixed | 1 | CONFIRMED | UpdateCursor uses local display name; no name field in TrameState. |
| T-40 | logic | §3.B collect_dirty | completeness | minor | fixed | 1 | CONFIRMED | collect_dirty_for_deleted called but not defined on DependencyGraph. |
| T-41 | logic | §4.B NamedRange | correctness | minor | fixed | 1 | NEEDS-HUMAN | NamedRange compiles to no ops; safe only under unstated parser guarantee. |
| T-42 | interop | §4.A delimiter | consistency | minor | fixed | 1 | CONFIRMED | Comment "first 10 lines" vs 1024-byte prefix sampling. |
| T-45 | interop | §3.B export | consistency | minor | fixed | 1 | CONFIRMED | formula_cache.get(&CellRef) but cache keyed by CellId. |

## Pass 2

- Entry point: anchor `interop`/`ui`, cross-checked against Foundations `sync`/`architecture`/`ui`/`canvas`. Emphasized the sparse categories from pass 1: (4) Rust/WASM implementability and (6) ripple effects. Different anchor + different category subset than pass 1 (anti-false-convergence satisfied).
- Generator: 25 candidates. Verifier (separate agent): 15 CONFIRMED, 2 NEEDS-HUMAN, 8 REJECTED.
- Rejected (not logged): sheet_indices SheetId-vs-u16 (two different maps, misread), SendUpdate "full snapshot" (delta-from-empty is legit), ImportProgress/ExportComplete (Phase-3 phasing not contradiction), UI letter_to_index (pure associated fn, no doc data), KrillaSurface crate placement (layout entry drives surface, ok), DateTime "n" ISO (dup of pass-1 ISO-vs-serial), fill_path bbox-reject (documented+accepted limitation), import_csv &mut span (dup of T-19).
- New dominant cluster: interop is written against a `TrameController`/`SheetId`/`Uid` model the logic spec does not define (logic uses `TrameState`, `CellRef{sheet:u16,row:u32,col:u32}` positional, `Uid::to_key_part`). T-46..T-50 share this root. Second cluster: UI IPC + rendering written against stale `LogicEnvelope`/postcard + vello `Scene` APIs contradicting the typed flume bus and the `DrawSurface` mandate (T-53..T-55).
- Categories exercised: (3) external-consistency, (4) rust-implementability, (6) ripple now well covered (were sparse in pass 1).

### Findings

| id | spec | section | category | severity | status | pass | verdict | note |
|---|---|---|---|---|---|---|---|---|
| T-46 | interop | §2.C | rust-implementability | major | fixed | 2 | CONFIRMED | convert_excel_formula called with 2 args (f, controller); defined with 3 (excel_formula, sheet_index, sheet_name_to_id). Arity/type mismatch. |
| T-47 | interop | §2.C | rust-implementability | major | fixed | 2 | CONFIRMED | if/else arms differ: if -> ImportedCellKind, else -> String (raw); then insert("v", final_raw) treats as String. |
| T-48 | interop/logic | interop§2.C vs logic§ CellRef | external-consistency | major | fixed | 2 | CONFIRMED | CellRef built from SheetId + Uid pair (resolve -> (Uid,Uid)); def is {sheet:u16,row:u32,col:u32} positional. |
| T-49 | interop/logic | interop§2.C vs logic§ Uid | rust-implementability | major | fixed | 2 | CONFIRMED | cell_ref.sheet.0 on bare u16; .row/.col.to_key_part() but to_key_part is on Uid, not u32. Distinct site from T-48. |
| T-50 | interop/logic | interop (all) vs logic§ TrameState | external-consistency | major | fixed | 2 | CONFIRMED | interop's whole method surface hangs off TrameController; logic type is TrameState. TrameController undefined. |
| T-51 | interop/architecture | interop§2.A/2.B/3.A vs arch§2.D | external-consistency | minor | fixed | 2 | CONFIRMED | interop routes import/export bytes UI<->Logic via postMessage; arch says that hop is flume typed-by-move, no postMessage. |
| T-52 | interop/logic | interop§7 vs logic§8 | rust-implementability | minor | fixed | 2 | CONFIRMED | interop yields between 500-cell batches mid-import; handle_command runs synchronously to completion, no re-entrant import command defined. |
| T-53 | ui/architecture | ui§2.B vs arch§2.D | external-consistency | major | fixed | 2 | CONFIRMED | ui decodes LogicEnvelope::ViewportUpdate via postcard::from_bytes on UI<->Logic; arch: that hop carries no postcard, typed plane. |
| T-54 | ui | §2.B vs §18 | internal-consistency | minor | fixed | 2 | CONFIRMED | Same spec: §2.B models IPC as LogicEnvelope+postcard, §18 says typed ambre-bus::TrameEvent exhaustive. Incompatible shapes. |
| T-55 | ui/canvas | ui render vs canvas§ DrawSurface | rust-implementability | major | fixed | 2 | CONFIRMED | ui uses vello Scene API (self.scene.fill/stroke, Fill::NonZero, Affine, Rect, Line); no vello dep; mandate is DrawSurface (fill_rect/stroke_line/draw_glyphs). |
| T-56 | interop | §3.B | error-path | minor | fixed | 2 | CONFIRMED | ws.set_name(...).unwrap() on user sheet name; rust_xlsxwriter errors on >31 chars/forbidden/duplicate -> panic. |
| T-57 | interop | §2.C | internal-consistency | minor | fixed | 2 | CONFIRMED | import_cell tags formula cells with value-derived t (n/s/b) even when formula present; import_cell_with_kind sets t:"f". Two divergent paths. |
| T-58 | ui | §3.A | edge-case | minor | fixed | 2 | CONFIRMED | visible_range: col_offsets.len() as u32 - 1 unguarded; empty offsets -> 0u32-1 underflow (col_at_pixel uses saturating_sub, this doesn't). |
| T-59 | interop/logic | interop§2/§3 vs logic§12 | completeness | major | fixed | 2 | CONFIRMED | logic§12 mandates interop map #id<->sheet-name for xlsx/CSV; export writes raw #s0!A1 verbatim, mapping absent. |
| T-60 | ui/architecture/logic | ui§5 vs arch§2.D/logic§ enum | external-consistency | minor | fixed | 2 | CONFIRMED | New sites beyond pass-1 enum cluster: ScrollViewport carries velocity_px_ms (in neither bus nor logic); SetCellText emitted, absent from logic enum (has EditCell/ClearCell). |
| T-61 | interop | §3.B export | rust-implementability | major | fixed | 2 | NEEDS-HUMAN | Numeric-vs-text export decided solely by formula_cache membership; unclear whether plain literals are guaranteed cached -> may export numbers as text. **Decided (user): type by the cell's evaluated `CellValue` via `evaluated_value` (single typing source of truth), not cache membership; §3.B documents the rule.** |
| T-62 | ui | §9 color_for_user | rust-implementability | minor | fixed | 2 | NEEDS-HUMAN | Color::from_hsl(u16, f32, f32) on peniko AlphaColor<Srgb>; existence of that constructor is an external API fact, unverifiable from specs. |
