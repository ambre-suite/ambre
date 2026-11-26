// ambre-app/src/shell/call_bar.rs
// ============================================================================
// The call controls of a Murmure channel header (the Murmure calls spec,
// section 3, the call controls). The page holds the microphone and the peer
// connection. So the view of the controls sits beside the call client of the
// page, with the icons the shell already renders. Murmure decides what the
// header offers (`ambre_murmure::ui::call_controls`), and holds its header.
// ============================================================================

use ambre_chrome::{Availability, CallVerb, CommandTip, ControlSize, IconButton, IconLook};
use ambre_core::{Bounded, Validated};
use ambre_murmure::MurmurePane;
use ambre_murmure::chrome::MurmureRelay;
use ambre_murmure::ui::call_controls::{
    CallControls, CallPhase, Camera, CameraPress, Microphone, Share, SharePress,
};
use ambre_murmure::ui::call_devices::{DeviceKind, Entry, Panel};
use ambre_murmure::ui::call_ring::{Ring, ring_title};
use ambre_murmure::ui::call_videos::{GridPages, Turn};
use ambre_murmure::ui::call_volumes::{
    Gain, gain_from_slider, slider_of_gain, slider_of_volume, volume_from_slider,
};
use ambre_murmure::ui::commands::{call_command, pin_command};
use leptos::prelude::*;
use leptos_icons::Icon;

use crate::{call_devices, call_volumes};

/// The call controls of the header: the call button, or mute and hang up in the channel of a
/// call. Another channel offers none during a call, and the call bubble holds them. Each
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
    let from = crate::call::where_from();
    let controls = Signal::derive(move || {
        let call = from.get().map(|(channel, _)| channel);
        CallControls::of(phase.get(), pane.get().shown_channel(), call)
    });
    let controls = move || controls.get();
    let offers_devices = move || !matches!(controls(), CallControls::None);
    let open = RwSignal::from(crate::call_stage::devices());
    let fallback: Signal<Option<ambre_murmure::ui::call_devices::Fallback>> =
        Signal::from(call_devices::fallback());
    let camera = crate::call::camera();
    let share = crate::call::share();
    let (ring, unanswered) = (crate::call_ring::ring(), crate::call_ring::unanswered());
    view! {
        <span class="murmure-call">
            {move || match controls() {
                CallControls::None => ().into_any(),
                CallControls::Start(channel) => view! {
                    <CallIcon
                        verb=Signal::stored(CallVerb::Start(channel))
                        icon=icondata::MdiPhone
                        relay=relay
                        class="murmure-call-start"
                    />
                }
                .into_any(),
                CallControls::During(microphone) => view! {
                    <RosterButton relay=relay />
                    <PageButtons relay=relay />
                    <FillButton relay=relay />
                    <CameraButton relay=relay />
                    <ShareButton relay=relay />
                    <MuteButton microphone=Signal::stored(microphone) relay=relay />
                    <HangUpButton relay=relay />
                }
                .into_any(),
            }}
            {move || offers_devices().then(|| view! {
                <CallIcon
                    verb=Signal::stored(CallVerb::Devices)
                    icon=icondata::MdiTuneVariant
                    relay=relay
                    class="murmure-call-devices"
                    expanded=Some(Signal::from(open))
                />
            })}
            {move || (offers_devices() && open.get() == ambre_chrome::Openness::Open)
                .then(|| view! { <DevicePanel phase=phase relay=relay /> })}
            {move || fallback.get().map(|gone| view! {
                <span class="murmure-call-fallback" role="status">{gone.notice()}</span>
            })}
            {move || matches!(ring.get(), Ring::Outgoing(_)).then(|| view! {
                <span class="murmure-call-ringing" role="status">"Calling: waiting for an answer"</span>
            })}
            {move || unanswered.get().map(|why| view! {
                <span class="murmure-call-unanswered" role="status">{why.notice()}</span>
            })}
            {move || phase.get().refusal().map(|refusal| view! {
                <span class="murmure-call-refusal" role="alert">{refusal.notice()}</span>
            })}
            {move || camera.get().refusal().map(|refusal| view! {
                <span class="murmure-call-camera-refusal" role="alert">{refusal.notice()}</span>
            })}
            {move || {
                let state = share.get();
                // A refusal is an alert; a take-over is news the member did not ask for.
                let role = if matches!(state, Share::Refused(_)) { "alert" } else { "status" };
                state.notice().map(|notice| view! {
                    <span class="murmure-call-share-notice" role=role>{notice}</span>
                })
            }}
        </span>
    }
}

