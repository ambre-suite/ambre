// ambre-trame/src/chrome/panel.rs
// ============================================================================
// The sheet's sections of the context panel (`ambre-spec-shell.md` section
// 2), wasm-only. The shell owns the frame; this module writes the content of
// each section, and `ui::panel` holds what each one shows.
//
// The controls are the ones the docked format toolbar held, moved here
// whole. Bold, italic and merge go through the shared command path; the other
// style controls emit a `StylePatch` routed as `SetCellStyle`
// (`trame-cell-styles`). Each control carries the id it reaches.
// ============================================================================

use crate::message::{DecimalsStep, StylePatch};
use ambre_chrome::commands::Reach;
use ambre_chrome::{
    CommandButton, Field, FontSizeSelect, Labelled, PanelContent, PanelSection, Pressed, Row,
};
use ambre_core::Flag as _;
use ambre_core::{Commands, FontSize, Rgb, StaticKey};
use leptos::prelude::*;
use leptos_icons::Icon;

use super::color::{theme_bg_default, theme_fg_default};
use super::popovers::{
    BordersPopover, ColorPopover, FontPopover, HAlignPopover, NumberFormatPopover, VAlignPopover,
};
use super::style::{Flip, StyleState};
use crate::ui::commands as registry;
use crate::ui::panel;

/// What the sheet's panel reads and where its controls write.
#[derive(Clone, Copy)]
pub struct TramePanelSignals {
    /// The style of the active cell.
    pub style: StyleState,
    /// The light or dark theme, for the colour pickers' defaults.
    pub theme: RwSignal<ambre_canvas::Theme>,
    /// The shared command sink.
    pub on_command: Callback<StaticKey<Commands>>,
    /// The style patch sink.
    pub on_style: Callback<StylePatch>,
}

/// The command the merge toggle sends: a merged cell splits, any other cell
/// merges the selection.
fn merge_command(merged: crate::message::Merge) -> StaticKey<Commands> {
    match merged {
        crate::message::Merge::Single => registry::MERGE,
        crate::message::Merge::Merged => registry::UNMERGE,
    }
}

/// The `data-reach` word of a property.
fn reach(id: StaticKey<ambre_core::Properties>) -> String {
    Reach::Property(id).marker()
}

