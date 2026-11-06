// ambre-app/src/outbox.rs
// ============================================================================
// The launcher's one writer on the UI to Logic queue (`BUS-011`, `BUS-035`).
//
// Several launcher parts send commands of their own: the shell, the landing,
// the route poll, the per-frame relay, the local store and the vault. None of
// them owns a frame, so none can drive an outbox of its own. They share this
// one, over one `ambre_bus::Outbox`. Their commands leave in call order, and a
// full queue holds a lossless command rather than dropping it.
//
// The per-frame relay flushes it. That relay runs on every animation frame,
// and on the timer of a hidden page (`CANVAS-075`). A held command thus leaves
// whether the page shows or not.
//
// The handle crosses the shell router's `Send` boundary, so it holds the
// outbox in a Leptos `StoredValue` with local storage. Every call runs on the
// page's main thread. No lock guards it: the Logic thread never reaches it
// (`ARCH-020`).
//
// The raw sender of the queue lives here and nowhere else in the launcher.
// The views take their ports from it, and nothing else of the launcher can
// name the queue (`BUS-035`).
// ============================================================================

use ambre_bus::conduit::Sender;
use ambre_bus::{
    ChannelPlane, Embed, LossyOutcome, Outbox, PlaneOf, Port, SendOutcome, is_lossy_class,
};
use ambre_logic::app::{AppCommand, LogicCommand};
use ambre_murmure::message::Murmure;
use leptos::prelude::{LocalStorage, StoredValue, UpdateValue, WithValue};

/// Whether the outbox refused a command the relay has not told the user of
/// yet (`BUS-018`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Refusal {
    #[default]
    Told,
    Untold,
}

/// The outbox and the refusal the relay still owes the user.
struct Hold {
    outbox: Outbox<AppCommand, Murmure>,
    refusal: Refusal,
}

/// The send path of every launcher sender that has no frame of its own.
///
/// One per page. Cheap to clone, and every clone writes into the same outbox,
/// so the order across all of them is the order of the calls.
#[derive(Clone)]
pub(crate) struct ShellOutbox {
    hold: StoredValue<Hold, LocalStorage>,
    /// The queue's raw end, kept for the views' ports alone.
    to_logic: Sender<LogicCommand>,
}

impl ShellOutbox {
    /// The outbox over `to_logic`, the one sender of the UI to Logic queue
    /// the launcher keeps.
    pub(crate) fn new(to_logic: Sender<LogicCommand>) -> Self {
        Self::with_outbox(Outbox::new(Port::direct(to_logic.clone())), to_logic)
    }

    /// The outbox with an explicit hold bound, so a test reaches the refusal
    /// without sixty-four commands.
    #[cfg(test)]
    pub(crate) fn with_capacity(to_logic: Sender<LogicCommand>, capacity: usize) -> Self {
        Self::with_outbox(
            Outbox::with_capacity(Port::direct(to_logic.clone()), capacity),
            to_logic,
        )
    }

    fn with_outbox(outbox: Outbox<AppCommand, Murmure>, to_logic: Sender<LogicCommand>) -> Self {
        Self {
            hold: StoredValue::new_local(Hold {
                outbox,
                refusal: Refusal::Told,
            }),
            to_logic,
        }
    }

    /// A port over the same queue, for one view's own outbox (`BUS-033`).
    pub(crate) fn port<A: 'static, P: ChannelPlane + 'static>(&self) -> Port<A, P>
    where
        AppCommand: Embed<A>,
        Murmure: PlaneOf<P>,
    {
        Port::embedded(self.to_logic.clone())
    }

    /// Send one lossless command, or hold it for the relay to flush.
    ///
    /// A command the outbox refuses at its bound is lost, and the relay tells
    /// the user on its next turn. The caller reflects the command in its own
    /// state only when the outcome is accepted (`BUS-018`).
    pub(crate) fn send(&self, cmd: LogicCommand) -> SendOutcome {
        self.hold
            .try_update_value(|hold| {
                let outcome = hold.outbox.send(cmd);
                if outcome == SendOutcome::Refused {
                    hold.refusal = Refusal::Untold;
                }
                outcome
            })
            .unwrap_or(SendOutcome::Disconnected)
    }

