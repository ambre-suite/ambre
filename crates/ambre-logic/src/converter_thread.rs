// ambre-logic/src/converter_thread.rs
// ============================================================================
// The converter thread. It is a third thread of the Main instance, beside
// the UI and Logic threads. It runs the conversions the Logic thread must
// never wait on (`ambre-spec-architecture.md section 2`, the converter
// thread).
//
// Today it runs the JPEG decode of the images module (`ARCH-097`) and the
// raster of a Toile PNG export (`toile-import-export`). It also reads and
// writes a Prose file (`ARCH-122`) and a Trame file in CSV, Excel or
// OpenDocument (`ARCH-123`), and writes every PDF export (`ARCH-124`). The
// other conversions still run on the Logic thread, and the
// `Every conversion runs off the Logic thread` Story moves them. So the two
// envelopes carry any `Conversion`, never a format of their own.
//
// The Logic thread sends a job and never waits for its answer. The answer
// rings the Logic thread's bell, and the next pass hands it to the document
// or the channel that asked (`ARCH-098`, `ARCH-101`). The samples cross as a `Vec` on the
// shared heap: the hop between the two threads copies no byte.
//
// A module that traps in a job answers it as unavailable, and its instance
// loads again (`ARCH-107`). A thread that stops answering is caught by a
// deadline on the Logic thread's side, which fails its jobs and spawns
// another thread (`ARCH-108`).
// ============================================================================

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::Duration;

use ambre_bus::Request;
use ambre_bus::conduit::{self, ChannelSlots, Doorbell, Receiver, Sender, TrySendError};
use ambre_convert::{Conversion, ConvertError, Converted, Converter};
use ambre_core::NarrowCount;
use ambre_core::id::{Channel, Document, Id};
use ambre_core::{
    Bounded, ConverterFamily, ConverterModule, Count, FracMs, Instant, Octets, PageLoad, Payload,
    Seq, TimeSpan,
};

/// What the Logic thread asks of the converter thread.
pub enum ToConverter {
    /// The checked bytes of a family's module (`ARCH-080`). The thread
    /// compiles its own instance: an instance is never shared between threads.
    Load {
        family: ConverterFamily,
        module: Payload<ConverterModule>,
    },
    /// One conversion. Answered once, by [`FromConverter`] under the same
    /// ticket.
    Run {
        ticket: Seq<Request>,
        conversion: Conversion,
    },
}

/// The answer of one [`ToConverter::Run`].
pub struct FromConverter {
    pub ticket: Seq<Request>,
    pub answer: Result<Converted, ConvertError>,
}

/// The jobs the Logic thread holds in flight on the converter thread at once.
/// Past it a job waits in the Logic thread's backlog, so no channel fills
/// (`ambre-spec-budgets.md section 6`).
pub(crate) const IN_FLIGHT: Count<JobsInFlight> = Count::new(8);

/// The jobs in flight on the converter thread.
pub(crate) enum JobsInFlight {}

impl NarrowCount for JobsInFlight {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("JobsInFlightCount");
}

/// The slots of each channel: the jobs in flight, and one load per family.
/// The thread answers each job once, so its answers never pass the bound.
const SLOTS: Bounded<ChannelSlots> = Bounded::<ChannelSlots>::of::<16>();

/// The memory a module may span after a job before the thread drops it and
/// loads it again. A WebAssembly memory never shrinks: a decode of a large
/// photograph would otherwise hold its peak for the session (`ARCH-099`,
/// `ambre-spec-budgets.md section 6`).
pub const RESET_FOOTPRINT: Count<Octets> = Count::new(64 * 1024 * 1024);

/// How long the converter thread may work on a job of `family` without
/// answering before the Logic thread holds it lost (`ARCH-108`, `ARCH-127`).
/// A measured figure is three times the dearest module call of the family
/// on the largest input it admits, at the phone rate of
/// `ambre-spec-budgets.md section 8`. No figure falls under
/// [`FLOOR_DEADLINE`]. The deadline runs from the latest answer, so a queue
/// of slow jobs that each answer never trips it.
#[must_use]
pub const fn answer_deadline(family: ConverterFamily) -> TimeSpan<FracMs> {
    match family {
        ConverterFamily::Images => IMAGES_DEADLINE,
        ConverterFamily::Office => OFFICE_DEADLINE,
        ConverterFamily::Pdf => PDF_DEADLINE,
        // The markup family measures far under the floor. A CSV file has no
        // bound to measure on, so the tables family keeps the floor
        // unmeasured. So does the boards family, which no bench drives.
        ConverterFamily::Markup | ConverterFamily::Tables | ConverterFamily::Boards => {
            FLOOR_DEADLINE
        }
    }
}

/// The answer deadline of the images family: a JPEG at the source bound.
const IMAGES_DEADLINE: TimeSpan<FracMs> = TimeSpan::new(45_000.0);

/// The answer deadline of the Office family: a sheet written at the part
/// bound.
const OFFICE_DEADLINE: TimeSpan<FracMs> = TimeSpan::new(40_000.0);

/// The answer deadline of the PDF family: a document and the pictures an
/// export admits.
const PDF_DEADLINE: TimeSpan<FracMs> = TimeSpan::new(35_000.0);

/// The shortest answer deadline. A deadline catches a thread that stopped,
/// which is rare, so a shorter one buys little and risks failing a job that
/// still works.
const FLOOR_DEADLINE: TimeSpan<FracMs> = TimeSpan::new(30_000.0);

/// The spawns the Logic thread tries again in one session, past the first
/// (`ambre-spec-budgets.md section 6`). A missed deadline asks one
/// (`ARCH-108`), and so does a job after a spawn that failed (`ARCH-118`). A thread held lost is
/// never ended: its worker and its stack stay for the session.
pub(crate) const RESPAWN_LIMIT: Count<Respawns> = Count::new(3);

/// The converter spawns tried again after a missed deadline or a failed
/// spawn.
pub(crate) enum Respawns {}

impl NarrowCount for Respawns {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("RespawnCount");
}

/// The Logic thread's end of the converter thread.
pub struct ConverterLink {
    to: Sender<ToConverter>,
    from: Receiver<FromConverter>,
}

/// The converter thread's ends, handed to [`run_converter_thread`].
pub struct ConverterThreadEnds {
    bell: Doorbell,
    from_logic: Receiver<ToConverter>,
    to_logic: Sender<FromConverter>,
}

/// How the Logic thread spawns another converter thread once one stops
/// answering (`ARCH-108`). The page builds it over the platform's threads,
/// spawns the first thread with it, and hands it to the Logic thread. It
/// gives back the Logic thread's end of the new thread, or `None` when the
/// platform refused the spawn: the spawner logs the reason.
pub struct ConverterSpawner(Box<dyn Fn() -> Option<ConverterLink> + Send>);

impl ConverterSpawner {
    /// The spawner that runs `spawn`.
    pub fn new(spawn: impl Fn() -> Option<ConverterLink> + Send + 'static) -> Self {
        Self(Box::new(spawn))
    }

    /// Spawn a converter thread, and give back the Logic thread's end of it.
    #[must_use]
    pub fn spawn(&self) -> Option<ConverterLink> {
        (self.0)()
    }

    /// The spawner the page builds over `wasm_thread`: each thread is a
    /// worker of the Main instance, and its answers ring `logic_bell`. A spawn
    /// from the Logic worker posts to the page's thread, which creates the
    /// worker, so the page's thread must run its event loop.
    #[cfg(target_arch = "wasm32")]
    #[must_use]
    pub fn over_threads(logic_bell: Doorbell) -> Self {
        Self::new(move || {
            let (link, ends) = converter_channels(&logic_bell);
            match wasm_thread::Builder::new()
                .name("converter".to_owned())
                .spawn(move || run_converter_thread(ends))
            {
                Ok(_) => Some(link),
                Err(error) => {
                    tracing::error!("the converter thread did not start: {error}");
                    None
                }
            }
        })
    }
}

