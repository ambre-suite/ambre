// ambre-app/src/shell/prose_status.rs
// ============================================================================
// The five figures of a Prose document, in the status band every application
// shares (`PROSE-FEAT-040`). The page view publishes them; this reads them.
// ============================================================================

use ambre_core::{Doc, Scale, View};
use ambre_prose::ui::StatusFigures;
use leptos::prelude::*;

/// The Prose content of the shared status bar slot: its label and the five
/// figures the page view last published. The page zoom closes the band
/// (`CHROME-206`).
#[component]
pub(crate) fn ProseFigures(
    figures: RwSignal<Option<StatusFigures>>,
    zoom: RwSignal<Scale<Doc, View>>,
    signals: crate::host::ProseSignals,
) -> impl IntoView {
    view! {
        <ambre_chrome::StatusLabel label="Prose" />
        <span class="statusbar-stats">
            {move || {
                figures.get().map(|figures| {
                    figures
                        .labels()
                        .map(|figure| view! { <span class="stat">{figure.to_owned()}</span> })
                        .into_iter()
                        .collect_view()
                })
            }}
        </span>
        // Suggestion mode stays in view while the panel is folded
        // (`PROSE-UI-275`).
        <ambre_prose::chrome::ProseModeChip signals=signals />
        <span class="spacer"></span>
        <ambre_chrome::ZoomChip
            zoom=zoom
            range=ambre_prose::view::zoom::ZOOM_RANGE
            title="Page zoom"
        />
    }
}
