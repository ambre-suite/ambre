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
use ambre_core::{Mm, PrintLength, Pt, Rgba};
use leptos::prelude::*;

use super::ProseSignals;
use crate::doc::{Orientation, PageFormat};
use crate::events::{Align, ParagraphStyle, TriState};
use crate::message::{IndentStep, ParagraphKind, SelectionValues};
use crate::view::annotate::review_intent;
use crate::view::panel::{self, CaretSuggestion, Margins, PanelKind, Paper, TextValue};
use crate::view::toolbar::state_of;

/// The value of every list and segmented control of the panel. One value
/// type gives one instance of each generic control, which keeps Main
/// small (`ambre-spec-budgets.md` section 5).
#[derive(Clone, Copy, PartialEq)]
enum Pick {
    Paper(Paper),
    Orientation(Orientation),
    Margins(Margins),
    Align(Align),
    Spacing(Option<crate::events::LineHeight>),
    Kind(Option<ParagraphKind>),
}

/// A reading of one property, as the value the panel's controls take.
fn picked<T>(reading: Reading<T>, wrap: fn(T) -> Pick) -> Reading<Pick> {
    match reading {
        Reading::Uniform(value) => Reading::Uniform(wrap(value)),
        Reading::Mixed => Reading::Mixed,
    }
}

/// The property of `id`. Every id this module names is in the table.
fn prop(
    id: ambre_core::StaticKey<ambre_core::Properties>,
) -> &'static ambre_chrome::commands::Property {
    panel::property(id)
}

