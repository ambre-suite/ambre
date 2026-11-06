// ambre-bus/src/outbox.rs
// ============================================================================
// A view's one way to write to the Logic thread (`BUS-011`, `BUS-016`,
// `BUS-018`, `BUS-023`).
//
// A view holds a bounded queue to the Logic thread and must never block, so
// every send is a try. The outbox owns the view's `Port`, which has no send of
// its own outside this crate, and offers two sends.
//
// A lossy command drops itself on a full queue, and the next frame resends the
// state. A lossless one carries intent the user gave once. The outbox holds
// such a command instead, reads busy while it holds anything, and the view
// refuses input until the queue drains. Past `HOLD_CAPACITY` it refuses the
// command and says so, and the view then leaves its own state alone
// (`BUS-018`).
// ============================================================================

use std::cell::Cell;
use std::collections::VecDeque;
use std::rc::Rc;

use crate::conduit::{Sender, TrySendError};
use crate::lossy::send_lossy;
use crate::{ChannelPlane, Embed, LogicCommand, LossyOutcome, Mutation, NoChannels, PlaneOf, Port};

/// Commands one lane holds before it refuses the next.
///
/// A view stops accepting input on the first held command, so the lane takes
/// what one frame already had in flight and nothing more. Sixty-four is well
/// above that and still a bound.
pub const HOLD_CAPACITY: usize = 64;

/// What one lossless send did, and what the outbox holds after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendOutcome {
    /// The command reached the queue, and the outbox holds nothing.
    Sent,
    /// The queue was full, or the lane held an older command. The lane holds
    /// this one for a later frame, and the view is busy until the queue
    /// drains.
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

/// The one hold queue of a UI to Logic queue, which every port over that
/// queue shares (`BUS-037`).
///
/// A view and the launcher each own an outbox, and each outbox holds through
/// its port. Were each to hold apart, a close the launcher sent after an edit
/// could leave first, and the Logic thread would drop the edit (`BUS-005`).
/// One lane gives every sender one order, the order of the calls.
///
/// Every call runs on the Main thread, so a `Cell` is the whole guard and no
/// lock is taken (`ARCH-020`). The lane is `!Send`, as each end it serves is.
pub struct Lane<S, Q: ChannelPlane = NoChannels> {
    shared: Rc<Held<S, Q>>,
}

impl<S, Q: ChannelPlane> Clone for Lane<S, Q> {
    fn clone(&self) -> Self {
        Self {
            shared: Rc::clone(&self.shared),
        }
    }
}

impl<S: 'static, Q: ChannelPlane + 'static> Lane<S, Q> {
    /// The lane of `to_logic`, holding up to [`HOLD_CAPACITY`] commands.
    pub fn new(to_logic: Sender<LogicCommand<S, Q>>) -> Self {
        Self::with_capacity(to_logic, HOLD_CAPACITY)
    }

    /// A lane with an explicit hold bound. A test drives the refusal branch
    /// through a small one instead of filling sixty-four slots.
    pub fn with_capacity(to_logic: Sender<LogicCommand<S, Q>>, capacity: usize) -> Self {
        Self {
            shared: Rc::new(Held {
                to_logic,
                queue: Cell::new(VecDeque::new()),
                capacity,
            }),
        }
    }

    /// A port for one sender of commands `A` on the plane `P`, which this
    /// lane embeds into `S` and `Q` on the way in.
    pub fn port<A, P>(&self) -> Port<A, P>
    where
        S: Embed<A>,
        Q: PlaneOf<P>,
        P: ChannelPlane,
    {
        Port::over(Rc::clone(&self.shared) as Rc<dyn Queue<A, P>>)
    }
}

/// What a port does with a command, whatever the vocabulary of the lane under
/// it. The lane embeds the command first, so it holds the queue's own type.
pub(crate) trait Queue<A, P: ChannelPlane> {
    fn send(&self, cmd: LogicCommand<A, P>) -> SendOutcome;
    fn send_now(&self, cmd: LogicCommand<A, P>) -> LossyOutcome;
    fn flush(&self) -> SendOutcome;
    fn held(&self) -> usize;
}

/// The queue's sender and what the lane holds for it, oldest first.
struct Held<S, Q: ChannelPlane> {
    to_logic: Sender<LogicCommand<S, Q>>,
    queue: Cell<VecDeque<LogicCommand<S, Q>>>,
    capacity: usize,
}

impl<S, Q: ChannelPlane> Held<S, Q> {
    /// Run `f` over the held commands. `f` sends on the queue and calls back
    /// into no lane, so nothing reads the cell while it is out.
    fn with_queue<R>(&self, f: impl FnOnce(&mut VecDeque<LogicCommand<S, Q>>) -> R) -> R {
        let mut queue = self.queue.take();
        let outcome = f(&mut queue);
        self.queue.set(queue);
        outcome
    }

