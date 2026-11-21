// ambre-toile/src/chrome/style.rs
// ============================================================================
// The board's sections of the context panel (`ambre-spec-shell.md` section 2,
// `toile-styles`), wasm-only. The shell owns the frame; this module writes the
// content of each section from the kit, and nothing else.
//
// The kind of the selection picks the sections (`ui::panel`). Nothing
// selected shows the board. Elements show a shape's style, its text and its
// arrangement. An arrow shows a connector's stroke.
//
// Style edits are DEBOUNCED and emitted as ONE `RestyleElements` over the whole
// selection, the style register being written whole (`TOILE-018`). The panel
// keeps a local working `ElementStyle` seeded from the published common style
// and mutates one field per control. After a quiet interval it writes that
// style to the shared `restyle` signal the canvas view drains. Each control
// writes its one field through `StyleEdit`, so a field nobody touched keeps the
// selection's value (`TOILE-UI-172`).
//
// The order, group and lock buttons are `StaticKey<Commands>`s the view
// resolves against its selection. They reuse the shared `on_command` path, as
// the context menu does.
// ============================================================================

use std::time::Duration;

use crate::message::{ElementStyle, FontFamily, FontSizeChoice};
use ambre_chrome::commands::Reading;
use ambre_chrome::{
    ColourField, CommandButton, Debouncer, Field, FontField, FontSizeSelect, LengthField,
    PanelContent, PanelSection, Pressed, PropertyToggle, Row,
};
use ambre_core::Flag as _;
use ambre_core::{Commands, FontSize, Px, Rgba, StaticKey, StoredBoard};
use leptos::prelude::*;

use crate::ui::commands as registry;
use crate::ui::floatbar::StyleEdit;
use crate::ui::menu;
use crate::ui::menu::CtxScope;
use crate::ui::panel::{self, PanelKind};
use crate::ui::style::{Mix, StyleModel};

/// How long the panel waits after the last edit before emitting the coalesced
/// `RestyleElements` (`toile-styles`, "edits are debounced"). Short enough to feel
/// live, long enough to fold a slider drag into a single command.
const RESTYLE_DEBOUNCE: Duration = Duration::from_millis(120);

/// The property of `id`. Every id this module names is in the table.
fn prop(id: StaticKey<ambre_core::Properties>) -> &'static ambre_chrome::commands::Property {
    panel::property(id)
}

/// What the panel reads off the selection and hands back to the view.
#[derive(Clone, Copy)]
pub struct ToilePanelSignals {
    /// The common style of the selection, `None` with nothing selected.
    pub model: RwSignal<Option<StyleModel>>,
    /// The scope of the selection, for the lock and the group buttons.
    pub scope: RwSignal<CtxScope>,
    /// The shared command sink.
    pub on_command: Callback<StaticKey<Commands>>,
    /// The whole-style edit the view turns into `RestyleElements`.
    pub restyle: RwSignal<Option<ElementStyle>>,
}

/// The value of one field, mixed when the selection disagrees on it.
fn reading<T>(value: T, mix: Mix) -> Reading<T> {
    if mix.is_mixed() {
        Reading::Mixed
    } else {
        Reading::Uniform(value)
    }
}

/// The state of a toggle of one field.
fn pressed(on: bool, mix: Mix) -> Pressed {
    if mix.is_mixed() {
        Pressed::Mixed
    } else {
        Pressed::from_bool(on)
    }
}

