//! A view's two ends of the queues it shares with the Logic thread, typed by
//! its own application's command and event (`BUS-032`).
//!
//! The UI to Logic queue carries the Logic runtime's closed sum, which no
//! application crate can name. A [`Port`] takes the view's own command and
//! embeds it on the way in. An [`Inbox`] projects the view's own event out of
//! what the launcher forwards. Both erase the sum, so a view type names its
//! own vocabulary and nothing downstream of it.

use std::sync::Arc;

use crate::conduit::{Receiver, Sender, TryRecvError, TrySendError};
use crate::{ChannelPlane, Embed, LogicCommand, NoChannels, PlaneOf, UiEvent};

/// What became of a command offered through [`Port::try_send`].
#[must_use]
pub enum Delivery<A, P: ChannelPlane = NoChannels> {
    /// The queue took it.
    Sent,
    /// The queue is at capacity: the command comes back.
    Full(LogicCommand<A, P>),
    /// The Logic thread is gone, and so is the command.
    Disconnected,
}

impl<A, P: ChannelPlane> Delivery<A, P> {
    /// Whether the queue took the command.
    pub fn sent(&self) -> bool {
        matches!(self, Self::Sent)
    }
}

type SendFn<A, P> = dyn Fn(LogicCommand<A, P>) -> Delivery<A, P> + Send + Sync;

/// A view's sending end of the UI to Logic queue, for commands of type `A`
/// and the channel plane `P`.
///
/// Only tries, as every end the Main thread holds (`ARCH-021`).
pub struct Port<A, P: ChannelPlane = NoChannels> {
    send: Arc<SendFn<A, P>>,
}

impl<A, P: ChannelPlane> Clone for Port<A, P> {
    fn clone(&self) -> Self {
        Self {
            send: Arc::clone(&self.send),
        }
    }
}

impl<A: Send + 'static, P: ChannelPlane + 'static> Port<A, P>
where
    P: PlaneOf<P>,
    LogicCommand<A, P>: Send,
{
    /// A port over a queue that carries `A` and `P` themselves, as a test
    /// rig's does.
    pub fn direct(to_logic: Sender<LogicCommand<A, P>>) -> Self {
        Self::embedded(to_logic)
    }
}

impl<A: 'static, P: ChannelPlane + 'static> Port<A, P> {
    /// A port over a queue that carries the sum `S`, which embeds `A`, and
    /// the plane `Q`, which embeds `P`.
    pub fn embedded<S, Q>(to_logic: Sender<LogicCommand<S, Q>>) -> Self
    where
        S: Embed<A> + 'static,
        Q: PlaneOf<P> + 'static,
        LogicCommand<S, Q>: Send,
    {
        Self {
            send: Arc::new(move |cmd: LogicCommand<A, P>| {
                match to_logic.try_send(cmd.embed_into()) {
                    Ok(()) => Delivery::Sent,
                    // The projection of a value just embedded never refuses it.
                    // Were it to, the command is lost as a disconnection would
                    // lose it.
                    Err(TrySendError::Full(cmd)) => {
                        cmd.project().map_or(Delivery::Disconnected, Delivery::Full)
                    }
                    Err(TrySendError::Disconnected(_)) => Delivery::Disconnected,
                }
            }),
        }
    }
}

impl<A, P: ChannelPlane> Port<A, P> {
    /// Offer `cmd` to the Logic thread, without waiting.
    pub fn try_send(&self, cmd: LogicCommand<A, P>) -> Delivery<A, P> {
        (self.send)(cmd)
    }
}

type RecvFn<E, P> = dyn Fn() -> Result<UiEvent<E, P>, TryRecvError> + Send;

/// A view's receiving end of the events the launcher forwards it, for
/// document events of type `E` and the channel plane `P`.
pub struct Inbox<E, P: ChannelPlane = NoChannels> {
    recv: Box<RecvFn<E, P>>,
}

impl<E: 'static, P: ChannelPlane + PlaneOf<P> + 'static> Inbox<E, P>
where
    UiEvent<E, P>: Send,
{
    /// An inbox over a queue that carries `E` and `P` themselves, as a test
    /// rig's does.
    pub fn direct(from_logic: Receiver<UiEvent<E, P>>) -> Self {
        Self::embedded(from_logic)
    }
}

impl<E: 'static, P: ChannelPlane + 'static> Inbox<E, P> {
    /// An inbox over a queue that carries the sum `S`, which embeds `E`, and
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

impl<E, P: ChannelPlane> Inbox<E, P> {
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
