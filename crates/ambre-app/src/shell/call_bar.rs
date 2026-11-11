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
use ambre_murmure::call_devices::{DeviceKind, Entry, Panel};
use leptos::prelude::*;
use leptos_icons::Icon;

use crate::call_devices;

/// The call controls of the header: the call button, or mute and hang up during a call. Each
/// is a button with an icon and an accessible name, and mute carries its pressed state. A
/// refused microphone shows why the call did not start, as an alert. The alert stays until the
/// next call gesture (the Murmure calls spec, section 3, the call controls).
#[component]
pub(super) fn CallBar(
    /// The pane the user is in, which names the shown channel.
    pane: RwSignal<MurmurePane>,
    /// The phase of the call of the page.
    phase: Signal<CallPhase>,
) -> impl IntoView {
    let controls = move || CallControls::of(phase.get(), pane.get().shown_channel());
    let offers_devices = move || !matches!(controls(), CallControls::None);
    let open = RwSignal::new(ambre_chrome::Openness::Closed);
    let fallback: Signal<Option<ambre_murmure::call_devices::Fallback>> =
        Signal::from(call_devices::fallback());
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
            {move || offers_devices().then(|| view! {
                <button
                    class="icon murmure-call-devices"
                    title="Call devices"
                    aria-label="Call devices"
                    aria-expanded=move || open.get().aria()
                    on:click=move |_| {
                        let next = open.get_untracked().toggled();
                        if next == ambre_chrome::Openness::Open {
                            leptos::task::spawn_local(async {
                                call_devices::refresh().await;
                            });
                        }
                        open.set(next);
                    }
                >
                    <Icon icon=icondata::MdiTuneVariant />
                </button>
            })}
            {move || (offers_devices() && open.get() == ambre_chrome::Openness::Open)
                .then(|| view! { <DevicePanel /> })}
            {move || fallback.get().map(|gone| view! {
                <span class="murmure-call-fallback" role="status">{gone.notice()}</span>
            })}
            {move || phase.get().refusal().map(|refusal| view! {
                <span class="murmure-call-refusal" role="alert">{refusal.notice()}</span>
            })}
        </span>
    }
}

/// The device panel (the Murmure calls spec, section 3, the devices). It offers a microphone,
/// and a speaker where the browser routes the audio of a call. Each choice is a
/// labelled select, so it carries an accessible name and takes the focus (`CHROME-022`).
#[component]
fn DevicePanel() -> impl IntoView {
    let listed: Signal<Vec<ambre_murmure::call_devices::Device>> =
        Signal::from(call_devices::listed());
    let panel = Memo::new(move |_| Panel::of(&listed.get(), call_devices::routing()));
    view! {
        <div class="murmure-call-panel" role="group" aria-label="Call devices">
            <DeviceSelect
                kind=DeviceKind::Microphone
                label="Microphone"
                entries=Signal::derive(move || panel.get().microphones)
            />
            {move || panel.get().speakers.map(|speakers| view! {
                <DeviceSelect
                    kind=DeviceKind::Speaker
                    label="Speaker"
                    entries=Signal::derive(move || speakers.clone())
                />
            })}
            {move || panel.get().names.hint().map(|hint| view! {
                <p class="murmure-call-hint">{hint}</p>
            })}
        </div>
    }
}

/// One choice of the device panel. A pick keeps the choice on this browser and moves a call in
/// progress to it (`CALL-078`, `CALL-082`).
#[component]
fn DeviceSelect(
    /// The kind the select chooses.
    kind: DeviceKind,
    /// The visible label of the select, which names it.
    label: &'static str,
    /// The entries, the default one first.
    entries: Signal<Vec<Entry>>,
) -> impl IntoView {
    let chosen: Signal<ambre_murmure::call_devices::DeviceChoice> =
        Signal::from(call_devices::choice(kind));
    let class = match kind {
        DeviceKind::Microphone => "murmure-call-microphone",
        DeviceKind::Speaker => "murmure-call-speaker",
    };
    let on_change = move |ev: leptos::ev::Event| {
        let picked = event_target_value(&ev)
            .parse::<usize>()
            .ok()
            .and_then(|at| entries.get_untracked().get(at).map(Entry::choice));
        let Some(choice) = picked else {
            return;
        };
        call_devices::keep(kind, choice.clone());
        match kind {
            DeviceKind::Microphone => crate::call::switch_microphone(choice),
            DeviceKind::Speaker => crate::call::switch_speaker(choice),
        }
    };
    view! {
        <label class="murmure-call-choice">
            {label}
            <select class=class on:change=on_change>
                {move || {
                    let current = chosen.get();
                    entries
                        .get()
                        .into_iter()
                        .enumerate()
                        .map(|(at, entry)| {
                            let selected = entry.choice() == current;
                            view! {
                                <option value=at.to_string() selected=selected>
                                    {entry.label(kind).to_owned()}
                                </option>
                            }
                        })
                        .collect_view()
                }}
            </select>
        </label>
    }
}
