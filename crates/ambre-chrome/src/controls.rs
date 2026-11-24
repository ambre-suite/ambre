// ambre-chrome/src/controls.rs
// ============================================================================
// The control primitives of the chrome (`ambre-spec-shell.md` section 2, the
// visual system). Each one draws one control at a height of the control
// scale, with the focus ring and the colours of the design tokens.
//
//   IconButton:  an icon in a box of one size, a tooltip with its name,
//                which a long press of a touch shows too (`tip`).
//   Select:      a styled list of choices, operable from the keyboard.
//   SegmentBar:  a row of exclusive choices on one track.
//   Stepper:     a number, its unit, and two steppers in one field.
//   ColorSwatch: a square of one colour that reports a press.
//   Slider:      a track of the accent, whose drag leaves as one write.
//
// None is generic. A choice is named by its place, so one copy of each
// control serves every value type. The property shims of `kit` map a value
// to its place and back. A generic control would compile once per type, and
// the main wasm is near its ceiling (`ambre-spec-budgets.md` section 5).
//
// Every control prevents the default of `mousedown`. A control that takes the
// focus would steal it from the canvas text pipe (the format bar hazard). The
// keyboard still reaches each one through Tab.
// ============================================================================

use ambre_commands::{Key, KeyBinding, Reach};
use ambre_core::Flag as _;
use ambre_core::index::{Axis, Circular, Pos};
use ambre_core::{Availability, Bounded, Css, Point, Px, Rgba, UnitFraction};
use leptos::prelude::*;
use leptos::wasm_bindgen::JsCast;
use leptos_icons::Icon;

use crate::debounce::{DRAG_DEBOUNCE, Debouncer};
use crate::format::Pressed;
use crate::kit::Step;
use crate::menu::{MenuKey, MenuRow, Openness, Selected, initial, menu_key};
use crate::tip::{LONG_PRESS, PointerKind, Press, PressEnd, TIP_DWELL, next_tip};

/// The height of a control, one step of the control scale of the tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ControlSize {
    /// A dense list: a row of a popover.
    Small,
    /// A row of the context panel.
    #[default]
    Medium,
    /// A floating bar, which a pointer reaches from afar.
    Large,
}

impl ControlSize {
    /// The word the stylesheet reads off `data-size`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Small => "sm",
            Self::Medium => "md",
            Self::Large => "lg",
        }
    }
}

/// How an [`IconButton`] draws its box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IconLook {
    /// A border on a sunken ground: a toggle or an action of a panel section.
    #[default]
    Outlined,
    /// No border and no ground until the pointer comes: the frame of the
    /// panel, a rail, a floating bar.
    Ghost,
}

impl IconLook {
    /// The word the stylesheet reads off `data-look`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Outlined => "outlined",
            Self::Ghost => "ghost",
        }
    }
}

/// The choices of a [`Select`] or a [`SegmentBar`], in the order they show.
#[derive(PartialEq, Eq)]
pub enum ChoiceRow {}

impl Axis for ChoiceRow {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("ChoiceRow");
}

/// The arrows wrap in an open list: past the last choice comes the first.
impl Circular for ChoiceRow {}

/// One choice a [`Select`] or a [`SegmentBar`] shows: a label, and an icon
/// a segment shows in place of its label.
#[derive(Clone)]
pub struct ChoiceView {
    /// The label a person reads, and the tooltip of an icon.
    pub label: String,
    /// The icon of a segment. A choice with none shows its label.
    pub icon: Option<icondata::Icon>,
    /// The command a pick of this choice runs, when the list runs one
    /// command per choice (`CHROME-207`). `None` for a choice that writes a
    /// value of the property the control names.
    pub reach: Option<Reach>,
    /// The CSS `font-family` stack the choice shows in, so a list of fonts
    /// previews each one. `None` keeps the face of the control.
    pub face: Option<&'static str>,
    /// A muted hint at the end of the row of a [`Select`], as the chord that
    /// picks the choice. The list of keys announces it, so a screen reader
    /// skips it here.
    pub hint: Option<String>,
}

