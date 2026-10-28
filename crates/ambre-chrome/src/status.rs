// ambre-chrome/src/status.rs
// ============================================================================
// Shared status-bar building blocks (app-agnostic, ambre-spec-chrome.md,
// CHROME-014).
//
// The status bar itself is app-specific: its left side shows Trame's cell
// aggregates or Toile's selection count/bounds. Three pieces are identical
// across the canvas apps and live here, so the two read and behave the same:
//   - `StatusLabel` -- the app-name caption pinned to the left edge.
//   - `ZoomChip`    -- the interactive zoom readout + preset menu (right side).
//   - `PerfChip`    -- the interactive fps readout + an app-supplied detail
//                      panel (right side).
// All three are DOM Leptos and reuse the `statusbar-*` CSS classes shared with
// the app styles, so Trame and Toile status bars are visually identical.
// ============================================================================

use leptos::prelude::*;

/// Zoom presets offered by [`ZoomChip`] when the app supplies none. The canvas
/// apps (Trame grid zoom, Toile board zoom) share the same ladder so the menu
/// reads identically.
pub const ZOOM_PRESETS: [ZoomFactor; 6] = [
    ZoomFactor::positive(0.5),
    ZoomFactor::positive(0.75),
    ZoomFactor::ONE,
    ZoomFactor::positive(1.25),
    ZoomFactor::positive(1.5),
    ZoomFactor::positive(2.0),
];

pub use ambre_core::ZoomFactor;

/// The app-name caption pinned to the left of a status bar (for example "Trame",
/// "Toile"): a dim, small label, styled by the shared `statusbar-label` class.
#[component]
pub fn StatusLabel(#[prop(into)] label: String) -> impl IntoView {
    view! { <span class="statusbar-label">{label}</span> }
}

/// Interactive zoom chip: a button showing `<pct> %` that opens a preset menu.
/// `zoom` is the shared zoom factor (`1.0` = 100%) the canvas view publishes
/// each frame; the menu writes it and the view polls it. Shared by Trame (grid
/// zoom) and Toile (board zoom, `toile-status-bar`) so the two read
/// identically.
#[component]
pub fn ZoomChip(
    /// The live zoom factor (`1.0` = 100%). The menu sets it; the view mirrors it.
    zoom: RwSignal<ZoomFactor>,
    /// Preset factors offered by the menu, low-to-high. Defaults to [`ZOOM_PRESETS`].
    /// The first and the last bound what the menu writes.
    #[prop(optional, into)]
    presets: Option<Vec<ZoomFactor>>,
    /// Tooltip for the button (app-specific wording). Defaults to `"Zoom"`.
    #[prop(optional, into)]
    title: Option<String>,
) -> impl IntoView {
    // The preset menu is collapsed by default: only the readout shows.
    let show = RwSignal::new(crate::Openness::Closed);
    // Stored so the (repeatable) `Show` child can iterate it without moving it.
    let presets = presets.unwrap_or_else(|| ZOOM_PRESETS.to_vec());
    // The presets may run either way, so the range is their extremes.
    let lo = presets
        .iter()
        .copied()
        .reduce(|a, b| if b < a { b } else { a })
        .unwrap_or(ZoomFactor::ONE);
    let hi = presets
        .iter()
        .copied()
        .reduce(|a, b| if b > a { b } else { a })
        .unwrap_or(ZoomFactor::ONE);
    let presets = StoredValue::new(presets);
    let title = title.unwrap_or_else(|| "Zoom".to_owned());
    let menu_ref = NodeRef::<leptos::html::Ul>::new();
    // An opened menu takes the focus, so the keys reach it (`CHROME-026`).
    Effect::new(move |_| {
        if let Some(menu) = menu_ref.get() {
            crate::menu::focus_first_item(&menu);
        }
    });

    view! {
        <div class="statusbar-zoom">
            <button
                class="statusbar-zoom-btn"
                title=title
                aria-haspopup="menu"
                aria-expanded=move || show.get().aria()
                on:click=move |_| show.update(|v| *v = v.toggled())
            >
                {move || format!("{:.0} %", zoom.get().get() * 100.0)}
            </button>
            <Show when=move || show.get() == crate::Openness::Open fallback=|| ()>
                <ul
                    class="statusbar-zoom-menu"
                    role="menu"
                    node_ref=menu_ref
                    on:keydown=move |ev| {
                        if let Some(menu) = menu_ref.get_untracked() {
                            crate::menu::on_menu_keydown(&ev, &menu, || show.set(crate::Openness::Closed));
                        }
                    }
                >
                    {presets
                        .get_value()
                        .into_iter()
                        .rev()
                        .map(|z| {
                            let pct = format!("{:.0} %", z.get() * 100.0);
                            // `selected` highlights the row matching the live zoom.
                            let selected =
                                move || (zoom.get().get() - z.get()).abs() < f64::EPSILON;
                            view! {
                                <li role="none">
                                    <button
                                        role="menuitemradio"
                                        class="statusbar-zoom-item"
                                        class:selected=selected
                                        aria-checked=move || crate::Checked::from_bool(selected()).aria()
                                        on:click=move |_| {
                                            zoom.set(z.clamped(lo, hi));
                                            show.set(crate::Openness::Closed);
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
    }
}

/// The expandable render-diagnostics panel behind [`PerfChip`]. App-supplied so
/// each app shows the timings relevant to its renderer (Trame's grid detail,
/// Toile's board detail) while sharing the fps readout and toggle behaviour.
#[slot]
pub struct PerfDetail {
    children: ChildrenFn,
}

/// Interactive perf chip: a `<fps> fps` button that toggles an app-supplied
/// detail panel (`PerfDetail`). Shared by Trame and Toile so the fps readout and
/// its toggle behave identically; the panel body differs per renderer. With no
/// `detail` slot the button is a passive readout (no toggle).
#[component]
pub fn PerfChip(
    /// The live frame rate. Derive it from the app's perf snapshot signal.
    #[prop(into)]
    fps: Signal<ambre_core::FrameRate>,
    #[prop(optional)] perf_detail: Option<PerfDetail>,
) -> impl IntoView {
    // The detail panel is collapsed by default: only the fps chip shows.
    let show = RwSignal::new(false);
    let has_detail = perf_detail.is_some();

    view! {
        <div class="statusbar-perf">
            <button
                class="statusbar-fps"
                title="Toggle render diagnostics"
                on:click=move |_| {
                    if has_detail {
                        show.update(|v| *v = !*v);
                    }
                }
            >
                {move || format!("{:.0} fps", fps.get().get())}
            </button>
            {perf_detail
                .map(|d| {
                    let body = d.children;
                    view! {
                        <Show when=move || show.get() fallback=|| ()>
                            {let body = body.clone();
                            move || body()}
                        </Show>
                    }
                })}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A zoom factor is built inside its range, so a reader can divide by it
    /// (`UI-THREAD-060`).
    #[test]
    fn a_zoom_factor_is_clamped_where_it_is_built() {
        assert_eq!(ZoomFactor::within(5.0, 0.5, 2.0).get(), 2.0);
        assert_eq!(ZoomFactor::within(0.1, 0.5, 2.0).get(), 0.5);
        assert_eq!(ZoomFactor::within(f64::NAN, 0.5, 2.0).get(), 1.0);
        assert_eq!(ZoomFactor::within(f64::INFINITY, 0.5, 2.0).get(), 2.0);
        assert!(ZoomFactor::within(0.0, -1.0, f64::INFINITY).get() > 0.0);
    }
}
