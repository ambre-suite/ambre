// ambre-trame/src/chrome/panel.rs
// ============================================================================
// The sheet's sections of the context panel (`ambre-spec-shell.md` section
// 2), wasm-only. The shell owns the frame; this module writes the content of
// each section, and `ui::panel` holds what each one shows.
//
// Every control is a primitive of the chrome toolkit. A list of commands
// names each command on its choice (`CHROME-207`). The panel then reaches the
// ids the context menu and the palette reach. Bold, italic and
// merge go through the shared command path; the other style controls emit a
// `StylePatch` routed as `SetCellStyle` (`trame-cell-styles`).
// ============================================================================

use crate::message::{DecimalsStep, StylePatch};
use ambre_chrome::commands::Reach;
use ambre_chrome::{
    Availability, ChoiceRow, ChoiceView, CommandButton, Field, FontSizeSelect, IconButton,
    Labelled, PanelContent, PanelSection, Pressed, Row, SegmentBar, Select,
};
use ambre_core::Flag as _;
use ambre_core::index::Pos;
use ambre_core::{Commands, FontSize, Rgb, StaticKey};
use leptos::prelude::*;

use super::color::{theme_bg_default, theme_fg_default};
use super::popovers::{
    BordersPopover, ColorPopover, FontPopover, H_ALIGNS, NUMBER_FORMATS, V_ALIGNS,
};
use super::style::{Flip, StyleState};
use crate::ui::commands as registry;
use crate::ui::panel;
use crate::ui::panel::{ActiveSheet, PanelSelection};

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
    /// The reference of the selection and the cells it spans, which the
    /// header names and counts.
    pub selection: RwSignal<PanelSelection>,
    /// The page setup and the frozen pane of the active sheet.
    pub sheet: RwSignal<Option<ActiveSheet>>,
    /// The sheets of the workbook, for the name of the active one.
    pub sheets: RwSignal<Vec<crate::message::SheetTab>>,
    /// The active sheet.
    pub active_sheet: RwSignal<ambre_core::Id<ambre_core::id::Sheet>>,
    /// The rename sink, the one the sheet tabs use.
    pub on_rename: Callback<(
        ambre_core::Id<ambre_core::id::Sheet>,
        crate::message::SheetName,
    )>,
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

/// The label of the property `id`, from its description.
/// The title of the property `id` in the interface language, which a field
/// shows as its label: the field reads the words of its property.
fn title(id: StaticKey<ambre_core::Properties>) -> &'static str {
    panel::PROPERTIES
        .iter()
        .find(|p| p.id == id)
        .map_or("", |p| ambre_commands::shown(&p.title))
}

fn label(id: StaticKey<ambre_core::Properties>) -> String {
    panel::PROPERTIES
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.title.pick(&ambre_chrome::language()).into_string())
        .unwrap_or_default()
}

/// One choice per command of `ids`, each labelled by `labels` and naming
/// the command its pick runs.
fn command_choices(ids: &[StaticKey<Commands>], labels: &[&str]) -> Vec<ChoiceView> {
    ids.iter()
        .zip(labels)
        .map(|(&id, &text)| ChoiceView::text(text).reaching(Reach::Command(id)))
        .collect()
}

/// The icon choice of the command `id`. Its tooltip names the command as the
/// registry describes it (`CHROME-186`).
fn icon_choice(id: StaticKey<Commands>, icon: icondata::Icon) -> ChoiceView {
    let name = registry::title(id, &ambre_chrome::language())
        .map(ambre_core::Validated::into_string)
        .unwrap_or_default();
    ChoiceView::icon(name, icon).reaching(Reach::Command(id))
}

/// A pick that runs the command at its place in `ids`.
fn run_place(
    ids: &'static [StaticKey<Commands>],
    on_command: Callback<StaticKey<Commands>>,
) -> Callback<Pos<ChoiceRow>> {
    Callback::new(move |p: Pos<ChoiceRow>| {
        if let Some(&id) = p.to_index().and_then(|i| ids.get(i)) {
            on_command.run(id);
        }
    })
}