/// Performs `verb` on the call of the page. The press of its icon runs it, and so does its
/// command from the palette (the Murmure calls spec, section 3, the call controls). A verb that
/// sets a state already set does nothing.
pub(super) fn run(verb: CallVerb, relay: MurmureRelay) {
    use ambre_murmure::ui::call_videos::Fill;
    match verb {
        CallVerb::Start(channel) => {
            // A call of a direct message rings the other member, and the header waits for the
            // answer (the Murmure calls spec, the ring).
            if let Some(call) = crate::call::start(channel) {
                crate::call_ring::started(&relay, call, channel);
            }
        }
        CallVerb::Mute | CallVerb::Unmute => {
            let target = if verb == CallVerb::Mute {
                Microphone::Muted
            } else {
                Microphone::Live
            };
            let phase = crate::call::phase().get_untracked();
            if matches!(phase, CallPhase::InCall(now) if now != target) {
                crate::call::toggle_mute();
            }
        }
        CallVerb::CameraOn | CallVerb::CameraOff => {
            let want = if verb == CallVerb::CameraOn {
                CameraPress::Open
            } else {
                CameraPress::Stop
            };
            if crate::call::camera().get_untracked().press() == want {
                crate::call::toggle_camera();
            }
        }
        CallVerb::ShareScreen | CallVerb::StopSharing => {
            let want = if verb == CallVerb::ShareScreen {
                SharePress::Open
            } else {
                SharePress::Stop
            };
            if crate::call::share().get_untracked().press() == want {
                crate::call::toggle_share();
            }
        }
        CallVerb::Fill | CallVerb::Shrink => {
            let from = if verb == CallVerb::Fill {
                Fill::Band
            } else {
                Fill::Region
            };
            if crate::call_stage::fill().get_untracked() == from {
                crate::call_stage::toggle_fill();
            }
        }
        CallVerb::Participants => {
            let roster = crate::call_stage::roster();
            roster.set(roster.get_untracked().toggled());
        }
        CallVerb::Devices => {
            let open = crate::call_stage::devices();
            let next = open.get_untracked().toggled();
            if next == ambre_chrome::Openness::Open {
                leptos::task::spawn_local(async {
                    call_devices::refresh().await;
                });
            }
            open.set(next);
        }
        CallVerb::PreviousPage => crate::call_stage::turn(Turn::Previous),
        CallVerb::NextPage => crate::call_stage::turn(Turn::Next),
        CallVerb::BackToCall => {
            if let Some((channel, _)) = crate::call::where_from().get_untracked() {
                relay.pane.set(MurmurePane::channel(channel));
            }
        }
        CallVerb::HangUp => crate::call::leave_call(),
        CallVerb::Accept => crate::call_ring::accept(),
        CallVerb::Decline => crate::call_ring::decline(),
    }
}