/// The spawner of the converter threads, handed to the Logic thread at boot.
/// The boot spawns no thread: the first job spawns the first one
/// (`ARCH-112`), so a session that converts nothing never pays a worker.
pub struct ConverterThreads {
    spawner: ConverterSpawner,
}

impl ConverterThreads {
    /// Keep `spawner` for the first thread and the next ones. A spawn that
    /// fails is no fatal error (`ARCH-100`).
    #[must_use]
    pub fn on_demand(spawner: ConverterSpawner) -> Self {
        Self { spawner }
    }
}

/// The two channels between the Logic thread and the converter thread. An
/// answer rings `logic_bell`, so a parked Logic thread wakes for it.
#[must_use]
pub fn converter_channels(logic_bell: &Doorbell) -> (ConverterLink, ConverterThreadEnds) {
    let bell = Doorbell::new();
    let (to, from_logic) = conduit::to_worker(SLOTS, &bell);
    let (to_logic, from) = conduit::to_worker(SLOTS, logic_bell);
    (
        ConverterLink { to, from },
        ConverterThreadEnds {
            bell,
            from_logic,
            to_logic,
        },
    )
}

// -- The thread --------------------------------------------------------------

/// The converter thread's state: one instance per family it was handed.
pub struct ConverterThread {
    from_logic: Receiver<ToConverter>,
    to_logic: Sender<FromConverter>,
    modules: BTreeMap<ConverterFamily, Held>,
}

/// A family's module bytes, kept to load the instance again, and the
/// instance, or why it did not load.
struct Held {
    module: Payload<ConverterModule>,
    converter: Result<Converter, ConvertError>,
}

impl ConverterThread {
    /// The thread's state over `ends`, without its bell: a test serves it by
    /// hand.
    #[must_use]
    pub fn new(ends: ConverterThreadEnds) -> (Self, Doorbell) {
        let ConverterThreadEnds {
            bell,
            from_logic,
            to_logic,
        } = ends;
        (
            Self {
                from_logic,
                to_logic,
                modules: BTreeMap::new(),
            },
            bell,
        )
    }

    /// Serve every message queued now, in order. A load comes before the
    /// jobs that need it, so a job never meets a module still on its way.
    pub fn serve(&mut self) {
        let queued = self.from_logic.len();
        for _ in 0..queued.get() {
            let Ok(message) = self.from_logic.try_recv() else {
                return;
            };
            match message {
                ToConverter::Load { family, module } => {
                    let converter = Converter::load(family, &module);
                    self.modules.insert(family, Held { module, converter });
                }
                ToConverter::Run { ticket, conversion } => {
                    let answer = self.run(&conversion);
                    // The Logic thread holds at most `IN_FLIGHT` jobs, so the
                    // channel holds every answer. A Logic thread gone takes
                    // its answers with it.
                    if let Err(TrySendError::Full(_)) =
                        self.to_logic.try_send(FromConverter { ticket, answer })
                    {
                        tracing::error!(
                            "the converter thread dropped an answer: its channel is full"
                        );
                    }
                }
            }
        }
    }

    /// Run `conversion` on its family's instance, then drop an instance that
    /// trapped or whose memory grew past [`RESET_FOOTPRINT`], and load it
    /// again.
    fn run(&mut self, conversion: &Conversion) -> Result<Converted, ConvertError> {
        let family = conversion.family();
        let held = self.modules.get_mut(&family).ok_or(ConvertError::Absent)?;
        let converter = held.converter.as_ref().map_err(Clone::clone)?;
        let answer = converter.convert(conversion);
        if needs_reload(&answer, converter.footprint()) {
            held.converter = Converter::load(family, &held.module);
        }
        answer
    }

    /// The span of `family`'s instance now. The browser's tests alone.
    #[cfg(all(test, target_arch = "wasm32"))]
    fn footprint(&self, family: ConverterFamily) -> Option<Count<Octets>> {
        self.modules
            .get(&family)
            .and_then(|held| held.converter.as_ref().ok())
            .and_then(Converter::footprint)
    }

    /// Whether every sender of the Logic thread is gone and nothing waits.
    fn is_over(&self) -> bool {
        self.from_logic.is_empty() && self.from_logic.is_disconnected()
    }
}

/// Whether an instance whose memory spans `footprint` is dropped and loaded
/// again (`ARCH-099`). The host runs the engine in its own heap, and reports
/// no footprint.
#[must_use]
pub fn needs_reset(footprint: Option<Count<Octets>>) -> bool {
    footprint.is_some_and(|bytes| bytes > RESET_FOOTPRINT)
}

/// Whether the instance that gave `answer`, and whose memory spans
/// `footprint` after it, is dropped and loaded again. A module that stopped
/// mid-job, a trap above all, may have left its memory half written, so the
/// next job never runs on it (`ARCH-107`). A memory past the bound is the
/// other case (`ARCH-099`).
#[must_use]
pub fn needs_reload(
    answer: &Result<Converted, ConvertError>,
    footprint: Option<Count<Octets>>,
) -> bool {
    matches!(answer, Err(ConvertError::Stopped(_))) || needs_reset(footprint)
}

/// The Logic thread's monotonic clock, which the answer deadline reads. In
/// the browser it is the worker's `performance.now()`; a worker without one
/// reads zero for ever, and the deadline never trips.
#[cfg(target_arch = "wasm32")]
pub(crate) fn now() -> Instant<PageLoad> {
    crate::pass_clock().unwrap_or_default()
}

/// The Logic thread's monotonic clock, which the answer deadline reads: on
/// the host, the time since the first read.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn now() -> Instant<PageLoad> {
    static ORIGIN: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let origin = *ORIGIN.get_or_init(std::time::Instant::now);
    Instant::after_epoch(TimeSpan::from_duration(origin.elapsed()))
}

/// Run the converter thread to completion: serve each job in order, then park
/// on its bell. It exits once the Logic thread dropped its end.
///
/// A thread that cannot bind its bell answers nothing. The Logic thread then
/// sees its jobs unanswered, and its link reports the thread gone once the
/// ends drop here (`ARCH-100`).
pub fn run_converter_thread(ends: ConverterThreadEnds) {
    let (mut thread, bell) = ConverterThread::new(ends);
    let Some(waiter) = bell.bind() else {
        tracing::error!("the converter thread's bell is bound to another thread");
        return;
    };
    tracing::info!("Ambre converter thread starting");
    loop {
        thread.serve();
        if thread.is_over() {
            tracing::info!("Ambre converter thread exiting");
            return;
        }
        if thread.from_logic.is_empty() {
            waiter.wait();
        }
    }
}

// -- The Logic thread's side -----------------------------------------------------

/// Who asked for a job, so its answer reaches it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum JobOwner {
    /// A picture of a Prose document, by the ticket Prose minted.
    Prose(Id<Document>),
    /// The read of an import or the write of an export of a Prose document,
    /// by the ticket Prose minted (`ARCH-122`).
    ProseFile(Id<Document>),
    /// The read of an import or the write of an export of a Trame workbook,
    /// by the ticket Trame minted (`ARCH-123`).
    TrameFile(Id<Document>),
    /// A picture a Toile board inserts, by the ticket Toile minted.
    Toile(Id<Document>),
    /// The PNG export of a Toile board, answered as its file
    /// (`toile-import-export`).
    ToilePng(Id<Document>),
    /// The write of a Toile deck's PDF export, by the ticket this thread
    /// minted, and the report the file answers with (`ARCH-124`).
    DeckPdf {
        document_id: Id<Document>,
        report: Box<ambre_toile::message::ToileReport>,
    },
    /// The preview of a Murmure message.
    Murmure {
        channel_id: Id<Channel>,
        media_ref: ambre_core::Validated<ambre_bus::ContentHash>,
    },
}