/// The number format list of the cells, which the panel and the range bar
/// share: one copy of the view serves both (`CHROME-246`).
pub(super) fn number_format(style: StyleState, on_style: Callback<StylePatch>) -> AnyView {
    let selected = Signal::derive(move || {
        let format = style.format.get();
        NUMBER_FORMATS
            .iter()
            .position(|(f, _)| *f == format)
            .map(Pos::<ChoiceRow>::from_usize)
    });
    let pick = Callback::new(move |p: Pos<ChoiceRow>| {
        if let Some((f, _)) = p.to_index().and_then(|i| NUMBER_FORMATS.get(i)) {
            on_style.run(StylePatch {
                format: Some(f.clone()),
                ..Default::default()
            });
        }
    });
    view! {
        <Select
            label=label(panel::NUMBER_FORMAT)
            choices=NUMBER_FORMATS.iter().map(|(_, l)| ChoiceView::text(ambre_commands::shown(l))).collect()
            selected=selected
            on_pick=pick
            placeholder=ambre_commands::tr!("Custom", "Personnalis\u{e9}")
            reach=Reach::Property(panel::NUMBER_FORMAT)
        />
    }
    .into_any()
}

/// The fill of the cells, which the panel and the range bar share.
pub(super) fn fill(
    style: StyleState,
    theme: RwSignal<ambre_canvas::Theme>,
    on_style: Callback<StylePatch>,
) -> AnyView {
    view! {
        <span class="ambre-kit-control" data-reach=reach(panel::FILL)>
            <ColorPopover
                title=label(panel::FILL)
                icon=icondata::BiColorFillSolid
                current=style.bg
                default=Signal::derive(move || theme_bg_default(theme.get()))
                reset_label=ambre_commands::tr!("No fill (transparent)", "Aucun remplissage (transparent)")
                on_pick=Callback::new(move |bg: Option<Rgb>| {
                    on_style.run(StylePatch { bg: Some(bg), ..Default::default() })
                })
            />
        </span>
    }
    .into_any()
}