/// One icon of the call: its tooltip names the command of `verb`, and its press performs `verb`
/// (the Murmure calls spec). A toggle passes the verb of its state, so its tooltip names
/// what a press does now (`CHROME-256`). One component serves every icon, so the page compiles
/// one view of an icon of the call.
#[component]
fn CallIcon(
    /// The verb a press performs, which names the command of the tooltip.
    verb: Signal<CallVerb>,
    /// The icon the button shows.
    #[prop(into)]
    icon: Signal<icondata::Icon>,
    /// The members of the spaces, which a start rings and the way back shows.
    relay: MurmureRelay,
    /// The class the stylesheet and the tests read.
    class: &'static str,
    /// The pressed state of a toggle.
    #[prop(optional_no_strip)]
    pressed: Option<Signal<ambre_chrome::Pressed>>,
    /// Whether the list or the panel the icon opens is open.
    #[prop(optional_no_strip)]
    expanded: Option<Signal<ambre_chrome::Openness>>,
    /// Whether a press acts: a button waits while the browser is asked.
    #[prop(optional, into)]
    availability: Option<Signal<Availability>>,
) -> impl IntoView {
    view! {
        <IconButton
            icon=icon
            tip=Signal::derive(move || CommandTip::of(call_command(verb.get())))
            look=IconLook::Ghost
            class=class
            pressed=pressed
            expanded=expanded
            availability=availability.unwrap_or_else(|| Signal::stored(Availability::Enabled))
            on_press=Callback::new(move |()| run(verb.get_untracked(), relay))
        />
    }
    .into_any()
}

/// The mute button of a call, in the header and in the call float (the Murmure calls spec,
/// section 3, the call controls). It carries its pressed state.
#[component]
fn MuteButton(
    /// The state of the microphone in the call.
    microphone: Signal<Microphone>,
    /// The members of the spaces, which `run` takes.
    relay: MurmureRelay,
) -> impl IntoView {
    let muted = move || microphone.get() == Microphone::Muted;
    view! {
        <CallIcon
            verb=Signal::derive(move || if muted() { CallVerb::Unmute } else { CallVerb::Mute })
            icon=icondata::MdiMicrophoneOff
            relay=relay
            class="murmure-call-mute"
            pressed=Some(Signal::derive(move || ambre_chrome::Pressed::from_bool(muted())))
        />
    }
}

/// The hang up button of a call, in the header and in the call float (the Murmure calls spec,
/// section 3, the call controls). The stylesheet gives it the colour of an error.
#[component]
fn HangUpButton(
    /// The members of the spaces, which `run` takes.
    relay: MurmureRelay,
) -> impl IntoView {
    view! {
        <CallIcon
            verb=Signal::stored(CallVerb::HangUp)
            icon=icondata::MdiPhoneHangup
            relay=relay
            class="murmure-call-hang-up"
        />
    }
}

/// The list of the participants of a call (the Murmure calls spec, section 3, the video region).
/// A button with an accessible name opens it and reports whether it is open. So each participant
/// the page of the grid shows no tile for reaches the page (`CHROME-029`). Each one carries a
/// pin button, with an accessible name and its pressed state. The pin is titled by its command,
/// and the item it sits in names the participant.
#[component]
fn RosterButton(
    /// The members of the spaces, which name each participant.
    relay: MurmureRelay,
) -> impl IntoView {
    let roster = crate::call_stage::roster();
    let participants = call_volumes::participants();
    let pinned = crate::call_stage::pinned();
    let (expanded, shown) = (roster.clone(), roster);
    let expanded = Signal::derive(move || expanded.get());
    let names = move || {
        participants
            .get()
            .into_iter()
            .map(|member| {
                let name = relay.member_name(member).map_or_else(
                    || ambre_chrome::commands::tr!("A participant", "Un participant").to_owned(),
                    |n| n.as_str().to_owned(),
                );
                let (titled, pressed, icon) = (pinned.clone(), pinned.clone(), pinned.clone());
                // A pin acts on one participant, so it runs no verb of the palette.
                let tip = Signal::derive(move || CommandTip::of(pin_command(member, titled.get())));
                let icon = Signal::derive(move || {
                    if icon.get() == Some(member) {
                        icondata::MdiPinOff
                    } else {
                        icondata::MdiPin
                    }
                });
                let pressed = Signal::derive(move || {
                    ambre_chrome::Pressed::from_bool(pressed.get() == Some(member))
                });
                view! {
                    <li>
                        <span class="murmure-call-roster-name">{name}</span>
                        <IconButton
                            icon=icon
                            tip=tip
                            size=ControlSize::Small
                            look=IconLook::Ghost
                            class="murmure-call-pin"
                            pressed=Some(pressed)
                            on_press=Callback::new(move |()| crate::call_stage::toggle_pin(member))
                        />
                    </li>
                }
            })
            .collect_view()
    };
    view! {
        <CallIcon
            verb=Signal::stored(CallVerb::Participants)
            icon=icondata::MdiAccountMultiple
            relay=relay
            class="murmure-call-roster"
            expanded=Some(expanded)
        />
        {move || (shown.get() == ambre_chrome::Openness::Open).then(|| view! {
            <ul class="murmure-call-roster-list" aria-label="Participants of the call">
                <li><span class="murmure-call-roster-name">"You"</span></li>
                {names.clone()}
            </ul>
        })}
    }
}

