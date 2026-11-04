//! The channels of the Main instance: between the page's main thread and the
//! Logic worker, and between two parts of the page.
//!
//! The browser refuses `Atomics.wait` on the page's main thread, and a lock
//! that finds itself contended ends in one. So no end the main thread holds
//! takes a lock (`ARCH-020`), and none of them can wait (`ARCH-021`). A
//! [`Sender`] only tries to push, a [`Receiver`] only tries to pop. The queue
//! under both is `crossbeam-queue`'s bounded `ArrayQueue`, which takes no lock.
//!
//! The Logic worker waits on a [`Doorbell`]. It binds the bell once, then parks
//! its own thread whenever both inbound channels are empty. Every push onto an
//! inbound channel unparks it (`ARCH-022`). Unparking is an `Atomics.notify`,
//! which the main thread may call.
//!
//! Three constructors, one per shape of conduit:
//! - [`to_worker`]: main thread -> Logic. The main thread tries, and a push
//!   rings the worker's bell.
//! - [`to_main`]: Logic -> main thread. The worker's [`WorkerSender`] parks
//!   while the queue is full, and a pop on the main thread wakes it
//!   (`ARCH-023`).
//! - [`local`]: a stream between two parts of the page. Nothing rings.
//! - [`unbounded`]: a stream inside one single-threaded worker, the Sync
//!   worker's. It never fills, so a [`UnboundedSender::send`] never waits, and
//!   the task on the other end awaits [`Receiver::recv`].
//!
//! Every push also wakes the task awaiting [`Receiver::recv`], through an
//! `AtomicWaker`: one atomic swap, no lock.

use std::collections::VecDeque;
use std::future::poll_fn;
use std::iter;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering, fence};
use std::sync::{Arc, OnceLock};
use std::task::Poll;
use std::thread::{self, Thread};

use ambre_core::{Bounded, Bounds, Count, CountKind, Refusal};
use atomic_waker::AtomicWaker;
use crossbeam_queue::{ArrayQueue, SegQueue};

/// The wake-up of one worker thread, shared by every channel that feeds it.
///
/// Created on the main thread before the worker is spawned, handed to the
/// constructors of the worker's channels, then bound by the worker itself.
/// Ringing a bell nobody bound yet does nothing: the worker drains its
/// channels after binding, before it first parks, so no message is missed.
#[derive(Clone, Debug, Default)]
pub struct Doorbell {
    thread: Arc<OnceLock<Thread>>,
}

impl Doorbell {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind the calling thread to this bell, and return the handle it waits
    /// with. `None` when another thread holds the bell already.
    pub fn bind(&self) -> Option<Waiter> {
        let current = thread::current();
        let id = current.id();
        let bound = self.thread.get_or_init(|| current);
        (bound.id() == id).then_some(Waiter {
            _not_send: PhantomData,
        })
    }

    /// Wake the bound thread. Never waits: `OnceLock::get` is one atomic
    /// load, and `Thread::unpark` one swap plus a notify.
    fn ring(&self) {
        if let Some(thread) = self.thread.get() {
            thread.unpark();
        }
    }

    /// Whether the calling thread is the one bound to this bell.
    fn is_current(&self) -> bool {
        self.thread
            .get()
            .is_some_and(|bound| bound.id() == thread::current().id())
    }
}

/// The right to wait on a [`Doorbell`], held by the one thread bound to it.
///
/// Neither `Send` nor `Sync`, so it never leaves that thread: the main thread
/// can never hold one, and so can never wait.
#[derive(Debug)]
pub struct Waiter {
    _not_send: PhantomData<*const ()>,
}

impl Waiter {
    /// Park until a channel rings the bell. It may return with nothing
    /// queued, so the caller re-checks its channels and waits again.
    pub fn wait(&self) {
        thread::park();
    }
}

/// A [`Sender::try_send`] that did not deliver: the value comes back.
#[derive(Debug, PartialEq, Eq)]
pub enum TrySendError<T> {
    /// The queue is at capacity.
    Full(T),
    /// The receiver is gone.
    Disconnected(T),
}

/// A [`WorkerSender::send`] that did not deliver: the value comes back.
#[derive(Debug, PartialEq, Eq)]
pub enum SendError<T> {
    /// The receiver is gone.
    Disconnected(T),
    /// The queue is full, and the calling thread is not the one bound to the
    /// channel's bell, so nothing would ever wake it.
    Unbound(T),
}

/// Why a [`Receiver::try_recv`] returned nothing.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum TryRecvError {
    /// Nothing queued, and a sender is still alive.
    Empty,
    /// Nothing queued, and every sender is gone.
    Disconnected,
}

