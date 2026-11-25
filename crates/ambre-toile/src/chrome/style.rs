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
    ChoiceRow, ChoiceView, ColourField, ColourLayout, CommandButton, Debouncer, Field, FontField,
    FontSizeSelect, IconButton, Labelled, LengthField, PanelContent, PanelSection, Pressed,
    PropertyToggle, Row, Select, Slider,
};
use ambre_core::Flag as _;
use ambre_core::{
    Board, Bounded, Commands, FontSize, Px, Radians, Rgba, StaticKey, StoredBoard, UnitFraction,
};
use leptos::prelude::*;

use crate::ui::commands as registry;
use crate::ui::deck::{DeckIntent, DeckSignals};
use crate::ui::floatbar::StyleEdit;
use crate::ui::menu;
use crate::ui::menu::CtxScope;
use crate::ui::panel::{self, PanelKind, PlaceEdit, PlaceField};
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
    /// The write of the position section the view turns into its one
    /// command (`CHROME-248`).
    pub place: RwSignal<Option<PlaceEdit>>,
    /// The snap and the ground of the board, which the board section shows,
    /// and the write the view turns into its command.
    pub board: RwSignal<crate::ui::grid::BoardLook>,
    /// The board's frames, which the frames section lists, and the intent
    /// that selects one (`CHROME-251`).
    pub deck: DeckSignals,
    /// The count of the selection, which the subtitle reads.
    pub stats: RwSignal<Option<crate::ui::status::SelStats>>,
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
        place,
        board: look,
        deck,
        stats,
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
    // A command with its label takes a whole line (`CHROME-218`).
    let labelled = move |id: StaticKey<Commands>| {
        ambre_chrome::commands::find(registry::COMMANDS, id).map(|c| {
            let icon = c.icon.and_then(super::icons::resolve);
            view! { <CommandButton command=c icon=icon on_command=on_command labelled=Labelled::Text /> }
        })
    };

    // A command the registry gives no glyph, drawn compact with the glyph
    // the panel names (`CHROME-250`).
    let icon_command = move |id: StaticKey<Commands>, icon: icondata::Icon| {
        ambre_chrome::commands::find(registry::COMMANDS, id).map(|c| {
            view! {
                <IconButton
                    icon=icon
                    label=c.title.pick(&ambre_chrome::language()).into_string()
                    chord=c.chord()
                    reach=Some(ambre_chrome::commands::Reach::Command(id))
                    on_press=Callback::new(move |()| on_command.run(id))
                />
            }
        })
    };

    // The place, the size and the rotation of a lone shape. Each write is
    // one command, so one undo step (`CHROME-247`, `CHROME-248`).
    let position = move || {
        let placed = move || model.get().and_then(|m| m.placement);
        let field = move |get: fn(&panel::Placement) -> Px<Board>| {
            Signal::derive(move || placed().map_or(Reading::Mixed, |p| Reading::Uniform(get(&p))))
        };
        let write = move |make: fn(Px<Board>) -> PlaceField| {
            Callback::new(move |v: Px<Board>| {
                if let Some(placement) = placed() {
                    place.set(Some(PlaceEdit {
                        placement,
                        field: make(v),
                    }));
                }
            })
        };
        let rotation = Signal::derive(move || {
            placed().map_or(Reading::Mixed, |p| Reading::Uniform(p.rotation))
        });
        // One row builder serves the four places and sizes, so the section
        // adds one view type, not four (`CHROME-247`).
        let row = move |name: &'static str,
                        id,
                        get: fn(&panel::Placement) -> Px<Board>,
                        make: fn(Px<Board>) -> PlaceField| {
            view! {
                <Field label=name>
                    <LengthField property=prop(id) value=field(get) on_write=write(make) />
                </Field>
            }
            .into_any()
        };
        let rows: Vec<AnyView> = vec![
            row("X", panel::X, |p| Px::<Board>::new(p.frame.x0), PlaceField::X),
            row("Y", panel::Y, |p| Px::<Board>::new(p.frame.y0), PlaceField::Y),
            row("W", panel::WIDTH, |p| Px::<Board>::new(p.frame.width()), PlaceField::Width),
            row("H", panel::HEIGHT, |p| Px::<Board>::new(p.frame.height()), PlaceField::Height),
            view! {
                <Field label="Rotation">
                    <LengthField
                        property=prop(panel::ROTATION)
                        value=rotation
                        on_write=Callback::new(move |r: Radians| {
                            if let Some(placement) = placed() {
                                place.set(Some(PlaceEdit { placement, field: PlaceField::Rotation(r) }));
                            }
                        })
                    />
                </Field>
            }
            .into_any(),
        ];
        rows.into_any()
    };

    // The board's frames, each with the count of its items. A press selects
    // the frame (`CHROME-251`).
    let frames = move || {
        let list = move || {
            let view = deck.view.get();
            let rows: Vec<_> = view.slides.into_iter().chain(view.off_deck).collect();
            if rows.is_empty() {
                return view! { <p class="ambre-kit-empty">"No frames"</p> }.into_any();
            }
            rows.into_iter()
                .map(|f| {
                    let id = f.id.clone();
                    let name = if f.name.is_empty() {
                        "Untitled frame".to_owned()
                    } else {
                        f.name
                    };
                    let count = f.items.get();
                    let items = if count == 1 {
                        "1 item".to_owned()
                    } else {
                        format!("{count} items")
                    };
                    view! {
                        <button
                            class="ambre-kit-command ambre-kit-frame"
                            on:mousedown=|ev| ev.prevent_default()
                            on:click=move |_| deck.intent.set(Some(DeckIntent::Select(id.clone())))
                        >
                            <leptos_icons::Icon icon=icondata::MdiImageFrame />
                            <span>{name}</span>
                            <span class="ambre-kit-count">{items}</span>
                        </button>
                    }
                })
                .collect_view()
                .into_any()
        };
        view! { <div class="ambre-kit-row">{list}</div> }.into_any()
    };

    // Find has no control here: the palette and its chord reach it.
    // The snap of the board is document state, so it lives here and never
    // in the status bar (`CHROME-163`, `TOILE-UI-429`).
    let board = move || {
        let snapped = move || look.with(|l| l.snap == crate::message::GridSnap::Snapped);
        view! {
            <Row>
                {labelled(menu::SELECT_ALL)}
                {labelled(crate::ui::menu_bar::COMMENTS)}
                {labelled(crate::ui::menu_bar::DECK)}
            </Row>
            <Field label="Grid">
                <PropertyToggle
                    property=prop(panel::SNAP)
                    icon=icondata::MdiMagnet
                    pressed=Signal::derive(move || Pressed::from_bool(snapped()))
                    on_toggle=Callback::new(move |()| {
                        let snap = if snapped() {
                            crate::message::GridSnap::Free
                        } else {
                            crate::message::GridSnap::Snapped
                        };
                        ask(look, crate::message::BoardEdit::Snap(snap));
                    })
                />
            </Field>
            <Field label="Background">{grounds(look)}</Field>
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
                    layout=ColourLayout::Swatches
                />
            </Field>
            {stroke_fields(base, commit)}
            // The link of an element is a property of the element, never of
            // its arrangement. One compact row (`CHROME-250`).
            <Field label="Link">
                {icon_command(crate::ui::menu_bar::LINK_SET, icondata::MdiLinkVariant)}
                {icon_command(crate::ui::menu_bar::LINK_CLEAR, icondata::MdiLinkVariantOff)}
            </Field>
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
            <Field label="Style">
                {toggle(panel::BOLD, icondata::BsTypeBold, |s| s.font.weight.is_bold(), |m| m.bold,
                    |on| StyleEdit::Bold(ambre_core::Weight::from_bool(on)))}
                {toggle(panel::ITALIC, icondata::BsTypeItalic, |s| s.font.italic.as_bool(), |m| m.italic,
                    |on| StyleEdit::Italic(ambre_core::Slant::from_bool(on)))}
                {toggle(panel::UNDERLINE, icondata::BsTypeUnderline, |s| s.font.underline.as_bool(), |m| m.underline,
                    |on| StyleEdit::Underline(ambre_core::Underline::from_bool(on)))}
                {toggle(panel::STRIKETHROUGH, icondata::BsTypeStrikethrough, |s| s.font.strikethrough.as_bool(), |m| m.strikethrough,
                    |on| StyleEdit::Strikethrough(ambre_core::Strikethrough::from_bool(on)))}
            </Field>
        }
        .into_any()
    };

    let arrange = move || {
        let multi = move || scope.with(CtxScope::multi);
        // A row that shows while the scope meets `when`. The predicate is a
        // function pointer, so every such row shares one view type.
        let shown = move |when: fn(&CtxScope) -> bool, id: StaticKey<Commands>| {
            (move || scope.with(when).then(|| labelled(id))).into_any()
        };
        let framing = move || {
            scope.with(|s| {
                s.containing_frame.is_some()
                    || s.framed == menu::FrameHeld::Framed
                    || s.frame.is_some()
            })
        };
        view! {
            // Labelled rows that name their chords (`CHROME-250`).
            <Row>
                {labelled(menu::DUPLICATE)}
                {shown(CtxScope::multi, menu::GROUP)}
                {shown(|s| s.groups == menu::GroupHeld::Group, menu::UNGROUP)}
                {shown(|s| s.locks.any_unlocked(), menu::LOCK)}
                {shown(|s| s.locks.any_locked(), menu::UNLOCK)}
                {labelled(menu::BRING_FRONT)}
                {labelled(menu::FORWARD)}
                {labelled(menu::BACKWARD)}
                {labelled(menu::SEND_BACK)}
                {labelled(menu::DELETE)}
            </Row>
            <Show when=multi>
                <Field label="Align">
                    {command(menu::ALIGN_LEFT)}
                    {command(menu::ALIGN_HCENTER)}
                    {command(menu::ALIGN_RIGHT)}
                    {command(menu::ALIGN_TOP)}
                    {command(menu::ALIGN_VCENTER)}
                    {command(menu::ALIGN_BOTTOM)}
                </Field>
                <Field label="Distribute">
                    {command(menu::DISTRIBUTE_H)}
                    {command(menu::DISTRIBUTE_V)}
                </Field>
            </Show>
            <Show when=framing>
                <Field label="Frame">
                    <Show when=move || scope.with(|s| s.containing_frame.is_some())>
                        {command(menu::ADD_TO_FRAME)}
                    </Show>
                    <Show when=move || scope.with(|s| s.framed == menu::FrameHeld::Framed)>
                        {command(menu::REMOVE_FROM_FRAME)}
                    </Show>
                    <Show when=move || scope.with(|s| s.frame.is_some())>
                        {command(menu::EXPORT_FRAME_PNG)}
                    </Show>
                </Field>
            </Show>
            <Field label="React">
                {command(menu::REACT_THUMBSUP)}
                {command(menu::REACT_HEART)}
                {command(menu::REACT_CHECK)}
                {command(menu::REACT_PARTY)}
                {command(menu::REACT_CLEAR)}
            </Field>
        }
        .into_any()
    };

    let connector = move || {
        view! {
            {stroke_fields(base, commit)}
            <Row>{labelled(menu::DELETE)}</Row>
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
                    k if k == panel::FRAMES => {
                        PanelSection::new(k, "Frames", icondata::MdiImageFrame, frames)
                    }
                    k if k == panel::POSITION => {
                        PanelSection::new(k, "Position", icondata::MdiArrowAll, position)
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
        subtitle: Signal::derive(move || kind.get().subtitle(stats.with(|s| s.map(|s| s.count)))),
        sections,
    }
}

/// Ask the view for one write of the board section.
fn ask(look: RwSignal<crate::ui::grid::BoardLook>, edit: crate::message::BoardEdit) {
    look.update(|l| l.asked = Some(edit));
}

/// The grounds a board lies on: the ground of the theme, then each light
/// tint, in the shared list control. The list shows the ground the board
/// lies on, and a pick writes another (`TOILE-UI-430`). Main already holds the
/// list, so the section adds no view of its own.
fn grounds(look: RwSignal<crate::ui::grid::BoardLook>) -> AnyView {
    use crate::message::BoardGround;
    use ambre_core::index::Pos;
    let ground_at = |p: Pos<ChoiceRow>| {
        p.to_usize()
            .checked_sub(1)
            .and_then(|i| crate::ui::grid::TINTS.get(i))
            .map_or(BoardGround::Theme, |&(_, c)| BoardGround::Tint(c))
    };
    let choices = std::iter::once("Default")
        .chain(crate::ui::grid::TINTS.iter().map(|&(name, _)| name))
        .map(ChoiceView::text)
        .collect();
    view! {
        <Select
            label=prop(panel::BACKGROUND).title.pick(&ambre_chrome::language()).into_string()
            choices=choices
            selected=Signal::derive(move || {
                let ground = look.with(|l| l.ground);
                (0..=crate::ui::grid::TINTS.len())
                    .map(Pos::<ChoiceRow>::from_usize)
                    .find(|&p| ground_at(p) == ground)
            })
            on_pick=Callback::new(move |p| ask(look, crate::message::BoardEdit::Ground(ground_at(p))))
            reach=ambre_chrome::commands::Reach::Property(panel::BACKGROUND)
        />
    }
    .into_any()
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
                layout=ColourLayout::Swatches
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
            <Slider
                label="Opacity"
                value=Signal::derive(move || opacity.get().uniform().map(Bounded::<ambre_core::Opacity>::fraction))
                on_write=Callback::new(move |p: Bounded<UnitFraction>| {
                    commit(StyleEdit::Opacity(Bounded::<ambre_core::Opacity>::of_fraction(p)));
                })
                reach=ambre_chrome::commands::Reach::Property(panel::OPACITY)
            />
        </Field>
    }
}
