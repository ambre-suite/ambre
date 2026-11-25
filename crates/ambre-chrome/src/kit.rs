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
use ambre_core::FontFamily;
use ambre_core::{Board, Commands, Mm, PrintLength, Px, Radians, StaticKey, StoredBoard};
use leptos::prelude::*;

use crate::color::PickedColour;
use crate::controls::ChoiceRow;
use crate::format::Pressed;
use ambre_core::index::Pos;

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
/// shows the state of the command on the selection. An icon alone is an
/// [`IconButton`](crate::IconButton): one box, and a tooltip with the name and
/// the chord of the command.
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
    let reach = Reach::Command(id);
    match (icon, labelled) {
        (Some(icon), Labelled::Icon) => view! {
            <crate::IconButton
                icon=icon
                label=label
                chord=command.chord()
                pressed=pressed
                reach=reach
                on_press=Callback::new(move |()| on_command.run(id))
            />
        }
        .into_any(),
        (icon, _) => {
            let (aria, tip) = (label.clone(), label.clone());
            view! {
                <button
                    class="ambre-kit-command"
                    data-reach=reach.marker()
                    aria-label=aria
                    aria-keyshortcuts=command.chord().map(|c| c.to_string())
                    title=tip
                    aria-pressed=move || pressed.map(|p| p.get().aria())
                    on:mousedown=move |ev| ev.prevent_default()
                    on:click=move |_| on_command.run(id)
                >
                    {icon.map(|icon| view! { <leptos_icons::Icon icon=icon /> })}
                    <span>{label}</span>
                    // A labelled row names its chord at its end (`CHROME-250`).
                    {command.chord().map(|c| view! { <crate::Kbd binding=c /> })}
                </button>
            }
            .into_any()
        }
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
    view! {
        <crate::IconButton
            icon=icon
            label=property_label(property)
            pressed=Some(pressed)
            reach=Reach::Property(property.id)
            on_press=on_toggle
        />
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
    /// Lay out the common colours inline, before the picker.
    #[prop(optional)]
    layout: ColourLayout,
) -> impl IntoView {
    // The picker edits a signal of its own; a reading seeds it.
    let current = RwSignal::new(value.get_untracked().uniform().flatten());
    Effect::new(move |_| current.set(value.get().uniform().flatten()));
    let inline = (layout == ColourLayout::Swatches).then(|| {
        swatch_row(
            reset_label,
            Signal::derive(move || current.get().map(C::as_rgba)),
            Callback::new(move |c: Option<ambre_core::Rgba>| on_write.run(c.map(C::from_rgba))),
        )
    });
    view! {
        <span class="ambre-kit-colour" data-reach=Reach::Property(property.id).marker()>
            {inline}
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

/// How a [`ColourField`] lays out its choices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColourLayout {
    /// The picker alone, behind its icon button.
    #[default]
    Popover,
    /// The common colours as inline swatches, then the picker as the
    /// custom-colour button (`CHROME-252`).
    Swatches,
}

/// The inline swatches of a colour field: no colour first, then each colour
/// of [`INLINE`](crate::color::INLINE). One copy of the view serves every
/// packing, which the caller converts.
fn swatch_row(
    none_label: &'static str,
    current: Signal<Option<ambre_core::Rgba>>,
    on_pick: Callback<Option<ambre_core::Rgba>>,
) -> AnyView {
    let choices = std::iter::once((none_label, None, None)).chain(
        crate::color::INLINE
            .iter()
            .map(|&(name, css, c)| (name, Some(css), Some(c))),
    );
    choices
        .map(|(name, css, rgb)| {
            let pressed =
                move || Pressed::from_bool(current.get().map(ambre_core::Rgba::rgb) == rgb).aria();
            let picked = rgb.map(|c| c.to_rgba(ambre_core::Alpha::OPAQUE));
            view! {
                <button
                    class="ambre-swatch"
                    data-empty=if css.is_none() { "true" } else { "false" }
                    aria-label=name
                    title=name
                    aria-pressed=pressed
                    style:background-color=css
                    on:mousedown=|ev| ev.prevent_default()
                    on:click=move |_| on_pick.run(picked)
                ></button>
            }
        })
        .collect_view()
        .into_any()
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

/// The unit of a [`LengthField`], as data. It names the suffix, the step of
/// one stepper, the floor and the scale the field shows at. One copy
/// of the field serves every unit, so each length type adds no code but its
/// adapter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LengthUnit {
    /// The unit the field shows after the number.
    pub suffix: &'static str,
    /// How far one stepper moves, in the shown scale.
    pub step: f64,
    /// The least value the field accepts, or `None` for a value of either
    /// sign: a place on the board or an angle.
    pub floor: Option<f64>,
    /// The shown number per stored unit: 1 for a length, degrees per radian
    /// for an angle.
    pub scale: f64,
}

impl LengthUnit {
    /// The text the field shows for `value`, to one decimal and with no
    /// decimal when whole: "20", never "20.0", and "12.7".
    #[must_use]
    pub fn text(&self, value: f64) -> String {
        // The field shows a tenth, so `f32` holds it; Main already formats
        // `f32`, and a second float formatter would cost its code.
        let text = format!("{:.1}", (value * self.scale) as f32);
        match text.strip_suffix(".0") {
            Some("-0") => "0".to_owned(),
            Some(whole) => whole.to_owned(),
            None => text,
        }
    }

    /// The value a typed text names, or `None` for a text that is no number
    /// or falls below the floor.
    #[must_use]
    pub fn parse(&self, text: &str) -> Option<f64> {
        let shown = f64::from(text.trim().replace(',', ".").parse::<f32>().ok()?);
        let fits = shown.is_finite() && self.floor.is_none_or(|f| shown >= f);
        fits.then_some(shown / self.scale)
    }

    /// The value one stepper moves `value` to, held at the floor.
    #[must_use]
    pub fn stepped(&self, value: f64, step: Step) -> f64 {
        let by = match step {
            Step::Down => -self.step,
            Step::Up => self.step,
        };
        let shown = value * self.scale + by;
        self.floor.map_or(shown, |f| shown.max(f)) / self.scale
    }
}

/// A length a [`LengthField`] edits: its unit, and the one conversion to and
/// from the number the field computes in.
pub trait FieldLength: Copy + PartialEq + Send + Sync + 'static {
    /// The unit of the field.
    const UNIT: &'static LengthUnit;

    /// The number the field computes in.
    fn to_field(self) -> f64;

    /// The length a number of the field names.
    fn of_field(value: f64) -> Self;
}

/// A margin of a printed page.
impl FieldLength for PrintLength<Mm> {
    const UNIT: &'static LengthUnit = &LengthUnit {
        suffix: "mm",
        step: 1.0,
        floor: Some(0.0),
        scale: 1.0,
    };

    fn to_field(self) -> f64 {
        f64::from(self.get())
    }

    fn of_field(value: f64) -> Self {
        // A margin is stored in `f32`; the field computes in `f64`.
        Self::new(value as f32)
    }
}

