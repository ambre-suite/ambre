// ambre-bus/src/outbox.rs
// ============================================================================
// The send discipline of the lossless class (`BUS-011`, `BUS-016`, `BUS-018`).
//
// A view holds a bounded sender to the Logic thread and must never block, so
// every send is a `try_send`. A lossy command drops itself on a full queue,
// and the next frame resends the state; a lossless one carries intent the user
// gave once. The outbox holds such a command instead,
// reads busy while it holds anything, and the view refuses input until the queue
// drains. Past `HOLD_CAPACITY` it refuses the command and says so, and the view
// then leaves its own state alone (`BUS-018`).
// ============================================================================

use std::collections::VecDeque;

use crate::LogicCommand;

/// Commands one outbox holds before it refuses the next.
///
/// The view stops accepting input on the first held command, so the queue takes
/// what one frame already had in flight and nothing more. Sixty-four is well
/// above that and still a bound.
pub const HOLD_CAPACITY: usize = 64;

/// What one lossless send did, and what the outbox holds after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendOutcome {
    /// The command reached the queue, and the outbox holds nothing.
    Sent,
    /// The queue was full. The outbox holds the command for a later frame,
    /// and the view is busy until the queue drains.
    Held,
    /// The hold queue is full, so the outbox took nothing. Nothing carries the
    /// command any further, and the view MUST NOT act as though it left
    /// (`BUS-018`).
    Refused,
    /// The Logic thread is gone. This queue carries nothing again.
    Disconnected,
}

impl SendOutcome {
    /// Whether the command is on its way to the Logic thread.
    ///
    /// The view reflects a command in its own state only when this is `true`.
    pub fn accepted(self) -> bool {
        matches!(self, Self::Sent | Self::Held)
    }
}

/// The lossless send path from one view to the Logic thread.
///
/// Wraps the bounded sender and the hold queue a full queue feeds. A view
/// owns one per open document and drives it from two places: the command
/// emitters call [`send`](Self::send), and the frame hook calls
/// [`flush`](Self::flush).
pub struct Outbox {
    to_logic: crate::conduit::Sender<LogicCommand>,
    held: VecDeque<LogicCommand>,
    capacity: usize,
}

/// Whether a view's outbox holds a command the Logic thread has not taken
/// (`BUS-016`). The view publishes it every frame as its busy signal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Load {
    #[default]
    Free,
    Busy,
}

impl Outbox {
    /// An outbox over `to_logic`, holding up to [`HOLD_CAPACITY`] commands.
    pub fn new(to_logic: crate::conduit::Sender<LogicCommand>) -> Self {
        Self::with_capacity(to_logic, HOLD_CAPACITY)
    }

    /// An outbox with an explicit hold bound. A test drives the refusal branch
    /// through a small one instead of filling sixty-four slots.
    pub fn with_capacity(to_logic: crate::conduit::Sender<LogicCommand>, capacity: usize) -> Self {
        Self {
            to_logic,
            held: VecDeque::new(),
            capacity,
        }
    }

    /// Whether the outbox holds a command the Logic thread has not taken.
    ///
    /// The view refuses input while it is `true` (`BUS-016`).
    pub fn is_busy(&self) -> bool {
        !self.held.is_empty()
    }

    /// The level the view publishes as its busy signal (`BUS-016`).
    pub fn load(&self) -> Load {
        if self.is_busy() {
            Load::Busy
        } else {
            Load::Free
        }
    }

    /// How many commands the outbox holds. For a test and a diagnostic.
    pub fn held(&self) -> usize {
        self.held.len()
    }

    /// Send one lossless command, or hold it for a later frame.
    ///
    /// A command sent while the outbox holds anything joins the back of the
    /// queue. The Logic thread applies an edit stream in the order the user
    /// typed it, and a command overtaking a held one would reorder that stream.
    pub fn send(&mut self, cmd: LogicCommand) -> SendOutcome {
        if self.is_busy() {
            return self.hold(cmd);
        }
        match self.to_logic.try_send(cmd) {
            Ok(()) => SendOutcome::Sent,
            Err(crate::conduit::TrySendError::Full(cmd)) => self.hold(cmd),
            Err(crate::conduit::TrySendError::Disconnected(_)) => SendOutcome::Disconnected,
        }
    }

    /// Offer the held commands to the queue, oldest first.
    ///
    /// Stops on the first one the queue refuses, so the queue keeps its
    /// order. Returns what the outbox holds afterwards: `Sent` when it is
    /// empty, `Held` when the queue filled again.
    pub fn flush(&mut self) -> SendOutcome {
        while let Some(cmd) = self.held.pop_front() {
            match self.to_logic.try_send(cmd) {
                Ok(()) => {}
                Err(crate::conduit::TrySendError::Full(cmd)) => {
                    self.held.push_front(cmd);
                    return SendOutcome::Held;
                }
                Err(crate::conduit::TrySendError::Disconnected(_)) => {
                    return SendOutcome::Disconnected;
                }
            }
        }
        SendOutcome::Sent
    }

