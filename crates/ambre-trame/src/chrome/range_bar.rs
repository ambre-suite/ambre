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
use ambre_chrome::{ChoiceRow, ChoiceView, FormatBar, IconButton, Pressed, Select};
use ambre_core::Flag as _;
use ambre_core::index::Pos;
use ambre_core::{Commands, Rgb, StaticKey};
use leptos::prelude::*;

use super::color::theme_bg_default;
use super::popovers::{ColorPopover, NUMBER_FORMATS};
use super::style::StyleState;
use crate::message::StylePatch;
use crate::ui::commands as registry;
use crate::ui::panel;

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
                    reach=Reach::Command(id)
                    on_press=Callback::new(move |()| on_command.run(id))
                />
            }
        })
    };
    move || {
        let at = super::handler::client_anchor(Some(anchor.get()?.at));
        let format_place = Signal::derive(move || {
            let format = style.format.get();
            NUMBER_FORMATS
                .iter()
                .position(|(f, _)| *f == format)
                .map(Pos::<ChoiceRow>::from_usize)
        });
        let merge = move || {
            let id = match style.merged.get() {
                crate::message::Merge::Single => registry::MERGE,
                crate::message::Merge::Merged => registry::UNMERGE,
            };
            command(id, None)
        };
        Some(view! {
            <FormatBar anchor=at>
                {command(
                    registry::BOLD,
                    Some(Signal::derive(move || Pressed::from_bool(style.bold.get().as_bool()))),
                )}
                <span class="ambre-kit-control" data-reach=Reach::Property(panel::FILL).marker()>
                    <ColorPopover
                        title="Fill colour"
                        icon=icondata::BiColorFillSolid
                        current=style.bg
                        default=Signal::derive(move || theme_bg_default(theme.get()))
                        reset_label="No fill (transparent)"
                        on_pick=Callback::new(move |bg: Option<Rgb>| {
                            on_style.run(StylePatch { bg: Some(bg), ..Default::default() })
                        })
                    />
                </span>
                <Select
                    label="Number format"
                    choices=NUMBER_FORMATS.iter().map(|(_, l)| ChoiceView::text(*l)).collect()
                    selected=format_place
                    on_pick=Callback::new(move |p: Pos<ChoiceRow>| {
                        if let Some((f, _)) = p.to_index().and_then(|i| NUMBER_FORMATS.get(i)) {
                            on_style.run(StylePatch { format: Some(f.clone()), ..Default::default() });
                        }
                    })
                    placeholder="Custom"
                    reach=Reach::Property(panel::NUMBER_FORMAT)
                />
                {merge}
                {command(registry::SORT_RANGE_ASC, None)}
                {command(registry::SORT_RANGE_DESC, None)}
            </FormatBar>
        })
    }
}
