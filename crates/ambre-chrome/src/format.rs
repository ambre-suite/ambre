// ambre-chrome/src/format.rs
// ============================================================================
// Shared, app-agnostic text-format controls (ambre-spec-chrome.md). The rest of the
// text-format cluster promoted out of Trame's cell toolbar. Trame cells and Toile
// elements drive the SAME controls (mirrors `FontPicker` / `ColorPopover`):
//
//   FontSizeSelect -- the point-size list (a curated preset list + an "unset"
//                     entry), value-agnostic (`Option<FontSize>`, `None` = the unset /
//                     auto entry). The caller maps `None` to its own meaning
//                     (Trame: the body default; Toile: auto-size to the card).
//   FormatToggle   -- a single pressed-state toolbar button (bold / italic /
//                     underline / strikethrough), reflecting a `pressed` signal
//                     and reporting clicks through `on_toggle`. The caller owns
//                     what a toggle means to its model (a `StaticKey<Commands>`, a style
//                     patch, ...): the control never reaches past the DOM.
//
//   FormatBar      -- the bar of format controls over a selection. Prose and
//                     Murmure hand it the anchor and the controls; it holds
//                     no state of its own.
//
// `FormatToggle` does `on:mousedown prevent_default` so a click never steals focus
// from the host's text pipe (the palette / grid-focus hazard both apps guard
// against). `FontSizeSelect` draws the styled `Select`, which does the same.
// ============================================================================

use ambre_core::FontSize;
use ambre_core::index::Pos;
use ambre_core::{Css, Point, Px};
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

/// The shared font-size list, in the styled [`Select`](crate::Select).
/// `current` is the active explicit size (`None` = unset -> the `auto_label`
/// entry is selected). `on_pick` receives the chosen size (`None` when the
/// unset entry is picked). `auto_label` names the unset entry ("Auto" for
/// Toile's card auto-size, "Default" for Trame's body default).
#[component]
pub fn FontSizeSelect(
    current: Signal<Option<FontSize>>,
    on_pick: Callback<Option<FontSize>>,
    #[prop(default = "Auto")] auto_label: &'static str,
) -> impl IntoView {
    // The unset entry comes first, then each preset in order.
    let choices: Vec<crate::ChoiceView> = std::iter::once(crate::ChoiceView::text(auto_label))
        .chain(
            FONT_SIZES
                .iter()
                .map(|s| crate::ChoiceView::text(s.whole_points().to_string())),
        )
        .collect();
    let selected = Signal::derive(move || {
        let place = match current.get() {
            None => Some(0),
            Some(size) => FONT_SIZES.iter().position(|s| *s == size).map(|i| i + 1),
        };
        place.map(Pos::<crate::ChoiceRow>::from_usize)
    });
    let pick = Callback::new(move |p: Pos<crate::ChoiceRow>| match p.to_index() {
        Some(0) => on_pick.run(None),
        Some(i) => {
            if let Some(size) = i.checked_sub(1).and_then(|i| FONT_SIZES.get(i)) {
                on_pick.run(Some(*size));
            }
        }
        None => {}
    });
    view! {
        <span class="ambre-font-size">
            <crate::Select
                label="Font size"
                choices=choices
                selected=selected
                on_pick=pick
                placeholder="Custom"
            />
        </span>
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
pub const FORMAT_BAR_LIFT: Px<Css> = Px::<Css>::new(40.0);

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
    /// The glyph the control shows in place of its label, which becomes the
    /// tooltip. `None` shows the label.
    pub icon: Option<icondata::Icon>,
    /// The chord the tooltip names beside the label.
    pub chord: Option<ambre_commands::KeyBinding>,
}

/// The format bar over a selection, centred on `anchor` in the viewport
/// (`CHROME-105`). A press keeps the selection: the bar never takes the focus.
#[component]
pub fn FormatBar(anchor: Point<Px<Css>>, controls: Vec<FormatControl>) -> impl IntoView {
    let left = format!("{}px", anchor.x.get());
    let top = format!(
        "{}px",
        (anchor.y - FORMAT_BAR_LIFT).max(Px::<Css>::ZERO).get()
    );
    // An icon control is the one icon button of the toolkit. The bar then
    // shows the glyphs of the context panel, at the size of a bar.
    let controls = controls.into_iter().map(|control| {
        let on_press = control.on_press;
        match control.icon {
            Some(icon) => view! {
                <crate::IconButton
                    icon=icon
                    label=control.label
                    chord=control.chord
                    pressed=Some(Signal::stored(control.pressed))
                    size=crate::ControlSize::Large
                    on_press=Callback::new(move |()| on_press.run(()))
                    {..}
                    class:ambre-format-control=true
                />
            }
            .into_any(),
            None => {
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
                .into_any()
            }
        }
    });
    view! {
        <div
            class="ambre-format-bar"
            style:position="fixed"
            style:left=left
            style:top=top
            style:transform="translateX(-50%)"
            style:z-index="62"
            role="toolbar"
            aria-label="Format"
        >
            {controls.collect_view()}
        </div>
    }
}
