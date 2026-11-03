// ambre-trame/src/chrome/status.rs
// ============================================================================
// The status bar (`trame-status-bar`): a thin band above the sheet-tab strip.
//
// Left  : selection aggregates (Sum / Average / Count / Min / Max), Excel/Sheets
//         style. A lone cell shows nothing. The Logic thread computes the totals
//         for a multi-cell selection, since the UI holds only the viewport
//         (`trame-status-bar`). They arrive as `SelectionStats`.
//
// Right : a live perf chip (`<fps> fps`) that is clickable to expand a small
//         panel with the full frame timings. The perf HUD migrated here out of
//         the wgpu overlay. Shaping debug glyphs on the GPU every frame was pure
//         waste for a developer read-out.
//
// DOM Leptos, like every other E3 chrome surface (context menu, filter popover,
// FormatBar, Find&Replace) -- not a canvas `DrawSurface` band.
// ============================================================================

use ambre_bus::app::trame::CellNumber;
use ambre_canvas::PerfSnapshot;
use ambre_chrome::{PerfChip, PerfDetail, StatusLabel, ZoomChip};
use ambre_core::{Doc, Scale, View};
use leptos::prelude::*;

use crate::chrome::commands::status_label;

/// Selection aggregates rendered by the status bar, mirrored from
/// [`TrameEvent::SelectionStats`](ambre_bus::TrameEvent::SelectionStats). `count`
/// is the non-empty cell count; `numbers` the totals of the numeric cells, shown
/// only when there is one.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct SelStats {
    pub count: ambre_core::Tally<ambre_bus::app::trame::Cells>,
    pub numbers: Option<ambre_bus::app::trame::NumericStats>,
}

/// Format an aggregate value: an integer shows without decimals, otherwise two
/// decimals (trailing zeros kept for stable width). A total past the largest
/// finite number is `None` and reads `#NUM!`, as a formula whose arithmetic
/// overflows does (`TRAME-UI-179`).
fn fmt_num(v: Option<CellNumber>) -> String {
    let Some(v) = v.map(CellNumber::get) else {
        return "#NUM!".to_owned();
    };
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{v:.0}")
    } else {
        format!("{v:.2}")
    }
}

/// The status bar component: app label, selection aggregates, a clickable grid
/// zoom menu, and a clickable perf chip backed by `perf`. `stats` is `None` for a
/// single-cell selection. `zoom` is the shared grid zoom factor (`1.0` = 100%):
/// the menu writes it and the canvas view polls it each frame.
#[component]
pub fn StatusBar(
    stats: RwSignal<Option<SelStats>>,
    perf: RwSignal<PerfSnapshot>,
    zoom: RwSignal<Scale<Doc, View>>,
) -> impl IntoView {
    // The aggregates line, recomputed whenever the selection stats change.
    let aggregates = move || {
        stats.get().map(|s| {
            // Numeric aggregates only when at least one number is selected.
            let nums = s.numbers.map(|n| {
                view! {
                    <span class="stat">"Sum: " {fmt_num(n.sum())}</span>
                    <span class="stat">"Avg: " {fmt_num(n.avg())}</span>
                    <span class="stat">"Min: " {fmt_num(Some(n.min()))}</span>
                    <span class="stat">"Max: " {fmt_num(Some(n.max()))}</span>
                }
            });
            view! {
                {nums}
                <span class="stat">"Count: " {s.count.to_string()}</span>
            }
        })
    };

    view! {
        <StatusLabel label=status_label() />
        <span class="statusbar-stats">{aggregates}</span>
        <span class="spacer"></span>
        // Shared interactive zoom + fps chips (identical to Toile,
        // `toile-status-bar`). The
        // grid-zoom ladder stays Trame's own so it never diverges from the
        // sheet's virtual-scroll steps; the perf panel shows Trame's grid detail.
        <ZoomChip
            zoom=zoom
            presets=crate::ui::grid::TrameUiState::ZOOM_PRESETS.to_vec()
            title="Grid zoom (scales the sheet only, not the toolbar)"
        />
        <PerfChip fps=Signal::derive(move || perf.get().fps)>
            <PerfDetail slot>
                {move || {
                    let p = perf.get();
                    view! {
                        <div class="statusbar-perf-panel">
                            <div>"frame " {format!("{:.1}", p.frame.get())} " ms"</div>
                            <div>"drain " {format!("{:.1}", p.drain.get())}
                                "  draw " {format!("{:.1}", p.draw.get())}
                                "  gpu " {format!("{:.1}", p.gpu.get())}</div>
                            <div>"bg " {format!("{:.2}", p.background.get())}
                                "  cells " {format!("{:.2}", p.cells.get())}
                                "  hdrs " {format!("{:.2}", p.headers.get())}</div>
                            <div>"drawn " {p.cells_drawn.to_string()}
                                "  miss " {p.glyph_misses.to_string()}
                                "  inlat " {format!("{:.1}", p.input_latency.get())} " ms"</div>
                        </div>
                    }
                }}
            </PerfDetail>
        </PerfChip>
    }
}