/// Why a [`Receiver::recv`] returned nothing: every sender is gone, and
/// nothing is queued.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct RecvError;

/// A [`UnboundedSender::send`] that did not deliver, because the receiver is
/// gone: the value comes back.
#[derive(Debug, PartialEq, Eq)]
pub struct Disconnected<T>(T);

impl<T> Disconnected<T> {
    /// Wraps a raw value. The constructor checks nothing.
    pub const fn new(value: T) -> Self {
        Self(value)
    }

    /// Returns the wrapped value.
    pub fn into_inner(self) -> T {
        self.0
    }
}

/// The queue under one channel. Both take no lock.
enum Queue<T> {
    Bounded(ArrayQueue<T>),
    Unbounded(SegQueue<T>),
}

impl<T> Queue<T> {
    fn push(&self, value: T) -> Result<(), T> {
        match self {
            Self::Bounded(queue) => queue.push(value),
            Self::Unbounded(queue) => {
                queue.push(value);
                Ok(())
            }
        }
    }

    fn pop(&self) -> Option<T> {
        match self {
            Self::Bounded(queue) => queue.pop(),
            Self::Unbounded(queue) => queue.pop(),
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::Bounded(queue) => queue.len(),
            Self::Unbounded(queue) => queue.len(),
        }
    }

    fn is_empty(&self) -> bool {
        match self {
            Self::Bounded(queue) => queue.is_empty(),
            Self::Unbounded(queue) => queue.is_empty(),
        }
    }
}

/// The state both ends of one channel share.
struct Shared<T> {
    queue: Queue<T>,
    /// The task awaiting [`Receiver::recv`], woken on a push and on the last
    /// sender's drop.
    waker: AtomicWaker,
    /// Rung on a push and on the last sender's drop: the receiving worker's.
    receiver_bell: Option<Doorbell>,
    /// Rung on a pop that frees room, and on the receiver's drop: the sending
    /// worker's.
    sender_bell: Option<Doorbell>,
    senders: AtomicUsize,
    receiver_alive: AtomicBool,
    /// A worker sender is parked, waiting for room.
    sender_parked: AtomicBool,
}

impl<T> Shared<T> {
    fn new(
        capacity: Option<Bounded<ChannelSlots>>,
        receiver_bell: Option<Doorbell>,
        sender_bell: Option<Doorbell>,
    ) -> Self {
        Self {
            queue: match capacity {
                Some(capacity) => Queue::Bounded(ArrayQueue::new(
                    usize::try_from(capacity.get()).unwrap_or(usize::MAX),
                )),
                None => Queue::Unbounded(SegQueue::new()),
            },
            waker: AtomicWaker::new(),
            receiver_bell,
            sender_bell,
            senders: AtomicUsize::new(1),
            receiver_alive: AtomicBool::new(true),
            sender_parked: AtomicBool::new(false),
        }
    }

    fn try_push(&self, value: T) -> Result<(), TrySendError<T>> {
        if !self.receiver_alive.load(Ordering::Acquire) {
            return Err(TrySendError::Disconnected(value));
        }
        self.queue.push(value).map_err(TrySendError::Full)?;
        self.wake_receiver();
        Ok(())
    }

    fn wake_receiver(&self) {
        if let Some(bell) = &self.receiver_bell {
            bell.ring();
        }
        self.waker.wake();
    }

    fn try_pop(&self) -> Result<T, TryRecvError> {
        if let Some(value) = self.queue.pop() {
            self.freed_room();
            return Ok(value);
        }
        if self.senders.load(Ordering::Acquire) > 0 {
            return Err(TryRecvError::Empty);
        }
        // The last sender may have pushed between the pop and the load. Its
        // push happens before its drop, which the load just observed.
        let value = self.queue.pop().ok_or(TryRecvError::Disconnected)?;
        self.freed_room();
        Ok(value)
    }

    /// Wake a sender parked on a full queue. The fence pairs with the one in
    /// [`WorkerSender::send`]: either the sender's retry sees this pop's room,
    /// or this load sees its flag.
    fn freed_room(&self) {
        if let Some(bell) = &self.sender_bell {
            fence(Ordering::SeqCst);
            if self.sender_parked.swap(false, Ordering::SeqCst) {
                bell.ring();
            }
        }
    }

    fn add_sender(&self) {
        self.senders.fetch_add(1, Ordering::Relaxed);
    }

    /// The last sender gone reads as a disconnection, so its receiver must
    /// wake to see it.
    fn drop_sender(&self) {
        if self.senders.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.wake_receiver();
        }
    }
}

