// ambre-bus/src/lossy.rs
// ============================================================================
// The send discipline of the lossy class (`BUS-023`, `BUS-024`).
//
// A lossy command asks the Logic thread for something the screen already shows:
// a viewport slice, a hit test, a set of aggregates. A full queue drops it,
// and the next frame offers it again. The application names each such command
// through `Mutation::traffic`. Any other command carries intent given once, so
// the lossy class refuses it and the caller owes the outbox (`BUS-011`).
//
// The caller reads the outcome. A dropped command it does not re-offer leaves
// the screen ahead of the document, which is the state `BUS-018` bans.
// ============================================================================

use crate::port::Delivery;
use crate::{ChannelPlane, LogicCommand, Mutation, Port, Traffic};

/// What one lossy send did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LossyOutcome {
    /// The command reached the queue.
    Sent,
    /// The queue was full. Nothing holds the command, so the caller re-offers
    /// it on the next frame or undoes what it drew for it (`BUS-024`).
    Dropped,
    /// The command changes the document, and the lossy class carries no such
    /// command (`BUS-023`). Nothing was sent: the caller owes it the outbox.
    Refused,
    /// The Logic thread is gone. This queue carries nothing again.
    Disconnected,
}

impl LossyOutcome {
    /// Whether the command reached the Logic thread.
    ///
    /// The caller reflects a command in its own state only when this is `true`.
    pub fn sent(self) -> bool {
        matches!(self, Self::Sent)
    }
}

/// Whether the lossy class may carry `cmd`.
///
/// The class is one document or channel command that its application names
/// lossy and that changes nothing. Every other command belongs to the
/// lossless path: a mutation, a command its application leaves lossless, and
/// every command of the service and control planes.
pub fn is_lossy_class<A: Mutation, P: ChannelPlane>(cmd: &LogicCommand<A, P>) -> bool {
    match cmd {
        LogicCommand::App { command, .. } => lossy(command),
        LogicCommand::Channel { command, .. } => lossy(command),
        _ => false,
    }
}

/// Whether one content command is lossy and changes nothing. A mutation
/// named lossy by mistake still takes the lossless path (`BUS-023`).
fn lossy<C: Mutation>(command: &C) -> bool {
    command.traffic() == Traffic::Lossy && !command.is_mutating()
}