/// A stroke width on the board.
impl FieldLength for Px<StoredBoard> {
    const UNIT: &'static LengthUnit = &LengthUnit {
        suffix: "px",
        step: 0.5,
        floor: Some(0.0),
        scale: 1.0,
    };

    fn to_field(self) -> f64 {
        f64::from(self.get())
    }

    fn of_field(value: f64) -> Self {
        // A stroke width is stored in `f32`; the field computes in `f64`.
        Self::new(value as f32)
    }
}

/// A place or a size on the board (`CHROME-247`). A place can be negative:
/// the board has no edge. A size below zero flips its box, which the caller
/// refuses.
impl FieldLength for Px<Board> {
    const UNIT: &'static LengthUnit = &LengthUnit {
        suffix: "px",
        step: 1.0,
        floor: None,
        scale: 1.0,
    };

    fn to_field(self) -> f64 {
        self.get()
    }

    fn of_field(value: f64) -> Self {
        Self::new(value)
    }
}

/// A rotation, which the field shows and reads in degrees (`CHROME-247`).
impl FieldLength for Radians {
    const UNIT: &'static LengthUnit = &LengthUnit {
        suffix: "deg",
        step: 1.0,
        floor: None,
        scale: 180.0 / std::f64::consts::PI,
    };

    fn to_field(self) -> f64 {
        self.get()
    }

    fn of_field(value: f64) -> Self {
        Self::new(value)
    }
}

