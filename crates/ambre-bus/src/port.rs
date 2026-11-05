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
use crate::{Embed, LogicCommand, UiEvent};

/// What became of a command offered through [`Port::try_send`].
#[derive(Debug)]
#[must_use]
pub enum Delivery<A> {
    /// The queue took it.
    Sent,
    /// The queue is at capacity: the command comes back.
    Full(LogicCommand<A>),
    /// The Logic thread is gone, and so is the command.
    Disconnected,
}

impl<A> Delivery<A> {
    /// Whether the queue took the command.
    pub fn sent(&self) -> bool {
        matches!(self, Self::Sent)
    }
}

type SendFn<A> = dyn Fn(LogicCommand<A>) -> Delivery<A> + Send + Sync;

/// A view's sending end of the UI to Logic queue, for commands of type `A`.
///
/// Only tries, as every end the Main thread holds (`ARCH-021`).
pub struct Port<A> {
    send: Arc<SendFn<A>>,
}

impl<A> Clone for Port<A> {
    fn clone(&self) -> Self {
        Self {
            send: Arc::clone(&self.send),
        }
    }
}

impl<A: Send + 'static> Port<A> {
    /// A port over a queue that carries `A` itself, as a test rig's does.
    pub fn direct(to_logic: Sender<LogicCommand<A>>) -> Self {
        Self::embedded(to_logic)
    }

    /// A port over a queue that carries the sum `S`, which embeds `A`.
    pub fn embedded<S: Embed<A> + Send + 'static>(to_logic: Sender<LogicCommand<S>>) -> Self {
        Self {
            send: Arc::new(move |cmd: LogicCommand<A>| {
                match to_logic.try_send(cmd.map(S::embed)) {
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

impl<A> Port<A> {
    /// Offer `cmd` to the Logic thread, without waiting.
    pub fn try_send(&self, cmd: LogicCommand<A>) -> Delivery<A> {
        (self.send)(cmd)
    }
}

type RecvFn<E> = dyn Fn() -> Result<UiEvent<E>, TryRecvError> + Send;

/// A view's receiving end of the events the launcher forwards it, for
/// document events of type `E`.
pub struct Inbox<E> {
    recv: Box<RecvFn<E>>,
}

impl<E: Send + 'static> Inbox<E> {
    /// An inbox over a queue that carries `E` itself, as a test rig's does.
    pub fn direct(from_logic: Receiver<UiEvent<E>>) -> Self {
        Self::embedded(from_logic)
    }

    /// An inbox over a queue that carries the sum `S`, which embeds `E`.
    ///
    /// A document event of another application is skipped: the view could do
    /// nothing with it, as its own match on the sum did nothing before.
    pub fn embedded<S: Embed<E> + Send + 'static>(from_logic: Receiver<UiEvent<S>>) -> Self {
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

impl<E> Inbox<E> {
    /// The next event queued for the view, without waiting.
    ///
    /// # Errors
    ///
    /// [`TryRecvError::Empty`] when nothing is queued, and
    /// [`TryRecvError::Disconnected`] once the Logic thread is gone.
    pub fn try_recv(&self) -> Result<UiEvent<E>, TryRecvError> {
        (self.recv)()
    }
}
