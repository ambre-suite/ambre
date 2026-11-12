// ambre-app/src/shell/call_bar.rs
// ============================================================================
// The call controls of a Murmure channel header (the Murmure calls spec,
// section 3, the call controls). The page holds the microphone and the peer
// connection. So the view of the controls sits beside the call client of the
// page, with the icons the shell already renders. Murmure decides what the
// header offers (`ambre_murmure::ui::call_controls`), and holds its header.
// ============================================================================

use ambre_core::Bounded;
use ambre_murmure::MurmurePane;
use ambre_murmure::chrome::MurmureRelay;
use ambre_murmure::ui::call_controls::{CallControls, CallPhase, Camera, Microphone};
use ambre_murmure::ui::call_devices::{DeviceKind, Entry, Panel};
use ambre_murmure::ui::call_volumes::{
    Gain, gain_from_slider, slider_of_gain, slider_of_volume, volume_from_slider,
};
use leptos::prelude::*;
use leptos_icons::Icon;

use crate::{call_devices, call_volumes};

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
    /// The members of the spaces, which name each participant of the volumes.
    relay: MurmureRelay,
) -> impl IntoView {
    let controls = move || CallControls::of(phase.get(), pane.get().shown_channel());
    let offers_devices = move || !matches!(controls(), CallControls::None);
    let open = RwSignal::new(ambre_chrome::Openness::Closed);
    let fallback: Signal<Option<ambre_murmure::ui::call_devices::Fallback>> =
        Signal::from(call_devices::fallback());
    let camera = crate::call::camera();
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
                    <CameraButton />
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
                .then(|| view! { <DevicePanel phase=phase relay=relay /> })}
            {move || fallback.get().map(|gone| view! {
                <span class="murmure-call-fallback" role="status">{gone.notice()}</span>
            })}
            {move || phase.get().refusal().map(|refusal| view! {
                <span class="murmure-call-refusal" role="alert">{refusal.notice()}</span>
            })}
            {move || camera.get().refusal().map(|refusal| view! {
                <span class="murmure-call-camera-refusal" role="alert">{refusal.notice()}</span>
            })}
        </span>
    }
}

/// The camera button of a call (the Murmure calls spec, section 3, the camera). It carries
/// `MdiVideo` while a press turns the camera on, and `MdiVideoOff` while a press turns it off.
/// It is a button with an accessible name and its pressed state, disabled while the browser is
/// asked.
#[component]
fn CameraButton() -> impl IntoView {
    let camera = crate::call::camera();
    let (titled, named) = (camera.clone(), camera.clone());
    let (pressed, waits, icon) = (camera.clone(), camera.clone(), camera);
    view! {
        <button
            class="icon murmure-call-camera"
            title=move || titled.get().label()
            aria-label=move || named.get().label()
            aria-pressed=move || ambre_chrome::Pressed::from_bool(pressed.get().sends()).aria()
            prop:disabled=move || waits.get() == Camera::Asking
            on:click=move |_| crate::call::toggle_camera()
        >
            {move || if icon.get().sends() {
                view! { <Icon icon=icondata::MdiVideoOff /> }.into_any()
            } else {
                view! { <Icon icon=icondata::MdiVideo /> }.into_any()
            }}
        </button>
    }
}

/// The device panel (the Murmure calls spec, section 3, the devices). It offers a microphone,
/// and a speaker where the browser routes the audio of a call. Each choice is a
/// labelled select, so it carries an accessible name and takes the focus (`CHROME-022`).
#[component]
fn DevicePanel(
    /// The phase of the call of the page: the volumes show during a call alone.
    phase: Signal<CallPhase>,
    /// The members of the spaces, which name each participant.
    relay: MurmureRelay,
) -> impl IntoView {
    let in_call = move || matches!(phase.get(), CallPhase::InCall(_));
    let listed: Signal<Vec<ambre_murmure::ui::call_devices::Device>> =
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
            {move || in_call().then(|| view! { <CallVolumes relay=relay /> })}
        </div>
    }
}