/// The sending end the main thread holds. It only tries.
pub struct Sender<T> {
    shared: Arc<Shared<T>>,
}

impl<T> Sender<T> {
    /// Push without waiting.
    ///
    /// # Errors
    ///
    /// [`TrySendError::Full`] when the queue is at capacity, and
    /// [`TrySendError::Disconnected`] when the receiver is gone. Either way the
    /// value comes back.
    pub fn try_send(&self, value: T) -> Result<(), TrySendError<T>> {
        self.shared.try_push(value)
    }

    /// The number of messages queued.
    pub fn len(&self) -> usize {
        self.shared.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.shared.queue.is_empty()
    }

    /// Whether the receiver is gone.
    pub fn is_disconnected(&self) -> bool {
        !self.shared.receiver_alive.load(Ordering::Acquire)
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        self.shared.add_sender();
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        self.shared.drop_sender();
    }
}

/// The sending end a worker holds. It may park while the queue is full.
pub struct WorkerSender<T> {
    shared: Arc<Shared<T>>,
}

impl<T> WorkerSender<T> {
    /// Push, parking while the queue is full until the receiver takes a
    /// message. Only the thread bound to the channel's bell may park.
    ///
    /// # Errors
    ///
    /// [`SendError::Disconnected`] when the receiver is gone, and
    /// [`SendError::Unbound`] when the queue is full and the calling thread is
    /// not the one bound to the bell. Either way the value comes back.
    pub fn send(&self, value: T) -> Result<(), SendError<T>> {
        let mut value = value;
        loop {
            match self.shared.try_push(value) {
                Ok(()) => return Ok(()),
                Err(TrySendError::Disconnected(v)) => return Err(SendError::Disconnected(v)),
                Err(TrySendError::Full(v)) => value = v,
            }
            if !self
                .shared
                .sender_bell
                .as_ref()
                .is_some_and(Doorbell::is_current)
            {
                return Err(SendError::Unbound(value));
            }
            // Raise the flag, then retry: a pop that ran before the flag was
            // visible freed the room this retry now finds.
            self.shared.sender_parked.store(true, Ordering::SeqCst);
            fence(Ordering::SeqCst);
            match self.shared.try_push(value) {
                Ok(()) => {
                    self.shared.sender_parked.store(false, Ordering::SeqCst);
                    return Ok(());
                }
                Err(TrySendError::Disconnected(v)) => return Err(SendError::Disconnected(v)),
                Err(TrySendError::Full(v)) => value = v,
            }
            thread::park();
        }
    }

    /// Push without waiting.
    ///
    /// # Errors
    ///
    /// [`TrySendError::Full`] when the queue is at capacity, and
    /// [`TrySendError::Disconnected`] when the receiver is gone. Either way the
    /// value comes back.
    pub fn try_send(&self, value: T) -> Result<(), TrySendError<T>> {
        self.shared.try_push(value)
    }
}

impl<T> Clone for WorkerSender<T> {
    fn clone(&self) -> Self {
        self.shared.add_sender();
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<T> Drop for WorkerSender<T> {
    fn drop(&mut self) {
        self.shared.drop_sender();
    }
}

/// The receiving end, on either side. It only tries: a worker waits on its
/// [`Doorbell`], never on a receiver.
pub struct Receiver<T> {
    shared: Arc<Shared<T>>,
}

impl<T> Receiver<T> {
    /// Pop without waiting.
    ///
    /// # Errors
    ///
    /// [`TryRecvError::Empty`] when nothing is queued and a sender is alive,
    /// and [`TryRecvError::Disconnected`] when nothing is queued and every
    /// sender is gone.
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        self.shared.try_pop()
    }

    /// Wait for the next message, as a task. The task parks nothing: it
    /// yields to its executor, and a push wakes it.
    ///
    /// # Errors
    ///
    /// [`RecvError`] once every sender is gone and nothing is queued.
    pub async fn recv(&self) -> Result<T, RecvError> {
        poll_fn(|cx| {
            match self.shared.try_pop() {
                Ok(value) => return Poll::Ready(Ok(value)),
                Err(TryRecvError::Disconnected) => return Poll::Ready(Err(RecvError)),
                Err(TryRecvError::Empty) => {}
            }
            // Register, then look again: a push between the first look and the
            // registration woke nobody, and this second look finds it.
            self.shared.waker.register(cx.waker());
            match self.shared.try_pop() {
                Ok(value) => Poll::Ready(Ok(value)),
                Err(TryRecvError::Disconnected) => Poll::Ready(Err(RecvError)),
                Err(TryRecvError::Empty) => Poll::Pending,
            }
        })
        .await
    }

    /// Every message queued, popped one by one until the queue is empty.
    pub fn try_iter(&self) -> impl Iterator<Item = T> + '_ {
        iter::from_fn(|| self.try_recv().ok())
    }