/// Send one lossy command, and say what became of it.
///
/// Never blocks: the UI thread runs the frame. A full queue drops the
/// command rather than holding it, because the next frame carries the same
/// request over a state that has moved on. A view reaches it through
/// [`Outbox::send_lossy`](crate::Outbox::send_lossy).
pub(crate) fn send_lossy<A: Mutation, P: ChannelPlane>(
    to_logic: &Port<A, P>,
    cmd: LogicCommand<A, P>,
) -> LossyOutcome {
    if !is_lossy_class(&cmd) {
        return LossyOutcome::Refused;
    }
    match to_logic.try_send(cmd) {
        Delivery::Sent => LossyOutcome::Sent,
        Delivery::Full(_) => LossyOutcome::Dropped,
        Delivery::Disconnected => LossyOutcome::Disconnected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_core::id::Id;

    /// A document command of no application. It holds one request that
    /// changes nothing, and the three mutation shapes the gate must refuse.
    /// It also holds a selection that stays lossless, and a mutation named
    /// lossy by mistake.
    #[derive(Debug)]
    enum Probe {
        Viewport,
        InsertColumn,
        MoveColumn,
        ResizeColumn,
        Select,
        MislabelledDelete,
    }

    impl Mutation for Probe {
        fn is_mutating(&self) -> bool {
            !matches!(self, Self::Viewport | Self::Select)
        }

        fn traffic(&self) -> Traffic {
            match self {
                Self::Viewport | Self::MislabelledDelete => Traffic::Lossy,
                Self::InsertColumn | Self::MoveColumn | Self::ResizeColumn | Self::Select => {
                    Traffic::Lossless
                }
            }
        }
    }

    fn app(command: Probe) -> LogicCommand<Probe> {
        LogicCommand::App {
            document_id: Id::from_bytes([1; 16]),
            command,
        }
    }

    fn port(
        slots: ambre_core::Bounded<crate::conduit::ChannelSlots>,
    ) -> (Port<Probe>, crate::conduit::Receiver<LogicCommand<Probe>>) {
        let (tx, rx) = crate::conduit::local(slots);
        (Port::direct(tx), rx)
    }

    fn viewport() -> LogicCommand<Probe> {
        app(Probe::Viewport)
    }

    fn mutations() -> Vec<LogicCommand<Probe>> {
        [Probe::InsertColumn, Probe::MoveColumn, Probe::ResizeColumn]
            .into_iter()
            .map(app)
            .collect()
    }

    #[test]
    fn an_open_channel_takes_a_request_that_changes_nothing() {
        let (tx, rx) = port(ambre_core::Bounded::<crate::conduit::ChannelSlots>::of::<2>());
        assert_eq!(send_lossy(&tx, viewport()), LossyOutcome::Sent);
        assert_eq!(rx.len(), 1);
    }

    #[test]
    fn a_full_channel_drops_the_request_and_says_so() {
        let (tx, rx) = port(ambre_core::Bounded::<crate::conduit::ChannelSlots>::of::<1>());
        assert_eq!(send_lossy(&tx, viewport()), LossyOutcome::Sent);
        assert_eq!(send_lossy(&tx, viewport()), LossyOutcome::Dropped);
        assert!(!LossyOutcome::Dropped.sent());
        assert_eq!(rx.len(), 1);
    }

    #[test]
    fn a_command_that_changes_the_document_never_reaches_the_channel() {
        let (tx, rx) = port(ambre_core::Bounded::<crate::conduit::ChannelSlots>::of::<8>());
        for cmd in mutations() {
            assert_eq!(send_lossy(&tx, cmd), LossyOutcome::Refused);
        }
        assert_eq!(rx.len(), 0);
    }

    #[test]
    fn a_command_that_changes_nothing_but_is_not_named_lossy_is_refused() {
        let (tx, rx) = port(ambre_core::Bounded::<crate::conduit::ChannelSlots>::of::<1>());
        assert_eq!(send_lossy(&tx, app(Probe::Select)), LossyOutcome::Refused);
        assert_eq!(rx.len(), 0);
    }

    #[test]
    fn a_mutation_named_lossy_is_still_refused() {
        let (tx, rx) = port(ambre_core::Bounded::<crate::conduit::ChannelSlots>::of::<1>());
        let cmd = app(Probe::MislabelledDelete);
        assert_eq!(send_lossy(&tx, cmd), LossyOutcome::Refused);
        assert_eq!(rx.len(), 0);
    }

    #[test]
    fn a_command_of_another_plane_is_refused_with_the_mutations() {
        let (tx, rx) = port(ambre_core::Bounded::<crate::conduit::ChannelSlots>::of::<1>());
        let close = LogicCommand::CloseDocument {
            document_id: Id::from_bytes([2; 16]),
        };
        assert_eq!(send_lossy(&tx, close), LossyOutcome::Refused);
        assert_eq!(rx.len(), 0);
    }

    /// A channel plane whose channel command is the probe, so the gate reads
    /// a channel command as it reads a document one.
    enum ProbePlane {}

    impl ChannelPlane for ProbePlane {
        type Command = Probe;
        type SpaceCommand = std::convert::Infallible;
        type DirectDraft = std::convert::Infallible;
        type Event = std::convert::Infallible;
        type SpaceEvent = std::convert::Infallible;
        type DirectEvent = std::convert::Infallible;
        type Matches = std::convert::Infallible;
    }

    #[test]
    fn a_channel_command_takes_the_class_its_application_names() {
        let (tx, rx) =
            crate::conduit::local(ambre_core::Bounded::<crate::conduit::ChannelSlots>::of::<4>());
        let to_logic = Port::<std::convert::Infallible, ProbePlane>::direct(tx);
        let channel = |command| LogicCommand::Channel {
            channel_id: Id::from_bytes([3; 16]),
            command,
        };
        assert_eq!(
            send_lossy(&to_logic, channel(Probe::Viewport)),
            LossyOutcome::Sent
        );
        assert_eq!(
            send_lossy(&to_logic, channel(Probe::Select)),
            LossyOutcome::Refused
        );
        assert_eq!(
            send_lossy(&to_logic, channel(Probe::InsertColumn)),
            LossyOutcome::Refused
        );
        assert_eq!(rx.len(), 1);
    }

    #[test]
    fn a_gone_logic_thread_is_reported_not_dropped() {
        let (tx, rx) = port(ambre_core::Bounded::<crate::conduit::ChannelSlots>::of::<1>());
        drop(rx);
        assert_eq!(send_lossy(&tx, viewport()), LossyOutcome::Disconnected);
    }
}