impl ChoiceView {
    /// A choice that shows its label.
    pub fn text(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            icon: None,
            reach: None,
            face: None,
            hint: None,
        }
    }

    /// A choice that shows `icon`, its label the tooltip.
    pub fn icon(label: impl Into<String>, icon: icondata::Icon) -> Self {
        Self {
            icon: Some(icon),
            ..Self::text(label)
        }
    }

    /// The same choice, shown in the face of the CSS `font-family` stack `css`.
    #[must_use]
    pub fn in_face(self, css: &'static str) -> Self {
        Self {
            face: Some(css),
            ..self
        }
    }

    /// The same choice, with `hint` at the end of its row.
    #[must_use]
    pub fn hinted(self, hint: impl Into<String>) -> Self {
        Self {
            hint: Some(hint.into()),
            ..self
        }
    }

    /// The same choice, naming the command its pick runs.
    #[must_use]
    pub fn reaching(self, reach: Reach) -> Self {
        Self {
            reach: Some(reach),
            ..self
        }
    }
}

/// The point under a control's box where its tooltip or its list hangs, in
/// the viewport. A fixed surface escapes the scroll of the panel.
fn below(el: &web_sys::Element) -> Point<Px<Css>> {
    let rect = el.get_bounding_client_rect();
    Point {
        x: Px::<Css>::new((rect.left() + rect.width() / 2.0) as f32),
        y: Px::<Css>::new(rect.bottom() as f32),
    }
}

/// The viewport point that a fixed descendant of `el` measures its `left` and
/// `top` from. That point is the corner of the viewport by default. An
/// ancestor with a transform, a filter, a perspective or a containment holds
/// its fixed descendants, as a floating bar centred by a transform does. The
/// point is then the corner of that ancestor.
fn fixed_origin(el: &web_sys::Element) -> Point<Px<Css>> {
    let Some(window) = web_sys::window() else {
        return Point::default();
    };
    let mut at = Some(el.clone());
    while let Some(node) = at {
        if let Ok(Some(style)) = window.get_computed_style(&node) {
            let set = |name| {
                let value = style.get_property_value(name).unwrap_or_default();
                !value.is_empty() && value != "none" && value != "auto" && value != "normal"
            };
            let will_change = style.get_property_value("will-change").unwrap_or_default();
            let contain = style.get_property_value("contain").unwrap_or_default();
            let holds = set("transform")
                || set("filter")
                || set("backdrop-filter")
                || set("perspective")
                || will_change.contains("transform")
                || will_change.contains("filter")
                || ["paint", "layout", "strict", "content"]
                    .iter()
                    .any(|word| contain.contains(word));
            if holds {
                let rect = node.get_bounding_client_rect();
                return Point {
                    x: Px::<Css>::new((rect.left() + f64::from(node.client_left())) as f32),
                    y: Px::<Css>::new((rect.top() + f64::from(node.client_top())) as f32),
                };
            }
        }
        at = node.parent_element();
    }
    Point::default()
}