    /// Take one command into the hold queue, or refuse it at the bound.
    fn hold(&mut self, cmd: LogicCommand) -> SendOutcome {
        if self.held.len() >= self.capacity {
            return SendOutcome::Refused;
        }
        self.held.push_back(cmd);
        SendOutcome::Held
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_core::id::Id;

    /// One control command, distinguishable by the document it names.
    fn cmd(n: u8) -> LogicCommand {
        LogicCommand::CloseDocument {
            document_id: Id::from_bytes([n; 16]),
        }
    }

    /// The document a `CloseDocument` names, so a test reads the order back.
    fn document_byte(cmd: &LogicCommand) -> u8 {
        match cmd {
            LogicCommand::CloseDocument { document_id } => document_id.as_bytes()[0],
            _ => 0,
        }
    }

    #[test]
    fn an_open_channel_takes_the_command_and_the_outbox_stays_idle() {
        let (tx, rx) =
            crate::conduit::local(ambre_core::Bounded::<crate::conduit::ChannelSlots>::of::<2>());
        let mut outbox = Outbox::new(tx);
        assert_eq!(outbox.send(cmd(1)), SendOutcome::Sent);
        assert!(!outbox.is_busy());
        assert_eq!(rx.len(), 1);
    }

    #[test]
    fn a_full_channel_holds_the_command_instead_of_dropping_it() {
        let (tx, rx) =
            crate::conduit::local(ambre_core::Bounded::<crate::conduit::ChannelSlots>::of::<1>());
        let mut outbox = Outbox::new(tx);
        assert_eq!(outbox.send(cmd(1)), SendOutcome::Sent);
        assert_eq!(outbox.send(cmd(2)), SendOutcome::Held);
        assert!(outbox.is_busy());
        // The queue took one and the outbox holds the other, so it kept both.
        assert_eq!(rx.len(), 1);
        assert_eq!(outbox.held(), 1);
    }

    #[test]
    fn a_flush_after_the_reader_drains_sends_the_held_command() {
        let (tx, rx) =
            crate::conduit::local(ambre_core::Bounded::<crate::conduit::ChannelSlots>::of::<1>());
        let mut outbox = Outbox::new(tx);
        outbox.send(cmd(1));
        outbox.send(cmd(2));
        assert!(rx.try_recv().is_ok());
        assert_eq!(outbox.flush(), SendOutcome::Sent);
        assert!(!outbox.is_busy());
        assert_eq!(document_byte(&rx.try_recv().unwrap()), 2);
    }

    #[test]
    fn a_held_queue_keeps_the_order_the_user_typed() {
        let (tx, rx) =
            crate::conduit::local(ambre_core::Bounded::<crate::conduit::ChannelSlots>::of::<1>());
        let mut outbox = Outbox::with_capacity(tx, 4);
        outbox.send(cmd(1));
        outbox.send(cmd(2));
        outbox.send(cmd(3));
        // The reader takes one slot at a time, so the flush is partial first.
        assert!(rx.try_recv().is_ok());
        assert_eq!(outbox.flush(), SendOutcome::Held);
        assert_eq!(document_byte(&rx.try_recv().unwrap()), 2);
        assert_eq!(outbox.flush(), SendOutcome::Sent);
        assert_eq!(document_byte(&rx.try_recv().unwrap()), 3);
    }

    #[test]
    fn a_send_while_busy_never_overtakes_a_held_command() {
        let (tx, rx) =
            crate::conduit::local(ambre_core::Bounded::<crate::conduit::ChannelSlots>::of::<2>());
        let mut outbox = Outbox::with_capacity(tx, 4);
        outbox.send(cmd(1));
        outbox.send(cmd(2));
        // The queue is full, so the outbox holds this one.
        assert_eq!(outbox.send(cmd(3)), SendOutcome::Held);
        // The reader frees a slot, and the next send must still queue behind 3.
        assert!(rx.try_recv().is_ok());
        assert_eq!(outbox.send(cmd(4)), SendOutcome::Held);
        assert!(rx.try_recv().is_ok());
        assert_eq!(outbox.flush(), SendOutcome::Sent);
        assert_eq!(document_byte(&rx.try_recv().unwrap()), 3);
        assert_eq!(document_byte(&rx.try_recv().unwrap()), 4);
    }

    #[test]
    fn the_hold_bound_refuses_rather_than_grows() {
        let (tx, _rx) =
            crate::conduit::local(ambre_core::Bounded::<crate::conduit::ChannelSlots>::of::<1>());
        let mut outbox = Outbox::with_capacity(tx, 2);
        outbox.send(cmd(1));
        assert_eq!(outbox.send(cmd(2)), SendOutcome::Held);
        assert_eq!(outbox.send(cmd(3)), SendOutcome::Held);
        assert_eq!(outbox.send(cmd(4)), SendOutcome::Refused);
        assert_eq!(outbox.held(), 2);
        assert!(!SendOutcome::Refused.accepted());
    }

    #[test]
    fn a_gone_logic_thread_is_reported_not_held() {
        let (tx, rx) =
            crate::conduit::local(ambre_core::Bounded::<crate::conduit::ChannelSlots>::of::<1>());
        drop(rx);
        let mut outbox = Outbox::new(tx);
        assert_eq!(outbox.send(cmd(1)), SendOutcome::Disconnected);
        assert!(!outbox.is_busy());
    }
}
