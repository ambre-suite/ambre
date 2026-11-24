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
    Choice, CommandButton, Field, LengthField, PanelContent, PanelSection, Pressed, Segmented,
    SelectField,
};
use ambre_core::{Mm, PrintLength};
use leptos::prelude::*;

use super::ProseSignals;
use crate::doc::{Orientation, PageFormat};
use crate::events::TriState;
use crate::view::annotate::review_intent;
use crate::view::panel::{self, CaretSuggestion, Margins, PanelKind, Paper};
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
        view! {
            <Field label="Style">{buttons(marks)}</Field>
            <Field label="Script">{buttons(script)}</Field>
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
        sections,
    }
}
