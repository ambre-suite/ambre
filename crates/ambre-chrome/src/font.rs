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
// a family label means to its renderer (`registered_family_name` in
// `ambre-canvas` maps the labels to registered faces).
// ============================================================================

use leptos::html;
use leptos::prelude::*;
use leptos_icons::Icon;
use wasm_bindgen::JsCast;

/// The font families offered by the picker. Each pairs an abstract family label
/// with the CSS `font-family` that renders the entry in its own typeface. The
/// label is what the renderer resolves to a face, and the CSS is the live
/// preview. "Default" maps to `None` (no explicit family -> the app default).
pub const FONT_FAMILIES: &[(&str, &str)] = &[
    ("Default", "inherit"),
    ("Sans", "'Roboto', system-ui, sans-serif"),
    ("Serif", "Georgia, 'Times New Roman', serif"),
    ("Monospace", "'Roboto Mono', ui-monospace, monospace"),
];

/// The composed font-family control. `current` is the active family label
/// (`None` = "Default"); `on_pick` receives the chosen label (`None` for the
/// "Default" reset). The popover dismisses on any pointer-down outside it.
#[component]
pub fn FontPicker(
    current: Signal<Option<String>>,
    on_pick: Callback<Option<String>>,
) -> impl IntoView {
    let open = RwSignal::new(false);
    // The wrapper node, so a global pointer-down can tell clicks inside the menu
    // apart from clicks elsewhere (another control, the canvas).
    let root_ref = NodeRef::<html::Span>::new();
    let handle = window_event_listener(leptos::ev::pointerdown, move |ev| {
        if !open.get_untracked() {
            return;
        }
        let inside = root_ref
            .get_untracked()
            .zip(ev.target())
            .and_then(|(root, target)| target.dyn_into::<web_sys::Node>().ok().map(|n| (root, n)))
            .is_some_and(|(root, node)| root.contains(Some(&node)));
        if !inside {
            open.set(false);
        }
    });
    on_cleanup(move || handle.remove());

    // The active family label ("Default" when unset).
    let current_label = move || current.get().unwrap_or_else(|| "Default".to_owned());
    // CSS font-family for the active family, so the trigger previews it too.
    let current_css = move || {
        let label = current_label();
        FONT_FAMILIES
            .iter()
            .find(|(name, _)| *name == label)
            .map_or("inherit", |(_, css)| css)
    };

    view! {
        <span class="ambre-font" node_ref=root_ref>
            <button
                class="font-trigger"
                aria-label="Font"
                aria-haspopup="true"
                aria-expanded=move || open.get().then_some("true")
                style:font-family=current_css
                // Keep DOM focus on the canvas text pipe (the FormatBar hazard).
                on:mousedown=move |ev| ev.prevent_default()
                on:click=move |_| open.update(|o| *o = !*o)
            >
                <span class="label">{current_label}</span>
                <Icon icon=icondata::BiDownArrowSolid />
            </button>
            <Show when=move || open.get() fallback=|| ()>
                <div class="ambre-font-popover">
                    {FONT_FAMILIES
                        .iter()
                        .map(|&(name, css)| {
                            view! {
                                <button
                                    class="font-item"
                                    style:font-family=css
                                    on:mousedown=move |ev| ev.prevent_default()
                                    on:click=move |_| {
                                        // "Default" clears the explicit family (None).
                                        let family = (name != "Default").then(|| name.to_owned());
                                        on_pick.run(family);
                                        open.set(false);
                                    }
                                >
                                    <span class="check">
                                        <Show
                                            when=move || current_label() == name
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
