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

use ambre_canvas::PerfSnapshot;
use ambre_chrome::{PerfChip, PerfDetail, StatusLabel, ZoomChip, ZoomFactor};
use leptos::prelude::*;

use crate::chrome::commands::status_label;

/// Selection aggregates rendered by the status bar, mirrored from
/// [`TrameEvent::SelectionStats`](ambre_bus::TrameEvent::SelectionStats). `count`
/// is the non-empty cell count; `count_num` the numeric one and gates whether
/// `sum`/`avg`/`min`/`max` are shown (they are meaningless without a number).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct SelStats {
    pub sum: f64,
    pub avg: f64,
    pub count: u64,
    pub count_num: u64,
    pub min: f64,
    pub max: f64,
}

/// Format an aggregate value: an integer shows without decimals, otherwise two
/// decimals (trailing zeros kept for stable width).
fn fmt_num(v: f64) -> String {
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
    zoom: RwSignal<ZoomFactor>,
) -> impl IntoView {
    // The aggregates line, recomputed whenever the selection stats change.
    let aggregates = move || {
        stats.get().map(|s| {
            // Numeric aggregates only when at least one number is selected.
            let nums = (s.count_num > 0).then(|| {
                view! {
                    <span class="stat">"Sum: " {fmt_num(s.sum)}</span>
                    <span class="stat">"Avg: " {fmt_num(s.avg)}</span>
                    <span class="stat">"Min: " {fmt_num(s.min)}</span>
                    <span class="stat">"Max: " {fmt_num(s.max)}</span>
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
                            <div>"frame " {format!("{:.1}", p.frame_ms.get())} " ms"</div>
                            <div>"drain " {format!("{:.1}", p.drain_ms.get())}
                                "  draw " {format!("{:.1}", p.draw_ms.get())}
                                "  gpu " {format!("{:.1}", p.gpu_ms.get())}</div>
                            <div>"bg " {format!("{:.2}", p.bg_ms.get())}
                                "  cells " {format!("{:.2}", p.cells_ms.get())}
                                "  hdrs " {format!("{:.2}", p.hdrs_ms.get())}</div>
                            <div>"drawn " {p.cells_drawn.to_string()}
                                "  miss " {p.glyph_misses.to_string()}
                                "  inlat " {format!("{:.1}", p.input_lat_ms.get())} " ms"</div>
                        </div>
                    }
                }}
            </PerfDetail>
        </PerfChip>
    }
}
