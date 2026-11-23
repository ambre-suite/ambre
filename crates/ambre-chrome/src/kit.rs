// ambre-chrome/src/kit.rs
// ============================================================================
// The kit of typed widgets an application writes its panel sections from
// (`ambre-spec-shell.md` section 2, the property model).
//
// Two kinds of widget. Structure lays a section out: `Field` puts a label
// before a control, `Row` puts controls side by side. A control edits one
// value type, or runs one command. Each control carries the id it reaches on
// its `data-reach` attribute (`CHROME-207`). A test then reads what a panel
// offers, and holds the context bar and the context menu to it.
//
// A property control reads a `Reading`: one value, or a mixed selection
// (`CHROME-210`). It writes back through one callback, which carries the new
// value alone (`CHROME-211`). The colour and the font controls are the ones
// every application already shares, so one colour picker serves the suite.
//
// Every control prevents the default of `mousedown`. A button that takes the
// focus would steal it from the canvas text pipe (the format bar hazard).
// ============================================================================

use ambre_commands::{Command, Locale, Property, Reach, Reading};
use ambre_core::Flag as _;
use ambre_core::FontFamily;
use ambre_core::{Commands, Mm, PrintLength, Px, StaticKey, StoredBoard};
use leptos::prelude::*;

use crate::color::PickedColour;
use crate::format::Pressed;

/// The label of a command, from its description and never a literal
/// (`UI-THREAD-036`).
fn command_label(command: &Command) -> String {
    (command.title)(&Locale::En).into_string()
}

/// The label of a property, from its description.
fn property_label(property: &Property) -> String {
    (property.title)(&Locale::En).into_string()
}

/// The hint a control shows beside a mixed value.
fn mixed_hint(mixed: bool) -> impl IntoView {
    mixed.then(|| view! { <span class="ambre-kit-mixed">"Mixed"</span> })
}

/// A label before a control, on one line of a section.
#[component]
pub fn Field(label: &'static str, children: Children) -> impl IntoView {
    view! {
        <div class="ambre-kit-field">
            <span class="ambre-kit-label">{label}</span>
            <div class="ambre-kit-control">{children()}</div>
        </div>
    }
}

/// Controls side by side, on one line of a section.
#[component]
pub fn Row(children: Children) -> impl IntoView {
    view! { <div class="ambre-kit-row">{children()}</div> }
}

/// A button that runs one command. With `pressed`, it is a toggle chip that
/// shows the state of the command on the selection.
#[component]
pub fn CommandButton(
    command: &'static Command,
    #[prop(into)] icon: Option<icondata::Icon>,
    on_command: Callback<StaticKey<Commands>>,
    #[prop(optional, into)] pressed: Option<Signal<Pressed>>,
    /// Show the label beside the icon. A button with no icon always shows it.
    #[prop(optional)]
    labelled: Labelled,
) -> impl IntoView {
    let label = command_label(command);
    let id = command.id;
    let show_text = labelled == Labelled::Text || icon.is_none();
    let (aria, tip) = (label.clone(), label.clone());
    view! {
        <button
            class="ambre-kit-command"
            data-reach=Reach::Command(id).marker()
            aria-label=aria
            title=tip
            aria-pressed=move || pressed.map(|p| p.get().aria())
            on:mousedown=move |ev| ev.prevent_default()
            on:click=move |_| on_command.run(id)
        >
            {icon.map(|icon| view! { <leptos_icons::Icon icon=icon /> })}
            {show_text.then(|| view! { <span>{label}</span> })}
        </button>
    }
}

/// A toggle of a property that holds one of two values, such as the weight
/// of a text: pressed, released, or mixed across the selection.
#[component]
pub fn PropertyToggle(
    property: &'static Property,
    icon: icondata::Icon,
    #[prop(into)] pressed: Signal<Pressed>,
    on_toggle: Callback<()>,
) -> impl IntoView {
    let label = property_label(property);
    let tip = label.clone();
    view! {
        <button
            class="ambre-kit-command"
            data-reach=Reach::Property(property.id).marker()
            aria-label=label
            title=tip
            aria-pressed=move || pressed.get().aria()
            on:mousedown=move |ev| ev.prevent_default()
            on:click=move |_| on_toggle.run(())
        >
            <leptos_icons::Icon icon=icon />
        </button>
    }
}

/// Whether a [`CommandButton`] shows its label beside its icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Labelled {
    /// The icon alone; the label is the tooltip and the accessible name.
    #[default]
    Icon,
    /// The icon and the label.
    Text,
}