/// A button that shows one icon in a box of one size. A tooltip shows its
/// name, its chord and its line of description under the pointer and the
/// keyboard focus (`CHROME-185`, `CHROME-187`). On a touch screen, a long
/// press shows the tooltip in place of the press (`CHROME-188`).
///
/// The name is the accessible name too. The chord and the description are its
/// accessible description, so a screen reader hears what the tooltip shows
/// (`CHROME-189`). A command's icon takes its words from a
/// [`CommandTip`](crate::CommandTip) (`CHROME-186`).
///
/// `pressed` makes it a toggle. `expanded` makes it the opener of a list or a
/// panel. `reach` is the id of the command or the property it reaches
/// (`CHROME-207`). The icon and the name may follow a state, as a toggle whose
/// press does the opposite does. `class` names the button for the stylesheet
/// of its application and for its tests.
#[component]
pub fn IconButton(
    #[prop(into)] icon: Signal<icondata::Icon>,
    #[prop(into)] label: Signal<String>,
    on_press: Callback<()>,
    #[prop(optional_no_strip)] chord: Option<KeyBinding>,
    #[prop(optional_no_strip)] description: Option<
        ambre_core::Validated<ambre_commands::CommandDescription>,
    >,
    #[prop(optional_no_strip)] pressed: Option<Signal<Pressed>>,
    #[prop(optional_no_strip)] expanded: Option<Signal<Openness>>,
    #[prop(optional, into)] availability: Option<Signal<Availability>>,
    #[prop(optional)] size: ControlSize,
    #[prop(optional)] look: IconLook,
    #[prop(optional_no_strip)] reach: Option<Reach>,
    #[prop(optional)] class: Option<&'static str>,
    #[prop(optional)] children: Option<Children>,
) -> impl IntoView {
    let tip_at = RwSignal::new(None::<Point<Px<Css>>>);
    let press = StoredValue::new(Press::Idle);
    let wait = StoredValue::new(None::<TimeoutHandle>);
    let node = NodeRef::<leptos::html::Button>::new();
    // The tooltip hangs under the button, in the frame of whatever holds its
    // fixed box: the viewport, or a floating bar a transform centres.
    let show = move || {
        if let Some(el) = node.get_untracked() {
            let (at, origin) = (below(&el), fixed_origin(&el));
            tip_at.try_set(Some(Point {
                x: at.x - origin.x,
                y: at.y - origin.y,
            }));
        }
    };
    let hide = move || {
        tip_at.try_set(None);
    };
    let cancel_wait = move || {
        if let Some(handle) = wait.try_get_value().flatten() {
            handle.clear();
        }
        wait.try_set_value(None);
    };
    on_cleanup(cancel_wait);
    // A touch starts the wait of a long press; the end of the wait shows the
    // tooltip if the touch still holds the icon.
    let down = move |ev: web_sys::PointerEvent| {
        cancel_wait();
        let next = Press::down(PointerKind::of(&ev));
        press.set_value(next);
        if next == Press::Pending {
            let handle = set_timeout_with_handle(
                move || {
                    let held = press.try_get_value().map(Press::waited);
                    if let Some(held) = held {
                        press.try_set_value(held);
                    }
                    if held == Some(Press::Held) {
                        show();
                    }
                },
                LONG_PRESS,
            );
            wait.set_value(handle.ok());
        }
    };
    // The finger lifts. A long press keeps its tooltip a moment, so the
    // person reads it with the finger away.
    let lift = move || {
        cancel_wait();
        let next = press.get_value().lifted();
        press.set_value(next);
        if next == Press::Held {
            set_timeout(hide, TIP_DWELL);
        }
    };
    let class = class.map_or_else(
        || "ambre-icon-button".to_owned(),
        |c| format!("ambre-icon-button {c}"),
    );
    let described =
        (chord.is_some() || description.is_some()).then(|| format!("ambre-tip-{}", next_tip()));
    let spoken = {
        let parts: Vec<String> = chord
            .map(|c| c.to_string())
            .into_iter()
            .chain(description.as_ref().map(|d| d.as_str().to_owned()))
            .collect();
        parts.join(". ")
    };
    let line = description.map(|d| d.into_string());
    let described_by = described.clone();
    view! {
        <button
            class=class
            node_ref=node
            data-size=size.as_str()
            data-look=look.as_str()
            data-reach=reach.map(|r| r.marker())
            aria-label=move || label.get()
            aria-describedby=described_by
            aria-keyshortcuts=chord.map(|c| c.to_string())
            aria-pressed=move || pressed.map(|p| p.get().aria())
            aria-expanded=move || expanded.map(|e| e.get().aria())
            disabled=move || availability.is_some_and(|a| a.get() == Availability::Disabled)
            on:mousedown=move |ev| ev.prevent_default()
            on:pointerdown=down
            on:pointerup=move |_| lift()
            on:pointercancel=move |_| lift()
            on:pointerenter=move |ev: web_sys::PointerEvent| {
                if PointerKind::of(&ev) == PointerKind::Hover {
                    show();
                }
            }
            on:pointerleave=move |ev: web_sys::PointerEvent| {
                if PointerKind::of(&ev) == PointerKind::Hover {
                    hide();
                }
            }
            on:contextmenu=move |ev| {
                // The platform menu of a long press would cover the tooltip.
                if press.get_value() != Press::Idle {
                    ev.prevent_default();
                }
            }
            on:focus=move |_| show()
            on:blur=move |_| hide()
            on:click=move |_| {
                let end = press.get_value().click();
                press.set_value(Press::Idle);
                if end == PressEnd::KeepTip {
                    return;
                }
                hide();
                on_press.run(());
            }
        >
            <Icon icon=icon />
            {children.map(|c| c())}
            <span
                class="ambre-tip"
                role="tooltip"
                aria-hidden="true"
                data-shown=move || if tip_at.get().is_some() { "true" } else { "false" }
                style:left=move || tip_at.get().map(|p| format!("{}px", p.x.get()))
                style:top=move || tip_at.get().map(|p| format!("{}px", p.y.get()))
            >
                <span class="ambre-tip-head">
                    <span class="ambre-tip-name">{move || label.get()}</span>
                    {chord.map(|c| view! { <crate::Kbd binding=c /> })}
                </span>
                {line.map(|l| view! { <span class="ambre-tip-description">{l}</span> })}
            </span>
            {described.map(|id| view! { <span id=id hidden>{spoken}</span> })}
        </button>
    }
}

