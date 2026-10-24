// ambre-toile/src/chrome/status.rs
// ============================================================================
// The Toile selection status bar (`toile-status-bar`): a thin DOM chrome band along
// the bottom edge, the whiteboard analog of Trame's status bar (`trame-status-bar`).
// Built on the app-agnostic `ambre-chrome` status toolkit so the two apps read
// and behave identically.
//
// Left: app label + selection tally + combined world bounds. An empty
//         or single-element selection shows only the count (no meaningful
//         multi-element bounds); the pure model + formatting live in
//         `ui::status` and are host-tested there.
// Right : the shared interactive zoom chip (board zoom) and the shared perf chip
//         (fps + a Toile-specific frame-timing panel).
// ============================================================================

use ambre_canvas::PerfSnapshot;
use ambre_chrome::{PerfChip, PerfDetail, StatusLabel, ZoomChip, ZoomFactor};
use leptos::prelude::*;

use crate::ui::status::{SelStats, bounds_label, count_label};

/// The Toile status bar component: app label + selection aggregates on the left,
/// interactive zoom + fps chips on the right. `stats` is `None` while no
/// selection reply has arrived. `zoom` is the shared board-zoom factor the canvas
/// view publishes each frame; `perf` is the live render snapshot.
#[component]
pub fn StatusBar(
    stats: RwSignal<Option<SelStats>>,
    perf: RwSignal<PerfSnapshot>,
    zoom: RwSignal<ZoomFactor>,
) -> impl IntoView {
    // The aggregates line, recomputed whenever the selection stats change. An
    // absent/empty selection collapses to nothing (only the zoom chip shows).
    let aggregates = move || {
        stats.get().and_then(|s| {
            count_label(&s).map(|count| {
                let bounds = bounds_label(&s);
                view! {
                    <span class="stat">{count}</span>
                    {bounds.map(|b| view! { <span class="stat">{b}</span> })}
                }
            })
        })
    };

    view! {
        <StatusLabel label="Toile" />
        <span class="statusbar-stats">{aggregates}</span>
        <span class="spacer"></span>
        // Shared interactive zoom + fps chips (identical to Trame). Toile uses
        // the default preset ladder; the perf panel shows the board frame timings.
        <ZoomChip zoom=zoom title="Board zoom" />
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
                            <div>"inlat " {format!("{:.1}", p.input_latency.get())} " ms"
                                "  miss " {p.glyph_misses.to_string()}</div>
                        </div>
                    }
                }}
            </PerfDetail>
        </PerfChip>
    }
}
