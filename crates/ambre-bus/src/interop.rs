//! The interop vocabulary every application shares
//! (`1-Foundations/ambre-spec-interop.md`).
//!
//! An application brings a file in and takes a document out through one
//! command shape, [`InteropRequest`], and answers through one event shape,
//! [`InteropAnswer`]. What a conversion carried, moved, approximated or dropped
//! is one [`ConversionReport`], filled by each reader and each writer. A sliced
//! conversion says how far it got through one [`ConversionProgress`].
//!
//! What differs per application is only what it converts: the formats it reads
//! and writes, with their options, and the constructs its documents hold. Each
//! application names those in its own crate, as the type parameters here. A
//! new format is then a reader or a writer, and never a new message.

use std::fmt;

use ambre_core::{Count, CountKind, ExportFilename, FileBytes, MediaType, Octets, Payload};

/// What a user does with a file: bring it into a document, or take a document
/// out as one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    Import,
    Export,
}

/// A file format a reader or a writer of the suite speaks. One closed list for
/// the four applications: a format is the same thing whichever application
/// reads it. A variant joins with the first reader or writer of that format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileFormat {
    /// The native `.ambre` file (`ambre-spec-docfile.md`).
    Ambre,
    Markdown,
    Csv,
    Svg,
    /// A raster picture: PNG, JPEG, WebP or GIF.
    Image,
    Pdf,
    /// A PowerPoint presentation (`.pptx`).
    Pptx,
}

impl FileFormat {
    /// The words the user reads for it.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Ambre => "Ambre file",
            Self::Markdown => "Markdown",
            Self::Csv => "CSV",
            Self::Svg => "SVG",
            Self::Image => "picture",
            Self::Pdf => "PDF",
            Self::Pptx => "PowerPoint",
        }
    }
}

impl fmt::Display for FileFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// What a conversion did with one construct. The order is the order a report
/// reads in: what arrived first, then what changed, worst last.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Outcome {
    /// The construct arrived whole.
    Carried,
    /// The construct arrived, at the nearest place the target allows.
    Moved,
    /// The construct arrived as the nearest thing the target can express.
    Approximated,
    /// The construct did not arrive. Its text stays where it has any.
    Dropped,
}

impl Outcome {
    /// The verb a report reads for it.
    #[must_use]
    pub const fn verb(self) -> &'static str {
        match self {
            Self::Carried => "carried",
            Self::Moved => "moved",
            Self::Approximated => "approximated",
            Self::Dropped => "dropped",
        }
    }
}

/// A thing a document holds that a conversion can carry or lose: a footnote, a
/// sticky, a row. Each application names its own as one closed `enum`, because
/// a sheet cannot lose what a board holds. The report reads it through this
/// trait alone.
pub trait Construct: Copy + Eq + fmt::Debug {
    /// The words the user reads for one of it.
    fn label(self) -> &'static str;

    /// The words the user reads for several of it. A report counts a lost
    /// construct as `label (n)`, so only a carried one reads this.
    fn plural(self) -> &'static str {
        self.label()
    }
}

/// How many times one construct met one outcome in one conversion.
pub enum Occurrences {}

impl CountKind for Occurrences {
    type Repr = u32;
    const NAME: &'static str = "OccurrenceCount";
}

/// One line of a report: a construct, what happened to it, and how often.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReportEntry<C> {
    pub construct: C,
    pub outcome: Outcome,
    pub count: Count<Occurrences>,
}

/// What a finished conversion carried and lost, for the user to read
/// (`INTEROP-003`). A conversion that produced nothing answers an
/// [`InteropError`] instead: the two are never one type (`INTEROP-002`).
///
/// One construct met twice with one outcome is one entry with its count, so a
/// long file does not bury the user in repeats. The entries keep the order the
/// conversion first met them in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversionReport<C> {
    direction: Direction,
    format: FileFormat,
    entries: Vec<ReportEntry<C>>,
}

/// How many lost constructs a sentence names per outcome before it counts the
/// rest. A list nobody reads at the corner of a screen says nothing.
const NAMED_PER_OUTCOME: usize = 3;

impl<C: Construct> ConversionReport<C> {
    /// A report of nothing yet carried and nothing lost.
    #[must_use]
    pub const fn new(direction: Direction, format: FileFormat) -> Self {
        Self {
            direction,
            format,
            entries: Vec::new(),
        }
    }

