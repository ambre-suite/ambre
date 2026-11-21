// ambre-chrome/src/gallery.rs
// ============================================================================
// The gallery of the visual system (`ambre-spec-testing.md` section 2, the
// computed-style snapshot). A test page shows each control primitive in each
// of its states, and the frame of the context panel.
//
// The browser tier opens it and forces the pointer and focus states through
// the DevTools Protocol. It records the computed style of every specimen in a
// text snapshot. A specimen names itself on `data-specimen`, and the browser
// tier reads the controls under it. The page writes nothing and
// reaches no thread: it mounts alone, behind the `gallery` feature, which no
// release build turns on.
//
// The gallery holds the primitives, never the content of a section. The
// content of each application changes story by story; the look of a
// primitive changes only when the visual system does.
// ============================================================================

use ambre_commands::{KeyBinding, KeyChar};
use ambre_core::index::Pos;
use ambre_core::{Availability, Bounded, Rgba, UnitFraction};
use leptos::prelude::*;

use crate::controls::{
    ChoiceRow, ChoiceView, ColorSwatch, ControlSize, IconButton, Readout, SegmentBar, Select,
    Slider, Stepper,
};
use crate::format::Pressed;
use crate::kit::Step;
use crate::{ContextPanel, DockSide, Field, Fold, PanelPrefs, PanelSection, PanelWidth};

/// The section key of the frame specimen.
const FRAME_SECTION: ambre_core::StaticKey<ambre_core::PanelSections> =
    ambre_core::StaticKey::new("gallery.section");

/// The command the action-list specimens run: one line of text and an icon.
static ACTION: ambre_commands::Command = ambre_commands::Command {
    id: ambre_core::StaticKey::<ambre_core::Commands>::new("gallery.action"),
    title: |_: &ambre_commands::Locale| ambre_core::Validated::new("Insert row above"),
    icon: None,
    keybindings: &[],
    category: ambre_core::StaticKey::<ambre_core::Categories>::new("gallery"),
    home: ambre_commands::Home::ContextPanel,
};

/// One specimen: a name the snapshot reads, and the control under it.
#[component]
fn Specimen(name: &'static str, children: Children) -> impl IntoView {
    view! {
        <div class="ambre-gallery-specimen" data-specimen=name>
            <span class="ambre-gallery-name">{name}</span>
            {children()}
        </div>
    }
}

/// A callback that does nothing: a specimen reports to no one.
fn inert<T: 'static>() -> Callback<T> {
    Callback::new(|_| {})
}

/// The two choices every list specimen shows.
fn choices() -> Vec<ChoiceView> {
    vec![ChoiceView::text("A4"), ChoiceView::text("Letter")]
}

/// The gallery page: every primitive in each of its states, then the frame
/// of the context panel.
#[component]
pub fn Gallery() -> impl IntoView {
    let bold = || KeyBinding::primary(KeyChar::B);
    let no_step: Callback<Step> = inert();
    let no_text: Callback<String> = inert();
    let no_press: Callback<()> = inert();
    let no_pick: Callback<Pos<ChoiceRow>> = inert();
    let no_place: Callback<Bounded<UnitFraction>> = inert();
    let no_command: Callback<ambre_core::StaticKey<ambre_core::Commands>> = inert();
    let first = Signal::stored(Some(Pos::<ChoiceRow>::from_usize(0)));
    let none = Signal::stored(None::<Pos<ChoiceRow>>);
    let half = Signal::stored(Bounded::<UnitFraction>::new(0.5));
    let ink = Signal::stored(Some(Rgba::from_bits(0xb453_09ff)));
    let section = PanelSection::new(
        FRAME_SECTION,
        "Section",
        icondata::MdiFormatText,
        move || {
            view! {
                <Field label="Label">
                    <Stepper
                        label="Length"
                        text=Signal::stored(Some("20.0".to_owned()))
                        unit="mm"
                        on_step=no_step
                        on_text=no_text
                    />
                </Field>
            }
            .into_any()
        },
    );
    let prefs = PanelPrefs::new(DockSide::Right, PanelWidth::DEFAULT, Fold::Open);
    view! {
        <main class="ambre-gallery">
            <Specimen name="icon-button">
                <IconButton icon=icondata::BsTypeBold label="Bold" chord=Some(bold()) on_press=no_press />
            </Specimen>
            <Specimen name="icon-button.pressed">
                <IconButton
                    icon=icondata::BsTypeBold
                    label="Bold"
                    pressed=Some(Signal::stored(Pressed::On))
                    on_press=no_press
                />
            </Specimen>
            <Specimen name="icon-button.disabled">
                <IconButton
                    icon=icondata::BsTypeBold
                    label="Bold"
                    availability=Signal::stored(Availability::Disabled)
                    on_press=no_press
                />
            </Specimen>
            <Specimen name="icon-button.small">
                <IconButton icon=icondata::BsTypeBold label="Bold" size=ControlSize::Small on_press=no_press />
            </Specimen>
            <Specimen name="icon-button.large">
                <IconButton icon=icondata::BsTypeBold label="Bold" size=ControlSize::Large on_press=no_press />
            </Specimen>
            <Specimen name="select">
                <Select label="Paper" choices=choices() selected=first on_pick=no_pick />
            </Specimen>
            <Specimen name="select.mixed">
                <Select label="Paper" choices=choices() selected=none on_pick=no_pick />
            </Specimen>
            <Specimen name="segments">
                <SegmentBar label="Orientation" choices=choices() selected=first on_pick=no_pick />
            </Specimen>
            <Specimen name="stepper">
                <Stepper
                    label="Margin"
                    text=Signal::stored(Some("20.0".to_owned()))
                    unit="mm"
                    on_step=no_step
                    on_text=no_text
                />
            </Specimen>
            <Specimen name="swatch">
                <ColorSwatch label="Accent" colour=ink on_press=no_press />
            </Specimen>
            <Specimen name="swatch.pressed">
                <ColorSwatch label="Accent" colour=ink pressed=Signal::stored(Pressed::On) on_press=no_press />
            </Specimen>
            <Specimen name="swatch.empty">
                <ColorSwatch label="No colour" colour=Signal::stored(None) on_press=no_press />
            </Specimen>
            <Specimen name="slider">
                <Slider label="Opacity" value=half on_write=no_place readout=Readout::Percent />
            </Specimen>
            <Specimen name="action">
                <crate::CommandButton
                    command=&ACTION
                    icon=icondata::MdiTableRowPlusBefore
                    on_command=no_command
                    labelled=crate::Labelled::Text
                />
            </Specimen>
            <Specimen name="action.pressed">
                <crate::CommandButton
                    command=&ACTION
                    icon=icondata::MdiTableRowPlusBefore
                    on_command=no_command
                    pressed=Signal::stored(Pressed::On)
                    labelled=crate::Labelled::Text
                />
            </Specimen>
            <Specimen name="frame">
                <div class="ambre-body ambre-gallery-frame">
                    <ContextPanel
                        prefs=prefs
                        title="Document"
                        sections=Signal::stored(vec![section])
                    />
                </div>
            </Specimen>
        </main>
    }
}
