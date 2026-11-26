// ambre-chrome/src/font.rs
// ============================================================================
// Shared, app-agnostic font-family picker (ambre-spec-shell.md section 2, the
// visual system). It is the shared `Select` of `controls`, so it matches every
// other list of the panel. Each entry shows in its OWN typeface, so the user
// previews the font before picking. A native `<select>` cannot style its
// `<option>`s per font across browsers, hence the styled list.
//
// Promoted here from Trame's cell font popover so Toile's style inspector and
// Trame's cell formatting share the very same control (mirrors `ColorPopover`).
// The control is value-agnostic: it takes the current family as a signal (`None`
// = the app's default) and reports picks through `on_pick`. The caller owns what
// a family means to its renderer (`registered_face` in `ambre-canvas`
// maps the families to registered faces).
// ============================================================================

use ambre_core::FontFamily;
use ambre_core::index::Pos;
use leptos::prelude::*;

use crate::{ChoiceRow, ChoiceView, Select};

/// One entry of the picker: the family it picks and the CSS `font-family`
/// that renders the entry in its own typeface, the live preview.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FamilyChoice {
    /// The family the entry picks. `None` is the "Default" entry: no explicit
    /// family, so the app default applies.
    pub family: Option<FontFamily>,
    /// The CSS `font-family` stack the entry previews in.
    pub css: &'static str,
}

impl FamilyChoice {
    /// The label the entry shows.
    #[must_use]
    pub fn label(&self) -> &'static str {
        self.family
            .as_ref()
            .and_then(FontFamily::picker_label)
            .unwrap_or(DEFAULT_LABEL)
    }
}

/// The label of the entry that names no family.
const DEFAULT_LABEL: &str = "Default";

/// The font families offered by the picker, in the order it lists them.
pub const FONT_FAMILIES: &[FamilyChoice] = &[
    FamilyChoice {
        family: None,
        css: "inherit",
    },
    FamilyChoice {
        family: Some(FontFamily::Sans),
        css: "'Roboto', system-ui, sans-serif",
    },
    FamilyChoice {
        family: Some(FontFamily::Serif),
        css: "Georgia, 'Times New Roman', serif",
    },
    FamilyChoice {
        family: Some(FontFamily::Mono),
        css: "'Roboto Mono', ui-monospace, monospace",
    },
];

/// The place of `current` in [`FONT_FAMILIES`], or `None` for a family the
/// picker does not offer.
fn place_of(current: Option<&FontFamily>) -> Option<Pos<ChoiceRow>> {
    FONT_FAMILIES
        .iter()
        .position(|choice| choice.family.as_ref() == current)
        .map(Pos::from_usize)
}

/// The composed font-family control, drawn as the shared [`Select`]: each
/// entry and the button show their family in its own face. `current` is the
/// active family (`None` = "Default"); `on_pick` receives the chosen family
/// (`None` for the "Default" reset). A family the picker does not offer
/// shows its own name, with no entry selected. `default_label` renames the
/// entry that names no family, where the caller's default has a name.
#[component]
pub fn FontPicker(
    current: Signal<Option<FontFamily>>,
    on_pick: Callback<Option<FontFamily>>,
    #[prop(default = None)] default_label: Option<&'static str>,
) -> impl IntoView {
    let choices = FONT_FAMILIES
        .iter()
        .map(|choice| {
            let label = match (&choice.family, default_label) {
                (None, Some(label)) => label,
                _ => choice.label(),
            };
            ChoiceView::text(label).in_face(choice.css)
        })
        .collect();
    let selected = Signal::derive(move || place_of(current.get().as_ref()));
    let unlisted = Signal::derive(move || {
        current
            .get()
            .map_or_else(String::new, |family| family.label().to_owned())
    });
    let pick = Callback::new(move |p: Pos<ChoiceRow>| {
        if let Some(choice) = p.to_index().and_then(|i| FONT_FAMILIES.get(i)) {
            on_pick.run(choice.family.clone());
        }
    });
    view! {
        <span class="ambre-font">
            <Select
                label=ambre_commands::tr!("Font", "Police")
                choices=choices
                selected=selected
                on_pick=pick
                placeholder=unlisted
            />
        </span>
    }
}