/// A job in flight on the converter thread: its ticket, who asked, and the
/// family whose deadline it runs under.
struct Flight {
    ticket: Seq<Request>,
    owner: JobOwner,
    family: ConverterFamily,
}

/// Which answer deadline the jobs in flight run under.
#[derive(Clone, Copy, Debug)]
enum Deadline {
    /// The deadline of the family of the oldest job in flight
    /// ([`answer_deadline`]).
    PerFamily,
    /// One deadline for every family: a host test's.
    #[cfg(all(test, any(not(target_arch = "wasm32"), target_feature = "atomics")))]
    Fixed(TimeSpan<FracMs>),
}

impl Deadline {
    /// The deadline a job of `family` runs under.
    fn of(self, family: ConverterFamily) -> TimeSpan<FracMs> {
        match self {
            Self::PerFamily => answer_deadline(family),
            #[cfg(all(test, any(not(target_arch = "wasm32"), target_feature = "atomics")))]
            Self::Fixed(span) => span,
        }
    }
}

/// A job the Logic thread holds back: its module is not here yet, or the
/// thread holds [`IN_FLIGHT`] jobs already.
pub(crate) struct Job {
    pub(crate) ticket: Seq<Request>,
    pub(crate) owner: JobOwner,
    pub(crate) conversion: Conversion,
}

pub(crate) use ambre_convert::JobFailure;

/// The failure an error of the converter means to its owner. The words of
/// the error reach the log here, and nowhere else.
pub(crate) fn failure_of(error: &ConvertError) -> JobFailure {
    match error {
        ConvertError::Refused(words) => {
            tracing::debug!("the converter refused a job: {words}");
            JobFailure::Refused
        }
        other => {
            tracing::debug!("a job of the converter thread failed: {other}");
            JobFailure::Unavailable
        }
    }
}

/// Whether the first converter thread is still owed to the first job
/// (`ARCH-112`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FirstSpawn {
    /// No job asked for a thread yet.
    Owed,
    /// The spawner was asked once, whatever it gave back.
    Asked,
}

/// The Logic thread's view of the converter thread. It holds the link, the
/// jobs in flight in the order they left, the backlog and the families the
/// thread holds. It also holds the answer deadline: the instant of the latest
/// progress, and the spawner of the first thread and of the next
/// (`ARCH-108`, `ARCH-112`).
pub(crate) struct Offload {
    link: Option<ConverterLink>,
    first: FirstSpawn,
    /// The thread serves its jobs in the order they left, so the first one is
    /// the job it works on (`ARCH-127`).
    in_flight: VecDeque<Flight>,
    backlog: VecDeque<Job>,
    loaded: BTreeSet<ConverterFamily>,
    /// The latest answer, or the first pass that saw a job in flight on an
    /// idle thread. `None` while nothing is in flight.
    progress: Option<Instant<PageLoad>>,
    deadline: Deadline,
    spawner: Option<ConverterSpawner>,
    respawns: Count<Respawns>,
}

impl Default for Offload {
    fn default() -> Self {
        Self {
            link: None,
            first: FirstSpawn::Asked,
            in_flight: VecDeque::new(),
            backlog: VecDeque::new(),
            loaded: BTreeSet::new(),
            progress: None,
            deadline: Deadline::PerFamily,
            spawner: None,
            respawns: Count::new(0),
        }
    }
}

impl Offload {
    /// The view over `link`, which spawns no other thread.
    #[cfg(test)]
    pub(crate) fn over(link: ConverterLink) -> Self {
        Self {
            link: Some(link),
            ..Self::default()
        }
    }

    /// The view that spawns its first thread on the first job, with the
    /// spawner of `threads` (`ARCH-112`). Once a thread misses the deadline,
    /// the same spawner spawns the next.
    pub(crate) fn started(threads: ConverterThreads) -> Self {
        Self {
            first: FirstSpawn::Owed,
            spawner: Some(threads.spawner),
            ..Self::default()
        }
    }

    /// Spawn a converter thread when a job needs one and none runs. The
    /// first spawn counts against no respawn limit (`ARCH-112`). A spawn that
    /// failed, the first or a respawn, is tried again by the next job, and
    /// each try counts against [`RESPAWN_LIMIT`] (`ARCH-118`). A job with no
    /// thread fails as a module that did not load (`ARCH-100`).
    pub(crate) fn start(&mut self) {
        if self.link.is_some() {
            return;
        }
        let Some(spawner) = self.spawner.as_ref() else {
            return;
        };
        match self.first {
            FirstSpawn::Owed => self.first = FirstSpawn::Asked,
            FirstSpawn::Asked if self.respawns < RESPAWN_LIMIT => {
                tracing::warn!("the converter thread is spawned again: the last spawn failed");
                self.respawns = self.respawns.plus(Count::new(1));
            }
            FirstSpawn::Asked => return,
        }
        self.link = spawner.spawn();
    }

