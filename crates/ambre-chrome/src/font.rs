// ambre-chrome/src/font.rs
// ============================================================================
// Shared, app-agnostic font-family picker (ambre-spec-chrome.md). A single
// toolbar trigger (the current family name + caret) toggles a context-menu
// popover listing the families. Each is rendered in its OWN typeface, so the
// user previews the font before picking. A `<select>` cannot reliably style its
// `<option>`s per-font across browsers, hence this custom popover.
//
// Promoted here from Trame's cell font popover so Toile's style inspector and
// Trame's cell formatting share the very same control (mirrors `ColorPopover`).
// The control is value-agnostic: it takes the current family as a signal (`None`
// = the app's default) and reports picks through `on_pick`. The caller owns what
// a family means to its renderer (`registered_family_name` in `ambre-canvas`
// maps the families to registered faces).
// ============================================================================

use ambre_core::FontFamily;
use leptos::html;
use leptos::prelude::*;
use leptos_icons::Icon;
use wasm_bindgen::JsCast;

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

/// The label the trigger shows for `current`: the entry's label, or the
/// family's own label for a family the picker does not offer.
fn shown_label(current: Option<&FontFamily>) -> String {
    match current {
        None => DEFAULT_LABEL.to_owned(),
        Some(family) => family.picker_label().unwrap_or(family.label()).to_owned(),
    }
}

/// The composed font-family control. `current` is the active family (`None`
/// = "Default"); `on_pick` receives the chosen family (`None` for the
/// "Default" reset). The popover dismisses on any pointer-down outside it.
#[component]
pub fn FontPicker(
    current: Signal<Option<FontFamily>>,
    on_pick: Callback<Option<FontFamily>>,
) -> impl IntoView {
    let open = RwSignal::new(crate::Openness::Closed);
    // The wrapper node, so a global pointer-down can tell clicks inside the menu
    // apart from clicks elsewhere (another control, the canvas).
    let root_ref = NodeRef::<html::Span>::new();
    let handle = window_event_listener(leptos::ev::pointerdown, move |ev| {
        if open.get_untracked() == crate::Openness::Closed {
            return;
        }
        let inside = root_ref
            .get_untracked()
            .zip(ev.target())
            .and_then(|(root, target)| target.dyn_into::<web_sys::Node>().ok().map(|n| (root, n)))
            .is_some_and(|(root, node)| root.contains(Some(&node)));
        if !inside {
            open.set(crate::Openness::Closed);
        }
    });
    on_cleanup(move || handle.remove());

    // The active family label ("Default" when unset).
    let current_label = move || shown_label(current.get().as_ref());
    // CSS font-family for the active family, so the trigger previews it too.
    let current_css = move || {
        let family = current.get();
        FONT_FAMILIES
            .iter()
            .find(|choice| choice.family == family)
            .map_or("inherit", |choice| choice.css)
    };

    view! {
        <span class="ambre-font" node_ref=root_ref>
            <button
                class="font-trigger"
                aria-label="Font"
                aria-haspopup="true"
                aria-expanded=move || open.get().aria()
                style:font-family=current_css
                // Keep DOM focus on the canvas text pipe (the FormatBar hazard).
                on:mousedown=move |ev| ev.prevent_default()
                on:click=move |_| open.update(|o| *o = o.toggled())
            >
                <span class="label">{current_label}</span>
                <Icon icon=icondata::BiDownArrowSolid />
            </button>
            <Show when=move || open.get() == crate::Openness::Open fallback=|| ()>
                <div class="ambre-font-popover">
                    {FONT_FAMILIES
                        .iter()
                        .map(|choice| {
                            let name = choice.label();
                            let css = choice.css;
                            let family = choice.family.clone();
                            let picked = choice.family.clone();
                            view! {
                                <button
                                    class="font-item"
                                    style:font-family=css
                                    on:mousedown=move |ev| ev.prevent_default()
                                    on:click=move |_| {
                                        // "Default" clears the explicit family (None).
                                        on_pick.run(picked.clone());
                                        open.set(crate::Openness::Closed);
                                    }
                                >
                                    <span class="check">
                                        <Show
                                            when=move || current.get() == family
                                            fallback=|| ()
                                        >
                                            <Icon icon=icondata::BsCheck />
                                        </Show>
                                    </span>
                                    <span class="label">{name}</span>
                                </button>
                            }
                        })
                        .collect_view()}
                </div>
            </Show>
        </span>
    }
}