/// The title and the sections of the sheet's context panel.
pub fn trame_panel(signals: TramePanelSignals) -> PanelContent {
    let TramePanelSignals {
        style,
        theme,
        on_command,
        on_style,
    } = signals;
    let command = move |id: StaticKey<Commands>| {
        ambre_chrome::commands::find(registry::COMMANDS, id).map(|c| {
            let icon = c.icon.and_then(super::icons::resolve);
            view! { <CommandButton command=c icon=icon on_command=on_command labelled=Labelled::Text /> }
        })
    };
    let commands = move |ids: &'static [StaticKey<Commands>]| {
        ids.iter().filter_map(|&id| command(id)).collect_view()
    };

    let sheet = move || view! { <Row>{commands(&panel::SHEET_COMMANDS)}</Row> }.into_any();

    let cells = move || {
        let toggle = move |id, pressed: Signal<Pressed>| {
            ambre_chrome::commands::find(registry::COMMANDS, id).map(|c| {
                let icon = c.icon.and_then(super::icons::resolve);
                view! { <CommandButton command=c icon=icon on_command=on_command pressed=pressed /> }
            })
        };
        view! {
            <Field label="Format">
                <span class="ambre-kit-control" data-reach=reach(panel::NUMBER_FORMAT)>
                    <NumberFormatPopover style=style on_style=on_style />
                </span>
                <span class="ambre-kit-control" data-reach=reach(panel::DECIMALS)>
                    <button
                        aria-label="Decrease decimals"
                        disabled=move || !style.format.get().has_decimals()
                        on:mousedown=move |ev| ev.prevent_default()
                        on:click=move |_| on_style.run(StylePatch { decimals_step: Some(DecimalsStep::FEWER), ..Default::default() })
                    >
                        <Icon icon=icondata::LuDecimalsArrowLeft />
                    </button>
                    <button
                        aria-label="Increase decimals"
                        disabled=move || !style.format.get().has_decimals()
                        on:mousedown=move |ev| ev.prevent_default()
                        on:click=move |_| on_style.run(StylePatch { decimals_step: Some(DecimalsStep::MORE), ..Default::default() })
                    >
                        <Icon icon=icondata::LuDecimalsArrowRight />
                    </button>
                </span>
            </Field>
            <Field label="Font">
                <span class="ambre-kit-control" data-reach=reach(panel::FONT)>
                    <FontPopover style=style on_style=on_style />
                </span>
                <span class="ambre-kit-control" data-reach=reach(panel::SIZE)>
                    <FontSizeSelect
                        current=Signal::derive(move || style.size.get())
                        on_pick=Callback::new(move |size: Option<FontSize>| {
                            on_style.run(StylePatch { size: Some(size), ..Default::default() });
                        })
                        auto_label="Default"
                    />
                </span>
            </Field>
            <Row>
                {toggle(registry::BOLD, Signal::derive(move || Pressed::from_bool(style.bold.get().as_bool())))}
                {toggle(registry::ITALIC, Signal::derive(move || Pressed::from_bool(style.italic.get().as_bool())))}
                <span class="ambre-kit-control" data-reach=reach(panel::UNDERLINE)>
                    <ambre_chrome::FormatToggle
                        label="Underline"
                        icon=icondata::BsTypeUnderline
                        pressed=Signal::derive(move || Pressed::from_bool(style.underline.get().as_bool()))
                        on_toggle=Callback::new(move |()| on_style.run(StylePatch {
                            underline: Some(style.underline.get_untracked().flipped()),
                            ..Default::default()
                        }))
                    />
                </span>
                <span class="ambre-kit-control" data-reach=reach(panel::STRIKETHROUGH)>
                    <ambre_chrome::FormatToggle
                        label="Strikethrough"
                        icon=icondata::BsTypeStrikethrough
                        pressed=Signal::derive(move || Pressed::from_bool(style.strikethrough.get().as_bool()))
                        on_toggle=Callback::new(move |()| on_style.run(StylePatch {
                            strikethrough: Some(style.strikethrough.get_untracked().flipped()),
                            ..Default::default()
                        }))
                    />
                </span>
            </Row>
            <Field label="Colours">
                <span class="ambre-kit-control" data-reach=reach(panel::TEXT_COLOUR)>
                    <ColorPopover
                        title="Text colour"
                        icon=icondata::BiFontColorRegular
                        current=style.fg
                        default=Signal::derive(move || theme_fg_default(theme.get()))
                        reset_label="Automatic"
                        on_pick=Callback::new(move |fg: Option<Rgb>| {
                            on_style.run(StylePatch { fg: Some(fg), ..Default::default() })
                        })
                    />
                </span>
                <span class="ambre-kit-control" data-reach=reach(panel::FILL)>
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
            </Field>
            <Field label="Alignment">
                <span class="ambre-kit-control" data-reach=reach(panel::H_ALIGN)>
                    <HAlignPopover style=style on_style=on_style />
                </span>
                <span class="ambre-kit-control" data-reach=reach(panel::V_ALIGN)>
                    <VAlignPopover style=style on_style=on_style />
                </span>
                <span class="ambre-kit-control" data-reach=reach(panel::WRAP)>
                    <button
                        aria-label="Wrap text"
                        aria-pressed=move || Pressed::from_bool(style.wrap.get().as_bool()).aria()
                        on:mousedown=move |ev| ev.prevent_default()
                        on:click=move |_| on_style.run(StylePatch {
                            wrap: Some(style.wrap.get_untracked().flipped()),
                            ..Default::default()
                        })
                    >
                        <Icon icon=icondata::BsTextWrap />
                    </button>
                </span>
            </Field>
            <Field label="Borders">
                <span class="ambre-kit-control" data-reach=reach(panel::BORDERS)>
                    <BordersPopover style=style on_style=on_style />
                </span>
            </Field>
            <Row>
                // The label and the reach name the command the click will
                // send, so a merged cell announces "Unmerge cells".
                {move || {
                    let id = merge_command(style.merged.get());
                    let pressed = Signal::derive(move || Pressed::from_bool(style.merged.get().as_bool()));
                    ambre_chrome::commands::find(registry::COMMANDS, id).map(|c| {
                        let icon = registry::icon(registry::MERGE).and_then(super::icons::resolve);
                        view! { <CommandButton command=c icon=icon on_command=on_command pressed=pressed /> }
                    })
                }}
            </Row>
        }
        .into_any()
    };

    let rows_cols = move || view! { <Row>{commands(&panel::ROW_COL_COMMANDS)}</Row> }.into_any();
    let sort_filter =
        move || view! { <Row>{commands(&panel::SORT_FILTER_COMMANDS)}</Row> }.into_any();

    let sections = Signal::derive(move || {
        vec![
            PanelSection::new(panel::SHEET, "Sheet", icondata::MdiTableLarge, sheet),
            PanelSection::new(panel::CELLS, "Cells", icondata::MdiFormatPaint, cells),
            PanelSection::new(
                panel::ROWS_COLS,
                "Rows and columns",
                icondata::MdiTableRowPlusAfter,
                rows_cols,
            ),
            PanelSection::new(
                panel::SORT_FILTER,
                "Sort and filter",
                icondata::MdiFilterOutline,
                sort_filter,
            ),
        ]
    });
    PanelContent {
        title: Signal::stored("Sheet".to_owned()),
        sections,
    }
}
