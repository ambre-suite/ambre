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
pub const ZOOM_PRESETS: [f64; 6] = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0];

/// A zoom factor, `1.0` being 100%: finite and strictly positive, so a reader
/// may divide by it. Each application builds it inside its own range.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct ZoomFactor(f64);

impl ZoomFactor {
    /// 100%.
    pub const ONE: Self = Self(1.0);

    /// `z` clamped into `lo ..= hi`. A NaN reads as 1, clamped the same way.
    /// A bound that is not positive and finite is pulled back to one that is.
    pub fn within(z: f64, lo: f64, hi: f64) -> Self {
        let lo = if lo.is_finite() {
            lo.max(f64::MIN_POSITIVE)
        } else {
            f64::MIN_POSITIVE
        };
        let hi = if hi.is_finite() { hi.max(lo) } else { lo };
        let z = if z.is_nan() { 1.0 } else { z };
        Self(z.clamp(lo, hi))
    }

    /// The factor as a number, `1.0` being 100%.
    pub fn get(self) -> f64 {
        self.0
    }
}

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
    presets: Option<Vec<f64>>,
    /// Tooltip for the button (app-specific wording). Defaults to `"Zoom"`.
    #[prop(optional, into)]
    title: Option<String>,
) -> impl IntoView {
    // The preset menu is collapsed by default: only the readout shows.
    let show = RwSignal::new(false);
    // Stored so the (repeatable) `Show` child can iterate it without moving it.
    let presets = presets.unwrap_or_else(|| ZOOM_PRESETS.to_vec());
    // The presets may run either way, so the range is their extremes.
    let lo = presets.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = presets.iter().copied().fold(f64::NEG_INFINITY, f64::max);
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
                aria-expanded=move || crate::menu::aria_bool(show.get())
                on:click=move |_| show.update(|v| *v = !*v)
            >
                {move || format!("{:.0} %", zoom.get().get() * 100.0)}
            </button>
            <Show when=move || show.get() fallback=|| ()>
                <ul
                    class="statusbar-zoom-menu"
                    role="menu"
                    node_ref=menu_ref
                    on:keydown=move |ev| {
                        if let Some(menu) = menu_ref.get_untracked() {
                            crate::menu::on_menu_keydown(&ev, &menu, || show.set(false));
                        }
                    }
                >
                    {presets
                        .get_value()
                        .into_iter()
                        .rev()
                        .map(|z| {
                            let pct = format!("{:.0} %", z * 100.0);
                            // `selected` highlights the row matching the live zoom.
                            let selected = move || (zoom.get().get() - z).abs() < f64::EPSILON;
                            view! {
                                <li role="none">
                                    <button
                                        role="menuitemradio"
                                        class="statusbar-zoom-item"
                                        class:selected=selected
                                        aria-checked=move || crate::menu::aria_bool(selected())
                                        on:click=move |_| {
                                            zoom.set(ZoomFactor::within(z, lo, hi));
                                            show.set(false);
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
    fps: Signal<f32>,
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
                {move || format!("{:.0} fps", fps.get())}
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
