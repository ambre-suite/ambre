// ambre-murmure/src/chrome.rs
// ============================================================================
// Murmure-SPECIFIC chrome (ambre-spec-architecture.md: per-app chrome lives in
// the app crate, NOT the launcher). Built on the app-agnostic `ambre-chrome`
// toolkit. It maps the pane onto the regions of the shell. It draws the
// channel header the command bar holds (the Murmure layout spec).
//
// Wasm-only, like the rest of the chrome stack (leptos CSR + DOM).
// ============================================================================

use crate::MurmurePane;
use ambre_chrome::ShellRegion;
use leptos::prelude::*;

/// The region of the shell a pane is in. The channel list is the sidebar,
/// a channel the content, and a thread the panel.
pub fn focus_of(pane: MurmurePane) -> ShellRegion {
    match pane {
        MurmurePane::ChannelList => ShellRegion::Sidebar,
        MurmurePane::Channel { .. } => ShellRegion::Content,
        MurmurePane::Thread { .. } => ShellRegion::Panel,
    }
}

/// Whether the thread panel is open: only while the user is in a thread.
pub fn thread_open(pane: MurmurePane) -> bool {
    matches!(pane, MurmurePane::Thread { .. })
}

/// The header of the current channel, in the command bar of the shell
/// (`MURMURE-428`). Its back control goes to the pane before, and touches
/// nothing but the pane (`MURMURE-168`, `MURMURE-169`).
#[component]
pub fn ChannelHeader(
    /// The pane the user is in.
    pane: RwSignal<MurmurePane>,
    /// The name of the current channel, once its engine has read it.
    #[prop(into)]
    name: Signal<Option<String>>,
) -> impl IntoView {
    view! {
        <div class="murmure-channel-header">
            <Show when=move || pane.get() != MurmurePane::ChannelList>
                <button
                    class="icon murmure-back"
                    aria-label="Back"
                    on:click=move |_| pane.update(|p| *p = p.back())
                >
                    "\u{2039}"
                </button>
            </Show>
            <span class="murmure-channel-name">{move || name.get().unwrap_or_default()}</span>
        </div>
    }
}