    /// The way the file went.
    #[must_use]
    pub const fn direction(&self) -> Direction {
        self.direction
    }

    /// The format of the file read or written.
    #[must_use]
    pub const fn format(&self) -> FileFormat {
        self.format
    }

    /// Every entry, in the order the conversion first met it.
    #[must_use]
    pub fn entries(&self) -> &[ReportEntry<C>] {
        &self.entries
    }

    /// Count one more `construct` with `outcome`.
    pub fn record(&mut self, construct: C, outcome: Outcome) {
        self.record_count(construct, outcome, Count::new(1));
    }

    /// Count `count` more `construct` with `outcome`. A zero count writes no
    /// entry, so a reader may record a total it never checked.
    pub fn record_count(&mut self, construct: C, outcome: Outcome, count: Count<Occurrences>) {
        if count.is_zero() {
            return;
        }
        match self
            .entries
            .iter_mut()
            .find(|entry| entry.construct == construct && entry.outcome == outcome)
        {
            Some(entry) => entry.count = entry.count.plus(count),
            None => self.entries.push(ReportEntry {
                construct,
                outcome,
                count,
            }),
        }
    }

    /// Fold `other` into this report, entry by entry. A reader and the step
    /// that applies its result each fill a report of one conversion.
    pub fn absorb(&mut self, other: Self) {
        for entry in other.entries {
            self.record_count(entry.construct, entry.outcome, entry.count);
        }
    }

    /// How many `construct` met `outcome`, zero when none did.
    #[must_use]
    pub fn count(&self, construct: C, outcome: Outcome) -> Count<Occurrences> {
        self.entries
            .iter()
            .filter(|entry| entry.construct == construct && entry.outcome == outcome)
            .map(|entry| entry.count)
            .sum()
    }

    /// Whether the conversion lost nothing: every entry was carried.
    #[must_use]
    pub fn is_lossless(&self) -> bool {
        self.entries
            .iter()
            .all(|entry| entry.outcome == Outcome::Carried)
    }

    /// The report as one sentence the user reads (`INTEROP-007`). An import
    /// opens on what it carried. Each outcome that lost something follows, worst
    /// last, naming its first constructs and counting the rest. A report with
    /// nothing to say answers `None`.
    #[must_use]
    pub fn sentence(&self) -> Option<String> {
        let head = self.carried_clause();
        let losses: Vec<String> = [Outcome::Moved, Outcome::Approximated, Outcome::Dropped]
            .into_iter()
            .filter_map(|outcome| self.loss_clause(outcome))
            .collect();
        let mut parts: Vec<String> = head.into_iter().chain(losses).collect();
        let first = parts.first_mut()?;
        if let Some(initial) = first.get(..1) {
            let capital = initial.to_ascii_uppercase();
            first.replace_range(..1, &capital);
        }
        Some(parts.join("; "))
    }

    /// `imported 2 rows and 3 columns`, or nothing for an export or an import
    /// that counts nothing it carried.
    fn carried_clause(&self) -> Option<String> {
        if self.direction == Direction::Export {
            return None;
        }
        let carried: Vec<String> = self
            .entries
            .iter()
            .filter(|entry| entry.outcome == Outcome::Carried)
            .map(|entry| {
                let noun = if entry.count.get() == 1 {
                    entry.construct.label()
                } else {
                    entry.construct.plural()
                };
                format!("{} {noun}", entry.count)
            })
            .collect();
        let (last, rest) = carried.split_last()?;
        Some(if rest.is_empty() {
            format!("imported {last}")
        } else {
            format!("imported {} and {last}", rest.join(", "))
        })
    }

    /// `dropped: footnote (2), HTML, and 3 more`, or nothing when no entry met
    /// `outcome`.
    fn loss_clause(&self, outcome: Outcome) -> Option<String> {
        let met: Vec<&ReportEntry<C>> = self
            .entries
            .iter()
            .filter(|entry| entry.outcome == outcome)
            .collect();
        if met.is_empty() {
            return None;
        }
        let named: Vec<String> = met
            .iter()
            .take(NAMED_PER_OUTCOME)
            .map(|entry| match entry.count.get() {
                1 => entry.construct.label().to_owned(),
                _ => format!("{} ({})", entry.construct.label(), entry.count),
            })
            .collect();
        let rest = met.len().saturating_sub(named.len());
        let list = if rest == 0 {
            named.join(", ")
        } else {
            format!("{}, and {rest} more", named.join(", "))
        };
        Some(format!("{}: {list}", outcome.verb()))
    }
}

