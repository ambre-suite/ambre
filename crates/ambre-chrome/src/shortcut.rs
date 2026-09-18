// ambre-chrome/src/shortcut.rs
// ============================================================================
// Keybinding presentation. The canonical chord string lives on `KeyBinding`'s
// Display (ambre-commands, host-testable). This module only wraps it as a DOM
// element, so menus, tooltips and the palette render shortcuts consistently.
// ============================================================================

use ambre_commands::KeyBinding;
use leptos::prelude::*;

/// Renders a keyboard chord as a `<kbd>` element, for example `Ctrl+Shift+B`.
/// Styling is left to the theme via the `ambre-kbd` class.
#[component]
pub fn Kbd(binding: KeyBinding) -> impl IntoView {
    view! { <kbd class="ambre-kbd">{binding.to_string()}</kbd> }
}