    /// The number of messages queued.
    pub fn len(&self) -> usize {
        self.shared.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.shared.queue.is_empty()
    }

    /// Whether every sender is gone. Messages may still be queued.
    pub fn is_disconnected(&self) -> bool {
        self.shared.senders.load(Ordering::Acquire) == 0
    }
}

impl<T> Drop for Receiver<T> {
    /// A sender parked on a full queue must wake to see the disconnection.
    fn drop(&mut self) {
        self.shared.receiver_alive.store(false, Ordering::SeqCst);
        if let Some(bell) = &self.shared.sender_bell {
            bell.ring();
        }
    }
}

/// What a lossless stream's [`Sender`] could not take yet, held in arrival
/// order on the main thread and offered again later (`BUS-025`). The main
/// thread may not wait for room, and a lossless message may not be dropped,
/// so this is the third way.
pub struct Backlog<T> {
    to: Sender<T>,
    held: VecDeque<T>,
    marks: Option<Marks>,
    /// The last request made of the source: it sends until asked to hold.
    asked: Pressure,
}

/// The two lengths between which a [`Backlog`] asks its source to stop. The
/// gap between them keeps one burst from flapping the source on and off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Marks {
    high: Count<Backlogged>,
    low: Count<Backlogged>,
}

impl Marks {
    /// Stop the source at `high` values held, and start it again at `low`.
    /// `None` unless `low` sits below `high`: the source would never be
    /// released. A `const fn`, so a pair checks at compile time, where `<`
    /// is not yet callable.
    #[must_use]
    pub const fn new(high: Count<Backlogged>, low: Count<Backlogged>) -> Option<Self> {
        if low.get() < high.get() {
            Some(Self { high, low })
        } else {
            None
        }
    }

    /// The length at which the source is asked to stop.
    #[must_use]
    pub const fn high(self) -> Count<Backlogged> {
        self.high
    }

    /// The length at which the source is asked to send again.
    #[must_use]
    pub const fn low(self) -> Count<Backlogged> {
        self.low
    }
}

/// The values a [`Backlog`] holds, as a [`Marks`] pair names a length:
/// `Count<Backlogged>`. `Debug` keeps the name of the hand-written type.
pub enum Backlogged {}

impl CountKind for Backlogged {
    type Repr = u32;
    const NAME: &'static str = "BacklogLength";
}

/// The values a bounded channel holds before a send finds it full:
/// `Bounded<ChannelSlots>`.
///
/// A queue holds at least one value, so the lowest capacity is one. The
/// queue allocates every slot when it is built, so the highest is 65 536.
/// That is 16 times the widest channel today, 4096 slots, and it bounds
/// the heap one channel takes. Every capacity is a constant, so it enters
/// through the `const` door, which refuses zero at compile time:
///
/// ```compile_fail
/// use ambre_bus::conduit::ChannelSlots;
/// use ambre_core::Bounded;
/// const NONE: Bounded<ChannelSlots> = Bounded::<ChannelSlots>::of::<0>();
/// ```
pub enum ChannelSlots {}

impl Bounds for ChannelSlots {
    type Repr = u32;
    const MIN: u32 = 1;
    const MAX: u32 = 1 << 16;
    const NAME: &'static str = "ChannelCapacity";
    const REFUSAL: Refusal = Refusal::new("a channel holds 1 to 65536 values, not ", "");
}

/// The backlog length of `factor` full channels of `capacity`, held at the
/// top of the count. A `const fn`, so a pair of [`Marks`] built from it
/// checks at compile time.
#[must_use]
pub const fn backlog(capacity: Bounded<ChannelSlots>, factor: u32) -> Count<Backlogged> {
    Count::new(capacity.get().saturating_mul(factor))
}

/// What a [`Backlog`] asks of the source that feeds it (`SYNC-059`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pressure {
    /// The backlog reached its high mark: stop sending.
    Hold,
    /// The backlog fell to its low mark: send again.
    Release,
}

impl<T> Backlog<T> {
    pub fn new(to: Sender<T>) -> Self {
        Self {
            to,
            held: VecDeque::new(),
            marks: None,
            asked: Pressure::Release,
        }
    }