/// The place of the choice an open list moves to on `ev`, from `at`, or what
/// else the key does. Tab leaves a list, as a native select lets it.
fn list_key(ev: &web_sys::KeyboardEvent, at: Pos<MenuRow>, initials: &[char]) -> MenuKey {
    let Some(key) = Key::from_dom(&ev.key()) else {
        return MenuKey::Pass;
    };
    if key == Key::Tab {
        return MenuKey::Dismiss;
    }
    menu_key(key, crate::command::modifiers_of(ev), at, initials)
}

/// The options of a rendered list, in order.
fn options(list: &web_sys::Element) -> Vec<web_sys::HtmlElement> {
    let Ok(found) = list.query_selector_all("[role=\"option\"]") else {
        return Vec::new();
    };
    (0..found.length())
        .filter_map(|i| found.item(i)?.dyn_into::<web_sys::HtmlElement>().ok())
        .collect()
}

/// A list of choices behind a button that shows the current one. It opens
/// under the button, or above it at the bottom of the window, and a press
/// outside closes it.
///
/// The keyboard opens it with Enter, Space or the down arrow. The arrows,
/// Home, End and a first letter move, Enter picks, and Escape or Tab close.
/// `selected` is `None` for a mixed selection, or for a value no choice
/// names, which shows `placeholder`. The button shows the face of the
/// current choice, as each row shows its own.
#[component]
pub fn Select(
    #[prop(into)] label: String,
    choices: Vec<ChoiceView>,
    #[prop(into)] selected: Signal<Option<Pos<ChoiceRow>>>,
    on_pick: Callback<Pos<ChoiceRow>>,
    #[prop(into, default = Signal::stored("Mixed".to_owned()))] placeholder: Signal<String>,
    #[prop(optional)] reach: Option<Reach>,
    #[prop(optional)] size: ControlSize,
) -> impl IntoView {
    let open = RwSignal::new(Openness::Closed);
    let at = RwSignal::new(None::<Point<Px<Css>>>);
    let root = NodeRef::<leptos::html::Span>::new();
    let trigger = NodeRef::<leptos::html::Button>::new();
    let list = NodeRef::<leptos::html::Div>::new();
    let initials: Vec<char> = choices.iter().map(|c| initial(&c.label)).collect();
    let count = choices.len();

    let close = move || {
        open.set(Openness::Closed);
        at.set(None);
    };
    let press = window_event_listener(leptos::ev::pointerdown, move |ev| {
        if open.get_untracked() == Openness::Closed {
            return;
        }
        let inside = root
            .get_untracked()
            .zip(ev.target())
            .and_then(|(r, t)| t.dyn_into::<web_sys::Node>().ok().map(|n| (r, n)))
            .is_some_and(|(r, n)| r.contains(Some(&n)));
        if !inside {
            close();
        }
    });
    on_cleanup(move || press.remove());

    // Open under the trigger. The keyboard takes the focus to the current
    // choice; a pointer leaves it on the canvas.
    let open_list = move |focus: Openness| {
        let Some(button) = trigger.get_untracked() else {
            return;
        };
        at.set(Some(below(&button)));
        open.set(Openness::Open);
        if focus == Openness::Open
            && let Some(list) = list.get_untracked()
        {
            // The attribute follows the signal on a later tick; an element
            // still hidden refuses the focus, so show it now.
            list.set_hidden(false);
            let index = selected
                .get_untracked()
                .and_then(Pos::to_index)
                .unwrap_or(0);
            if let Some(option) = options(&list).get(index) {
                let _ = option.focus();
            }
        }
    };
    let pick = move |p: Pos<ChoiceRow>| {
        on_pick.run(p);
        close();
        if let Some(button) = trigger.get_untracked() {
            let _ = button.focus();
        }
    };
    let on_trigger_key = move |ev: web_sys::KeyboardEvent| {
        if matches!(ev.key().as_str(), "Enter" | " " | "ArrowDown") {
            ev.prevent_default();
            open_list(Openness::Open);
        }
    };
    let keys = initials.clone();
    let on_list_key = move |ev: web_sys::KeyboardEvent| {
        let Some(list) = list.get_untracked() else {
            return;
        };
        let items = options(&list);
        let active = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.active_element());
        let here = items
            .iter()
            .position(|o| active.as_ref() == Some(o.as_ref()))
            .map_or(Pos::ZERO, Pos::from_usize);
        match list_key(&ev, here, &keys) {
            MenuKey::Focus(p) => {
                ev.prevent_default();
                if let Some(o) = p.to_index().and_then(|i| items.get(i)) {
                    let _ = o.focus();
                }
            }
            MenuKey::Choose(p) => {
                ev.prevent_default();
                if let Some(i) = p.to_index().filter(|&i| i < count) {
                    pick(Pos::from_usize(i));
                }
            }
            MenuKey::Dismiss => {
                if ev.key() != "Tab" {
                    ev.prevent_default();
                }
                close();
                if let Some(button) = trigger.get_untracked() {
                    let _ = button.focus();
                }
            }
            MenuKey::Pass => {}
        }
    };

    let current = {
        let choices = choices.clone();
        move || {
            selected
                .get()
                .and_then(Pos::to_index)
                .and_then(|i| choices.get(i).cloned())
        }
    };
    let current_face = current.clone();
    let shown = move || current().map_or_else(|| placeholder.get(), |c| c.label);
    let shown_face = move || current_face().and_then(|c| c.face);
    let rows = choices
        .into_iter()
        .enumerate()
        .map(|(i, choice)| {
            let p = Pos::<ChoiceRow>::from_usize(i);
            let is = move || selected.get() == Some(p);
            view! {
                <div
                    class="ambre-select-option"
                    role="option"
                    tabindex="-1"
                    data-reach=choice.reach.map(|r| r.marker())
                    style:font-family=choice.face
                    aria-selected=move || Selected::from_bool(is()).aria()
                    on:mousedown=move |ev| ev.prevent_default()
                    on:click=move |_| pick(p)
                >
                    {choice.icon.map(|icon| view! { <Icon icon=icon /> })}
                    <span>{choice.label}</span>
                    {choice.hint.map(|hint| view! {
                        <span class="ambre-select-hint" aria-hidden="true">{hint}</span>
                    })}
                </div>
            }
        })
        .collect_view();

    view! {
        <span class="ambre-select" node_ref=root data-reach=reach.map(|r| r.marker())>
            <button
                class="ambre-select-trigger"
                node_ref=trigger
                data-size=size.as_str()
                aria-haspopup="listbox"
                aria-expanded=move || open.get().aria()
                aria-label=label.clone()
                on:mousedown=move |ev| ev.prevent_default()
                on:click=move |_| match open.get_untracked() {
                    Openness::Open => close(),
                    Openness::Closed => open_list(Openness::Closed),
                }
                on:keydown=on_trigger_key
            >
                <span class="ambre-select-value" style:font-family=shown_face>
                    {shown}
                </span>
                <Icon icon=icondata::LuChevronDown />
            </button>
            <div
                class="ambre-select-list"
                role="listbox"
                aria-label=label
                node_ref=list
                hidden=move || open.get() == Openness::Closed
                style:left=move || at.get().map(|p| format!("{}px", p.x.get()))
                style:top=move || at.get().map(|p| format!("{}px", p.y.get()))
                on:keydown=on_list_key
            >
                {rows}
            </div>
        </span>
    }
}