/// A length of a property: a number, its unit, and two steppers, drawn by
/// the one [`Stepper`](crate::Stepper). The body is a thin adapter to the
/// one copy of the field (`length_core`), so a length type costs its two
/// conversions alone.
#[component]
pub fn LengthField<L: FieldLength>(
    property: &'static Property,
    #[prop(into)] value: Signal<Reading<L>>,
    on_write: Callback<L>,
) -> impl IntoView {
    length_core(
        property,
        L::UNIT,
        Signal::derive(move || value.get().uniform().map(L::to_field)),
        Callback::new(move |v: f64| on_write.run(L::of_field(v))),
    )
}

/// The one copy of a length field, over the number it computes in. `value`
/// is `None` while the selection disagrees.
fn length_core(
    property: &'static Property,
    unit: &'static LengthUnit,
    value: Signal<Option<f64>>,
    on_write: Callback<f64>,
) -> AnyView {
    let step = Callback::new(move |s: Step| {
        if let Some(v) = value.get_untracked() {
            on_write.run(unit.stepped(v, s));
        }
    });
    let typed = Callback::new(move |text: String| {
        if let Some(v) = unit.parse(&text) {
            on_write.run(v);
        }
    });
    view! {
        <crate::Stepper
            label=property_label(property)
            text=Signal::derive(move || value.get().map(|v| unit.text(v)))
            unit=unit.suffix
            on_step=step
            on_text=typed
            reach=Reach::Property(property.id)
        />
    }
    .into_any()
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

/// The views of `choices`, in order, for a control that names a choice by
/// its place.
fn choice_views<T>(choices: &[Choice<T>]) -> Vec<crate::ChoiceView> {
    choices
        .iter()
        .map(|c| crate::ChoiceView {
            label: c.label.to_owned(),
            icon: c.icon,
            reach: None,
            face: None,
            hint: None,
        })
        .collect()
}

/// The place of the choice `value` reads, or `None` for a mixed reading or a
/// value no choice names.
fn place_of<T: PartialEq>(choices: &[Choice<T>], value: &Reading<T>) -> Option<Pos<ChoiceRow>> {
    match value {
        Reading::Uniform(v) => choices
            .iter()
            .position(|c| c.value == *v)
            .map(Pos::from_usize),
        Reading::Mixed => None,
    }
}

/// The value at the place `p` of `values`, written through `on_write`.
fn write_place<T: Copy>(values: &[T], p: Pos<ChoiceRow>, on_write: Callback<T>) {
    if let Some(v) = p.to_index().and_then(|i| values.get(i)) {
        on_write.run(*v);
    }
}

/// A row of exclusive choices of a property: an alignment, an orientation.
/// The [`SegmentBar`](crate::SegmentBar) draws it; this shim maps a value to
/// its place and back, so the bar compiles once for every value type.
#[component]
pub fn Segmented<T: Copy + PartialEq + Send + Sync + 'static>(
    property: &'static Property,
    choices: Vec<Choice<T>>,
    #[prop(into)] value: Signal<Reading<T>>,
    on_write: Callback<T>,
) -> impl IntoView {
    let views = choice_views(&choices);
    let values: Vec<T> = choices.iter().map(|c| c.value).collect();
    let selected = Signal::derive(move || value.with(|v| place_of(&choices, v)));
    let pick = Callback::new(move |p: Pos<ChoiceRow>| write_place(&values, p, on_write));
    view! {
        <span class="ambre-kit-segmented">
            <crate::SegmentBar
                label=property_label(property)
                choices=views
                selected=selected
                on_pick=pick
                reach=Reach::Property(property.id)
            />
            {move || mixed_hint(value.get().is_mixed())}
        </span>
    }
}