    /// The same view, which spawns the next thread with `spawner`.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(crate) fn respawning(self, spawner: ConverterSpawner) -> Self {
        Self {
            spawner: Some(spawner),
            ..self
        }
    }

    /// The same view, under one answer deadline for every family. The
    /// browser's tests reach it on the threaded build alone.
    #[cfg(all(test, any(not(target_arch = "wasm32"), target_feature = "atomics")))]
    pub(crate) fn with_deadline(self, deadline: TimeSpan<FracMs>) -> Self {
        Self {
            deadline: Deadline::Fixed(deadline),
            ..self
        }
    }

    /// Whether the thread was handed `family`'s module.
    pub(crate) fn holds(&self, family: ConverterFamily) -> bool {
        self.loaded.contains(&family)
    }

    /// Hand the thread `family`'s module.
    ///
    /// # Errors
    ///
    /// The reason the thread cannot take it: it is gone, or there is none.
    pub(crate) fn load(
        &mut self,
        family: ConverterFamily,
        module: Payload<ConverterModule>,
    ) -> Result<(), ConvertError> {
        let link = self.link.as_ref().ok_or_else(gone)?;
        match link.to.try_send(ToConverter::Load { family, module }) {
            Ok(()) => {
                self.loaded.insert(family);
                Ok(())
            }
            Err(_) => Err(gone()),
        }
    }

    /// Send `job` now, or keep it in the backlog while [`IN_FLIGHT`] jobs
    /// run.
    ///
    /// # Errors
    ///
    /// The job's ticket and owner, with the reason, when the thread is gone.
    pub(crate) fn send(&mut self, job: Job) -> Result<(), (Seq<Request>, JobOwner, ConvertError)> {
        if Count::<JobsInFlight>::of(self.in_flight.len()) >= IN_FLIGHT {
            self.backlog.push_back(job);
            return Ok(());
        }
        let Job {
            ticket,
            owner,
            conversion,
        } = job;
        let Some(link) = self.link.as_ref() else {
            return Err((ticket, owner, gone()));
        };
        let family = conversion.family();
        match link.to.try_send(ToConverter::Run { ticket, conversion }) {
            Ok(()) => {
                self.in_flight.push_back(Flight {
                    ticket,
                    owner,
                    family,
                });
                Ok(())
            }
            Err(_) => Err((ticket, owner, gone())),
        }
    }

    /// Whether no job is in flight and none waits in the backlog.
    #[cfg(test)]
    pub(crate) fn is_idle(&self) -> bool {
        self.in_flight.is_empty() && self.backlog.is_empty()
    }

    /// Whether an answer waits.
    pub(crate) fn has_answers(&self) -> bool {
        self.link.as_ref().is_some_and(|link| !link.from.is_empty())
    }

    /// Whether the thread is gone while jobs wait on it.
    fn is_lost(&self) -> bool {
        self.link
            .as_ref()
            .is_none_or(|link| link.to.is_disconnected())
    }

    /// The answers queued now, each with the owner that asked, read at
    /// `now`. Every job in flight fails when the thread is gone. It fails
    /// too when the thread answered nothing for the deadline: that thread is
    /// held lost, and the next one is spawned (`ARCH-108`). The backlog moves
    /// up into the room the answers freed.
    pub(crate) fn take_answers(
        &mut self,
        now: Instant<PageLoad>,
    ) -> Vec<(Seq<Request>, JobOwner, Result<Converted, ConvertError>)> {
        let mut out = Vec::new();
        if let Some(link) = self.link.as_ref() {
            for FromConverter { ticket, answer } in link.from.try_iter_now() {
                let flight = self.in_flight.iter().position(|f| f.ticket == ticket);
                if let Some(Flight { owner, .. }) = flight.and_then(|at| self.in_flight.remove(at))
                {
                    out.push((ticket, owner, answer));
                }
            }
        }
        if self.is_lost() && !self.in_flight.is_empty() {
            out.extend(
                self.in_flight
                    .drain(..)
                    .map(|Flight { ticket, owner, .. }| (ticket, owner, Err(gone()))),
            );
        }
        if self.in_flight.is_empty() {
            self.progress = None;
        } else if !out.is_empty() || self.progress.is_none() {
            self.progress = Some(now);
        } else if self.is_due(now) {
            tracing::warn!("the converter thread answered nothing before its deadline");
            out.extend(
                self.in_flight
                    .drain(..)
                    .map(|Flight { ticket, owner, .. }| (ticket, owner, Err(ConvertError::Absent))),
            );
            self.respawn();
        }
        out
    }

    /// Whether a pass is due for the jobs in flight, read at `now`. Either
    /// the thread answered none of them for the deadline, or no pass dated
    /// them yet.
    pub(crate) fn is_due(&self, now: Instant<PageLoad>) -> bool {
        self.current_deadline()
            .is_some_and(|deadline| self.progress.is_none_or(|since| now - since >= deadline))
    }

    /// The deadline of the job the thread works on: the oldest in flight,
    /// under the deadline of its family (`ARCH-127`). `None` while nothing
    /// is in flight.
    fn current_deadline(&self) -> Option<TimeSpan<FracMs>> {
        self.in_flight
            .front()
            .map(|flight| self.deadline.of(flight.family))
    }

    /// How long the Logic thread may park before the deadline trips, read at
    /// `now`. `None` while nothing is in flight: it may park for good.
    pub(crate) fn wake_in(&self, now: Instant<PageLoad>) -> Option<Duration> {
        let deadline = self.current_deadline()?;
        let left = self
            .progress
            .map_or(TimeSpan::ZERO, |since| deadline - (now - since));
        // A span past the deadline, negative or not a number, wakes at once.
        Some(Duration::try_from_secs_f64(left.get() / 1000.0).unwrap_or(Duration::ZERO))
    }

    /// Hold the thread lost: drop its link, so a thread that comes back finds
    /// its queues closed and ends, and spawn the next one. The new thread
    /// holds no module, so the next job of each family loads it again. Past
    /// [`RESPAWN_LIMIT`] no thread is spawned, and every job fails at once
    /// (`ARCH-100`).
    fn respawn(&mut self) {
        self.link = None;
        self.loaded.clear();
        self.progress = None;
        if self.respawns >= RESPAWN_LIMIT {
            tracing::warn!("the converter thread is not spawned again: its limit is reached");
            return;
        }
        if let Some(spawner) = self.spawner.as_ref() {
            self.respawns = self.respawns.plus(Count::new(1));
            self.link = spawner.spawn();
        }
    }

    /// The jobs of the backlog that fit in flight now, oldest first.
    pub(crate) fn release(&mut self) -> Vec<Job> {
        let room = IN_FLIGHT
            .get()
            .saturating_sub(Count::<JobsInFlight>::of(self.in_flight.len()).get());
        let take = usize::try_from(room).unwrap_or(0).min(self.backlog.len());
        self.backlog.drain(..take).collect()
    }
}