/// A row of exclusive choices on one track: an alignment, an orientation.
/// The chosen segment rises off the track. `selected` is `None` for a mixed
/// selection, which checks no segment.
#[component]
pub fn SegmentBar(
    #[prop(into)] label: String,
    choices: Vec<ChoiceView>,
    #[prop(into)] selected: Signal<Option<Pos<ChoiceRow>>>,
    on_pick: Callback<Pos<ChoiceRow>>,
    #[prop(optional)] reach: Option<Reach>,
) -> impl IntoView {
    let segments = choices
        .into_iter()
        .enumerate()
        .map(|(i, choice)| {
            let p = Pos::<ChoiceRow>::from_usize(i);
            let checked = move || selected.get() == Some(p);
            let name = choice.label.clone();
            view! {
                <button
                    role="radio"
                    data-reach=choice.reach.map(|r| r.marker())
                    aria-checked=move || crate::Checked::from_bool(checked()).aria()
                    aria-label=name.clone()
                    title=name
                    on:mousedown=move |ev| ev.prevent_default()
                    on:click=move |_| on_pick.run(p)
                >
                    {match choice.icon {
                        Some(icon) => view! { <Icon icon=icon /> }.into_any(),
                        None => view! { <span>{choice.label}</span> }.into_any(),
                    }}
                </button>
            }
        })
        .collect_view();
    view! {
        <span class="ambre-segments" role="radiogroup" aria-label=label data-reach=reach.map(|r| r.marker())>
            {segments}
        </span>
    }
}

