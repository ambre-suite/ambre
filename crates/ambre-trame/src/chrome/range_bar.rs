// ambre-trame/src/chrome/range_bar.rs
// ============================================================================
// The range bar: the format bar of the sheet over a selection of two cells or
// more (`ambre-spec-shell.md` section 2, `CHROME-236`), wasm-only.
//
// It is the shared `FormatBar` of Prose, so the three applications draw one
// bar. Bold, merge and the two sorts go through the shared command path, as
// the context panel sends them. The fill and the number format emit a
// `StylePatch`, as the cells section does. Each entry is a control the panel
// shows for a range (`CHROME-208`), and `ui::panel::BAR` declares the list.
// ============================================================================

use ambre_chrome::commands::Reach;
use ambre_chrome::{FormatBar, IconButton, Pressed};
use ambre_core::Flag as _;
use ambre_core::{Commands, StaticKey};
use leptos::prelude::*;

use super::style::StyleState;
use crate::message::StylePatch;
use crate::ui::commands as registry;

/// The range bar over the published anchor. `None` draws nothing.
#[component]
pub fn RangeBar(
    /// Where the bar sits, in view pixels, as the sheet view publishes it.
    anchor: RwSignal<Option<crate::ui::panel::RangeBarAnchor>>,
    /// The style of the active cell, which the controls read.
    style: StyleState,
    /// The light or dark theme, for the default of the fill.
    theme: RwSignal<ambre_canvas::Theme>,
    /// The shared command sink.
    on_command: Callback<StaticKey<Commands>>,
    /// The style patch sink.
    on_style: Callback<StylePatch>,
) -> impl IntoView {
    // A command button with the glyph, the name and the chord its command
    // declares (`UI-THREAD-019`).
    let command = move |id: StaticKey<Commands>, pressed: Option<Signal<Pressed>>| {
        ambre_chrome::commands::find(registry::COMMANDS, id).map(|c| {
            let label = (c.title)(&ambre_chrome::commands::Locale::En).into_string();
            let icon = registry::icon(id)
                .and_then(super::icons::resolve)
                .unwrap_or(icondata::LuSquare);
            view! {
                <IconButton
                    icon=icon
                    label=label
                    chord=c.chord()
                    pressed=pressed
                    size=ambre_chrome::ControlSize::Large
                    reach=Some(Reach::Command(id))
                    on_press=Callback::new(move |()| on_command.run(id))
                />
            }
        })
    };
    move || {
        let at = super::handler::client_anchor(Some(anchor.get()?.at));
        let merge = move || {
            let id = match style.merged.get() {
                crate::message::Merge::Single => registry::MERGE,
                crate::message::Merge::Merged => registry::UNMERGE,
            };
            command(id, None)
        };
        // One list of erased views: the bar adds no nested view type of its
        // own (`CHROME-246`).
        let bold = Signal::derive(move || Pressed::from_bool(style.bold.get().as_bool()));
        let controls: Vec<AnyView> = vec![
            command(registry::BOLD, Some(bold)).into_any(),
            super::panel::fill(style, theme, on_style),
            super::panel::number_format(style, on_style),
            merge.into_any(),
            command(registry::SORT_RANGE_ASC, None).into_any(),
            command(registry::SORT_RANGE_DESC, None).into_any(),
        ];
        Some(view! { <FormatBar anchor=at>{controls}</FormatBar> })
    }
}
