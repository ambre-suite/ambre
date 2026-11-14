// ambre-logic/src/converter_thread.rs
// ============================================================================
// The converter thread. It is a third thread of the Main instance, beside
// the UI and Logic threads. It runs the conversions the Logic thread must
// never wait on (`ambre-spec-architecture.md section 2`, the converter
// thread).
//
// Today it runs the JPEG decode of the images module alone (`ARCH-097`). The
// other conversions still run on the Logic thread, and the
// `Every conversion runs off the Logic thread` Story moves them. So the two
// envelopes carry any `Conversion`, never a JPEG of their own.
//
// The Logic thread sends a job and never waits for its answer. The answer
// rings the Logic thread's bell, and the next pass hands it to the document
// or the channel that asked (`ARCH-098`, `ARCH-101`). The samples cross as a `Vec` on the
// shared heap: the hop between the two threads copies no byte.
// ============================================================================

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use ambre_bus::Request;
use ambre_bus::conduit::{self, ChannelSlots, Doorbell, Receiver, Sender, TrySendError};
use ambre_convert::{Conversion, ConvertError, Converted, Converter};
use ambre_core::id::{Channel, Document, Id};
use ambre_core::{
    Bounded, ConverterFamily, ConverterModule, Count, CountKind, Octets, Payload, Seq,
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

impl CountKind for JobsInFlight {
    type Repr = u32;
    const NAME: &'static str = "JobsInFlightCount";
}

/// The slots of each channel: the jobs in flight, and one load per family.
/// The thread answers each job once, so its answers never pass the bound.
const SLOTS: Bounded<ChannelSlots> = Bounded::<ChannelSlots>::of::<16>();

/// The memory a module may span after a job before the thread drops it and
/// loads it again. A WebAssembly memory never shrinks: a decode of a large
/// photograph would otherwise hold its peak for the session (`ARCH-099`,
/// `ambre-spec-budgets.md section 6`).
pub const RESET_FOOTPRINT: Count<Octets> = Count::new(64 * 1024 * 1024);

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
        for _ in 0..queued {
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

    /// Run `conversion` on its family's instance, then drop an instance whose
    /// memory grew past [`RESET_FOOTPRINT`] and load it again.
    fn run(&mut self, conversion: &Conversion) -> Result<Converted, ConvertError> {
        let family = conversion.family();
        let held = self.modules.get_mut(&family).ok_or(ConvertError::Absent)?;
        let converter = held.converter.as_ref().map_err(Clone::clone)?;
        let answer = converter.convert(conversion);
        if needs_reset(converter.footprint()) {
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
    /// A picture a Toile board inserts, by the ticket Toile minted.
    Toile(Id<Document>),
    /// The preview of a Murmure message.
    Murmure {
        channel_id: Id<Channel>,
        media_ref: ambre_core::Validated<ambre_bus::ContentHash>,
    },
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

/// The Logic thread's view of the converter thread: the link, the jobs in
/// flight by their ticket, the backlog and the families the thread holds.
#[derive(Default)]
pub(crate) struct Offload {
    link: Option<ConverterLink>,
    in_flight: HashMap<Seq<Request>, JobOwner>,
    backlog: VecDeque<Job>,
    loaded: BTreeSet<ConverterFamily>,
}

impl Offload {
    /// The view over `link`.
    pub(crate) fn over(link: ConverterLink) -> Self {
        Self {
            link: Some(link),
            ..Self::default()
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
    ) -> Result<(), JobFailure> {
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
    pub(crate) fn send(&mut self, job: Job) -> Result<(), (Seq<Request>, JobOwner, JobFailure)> {
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
        match link.to.try_send(ToConverter::Run { ticket, conversion }) {
            Ok(()) => {
                self.in_flight.insert(ticket, owner);
                Ok(())
            }
            Err(_) => Err((ticket, owner, gone())),
        }
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

    /// The answers queued now, each with the owner that asked, and every job
    /// in flight when the thread is gone, failed. The backlog moves up into
    /// the room the answers freed.
    pub(crate) fn take_answers(
        &mut self,
    ) -> Vec<(Seq<Request>, JobOwner, Result<Converted, JobFailure>)> {
        let mut out = Vec::new();
        if let Some(link) = self.link.as_ref() {
            for FromConverter { ticket, answer } in link.from.try_iter().take(link.from.len()) {
                if let Some(owner) = self.in_flight.remove(&ticket) {
                    out.push((ticket, owner, answer.map_err(|error| failure_of(&error))));
                }
            }
        }
        if self.is_lost() && !self.in_flight.is_empty() {
            out.extend(
                self.in_flight
                    .drain()
                    .map(|(ticket, owner)| (ticket, owner, Err(gone()))),
            );
        }
        out
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

/// The failure of a job whose thread is gone or was never spawned.
fn gone() -> JobFailure {
    tracing::debug!("the converter thread is not running");
    JobFailure::Unavailable
}

#[cfg(test)]
mod tests {
    use super::*;

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
            let answers = offload.take_answers();
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
        let answers = offload.take_answers();
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
        let answers = offload.take_answers();
        assert!(matches!(
            answers.as_slice(),
            [(_, _, Err(JobFailure::Unavailable))]
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
mod wasm_tests {
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    /// A module of one page of memory whose `run` grows it by 1 100 pages,
    /// past 64 MiB. Its other exports answer 0, so a conversion reads no
    /// answer and fails as malformed.
    const GROWS: &[u8] = &[
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
        let answers = offload.take_answers();
        assert!(
            matches!(answers.as_slice(), [(_, _, Err(JobFailure::Unavailable))]),
            "the job is answered once"
        );
        assert_eq!(
            thread.footprint(ConverterFamily::Tables),
            Some(page),
            "the grown instance was dropped and loaded again"
        );
    }
}