/// A number and its unit in one field, between a step down and a step up.
/// `text` is the number shown, `None` for a mixed selection. A typed number
/// leaves through `on_text` when the field commits it.
#[component]
pub fn Stepper(
    #[prop(into)] label: String,
    #[prop(into)] text: Signal<Option<String>>,
    unit: &'static str,
    on_step: Callback<Step>,
    on_text: Callback<String>,
    #[prop(optional)] reach: Option<Reach>,
) -> impl IntoView {
    let less = format!("{label}: less");
    let more = format!("{label}: more");
    view! {
        <span class="ambre-stepper" data-reach=reach.map(|r| r.marker())>
            <button
                class="ambre-stepper-step"
                aria-label=less
                on:mousedown=move |ev| ev.prevent_default()
                on:click=move |_| on_step.run(Step::Down)
            >
                <Icon icon=icondata::LuMinus />
            </button>
            <input
                type="text"
                inputmode="decimal"
                aria-label=label
                placeholder=move || text.get().is_none().then_some("Mixed")
                prop:value=move || text.get().unwrap_or_default()
                on:change=move |ev| on_text.run(event_target_value(&ev))
            />
            <span class="ambre-stepper-unit">{unit}</span>
            <button
                class="ambre-stepper-step"
                aria-label=more
                on:mousedown=move |ev| ev.prevent_default()
                on:click=move |_| on_step.run(Step::Up)
            >
                <Icon icon=icondata::LuPlus />
            </button>
        </span>
    }
}