/// Why a conversion produced nothing (`INTEROP-002`). The document holds what
/// it held before the command.
///
/// A reason may quote the foreign file, so `Debug` prints its length alone
/// (`INTEROP-004`).
#[derive(Clone, PartialEq, Eq)]
pub enum InteropError {
    /// The bytes do not read as the format named.
    ParseFailed(String),
    /// The file is larger than a conversion accepts.
    TooLarge(Count<Octets>),
    /// The conversion read the file, then stopped before it produced anything.
    /// A safety bound, a failed upload or an empty page each stop it.
    Stopped(String),
}

impl fmt::Display for InteropError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ParseFailed(reason) => write!(f, "the file could not be read: {reason}"),
            Self::TooLarge(size) => write!(f, "the file is too large to import ({size} bytes)"),
            Self::Stopped(reason) => write!(f, "the conversion stopped: {reason}"),
        }
    }
}

impl fmt::Debug for InteropError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ParseFailed(reason) => f
                .debug_tuple("ParseFailed")
                .field(&ambre_core::RedactedChars::of(reason))
                .finish(),
            Self::TooLarge(size) => f.debug_tuple("TooLarge").field(size).finish(),
            Self::Stopped(reason) => f
                .debug_tuple("Stopped")
                .field(&ambre_core::RedactedChars::of(reason))
                .finish(),
        }
    }
}

impl std::error::Error for InteropError {}

/// A conversion that produced nothing, and which way it went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InteropFailure {
    pub direction: Direction,
    pub error: InteropError,
}

/// The unit a sliced conversion counts its work in. Each application chooses
/// its own: Toile counts the units of an import batch. The view reads only the
/// ratio, so the unit never reaches the user.
pub enum Steps {}

impl CountKind for Steps {
    type Repr = u32;
    const NAME: &'static str = "StepCount";
}

/// How far a sliced conversion got (`INTEROP-008`). The count never passes its
/// total (`INTEROP-010`), so the bar a view draws never overflows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConversionProgress {
    direction: Direction,
    done: Count<Steps>,
    total: Count<Steps>,
}

impl ConversionProgress {
    /// The progress of `done` steps out of `total`. A `done` past the total
    /// reads as the total.
    #[must_use]
    pub fn new(direction: Direction, done: Count<Steps>, total: Count<Steps>) -> Self {
        Self {
            direction,
            done: done.min(total),
            total,
        }
    }

    /// The way the file goes.
    #[must_use]
    pub const fn direction(self) -> Direction {
        self.direction
    }

    /// The steps run so far.
    #[must_use]
    pub const fn done(self) -> Count<Steps> {
        self.done
    }

    /// The steps the whole conversion runs.
    #[must_use]
    pub const fn total(self) -> Count<Steps> {
        self.total
    }

    /// The share of the work done, from 0 to 1. A conversion of no step is
    /// done.
    #[must_use]
    pub fn ratio(self) -> f64 {
        if self.total.get() == 0 {
            return 1.0;
        }
        f64::from(self.done.get()) / f64::from(self.total.get())
    }
}

/// A file an export wrote, for the UI thread to hand to the browser. The Logic
/// thread holds the document, so it writes the bytes; the UI thread owns the
/// page, so it starts the download.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportedFile {
    /// The name, which `DOCFILE-019` and `DOCFILE-020` hold, with its extension.
    pub filename: ExportFilename,
    pub media_type: MediaType,
    pub data: Payload<FileBytes>,
}

/// The one command shape of import and export (`INTEROP-001`). `I` names a
/// format an application reads, with its options and the bytes. `E` names a
/// format it writes, with its options. An application with no reader yet
/// passes an empty `enum`, and the variant cannot be built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InteropRequest<I, E> {
    /// Bring a file into the document, as one undo item.
    Import(I),
    /// Write the document out. It writes nothing into the document.
    Export(E),
}

impl<I, E> InteropRequest<I, E> {
    /// Whether the command can change the document: an import writes, an
    /// export reads (`INTEROP-005`, `INTEROP-006`). The role gate reads it, so
    /// a reader keeps the export and loses the import.
    #[must_use]
    pub const fn is_mutating(&self) -> bool {
        match self {
            Self::Import(_) => true,
            Self::Export(_) => false,
        }
    }

