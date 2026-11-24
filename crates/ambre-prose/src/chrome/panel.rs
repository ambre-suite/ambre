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

use ambre_chrome::commands::Reading;
use ambre_chrome::{
    Choice, ColourField, CommandButton, Field, FontField, FontSizeSelect, LengthField,
    PanelContent, PanelSection, Pressed, Segmented, SelectField,
};
use ambre_core::{Mm, PrintLength};
use leptos::prelude::*;

use super::ProseSignals;
use crate::doc::{Orientation, PageFormat};
use crate::events::{ActiveMarks, Align, BlockFormat, IndentStep, ParagraphStyle, TriState};
use crate::view::annotate::review_intent;
use crate::view::panel::{self, CaretSuggestion, Margins, PanelKind, Paper};
use crate::view::toolbar::TextWrite;
use crate::view::toolbar::state_of;

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

/// What the Logic thread read of one value for this very selection. Before
/// its answer, the value reads as mixed rather than as a guess.
fn read<T>(signals: ProseSignals, pick: impl Fn(&ActiveMarks) -> Reading<T>) -> Reading<T> {
    signals
        .toolbar
        .with(|bar| bar.active.as_ref().map_or(Reading::Mixed, pick))
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

    let text_section = move || {
        let buttons = move |ids: &'static [ambre_core::StaticKey<ambre_core::Commands>]| {
            ids
            .iter()
            .filter_map(|&id| {
                let command = ambre_commands::find(crate::commands::COMMANDS, id)?;
                let mark = crate::commands::mark_key(id);
                let pressed = mark.map(|key| {
                    Signal::derive(move || {
                        match signals.toolbar.with(|bar| bar.active.as_ref().map(|m| state_of(m, key))) {
                            Some(TriState::On) => Pressed::On,
                            Some(TriState::Mixed) => Pressed::Mixed,
                            _ => Pressed::Off,
                        }
                    })
                });
                let on_command = Callback::new(move |id| signals.format.set(Some(id)));
                Some(match pressed {
                    Some(pressed) => view! {
                        <CommandButton command=command icon=super::icon_of(command) on_command=on_command pressed=pressed />
                    }
                    .into_any(),
                    None => view! {
                        <CommandButton command=command icon=super::icon_of(command) on_command=on_command />
                    }
                    .into_any(),
                })
            })
            .collect_view()
        };
        // Each group of the text section carries a label (`CHROME-218`).
        let all: &'static [ambre_core::StaticKey<ambre_core::Commands>] = &panel::TEXT_COMMANDS;
        let (marks, rest) = all.split_at(5);
        let (script, clear) = rest.split_at(2);
        // What the Logic thread read for this very selection. Before its
        // answer, each value reads as mixed rather than as a guess.
        let write = move |w: TextWrite| signals.text_write.set(Some(w));
        let size_shown = Signal::derive(move || read(signals, |m| m.font_size).uniform().flatten());
        let styles = panel::PARAGRAPH_STYLES
            .into_iter()
            .map(|style| Choice {
                value: style,
                label: panel::style_label(style),
                icon: None,
            })
            .collect::<Vec<_>>();
        let lists = panel::ListChoice::ALL
            .into_iter()
            .map(|choice| Choice {
                value: choice,
                label: choice.label(),
                icon: None,
            })
            .collect::<Vec<_>>();
        let indent = move |id, icon: icondata::Icon, step: IndentStep| {
            view! {
                <ambre_chrome::IconButton
                    icon=icon
                    label=label_of(id)
                    reach=Some(ambre_chrome::commands::Reach::Property(id))
                    on_press=Callback::new(move |()| write(TextWrite::Block(BlockFormat::Indent(step))))
                />
            }
        };
        let link = Signal::derive(move || {
            signals.toolbar.with(|bar| {
                bar.active
                    .as_ref()
                    .and_then(|m| m.link.as_ref())
                    .map(|l| l.as_str().to_owned())
                    .unwrap_or_default()
            })
        });
        view! {
            <Field label="Style">{buttons(marks)}</Field>
            <Field label="Script">{buttons(script)}</Field>
            <Field label="Font">
                <FontField
                    property=prop(panel::TEXT_FONT)
                    value=Signal::derive(move || read(signals, |m| m.font_family.clone()))
                    on_write=Callback::new(move |f| write(TextWrite::Family(f)))
                />
            </Field>
            <Field label="Size">
                <span class="ambre-kit-control" data-reach=ambre_chrome::commands::Reach::Property(panel::TEXT_SIZE).marker()>
                    <FontSizeSelect
                        current=size_shown
                        on_pick=Callback::new(move |s| write(TextWrite::Size(s)))
                        auto_label="Default"
                    />
                </span>
            </Field>
            <Field label="Colour">
                <ColourField
                    property=prop(panel::TEXT_COLOR)
                    icon=icondata::MdiFormatColorText
                    value=Signal::derive(move || read(signals, |m| m.color))
                    default=Signal::derive(|| ambre_core::Rgba::from_channels(0, 0, 0, 0xFF))
                    reset_label="Automatic"
                    on_write=Callback::new(move |c| write(TextWrite::Color(c)))
                />
                <ColourField
                    property=prop(panel::TEXT_HIGHLIGHT)
                    icon=icondata::MdiFormatColorHighlight
                    value=Signal::derive(move || read(signals, |m| m.background))
                    default=Signal::derive(|| ambre_core::Rgba::from_channels(0xFF, 0xEE, 0x00, 0xFF))
                    reset_label="No highlight"
                    on_write=Callback::new(move |c| write(TextWrite::Highlight(c)))
                />
            </Field>
            <Field label="Paragraph">
                <SelectField
                    property=prop(panel::PARAGRAPH_STYLE)
                    choices=styles
                    value=Signal::derive(move || read(signals, |m| m.heading))
                    on_write=Callback::new(move |h| write(TextWrite::Block(BlockFormat::Heading(h))))
                />
            </Field>
            <Field label="Align">
                <Segmented
                    property=prop(panel::PARAGRAPH_ALIGN)
                    choices=vec![
                        Choice { value: Align::Left, label: "Left", icon: Some(icondata::MdiFormatAlignLeft) },
                        Choice { value: Align::Center, label: "Centre", icon: Some(icondata::MdiFormatAlignCenter) },
                        Choice { value: Align::Right, label: "Right", icon: Some(icondata::MdiFormatAlignRight) },
                        Choice { value: Align::Justify, label: "Justify", icon: Some(icondata::MdiFormatAlignJustify) },
                    ]
                    value=Signal::derive(move || panel::align_shown(read(signals, |m| m.align)))
                    on_write=Callback::new(move |align| {
                        write(TextWrite::Block(BlockFormat::Paragraph(ParagraphStyle {
                            align: Some(align),
                            ..ParagraphStyle::default()
                        })));
                    })
                />
            </Field>
            <Field label="List">
                <Segmented
                    property=prop(panel::PARAGRAPH_LIST)
                    choices=lists
                    value=Signal::derive(move || panel::ListChoice::of(read(signals, |m| m.list)))
                    on_write=Callback::new(move |c: panel::ListChoice| write(TextWrite::Block(c.format())))
                />
            </Field>
            <Field label="Indent">
                {indent(panel::INDENT_OUT, icondata::MdiFormatIndentDecrease, IndentStep::Out)}
                {indent(panel::INDENT_IN, icondata::MdiFormatIndentIncrease, IndentStep::In)}
            </Field>
            <Field label="Link">
                <input
                    type="url"
                    class="ambre-prose-link-field"
                    placeholder="https://"
                    aria-label=label_of(panel::TEXT_LINK)
                    data-reach=ambre_chrome::commands::Reach::Property(panel::TEXT_LINK).marker()
                    prop:value=move || link.get()
                    on:change=move |ev| {
                        let typed = event_target_value(&ev);
                        write(TextWrite::Link(Some(ambre_core::LinkUrl::new(typed))));
                    }
                />
            </Field>
            <Field label="Clear">{buttons(clear)}</Field>
        }
        .into_any()
    };

    // Accept and reject of the suggestion at the caret, so a key or a tap
    // reaches them with no hover (`PROSE-UI-259`, `CHROME-183`).
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

    // Suggestion mode, whatever is selected (`PROSE-UI-262`). The control
    // reads the mode the Logic thread reported, so its pressed state is the
    // mode itself (`PROSE-UI-261`).
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
