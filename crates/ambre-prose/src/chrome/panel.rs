// ambre-prose/src/chrome/panel.rs
// ============================================================================
// The page's sections of the context panel (`ambre-spec-shell.md` section 2),
// wasm-only. The shell owns the frame; this module writes the content of each
// section from the kit, and nothing else (`view::panel` holds the rules).
//
// Every control hands its command id or its page format back to the view
// through a signal, as the format bar does. The panel holds no document state,
// and no path of its own into the document (`PROSE-UI-098`).
// ============================================================================

use ambre_chrome::commands::{Reach, Reading};
use ambre_chrome::{
    Choice, ColourField, CommandButton, Field, FontField, FontSizeSelect, Labelled, LengthField,
    PanelContent, PanelSection, Pressed, Row, Segmented, SelectField,
};
use ambre_core::{Mm, PrintLength, Pt, Rgb, Rgba};
use leptos::prelude::*;

use super::ProseSignals;
use crate::doc::{Orientation, PageFormat};
use crate::events::{Align, ParagraphStyle, TriState};
use crate::message::{ParagraphKind, SelectionValues};
use crate::view::panel::{self, Margins, PanelKind, Paper, TextWrite};
use crate::view::toolbar::state_of;

/// The property of `id`. Every id this module names is in the table.
fn prop(
    id: ambre_core::StaticKey<ambre_core::Properties>,
) -> &'static ambre_chrome::commands::Property {
    panel::property(id)
}

