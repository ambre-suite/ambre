// ambre-app/src/shell/call_bar.rs
// ============================================================================
// The call controls of a Murmure channel header (the Murmure calls spec,
// section 3, the call controls). The page holds the microphone and the peer
// connection. So the view of the controls sits beside the call client of the
// page, with the icons the shell already renders. Murmure decides what the
// header offers (`ambre_murmure::call_controls`), and holds its header.
// ============================================================================

use ambre_murmure::MurmurePane;
use ambre_murmure::call_controls::{CallControls, CallPhase, Microphone};
use leptos::prelude::*;
use leptos_icons::Icon;

/// The call controls of the header: the call button, or mute and hang up during a call
/// (`MURMURE-599`, `MURMURE-602`). Each is a button with an icon and an accessible name, and
/// mute carries its pressed state (`MURMURE-604`). A refused microphone shows why the call did
/// not start, as an alert, until the next call gesture (`MURMURE-601`).
#[component]
pub(super) fn CallBar(
    /// The pane the user is in, which names the shown channel.
    pane: RwSignal<MurmurePane>,
    /// The phase of the call of the page.
    phase: Signal<CallPhase>,
) -> impl IntoView {
    let controls = move || CallControls::of(phase.get(), pane.get().shown_channel());
    view! {
        <span class="murmure-call">
            {move || match controls() {
                CallControls::None => ().into_any(),
                CallControls::Start(channel) => view! {
                    <button
                        class="icon murmure-call-start"
                        title="Call"
                        aria-label="Start a call"
                        on:click=move |_| {
                            crate::call::start(channel);
                        }
                    >
                        <Icon icon=icondata::MdiPhone />
                    </button>
                }
                .into_any(),
                CallControls::During(microphone) => view! {
                    <button
                        class="icon murmure-call-mute"
                        title="Mute"
                        aria-label="Mute"
                        aria-pressed=ambre_chrome::Pressed::from_bool(microphone == Microphone::Muted).aria()
                        on:click=move |_| crate::call::toggle_mute()
                    >
                        <Icon icon=icondata::MdiMicrophoneOff />
                    </button>
                    <button
                        class="icon murmure-call-hang-up"
                        title="Hang up"
                        aria-label="Hang up"
                        on:click=move |_| crate::call::leave_call()
                    >
                        <Icon icon=icondata::MdiPhoneHangup />
                    </button>
                }
                .into_any(),
            }}
            {move || phase.get().refusal().map(|refusal| view! {
                <span class="murmure-call-refusal" role="alert">{refusal.notice()}</span>
            })}
        </span>
    }
}