    /// Send one command on the class its application names for it.
    ///
    /// A lossy command takes the lossy send and drops on a full queue, since
    /// its sender offers it again (`BUS-024`). Every other command takes
    /// [`send`](Self::send).
    pub(crate) fn offer(&self, cmd: LogicCommand) {
        if is_lossy_class(&cmd) {
            let outcome = self
                .hold
                .try_with_value(|hold| hold.outbox.send_lossy(cmd))
                .unwrap_or(LossyOutcome::Disconnected);
            if outcome == LossyOutcome::Dropped {
                tracing::debug!("a lossy launcher command dropped, its sender offers it again");
            }
        } else {
            self.send(cmd);
        }
    }

    /// Offer the held commands to the queue, oldest first. The relay calls it
    /// once per turn, before anything newer leaves.
    pub(crate) fn flush(&self) -> SendOutcome {
        self.hold
            .try_update_value(|hold| hold.outbox.flush())
            .unwrap_or(SendOutcome::Disconnected)
    }

    /// Whether the outbox holds a command the Logic thread has not taken.
    /// The relay raises the busy notice while it is `true` (`BUS-016`).
    pub(crate) fn is_busy(&self) -> bool {
        self.hold
            .try_with_value(|hold| hold.outbox.is_busy())
            .unwrap_or(false)
    }

    /// Whether a command was refused since the last call. The relay tells the
    /// user once per refusal episode, rather than writing a log line nobody
    /// reads (`BUS-018`).
    pub(crate) fn take_refusal(&self) -> bool {
        self.hold
            .try_update_value(|hold| std::mem::take(&mut hold.refusal) == Refusal::Untold)
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use ambre_bus::conduit::{self, ChannelSlots, Receiver};
    use ambre_core::Bounded;
    use ambre_core::id::{Document, Id};
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    /// One control command, told apart by the document it names.
    fn close(n: u8) -> LogicCommand {
        LogicCommand::CloseDocument {
            document_id: Id::<Document>::from_bytes([n; 16]),
        }
    }

    /// The document byte of each command on `rx`, in queue order.
    fn drained(rx: &Receiver<LogicCommand>) -> Vec<u8> {
        rx.try_iter()
            .map(|cmd| match cmd {
                LogicCommand::CloseDocument { document_id } => document_id.as_bytes()[0],
                _ => 0,
            })
            .collect()
    }

    /// Two launcher senders on one full queue: what each sent is held, not
    /// dropped, and the relay's flush hands it over in call order.
    #[wasm_bindgen_test]
    fn two_senders_on_a_full_queue_hold_their_commands_in_call_order() {
        let (tx, rx) = conduit::local(Bounded::<ChannelSlots>::of::<1>());
        let shell = ShellOutbox::new(tx);
        let relay = shell.clone();
        assert_eq!(shell.send(close(1)), SendOutcome::Sent);
        assert_eq!(relay.send(close(2)), SendOutcome::Held);
        assert_eq!(shell.send(close(3)), SendOutcome::Held);
        assert!(relay.is_busy());
        assert_eq!(drained(&rx), [1]);
        assert_eq!(relay.flush(), SendOutcome::Held);
        assert_eq!(drained(&rx), [2]);
        assert_eq!(shell.flush(), SendOutcome::Sent);
        assert_eq!(drained(&rx), [3]);
        assert!(!shell.is_busy());
    }

    /// Past the hold bound the command is refused, and the refusal waits for
    /// the relay to tell the user once.
    #[wasm_bindgen_test]
    fn a_refusal_past_the_bound_is_kept_for_the_relay_to_tell() {
        let (tx, _rx) = conduit::local(Bounded::<ChannelSlots>::of::<1>());
        let shell = ShellOutbox::with_capacity(tx, 1);
        shell.send(close(1));
        assert_eq!(shell.send(close(2)), SendOutcome::Held);
        assert!(!shell.take_refusal());
        assert_eq!(shell.send(close(3)), SendOutcome::Refused);
        assert!(shell.take_refusal());
        assert!(!shell.take_refusal());
    }
}