/// The title and the sections of the sheet's context panel.
pub fn trame_panel(signals: TramePanelSignals) -> PanelContent {
    let TramePanelSignals {
        style,
        theme,
        on_command,
        on_style,
        selection,
        sheet,
        sheets,
        active_sheet,
        on_rename,
    } = signals;
    // The place in `ids` of the choice the active sheet reads.
    let place = move |ids: &'static [StaticKey<Commands>],
                      f: fn(ActiveSheet) -> Option<StaticKey<Commands>>| {
        Signal::derive(move || {
            let id = sheet.get().and_then(f)?;
            ids.iter()
                .position(|&c| c == id)
                .map(Pos::<ChoiceRow>::from_usize)
        })
    };

    let sheet_section = move || {
        // The name of the active sheet. A name the rule refuses keeps its
        // reason on the field and renames nothing (`TRAME-234`).
        let name = move || {
            let id = active_sheet.get();
            sheets.with(|all| {
                all.iter()
                    .find(|s| s.id == id)
                    .map(|s| s.name.as_str().to_owned())
                    .unwrap_or_default()
            })
        };
        view! {
            <Field label=ambre_commands::tr!("Name", "Nom")>
                <input
                    class="ambre-kit-text"
                    type="text"
                    aria-label=label(panel::SHEET_NAME)
                    data-reach=reach(panel::SHEET_NAME)
                    prop:value=name
                    on:change=move |ev| {
                        let input = event_target::<web_sys::HtmlInputElement>(&ev);
                        match crate::message::SheetName::new(&input.value()) {
                            Ok(next) => {
                                let _ = input.remove_attribute("aria-invalid");
                                input.set_title("");
                                on_rename.run((active_sheet.get_untracked(), next));
                            }
                            Err(refusal) => {
                                input.set_title(&refusal.to_string());
                                let _ = input.set_attribute("aria-invalid", "true");
                            }
                        }
                    }
                />
            </Field>
            <Field label=title(panel::FREEZE)>
                <Select
                    label=label(panel::FREEZE)
                    choices=command_choices(
                        &panel::FREEZE_COMMANDS,
                        &[ambre_commands::tr!("None", "Aucun"), ambre_commands::tr!("Up to this row", "Jusqu'\u{e0} cette ligne"), ambre_commands::tr!("Up to this column", "Jusqu'\u{e0} cette colonne")],
                    )
                    selected=place(&panel::FREEZE_COMMANDS, |s| panel::freeze_choice(s.frozen))
                    on_pick=run_place(&panel::FREEZE_COMMANDS, on_command)
                    placeholder=ambre_commands::tr!("Rows and columns", "Lignes et colonnes")
                    reach=Reach::Property(panel::FREEZE)
                />
            </Field>
        }
        .into_any()
    };

    let print_section = move || {
        view! {
            <Field label=title(panel::PAPER)>
                <Select
                    label=label(panel::PAPER)
                    choices=command_choices(&panel::PAPER_COMMANDS, &["A4", "A3", ambre_commands::tr!("Letter", "Lettre US"), ambre_commands::tr!("Legal", "L\u{e9}gal US")])
                    selected=place(&panel::PAPER_COMMANDS, |s| Some(panel::paper_choice(s.page.paper)))
                    on_pick=run_place(&panel::PAPER_COMMANDS, on_command)
                    reach=Reach::Property(panel::PAPER)
                />
            </Field>
            <Field label=title(panel::ORIENTATION)>
                <SegmentBar
                    label=label(panel::ORIENTATION)
                    choices=vec![
                        icon_choice(registry::PAGE_PORTRAIT, icondata::LuRectangleVertical),
                        icon_choice(registry::PAGE_LANDSCAPE, icondata::LuRectangleHorizontal),
                    ]
                    selected=place(&panel::ORIENTATION_COMMANDS, |s| Some(panel::orientation_choice(s.page.orientation)))
                    on_pick=run_place(&panel::ORIENTATION_COMMANDS, on_command)
                    reach=Reach::Property(panel::ORIENTATION)
                />
            </Field>
            <Field label=title(panel::MARGINS)>
                <Select
                    label=label(panel::MARGINS)
                    choices=command_choices(&panel::MARGIN_COMMANDS, &[ambre_commands::tr!("Normal", "Normales"), ambre_commands::tr!("Narrow", "\u{c9}troites"), ambre_commands::tr!("Wide", "Larges")])
                    selected=place(&panel::MARGIN_COMMANDS, |s| panel::margin_choice(s.page.margins))
                    on_pick=run_place(&panel::MARGIN_COMMANDS, on_command)
                    placeholder=ambre_commands::tr!("Custom", "Personnalis\u{e9}")
                    reach=Reach::Property(panel::MARGINS)
                />
            </Field>
        }
        .into_any()
    };

    let cells = move || {
        let toggle = move |id, pressed: Signal<Pressed>| {
            ambre_chrome::commands::find(registry::COMMANDS, id).map(|c| {
                let icon = c.icon.and_then(super::icons::resolve);
                view! { <CommandButton command=c icon=icon on_command=on_command pressed=pressed /> }
            })
        };
        let decimals = Signal::derive(move || {
            if style.format.get().has_decimals() {
                Availability::Enabled
            } else {
                Availability::Disabled
            }
        });
        let h_place = Signal::derive(move || {
            let at = style.h_align.get();
            H_ALIGNS
                .iter()
                .position(|(a, _, _)| *a == at)
                .map(Pos::<ChoiceRow>::from_usize)
        });
        let v_place = Signal::derive(move || {
            let at = style.v_align.get();
            V_ALIGNS
                .iter()
                .position(|(a, _, _)| *a == at)
                .map(Pos::<ChoiceRow>::from_usize)
        });
        view! {
            <Field label=ambre_commands::tr!("Number", "Nombre")>{number_format(style, on_style)}</Field>
            <Field label=title(panel::DECIMALS)>
                <span class="ambre-kit-control" data-reach=reach(panel::DECIMALS)>
                    <IconButton
                        icon=icondata::LuDecimalsArrowLeft
                        label=ambre_commands::tr!("Fewer decimals", "Moins de d\u{e9}cimales")
                        availability=decimals
                        on_press=Callback::new(move |()| on_style.run(StylePatch { decimals_step: Some(DecimalsStep::FEWER), ..Default::default() }))
                    />
                    <IconButton
                        icon=icondata::LuDecimalsArrowRight
                        label=ambre_commands::tr!("More decimals", "Plus de d\u{e9}cimales")
                        availability=decimals
                        on_press=Callback::new(move |()| on_style.run(StylePatch { decimals_step: Some(DecimalsStep::MORE), ..Default::default() }))
                    />
                </span>
            </Field>
            <Field label=title(panel::FONT)>
                <span class="ambre-kit-control" data-reach=reach(panel::FONT)>
                    <FontPopover style=style on_style=on_style />
                </span>
            </Field>
            <Field label=title(panel::SIZE)>
                <span class="ambre-kit-control" data-reach=reach(panel::SIZE)>
                    <FontSizeSelect
                        current=Signal::derive(move || style.size.get())
                        on_pick=Callback::new(move |size: Option<FontSize>| {
                            on_style.run(StylePatch { size: Some(size), ..Default::default() });
                        })
                        auto_label=ambre_commands::tr!("Default", "Par d\u{e9}faut")
                    />
                </span>
            </Field>
            <Field label=ambre_commands::tr!("Style", "Style")>
                {toggle(registry::BOLD, Signal::derive(move || Pressed::from_bool(style.bold.get().as_bool())))}
                {toggle(registry::ITALIC, Signal::derive(move || Pressed::from_bool(style.italic.get().as_bool())))}
                <span class="ambre-kit-control" data-reach=reach(panel::UNDERLINE)>
                    <IconButton
                        icon=icondata::BsTypeUnderline
                        label=title(panel::UNDERLINE)
                        pressed=Some(Signal::derive(move || Pressed::from_bool(style.underline.get().as_bool())))
                        on_press=Callback::new(move |()| on_style.run(StylePatch {
                            underline: Some(style.underline.get_untracked().flipped()),
                            ..Default::default()
                        }))
                    />
                </span>
                <span class="ambre-kit-control" data-reach=reach(panel::STRIKETHROUGH)>
                    <IconButton
                        icon=icondata::BsTypeStrikethrough
                        label=title(panel::STRIKETHROUGH)
                        pressed=Some(Signal::derive(move || Pressed::from_bool(style.strikethrough.get().as_bool())))
                        on_press=Callback::new(move |()| on_style.run(StylePatch {
                            strikethrough: Some(style.strikethrough.get_untracked().flipped()),
                            ..Default::default()
                        }))
                    />
                </span>
            </Field>
            <Field label=ambre_commands::tr!("Colours", "Couleurs")>
                <span class="ambre-kit-control" data-reach=reach(panel::TEXT_COLOUR)>
                    <ColorPopover
                        title=label(panel::TEXT_COLOUR)
                        icon=icondata::BiFontColorRegular
                        current=style.fg
                        default=Signal::derive(move || theme_fg_default(theme.get()))
                        reset_label=ambre_commands::tr!("Automatic", "Automatique")
                        on_pick=Callback::new(move |fg: Option<Rgb>| {
                            on_style.run(StylePatch { fg: Some(fg), ..Default::default() })
                        })
                    />
                </span>
                {fill(style, theme, on_style)}
            </Field>
            <Field label=ambre_commands::tr!("Horizontal", "Horizontal")>
                <SegmentBar
                    label=label(panel::H_ALIGN)
                    choices=H_ALIGNS.iter().map(|(_, i, l)| ChoiceView::icon(ambre_commands::shown(l), i)).collect()
                    selected=h_place
                    on_pick=Callback::new(move |p: Pos<ChoiceRow>| {
                        if let Some((a, _, _)) = p.to_index().and_then(|i| H_ALIGNS.get(i)) {
                            on_style.run(StylePatch { h_align: Some(*a), ..Default::default() });
                        }
                    })
                    reach=Reach::Property(panel::H_ALIGN)
                />
            </Field>
            <Field label=ambre_commands::tr!("Vertical", "Vertical")>
                <SegmentBar
                    label=label(panel::V_ALIGN)
                    choices=V_ALIGNS.iter().map(|(_, i, l)| ChoiceView::icon(ambre_commands::shown(l), i)).collect()
                    selected=v_place
                    on_pick=Callback::new(move |p: Pos<ChoiceRow>| {
                        if let Some((a, _, _)) = p.to_index().and_then(|i| V_ALIGNS.get(i)) {
                            on_style.run(StylePatch { v_align: Some(*a), ..Default::default() });
                        }
                    })
                    reach=Reach::Property(panel::V_ALIGN)
                />
                <span class="ambre-kit-control" data-reach=reach(panel::WRAP)>
                    <IconButton
                        icon=icondata::BsTextWrap
                        label=title(panel::WRAP)
                        pressed=Some(Signal::derive(move || Pressed::from_bool(style.wrap.get().as_bool())))
                        on_press=Callback::new(move |()| on_style.run(StylePatch {
                            wrap: Some(style.wrap.get_untracked().flipped()),
                            ..Default::default()
                        }))
                    />
                </span>
            </Field>
            <Field label=title(panel::BORDERS)>
                <span class="ambre-kit-control" data-reach=reach(panel::BORDERS)>
                    <BordersPopover style=style on_style=on_style />
                </span>
            </Field>
            // The label and the reach name the command the click will send,
            // so a merged cell announces "Unmerge cells".
            <Row>
            {move || {
                let id = merge_command(style.merged.get());
                ambre_chrome::commands::find(registry::COMMANDS, id).map(|c| {
                    let icon = registry::icon(registry::MERGE).and_then(super::icons::resolve);
                    view! { <CommandButton command=c icon=icon on_command=on_command labelled=Labelled::Text /> }
                })
            }}
            </Row>
        }
        .into_any()
    };

    // A labelled row per command (`CHROME-249`): the name reads at a glance,
    // where a row of bare icons asks the user to hover each one.
    let labelled = move |id: StaticKey<Commands>, icon_of: StaticKey<Commands>| {
        ambre_chrome::commands::find(registry::COMMANDS, id).map(|c| {
            let icon = registry::icon(icon_of).and_then(super::icons::resolve);
            view! { <CommandButton command=c icon=icon on_command=on_command labelled=Labelled::Text /> }
        })
    };
    let rows = move |ids: &[StaticKey<Commands>]| {
        ids.iter().filter_map(|&id| labelled(id, id)).collect_view()
    };
    let rows_cols = move || {
        view! {
            <Row>
                {rows(&[
                    registry::INSERT_ROW_ABOVE,
                    registry::INSERT_ROW_BELOW,
                    registry::INSERT_COL_LEFT,
                    registry::INSERT_COL_RIGHT,
                    registry::DELETE_ROWS,
                    registry::DELETE_COLS,
                ])}
            </Row>
        }
        .into_any()
    };
    let sort_filter = move || {
        view! {
            <Row>
                {rows(&[registry::SORT_RANGE_ASC, registry::SORT_RANGE_DESC, registry::SORT_RANGE_MULTI])}
                // The two sheet sorts declare one icon; the direction reads
                // from the range sorts' arrows.
                {labelled(registry::SORT_SHEET_ASC, registry::SORT_RANGE_ASC)}
                {labelled(registry::SORT_SHEET_DESC, registry::SORT_RANGE_DESC)}
                {rows(&[registry::FILTER_COLUMN, registry::CLEAR_FILTER])}
            </Row>
        }
        .into_any()
    };

    // The shape of the selection picks the sections, never its content
    // (`CHROME-253`).
    let shape = Memo::new(move |_| selection.with(|s| panel::SelectionShape::of(&s.reference)));
    let sections = Signal::derive(move || {
        shape
            .get()
            .sections()
            .iter()
            .filter_map(|&key| {
                Some(match key {
                    k if k == panel::SHEET => PanelSection::new(
                        k,
                        ambre_commands::tr!("Sheet", "Feuille"),
                        icondata::MdiTableLarge,
                        sheet_section,
                    ),
                    k if k == panel::CELLS => PanelSection::new(
                        k,
                        ambre_commands::tr!("Cells", "Cellules"),
                        icondata::MdiFormatPaint,
                        cells,
                    ),
                    k if k == panel::ROWS_COLS => PanelSection::new(
                        k,
                        ambre_commands::tr!("Rows and columns", "Lignes et colonnes"),
                        icondata::MdiTableRowPlusAfter,
                        rows_cols,
                    ),
                    k if k == panel::SORT_FILTER => PanelSection::new(
                        k,
                        ambre_commands::tr!("Sort and filter", "Trier et filtrer"),
                        icondata::MdiFilterOutline,
                        sort_filter,
                    ),
                    k if k == panel::PRINT => PanelSection::new(
                        k,
                        ambre_commands::tr!("Print", "Imprimer"),
                        icondata::LuPrinter,
                        print_section,
                    )
                    .folded_first(),
                    _ => return None,
                })
            })
            .collect::<Vec<_>>()
    });
    PanelContent {
        title: Signal::derive(move || selection.with(|s| s.reference.as_str().to_owned())),
        subtitle: Signal::derive(move || selection.with(PanelSelection::subtitle)),
        sections,
    }
}
