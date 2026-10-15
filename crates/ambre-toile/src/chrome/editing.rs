// ambre-toile/src/chrome/editing.rs
// ============================================================================
// The board's editing chrome, mounted only while the board is not presenting
// (`TOILE-UI-083`). A presentation shows the slide alone. The palette, the
// menus, the style surfaces, the comments and the deck panel leave the DOM,
// and come back when it ends.
// ============================================================================

use leptos::prelude::*;

/// Mount `children` while `present` is false. The launcher wraps every
/// editing surface of a board in it, so one signal hides them all.
#[component]
pub fn EditingSurfaces(present: RwSignal<bool>, children: ChildrenFn) -> impl IntoView {
    view! {
        <Show when=move || !present.get() fallback=|| ()>
            {children()}
        </Show>
    }
}