/// The title and the sections of the board's context panel.
pub fn toile_panel(signals: ToilePanelSignals) -> PanelContent {
    let ToilePanelSignals {
        model,
        scope,
        on_command,
        restyle,
    } = signals;
    let kind = Memo::new(move |_| model.with(|m| PanelKind::of(m.as_ref())));

    // The working copy of the whole style register, re-seeded whenever the view
    // publishes a new common style.
    let working = RwSignal::new(ElementStyle::default());
    Effect::new(move |_| {
        if let Some(m) = model.get() {
            working.set(m.base);
        }
    });
    let debouncer = Debouncer::new();
    let commit = move |edit: StyleEdit| {
        working.update(|s| edit.apply_to(s));
        let style = working.get_untracked();
        debouncer.call(RESTYLE_DEBOUNCE, move || restyle.set(Some(style)));
    };

    let base = move || model.get().map(|m| (m.base, m.mixed));
    let colour = move |field: fn(&ElementStyle) -> Option<Rgba>,
                       mix: fn(&crate::ui::style::MixedFlags) -> Mix| {
        Signal::derive(move || {
            base().map_or(Reading::Uniform(None), |(b, m)| reading(field(&b), mix(&m)))
        })
    };
    let command = move |id: StaticKey<Commands>| {
        ambre_chrome::commands::find(registry::COMMANDS, id).map(|c| {
            let icon = c.icon.and_then(super::icons::resolve);
            view! { <CommandButton command=c icon=icon on_command=on_command /> }
        })
    };

    let board = move || {
        view! {
            <Row>
                {command(menu::SELECT_ALL)}
                {command(crate::ui::find::FIND)}
                {command(crate::ui::menu_bar::COMMENTS)}
                {command(crate::ui::menu_bar::DECK)}
            </Row>
        }
        .into_any()
    };

    let shape = move || {
        view! {
            <Field label="Fill">
                <ColourField
                    property=prop(panel::FILL)
                    icon=icondata::MdiFormatColorFill
                    value=colour(|s| s.fill, |m| m.fill)
                    default=Signal::derive(|| Rgba::from_bits(0xffff_ffff))
                    reset_label="No fill"
                    on_write=Callback::new(move |c| commit(StyleEdit::Fill(c)))
                />
            </Field>
            {stroke_fields(base, commit)}
        }
        .into_any()
    };

    let text = move || {
        let font = Signal::derive(move || {
            base().map_or(Reading::Uniform(None), |(b, m)| {
                // A family the picker does not offer shows the "Default" entry.
                reading(
                    b.font.family.filter(|f| f.picker_label().is_some()),
                    m.font_family,
                )
            })
        });
        let size = Signal::derive(move || base().and_then(|(b, _)| b.font.size.as_fixed()));
        let toggle = move |prop_id,
                           icon,
                           on: fn(&ElementStyle) -> bool,
                           mix: fn(&crate::ui::style::MixedFlags) -> Mix,
                           edit: fn(bool) -> StyleEdit| {
            let state = Signal::derive(move || {
                base().map_or(Pressed::Off, |(b, m)| pressed(on(&b), mix(&m)))
            });
            view! {
                <PropertyToggle
                    property=prop(prop_id)
                    icon=icon
                    pressed=state
                    on_toggle=Callback::new(move |()| {
                        let next = state.get_untracked() != Pressed::On;
                        commit(edit(next));
                    })
                />
            }
        };
        view! {
            <Field label="Colour">
                <ColourField
                    property=prop(panel::TEXT_COLOUR)
                    icon=icondata::MdiFormatColorText
                    value=colour(|s| s.text_color, |m| m.text_color)
                    default=Signal::derive(|| Rgba::from_bits(0x1a1a_1aff))
                    reset_label="Default"
                    on_write=Callback::new(move |c| commit(StyleEdit::TextColor(c)))
                />
            </Field>
            <Field label="Font">
                <FontField
                    property=prop(panel::FONT)
                    value=font
                    on_write=Callback::new(move |f: Option<FontFamily>| commit(StyleEdit::Font(f)))
                />
            </Field>
            <Field label="Size">
                <span data-reach=ambre_chrome::commands::Reach::Property(panel::SIZE).marker()>
                    <FontSizeSelect
                        current=size
                        on_pick=Callback::new(move |p: Option<FontSize>| {
                            commit(StyleEdit::Size(FontSizeChoice::picked(p)))
                        })
                        auto_label="Auto"
                    />
                </span>
            </Field>
            <Row>
                {toggle(panel::BOLD, icondata::BsTypeBold, |s| s.font.weight.is_bold(), |m| m.bold,
                    |on| StyleEdit::Bold(ambre_core::Weight::from_bool(on)))}
                {toggle(panel::ITALIC, icondata::BsTypeItalic, |s| s.font.italic.as_bool(), |m| m.italic,
                    |on| StyleEdit::Italic(ambre_core::Slant::from_bool(on)))}
                {toggle(panel::UNDERLINE, icondata::BsTypeUnderline, |s| s.font.underline.as_bool(), |m| m.underline,
                    |on| StyleEdit::Underline(ambre_core::Underline::from_bool(on)))}
                {toggle(panel::STRIKETHROUGH, icondata::BsTypeStrikethrough, |s| s.font.strikethrough.as_bool(), |m| m.strikethrough,
                    |on| StyleEdit::Strikethrough(ambre_core::Strikethrough::from_bool(on)))}
            </Row>
        }
        .into_any()
    };

    let arrange = move || {
        let any_locked = move || scope.with(|s| s.locks.any_locked());
        let any_unlocked = move || scope.with(|s| s.locks.any_unlocked());
        let multi = move || scope.with(CtxScope::multi);
        let grouped = move || scope.with(|s| s.groups == menu::GroupHeld::Group);
        view! {
            <Row>
                {command(menu::BRING_FRONT)}
                {command(menu::FORWARD)}
                {command(menu::BACKWARD)}
                {command(menu::SEND_BACK)}
            </Row>
            <Row>
                <Show when=multi>{command(menu::GROUP)}</Show>
                <Show when=grouped>{command(menu::UNGROUP)}</Show>
                <Show when=any_unlocked>{command(menu::LOCK)}</Show>
                <Show when=any_locked>{command(menu::UNLOCK)}</Show>
                {command(menu::DUPLICATE)}
                {command(menu::DELETE)}
            </Row>
            <Show when=multi>
                <Row>
                    {command(menu::ALIGN_LEFT)}
                    {command(menu::ALIGN_HCENTER)}
                    {command(menu::ALIGN_RIGHT)}
                    {command(menu::ALIGN_TOP)}
                    {command(menu::ALIGN_VCENTER)}
                    {command(menu::ALIGN_BOTTOM)}
                    {command(menu::DISTRIBUTE_H)}
                    {command(menu::DISTRIBUTE_V)}
                </Row>
            </Show>
            <Row>
                {command(crate::ui::menu_bar::LINK_SET)}
                {command(crate::ui::menu_bar::LINK_CLEAR)}
            </Row>
        }
        .into_any()
    };

    let connector = move || {
        view! {
            {stroke_fields(base, commit)}
            <Row>{command(menu::DELETE)}</Row>
        }
        .into_any()
    };

    let sections = Signal::derive(move || {
        kind.get()
            .sections()
            .iter()
            .filter_map(|&key| {
                Some(match key {
                    k if k == panel::BOARD => {
                        PanelSection::new(k, "Board", icondata::MdiViewDashboardOutline, board)
                    }
                    k if k == panel::SHAPE => {
                        PanelSection::new(k, "Shape", icondata::MdiShapeOutline, shape)
                    }
                    k if k == panel::TEXT => {
                        PanelSection::new(k, "Text", icondata::MdiFormatText, text)
                    }
                    k if k == panel::ARRANGE => {
                        PanelSection::new(k, "Arrange", icondata::MdiArrangeBringToFront, arrange)
                    }
                    k if k == panel::CONNECTOR => {
                        PanelSection::new(k, "Connector", icondata::MdiVectorLine, connector)
                    }
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

/// The stroke colour, the stroke width and the opacity, which a shape and a
/// connector share.
fn stroke_fields(
    base: impl Fn() -> Option<(ElementStyle, crate::ui::style::MixedFlags)>
    + Copy
    + Send
    + Sync
    + 'static,
    commit: impl Fn(StyleEdit) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let stroke = Signal::derive(move || {
        base().map_or(Reading::Uniform(None), |(b, m)| reading(b.stroke, m.stroke))
    });
    let width = Signal::derive(move || {
        base().map_or(Reading::Uniform(Px::<StoredBoard>::new(1.0)), |(b, m)| {
            reading(b.stroke_w, m.stroke_w)
        })
    });
    let opacity = Signal::derive(move || {
        base().map_or(
            Reading::Uniform(ambre_core::Bounded::<ambre_core::Opacity>::OPAQUE),
            |(b, m)| reading(b.opacity, m.opacity),
        )
    });
    view! {
        <Field label="Stroke">
            <ColourField
                property=prop(panel::STROKE)
                icon=icondata::MdiBorderColor
                value=stroke
                default=Signal::derive(|| Rgba::from_bits(0x3333_33ff))
                reset_label="No stroke"
                on_write=Callback::new(move |c| commit(StyleEdit::Stroke(c)))
            />
        </Field>
        <Field label="Width">
            <LengthField
                property=prop(panel::STROKE_WIDTH)
                value=width
                on_write=Callback::new(move |w| commit(StyleEdit::StrokeWidth(w)))
            />
        </Field>
        <Field label="Opacity">
            <span class="ambre-kit-control" data-reach=ambre_chrome::commands::Reach::Property(panel::OPACITY).marker()>
                <input
                    type="range"
                    min="0"
                    max="100"
                    aria-label="Opacity"
                    prop:value=move || opacity.get().uniform().map(|o| format!("{:.0}", o.get() * 100.0)).unwrap_or_default()
                    on:mousedown=move |ev| ev.prevent_default()
                    on:input=move |ev| {
                        if let Ok(v) = event_target_value(&ev).parse::<f32>() {
                            commit(StyleEdit::Opacity(ambre_core::Bounded::<ambre_core::Opacity>::clamped(v / 100.0)));
                        }
                    }
                />
            </span>
        </Field>
    }
}
