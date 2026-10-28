// ambre-chrome/src/format.rs
// ============================================================================
// Shared, app-agnostic text-format controls (ambre-spec-chrome.md). The rest of the
// text-format cluster promoted out of Trame's cell toolbar. Trame cells and Toile
// elements drive the SAME controls (mirrors `FontPicker` / `ColorPopover`):
//
//   FontSizeSelect -- the point-size dropdown (a curated preset list + an "unset"
//                     entry), value-agnostic (`Option<FontSize>`, `None` = the unset /
//                     auto entry). The caller maps `None` to its own meaning
//                     (Trame: the body default; Toile: auto-size to the card).
//   FormatToggle   -- a single pressed-state toolbar button (bold / italic /
//                     underline / strikethrough), reflecting a `pressed` signal
//                     and reporting clicks through `on_toggle`. The caller owns
//                     what a toggle means to its model (a `CommandId`, a style
//                     patch, ...): the control never reaches past the DOM.
//
//   FormatBar      -- the bar of format controls over a selection. Prose and
//                     Murmure hand it the anchor and the controls; it holds
//                     no state of its own.
//
// `FormatToggle` does `on:mousedown prevent_default` so a click never steals focus
// from the host's text pipe (the palette / grid-focus hazard both apps guard
// against). `FontSizeSelect` must NOT: on a native <select>, preventing the mousedown
// default suppresses the browser's opening of the option list, so the dropdown never
// appears. It relies on `on:change` instead.
// ============================================================================

use ambre_core::{CssPoint, CssPx, FontSize};
use leptos::prelude::*;
use leptos_icons::Icon;

/// The point sizes offered by [`FontSizeSelect`], shared by both apps so the cell
/// and element size pickers list the very same steps.
pub const FONT_SIZES: &[FontSize] = &[
    FontSize::from_whole_points(8),
    FontSize::from_whole_points(9),
    FontSize::from_whole_points(10),
    FontSize::from_whole_points(11),
    FontSize::from_whole_points(12),
    FontSize::from_whole_points(14),
    FontSize::from_whole_points(16),
    FontSize::from_whole_points(18),
    FontSize::from_whole_points(24),
    FontSize::from_whole_points(36),
];

/// The shared font-size dropdown. `current` is the active explicit size (`None` =
/// unset -> the `auto_label` entry is selected). `on_pick` receives the chosen size
/// (`None` when the unset entry is picked). `auto_label` names the unset entry
/// ("Auto" for Toile's card auto-size, "Default" for Trame's body default).
#[component]
pub fn FontSizeSelect(
    current: Signal<Option<FontSize>>,
    on_pick: Callback<Option<FontSize>>,
    #[prop(default = "Auto")] auto_label: &'static str,
) -> impl IntoView {
    view! {
        <select
            class="ambre-font-size"
            aria-label="Font size"
            // The unset entry has an empty value, so `None` selects it.
            prop:value=move || {
                current.get().map(|s| s.whole_points().to_string()).unwrap_or_default()
            }
            // NOTE: unlike the toggle buttons, a native <select> must NOT
            // preventDefault on mousedown. Doing so suppresses the browser's own
            // opening of the option list, so the dropdown never appears. The pick is
            // committed by on:change, which fires regardless of focus movement.
            on:change=move |ev| {
                // An empty value (the unset entry) parses to `None`.
                let points = event_target_value(&ev).parse::<u16>().ok();
                on_pick.run(points.map(FontSize::from_whole_points));
            }
        >
            <option value="">{auto_label}</option>
            {FONT_SIZES
                .iter()
                .map(|s| {
                    let points = s.whole_points().to_string();
                    let label = points.clone();
                    view! { <option value=points>{label}</option> }
                })
                .collect_view()}
        </select>
    }
}

/// A single pressed-state format toggle button (bold / italic / underline /
/// strikethrough). `pressed` drives the `aria-pressed` state (and the CSS active
/// look); `on_toggle` fires on click. The caller decides what the toggle does --
/// the control only reflects state and reports intent.
/// `label` and `icon` are owned rather than static, so a caller can hand over
/// what a command registry describes instead of a literal
/// (`ambre-spec-commands.md`, UI-THREAD-019). A control that backs no command --
/// underline, strikethrough -- still passes its own, having no description to
/// read.
#[component]
pub fn FormatToggle(
    #[prop(into)] label: String,
    #[prop(into)] icon: Option<icondata::Icon>,
    pressed: Signal<Pressed>,
    on_toggle: Callback<()>,
) -> impl IntoView {
    let title = label.clone();
    view! {
        <button
            aria-label=label
            title=title
            aria-pressed=move || pressed.get().aria()
            on:mousedown=move |ev| ev.prevent_default()
            on:click=move |_| on_toggle.run(())
        >
            {icon.map(|ic| view! { <Icon icon=ic /> })}
        </button>
    }
}

/// How far above its anchor the format bar sits, in CSS pixels.
pub const FORMAT_BAR_LIFT: CssPx = CssPx::new(40.0);

/// Whether a control's mark covers the selection, as `aria-pressed` reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pressed {
    On,
    Mixed,
    Off,
}

impl Pressed {
    /// A two-state toggle's value: `On` or `Off`, never `Mixed`. An absent
    /// `aria-pressed` says "this is not a toggle" (`CHROME-070`), so a toggle
    /// writes `Off` rather than nothing.
    pub const fn from_bool(on: bool) -> Self {
        if on { Self::On } else { Self::Off }
    }

    /// The `aria-pressed` value.
    pub const fn aria(self) -> &'static str {
        match self {
            Self::On => "true",
            Self::Mixed => "mixed",
            Self::Off => "false",
        }
    }
}

/// One control of the format bar: what it announces, whether its mark covers
/// the selection, and what a press reports.
#[derive(Clone)]
pub struct FormatControl {
    pub label: String,
    pub pressed: Pressed,
    pub on_press: Callback<()>,
}

/// The format bar over a selection, centred on `anchor` in the viewport
/// (`CHROME-105`). A press keeps the selection: the bar never takes the focus.
#[component]
pub fn FormatBar(anchor: CssPoint, controls: Vec<FormatControl>) -> impl IntoView {
    let style = format!(
        "position: fixed; left: {}px; top: {}px; transform: translateX(-50%); z-index: 62;",
        anchor.x.get(),
        (anchor.y - FORMAT_BAR_LIFT).max(CssPx::ZERO).get()
    );
    let controls = controls.into_iter().map(|control| {
        let on_press = control.on_press;
        let text = control.label.clone();
        view! {
            <button
                class="ambre-format-control"
                aria-pressed=control.pressed.aria()
                aria-label=control.label
                on:mousedown=|ev: web_sys::MouseEvent| ev.prevent_default()
                on:click=move |_| on_press.run(())
            >
                {text}
            </button>
        }
    });
    view! {
        <div class="ambre-format-bar" style=style role="toolbar" aria-label="Format">
            {controls.collect_view()}
        </div>
    }
}
