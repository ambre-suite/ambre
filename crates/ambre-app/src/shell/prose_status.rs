// ambre-app/src/shell/prose_status.rs
// ============================================================================
// The five figures of a Prose document, in the status band every application
// shares (`PROSE-FEAT-040`). The page view publishes them; this reads them.
// ============================================================================

use leptos::prelude::*;

/// The Prose content of the shared status bar slot: its label and the five
/// figures the page view last published.
#[component]
pub(crate) fn ProseFigures(figures: RwSignal<Option<[String; 5]>>) -> impl IntoView {
    view! {
        <ambre_chrome::StatusLabel label="Prose" />
        <span class="statusbar-stats">
            {move || {
                figures.get().map(|figures| {
                    figures
                        .into_iter()
                        .map(|figure| view! { <span class="stat">{figure}</span> })
                        .collect_view()
                })
            }}
        </span>
        <span class="spacer"></span>
    }
}