/// The page buttons of a call, while its grid holds more than one page (the Murmure calls spec,
/// the video region). Each is a button with an accessible name, which takes the focus. A button
/// is disabled at the end it turns to. The two turn the page as the arrows of the grid do.
#[component]
fn PageButtons(
    /// The members of the spaces, which `run` takes.
    relay: MurmureRelay,
) -> impl IntoView {
    let participants = call_volumes::participants();
    let paging = Memo::new(move |_| crate::call_stage::paging(&participants.get()));
    let button = move |way: Turn, class: &'static str| {
        let icon = match way {
            Turn::Previous => icondata::MdiChevronLeft,
            Turn::Next => icondata::MdiChevronRight,
        };
        let availability = Signal::derive(move || {
            let paging = paging.get();
            if paging.turned(way) == paging.page {
                Availability::Disabled
            } else {
                Availability::Enabled
            }
        });
        view! {
            <CallIcon
                verb=Signal::stored(way.verb())
                icon=icon
                relay=relay
                class=class
                availability=availability
            />
        }
    };
    move || {
        let paging = paging.get();
        (paging.pages > ambre_core::index::Len::<GridPages>::ONE).then(|| {
            view! {
                <span class="murmure-call-pages" role="group" aria-label="Pages of the call">
                    {button(Turn::Previous, "murmure-call-page-previous")}
                    <span class="murmure-call-page-number">
                        {format!("{} / {}", paging.page.number(), paging.pages)}
                    </span>
                    {button(Turn::Next, "murmure-call-page-next")}
                </span>
            }
        })
    }
}

/// The fill button of a call (the Murmure calls spec, section 3, the video region). It widens
/// the call to the whole channel region, and a second press brings back the band. It carries
/// `MdiFullscreen` while a press widens the call, and `MdiFullscreenExit` while a press shrinks
/// it. It is a button with an accessible name and its pressed state, which takes the focus.
#[component]
fn FillButton(
    /// The members of the spaces, which `run` takes.
    relay: MurmureRelay,
) -> impl IntoView {
    let fill = crate::call_stage::fill();
    let (named, pressed, icon) = (fill.clone(), fill.clone(), fill);
    let widened = move |fill: ambre_murmure::ui::call_videos::Fill| {
        fill == ambre_murmure::ui::call_videos::Fill::Region
    };
    view! {
        <CallIcon
            verb=Signal::derive(move || named.get().verb())
            icon=Signal::derive(move || {
                if widened(icon.get()) { icondata::MdiFullscreenExit } else { icondata::MdiFullscreen }
            })
            relay=relay
            class="murmure-call-fill"
            pressed=Some(Signal::derive(move || ambre_chrome::Pressed::from_bool(widened(pressed.get()))))
        />
    }
}