    /// A backlog that asks its source to stop at the high mark, and to start
    /// again at the low one. Read the request with [`Backlog::pressure`].
    pub fn with_marks(to: Sender<T>, marks: Marks) -> Self {
        Self {
            marks: Some(marks),
            ..Self::new(to)
        }
    }

    /// What the source must do now, once per crossing of a mark. `None` when
    /// nothing changed since the last call, or when the backlog has no marks.
    pub fn pressure(&mut self) -> Option<Pressure> {
        let marks = self.marks?;
        let len = Count::<Backlogged>::of(self.held.len());
        let next = match self.asked {
            Pressure::Release if len >= marks.high => Pressure::Hold,
            Pressure::Hold if len <= marks.low => Pressure::Release,
            Pressure::Release | Pressure::Hold => return None,
        };
        self.asked = next;
        Some(next)
    }

    /// Queue `value` behind every value held before it, then offer them all.
    pub fn push(&mut self, value: T) {
        self.held.push_back(value);
        self.flush();
    }

    /// Offer the held values in order, until the channel is full. A receiver
    /// gone takes nothing ever again, so what waits is dropped.
    pub fn flush(&mut self) {
        while let Some(value) = self.held.pop_front() {
            match self.to.try_send(value) {
                Ok(()) => {}
                Err(TrySendError::Full(value)) => {
                    self.held.push_front(value);
                    return;
                }
                Err(TrySendError::Disconnected(_)) => {
                    self.held.clear();
                    return;
                }
            }
        }
    }

    /// The number of values held.
    pub fn len(&self) -> usize {
        self.held.len()
    }

    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }
}

/// The sending end of an [`unbounded`] channel. The queue never fills, so a
/// send never waits and only fails once the receiver is gone.
pub struct UnboundedSender<T> {
    shared: Arc<Shared<T>>,
}

impl<T> UnboundedSender<T> {
    /// Push, and wake the task awaiting the receiver.
    ///
    /// # Errors
    ///
    /// [`Disconnected`] when the receiver is gone. The value comes back.
    pub fn send(&self, value: T) -> Result<(), Disconnected<T>> {
        self.shared.try_push(value).map_err(|err| match err {
            TrySendError::Full(value) | TrySendError::Disconnected(value) => Disconnected(value),
        })
    }

    /// Whether the receiver is gone.
    pub fn is_disconnected(&self) -> bool {
        !self.shared.receiver_alive.load(Ordering::Acquire)
    }
}