/// A list of choices of a property, in the styled [`Select`](crate::Select).
#[component]
pub fn SelectField<T: Copy + PartialEq + Send + Sync + 'static>(
    property: &'static Property,
    choices: Vec<Choice<T>>,
    #[prop(into)] value: Signal<Reading<T>>,
    on_write: Callback<T>,
) -> impl IntoView {
    let views = choice_views(&choices);
    let values: Vec<T> = choices.iter().map(|c| c.value).collect();
    let selected = Signal::derive(move || value.with(|v| place_of(&choices, v)));
    let pick = Callback::new(move |p: Pos<ChoiceRow>| write_place(&values, p, on_write));
    view! {
        <crate::Select
            label=property_label(property)
            choices=views
            selected=selected
            on_pick=pick
            reach=Reach::Property(property.id)
        />
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

/// The native controls under `root` that a primitive replaces: a select
/// element, and a range input outside a [`Slider`](crate::Slider). A test
/// holds the list empty: the panel shows no control in the look of the
/// browser (`CHROME-219`).
pub fn native_controls(root: &web_sys::Element) -> Vec<String> {
    let Ok(list) = root.query_selector_all("select, input[type=\"range\"]") else {
        return Vec::new();
    };
    (0..list.length())
        .filter_map(|i| list.item(i))
        .filter_map(|n| leptos::wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(n).ok())
        .filter(|e| e.closest(".ambre-slider").ok().flatten().is_none())
        .map(|e| {
            e.get_attribute("aria-label")
                .unwrap_or_else(|| e.outer_html())
        })
        .collect()
}

/// The label of a section under `root` that the header of the panel repeats,
/// or `None`. A test holds it `None`: the header names the selection, and a
/// section names a group of its controls (`CHROME-217`).
pub fn repeated_title(root: &web_sys::Element) -> Option<String> {
    let title = root
        .query_selector(".ambre-ctx-title")
        .ok()
        .flatten()?
        .text_content()?;
    let heads = root
        .query_selector_all(".ambre-ctx-section-head > span")
        .ok()?;
    (0..heads.length())
        .filter_map(|i| heads.item(i)?.text_content())
        .find(|label| label.trim().eq_ignore_ascii_case(title.trim()))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_length_shows_a_decimal_only_when_it_has_one() {
        let mm = PrintLength::<Mm>::UNIT;
        assert_eq!(mm.text(20.0), "20");
        assert_eq!(mm.text(PrintLength::<Mm>::new(12.7).to_field()), "12.7");
        assert_eq!(mm.text(0.04), "0");
        assert_eq!(Px::<StoredBoard>::UNIT.text(1.5), "1.5");
    }

    /// `CHROME-247`: a place on the board reads either sign, a stroke width
    /// stays positive, and a rotation reads and shows degrees.
    #[test]
    fn a_place_reads_either_sign_and_a_rotation_reads_degrees() {
        let board = Px::<Board>::UNIT;
        assert_eq!(board.parse("-12.5"), Some(-12.5));
        assert_eq!(board.text(-3.0), "-3");
        assert_eq!(Px::<StoredBoard>::UNIT.parse("-1"), None);
        assert_eq!(
            Px::<StoredBoard>::UNIT.stepped(0.2, Step::Down),
            0.0,
            "held at the floor"
        );
        let deg = Radians::UNIT;
        let quarter = deg.parse("90").expect("a number");
        assert!((quarter - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
        assert_eq!(deg.text(quarter), "90");
        assert_eq!(
            deg.parse("-45").map(|v| deg.text(v)),
            Some("-45".to_owned())
        );
        assert_eq!(deg.text(deg.stepped(quarter, Step::Up)), "91");
        assert_eq!(deg.parse("a"), None);
    }
}
