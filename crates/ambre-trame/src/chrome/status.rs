// ambre-trame/src/chrome/status.rs
// ============================================================================
// The status bar (`-ui.md` sec.13bis): a thin band above the sheet-tab strip.
//
// Left  : selection aggregates (Sum / Average / Count / Min / Max), Excel/Sheets
//         style. A lone cell shows nothing; the Logic thread computes the totals
//         for a multi-cell selection (the UI holds only the viewport, sec.16) and
//         they arrive as `SelectionStats`.
// Right : a live perf chip (`<fps> fps`) that is clickable to expand a small
//         panel with the full frame timings. The perf HUD migrated here out of
//         the wgpu overlay -- shaping debug glyphs on the GPU every frame was
//         pure waste for a developer read-out.
//
// DOM Leptos, like every other E3 chrome surface (context menu, filter popover,
// FormatBar, Find&Replace) -- not a canvas `DrawSurface` band.
// ============================================================================

use ambre_canvas::PerfSnapshot;
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
    zoom: RwSignal<f64>,
) -> impl IntoView {
    // Whether the full perf panel is expanded (collapsed by default: only the
    // fps chip shows).
    let show_perf = RwSignal::new(false);

    // Whether the grid-zoom preset menu is open (collapsed by default).
    let show_zoom = RwSignal::new(false);

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

    let fps_label = move || format!("{:.0} fps", perf.get().fps);

    let perf_panel = move || {
        let p = perf.get();
        view! {
            <div class="statusbar-perf-panel">
                <div>"frame " {format!("{:.1}", p.frame_ms)} " ms"</div>
                <div>"drain " {format!("{:.1}", p.drain_ms)}
                    "  draw " {format!("{:.1}", p.draw_ms)}
                    "  gpu " {format!("{:.1}", p.gpu_ms)}</div>
                <div>"bg " {format!("{:.2}", p.bg_ms)}
                    "  cells " {format!("{:.2}", p.cells_ms)}
                    "  hdrs " {format!("{:.2}", p.hdrs_ms)}</div>
                <div>"drawn " {p.cells_drawn.to_string()}
                    "  miss " {p.glyph_misses.to_string()}
                    "  inlat " {format!("{:.1}", p.input_lat_ms)} " ms"</div>
            </div>
        }
    };

    view! {
        <span class="statusbar-label">{status_label()}</span>
        <span class="statusbar-stats">{aggregates}</span>
        <span class="spacer"></span>
        <div class="statusbar-zoom">
            <button
                class="statusbar-zoom-btn"
                title="Grid zoom (scales the sheet only, not the toolbar)"
                aria-haspopup="menu"
                aria-expanded=move || show_zoom.get().then_some("true")
                on:click=move |_| show_zoom.update(|v| *v = !*v)
            >
                {move || format!("{:.0} %", zoom.get() * 100.0)}
            </button>
            <Show when=move || show_zoom.get() fallback=|| ()>
                <ul class="statusbar-zoom-menu" role="menu">
                    {crate::ui::grid::TrameUiState::ZOOM_PRESETS
                        .iter()
                        .rev()
                        .map(|&z| {
                            let pct = format!("{:.0} %", z * 100.0);
                            // `selected` highlights the row matching the live zoom.
                            let selected = move || (zoom.get() - z).abs() < f64::EPSILON;
                            view! {
                                <li role="none">
                                    <button
                                        role="menuitemradio"
                                        class="statusbar-zoom-item"
                                        class:selected=selected
                                        aria-checked=move || selected().then_some("true")
                                        on:click=move |_| {
                                            zoom.set(z);
                                            show_zoom.set(false);
                                        }
                                    >
                                        {pct}
                                    </button>
                                </li>
                            }
                        })
                        .collect_view()}
                </ul>
            </Show>
        </div>
        <div class="statusbar-perf">
            <button
                class="statusbar-fps"
                title="Toggle render diagnostics"
                on:click=move |_| show_perf.update(|v| *v = !*v)
            >
                {fps_label}
            </button>
            <Show when=move || show_perf.get() fallback=|| ()>
                {perf_panel}
            </Show>
        </div>
    }
}
