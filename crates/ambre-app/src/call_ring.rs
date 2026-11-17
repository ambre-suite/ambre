//! The ring of the page (the Murmure calls spec, section 3, the ring).
//!
//! `ambre_murmure::ui::call_ring` decides each step. This module holds the state of the page.
//! That is the ring the alert reads, the timer of its timeout and the tone. It also holds what
//! the caller reads once a ring ends with no answer. A step asks the page to join, to write a decline, or to
//! hang up, and this module does it.

use std::cell::RefCell;

use ambre_bus::{Authored, CallCue, Embed, Instant};
use ambre_core::id::{Call, Channel, Id, User};
use ambre_core::{Ms, TimeSpan};
use ambre_logic::app::AppCommand;
use ambre_murmure::chrome::MurmureRelay;
use ambre_murmure::ui::call_controls::CallPhase;
use ambre_murmure::ui::call_ring::{
    Conversation, Line, Ring, RingAct, Unanswered, decline_command,
};
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use crate::outbox::ShellOutbox;

thread_local! {
    /// The ring of the page, which the alert and the header read.
    static RING: ArcRwSignal<Ring> = ArcRwSignal::new(Ring::Quiet);
    /// Why the last call this page started ended with no answer, until the next call gesture.
    static UNANSWERED: ArcRwSignal<Option<Unanswered>> = ArcRwSignal::new(None);
    /// The outbox a decline leaves by, and the Murmure chrome that names this user, once the
    /// Murmure mirror installs them.
    static OUTBOX: RefCell<Option<(ShellOutbox, MurmureRelay)>> = const { RefCell::new(None) };
    /// The tone of a ring on this page, while it sounds.
    static TONE: RefCell<Option<web_sys::AudioContext>> = const { RefCell::new(None) };
}

/// The ring of the page, for the alert and the header.
pub(crate) fn ring() -> ArcRwSignal<Ring> {
    RING.with(ArcRwSignal::clone)
}

/// Why the last call of the page rang with no answer, for the header.
pub(crate) fn unanswered() -> ArcRwSignal<Option<Unanswered>> {
    UNANSWERED.with(ArcRwSignal::clone)
}

/// Gives the ring the outbox a decline leaves by, and the chrome that names this user.
pub(crate) fn install(outbox: ShellOutbox, relay: MurmureRelay) {
    OUTBOX.with(|slot| *slot.borrow_mut() = Some((outbox, relay)));
}

/// The clock of the page, on the base of the instants a writer stamps.
fn now() -> Instant<Authored> {
    Instant::<Authored>::new(js_sys::Date::now() as u64)
}

/// Whether the page is in a call or opens one: a page on the line rings for no other call.
fn line() -> Line {
    match crate::call::phase().get_untracked() {
        CallPhase::Idle | CallPhase::Refused(_) => Line::Free,
        CallPhase::Asking | CallPhase::InCall(_) => Line::Busy,
    }
}

/// The page heard the cue `cue` of `channel` (the Murmure calls spec, the ring). The sidebar tells whether the
/// channel is a direct message, and the name of the other member.
pub(crate) fn heard(relay: &MurmureRelay, channel: Id<Channel>, cue: CallCue) {
    // A page that shows no ring alert rings nothing: the Murmure page alone installs the ring.
    if OUTBOX.with(|slot| slot.borrow().is_none()) {
        return;
    }
    let (conversation, caller) = relay.sidebar.with_untracked(|sidebar| {
        let conversation = if sidebar.is_direct(channel) {
            Conversation::Direct
        } else {
            Conversation::Channel
        };
        (conversation, sidebar.name_of(channel))
    });
    let current = ring().get_untracked();
    let step = current.heard(channel, cue, conversation, line(), caller, now());
    apply(step);
}

/// This page started `call` in `channel` from the call button (the Murmure calls spec, the ring). A call of a
/// direct message waits for the other member.
pub(crate) fn started(relay: &MurmureRelay, call: Id<Call>, channel: Id<Channel>) {
    let conversation = if relay.sidebar.with_untracked(|s| s.is_direct(channel)) {
        Conversation::Direct
    } else {
        Conversation::Channel
    };
    let next = ring()
        .get_untracked()
        .started(call, channel, conversation, now());
    apply((next, None));
}

/// The call of the page admitted `member`: another member answered (the Murmure calls spec, the ring).
pub(crate) fn admitted(member: Id<User>) {
    let me = OUTBOX.with(|slot| slot.borrow().as_ref().and_then(|(_, relay)| relay.me()));
    let next = ring().get_untracked().admitted(member, me);
    apply((next, None));
}