/// The label of the property of `id`, as a control names it.
fn label_of(id: ambre_core::StaticKey<ambre_core::Properties>) -> String {
    (prop(id).title)(&ambre_chrome::commands::Locale::En).into_string()
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
        let paper = Signal::derive(move || Reading::Uniform(Pick::Paper(Paper::of(&page()))));
        let orientation = Signal::derive(move || {
            Reading::Uniform(Pick::Orientation(panel::orientation_of(&page())))
        });
        // A pick of custom shows the four steppers before any margin moves;
        // margins no preset names show them too.
        let custom = RwSignal::new(false);
        let margins = Signal::derive(move || {
            if custom.get() {
                Reading::Uniform(Pick::Margins(Margins::Custom))
            } else {
                Reading::Uniform(Pick::Margins(Margins::of(&page())))
            }
        });
        let margin_choices = Margins::ALL
            .into_iter()
            .map(|m| Choice {
                value: Pick::Margins(m),
                label: m.label(),
                icon: None,
            })
            .collect::<Vec<_>>();
        let show_steppers =
            move || margins.get() == Reading::Uniform(Pick::Margins(Margins::Custom));
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
                value: Pick::Paper(p),
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
                        if let Pick::Paper(p) = p
                            && let Some(format) = panel::with_paper(page(), p)
                        {
                            write(format);
                        }
                    })
                />
            </Field>
            <Field label="Orientation">
                <Segmented
                    property=prop(panel::ORIENTATION)
                    choices=vec![
                        Choice { value: Pick::Orientation(Orientation::Portrait), label: "Portrait", icon: Some(icondata::LuRectangleVertical) },
                        Choice { value: Pick::Orientation(Orientation::Landscape), label: "Landscape", icon: Some(icondata::LuRectangleHorizontal) },
                    ]
                    value=orientation
                    on_write=Callback::new(move |o| {
                        if let Pick::Orientation(o) = o {
                            write(panel::with_orientation(page(), o));
                        }
                    })
                />
            </Field>
            <Field label="Margins">
                <SelectField
                    property=prop(panel::MARGINS)
                    choices=margin_choices
                    value=margins
                    on_write=Callback::new(move |m| {
                        let Pick::Margins(m) = m else { return };
                        match panel::with_margins(page(), m) {
                            Some(format) => {
                                custom.set(false);
                                write(format);
                            }
                            None => custom.set(true),
                        }
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
    // its answer arrives (`PROSE-UI-258`).
    let values = Memo::new(move |_| {
        signals.toolbar.with(|bar| {
            bar.active
                .as_ref()
                .map(|m| m.values.clone())
                .unwrap_or_default()
        })
    });
    let text_write = move |value: TextValue| signals.pick_text_write(value);
    let paragraph = move |style: ParagraphStyle| text_write(TextValue::Paragraph(style));
    let write_kind = move |pick: Pick| {
        if let Pick::Kind(Some(kind)) = pick {
            text_write(TextValue::Kind(kind));
        }
    };
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
        view! {
            <Field label="Font">
                <FontField
                    property=prop(panel::FONT)
                    value=Signal::derive(move || values.with(|v| v.font_family.clone()))
                    on_write=Callback::new(move |f| text_write(TextValue::Font(f)))
                />
            </Field>
            <Field label="Size">
                <span data-reach=Reach::Property(panel::SIZE).marker()>
                    <FontSizeSelect
                        current=size
                        on_pick=Callback::new(move |s| text_write(TextValue::Size(s)))
                        auto_label="Default"
                    />
                </span>
            </Field>
            <Field label="Style">{command_buttons(marks, Labelled::Icon)}</Field>
            <Field label="Colour">
                <ColourField
                    property=prop(panel::COLOUR)
                    icon=icondata::MdiFormatColorText
                    value=Signal::derive(move || values.with(|v| v.color))
                    default=Signal::stored(Rgba::from_bits(0x2924_1eff))
                    reset_label="Automatic"
                    on_write=Callback::new(move |c| text_write(TextValue::Colour(c)))
                />
            </Field>
            <Field label="Highlight">
                <ColourField
                    property=prop(panel::HIGHLIGHT)
                    icon=icondata::MdiFormatColorHighlight
                    value=Signal::derive(move || values.with(|v| v.highlight))
                    default=Signal::stored(Rgba::from_bits(0xffff_00ff))
                    reset_label="No highlight"
                    on_write=Callback::new(move |c| text_write(TextValue::Highlight(c)))
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
                value: Pick::Spacing(value),
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
                value: Pick::Kind(Some(ParagraphKind::List(style))),
                label,
                icon: Some(icon),
            })
            .collect::<Vec<_>>();
        // One step in or out: a list item changes level, another block moves
        // its left indent (`PROSE-UI-267`).
        let step = move |id, icon: icondata::Icon, step: IndentStep| {
            view! {
                <ambre_chrome::IconButton
                    icon=icon
                    label=label_of(id)
                    reach=Some(Reach::Property(id))
                    on_press=Callback::new(move |()| text_write(TextValue::Indent(step)))
                />
            }
        };
        let list = Signal::derive(move || match values.with(|v| v.kind) {
            Reading::Uniform(Some(ParagraphKind::List(style))) => {
                Reading::Uniform(Pick::Kind(Some(ParagraphKind::List(style))))
            }
            Reading::Uniform(_) => Reading::Uniform(Pick::Kind(None)),
            Reading::Mixed => Reading::Mixed,
        });
        view! {
            <Field label="Alignment">
                <Segmented
                    property=prop(panel::ALIGN)
                    choices=vec![
                        Choice { value: Pick::Align(Align::Left), label: "Left", icon: Some(icondata::MdiFormatAlignLeft) },
                        Choice { value: Pick::Align(Align::Center), label: "Centre", icon: Some(icondata::MdiFormatAlignCenter) },
                        Choice { value: Pick::Align(Align::Right), label: "Right", icon: Some(icondata::MdiFormatAlignRight) },
                        Choice { value: Pick::Align(Align::Justify), label: "Justified", icon: Some(icondata::MdiFormatAlignJustify) },
                    ]
                    value=Signal::derive(move || picked(values.with(|v| v.align), Pick::Align))
                    on_write=Callback::new(move |a| {
                        if let Pick::Align(a) = a {
                            paragraph(ParagraphStyle { align: Some(a), ..ParagraphStyle::default() });
                        }
                    })
                />
            </Field>
            <Field label="Line spacing">
                <SelectField
                    property=prop(panel::LINE_SPACING)
                    choices=spacings
                    value=Signal::derive(move || picked(values.with(|v| v.line_height), Pick::Spacing))
                    on_write=Callback::new(move |h: Pick| {
                        let Pick::Spacing(h) = h else { return };
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
            <Field label="Indent">
                {step(panel::INDENT_OUT, icondata::MdiFormatIndentDecrease, IndentStep::Out)}
                {step(panel::INDENT_IN, icondata::MdiFormatIndentIncrease, IndentStep::In)}
            </Field>
            <Field label="List">
                <Segmented
                    property=prop(panel::KIND)
                    choices=lists
                    value=list
                    on_write=Callback::new(write_kind)
                />
            </Field>
        }
        .into_any()
    };

    let kind_section = move || {
        let kinds = panel::KINDS
            .into_iter()
            .map(|(kind, label)| Choice {
                value: Pick::Kind(Some(kind)),
                label,
                // A heading shows its short glyph, so the five chips fit the
                // panel; its full name is the tooltip and the accessible name.
                icon: match kind {
                    ParagraphKind::Heading(crate::message::HeadingLevel::H1) => {
                        Some(icondata::MdiFormatHeader1)
                    }
                    ParagraphKind::Heading(_) => Some(icondata::MdiFormatHeader2),
                    _ => None,
                },
            })
            .collect::<Vec<_>>();
        view! {
            <Segmented
                property=prop(panel::KIND)
                choices=kinds
                value=Signal::derive(move || match values.with(|v| v.kind) {
                    Reading::Uniform(kind) => Reading::Uniform(Pick::Kind(panel::kind_chip(kind))),
                    Reading::Mixed => Reading::Mixed,
                })
                on_write=Callback::new(write_kind)
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
                            text_write(TextValue::DefaultFont(f));
                        }
                    })
                />
            </Field>
        }
        .into_any()
    };

    // Accept and reject of the suggestion at the caret, so a key or a tap
    // reaches them with no hover (`PROSE-UI-273`, `CHROME-183`).
    let suggestion_section = move || {
        let on_command = Callback::new(move |id| {
            if let Some(intent) = review_intent(id, signals.at_caret.get_untracked()) {
                signals.intent.set(Some(intent));
            }
        });
        let buttons = panel::SUGGESTION_COMMANDS
            .iter()
            .filter_map(|&id| {
                let command = ambre_commands::find(crate::commands::COMMANDS, id)?;
                Some(view! { <CommandButton command=command icon={ Option::<icondata::Icon>::None } on_command=on_command /> })
            })
            .collect_view();
        view! { <Field label="Review">{buttons}</Field> }.into_any()
    };

    // Suggestion mode, whatever is selected (`PROSE-UI-276`). The control
    // reads the mode the Logic thread reported, so its pressed state is the
    // mode itself (`PROSE-UI-275`).
    let review_section = move || {
        let on_command = Callback::new(move |id| {
            if let Some(intent) = review_intent(id, signals.at_caret.get_untracked()) {
                signals.intent.set(Some(intent));
            }
        });
        let pressed = Signal::derive(move || {
            if signals.mode.get() == crate::events::EditMode::Suggesting {
                Pressed::On
            } else {
                Pressed::Off
            }
        });
        let buttons = panel::REVIEW_COMMANDS
            .iter()
            .filter_map(|&id| {
                let command = ambre_commands::find(crate::commands::COMMANDS, id)?;
                Some(view! {
                    <CommandButton
                        command=command
                        icon=super::icon_of(command)
                        on_command=on_command
                        pressed=pressed
                        labelled=ambre_chrome::Labelled::Text
                    />
                })
            })
            .collect_view();
        view! { <Field label="Mode">{buttons}</Field> }.into_any()
    };

    let at_caret = Memo::new(move |_| {
        if signals.at_caret.with(Option::is_some) {
            CaretSuggestion::Held
        } else {
            CaretSuggestion::Outside
        }
    });
    let sections = Signal::derive(move || {
        panel::sections(kind.get(), at_caret.get())
            .into_iter()
            .filter_map(|key| {
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
                    k if k == panel::SUGGESTION => PanelSection::new(
                        k,
                        "Suggestion",
                        icondata::MdiFileDocumentEditOutline,
                        suggestion_section,
                    ),
                    k if k == panel::REVIEW => PanelSection::new(
                        k,
                        "Review",
                        icondata::MdiFileDocumentEditOutline,
                        review_section,
                    ),
                    _ => return None,
                })
            })
            .collect::<Vec<_>>()
    });
    PanelContent {
        title: Signal::derive(move || kind.get().title().to_owned()),
        subtitle: Signal::derive(move || kind.get().subtitle(signals.selection_words.get())),
        sections,
    }
}