/// The colour of a property: the colour picker every application shares, and
/// a mixed hint.
#[component]
pub fn ColourField<C: PickedColour>(
    property: &'static Property,
    icon: icondata::Icon,
    #[prop(into)] value: Signal<Reading<Option<C>>>,
    #[prop(into)] default: Signal<C>,
    reset_label: &'static str,
    on_write: Callback<Option<C>>,
) -> impl IntoView {
    // The picker edits a signal of its own; a reading seeds it.
    let current = RwSignal::new(value.get_untracked().uniform().flatten());
    Effect::new(move |_| current.set(value.get().uniform().flatten()));
    view! {
        <span class="ambre-kit-colour" data-reach=Reach::Property(property.id).marker()>
            <crate::ColorPopover
                title=property_label(property)
                icon=icon
                current=current
                default=default
                reset_label=reset_label
                on_pick=on_write
            />
            {move || mixed_hint(value.get().is_mixed())}
        </span>
    }
}

/// The font family of a property: the picker that shows each family in
/// itself.
#[component]
pub fn FontField(
    property: &'static Property,
    #[prop(into)] value: Signal<Reading<Option<FontFamily>>>,
    on_write: Callback<Option<FontFamily>>,
) -> impl IntoView {
    let current = Signal::derive(move || value.get().uniform().flatten());
    view! {
        <span
            class="ambre-kit-font"
            data-reach=Reach::Property(property.id).marker()
            aria-label=property_label(property)
        >
            <crate::FontPicker current=current on_pick=on_write />
            {move || mixed_hint(value.get().is_mixed())}
        </span>
    }
}

/// The direction of a stepper of a [`LengthField`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// One step less.
    Down,
    /// One step more.
    Up,
}

/// A length a [`LengthField`] edits: its unit, its step and its text.
pub trait FieldLength: Copy + PartialEq + Send + Sync + 'static {
    /// The unit the field shows after the number.
    const UNIT: &'static str;

    /// The length one stepper moves, from `self`, never below zero.
    #[must_use]
    fn stepped(self, step: Step) -> Self;

    /// The number the field shows.
    fn field_text(self) -> String;

    /// The length a typed number names, or `None` for a text that is no
    /// length: a negative number, or no number.
    fn from_field(text: &str) -> Option<Self>;
}

/// The number a field reads, when it is a finite length of zero or more.
fn field_number(text: &str) -> Option<f32> {
    text.trim()
        .replace(',', ".")
        .parse::<f32>()
        .ok()
        .filter(|v| v.is_finite() && *v >= 0.0)
}

impl FieldLength for PrintLength<Mm> {
    const UNIT: &'static str = "mm";

    fn stepped(self, step: Step) -> Self {
        let by = match step {
            Step::Down => -1.0,
            Step::Up => 1.0,
        };
        Self::new((self.get() + by).max(0.0))
    }

    fn field_text(self) -> String {
        format!("{:.1}", self.get())
    }

    fn from_field(text: &str) -> Option<Self> {
        field_number(text).map(Self::new)
    }
}

impl FieldLength for Px<StoredBoard> {
    const UNIT: &'static str = "px";

    fn stepped(self, step: Step) -> Self {
        let by = match step {
            Step::Down => -0.5,
            Step::Up => 0.5,
        };
        Self::new((self.get() + by).max(0.0))
    }

    fn field_text(self) -> String {
        format!("{:.1}", self.get())
    }

    fn from_field(text: &str) -> Option<Self> {
        field_number(text).map(Self::new)
    }
}

/// A length of a property: a number, its unit, and two steppers.
#[component]
pub fn LengthField<L: FieldLength>(
    property: &'static Property,
    #[prop(into)] value: Signal<Reading<L>>,
    on_write: Callback<L>,
) -> impl IntoView {
    let label = property_label(property);
    let step = move |s: Step| {
        if let Some(v) = value.get_untracked().uniform() {
            on_write.run(v.stepped(s));
        }
    };
    view! {
        <span class="ambre-kit-length" data-reach=Reach::Property(property.id).marker()>
            <button
                class="icon"
                aria-label=format!("{label}: less")
                on:mousedown=move |ev| ev.prevent_default()
                on:click=move |_| step(Step::Down)
            >
                "\u{2212}"
            </button>
            <input
                type="text"
                inputmode="decimal"
                aria-label=label.clone()
                placeholder=move || value.get().is_mixed().then_some("Mixed")
                prop:value=move || value.get().uniform().map(L::field_text).unwrap_or_default()
                on:change=move |ev| {
                    if let Some(v) = L::from_field(&event_target_value(&ev)) {
                        on_write.run(v);
                    }
                }
            />
            <span class="ambre-kit-unit">{L::UNIT}</span>
            <button
                class="icon"
                aria-label=format!("{label}: more")
                on:mousedown=move |ev| ev.prevent_default()
                on:click=move |_| step(Step::Up)
            >
                "+"
            </button>
        </span>
    }
}