/// The failure of a job whose thread is gone or was never spawned. Its
/// module is absent, as a module that did not load is (`ARCH-100`).
fn gone() -> ConvertError {
    tracing::debug!("the converter thread is not running");
    ConvertError::Absent
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The deadline the JPEG decodes of these tests run under.
    #[cfg(not(target_arch = "wasm32"))]
    const IMAGE_DEADLINE: TimeSpan<FracMs> = answer_deadline(ConverterFamily::Images);

    #[cfg(not(target_arch = "wasm32"))]
    fn decode_job(ticket: u64) -> Job {
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
            .encode_image(&image::RgbImage::from_pixel(4, 4, image::Rgb([1, 2, 3])))
            .expect("a JPEG encodes");
        Job {
            ticket: Seq::new(ticket),
            owner: JobOwner::Prose(Id::from_bytes([1; 16])),
            conversion: Conversion::DecodeJpeg(ambre_convert::jpeg::JpegDecode {
                data: Payload::new(jpeg),
                bound: ambre_core::Size::new(ambre_core::Px::new(8), ambre_core::Px::new(8)),
                size: ambre_core::Size::new(ambre_core::Px::new(4), ambre_core::Px::new(4)),
                turn: ambre_convert::jpeg::JpegTurn::Stored,
            }),
        }
    }

    /// `ARCH-102`, `ARCH-103`: the Logic thread holds at most `IN_FLIGHT` jobs on the
    /// thread. The rest wait in its backlog, and each answer makes room for
    /// one more, oldest first. Every job is answered once.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn jobs_past_the_flight_bound_wait_and_each_is_answered_once() {
        let (link, ends) = converter_channels(&Doorbell::new());
        let (mut thread, _) = ConverterThread::new(ends);
        let mut offload = Offload::over(link);
        offload
            .load(ConverterFamily::Images, Payload::new(Vec::new()))
            .expect("the thread takes the module");
        let total = 12u64;
        for ticket in 1..=total {
            assert!(offload.send(decode_job(ticket)).is_ok());
        }
        assert_eq!(
            Count::<JobsInFlight>::of(offload.in_flight.len()),
            IN_FLIGHT
        );
        let mut answered = Vec::new();
        while answered.len() < 12 {
            thread.serve();
            let answers = offload.take_answers(now());
            assert!(!answers.is_empty(), "the thread answers what it holds");
            for (ticket, _, answer) in answers {
                assert!(matches!(answer, Ok(Converted::Pixels(_))));
                answered.push(ticket.get());
            }
            for job in offload.release() {
                assert!(offload.send(job).is_ok());
            }
        }
        answered.sort_unstable();
        assert_eq!(answered, (1..=total).collect::<Vec<_>>());
        assert!(offload.in_flight.is_empty() && offload.backlog.is_empty());
    }

    /// `ARCH-101`: the converter thread runs on its own, and its answer
    /// wakes a parked Logic thread. That thread waited on its own bell alone.
    /// Dropping the Logic end ends the thread.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn an_answer_wakes_the_parked_logic_thread_and_the_thread_ends_with_its_link() {
        let logic_bell = Doorbell::new();
        let waiter = logic_bell
            .bind()
            .expect("the test thread binds the Logic bell");
        let (link, ends) = converter_channels(&logic_bell);
        let thread = std::thread::spawn(move || run_converter_thread(ends));
        let mut offload = Offload::over(link);
        offload
            .load(ConverterFamily::Images, Payload::new(Vec::new()))
            .expect("the thread takes the module");
        assert!(offload.send(decode_job(7)).is_ok());
        while !offload.has_answers() {
            waiter.wait();
        }
        let answers = offload.take_answers(now());
        assert!(matches!(
            answers.as_slice(),
            [(ticket, _, Ok(Converted::Pixels(_)))] if ticket.get() == 7
        ));
        drop(offload);
        assert!(thread.join().is_ok(), "the thread ends once its link drops");
    }

    /// `ARCH-102`: a job of a family the thread was never handed is answered
    /// with a failure, never dropped.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_job_of_a_module_the_thread_lacks_is_answered_unavailable() {
        let (link, ends) = converter_channels(&Doorbell::new());
        let (mut thread, _) = ConverterThread::new(ends);
        let mut offload = Offload::over(link);
        assert!(offload.send(decode_job(1)).is_ok());
        thread.serve();
        let answers = offload.take_answers(now());
        assert!(matches!(
            answers.as_slice(),
            [(_, _, Err(ConvertError::Absent))]
        ));
    }

    /// The instant `millis` after the epoch of the page clock.
    #[cfg(not(target_arch = "wasm32"))]
    fn at(millis: f64) -> Instant<PageLoad> {
        Instant::after_epoch(TimeSpan::new(millis))
    }

    /// A spawner whose threads a test serves by hand: each spawn leaves the
    /// new thread in `spawned`.
    #[cfg(not(target_arch = "wasm32"))]
    fn hand_spawner() -> (
        ConverterSpawner,
        std::sync::Arc<std::sync::Mutex<Vec<ConverterThread>>>,
    ) {
        let spawned = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let kept = std::sync::Arc::clone(&spawned);
        let spawner = ConverterSpawner::new(move || {
            let (link, ends) = converter_channels(&Doorbell::new());
            kept.lock()
                .expect("the test holds the lock")
                .push(ConverterThread::new(ends).0);
            Some(link)
        });
        (spawner, spawned)
    }

    /// `ARCH-112`: the boot spawns no thread, and a pass with nothing to
    /// convert spawns none either. The first job spawns the first thread,
    /// which counts against no limit: `RESPAWN_LIMIT` threads still follow it.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_first_thread_waits_for_the_first_job_and_counts_no_respawn() {
        let (spawner, spawned) = hand_spawner();
        let mut offload = Offload::started(ConverterThreads::on_demand(spawner));
        assert!(offload.take_answers(at(0.0)).is_empty());
        assert!(offload.release().is_empty());
        assert!(!offload.has_answers() && !offload.is_due(at(0.0)));
        assert_eq!(offload.wake_in(at(0.0)), None);
        assert!(
            spawned.lock().expect("lock").is_empty(),
            "no thread at boot"
        );
        offload.start();
        offload
            .load(ConverterFamily::Images, Payload::new(Vec::new()))
            .expect("the first thread takes the module");
        assert!(offload.send(decode_job(1)).is_ok());
        offload.start();
        assert_eq!(spawned.lock().expect("lock").len(), 1, "one first thread");
        spawned
            .lock()
            .expect("lock")
            .iter_mut()
            .for_each(ConverterThread::serve);
        assert!(matches!(
            offload.take_answers(at(0.0)).as_slice(),
            [(ticket, _, Ok(Converted::Pixels(_)))] if ticket.get() == 1
        ));
        assert_eq!(offload.respawns, Count::new(0));
        let mut clock = 0.0;
        for ticket in 2..=5 {
            offload
                .load(ConverterFamily::Images, Payload::new(Vec::new()))
                .expect("the thread takes the module");
            assert!(offload.send(decode_job(ticket)).is_ok());
            assert!(offload.take_answers(at(clock)).is_empty());
            clock += IMAGE_DEADLINE.get();
            assert_eq!(offload.take_answers(at(clock)).len(), 1);
        }
        assert_eq!(
            Count::<Respawns>::of(spawned.lock().expect("lock").len()),
            RESPAWN_LIMIT.plus(Count::new(1)),
            "the first thread and every thread spawned again"
        );
    }

    /// `ARCH-118`: a first spawn that fails is tried again by the next job,
    /// once a job. Each try past the first counts against `RESPAWN_LIMIT`;
    /// past it, each job fails at once and nothing is spawned. A job with no
    /// thread fails as a module that did not load, and nothing is fatal.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_first_spawn_that_fails_is_tried_again_by_the_next_job() {
        let asked = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = std::sync::Arc::clone(&asked);
        let spawner = ConverterSpawner::new(move || {
            counted.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            None
        });
        let mut offload = Offload::started(ConverterThreads::on_demand(spawner));
        let tries = u64::from(RESPAWN_LIMIT.get()) + 1;
        for ticket in 1..=tries + 2 {
            offload.start();
            assert_eq!(
                offload.load(ConverterFamily::Images, Payload::new(Vec::new())),
                Err(ConvertError::Absent)
            );
            assert!(matches!(
                offload.send(decode_job(ticket)),
                Err((_, _, ConvertError::Absent))
            ));
            let expected = usize::try_from(ticket.min(tries)).expect("a small count");
            assert_eq!(
                asked.load(std::sync::atomic::Ordering::Relaxed),
                expected,
                "one try a job, up to the first spawn and the limit"
            );
        }
        assert_eq!(offload.respawns, RESPAWN_LIMIT);
    }

    /// `ARCH-118`: a first spawn that fails and a second that succeeds. The
    /// second thread takes the module and answers the next job, and one
    /// respawn of `RESPAWN_LIMIT` is spent.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_failed_first_spawn_then_a_thread_lands_the_next_job() {
        let (working, spawned) = hand_spawner();
        let failed_once = std::sync::atomic::AtomicBool::new(false);
        let spawner = ConverterSpawner::new(move || {
            if failed_once.swap(true, std::sync::atomic::Ordering::Relaxed) {
                working.spawn()
            } else {
                None
            }
        });
        let mut offload = Offload::started(ConverterThreads::on_demand(spawner));
        offload.start();
        assert!(matches!(
            offload.send(decode_job(1)),
            Err((_, _, ConvertError::Absent))
        ));
        offload.start();
        offload
            .load(ConverterFamily::Images, Payload::new(Vec::new()))
            .expect("the second thread takes the module");
        assert!(offload.send(decode_job(2)).is_ok());
        spawned
            .lock()
            .expect("lock")
            .iter_mut()
            .for_each(ConverterThread::serve);
        assert!(matches!(
            offload.take_answers(at(0.0)).as_slice(),
            [(ticket, _, Ok(Converted::Pixels(_)))] if ticket.get() == 2
        ));
        assert_eq!(offload.respawns, Count::new(1));
    }

    /// `ARCH-108`: a thread that holds a job and answers nothing for the
    /// deadline fails it as unavailable. The Logic thread then spawns the
    /// next thread, which holds no module: the module loads again, and the
    /// next job is answered.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_job_unanswered_past_the_deadline_fails_and_the_thread_is_spawned_again() {
        let (link, _stuck) = converter_channels(&Doorbell::new());
        let (spawner, spawned) = hand_spawner();
        let mut offload = Offload::over(link).respawning(spawner);
        offload
            .load(ConverterFamily::Images, Payload::new(Vec::new()))
            .expect("the thread takes the module");
        assert!(offload.send(decode_job(1)).is_ok());
        assert!(offload.is_due(at(0.0)), "no pass dated the job yet");
        assert!(offload.take_answers(at(0.0)).is_empty());
        let just_before = IMAGE_DEADLINE.get() - 1.0;
        assert!(!offload.is_due(at(just_before)));
        assert!(offload.take_answers(at(just_before)).is_empty());
        assert!(
            offload
                .wake_in(at(just_before))
                .is_some_and(|left| left <= Duration::from_millis(1))
        );
        let answers = offload.take_answers(at(IMAGE_DEADLINE.get()));
        assert!(
            matches!(
                answers.as_slice(),
                [(ticket, _, Err(ConvertError::Absent))] if ticket.get() == 1
            ),
            "the job fails as unavailable"
        );
        assert!(
            !offload.holds(ConverterFamily::Images),
            "the new thread holds no module"
        );
        assert_eq!(offload.wake_in(at(IMAGE_DEADLINE.get())), None);
        offload
            .load(ConverterFamily::Images, Payload::new(Vec::new()))
            .expect("the new thread takes the module");
        assert!(offload.send(decode_job(2)).is_ok());
        let mut threads = spawned.lock().expect("the test holds the lock");
        assert_eq!(threads.len(), 1, "one thread is spawned");
        threads.iter_mut().for_each(ConverterThread::serve);
        let answers = offload.take_answers(at(IMAGE_DEADLINE.get()));
        assert!(matches!(
            answers.as_slice(),
            [(ticket, _, Ok(Converted::Pixels(_)))] if ticket.get() == 2
        ));
    }

    /// `ARCH-108`: the deadline runs from the latest answer, so a thread that
    /// keeps answering keeps the jobs it holds.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_deadline_runs_from_the_latest_answer() {
        let (link, ends) = converter_channels(&Doorbell::new());
        let (mut thread, _) = ConverterThread::new(ends);
        let mut offload = Offload::over(link);
        offload
            .load(ConverterFamily::Images, Payload::new(Vec::new()))
            .expect("the thread takes the module");
        assert!(offload.send(decode_job(1)).is_ok());
        assert!(offload.take_answers(at(0.0)).is_empty());
        thread.serve();
        assert!(offload.send(decode_job(2)).is_ok());
        let late = IMAGE_DEADLINE.get() * 0.9;
        assert_eq!(offload.take_answers(at(late)).len(), 1, "job 1 answers");
        let past_send = IMAGE_DEADLINE.get() * 1.5;
        assert!(
            offload.take_answers(at(past_send)).is_empty(),
            "job 2 waits: its thread answered within the deadline"
        );
        let past_answer = late + IMAGE_DEADLINE.get();
        assert!(matches!(
            offload.take_answers(at(past_answer)).as_slice(),
            [(_, _, Err(ConvertError::Absent))]
        ));
    }

    /// A PDF write of an empty document, which no test thread serves.
    #[cfg(not(target_arch = "wasm32"))]
    fn pdf_job(ticket: u64) -> Job {
        Job {
            ticket: Seq::new(ticket),
            owner: JobOwner::Prose(Id::from_bytes([1; 16])),
            conversion: Conversion::WritePdf {
                document: ambre_draw::list::PrintDocument::default(),
            },
        }
    }

    /// A sheet read, which no test thread serves.
    #[cfg(not(target_arch = "wasm32"))]
    fn sheet_job(ticket: u64) -> Job {
        Job {
            conversion: Conversion::ReadXlsx {
                data: Payload::new(Vec::new()),
            },
            ..pdf_job(ticket)
        }
    }

    /// `ARCH-127`: the job the thread works on is the oldest in flight, and
    /// its family sets the deadline. A sheet read is failed at the Office
    /// deadline, never held to the longer deadline of a decode queued behind
    /// it. A decode is held to its own deadline, never failed at the shorter
    /// one of a sheet read behind it.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_oldest_job_in_flight_sets_the_deadline_by_its_family() {
        let office = answer_deadline(ConverterFamily::Office).get();
        assert!(
            office < IMAGE_DEADLINE.get(),
            "the test needs two deadlines"
        );
        let (link, _stuck) = converter_channels(&Doorbell::new());
        let mut offload = Offload::over(link);
        assert!(offload.send(sheet_job(1)).is_ok());
        assert!(offload.send(decode_job(2)).is_ok());
        assert!(offload.take_answers(at(0.0)).is_empty());
        assert!(offload.take_answers(at(office - 1.0)).is_empty());
        assert_eq!(
            offload.take_answers(at(office)).len(),
            2,
            "the sheet read is the oldest: its deadline fails both"
        );
        let (link, _stuck) = converter_channels(&Doorbell::new());
        let mut offload = Offload::over(link);
        assert!(offload.send(decode_job(3)).is_ok());
        assert!(offload.send(sheet_job(4)).is_ok());
        assert!(offload.take_answers(at(0.0)).is_empty());
        assert!(
            offload.take_answers(at(office)).is_empty(),
            "the decode outlasts the deadline of the sheet read behind it"
        );
        assert!(
            offload
                .wake_in(at(office))
                .is_some_and(|left| left > Duration::ZERO),
            "the Logic thread parks until the deadline of the decode"
        );
        assert_eq!(offload.take_answers(at(IMAGE_DEADLINE.get())).len(), 2);
    }

    /// `ARCH-127`: no family runs under a deadline shorter than the floor.
    #[test]
    fn no_family_falls_under_the_floor() {
        assert!(
            ConverterFamily::ALL
                .iter()
                .all(|family| answer_deadline(*family) >= FLOOR_DEADLINE)
        );
    }

    /// `ARCH-127`: a PDF write runs under the deadline of its family.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_pdf_write_runs_under_the_pdf_deadline() {
        let (link, _stuck) = converter_channels(&Doorbell::new());
        let mut offload = Offload::over(link);
        let pdf = answer_deadline(ConverterFamily::Pdf).get();
        assert!(offload.send(pdf_job(1)).is_ok());
        assert!(offload.take_answers(at(0.0)).is_empty());
        assert!(offload.take_answers(at(pdf - 1.0)).is_empty());
        assert!(matches!(
            offload.take_answers(at(pdf)).as_slice(),
            [(_, _, Err(ConvertError::Absent))]
        ));
    }

    /// `ARCH-108`: past the limit of threads spawned again, no
    /// thread is spawned, and every job fails at once.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn past_the_respawn_limit_every_job_fails_at_once() {
        let (link, _stuck) = converter_channels(&Doorbell::new());
        let (spawner, spawned) = hand_spawner();
        let mut offload = Offload::over(link).respawning(spawner);
        let mut clock = 0.0;
        for ticket in 1..=4 {
            offload
                .load(ConverterFamily::Images, Payload::new(Vec::new()))
                .expect("the thread takes the module");
            assert!(offload.send(decode_job(ticket)).is_ok());
            assert!(offload.take_answers(at(clock)).is_empty());
            clock += IMAGE_DEADLINE.get();
            assert_eq!(offload.take_answers(at(clock)).len(), 1);
        }
        assert_eq!(
            Count::<Respawns>::of(spawned.lock().expect("lock").len()),
            RESPAWN_LIMIT
        );
        assert_eq!(
            offload.load(ConverterFamily::Images, Payload::new(Vec::new())),
            Err(ConvertError::Absent)
        );
        assert!(matches!(
            offload.send(decode_job(5)),
            Err((_, _, ConvertError::Absent))
        ));
    }

    /// `ARCH-107`: an instance that stopped mid-job is loaded again, as one
    /// past the memory bound is. An instance that answered keeps its memory.
    #[test]
    fn an_instance_that_stopped_mid_job_is_loaded_again() {
        let stopped: Result<Converted, ConvertError> =
            Err(ConvertError::Stopped("unreachable".to_owned()));
        assert!(needs_reload(&stopped, None));
        assert!(!needs_reload(&Err(ConvertError::Malformed), None));
        assert!(!needs_reload(
            &Ok(Converted::File(Payload::new(Vec::new()))),
            None
        ));
        assert!(needs_reload(
            &Err(ConvertError::Malformed),
            Some(RESET_FOOTPRINT.plus(Count::new(1)))
        ));
    }

    #[test]
    fn a_footprint_past_the_bound_resets_the_module_and_the_host_never_does() {
        assert!(!needs_reset(None));
        assert!(!needs_reset(Some(RESET_FOOTPRINT)));
        assert!(needs_reset(Some(RESET_FOOTPRINT.plus(Count::new(1)))));
    }
}