/// The title and the sections of the page's context panel.
pub fn prose_panel(signals: ProseSignals) -> PanelContent {
    let kind = Memo::new(move |_| {
        if signals.toolbar.with(|bar| bar.anchor.is_some()) {
            PanelKind::Text
        } else {
            PanelKind::Page
        }
    });
    let page = move || signals.page.get().unwrap_or_default();
    let write = move |format: PageFormat| signals.page_write.set(Some(format));

    let page_section = move || {
        let paper = Signal::derive(move || Reading::Uniform(Paper::of(&page())));
        let orientation = Signal::derive(move || Reading::Uniform(panel::orientation_of(&page())));
        // A pick of custom shows the four steppers before any margin moves;
        // margins no preset names show them too.
        let custom = RwSignal::new(false);
        let margins = Signal::derive(move || {
            if custom.get() {
                Reading::Uniform(Margins::Custom)
            } else {
                Reading::Uniform(Margins::of(&page()))
            }
        });
        let margin_choices = Margins::ALL
            .into_iter()
            .map(|m| Choice {
                value: m,
                label: m.label(),
                icon: None,
            })
            .collect::<Vec<_>>();
        let show_steppers = move || margins.get() == Reading::Uniform(Margins::Custom);
        let margin = move |id,
                           get: fn(&PageFormat) -> PrintLength<Mm>,
                           set: fn(&mut PageFormat, PrintLength<Mm>)| {
            view! {
                <LengthField
                    property=prop(id)
                    value=Signal::derive(move || Reading::Uniform(get(&page())))
                    on_write=Callback::new(move |m| {
                        let mut format = page();
                        set(&mut format, m);
                        write(format);
                    })
                />
            }
        };
        let papers = Paper::NAMED
            .into_iter()
            .chain([Paper::Other])
            .map(|p| Choice {
                value: p,
                label: p.label(),
                icon: None,
            })
            .collect::<Vec<_>>();
        view! {
            <Field label="Paper">
                <SelectField
                    property=prop(panel::PAPER)
                    choices=papers
                    value=paper
                    on_write=Callback::new(move |p| {
                        if let Some(format) = panel::with_paper(page(), p) {
                            write(format);
                        }
                    })
                />
            </Field>
            <Field label="Orientation">
                <Segmented
                    property=prop(panel::ORIENTATION)
                    choices=vec![
                        Choice { value: Orientation::Portrait, label: "Portrait", icon: Some(icondata::LuRectangleVertical) },
                        Choice { value: Orientation::Landscape, label: "Landscape", icon: Some(icondata::LuRectangleHorizontal) },
                    ]
                    value=orientation
                    on_write=Callback::new(move |o| write(panel::with_orientation(page(), o)))
                />
            </Field>
            <Field label="Margins">
                <SelectField
                    property=prop(panel::MARGINS)
                    choices=margin_choices
                    value=margins
                    on_write=Callback::new(move |m| match panel::with_margins(page(), m) {
                        Some(format) => {
                            custom.set(false);
                            write(format);
                        }
                        None => custom.set(true),
                    })
                />
            </Field>
            <Show when=show_steppers>
                <Field label="Top">{margin(panel::MARGIN_TOP, |f| f.margin_top, |f, m| f.margin_top = m)}</Field>
                <Field label="Bottom">{margin(panel::MARGIN_BOTTOM, |f| f.margin_bottom, |f, m| f.margin_bottom = m)}</Field>
                <Field label="Left">{margin(panel::MARGIN_LEFT, |f| f.margin_left, |f, m| f.margin_left = m)}</Field>
                <Field label="Right">{margin(panel::MARGIN_RIGHT, |f| f.margin_right, |f, m| f.margin_right = m)}</Field>
            </Show>
        }
        .into_any()
    };

    // What the Logic thread read for this selection; the defaults before
    // its answer arrives (`PROSE-UI-255`).
    let values = Memo::new(move |_| {
        signals.toolbar.with(|bar| {
            bar.active
                .as_ref()
                .map(|m| m.values.clone())
                .unwrap_or_default()
        })
    });
    let text_write = move |write: TextWrite| signals.text_write.set(Some(write));
    let paragraph = move |style: ParagraphStyle| text_write(TextWrite::Paragraph(style));
    let command_buttons = move |ids: &'static [ambre_core::StaticKey<ambre_core::Commands>],
                                labelled: Labelled| {
        ids.iter()
                .filter_map(|&id| {
                    let command = ambre_commands::find(crate::commands::COMMANDS, id)?;
                    let mark = crate::commands::mark_key(id);
                    let pressed = mark.map(|key| {
                        Signal::derive(move || {
                            match signals
                                .toolbar
                                .with(|bar| bar.active.as_ref().map(|m| state_of(m, key)))
                            {
                                Some(TriState::On) => Pressed::On,
                                Some(TriState::Mixed) => Pressed::Mixed,
                                _ => Pressed::Off,
                            }
                        })
                    });
                    // The link and the comment open their field in the bar;
                    // every other id reaches the view.
                    let on_command = Callback::new(move |id| {
                        if signals.open_prompt(id).is_none() {
                            signals.format.set(Some(id));
                        }
                    });
                    Some(match pressed {
                        Some(pressed) => view! {
                            <CommandButton command=command icon=super::icon_of(command) on_command=on_command pressed=pressed labelled=labelled />
                        }
                        .into_any(),
                        None => view! {
                            <CommandButton command=command icon=super::icon_of(command) on_command=on_command labelled=labelled />
                        }
                        .into_any(),
                    })
                })
                .collect_view()
    };

    let text_section = move || {
        // Each group of the text section carries a label (`CHROME-218`).
        let all: &'static [ambre_core::StaticKey<ambre_core::Commands>] = &panel::TEXT_COMMANDS;
        let (marks, rest) = all.split_at(5);
        let (script, clear) = rest.split_at(2);
        let size = Signal::derive(move || values.with(|v| v.font_size.uniform().flatten()));
        let colour = |reading: Reading<Option<Rgba>>| match reading {
            Reading::Uniform(c) => Reading::Uniform(c.map(Rgba::rgb)),
            Reading::Mixed => Reading::Mixed,
        };
        view! {
            <Field label="Font">
                <FontField
                    property=prop(panel::FONT)
                    value=Signal::derive(move || values.with(|v| v.font_family.clone()))
                    on_write=Callback::new(move |f| text_write(TextWrite::Font(f)))
                />
            </Field>
            <Field label="Size">
                <span data-reach=Reach::Property(panel::SIZE).marker()>
                    <FontSizeSelect
                        current=size
                        on_pick=Callback::new(move |s| text_write(TextWrite::Size(s)))
                        auto_label="Default"
                    />
                </span>
            </Field>
            <Field label="Style">{command_buttons(marks, Labelled::Icon)}</Field>
            <Field label="Colour">
                <ColourField
                    property=prop(panel::COLOUR)
                    icon=icondata::MdiFormatColorText
                    value=Signal::derive(move || colour(values.with(|v| v.color)))
                    default=Signal::stored(Rgb::from_bits(0x29_241e))
                    reset_label="Automatic"
                    on_write=Callback::new(move |c| text_write(TextWrite::Colour(c)))
                />
            </Field>
            <Field label="Highlight">
                <ColourField
                    property=prop(panel::HIGHLIGHT)
                    icon=icondata::MdiFormatColorHighlight
                    value=Signal::derive(move || colour(values.with(|v| v.highlight)))
                    default=Signal::stored(Rgb::from_bits(0xff_ff00))
                    reset_label="No highlight"
                    on_write=Callback::new(move |c| text_write(TextWrite::Highlight(c)))
                />
            </Field>
            <Field label="Script">{command_buttons(script, Labelled::Icon)}</Field>
            <Field label="Clear">{command_buttons(clear, Labelled::Icon)}</Field>
        }
        .into_any()
    };

    let paragraph_section = move || {
        let indent = move |id,
                           get: fn(&SelectionValues) -> Reading<PrintLength<Pt>>,
                           field: fn(PrintLength<Pt>) -> ParagraphStyle| {
            view! {
                <LengthField
                    property=prop(id)
                    value=Signal::derive(move || match values.with(get) {
                        Reading::Uniform(pt) => Reading::Uniform(pt.in_unit::<Mm>()),
                        Reading::Mixed => Reading::Mixed,
                    })
                    on_write=Callback::new(move |mm: PrintLength<Mm>| paragraph(field(mm.in_unit::<Pt>())))
                />
            }
        };
        let spacings = panel::LINE_SPACINGS
            .into_iter()
            .map(|(value, label)| Choice {
                value,
                label,
                icon: None,
            })
            .collect::<Vec<_>>();
        let lists = panel::LISTS
            .into_iter()
            .zip([
                icondata::MdiFormatListBulleted,
                icondata::MdiFormatListNumbered,
                icondata::MdiFormatListChecks,
            ])
            .map(|((style, label), icon)| Choice {
                value: Some(ParagraphKind::List(style)),
                label,
                icon: Some(icon),
            })
            .collect::<Vec<_>>();
        let list = Signal::derive(move || match values.with(|v| v.kind) {
            Reading::Uniform(Some(ParagraphKind::List(style))) => {
                Reading::Uniform(Some(ParagraphKind::List(style)))
            }
            Reading::Uniform(_) => Reading::Uniform(None),
            Reading::Mixed => Reading::Mixed,
        });
        view! {
            <Field label="Alignment">
                <Segmented
                    property=prop(panel::ALIGN)
                    choices=vec![
                        Choice { value: Align::Left, label: "Left", icon: Some(icondata::MdiFormatAlignLeft) },
                        Choice { value: Align::Center, label: "Centre", icon: Some(icondata::MdiFormatAlignCenter) },
                        Choice { value: Align::Right, label: "Right", icon: Some(icondata::MdiFormatAlignRight) },
                        Choice { value: Align::Justify, label: "Justified", icon: Some(icondata::MdiFormatAlignJustify) },
                    ]
                    value=Signal::derive(move || values.with(|v| v.align))
                    on_write=Callback::new(move |a| paragraph(ParagraphStyle { align: Some(a), ..ParagraphStyle::default() }))
                />
            </Field>
            <Field label="Line spacing">
                <SelectField
                    property=prop(panel::LINE_SPACING)
                    choices=spacings
                    value=Signal::derive(move || values.with(|v| v.line_height))
                    on_write=Callback::new(move |h: Option<crate::events::LineHeight>| {
                        // The default is the document's: the paragraph's own
                        // spacing goes back to single, the seeded default.
                        let height = h.unwrap_or(crate::events::LineHeight::Multiple(
                            crate::events::LineSpacingMultiple::ONE_AND_HALF,
                        ));
                        paragraph(ParagraphStyle { line_height: Some(height), ..ParagraphStyle::default() });
                    })
                />
            </Field>
            <Field label="Left indent">
                {indent(panel::INDENT_LEFT, |v| v.indent_left, |pt| ParagraphStyle { indent_left: Some(pt), ..ParagraphStyle::default() })}
            </Field>
            <Field label="First line">
                {indent(panel::INDENT_FIRST, |v| v.indent_first, |pt| ParagraphStyle { indent_first: Some(pt), ..ParagraphStyle::default() })}
            </Field>
            <Field label="List">
                <Segmented
                    property=prop(panel::KIND)
                    choices=lists
                    value=list
                    on_write=Callback::new(move |kind: Option<ParagraphKind>| {
                        if let Some(kind) = kind {
                            text_write(TextWrite::Kind(kind));
                        }
                    })
                />
            </Field>
        }
        .into_any()
    };

    let kind_section = move || {
        let kinds = panel::KINDS
            .into_iter()
            .map(|(kind, label)| Choice {
                value: Some(kind),
                label,
                icon: None,
            })
            .collect::<Vec<_>>();
        view! {
            <Segmented
                property=prop(panel::KIND)
                choices=kinds
                value=Signal::derive(move || match values.with(|v| v.kind) {
                    Reading::Uniform(kind) => Reading::Uniform(panel::kind_chip(kind)),
                    Reading::Mixed => Reading::Mixed,
                })
                on_write=Callback::new(move |kind: Option<ParagraphKind>| {
                    if let Some(kind) = kind {
                        text_write(TextWrite::Kind(kind));
                    }
                })
            />
        }
        .into_any()
    };

    let actions_section = move || {
        let ids: &'static [ambre_core::StaticKey<ambre_core::Commands>] = &panel::ACTION_COMMANDS;
        view! { <Row>{command_buttons(ids, Labelled::Text)}</Row> }.into_any()
    };

    let defaults_section = move || {
        let fonts = move || signals.fonts.get().unwrap_or_default();
        view! {
            <Field label="Body">
                <FontField
                    property=prop(panel::BODY_FONT)
                    value=Signal::derive(move || Reading::Uniform(Some(fonts().body)))
                    on_write=Callback::new(move |f: Option<ambre_core::FontFamily>| {
                        if let Some(f) = f {
                            text_write(TextWrite::DefaultFont(f));
                        }
                    })
                />
            </Field>
        }
        .into_any()
    };

    let sections = Signal::derive(move || {
        kind.get()
            .sections()
            .iter()
            .filter_map(|&key| {
                Some(match key {
                    k if k == panel::PAGE => {
                        PanelSection::new(k, "Page", icondata::MdiFileDocumentOutline, page_section)
                    }
                    k if k == panel::TEXT => {
                        PanelSection::new(k, "Text", icondata::MdiFormatText, text_section)
                    }
                    k if k == panel::PARAGRAPH => PanelSection::new(
                        k,
                        "Paragraph",
                        icondata::MdiFormatAlignLeft,
                        paragraph_section,
                    ),
                    k if k == panel::PARAGRAPH_STYLE => PanelSection::new(
                        k,
                        "Paragraph style",
                        icondata::MdiFormatHeaderPound,
                        kind_section,
                    ),
                    k if k == panel::ACTIONS => {
                        PanelSection::new(k, "Actions", icondata::MdiLink, actions_section)
                    }
                    k if k == panel::DEFAULT_STYLES => PanelSection::new(
                        k,
                        "Default styles",
                        icondata::MdiFormatFont,
                        defaults_section,
                    ),
                    _ => return None,
                })
            })
            .collect::<Vec<_>>()
    });
    PanelContent {
        title: Signal::derive(move || kind.get().title().to_owned()),
        sections,
    }
}