    /// The way the file goes.
    #[must_use]
    pub const fn direction(&self) -> Direction {
        match self {
            Self::Import(_) => Direction::Import,
            Self::Export(_) => Direction::Export,
        }
    }
}

/// The one event shape of import and export (`INTEROP-001`), generic over the
/// constructs of the application that answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InteropAnswer<C> {
    /// An import landed, as one undo item. The report says what it carried
    /// and what it lost.
    ImportCompleted { report: ConversionReport<C> },
    /// An export wrote a file. The report says what the format could not
    /// express.
    ExportReady {
        file: ExportedFile,
        report: ConversionReport<C>,
    },
    /// A sliced conversion ran one more slice.
    Progress(ConversionProgress),
    /// A conversion produced nothing, and the document is as it was.
    Failed(InteropFailure),
}

impl<C> InteropAnswer<C> {
    /// The answer to a conversion that produced nothing.
    #[must_use]
    pub const fn failed(direction: Direction, error: InteropError) -> Self {
        Self::Failed(InteropFailure { direction, error })
    }
}

impl<C: Construct> InteropAnswer<C> {
    /// What the user reads for this event, if anything (`INTEROP-007`,
    /// `INTEROP-011`). A progress reads nothing: the view draws a bar.
    #[must_use]
    pub fn message(&self) -> Option<String> {
        match self {
            Self::ImportCompleted { report } => Some(
                report
                    .sentence()
                    .unwrap_or_else(|| format!("Imported the {} file", report.format())),
            ),
            Self::ExportReady { file, report } => Some(match report.sentence() {
                Some(sentence) => format!("Exported {}; {sentence}", file.filename),
                None => format!("Exported {}", file.filename),
            }),
            Self::Progress(_) => None,
            Self::Failed(failure) => Some(match failure.direction {
                Direction::Import => format!("Import failed: {}", failure.error),
                Direction::Export => format!("Export failed: {}", failure.error),
            }),
        }
    }

    /// Whether the user should read [`message`](Self::message) as a warning:
    /// a conversion that lost something, or produced nothing.
    #[must_use]
    pub fn is_warning(&self) -> bool {
        match self {
            Self::ImportCompleted { report } | Self::ExportReady { report, .. } => {
                !report.is_lossless()
            }
            Self::Progress(_) => false,
            Self::Failed(_) => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Thing {
        Row,
        Column,
        Footnote,
        Html,
        Table,
        Gradient,
    }

    impl Construct for Thing {
        fn label(self) -> &'static str {
            match self {
                Self::Row => "row",
                Self::Column => "column",
                Self::Footnote => "footnote",
                Self::Html => "HTML",
                Self::Table => "table",
                Self::Gradient => "gradient",
            }
        }

        fn plural(self) -> &'static str {
            match self {
                Self::Row => "rows",
                Self::Column => "columns",
                other => other.label(),
            }
        }
    }

    fn import() -> ConversionReport<Thing> {
        ConversionReport::new(Direction::Import, FileFormat::Markdown)
    }

    /// `INTEROP-003`: one construct met twice with one outcome is one entry
    /// with its count, and a second outcome is a second entry.
    #[test]
    fn the_same_construct_twice_is_one_entry_with_its_count() {
        let mut report = import();
        report.record(Thing::Footnote, Outcome::Dropped);
        report.record(Thing::Footnote, Outcome::Dropped);
        report.record(Thing::Footnote, Outcome::Approximated);
        report.record_count(Thing::Html, Outcome::Dropped, Count::new(0));
        assert_eq!(report.entries().len(), 2);
        assert_eq!(report.count(Thing::Footnote, Outcome::Dropped).get(), 2);
        assert_eq!(
            report.count(Thing::Footnote, Outcome::Approximated).get(),
            1
        );
        assert_eq!(report.count(Thing::Html, Outcome::Dropped).get(), 0);
    }

    /// Folding a second report counts each entry once more, in place.
    #[test]
    fn a_report_absorbed_adds_its_counts() {
        let mut first = import();
        first.record(Thing::Footnote, Outcome::Dropped);
        let mut second = import();
        second.record(Thing::Footnote, Outcome::Dropped);
        second.record(Thing::Table, Outcome::Moved);
        first.absorb(second);
        assert_eq!(first.count(Thing::Footnote, Outcome::Dropped).get(), 2);
        assert_eq!(first.count(Thing::Table, Outcome::Moved).get(), 1);
        assert_eq!(first.entries().len(), 2);
    }