/// The call of the page ended: it waits for nobody.
pub(crate) fn hung_up() {
    let next = ring().get_untracked().hung_up();
    apply((next, None));
}

/// The member pressed accept: the page joins the call, the microphone first (the Murmure calls spec, the ring).
pub(crate) fn accept() {
    let step = ring().get_untracked().accept();
    apply(step);
}

/// The member pressed decline: the page writes the decline (the Murmure calls spec, the ring).
pub(crate) fn decline() {
    let step = ring().get_untracked().decline();
    apply(step);
}

/// A new call gesture clears what the header said of the last ring.
pub(crate) fn clear_notice() {
    UNANSWERED.with(|unanswered| unanswered.set(None));
}

/// Takes the ring `next`, does `act`, then sounds the tone and arms the timer the ring needs.
fn apply((next, act): (Ring, Option<RingAct>)) {
    let rang = ring().get_untracked();
    if next != rang {
        RING.with(|ring| ring.set(next.clone()));
    }
    match act {
        None => {}
        Some(RingAct::Join { call, channel }) => crate::call::join(call, channel),
        Some(RingAct::Decline { call, channel }) => send_decline(call, channel),
        Some(RingAct::HangUp(why)) => {
            UNANSWERED.with(|unanswered| unanswered.set(Some(why)));
            crate::call::leave_call();
        }
    }
    if next.rings_here() != rang.rings_here() {
        if next.rings_here() {
            sound(next.deadline());
        } else {
            silence();
        }
    }
    if next.deadline() != rang.deadline() {
        arm(next.deadline());
    }
}

/// Writes the decline of `call` into its call message. The Logic thread opens the channel
/// when it is not open (the Murmure calls spec, the ring).
fn send_decline(call: Id<Call>, channel: Id<Channel>) {
    let command = decline_command(call, channel, now());
    OUTBOX.with(|slot| {
        if let Some((outbox, _)) = slot.borrow().as_ref() {
            outbox.send(command.map(AppCommand::embed));
        }
    });
}

/// The time a timer waits past the deadline of a ring. A browser timer may fire a little before
/// its delay, and a tick before the deadline ends nothing.
const MARGIN: TimeSpan<Ms> = TimeSpan::new(100);

/// Arms a timer at `deadline`. A timer of an earlier ring may still fire: the tick reads the
/// ring of that instant, so it ends nothing before its deadline. A hidden tab may fire a timer
/// late, and the ring then ends late (the Murmure calls spec, the ring).
fn arm(deadline: Option<Instant<Authored>>) {
    let (Some(window), Some(deadline)) = (web_sys::window(), deadline) else {
        return;
    };
    let wait = deadline.plus(MARGIN).since(now());
    let fire = Closure::once_into_js(move || {
        let step = ring().get_untracked().tick(now());
        apply(step);
    });
    let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
        fire.unchecked_ref(),
        wait.to_timer_i32(),
    );
}

/// Sounds the tone of a ring until `deadline`: two tones together, one second on and two off,
/// as a telephone rings. The tone is synthesized, so the page ships no sound file. A browser
/// that holds the audio until a gesture keeps the tone silent, and the alert still shows.
fn sound(deadline: Option<Instant<Authored>>) {
    silence();
    let Some(deadline) = deadline else {
        return;
    };
    let Ok(context) = web_sys::AudioContext::new() else {
        return;
    };
    let Ok(gain) = context.create_gain() else {
        return;
    };
    let start = context.current_time();
    let span = deadline.since(now()).get() as f64 / 1_000.0;
    let level = gain.gain();
    level.set_value(0.0);
    // One ring each three seconds, for the span of the ring.
    let mut at = start;
    while at < start + span {
        let _ = level.set_value_at_time(0.2, at);
        let _ = level.set_value_at_time(0.0, at + 1.0);
        at += 3.0;
    }
    let _ = gain.connect_with_audio_node(&context.destination());
    for pitch in [440.0, 480.0] {
        if let Ok(tone) = context.create_oscillator() {
            tone.frequency().set_value(pitch);
            let _ = tone.connect_with_audio_node(&gain);
            let _ = tone.start();
        }
    }
    // A context made outside a gesture may start suspended: ask once, and stay silent on a
    // refusal.
    let _ = context.resume();
    TONE.with(|slot| *slot.borrow_mut() = Some(context));
}

/// Stops the tone, if one sounds.
fn silence() {
    if let Some(context) = TONE.with(|slot| slot.borrow_mut().take()) {
        let _ = context.close();
    }
}
