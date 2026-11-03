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

use crate::LogicCommand;

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
pub fn is_lossy_class(cmd: &LogicCommand) -> bool {
    matches!(cmd, LogicCommand::App { command, .. } if !command.is_mutating())
}

/// Send one lossy command, and say what became of it.
///
/// Never blocks: the UI thread runs the frame. A full queue drops the
/// command rather than holding it, because the next frame carries the same
/// request over a state that has moved on.
pub fn send_lossy(
    to_logic: &crate::conduit::Sender<LogicCommand>,
    cmd: LogicCommand,
) -> LossyOutcome {
    if !is_lossy_class(&cmd) {
        return LossyOutcome::Refused;
    }
    match to_logic.try_send(cmd) {
        Ok(()) => LossyOutcome::Sent,
        Err(crate::conduit::TrySendError::Full(_)) => LossyOutcome::Dropped,
        Err(crate::conduit::TrySendError::Disconnected(_)) => LossyOutcome::Disconnected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppCommand, TrameCommand};
    use crate::{Col, Row};
    use ambre_core::id::Id;
    use ambre_core::index::Pos;

    /// The viewport request, the archetype of the class.
    fn viewport() -> LogicCommand {
        LogicCommand::App {
            document_id: Id::from_bytes([1; 16]),
            command: AppCommand::Trame(TrameCommand::ScrollViewport {
                version: ambre_core::Seq::nth(1),
                row_start: Pos::<Row>::new(0),
                row_end: Pos::<Row>::new(10),
                col_start: Pos::<Col>::new(0),
                col_end: Pos::<Col>::new(10),
            }),
        }
    }

    /// One document mutation of each shape the interface used to send lossy.
    fn mutations() -> Vec<LogicCommand> {
        [
            TrameCommand::InsertColLeft,
            TrameCommand::InsertRowAbove,
            TrameCommand::MoveColumn {
                from: Pos::<Col>::new(0),
                to: Pos::<Col>::new(3),
            },
            TrameCommand::ResizeCol {
                version: ambre_core::Seq::nth(1),
                sheet_id: ambre_core::id::Id::<ambre_core::id::Sheet>::from_text("s1".to_string()),
                col: Pos::<Col>::new(0),
                width: ambre_core::Px::<ambre_core::Doc>::new(120.0),
            },
        ]
        .into_iter()
        .map(|command| LogicCommand::App {
            document_id: Id::from_bytes([1; 16]),
            command: AppCommand::Trame(command),
        })
        .collect()
    }

    #[test]
    fn an_open_channel_takes_a_request_that_changes_nothing() {
        let (tx, rx) = crate::conduit::local(crate::conduit::ChannelCapacity::new(2));
        assert_eq!(send_lossy(&tx, viewport()), LossyOutcome::Sent);
        assert_eq!(rx.len(), 1);
    }

    #[test]
    fn a_full_channel_drops_the_request_and_says_so() {
        let (tx, rx) = crate::conduit::local(crate::conduit::ChannelCapacity::new(1));
        assert_eq!(send_lossy(&tx, viewport()), LossyOutcome::Sent);
        assert_eq!(send_lossy(&tx, viewport()), LossyOutcome::Dropped);
        assert!(!LossyOutcome::Dropped.sent());
        assert_eq!(rx.len(), 1);
    }

    #[test]
    fn a_command_that_changes_the_document_never_reaches_the_channel() {
        let (tx, rx) = crate::conduit::local(crate::conduit::ChannelCapacity::new(8));
        for cmd in mutations() {
            assert_eq!(send_lossy(&tx, cmd), LossyOutcome::Refused);
        }
        assert_eq!(rx.len(), 0);
    }

    #[test]
    fn a_command_of_another_plane_is_refused_with_the_mutations() {
        let (tx, rx) = crate::conduit::local(crate::conduit::ChannelCapacity::new(1));
        let close = LogicCommand::CloseDocument {
            document_id: Id::from_bytes([2; 16]),
        };
        assert_eq!(send_lossy(&tx, close), LossyOutcome::Refused);
        assert_eq!(rx.len(), 0);
    }

    #[test]
    fn a_gone_logic_thread_is_reported_not_dropped() {
        let (tx, rx) = crate::conduit::local(crate::conduit::ChannelCapacity::new(1));
        drop(rx);
        assert_eq!(send_lossy(&tx, viewport()), LossyOutcome::Disconnected);
    }
}
