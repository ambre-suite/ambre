// ambre-bus/src/lossy.rs
// ============================================================================
// The send discipline of the lossy class (`BUS-023`, `BUS-024`).
//
// A lossy command asks the Logic thread for something the screen already shows:
// a viewport slice, a hit test, a set of aggregates. A full queue drops it,
// and the next frame offers it again. A command that changes the document
// carries intent given once, so the lossy class refuses it and the caller owes
// the outbox (`BUS-011`).
//
// The caller reads the outcome. A dropped command it does not re-offer leaves
// the screen ahead of the document, which is the state `BUS-018` bans.
// ============================================================================

use crate::{Delivery, LogicCommand, Mutation, Port};

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
/// The class is one document command that changes nothing. A command that
/// mutates the document, and every command of the service and control planes,
/// belongs to the lossless path instead.
pub fn is_lossy_class<A: Mutation>(cmd: &LogicCommand<A>) -> bool {
    matches!(cmd, LogicCommand::App { command, .. } if !command.is_mutating())
}

/// Send one lossy command, and say what became of it.
///
/// Never blocks: the UI thread runs the frame. A full queue drops the
/// command rather than holding it, because the next frame carries the same
/// request over a state that has moved on.
pub fn send_lossy<A: Mutation>(to_logic: &Port<A>, cmd: LogicCommand<A>) -> LossyOutcome {
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

    /// A document command of no application: one request that changes
    /// nothing, and the three mutation shapes the gate must refuse.
    #[derive(Debug)]
    enum Probe {
        Viewport,
        InsertColumn,
        MoveColumn,
        ResizeColumn,
    }

    impl Mutation for Probe {
        fn is_mutating(&self) -> bool {
            !matches!(self, Self::Viewport)
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
    fn a_command_of_another_plane_is_refused_with_the_mutations() {
        let (tx, rx) = port(ambre_core::Bounded::<crate::conduit::ChannelSlots>::of::<1>());
        let close = LogicCommand::CloseDocument {
            document_id: Id::from_bytes([2; 16]),
        };
        assert_eq!(send_lossy(&tx, close), LossyOutcome::Refused);
        assert_eq!(rx.len(), 0);
    }

    #[test]
    fn a_gone_logic_thread_is_reported_not_dropped() {
        let (tx, rx) = port(ambre_core::Bounded::<crate::conduit::ChannelSlots>::of::<1>());
        drop(rx);
        assert_eq!(send_lossy(&tx, viewport()), LossyOutcome::Disconnected);
    }
}