impl<T> Clone for UnboundedSender<T> {
    fn clone(&self) -> Self {
        self.shared.add_sender();
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<T> Drop for UnboundedSender<T> {
    fn drop(&mut self) {
        self.shared.drop_sender();
    }
}

/// A channel from the main thread to the worker bound to `bell`.
pub fn to_worker<T>(capacity: Bounded<ChannelSlots>, bell: &Doorbell) -> (Sender<T>, Receiver<T>) {
    let shared = Arc::new(Shared::new(Some(capacity), Some(bell.clone()), None));
    (
        Sender {
            shared: Arc::clone(&shared),
        },
        Receiver { shared },
    )
}

/// A channel from the worker bound to `bell` to the main thread.
pub fn to_main<T>(
    capacity: Bounded<ChannelSlots>,
    bell: &Doorbell,
) -> (WorkerSender<T>, Receiver<T>) {
    let shared = Arc::new(Shared::new(Some(capacity), None, Some(bell.clone())));
    (
        WorkerSender {
            shared: Arc::clone(&shared),
        },
        Receiver { shared },
    )
}

/// A channel between two parts of the main thread.
pub fn local<T>(capacity: Bounded<ChannelSlots>) -> (Sender<T>, Receiver<T>) {
    let shared = Arc::new(Shared::new(Some(capacity), None, None));
    (
        Sender {
            shared: Arc::clone(&shared),
        },
        Receiver { shared },
    )
}

/// A channel with no capacity, inside one single-threaded worker. Its memory
/// grows with what the receiver has not taken yet, so it carries only what a
/// task drains as fast as it arrives.
pub fn unbounded<T>() -> (UnboundedSender<T>, Receiver<T>) {
    let shared = Arc::new(Shared::new(None, None, None));
    (
        UnboundedSender {
            shared: Arc::clone(&shared),
        },
        Receiver { shared },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    /// Long enough for a parked thread to have parked, short enough that a
    /// lost wake fails the test instead of hanging it.
    const PATIENCE: Duration = Duration::from_secs(5);

    #[test]
    fn messages_arrive_in_order() {
        let (tx, rx) = local(Bounded::<ChannelSlots>::of::<4>());
        for n in 0..4 {
            tx.try_send(n).expect("room");
        }
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), vec![0, 1, 2, 3]);
        assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));
    }

    #[test]
    fn a_full_channel_hands_the_value_back() {
        let (tx, rx) = local(Bounded::<ChannelSlots>::of::<1>());
        tx.try_send(1).expect("room");
        assert_eq!(tx.try_send(2), Err(TrySendError::Full(2)));
        assert_eq!(rx.try_recv(), Ok(1));
    }

    /// The lowest capacity holds one value, and no capacity holds none: a
    /// run-time zero is refused, as the `const` door refuses it at compile
    /// time.
    #[test]
    fn the_lowest_capacity_holds_one_and_zero_is_refused() {
        let (tx, _rx) = local(Bounded::<ChannelSlots>::MIN);
        assert_eq!(tx.try_send(1), Ok(()));
        assert_eq!(tx.try_send(2), Err(TrySendError::Full(2)));
        assert_eq!(Bounded::<ChannelSlots>::of::<1>(), Bounded::MIN);
        assert_eq!(Bounded::<ChannelSlots>::new(0), None);
        assert_eq!(Bounded::<ChannelSlots>::new(1 << 16), Some(Bounded::MAX));
        assert_eq!(Bounded::<ChannelSlots>::new((1 << 16) + 1), None);
        assert_eq!(
            format!("{:?}", Bounded::<ChannelSlots>::of::<8>()),
            "ChannelCapacity(8)"
        );
    }

    #[test]
    fn a_backlog_counts_full_channels_and_holds_at_the_top() {
        let capacity = Bounded::<ChannelSlots>::of::<1024>();
        assert_eq!(backlog(capacity, 4), Count::<Backlogged>::new(4096));
        assert_eq!(backlog(capacity, 1), Count::<Backlogged>::new(1024));
        assert_eq!(
            backlog(Bounded::<ChannelSlots>::MAX, u32::MAX),
            Count::<Backlogged>::new(u32::MAX)
        );
        assert_eq!(
            format!("{:?}", Count::<Backlogged>::new(3)),
            "BacklogLength(3)"
        );
    }

    /// A pair of marks whose low one does not sit below the high one would
    /// hold its source for good, and is refused.
    #[test]
    fn a_low_mark_must_sit_below_the_high_one() {
        assert_eq!(
            Marks::new(Count::<Backlogged>::new(3), Count::<Backlogged>::new(3)),
            None
        );
        assert_eq!(
            Marks::new(Count::<Backlogged>::new(1), Count::<Backlogged>::new(3)),
            None
        );
        let marks = Marks::new(Count::<Backlogged>::new(3), Count::<Backlogged>::new(1))
            .expect("1 sits below 3");
        assert_eq!(
            (marks.high(), marks.low()),
            (Count::<Backlogged>::new(3), Count::<Backlogged>::new(1))
        );
    }

    /// `ARCH-024`: the last sender gone reads as a disconnection, once the
    /// messages it queued are taken.
    #[test]
    fn the_last_sender_dropped_disconnects_after_its_messages() {
        let (tx, rx) = local(Bounded::<ChannelSlots>::of::<4>());
        let second = tx.clone();
        tx.try_send(1).expect("room");
        drop(tx);
        assert!(!rx.is_disconnected(), "a clone still holds the channel");
        drop(second);
        assert_eq!(rx.try_recv(), Ok(1));
        assert_eq!(rx.try_recv(), Err(TryRecvError::Disconnected));
    }

    #[test]
    fn a_dropped_receiver_refuses_the_send() {
        let (tx, rx) = local(Bounded::<ChannelSlots>::of::<4>());
        drop(rx);
        assert!(tx.is_disconnected());
        assert_eq!(tx.try_send(1), Err(TrySendError::Disconnected(1)));
    }

    #[test]
    fn a_second_thread_cannot_bind_a_bound_bell() {
        let bell = Doorbell::new();
        let _waiter = bell.bind().expect("first bind");
        let other = bell.clone();
        let bound = thread::spawn(move || other.bind().is_some())
            .join()
            .expect("joined");
        assert!(!bound);
    }

    /// `BUS-025`: what a full channel refuses is held, then delivered in
    /// arrival order once the receiver takes room. Nothing is dropped.
    #[test]
    fn a_backlog_holds_past_a_full_channel_and_keeps_the_order() {
        let (tx, rx) = local(Bounded::<ChannelSlots>::of::<2>());
        let mut backlog = Backlog::new(tx);
        for n in 0..5 {
            backlog.push(n);
        }
        assert_eq!(backlog.len(), 3, "two in the channel, three held");
        let mut read: Vec<_> = rx.try_iter().collect();
        backlog.flush();
        read.extend(rx.try_iter());
        backlog.flush();
        read.extend(rx.try_iter());
        assert_eq!(read, vec![0, 1, 2, 3, 4]);
        assert!(backlog.is_empty());
    }

    /// A value pushed while others are held waits behind them, even when the
    /// channel has room by then.
    #[test]
    fn a_backlog_never_lets_a_new_value_overtake_a_held_one() {
        let (tx, rx) = local(Bounded::<ChannelSlots>::of::<1>());
        let mut backlog = Backlog::new(tx);
        backlog.push(1);
        backlog.push(2);
        assert_eq!(rx.try_recv(), Ok(1));
        backlog.push(3);
        assert_eq!(rx.try_recv(), Ok(2));
        backlog.flush();
        assert_eq!(rx.try_recv(), Ok(3));
    }

    /// A backlog asks its source to stop once at the high mark, and to start
    /// again once at the low mark, never in between (`SYNC-059`).
    #[test]
    fn a_backlog_holds_its_source_at_the_high_mark_and_releases_it_at_the_low() {
        let (tx, rx) = local(Bounded::<ChannelSlots>::of::<1>());
        let mut backlog = Backlog::with_marks(
            tx,
            Marks::new(Count::<Backlogged>::new(3), Count::<Backlogged>::new(1))
                .expect("1 sits below 3"),
        );
        let mut asked = Vec::new();
        for n in 0..5 {
            backlog.push(n);
            asked.extend(backlog.pressure());
        }
        assert_eq!(backlog.len(), 4, "one in the channel, four held");
        assert_eq!(asked, vec![Pressure::Hold]);
        for _ in 0..3 {
            let _ = rx.try_recv();
            backlog.flush();
            asked.extend(backlog.pressure());
        }
        assert_eq!(backlog.len(), 1);
        assert_eq!(asked, vec![Pressure::Hold, Pressure::Release]);
        assert_eq!(backlog.pressure(), None, "one request per crossing");
    }

    #[test]
    fn a_backlog_drops_what_a_gone_receiver_will_never_take() {
        let (tx, rx) = local(Bounded::<ChannelSlots>::of::<1>());
        let mut backlog = Backlog::new(tx);
        backlog.push(1);
        backlog.push(2);
        drop(rx);
        backlog.flush();
        assert!(backlog.is_empty());
    }

    /// Spawn a worker that binds `bell`, reports it is bound, then waits
    /// until `rx` yields a message or disconnects, and hands back what it
    /// read.
    fn parked_reader(
        bell: Doorbell,
        rx: Receiver<u32>,
    ) -> (
        mpsc::Receiver<()>,
        mpsc::Receiver<Result<u32, TryRecvError>>,
    ) {
        let (bound_tx, bound_rx) = mpsc::channel();
        let (read_tx, read_rx) = mpsc::channel();
        thread::spawn(move || {
            let waiter = bell.bind().expect("bind");
            bound_tx.send(()).expect("report bound");
            let read = loop {
                match rx.try_recv() {
                    Err(TryRecvError::Empty) => waiter.wait(),
                    other => break other,
                }
            };
            read_tx.send(read).expect("report read");
        });
        (bound_rx, read_rx)
    }

    /// `ARCH-022`: a push from another thread wakes the worker parked on
    /// the channel's bell.
    #[test]
    fn a_send_wakes_the_parked_worker() {
        let bell = Doorbell::new();
        let (tx, rx) = to_worker(Bounded::<ChannelSlots>::of::<4>(), &bell);
        let (bound, read) = parked_reader(bell, rx);
        bound.recv_timeout(PATIENCE).expect("bound");
        thread::sleep(Duration::from_millis(20));
        tx.try_send(7).expect("room");
        assert_eq!(read.recv_timeout(PATIENCE), Ok(Ok(7)));
    }

    /// `ARCH-024`: the last sender dropped wakes the parked worker, which
    /// reads the disconnection.
    #[test]
    fn the_last_sender_dropped_wakes_the_parked_worker() {
        let bell = Doorbell::new();
        let (tx, rx) = to_worker::<u32>(Bounded::<ChannelSlots>::of::<4>(), &bell);
        let (bound, read) = parked_reader(bell, rx);
        bound.recv_timeout(PATIENCE).expect("bound");
        thread::sleep(Duration::from_millis(20));
        drop(tx);
        assert_eq!(
            read.recv_timeout(PATIENCE),
            Ok(Err(TryRecvError::Disconnected))
        );
    }

    /// `ARCH-023`: a worker send on a full channel parks, and completes once
    /// the main side takes a message. Nothing is dropped, and the order holds.
    #[test]
    fn a_full_send_resumes_when_the_receiver_takes_one() {
        let bell = Doorbell::new();
        let (tx, rx) = to_main(Bounded::<ChannelSlots>::of::<2>(), &bell);
        let (done_tx, done_rx) = mpsc::channel();
        thread::spawn(move || {
            let _waiter = bell.bind().expect("bind");
            let sent: Vec<_> = (0..5).map(|n| tx.send(n)).collect();
            done_tx.send(sent).expect("report");
        });
        let mut read = Vec::new();
        while read.len() < 5 {
            match rx.try_recv() {
                Ok(n) => read.push(n),
                Err(TryRecvError::Empty) => thread::sleep(Duration::from_millis(5)),
                Err(TryRecvError::Disconnected) => break,
            }
        }
        assert_eq!(read, vec![0, 1, 2, 3, 4]);
        let sent = done_rx.recv_timeout(PATIENCE).expect("the sender finished");
        assert!(sent.iter().all(Result::is_ok));
    }

    /// A receiver dropped while the worker is parked on a full channel wakes
    /// it with the value it could not deliver.
    #[test]
    fn a_dropped_receiver_wakes_a_parked_sender() {
        let bell = Doorbell::new();
        let (tx, rx) = to_main(Bounded::<ChannelSlots>::of::<1>(), &bell);
        let (parked_tx, parked_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        thread::spawn(move || {
            let _waiter = bell.bind().expect("bind");
            tx.send(1).expect("room");
            parked_tx.send(()).expect("report");
            done_tx.send(tx.send(2)).expect("report");
        });
        parked_rx.recv_timeout(PATIENCE).expect("first send");
        thread::sleep(Duration::from_millis(20));
        drop(rx);
        assert_eq!(
            done_rx.recv_timeout(PATIENCE),
            Ok(Err(SendError::Disconnected(2)))
        );
    }

    /// A full send from a thread the bell does not know returns instead of
    /// parking forever.
    #[test]
    fn a_full_send_off_the_bound_thread_hands_the_value_back() {
        let bell = Doorbell::new();
        let (tx, _rx) = to_main(Bounded::<ChannelSlots>::of::<1>(), &bell);
        tx.send(1).expect("room");
        assert_eq!(tx.send(2), Err(SendError::Unbound(2)));
    }

    /// Counts its wakes, so a test polls a future by hand, with no executor.
    struct CountingWaker(std::sync::atomic::AtomicUsize);

    impl std::task::Wake for CountingWaker {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn poll_once<F: std::future::Future>(
        future: std::pin::Pin<&mut F>,
        wakes: &Arc<CountingWaker>,
    ) -> Poll<F::Output> {
        let waker = std::task::Waker::from(Arc::clone(wakes));
        future.poll(&mut std::task::Context::from_waker(&waker))
    }

    #[test]
    fn a_push_wakes_the_awaiting_task() {
        let (tx, rx) = unbounded();
        let wakes = Arc::new(CountingWaker(AtomicUsize::new(0)));
        let mut recv = std::pin::pin!(rx.recv());
        assert!(poll_once(recv.as_mut(), &wakes).is_pending());

        tx.send(7).expect("receiver alive");

        assert_eq!(wakes.0.load(Ordering::SeqCst), 1);
        assert_eq!(poll_once(recv.as_mut(), &wakes), Poll::Ready(Ok(7)));
    }

    #[test]
    fn the_last_sender_dropped_ends_the_awaiting_task() {
        let (tx, rx) = unbounded::<u8>();
        let wakes = Arc::new(CountingWaker(AtomicUsize::new(0)));
        let mut recv = std::pin::pin!(rx.recv());
        assert!(poll_once(recv.as_mut(), &wakes).is_pending());

        drop(tx);

        assert_eq!(wakes.0.load(Ordering::SeqCst), 1);
        assert_eq!(
            poll_once(recv.as_mut(), &wakes),
            Poll::Ready(Err(RecvError))
        );
    }

    #[test]
    fn an_unbounded_channel_never_fills() {
        let (tx, rx) = unbounded();
        for n in 0..10_000 {
            tx.send(n).expect("receiver alive");
        }
        assert!(rx.try_iter().eq(0..10_000));
        drop(rx);
        assert_eq!(tx.send(1), Err(Disconnected(1)));
    }
}