/// One choice of a [`Segmented`] control or a [`SelectField`].
#[derive(Clone, Copy)]
pub struct Choice<T> {
    /// The value the choice writes.
    pub value: T,
    /// The label a person reads, or the tooltip of an icon.
    pub label: &'static str,
    /// The icon, for a segmented control. A choice with none shows its label.
    pub icon: Option<icondata::Icon>,
}

/// A row of exclusive choices of a property: an alignment, an orientation.
#[component]
pub fn Segmented<T: Copy + PartialEq + Send + Sync + 'static>(
    property: &'static Property,
    choices: Vec<Choice<T>>,
    #[prop(into)] value: Signal<Reading<T>>,
    on_write: Callback<T>,
) -> impl IntoView {
    let buttons = choices
        .into_iter()
        .map(|choice| {
            let checked = move || value.get().uniform() == Some(choice.value);
            view! {
                <button
                    role="radio"
                    aria-checked=move || crate::Checked::from_bool(checked()).aria()
                    aria-label=choice.label
                    title=choice.label
                    on:mousedown=move |ev| ev.prevent_default()
                    on:click=move |_| on_write.run(choice.value)
                >
                    {match choice.icon {
                        Some(icon) => view! { <leptos_icons::Icon icon=icon /> }.into_any(),
                        None => view! { <span>{choice.label}</span> }.into_any(),
                    }}
                </button>
            }
        })
        .collect_view();
    view! {
        <span
            class="ambre-kit-segmented"
            role="radiogroup"
            aria-label=property_label(property)
            data-reach=Reach::Property(property.id).marker()
        >
            {buttons}
            {move || mixed_hint(value.get().is_mixed())}
        </span>
    }
}

/// A list of choices of a property, in a native select.
#[component]
pub fn SelectField<T: Copy + PartialEq + Send + Sync + 'static>(
    property: &'static Property,
    choices: Vec<Choice<T>>,
    #[prop(into)] value: Signal<Reading<T>>,
    on_write: Callback<T>,
) -> impl IntoView {
    let picked = choices.clone();
    let selected = move || {
        value
            .get()
            .uniform()
            .and_then(|v| picked.iter().position(|c| c.value == v))
            .map(|i| i.to_string())
            .unwrap_or_default()
    };
    let options = choices
        .iter()
        .enumerate()
        .map(|(i, c)| view! { <option value=i.to_string()>{c.label}</option> })
        .collect_view();
    view! {
        <select
            class="ambre-kit-select"
            aria-label=property_label(property)
            data-reach=Reach::Property(property.id).marker()
            prop:value=selected
            on:change=move |ev| {
                if let Some(choice) = event_target_value(&ev)
                    .parse::<usize>()
                    .ok()
                    .and_then(|i| choices.get(i))
                {
                    on_write.run(choice.value);
                }
            }
        >
            // The empty entry stands for a mixed selection, which no choice names.
            <option value="" disabled=true>"Mixed"</option>
            {options}
        </select>
    }
}

/// The `data-reach` markers of every control under `root`, in document order:
/// what the panel offers, as a test reads it (`CHROME-207`).
pub fn reach_markers(root: &web_sys::Element) -> Vec<String> {
    let Ok(list) = root.query_selector_all("[data-reach]") else {
        return Vec::new();
    };
    (0..list.length())
        .filter_map(|i| list.item(i))
        .filter_map(|n| leptos::wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(n).ok())
        .filter_map(|e| e.get_attribute("data-reach"))
        .collect()
}

/// The accessible names of the controls of the panel's sections that carry
/// no `data-reach` themselves nor inside one (`CHROME-207`). A test holds it
/// empty: every control names the command or the property it reaches.
pub fn unreached_controls(root: &web_sys::Element) -> Vec<String> {
    let Ok(list) = root.query_selector_all(
        ".ambre-ctx-section-body button, .ambre-ctx-section-body input, .ambre-ctx-section-body select",
    ) else {
        return Vec::new();
    };
    (0..list.length())
        .filter_map(|i| list.item(i))
        .filter_map(|n| leptos::wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(n).ok())
        .filter(|e| e.closest("[data-reach]").ok().flatten().is_none())
        .map(|e| {
            e.get_attribute("aria-label")
                .unwrap_or_else(|| e.outer_html())
        })
        .collect()
}