    /// `INTEROP-007`: a report reads as one sentence. It opens on what an
    /// import carried, then names each loss by outcome with its count.
    #[test]
    fn a_report_reads_as_one_sentence() {
        assert_eq!(import().sentence(), None);
        let mut report = import();
        report.record_count(Thing::Row, Outcome::Carried, Count::new(2));
        report.record(Thing::Column, Outcome::Carried);
        assert_eq!(
            report.sentence().as_deref(),
            Some("Imported 2 rows and 1 column")
        );
        report.record(Thing::Footnote, Outcome::Dropped);
        report.record(Thing::Footnote, Outcome::Dropped);
        report.record(Thing::Html, Outcome::Dropped);
        report.record(Thing::Table, Outcome::Moved);
        assert_eq!(
            report.sentence().as_deref(),
            Some("Imported 2 rows and 1 column; moved: table; dropped: footnote (2), HTML")
        );
    }

    /// An export carries no head, and a long list counts what it does not
    /// name.
    #[test]
    fn a_long_list_names_three_and_counts_the_rest() {
        let mut report = ConversionReport::new(Direction::Export, FileFormat::Svg);
        report.record(Thing::Row, Outcome::Carried);
        for thing in [
            Thing::Footnote,
            Thing::Html,
            Thing::Table,
            Thing::Gradient,
            Thing::Column,
        ] {
            report.record(thing, Outcome::Dropped);
        }
        assert_eq!(
            report.sentence().as_deref(),
            Some("Dropped: footnote, HTML, table, and 2 more")
        );
    }

    /// `INTEROP-004`: an error that quotes the file prints its length alone.
    #[test]
    fn an_error_prints_the_length_of_its_reason_alone() {
        let secret = "secret text".to_owned();
        for error in [
            InteropError::ParseFailed(secret.clone()),
            InteropError::Stopped(secret),
        ] {
            let printed = format!("{error:?}");
            assert!(!printed.contains("secret"), "{printed}");
            assert!(printed.contains("11"), "{printed}");
        }
    }

    /// `INTEROP-010`: a progress never counts past its total, and a
    /// conversion of no step reads as done.
    #[test]
    fn a_progress_never_passes_its_total() {
        let progress = ConversionProgress::new(Direction::Import, Count::new(9), Count::new(4));
        assert_eq!(progress.done(), progress.total());
        assert!((progress.ratio() - 1.0).abs() < f64::EPSILON);
        let half = ConversionProgress::new(Direction::Import, Count::new(2), Count::new(4));
        assert!((half.ratio() - 0.5).abs() < f64::EPSILON);
        let empty = ConversionProgress::new(Direction::Export, Count::new(0), Count::new(0));
        assert!((empty.ratio() - 1.0).abs() < f64::EPSILON);
    }

    /// `INTEROP-005`, `INTEROP-006`: an import is a write and an export is
    /// not, whatever the format names.
    #[test]
    fn an_import_writes_and_an_export_reads() {
        let import: InteropRequest<(), ()> = InteropRequest::Import(());
        let export: InteropRequest<(), ()> = InteropRequest::Export(());
        assert!(import.is_mutating());
        assert!(!export.is_mutating());
        assert_eq!(import.direction(), Direction::Import);
        assert_eq!(export.direction(), Direction::Export);
    }

    /// `INTEROP-011`: a failure reads as a warning naming its direction and
    /// its reason, and a clean import still says it landed.
    #[test]
    fn every_answer_but_a_progress_reads_as_a_message() {
        let failed: InteropAnswer<Thing> = InteropAnswer::failed(
            Direction::Export,
            InteropError::Stopped("the board is empty".to_owned()),
        );
        assert_eq!(
            failed.message().as_deref(),
            Some("Export failed: the conversion stopped: the board is empty")
        );
        assert!(failed.is_warning());
        let clean = InteropAnswer::ImportCompleted { report: import() };
        assert_eq!(
            clean.message().as_deref(),
            Some("Imported the Markdown file")
        );
        assert!(!clean.is_warning());
        let progress: InteropAnswer<Thing> = InteropAnswer::Progress(ConversionProgress::new(
            Direction::Import,
            Count::new(1),
            Count::new(2),
        ));
        assert_eq!(progress.message(), None);
    }
}
