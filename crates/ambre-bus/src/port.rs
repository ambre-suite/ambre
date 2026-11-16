//! A view's two ends of the queues it shares with the Logic thread, typed by
//! its own application's command and event (`BUS-032`).
//!
//! The UI to Logic queue carries the Logic runtime's closed sum, which no
//! application crate can name. A [`Port`] takes the view's own command and
//! embeds it on the way in. A [`Feed`] projects the view's own event out of
//! what the launcher forwards. Both erase the sum, so a view type names its
//! own vocabulary and nothing downstream of it.
//!
//! A [`Port`] has no send of its own outside this crate. A view moves it into
//! its one [`Outbox`](crate::Outbox), whose two sends are the only way a
//! view writes to the Logic thread (`BUS-011`, `BUS-023`). Every port over
//! one queue holds through that queue's one [`Lane`] (`BUS-037`).

use std::rc::Rc;

use crate::conduit::{Receiver, TryRecvError};
use crate::outbox::{Lane, Queue};
use crate::{
    ChannelPlane, Embed, LogicCommand, LossyOutcome, NoChannels, PlaneOf, SendOutcome, UiEvent,
};

/// A view's sending end of the UI to Logic queue, for commands of type `A`
/// and the channel plane `P`.
///
/// Only tries, as every end the Main thread holds (`ARCH-021`). It has no
/// `Clone` and no public send. A port moves into one
/// [`Outbox`](crate::Outbox), so no second path reaches its queue. Every port
/// over one queue shares the queue's one [`Lane`], so no second hold queue
/// reorders what the first one holds (`BUS-037`).
pub struct Port<A, P: ChannelPlane = NoChannels> {
    lane: Rc<dyn Queue<A, P>>,
}

impl<A: 'static, P: ChannelPlane + PlaneOf<P> + 'static> Port<A, P> {
    /// A port over a queue that carries `A` and `P` themselves, on a lane of
    /// its own, as a test rig's is.
    pub fn direct(to_logic: crate::conduit::Sender<LogicCommand<A, P>>) -> Self {
        Lane::new(to_logic).port()
    }

    /// A port over no queue: every send reports the Logic thread gone. A
    /// launcher whose lane is already dropped hands this one out.
    pub fn closed() -> Self {
        let (to_logic, gone) =
            crate::conduit::local(ambre_core::Bounded::<crate::conduit::ChannelSlots>::of::<1>());
        drop(gone);
        Self::direct(to_logic)
    }
}

impl<A, P: ChannelPlane> Port<A, P> {
    /// A port over `lane`. Only a [`Lane`] builds one.
    pub(crate) fn over(lane: Rc<dyn Queue<A, P>>) -> Self {
        Self { lane }
    }

    /// Send one lossless command through the lane, or hold it there.
    pub(crate) fn send(&self, cmd: LogicCommand<A, P>) -> SendOutcome {
        self.lane.send(cmd)
    }

    /// Offer one lossy command to the queue, and never hold it.
    pub(crate) fn send_now(&self, cmd: LogicCommand<A, P>) -> LossyOutcome {
        self.lane.send_now(cmd)
    }

    /// Offer what the lane holds to the queue, oldest first.
    pub(crate) fn flush(&self) -> SendOutcome {
        self.lane.flush()
    }

    /// How many commands the lane holds, whichever port sent them.
    pub(crate) fn held(&self) -> ambre_core::Count<crate::outbox::HeldCommands> {
        self.lane.held()
    }
}

type RecvFn<E, P> = dyn Fn() -> Result<UiEvent<E, P>, TryRecvError> + Send;

/// A view's receiving end of the events the launcher forwards it, for
/// document events of type `E` and the channel plane `P`.
pub struct Feed<E, P: ChannelPlane = NoChannels> {
    recv: Box<RecvFn<E, P>>,
}

impl<E: 'static, P: ChannelPlane + PlaneOf<P> + 'static> Feed<E, P>
where
    UiEvent<E, P>: Send,
{
    /// A feed over a queue that carries `E` and `P` themselves, as a test
    /// rig's does.
    pub fn direct(from_logic: Receiver<UiEvent<E, P>>) -> Self {
        Self::embedded(from_logic)
    }
}

impl<E: 'static, P: ChannelPlane + 'static> Feed<E, P> {
    /// A feed over a queue that carries the sum `S`, which embeds `E`, and
    /// the plane `Q`, which embeds `P`.
    ///
    /// A document event of another application is skipped, and so is a
    /// channel event a view without channels cannot hold. The view could do
    /// nothing with either, as its own match on the sum did nothing before.
    pub fn embedded<S, Q>(from_logic: Receiver<UiEvent<S, Q>>) -> Self
    where
        S: Embed<E> + 'static,
        Q: PlaneOf<P> + 'static,
        UiEvent<S, Q>: Send,
    {
        Self {
            recv: Box::new(move || {
                loop {
                    if let Some(ev) = from_logic.try_recv()?.project() {
                        return Ok(ev);
                    }
                }
            }),
        }
    }
}

impl<E, P: ChannelPlane> Feed<E, P> {
    /// The next event queued for the view, without waiting.
    ///
    /// # Errors
    ///
    /// [`TryRecvError::Empty`] when nothing is queued, and
    /// [`TryRecvError::Disconnected`] once the Logic thread is gone.
    pub fn try_recv(&self) -> Result<UiEvent<E, P>, TryRecvError> {
        (self.recv)()
    }
}