/// The reset of an instance in the browser, on a module written by hand
/// (`ARCH-099`).
#[cfg(all(test, target_arch = "wasm32"))]
pub(crate) mod wasm_tests {
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    /// A module of one page of memory whose `run` grows it by 1 100 pages,
    /// past 64 MiB. Its other exports answer 0, so a conversion reads no
    /// answer and fails as malformed.
    pub(crate) const GROWS: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, // magic, version
        // types: (i32) -> (i32), () -> (), () -> (i32)
        0x01, 0x0d, 0x03, 0x60, 0x01, 0x7f, 0x01, 0x7f, 0x60, 0x00, 0x00, 0x60, 0x00, 0x01, 0x7f,
        // four functions
        0x03, 0x05, 0x04, 0x00, 0x01, 0x02, 0x02, // one page of memory
        0x05, 0x03, 0x01, 0x00, 0x01, // exports
        0x07, 0x3a, 0x05, 0x06, b'm', b'e', b'm', b'o', b'r', b'y', 0x02, 0x00, 0x0d, b'i', b'n',
        b'p', b'u', b't', b'_', b'r', b'e', b's', b'e', b'r', b'v', b'e', 0x00, 0x00, 0x03, b'r',
        b'u', b'n', 0x00, 0x01, 0x0a, b'o', b'u', b't', b'p', b'u', b't', b'_', b'p', b't', b'r',
        0x00, 0x02, 0x0a, b'o', b'u', b't', b'p', b'u', b't', b'_', b'l', b'e', b'n', 0x00, 0x03,
        // code: input_reserve, run (i32.const 1100; memory.grow; drop), and
        // the two outputs
        0x0a, 0x19, 0x04, 0x04, 0x00, 0x41, 0x00, 0x0b, 0x08, 0x00, 0x41, 0xcc, 0x08, 0x40, 0x00,
        0x1a, 0x0b, 0x04, 0x00, 0x41, 0x00, 0x0b, 0x04, 0x00, 0x41, 0x00, 0x0b,
    ];

    /// The module of [`GROWS`] with another `run`. It grows the memory by
    /// one page, then traps on `unreachable`: a trap in the middle of a job,
    /// which leaves its memory changed.
    const TRAPS: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, // magic, version
        // types: (i32) -> (i32), () -> (), () -> (i32)
        0x01, 0x0d, 0x03, 0x60, 0x01, 0x7f, 0x01, 0x7f, 0x60, 0x00, 0x00, 0x60, 0x00, 0x01, 0x7f,
        // four functions
        0x03, 0x05, 0x04, 0x00, 0x01, 0x02, 0x02, // one page of memory
        0x05, 0x03, 0x01, 0x00, 0x01, // exports
        0x07, 0x3a, 0x05, 0x06, b'm', b'e', b'm', b'o', b'r', b'y', 0x02, 0x00, 0x0d, b'i', b'n',
        b'p', b'u', b't', b'_', b'r', b'e', b's', b'e', b'r', b'v', b'e', 0x00, 0x00, 0x03, b'r',
        b'u', b'n', 0x00, 0x01, 0x0a, b'o', b'u', b't', b'p', b'u', b't', b'_', b'p', b't', b'r',
        0x00, 0x02, 0x0a, b'o', b'u', b't', b'p', b'u', b't', b'_', b'l', b'e', b'n', 0x00, 0x03,
        // code: input_reserve, run (i32.const 1; memory.grow; drop;
        // unreachable), and the two outputs
        0x0a, 0x19, 0x04, 0x04, 0x00, 0x41, 0x00, 0x0b, 0x08, 0x00, 0x41, 0x01, 0x40, 0x00, 0x1a,
        0x00, 0x0b, 0x04, 0x00, 0x41, 0x00, 0x0b, 0x04, 0x00, 0x41, 0x00, 0x0b,
    ];

    /// A CSV write of no record, which reaches the module's `run`.
    fn csv_job(ticket: u64) -> Job {
        Job {
            ticket: Seq::new(ticket),
            owner: JobOwner::Prose(Id::from_bytes([1; 16])),
            conversion: Conversion::WriteCsv {
                records: Vec::new(),
                delimiter: ambre_convert::CsvDelimiter::COMMA,
            },
        }
    }

    /// `ARCH-107`: a module that traps in the middle of a job fails that job
    /// as unavailable, and the thread catches the trap and lives on. Its
    /// instance loads again: the page the trap left grown is gone. The next
    /// job is answered too, so no job waits for ever.
    #[wasm_bindgen_test]
    fn a_module_that_traps_mid_job_fails_it_and_is_loaded_again() {
        let (link, ends) = converter_channels(&Doorbell::new());
        let (mut thread, _) = ConverterThread::new(ends);
        let mut offload = Offload::over(link);
        assert!(
            offload
                .load(ConverterFamily::Tables, Payload::new(TRAPS.to_vec()))
                .is_ok(),
            "the thread takes the module"
        );
        thread.serve();
        let page = Count::<Octets>::new(64 * 1024);
        assert_eq!(thread.footprint(ConverterFamily::Tables), Some(page));
        for ticket in 1..=2 {
            assert!(offload.send(csv_job(ticket)).is_ok());
            thread.serve();
            let answers = offload.take_answers(now());
            assert!(
                matches!(
                    answers.as_slice(),
                    [(answered, _, Err(ConvertError::Absent))] if answered.get() == ticket
                ),
                "the trapped job is answered once, as unavailable"
            );
            assert_eq!(
                thread.footprint(ConverterFamily::Tables),
                Some(page),
                "the trapped instance was dropped and loaded again"
            );
        }
    }

    #[wasm_bindgen_test]
    fn a_module_grown_past_the_bound_is_loaded_again() {
        let (link, ends) = converter_channels(&Doorbell::new());
        let (mut thread, _) = ConverterThread::new(ends);
        let mut offload = Offload::over(link);
        assert!(
            offload
                .load(ConverterFamily::Tables, Payload::new(GROWS.to_vec()))
                .is_ok(),
            "the thread takes the module"
        );
        thread.serve();
        let page = Count::<Octets>::new(64 * 1024);
        assert_eq!(thread.footprint(ConverterFamily::Tables), Some(page));
        let job = Job {
            ticket: Seq::new(1),
            owner: JobOwner::Prose(Id::from_bytes([1; 16])),
            conversion: Conversion::WriteCsv {
                records: Vec::new(),
                delimiter: ambre_convert::CsvDelimiter::COMMA,
            },
        };
        assert!(offload.send(job).is_ok());
        thread.serve();
        let answers = offload.take_answers(now());
        assert!(
            matches!(answers.as_slice(), [(_, _, Err(ConvertError::Absent))]),
            "the job is answered once"
        );
        assert_eq!(
            thread.footprint(ConverterFamily::Tables),
            Some(page),
            "the grown instance was dropped and loaded again"
        );
    }
}