/// A square of one colour that reports a press. `colour` is `None` for no
/// colour, which shows a struck square. `pressed` marks the chosen swatch of
/// a row.
#[component]
pub fn ColorSwatch(
    #[prop(into)] label: String,
    #[prop(into)] colour: Signal<Option<Rgba>>,
    on_press: Callback<()>,
    #[prop(optional, into)] pressed: Option<Signal<Pressed>>,
    #[prop(optional)] reach: Option<Reach>,
) -> impl IntoView {
    let tip = label.clone();
    view! {
        <button
            class="ambre-swatch"
            data-reach=reach.map(|r| r.marker())
            data-empty=move || if colour.get().is_none() { "true" } else { "false" }
            aria-label=label
            title=tip
            aria-pressed=move || pressed.map(|p| p.get().aria())
            style:background-color=move || colour.get().map(|c| c.to_string())
            on:mousedown=move |ev| ev.prevent_default()
            on:click=move |_| on_press.run(())
        ></button>
    }
}

/// What a [`Slider`] shows beside its track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Readout {
    /// The position as a whole percent.
    #[default]
    Percent,
    /// Nothing: the track says enough.
    Hidden,
}

/// The steps of a [`Slider`] track, the resolution of the range input.
const SLIDER_STEPS: f32 = 1000.0;

/// The text of a readout for the place `p` of a track.
fn readout_text(readout: Readout, p: Bounded<UnitFraction>) -> String {
    match readout {
        Readout::Percent => format!("{:.0}%", p.get() * 100.0),
        Readout::Hidden => String::new(),
    }
}

/// A track of the accent with a thumb. The thumb follows the drag at once,
/// and the burst leaves as one write of its last place (`CHROME-015`).
/// `value` is `None` for a mixed selection.
#[component]
pub fn Slider(
    #[prop(into)] label: String,
    #[prop(into)] value: Signal<Option<Bounded<UnitFraction>>>,
    on_write: Callback<Bounded<UnitFraction>>,
    #[prop(optional)] readout: Readout,
    #[prop(optional)] reach: Option<Reach>,
) -> impl IntoView {
    let drag = Debouncer::new();
    // The place under the thumb: the drag moves it before the write leaves.
    let local = RwSignal::new(value.get_untracked());
    Effect::new(move |_| local.set(value.get()));
    let fill = move || {
        local
            .get()
            .map_or_else(|| "0%".to_owned(), |p| format!("{}%", p.get() * 100.0))
    };
    view! {
        <span class="ambre-slider" data-reach=reach.map(|r| r.marker())>
            <input
                type="range"
                min="0"
                max=SLIDER_STEPS.to_string()
                aria-label=label
                style:--fill=fill
                prop:value=move || {
                    local.get().map(|p| format!("{:.0}", p.get() * SLIDER_STEPS)).unwrap_or_default()
                }
                on:mousedown=move |ev| ev.stop_propagation()
                on:input=move |ev| {
                    let Ok(step) = event_target_value(&ev).parse::<f32>() else {
                        return;
                    };
                    let place = Bounded::<UnitFraction>::new(step / SLIDER_STEPS)
                        .unwrap_or(Bounded::<UnitFraction>::ONE);
                    local.set(Some(place));
                    drag.call(DRAG_DEBOUNCE, move || on_write.run(place));
                }
            />
            <span class="ambre-slider-readout">
                {move || local.get().map(|p| readout_text(readout, p))}
            </span>
        </span>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_size_names_its_step_of_the_scale() {
        let words: Vec<&str> = [ControlSize::Small, ControlSize::Medium, ControlSize::Large]
            .into_iter()
            .map(ControlSize::as_str)
            .collect();
        assert_eq!(words, ["sm", "md", "lg"]);
        assert_eq!(ControlSize::default(), ControlSize::Medium);
        assert_eq!(IconLook::default(), IconLook::Outlined);
        assert_eq!(IconLook::Ghost.as_str(), "ghost");
    }

    #[test]
    fn a_percent_readout_rounds_to_a_whole_percent() {
        let p = Bounded::<UnitFraction>::new(0.426).expect("in range");
        assert_eq!(readout_text(Readout::Percent, p), "43%");
        assert_eq!(readout_text(Readout::Hidden, p), "");
        assert_eq!(
            readout_text(Readout::Percent, Bounded::<UnitFraction>::ONE),
            "100%"
        );
    }
}