/// The camera button of a call (the Murmure calls spec, section 3, the camera). It carries
/// `MdiVideo` while a press turns the camera on, and `MdiVideoOff` while a press turns it off.
/// It is a button with an accessible name and its pressed state, disabled while the browser is
/// asked.
#[component]
fn CameraButton(
    /// The members of the spaces, which `run` takes.
    relay: MurmureRelay,
) -> impl IntoView {
    let camera = crate::call::camera();
    let named = camera.clone();
    let (pressed, waits, icon) = (camera.clone(), camera.clone(), camera);
    view! {
        <CallIcon
            verb=Signal::derive(move || named.get().verb())
            icon=Signal::derive(move || {
                if icon.get().sends() { icondata::MdiVideoOff } else { icondata::MdiVideo }
            })
            relay=relay
            class="murmure-call-camera"
            pressed=Some(Signal::derive(move || ambre_chrome::Pressed::from_bool(pressed.get().sends())))
            availability=Signal::derive(move || {
                if waits.get() == Camera::Asking { Availability::Disabled } else { Availability::Enabled }
            })
        />
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
                label=ambre_chrome::commands::tr!("Microphone", "Micro")
                entries=Signal::derive(move || panel.get().microphones)
            />
            {move || panel.get().speakers.map(|speakers| view! {
                <DeviceSelect
                    kind=DeviceKind::Speaker
                    label=ambre_chrome::commands::tr!("Speaker", "Haut-parleur")
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
    // The slider holds the debouncer in its own handler, so the gate reads the guard on the
    // control it guards.
    let gain_drag = ambre_chrome::Debouncer::new();
    let on_gain = Callback::new(move |text: String| {
        if let Some(factor) = gain_from_slider(&text) {
            crate::call::set_gain(Gain::ByHand(factor));
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
                        crate::call::set_volume(member, volume);
                    }
                });
                view! {
                    <Slider
                        text=name.clone()
                        of=SliderOf::Volume
                        name=ambre_chrome::commands::trf!("Volume of {name}", "Volume de {name}")
                        value=Signal::derive(move || slider_of_volume(volumes.get().of(member)))
                        drag=drag
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
                text=ambre_chrome::commands::tr!("Microphone gain", "Gain du micro").to_owned()
                of=SliderOf::Gain
                name=ambre_chrome::commands::tr!("Microphone gain", "Gain du micro").to_owned()
                value=Signal::derive(move || slider_of_gain(gain_shown.get().factor()))
                drag=gain_drag
                on_input=on_gain
            />
            <label class="murmure-call-choice">
                <input
                    type="checkbox"
                    class="murmure-call-automatic-gain"
                    prop:checked=move || gain_checked.get() == Gain::Automatic
                    on:change=on_automatic
                />
                {ambre_chrome::commands::tr!("Automatic gain", "Gain automatique")}
            </label>
            <meter
                class="murmure-call-meter"
                aria-label=ambre_chrome::commands::tr!("Microphone level", "Niveau du micro")
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
    /// The debouncer of the slider: a drag leaves as one percent, the last (`CHROME-015`).
    drag: ambre_chrome::Debouncer,
    /// What a move of the slider does with its percent, once the drag rests.
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
                on:input=move |ev| {
                    let text = event_target_value(&ev);
                    later(drag, ambre_chrome::DRAG_DEBOUNCE, Box::new(move || on_input.run(text)));
                }
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

/// The share button of a call (the Murmure calls spec, section 3, the screen). It carries
/// `MdiMonitorShare` while a press shares the screen, and `MdiMonitorOff` while a press stops the
/// share. It is a button with an accessible name and its pressed state, disabled while the
/// browser is asked.
#[component]
fn ShareButton(
    /// The members of the spaces, which `run` takes.
    relay: MurmureRelay,
) -> impl IntoView {
    let share = crate::call::share();
    let named = share.clone();
    let (pressed, waits, icon) = (share.clone(), share.clone(), share);
    view! {
        <CallIcon
            verb=Signal::derive(move || named.get().verb())
            icon=Signal::derive(move || {
                if icon.get().sends() { icondata::MdiMonitorOff } else { icondata::MdiMonitorShare }
            })
            relay=relay
            class="murmure-call-share"
            pressed=Some(Signal::derive(move || ambre_chrome::Pressed::from_bool(pressed.get().sends())))
            availability=Signal::derive(move || {
                if waits.get() == Share::Asking { Availability::Disabled } else { Availability::Enabled }
            })
        />
    }
}

/// The alert of a call that rings on this page (the Murmure calls spec, section 3, the ring).
/// Its title names the caller as an alert, so a screen reader reads it when it shows. Accept
/// and decline are buttons with an accessible name, each in the order of the keyboard. The
/// alert takes no focus: a member who types keeps the key in the field.
#[component]
pub(super) fn RingAlert(
    /// The members of the spaces, which `run` takes.
    relay: MurmureRelay,
) -> impl IntoView {
    let ring = crate::call_ring::ring();
    view! {
        {move || match ring.get() {
            Ring::Incoming(incoming) => {
                let title = ring_title(incoming.caller.as_ref());
                view! {
                    <div class="murmure-ring" role="group" aria-label="Incoming call">
                        <p class="murmure-ring-title" role="alert">{title}</p>
                        <CallIcon
                            verb=Signal::stored(CallVerb::Accept)
                            icon=icondata::MdiPhone
                            relay=relay
                            class="murmure-ring-accept"
                        />
                        <CallIcon
                            verb=Signal::stored(CallVerb::Decline)
                            icon=icondata::MdiPhoneHangup
                            relay=relay
                            class="murmure-ring-decline"
                        />
                    </div>
                }
                .into_any()
            }
            Ring::Quiet | Ring::Outgoing(_) => ().into_any(),
        }}
    }
}

/// The call away from its channel (the Murmure calls spec, section 3, away from the channel of
/// the call). While Murmure shows another channel, a thread or the list, the page shows a bubble.
/// Under the phone width the stylesheet turns it into a pill. Each
/// names the channel of the call and the time in it, with the faces of the participants.
///
/// The bubble offers mute, the camera, the share, the way back and hang up, and the pill keeps
/// mute and hang up. A drag of the bubble moves it, and its release rests it in the nearest
/// corner. Nothing here writes into a channel or changes the choice of videos.
///
/// The bubble floats over the canvas, so it publishes its box into `cover` each time it rests,
/// and clears it when it leaves (`CHROME-171`).
///
/// A press on the faces of the bubble opens its mini video. Over a picture, the box of the mini
/// video paints nothing. The render runtime draws the picture on the canvas below it
/// (`CALL-015`). So the bubble also publishes the box of its mini video at each move and each
/// resize (the Murmure calls spec, section 3).
#[component]
pub(super) fn CallFloat(
    /// The pane the user is in, which names the shown channel.
    pane: RwSignal<MurmurePane>,
    /// The phase of the call of the page.
    phase: Signal<CallPhase>,
    /// The sidebar names the channel, and the spaces name each participant.
    relay: MurmureRelay,
    /// Where the bubble publishes the box it covers.
    cover: RwSignal<ambre_client_core::cover::Cover>,
) -> impl IntoView {
    use crate::call_stage::MiniShows;
    use ambre_client_core::cover::Surface;
    use ambre_murmure::ui::call_float::{Corner, Float, Mini, MiniBox, clock};
    let from = crate::call::where_from();
    let from_shown = from.clone();
    let shown = Signal::derive(move || {
        let call = from_shown.get().map(|(channel, _)| channel);
        Float::of(phase.get(), call, pane.get().shown_channel()) == Float::Shown
    });
    let corner = RwSignal::new(Corner::default());
    let node = NodeRef::<leptos::html::Div>::new();
    ambre_chrome::cover::watch_fixed(node, Surface::CallBubble, cover);
    // A bubble that rests in another corner keeps its size, so the observer
    // misses the move. The bubble measures again on the next frame, once the
    // page laid it out. A bubble that leaves covers nothing.
    Effect::new(move |_| {
        corner.track();
        if shown.get() {
            request_animation_frame(move || {
                ambre_chrome::cover::measure_fixed(node, Surface::CallBubble, cover);
            });
        } else {
            ambre_chrome::cover::clear(cover, Surface::CallBubble);
        }
    });
    // The pointer offset of a drag in progress, and the place of the bubble during it.
    let dragged = RwSignal::new(None::<(f64, f64)>);
    let tick = RwSignal::new(crate::host::now());
    // The clock ticks while the float is mounted, and stops with it.
    if let Ok(handle) = set_interval_with_handle(
        move || tick.set(crate::host::now()),
        std::time::Duration::from_secs(1),
    ) {
        on_cleanup(move || handle.clear());
    }
    let from_clock = from.clone();
    let elapsed = Signal::derive(move || {
        from_clock
            .get()
            .map_or_else(String::new, |(_, at)| clock(tick.get() - at))
    });
    let from_name = from.clone();
    let channel_name = Signal::derive(move || {
        from_name
            .get()
            .and_then(|(channel, _)| relay.sidebar.with(|s| s.name_of(channel)))
            .map_or_else(
                || ambre_chrome::commands::tr!("Call", "Appel").to_owned(),
                Validated::into_string,
            )
    });
    let participants = crate::call_volumes::participants();
    let faces = Signal::derive(move || {
        participants
            .get()
            .into_iter()
            .take(4)
            .map(|member| {
                let name = relay.member_name(member);
                ambre_murmure::ui::call_videos::initials(name.as_ref())
            })
            .collect::<Vec<String>>()
    });
    let back = Callback::new(move |()| {
        if let Some((channel, _)) = from.get_untracked() {
            pane.set(MurmurePane::channel(channel));
        }
    });
    let release = move |ev: web_sys::PointerEvent| {
        if dragged.get_untracked().is_none() {
            return;
        }
        dragged.set(None);
        let Some(window) = web_sys::window() else {
            return;
        };
        let size = |v: Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue>| {
            v.ok().and_then(|v| v.as_f64()).unwrap_or_default()
        };
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a coordinate of the window fits an f32"
        )]
        let area = ambre_core::Size::new(
            ambre_core::Px::<ambre_core::View>::new(size(window.inner_width()) as f32),
            ambre_core::Px::<ambre_core::View>::new(size(window.inner_height()) as f32),
        );
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a coordinate of the window fits an f32"
        )]
        let at = (
            ambre_core::Px::<ambre_core::View>::new(f64::from(ev.client_x()) as f32),
            ambre_core::Px::<ambre_core::View>::new(f64::from(ev.client_y()) as f32),
        );
        corner.set(Corner::nearest(at, area));
    };
    let microphone = Signal::derive(move || match phase.get() {
        CallPhase::InCall(microphone) => microphone,
        _ => Microphone::Live,
    });
    let mini = RwSignal::from(crate::call_stage::mini());
    let mini_shows = RwSignal::from(crate::call_stage::mini_shows());
    let mini_box = NodeRef::<leptos::html::Div>::new();
    // The page box of the mini video, after the layout of the frame.
    let measure = move || {
        let open = shown.get_untracked() && mini.get_untracked() == Mini::Open;
        let placed = open
            .then(|| mini_box.get_untracked())
            .flatten()
            .map(|el| el.get_bounding_client_rect())
            .and_then(|r| {
                MiniBox::on_page(ambre_draw::Rect::new(
                    r.left(),
                    r.top(),
                    r.right(),
                    r.bottom(),
                ))
            });
        crate::call_stage::publish_mini_box(placed);
    };
    // A change of place, of size or of the mini video measures again once the layout ran. The
    // clock measures each second too, so a late font that moves the box is caught.
    Effect::new(move |_| {
        let _ = (
            shown.get(),
            corner.get(),
            dragged.get(),
            mini.get(),
            mini_shows.get(),
            tick.get(),
        );
        request_animation_frame(measure);
    });
    let on_resize = window_event_listener(leptos::ev::resize, move |_| {
        request_animation_frame(measure);
    });
    on_cleanup(move || {
        on_resize.remove();
        crate::call_stage::publish_mini_box(None);
    });
    view! {
        <Show when=move || shown.get()>
            <div
                node_ref=node
                class="murmure-call-float"
                role="region"
                aria-label=ambre_chrome::commands::tr!("Call in progress", "Appel en cours")
                data-mini=move || match (mini.get(), mini_shows.get()) {
                    (Mini::Folded, _) => None,
                    (Mini::Open, MiniShows::Faces) => Some("faces"),
                    (Mini::Open, MiniShows::Video) => Some("video"),
                }
                data-corner=move || corner.get().word()
                data-dragging=move || dragged.get().is_some().then_some("true")
                style:left=move || dragged.get().map(|(x, _)| format!("{x}px"))
                style:top=move || dragged.get().map(|(_, y)| format!("{y}px"))
                style:right=move || dragged.get().map(|_| "auto")
                style:bottom=move || dragged.get().map(|_| "auto")
                on:pointermove=move |ev: web_sys::PointerEvent| {
                    if dragged.get_untracked().is_some() {
                        let (x, y) = (f64::from(ev.client_x()), f64::from(ev.client_y()));
                        dragged.set(Some((x - 120.0, y - 18.0)));
                    }
                }
                on:pointerup=release
            >
                <button
                    r#type="button"
                    class="murmure-float-grip"
                    aria-label=move || ambre_chrome::commands::trf!("Back to the call of {}", "Revenir \u{e0} l'appel de {}", channel_name.get())
                    on:click=move |_| back.run(())
                    on:pointerdown=move |ev: web_sys::PointerEvent| {
                        if ev.pointer_type() == "mouse" {
                            let (x, y) = (f64::from(ev.client_x()), f64::from(ev.client_y()));
                            dragged.set(Some((x - 120.0, y - 18.0)));
                        }
                    }
                >
                    <Icon icon=icondata::MdiPhone />
                    <span class="murmure-float-channel">{channel_name}</span>
                    <span class="murmure-float-clock">{elapsed}</span>
                </button>
                <button
                    r#type="button"
                    class="murmure-float-faces"
                    aria-label=move || match mini.get() {
                        Mini::Folded => ambre_chrome::commands::tr!("Show the video of the call", "Afficher la vid\u{e9}o de l'appel"),
                        Mini::Open => ambre_chrome::commands::tr!("Hide the video of the call", "Masquer la vid\u{e9}o de l'appel"),
                    }
                    aria-pressed=move || {
                        ambre_chrome::Pressed::from_bool(mini.get() == Mini::Open).aria()
                    }
                    on:click=move |_| mini.update(|m| *m = m.toggled())
                >
                    {move || {
                        faces
                            .get()
                            .into_iter()
                            .map(|initials| view! {
                                <span class="murmure-float-face" aria-hidden="true">{initials}</span>
                            })
                            .collect_view()
                    }}
                </button>
                <Show when=move || mini.get() == Mini::Open>
                    // The box the render runtime draws the picture through. It holds a word
                    // alone while no video arrives, and nothing over a picture.
                    <div class="murmure-float-mini" node_ref=mini_box>
                        {move || (mini_shows.get() == MiniShows::Faces).then_some(view! {
                            <span class="murmure-float-mini-none">"No video"</span>
                        })}
                    </div>
                </Show>
                <div class="murmure-float-controls">
                    <MuteButton microphone=microphone relay=relay />
                    <span class="murmure-float-wide">
                        <CameraButton relay=relay />
                        <ShareButton relay=relay />
                        <CallIcon
                            verb=Signal::stored(CallVerb::BackToCall)
                            icon=icondata::MdiArrowULeftTop
                            relay=relay
                            class="murmure-float-back"
                        />
                    </span>
                    <HangUpButton relay=relay />
                </div>
            </div>
        </Show>
    }
}