    /// Take one command into the hold queue, or refuse it at the bound.
    fn hold(
        &self,
        queue: &mut VecDeque<LogicCommand<S, Q>>,
        cmd: LogicCommand<S, Q>,
    ) -> SendOutcome {
        if queue.len() >= self.capacity {
            return SendOutcome::Refused;
        }
        queue.push_back(cmd);
        SendOutcome::Held
    }
}

impl<A, P, S, Q> Queue<A, P> for Held<S, Q>
where
    P: ChannelPlane,
    Q: PlaneOf<P>,
    S: Embed<A>,
{
    /// A command sent while the lane holds anything joins the back of it,
    /// whichever port sent the older one. The Logic thread applies the
    /// commands in the order of the calls, and an overtaking command would
    /// reorder them.
    fn send(&self, cmd: LogicCommand<A, P>) -> SendOutcome {
        let cmd = cmd.embed_into::<S, Q>();
        self.with_queue(|queue| {
            if !queue.is_empty() {
                return self.hold(queue, cmd);
            }
            match self.to_logic.try_send(cmd) {
                Ok(()) => SendOutcome::Sent,
                Err(TrySendError::Full(cmd)) => self.hold(queue, cmd),
                Err(TrySendError::Disconnected(_)) => SendOutcome::Disconnected,
            }
        })
    }

    /// A lossy command does not wait behind the held ones, and does not pass
    /// them either: it drops, and its sender offers it again (`BUS-024`).
    fn send_now(&self, cmd: LogicCommand<A, P>) -> LossyOutcome {
        if self.held() > 0 {
            return LossyOutcome::Dropped;
        }
        match self.to_logic.try_send(cmd.embed_into::<S, Q>()) {
            Ok(()) => LossyOutcome::Sent,
            Err(TrySendError::Full(_)) => LossyOutcome::Dropped,
            Err(TrySendError::Disconnected(_)) => LossyOutcome::Disconnected,
        }
    }

    fn flush(&self) -> SendOutcome {
        self.with_queue(|queue| {
            while let Some(cmd) = queue.pop_front() {
                match self.to_logic.try_send(cmd) {
                    Ok(()) => {}
                    Err(TrySendError::Full(cmd)) => {
                        queue.push_front(cmd);
                        return SendOutcome::Held;
                    }
                    Err(TrySendError::Disconnected(_)) => return SendOutcome::Disconnected,
                }
            }
            SendOutcome::Sent
        })
    }

    fn held(&self) -> usize {
        self.with_queue(|queue| queue.len())
    }
}

/// The send path from one view to the Logic thread, and the only one.
///
/// Wraps the view's port, which holds through the lane of its queue. A view
/// owns one per open document and drives it from two places. The command
/// emitters call [`send`](Self::send) or [`send_lossy`](Self::send_lossy), and
/// the frame hook calls [`flush`](Self::flush).
pub struct Outbox<A, P: ChannelPlane = NoChannels> {
    to_logic: Port<A, P>,
}

impl<A, P: ChannelPlane> Outbox<A, P> {
    /// An outbox over `to_logic`. Its hold bound is the lane's.
    pub fn new(to_logic: Port<A, P>) -> Self {
        Self { to_logic }
    }

    /// Whether the lane holds a command the Logic thread has not taken.
    ///
    /// The view refuses input while it is `true`, and publishes it every frame
    /// as its busy signal (`BUS-016`). A command another sender holds makes
    /// it `true` too: the queue is full for every sender alike.
    pub fn is_busy(&self) -> bool {
        self.held() > 0
    }

    /// How many commands the lane holds. For a test and a diagnostic.
    pub fn held(&self) -> usize {
        self.to_logic.held()
    }

    /// Send one lossless command, or hold it for a later frame.
    ///
    /// A command sent while the lane holds anything joins the back of the
    /// lane. The Logic thread applies an edit stream in the order the user
    /// typed it, and a command overtaking a held one would reorder that stream.
    pub fn send(&mut self, cmd: LogicCommand<A, P>) -> SendOutcome {
        self.to_logic.send(cmd)
    }

    /// Offer the held commands to the queue, oldest first, whichever sender
    /// holds them.
    ///
    /// Stops on the first one the queue refuses, so the queue keeps its
    /// order. Returns what the lane holds afterwards: `Sent` when it is
    /// empty, `Held` when the queue filled again.
    pub fn flush(&mut self) -> SendOutcome {
        self.to_logic.flush()
    }

    /// Send one lossy command, or drop it on a full queue (`BUS-024`).
    ///
    /// Refuses, and sends nothing, unless the command's application names it
    /// lossy and it changes nothing (`BUS-023`). The refused command owes
    /// [`send`](Self::send). A lossy command never waits behind the held
    /// ones. It drops while the lane holds anything, and the next frame offers
    /// it again. Its answer carries a version the view checks (`BUS-017`).
    pub fn send_lossy(&self, cmd: LogicCommand<A, P>) -> LossyOutcome
    where
        A: Mutation,
    {
        send_lossy(&self.to_logic, cmd)
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
        let mut outbox = Outbox::new(Port::direct(tx));
        assert_eq!(outbox.send(cmd(1)), SendOutcome::Sent);
        assert!(!outbox.is_busy());
        assert_eq!(rx.len(), 1);
    }