/// The respawn of a converter thread in the browser, through the spawner the
/// page builds (`ARCH-108`). It spawns real workers, so it needs a shared
/// memory: the threaded build of `just test-wasm` alone compiles it.
#[cfg(all(test, target_arch = "wasm32", target_feature = "atomics"))]
mod respawn_tests {
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    /// The sections of a module of one page of memory, up to its code: the
    /// four exports of a converter module (`ARCH-079`).
    const HEAD: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, // magic, version
        // types: (i32) -> (i32), () -> (), () -> (i32)
        0x01, 0x0d, 0x03, 0x60, 0x01, 0x7f, 0x01, 0x7f, 0x60, 0x00, 0x00, 0x60, 0x00, 0x01, 0x7f,
        // four functions
        0x03, 0x05, 0x04, 0x00, 0x01, 0x02, 0x02, // one page of memory
        0x05, 0x03, 0x01, 0x00, 0x01, // exports
        0x07, 0x3a, 0x05, 0x06, b'm', b'e', b'm', b'o', b'r', b'y', 0x02, 0x00, 0x0d, b'i', b'n',
        b'p', b'u', b't', b'_', b'r', b'e', b's', b'e', b'r', b'v', b'e', 0x00, 0x00, 0x03, b'r',
        b'u', b'n', 0x00, 0x01, 0x0a, b'o', b'u', b't', b'p', b'u', b't', b'_', b'p', b't', b'r',
        0x00, 0x02, 0x0a, b'o', b'u', b't', b'p', b'u', b't', b'_', b'l', b'e', b'n', 0x00, 0x03,
    ];

    /// A `run` that does nothing. The answer is four zero bytes at 1 KiB,
    /// past the request: an empty table, so each job answers `Ok`.
    const ANSWERS: &[u8] = &[
        0x0a, 0x14, 0x04, 0x04, 0x00, 0x41, 0x00, 0x0b, 0x02, 0x00, 0x0b, 0x05, 0x00, 0x41, 0x80,
        0x08, 0x0b, 0x04, 0x00, 0x41, 0x04, 0x0b,
    ];

    /// A `run` that loops for ever: a job that never ends, so its thread
    /// stops answering with its queues open.
    const SPINS: &[u8] = &[
        0x0a, 0x18, 0x04, 0x04, 0x00, 0x41, 0x00, 0x0b, 0x07, 0x00, 0x03, 0x40, 0x0c, 0x00, 0x0b,
        0x0b, 0x04, 0x00, 0x41, 0x00, 0x0b, 0x04, 0x00, 0x41, 0x00, 0x0b,
    ];

    /// The module of [`HEAD`] with the code section `code`.
    fn module(code: &[u8]) -> Payload<ConverterModule> {
        Payload::new([HEAD, code].concat())
    }

    /// How one job ended, as the Logic worker saw it.
    #[derive(Debug, PartialEq)]
    enum Outcome {
        Answered,
        Failed(ConvertError),
    }

    fn job(ticket: u64, conversion: Conversion) -> Job {
        Job {
            ticket: Seq::new(ticket),
            owner: JobOwner::Prose(Id::from_bytes([1; 16])),
            conversion,
        }
    }

    fn table_job(ticket: u64) -> Job {
        job(
            ticket,
            Conversion::WriteCsv {
                records: Vec::new(),
                delimiter: ambre_convert::CsvDelimiter::COMMA,
            },
        )
    }

    /// Send `job`, then park on the Logic bell until a pass takes answers,
    /// as the Logic thread's loop does.
    fn run(
        offload: &mut Offload,
        waiter: &ambre_bus::conduit::Waiter,
        job: Job,
    ) -> Vec<(u64, Outcome)> {
        if let Err((ticket, _, failure)) = offload.send(job) {
            return vec![(ticket.get(), Outcome::Failed(failure))];
        }
        loop {
            let at = now();
            let answers = offload.take_answers(at);
            if !answers.is_empty() {
                return answers
                    .into_iter()
                    .map(|(ticket, _, answer)| {
                        let outcome = answer.map_or_else(Outcome::Failed, |_| Outcome::Answered);
                        (ticket.get(), outcome)
                    })
                    .collect();
            }
            let Some(left) = offload.wake_in(at) else {
                return Vec::new();
            };
            waiter.wait_at_most(left);
        }
    }

    /// The Logic worker's side. The first thread answers a job, then spins
    /// in the next one past a short deadline. The worker then spawns the
    /// next thread, loads the module again, and the next job lands there.
    fn logic_worker(bell: &Doorbell, threads: ConverterThreads) -> (Vec<(u64, Outcome)>, bool) {
        let Some(waiter) = bell.bind() else {
            return (Vec::new(), false);
        };
        let mut offload = Offload::started(threads);
        // The first job spawns the first thread (`ARCH-112`).
        offload.start();
        let loaded = offload
            .load(ConverterFamily::Tables, module(ANSWERS))
            .is_ok()
            && offload.load(ConverterFamily::Markup, module(SPINS)).is_ok();
        let mut seen = run(&mut offload, &waiter, table_job(1));
        // The first thread is up: a short deadline holds it lost quickly.
        let mut offload = offload.with_deadline(TimeSpan::new(500.0));
        let html = Conversion::ReadHtml {
            html: String::new(),
        };
        seen.extend(run(&mut offload, &waiter, job(2, html)));
        // The next thread starts as the first did, under the production
        // deadline of the family it runs.
        let mut offload = offload.with_deadline(answer_deadline(ConverterFamily::Tables));
        let reloaded = !offload.holds(ConverterFamily::Tables)
            && offload
                .load(ConverterFamily::Tables, module(ANSWERS))
                .is_ok();
        seen.extend(run(&mut offload, &waiter, table_job(3)));
        (seen, loaded && reloaded)
    }

    /// Build the spawner as the page does, run [`logic_worker`] on a
    /// Logic worker, and wait for what it saw. `None` when the Logic worker
    /// did not spawn or did not end.
    async fn boot_and_run() -> Option<(Vec<(u64, Outcome)>, bool)> {
        let logic_bell = Doorbell::new();
        let threads =
            ConverterThreads::on_demand(ConverterSpawner::over_threads(logic_bell.clone()));
        let logic = wasm_thread::Builder::new()
            .name("logic".to_owned())
            .spawn(move || logic_worker(&logic_bell, threads))
            .ok()?;
        logic.join_async().await.ok()
    }

    /// `ARCH-108`: a converter thread that spins in a job is held lost past
    /// the deadline, and that job fails as unavailable. The Logic worker
    /// spawns the next thread through the spawner the page builds. The page's
    /// thread creates that worker, and the next job lands on it, once the
    /// module loads again.
    #[wasm_bindgen_test]
    async fn a_lost_thread_is_spawned_again_from_the_logic_worker_and_lands_the_next_job() {
        let seen = vec![
            (1, Outcome::Answered),
            (2, Outcome::Failed(ConvertError::Absent)),
            (3, Outcome::Answered),
        ];
        assert_eq!(
            boot_and_run().await,
            Some((seen, true)),
            "the first thread answers, then is held lost, and the next one answers"
        );
    }
}