/// The volumes of a call (the Murmure calls spec, section 3, the volumes). The panel offers a
/// volume for each other participant where the browser sets the volume of an element. It offers
/// the gain of the microphone, its return to the automatic gain and a meter of the level sent.
/// Each control is a native slider or checkbox with an accessible name. The
/// panel shows no volume for the whole call: the system holds it.
#[component]
fn CallVolumes(
    /// The members of the spaces, which name each participant.
    relay: MurmureRelay,
) -> impl IntoView {
    let offers = call_volumes::support().offers_volumes();
    // The signals are read as the call client holds them: a conversion to a `Signal` of each
    // type would add its code to the page.
    let participants = call_volumes::participants();
    let volumes = call_volumes::volumes();
    let gain = call_volumes::gain();
    let level = call_volumes::level();
    // The click that opened the panel is a gesture, so the graph of the meter may run.
    // The graph comes after the render, which may run inside a borrow of the call.
    leptos::task::spawn_local(async { crate::call::watch_level() });
    let reads = call_volumes::start_meter();
    on_cleanup(move || {
        if let Some(reads) = reads {
            reads.clear();
        }
    });
    // A drag of a slider leaves as one value, the last (`CHROME-015`). A gain moved from the
    // automatic one opens the microphone again, so a drag must not open it at each step. The
    // checkbox goes through the same debouncer, so a drag still pending never lands after it.
    let gain_drag = ambre_chrome::Debouncer::new();
    let on_gain = Callback::new(move |text: String| {
        if let Some(factor) = gain_from_slider(&text) {
            later(
                gain_drag,
                ambre_chrome::DRAG_DEBOUNCE,
                Box::new(move || {
                    crate::call::set_gain(Gain::ByHand(factor));
                }),
            );
        }
    });
    let gain_shown = gain.clone();
    let gain_checked = gain.clone();
    let on_automatic = move |ev: leptos::ev::Event| {
        let next = if event_target_checked(&ev) {
            Gain::Automatic
        } else {
            Gain::ByHand(gain.get_untracked().factor())
        };
        later(
            gain_drag,
            std::time::Duration::ZERO,
            Box::new(move || {
                crate::call::set_gain(next);
            }),
        );
    };
    let volume_rows = move || {
        participants
            .get()
            .into_iter()
            .map(|member| {
                let volumes = volumes.clone();
                let name = relay
                    .member_name(member)
                    .map_or_else(|| "a participant".to_owned(), |n| n.as_str().to_owned());
                let drag = ambre_chrome::Debouncer::new();
                let on_volume = Callback::new(move |text: String| {
                    if let Some(volume) = volume_from_slider(&text) {
                        later(
                            drag,
                            ambre_chrome::DRAG_DEBOUNCE,
                            Box::new(move || {
                                crate::call::set_volume(member, volume);
                            }),
                        );
                    }
                });
                view! {
                    <Slider
                        text=name.clone()
                        of=SliderOf::Volume
                        name=format!("Volume of {name}")
                        value=Signal::derive(move || slider_of_volume(volumes.get().of(member)))
                        on_input=on_volume
                    />
                }
            })
            .collect_view()
    };
    view! {
        <div class="murmure-call-volumes" role="group" aria-label="Call volumes">
            {move || offers.then(&volume_rows)}
            <Slider
                text="Microphone gain".to_owned()
                of=SliderOf::Gain
                name="Microphone gain".to_owned()
                value=Signal::derive(move || slider_of_gain(gain_shown.get().factor()))
                on_input=on_gain
            />
            <label class="murmure-call-choice">
                <input
                    type="checkbox"
                    class="murmure-call-automatic-gain"
                    prop:checked=move || gain_checked.get() == Gain::Automatic
                    on:change=on_automatic
                />
                "Automatic gain"
            </label>
            <meter
                class="murmure-call-meter"
                aria-label="Microphone level"
                min="0"
                max="1"
                prop:value=move || f64::from(level.get().get())
            ></meter>
        </div>
    }
}

/// Runs `apply` once the drag that `drag` debounces rests for `delay`. The closure is boxed, so
/// the debouncer compiles one call for every slider of the panel.
fn later(drag: ambre_chrome::Debouncer, delay: std::time::Duration, apply: Box<dyn FnOnce()>) {
    drag.call(delay, apply);
}

/// What a slider of the volumes sets.
#[derive(Clone, Copy)]
enum SliderOf {
    /// The volume of one participant.
    Volume,
    /// The gain of the microphone.
    Gain,
}

impl SliderOf {
    /// The class of the input, which the stylesheet and the belt read.
    const fn class(self) -> &'static str {
        match self {
            Self::Volume => "murmure-call-volume",
            Self::Gain => "murmure-call-gain",
        }
    }

    /// The highest percent of the slider: the full level, or the cap of the gain.
    fn max(self) -> String {
        match self {
            Self::Volume => slider_of_volume(Bounded::<ambre_core::ParticipantVolume>::FULL),
            Self::Gain => slider_of_gain(Bounded::<ambre_core::MicrophoneGain>::MAX),
        }
    }
}

/// One slider of the volumes of a call: a native range input in steps of 5 percent. Its name is
/// `name`, the keyboard reaches it and the arrow keys move it (`CHROME-022`). One component
/// serves each volume and the gain, so the panel compiles one slider.
#[component]
fn Slider(
    /// The visible text of the label that wraps the slider.
    text: String,
    /// What the slider sets, which gives its class and its highest percent.
    of: SliderOf,
    /// The accessible name of the input.
    name: String,
    /// The percent the slider shows.
    value: Signal<String>,
    /// What a move of the slider does with its percent.
    on_input: Callback<String>,
) -> impl IntoView {
    view! {
        <label class="murmure-call-choice">
            {text}
            <input
                type="range"
                class=of.class()
                min="0"
                max=of.max()
                step="5"
                aria-label=name
                prop:value=move || value.get()
                on:input=move |ev| on_input.run(event_target_value(&ev))
            />
        </label>
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
    let chosen: Signal<ambre_murmure::ui::call_devices::DeviceChoice> =
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