    #[test]
    fn a_full_channel_holds_the_command_instead_of_dropping_it() {
        let (tx, rx) =
            crate::conduit::local(ambre_core::Bounded::<crate::conduit::ChannelSlots>::of::<1>());
        let mut outbox = Outbox::new(Port::direct(tx));
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
        let mut outbox = Outbox::new(Port::direct(tx));
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
        let mut outbox = Outbox::new(Lane::<std::convert::Infallible>::with_capacity(tx, 4).port());
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
        let mut outbox = Outbox::new(Lane::<std::convert::Infallible>::with_capacity(tx, 4).port());
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
        let mut outbox = Outbox::new(Lane::<std::convert::Infallible>::with_capacity(tx, 2).port());
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
        let mut outbox = Outbox::new(Port::direct(tx));
        assert_eq!(outbox.send(cmd(1)), SendOutcome::Disconnected);
        assert!(!outbox.is_busy());
    }
    /// A view and the launcher over one queue, each with its own outbox. The
    /// view holds an edit, then the launcher closes the document. The close
    /// joins the lane behind the edit, so the Logic thread reads the edit
    /// while the document is still open (`BUS-037`).
    #[test]
    fn a_close_never_overtakes_an_edit_another_outbox_holds() {
        let (tx, rx) =
            crate::conduit::local(ambre_core::Bounded::<crate::conduit::ChannelSlots>::of::<1>());
        let lane = Lane::<std::convert::Infallible>::new(tx);
        let mut view = Outbox::new(lane.port());
        let mut launcher = Outbox::new(lane.port());
        assert_eq!(view.send(cmd(1)), SendOutcome::Sent);
        assert_eq!(view.send(cmd(2)), SendOutcome::Held);
        // The Logic thread frees the one slot before the launcher sends.
        assert_eq!(rx.try_recv().map(|c| document_byte(&c)), Ok(1));
        assert_eq!(launcher.send(cmd(9)), SendOutcome::Held);
        assert!(launcher.is_busy());
        assert_eq!(view.flush(), SendOutcome::Held);
        assert_eq!(rx.try_recv().map(|c| document_byte(&c)), Ok(2));
        assert_eq!(launcher.flush(), SendOutcome::Sent);
        assert_eq!(rx.try_recv().map(|c| document_byte(&c)), Ok(9));
        assert!(!view.is_busy());
    }

    /// The other direction: what the launcher holds, an open for example,
    /// leaves before an edit a view sends after it.
    #[test]
    fn an_edit_never_overtakes_a_command_the_launcher_holds() {
        let (tx, rx) =
            crate::conduit::local(ambre_core::Bounded::<crate::conduit::ChannelSlots>::of::<1>());
        let lane = Lane::<std::convert::Infallible>::new(tx);
        let mut launcher = Outbox::new(lane.port());
        let mut view = Outbox::new(lane.port());
        assert_eq!(launcher.send(cmd(1)), SendOutcome::Sent);
        assert_eq!(launcher.send(cmd(2)), SendOutcome::Held);
        assert!(rx.try_recv().is_ok());
        assert_eq!(view.send(cmd(3)), SendOutcome::Held);
        assert_eq!(view.flush(), SendOutcome::Held);
        assert_eq!(rx.try_recv().map(|c| document_byte(&c)), Ok(2));
        assert_eq!(view.flush(), SendOutcome::Sent);
        assert_eq!(rx.try_recv().map(|c| document_byte(&c)), Ok(3));
    }

    /// The hold bound belongs to the lane, so two outboxes on it share it.
    #[test]
    fn two_outboxes_on_one_lane_share_its_hold_bound() {
        let (tx, _rx) =
            crate::conduit::local(ambre_core::Bounded::<crate::conduit::ChannelSlots>::of::<1>());
        let lane = Lane::<std::convert::Infallible>::with_capacity(tx, 2);
        let mut view = Outbox::new(lane.port());
        let mut launcher = Outbox::new(lane.port());
        view.send(cmd(1));
        assert_eq!(view.send(cmd(2)), SendOutcome::Held);
        assert_eq!(launcher.send(cmd(3)), SendOutcome::Held);
        assert_eq!(launcher.send(cmd(4)), SendOutcome::Refused);
        assert_eq!(view.held(), 2);
    }

    /// A port over no queue reports the Logic thread gone, and holds nothing.
    #[test]
    fn a_closed_port_reports_the_logic_thread_gone() {
        let mut outbox = Outbox::new(Port::<std::convert::Infallible>::closed());
        assert_eq!(outbox.send(cmd(1)), SendOutcome::Disconnected);
        assert!(!outbox.is_busy());
    }
}
