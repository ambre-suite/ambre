//! The PDF export's walk over the pages
//! (`features/ambre-spec-prose-pdf-export.md section 3`).
//!
//! The walk reads the layout the engine already holds, one page at a time,
//! and never places a block again (`PROSE-IO-037`). Each page is drawn by
//! the screen's walk onto a recorded page, and the PDF converter module
//! writes the recorded pages (`PDF-009`). The walk runs in slices, as every
//! Prose export does. The Logic thread serves the other documents and the
//! view between two of them (`PROSE-IO-048`, `ARCH-030`).
//!
//! A page far from the view needs the runs the paginator dropped. A slice
//! shapes some of them, up to [`PRINT_SHAPE_ROOM`], and the job holds them.
//! The slice that finds every run of the page shaped draws the page, and
//! nothing else. So no slice both shapes and draws (`PROSE-IO-103`).
//!
//! A command that writes the document lands the running export first, and
//! a remote delta settles it first (`PROSE-IO-100`, `ARCH-031`). The layout
//! therefore holds the document as it stood when the export began, for the
//! length of the walk (`PROSE-IO-038`).

use std::collections::HashMap;

use ambre_bus::interop::{
    Direction, ExportedFile, FileFormat, InteropAnswer, InteropError, Outcome, Steps,
};
use ambre_client_core::DocumentLocale;
use ambre_convert::{Conversion, Converted, Converter};
use ambre_core::index::{Len, Pos, Span};
use ambre_core::{Count, Doc, PrintUnit, Pt, Px};
use ambre_draw::list::{
    ListRecorder, PrintDocument, PrintLink, PrintMeta, PrintPage, PrintTarget, PrintedPages,
    RecorderTables,
};
use ambre_draw::{Affine, DrawSurface, Rect};
use loro::TreeID;

use super::ProseState;
use crate::doc;
use crate::events::{Page, ProseEvent, ProseOutcome};
use crate::layout::run::{RunChar, ShapedRun};
use crate::message::{BlockType, ConversionReport, DocChar, PdfExport, ProseConstruct};
use crate::ui::ProseUiState;
use crate::view::print::{LinkArea, LinkSource, LinkTo, PrintedBlock, link_areas};

/// The characters one slice of a running PDF export shapes again, at most.
/// The first block of a slice is shaped whole, whatever its length. The
/// `print_slice` operation of `benches/instructions.rs` holds every slice of
/// a page to its budget row (`PROSE-IO-103`).
pub(super) const PRINT_SHAPE_ROOM: Len<RunChar> = Len::new(500);

/// How much shaping one step of a print may do.
#[derive(Clone, Copy)]
enum ShapeRoom {
    /// Up to this many characters, then the step ends.
    Chars(Len<RunChar>),
    /// Every run the page needs, then the step draws the page.
    Page,
}

/// What one step of a print did.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    /// It shaped runs of the next page, and drew nothing.
    Shaped,
    /// It drew the next page.
    Drew,
    /// It could do nothing: no paginator, or no page left.
    Stuck,
}

/// A PDF export between two slices: the view it prints from, the pages
/// recorded so far, and where each link leads.
pub(super) struct PrintJob {
    view: Box<ProseUiState>,
    document: PrintDocument,
    tables: RecorderTables,
    /// The spans of each block that lead somewhere, a table of contents
    /// aside.
    sources: HashMap<TreeID, Vec<LinkSource>>,
    /// The headings each table of contents lists, in the order of its lines.
    tocs: HashMap<TreeID, Vec<TreeID>>,
    /// Where each block of the flow starts: its page, and its top on it.
    places: HashMap<TreeID, (Pos<Page>, Px<Doc>)>,
    /// What the file carries and loses, beside the pages.
    report: ConversionReport,
    /// The file of each picture the export fetched, by address, read once
    /// for every page that shows it (`PROSE-IO-107`).
    pictures: HashMap<
        ambre_core::Validated<ambre_core::ContentHash>,
        std::sync::Arc<crate::message::PrintedPicture>,
    >,
    /// The runs shaped again for the next page, each block once. A block
    /// the page breaks keeps its run for the page after (`PROSE-IO-037`).
    /// `None` marks a block that shaped into no run.
    shaped: Vec<(TreeID, Option<ShapedRun>)>,
    /// The next page to draw.
    next: Pos<Page>,
    /// The pages drawn so far, as the progress counts them.
    printed: Count<Steps>,
    /// Every page of the layout, as the progress counts them.
    total: Count<Steps>,
}

impl PrintJob {
    /// The pages drawn so far.
    pub(super) fn printed(&self) -> Count<Steps> {
        self.printed
    }

    /// Every page of the layout.
    pub(super) fn total(&self) -> Count<Steps> {
        self.total
    }

    /// The recorded file and its report, once every page is drawn.
    pub(super) fn finish(self) -> (PrintDocument, ConversionReport) {
        (self.document, self.report)
    }
}

impl ProseState {
    /// Start the walk of a PDF export under `options`.
    ///
    /// # Errors
    ///
    /// [`InteropError::Stopped`] when the layout names a script it had no
    /// face for: its page numbers are not ones to hand out (`PROSE-112`).
    pub(super) fn print_start(
        &mut self,
        options: &PdfExport,
        fetched: HashMap<
            ambre_core::Validated<ambre_core::ContentHash>,
            ambre_core::Payload<ambre_core::MediaBytes>,
        >,
    ) -> Result<PrintJob, InteropError> {
        // The layout the screen shows, brought up to the document. A
        // document already laid out places nothing again.
        let layout = self.paginate(&[]).clone();
        if !layout.missing_scripts.is_empty() {
            return Err(InteropError::Stopped(ambre_core::StopCause::FontsLoading));
        }
        let tree = doc::blocks(&self.doc);
        let body_top = layout.body_top();
        let places = layout
            .blocks
            .iter()
            .map(|block| {
                (
                    block.block_id,
                    (block.page_start, body_top + block.y_in_page),
                )
            })
            .collect();
        let tocs = layout
            .blocks
            .iter()
            .filter(|block| block.block_type == BlockType::Toc)
            .filter_map(|block| {
                let toc = crate::toc::build(&self.doc, &tree, &block.block_id, &HashMap::new())?;
                let headings = toc
                    .entries
                    .iter()
                    .map(|entry| crate::events::tree_id(entry.block_id))
                    .collect();
                Some((block.block_id, headings))
            })
            .collect();
        let mut report = ConversionReport::new(Direction::Export, FileFormat::Pdf);
        // Each picture goes into the file as its stored bytes. One whose
        // bytes did not come back is a placeholder, and the report names it
        // (`PROSE-IO-008`).
        let pictures: HashMap<_, _> = fetched
            .into_iter()
            .filter_map(|(address, bytes)| {
                Some((
                    address,
                    std::sync::Arc::new(crate::picture::printed(&bytes)?),
                ))
            })
            .collect();
        let missing = layout
            .blocks
            .iter()
            .filter(|block| block.block_type == BlockType::Image)
            .filter(|block| {
                crate::picture::read_image(&tree, &block.block_id)
                    .and_then(|record| record.address)
                    .is_none_or(|address| !pictures.contains_key(&address))
            })
            .count();
        if missing > 0 {
            report.record_count(
                ProseConstruct::Feature(crate::message::UnsupportedFeature::PictureMissing),
                Outcome::Approximated,
                Count::of(missing),
            );
        }
        let title = doc::read_title(&self.doc);
        let stored = doc::read_locale(&self.doc);
        let document_locale =
            DocumentLocale::from_register(stored.as_ref().map(ambre_core::Validated::as_str));
        let mut document = PrintDocument::new(PrintPage {
            width: layout.page_width.to_pt(),
            height: layout.page_height.to_pt(),
        });
        document.meta = PrintMeta {
            title: (!title.is_empty()).then_some(title),
            language: ambre_client_core::locale::resolve(Some(&options.language), &document_locale),
        };
        Ok(PrintJob {
            view: Box::new(ProseUiState::printing(&self.suggestions_event())),
            document,
            tables: RecorderTables::default(),
            sources: self.link_sources(),
            tocs,
            places,
            report,
            pictures,
            shaped: Vec::new(),
            next: Pos::<Page>::ZERO,
            printed: Count::ZERO,
            // A PDF export takes one step a page.
            total: Count::of(layout.page_count.to_usize()),
        })
    }

    /// Every span of the document that leads somewhere: each link a reader
    /// may open, and each cross reference.
    fn link_sources(&self) -> HashMap<TreeID, Vec<LinkSource>> {
        let mut sources: HashMap<TreeID, Vec<LinkSource>> = HashMap::new();
        for link in crate::link::links(&self.doc) {
            // A target the view would not open is no link in the file
            // either (`PROSE-UI-153`).
            if !crate::link::openable(&link.url) {
                continue;
            }
            sources
                .entry(crate::events::tree_id(link.block_id))
                .or_default()
                .push(LinkSource {
                    span: crate::view::answered_span(link.start, link.end),
                    to: LinkTo::Address(link.url),
                });
        }
        for anchor in doc::anchors(
            &self.doc,
            crate::mark::MarkRoot::CrossRef,
            crate::xref::decode,
        ) {
            let (target, _) = anchor.value;
            let carrier = anchor.offset + Len::<DocChar>::new(1);
            sources.entry(anchor.block).or_default().push(LinkSource {
                span: crate::view::answered_span(anchor.offset, carrier),
                to: LinkTo::Block(target),
            });
        }
        sources
    }

    /// Draw up to `room` more pages of `job`, each with every run it needs
    /// shaped first. The run of a block the paginator dropped is shaped for
    /// the print alone (`PROSE-IO-037`).
    pub(super) fn print_pages(&mut self, job: &mut PrintJob, room: Count<Steps>) {
        let stop = job.printed.plus(room).min(job.total);
        while job.printed < stop {
            if self.print_step(job, ShapeRoom::Page) == Step::Stuck {
                break;
            }
        }
    }

    /// Take one slice of `job`: shape up to [`PRINT_SHAPE_ROOM`] of the
    /// runs the next page needs, or draw that page once none is left
    /// (`PROSE-IO-103`).
    pub(super) fn print_slice(&mut self, job: &mut PrintJob) {
        let _ = self.print_step(job, ShapeRoom::Chars(PRINT_SHAPE_ROOM));
    }

    /// Shape what `room` allows of the runs the next page needs. Draw the
    /// page when no run is left to shape, or when `room` takes the page
    /// whole.
    fn print_step(&mut self, job: &mut PrintJob, room: ShapeRoom) -> Step {
        let Some(paginator) = self.paginator.as_mut() else {
            return Step::Stuck;
        };
        if job.printed >= job.total {
            return Step::Stuck;
        }
        let page = job.next;
        let wanted: Vec<(TreeID, Len<RunChar>)> = paginator
            .print_dropped(page.through(page))
            .into_iter()
            .filter(|(id, _)| !job.shaped.iter().any(|(held, _)| held == id))
            .collect();
        if !wanted.is_empty() {
            let ids = match room {
                ShapeRoom::Page => wanted.iter().map(|(id, _)| *id).collect(),
                ShapeRoom::Chars(limit) => within(&wanted, limit),
            };
            let mut runs: HashMap<TreeID, ShapedRun> = paginator
                .print_reshape(&self.doc, &ids)
                .into_iter()
                .collect();
            job.shaped
                .extend(ids.iter().map(|id| (*id, runs.remove(id))));
            if let ShapeRoom::Chars(_) = room {
                return Step::Shaped;
            }
        }
        let shaped = job
            .shaped
            .iter()
            .filter_map(|(id, run)| Some((*id, run.clone()?)))
            .collect();
        job.view
            .take_print_window(paginator.print_window(page.through(page), shaped));
        // The pictures this page shows, each as the file the export fetched.
        job.view.pictures = job
            .view
            .shaped
            .pictures
            .values()
            .filter_map(|shape| {
                let address = shape.address.clone()?;
                let file = job.pictures.get(&address)?;
                Some((
                    address,
                    crate::message::PictureShown::Printed(std::sync::Arc::clone(file)),
                ))
            })
            .collect();
        let scale = Affine::scale(1.0 / f64::from(Pt::DOC_PX));
        let mut recorder = ListRecorder::new(&mut job.document, &mut job.tables);
        recorder.push_transform(scale);
        let printed = job.view.print_page(&mut recorder, page);
        recorder.pop_transform();
        recorder.finish();
        // The sources of the blocks the page drew, not of the document.
        let mut sources: HashMap<TreeID, Vec<LinkSource>> = printed
            .iter()
            .filter_map(|block| Some((block.block, job.sources.get(&block.block)?.clone())))
            .collect();
        for (block, links) in toc_sources(&job.tocs, &printed) {
            sources.entry(block).or_default().extend(links);
        }
        let on = page.as_axis::<PrintedPages>();
        let links = link_areas(&printed, &sources)
            .into_iter()
            .filter_map(|area| print_link(on, area, &job.places))
            .collect::<Vec<_>>();
        job.document.links.extend(links);
        job.next += Len::<Page>::new(1);
        job.printed = job.printed.incremented();
        // A block the page broke shows on the next page too: its run stays.
        let next = job.next;
        let carried = paginator.print_dropped(next.through(next));
        job.shaped
            .retain(|(id, _)| carried.iter().any(|(next, _)| next == id));
        Step::Drew
    }

    /// Hand the recorded file to the PDF module, and answer it with its
    /// report (`PDF-015`). The export wrote nothing to the document.
    pub(super) fn write_pdf(
        document: PrintDocument,
        report: ConversionReport,
        converter: &Converter,
    ) -> ProseOutcome {
        let filename = ambre_client_core::ExportFilename::from_title(
            document.meta.title.as_deref().unwrap_or_default(),
            "document",
            ambre_client_core::ExportExtension::Pdf,
        );
        let answer = match converter.convert(&Conversion::WritePdf { document }) {
            Ok(Converted::File(data)) => InteropAnswer::ExportReady {
                file: ExportedFile {
                    filename,
                    media_type: ambre_core::MediaType::from("application/pdf"),
                    data,
                },
                report,
            },
            Ok(_) => InteropAnswer::failed(
                Direction::Export,
                InteropError::Stopped(ambre_core::StopCause::ConverterAnswered),
            ),
            Err(e) => InteropAnswer::failed(Direction::Export, crate::interop::stopped(&e)),
        };
        ProseOutcome::display_only(vec![ProseEvent::Interop(answer)])
    }
}

/// The first blocks of `wanted` whose characters fit in `limit`, and at
/// least the first one: a block longer than `limit` is shaped alone.
fn within(wanted: &[(TreeID, Len<RunChar>)], limit: Len<RunChar>) -> Vec<TreeID> {
    let mut used = Len::<RunChar>::ZERO;
    wanted
        .iter()
        .enumerate()
        .take_while(|(index, (_, len))| {
            used += *len;
            *index == 0 || used <= limit
        })
        .map(|(_, (id, _))| *id)
        .collect()
}

/// The lines of each table of contents `printed` drew, each leading to the
/// heading it lists. Line `n` of the run is entry `n` (`crate::toc::lines`).
fn toc_sources(
    tocs: &HashMap<TreeID, Vec<TreeID>>,
    printed: &[PrintedBlock],
) -> Vec<(TreeID, Vec<LinkSource>)> {
    printed
        .iter()
        .filter_map(|block| {
            let headings = tocs.get(&block.block)?;
            let mut start = Pos::<RunChar>::ZERO;
            let links = block
                .run
                .data()
                .text()
                .split('\n')
                .zip(headings)
                .map(|(line, heading)| {
                    let end = start + ambre_core::char_len(line).as_axis::<RunChar>();
                    let span = Span::<RunChar>::new(start, end);
                    // The line break the entry ends on.
                    start = end + Len::<RunChar>::new(1);
                    LinkSource {
                        span,
                        to: LinkTo::Block(*heading),
                    }
                })
                .collect();
            Some((block.block, links))
        })
        .collect()
}

/// The link of `area` on the page `on`, in points. A link to a block the
/// layout does not place leads nowhere, and writes nothing.
fn print_link(
    on: Pos<PrintedPages>,
    area: LinkArea,
    places: &HashMap<TreeID, (Pos<Page>, Px<Doc>)>,
) -> Option<PrintLink> {
    let target = match area.to {
        LinkTo::Address(url) => PrintTarget::Address(url),
        LinkTo::Block(block) => {
            let (page, top) = places.get(&block)?;
            PrintTarget::Place {
                page: page.as_axis::<PrintedPages>(),
                y: top.to_pt(),
            }
        }
    };
    let points = 1.0 / f64::from(Pt::DOC_PX);
    let rect = area.area;
    Some(PrintLink {
        page: on,
        area: Rect::new(
            rect.x0 * points,
            rect.y0 * points,
            rect.x1 * points,
            rect.y1 * points,
        ),
        target,
    })
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;
    use crate::events::{BlockId, BlockInit, CrossRefKind, HeadingLevel, ProseCommand};
    use crate::message::{ExportFormat, TocNumbering};
    use crate::view::tests::append;
    use ambre_bus::interop::InteropRequest;
    use ambre_client_core::FileBytes;
    use ambre_core::Payload;

    /// A document on the faces the application ships.
    fn state() -> ProseState {
        let mut state = ProseState::with_peer(ambre_core::PeerId::new(1)).expect("a document");
        state.load_faces(&crate::layout::fonts::shipped_faces());
        state
    }

    fn pdf_command(language: DocumentLocale) -> ProseCommand {
        ProseCommand::Interop(InteropRequest::Export(ExportFormat::Pdf(PdfExport {
            language,
        })))
    }

    /// The file a whole export answers, through the PDF engine the host
    /// links in place.
    fn export(state: &mut ProseState, language: DocumentLocale) -> Payload<FileBytes> {
        let outcome = state.handle_converted_in_family(pdf_command(language));
        file_of(&outcome).unwrap_or_else(|| panic!("no file: {:?}", outcome.display))
    }

    fn file_of(outcome: &ProseOutcome) -> Option<Payload<FileBytes>> {
        outcome.display.iter().find_map(|event| match event {
            ProseEvent::Interop(InteropAnswer::ExportReady { file, .. }) => Some(file.data.clone()),
            _ => None,
        })
    }

    fn parsed(bytes: &Payload<FileBytes>) -> lopdf::Document {
        lopdf::Document::load_mem(bytes.as_bytes()).expect("the file parses")
    }

    /// The text of each page of the file, its spaces dropped.
    fn page_texts(bytes: &Payload<FileBytes>) -> Vec<String> {
        pdf_extract::extract_text_from_mem_by_pages(bytes.as_bytes())
            .expect("the file reads")
            .iter()
            .map(|page| page.split_whitespace().collect())
            .collect()
    }

    /// `count` paragraphs, each opening on its own marker word and long
    /// enough to wrap.
    fn paragraphs(state: &mut ProseState, count: usize) -> Vec<(BlockId, String)> {
        let filler = " the quick brown fox jumps over the lazy dog".repeat(6);
        (0..count)
            .map(|index| {
                let marker = format!("marker{index:03}");
                let block = append(state, BlockInit::Paragraph, &format!("{marker}{filler}"));
                (block, marker)
            })
            .collect()
    }

    /// The page the layout starts `block` on.
    fn page_of(state: &mut ProseState, block: BlockId) -> Pos<Page> {
        let id = crate::events::tree_id(block);
        state
            .paginate(&[])
            .blocks
            .iter()
            .find(|placed| placed.block_id == id)
            .map(|placed| placed.page_start)
            .expect("the block is placed")
    }

    /// `PROSE-IO-029`, `PDF-009`: the file holds the pages the layout holds,
    /// at the document's paper size. Each page holds the paragraphs the
    /// layout starts on it, and none other.
    #[test]
    fn a_pdf_page_carries_the_pages_the_screen_shows() {
        let mut state = state();
        let blocks = paragraphs(&mut state, 40);
        let layout = state.paginate(&[]).clone();
        assert!(
            layout.page_count.to_usize() >= 3,
            "the document spans pages"
        );
        let bytes = export(&mut state, DocumentLocale::NONE);
        let file = parsed(&bytes);
        assert_eq!(file.get_pages().len(), layout.page_count.to_usize());
        let page = *file.get_pages().values().next().expect("a page");
        let media = file
            .get_dictionary(page)
            .and_then(|page| page.get(b"MediaBox"))
            .and_then(lopdf::Object::as_array)
            .expect("a media box")
            .iter()
            .map(|n| n.as_float().expect("a number"))
            .collect::<Vec<_>>();
        let points = |px: Px<Doc>| px.to_pt().get();
        assert!(
            (media[2] - points(layout.page_width)).abs() < 0.01,
            "{media:?}"
        );
        assert!(
            (media[3] - points(layout.page_height)).abs() < 0.01,
            "{media:?}"
        );
        let texts = page_texts(&bytes);
        for (block, marker) in &blocks {
            let on = page_of(&mut state, *block).to_usize();
            for (index, text) in texts.iter().enumerate() {
                assert_eq!(
                    text.contains(marker.as_str()),
                    index == on,
                    "{marker} is laid out on page {on}, read on page {index}"
                );
            }
        }
    }

    /// The annotations of every page: the page it sits on, the URI it
    /// opens or the page its destination names.
    fn links(file: &lopdf::Document) -> Vec<(usize, Option<String>, Option<usize>)> {
        let pages: Vec<lopdf::ObjectId> = file.get_pages().values().copied().collect();
        let deref = |object: &lopdf::Object| match object {
            lopdf::Object::Reference(id) => file.get_object(*id).ok().cloned(),
            other => Some(other.clone()),
        };
        let mut out = Vec::new();
        for (index, page) in pages.iter().enumerate() {
            for annotation in file.get_page_annotations(*page).unwrap_or_default() {
                let uri = annotation
                    .get(b"A")
                    .ok()
                    .and_then(deref)
                    .and_then(|action| action.as_dict().ok().cloned())
                    .and_then(|action| {
                        let uri = action.get(b"URI").ok()?.as_str().ok()?.to_vec();
                        Some(String::from_utf8_lossy(&uri).into_owned())
                    });
                let destination = annotation
                    .get(b"Dest")
                    .ok()
                    .and_then(deref)
                    .and_then(|dest| dest.as_array().ok().cloned())
                    .and_then(|dest| dest.first()?.as_reference().ok())
                    .and_then(|target| pages.iter().position(|page| *page == target));
                out.push((index, uri, destination));
            }
        }
        out
    }

    /// `PROSE-IO-031`: a table-of-contents entry and a cross reference lead
    /// to the page of their heading, and an address opens. An address a
    /// reader may not open is no link in the file.
    #[test]
    fn every_link_the_document_carries_reaches_the_file_live() {
        let mut state = state();
        let first = *state.root_blocks().first().expect("a root block");
        state.handle_command(ProseCommand::InsertToc {
            parent_id: None,
            after_id: Some(first),
            max_level: HeadingLevel::H2,
            numbered: TocNumbering::default(),
        });
        let source = append(&mut state, BlockInit::Paragraph, "See the site, or here.");
        let unsafe_link = append(&mut state, BlockInit::Paragraph, "Run this script now.");
        let _ = paragraphs(&mut state, 30);
        let heading = append(
            &mut state,
            BlockInit::Heading {
                level: HeadingLevel::H1,
            },
            "Details",
        );
        for (block, end, url) in [
            (source, 12, "https://example.org/"),
            (unsafe_link, 15, "javascript:alert(1)"),
        ] {
            state.handle_command(ProseCommand::ApplyMark {
                block_id: block,
                range: Span::<DocChar>::new(Pos::<DocChar>::new(4), Pos::<DocChar>::new(end)),
                key: "a".into(),
                value: Some(crate::mark::test_value(url)),
            });
        }
        state.handle_command(ProseCommand::InsertCrossRef {
            block_id: source,
            pos: Pos::<DocChar>::new(21),
            target_block_id: heading,
            kind: CrossRefKind::PageNumber,
        });
        let target = page_of(&mut state, heading).to_usize();
        assert!(target > 0, "the heading sits past the first page");
        let found = links(&parsed(&export(&mut state, DocumentLocale::NONE)));
        let addresses: Vec<&str> = found.iter().filter_map(|l| l.1.as_deref()).collect();
        assert_eq!(addresses, ["https://example.org/"], "{found:?}");
        let inside: Vec<usize> = found.iter().filter_map(|l| l.2).collect();
        assert_eq!(
            inside,
            [target, target],
            "the entry and the reference: {found:?}"
        );
    }

    /// The language the catalog of an exported file declares.
    fn language(bytes: &Payload<FileBytes>) -> Option<String> {
        let file = parsed(bytes);
        let lang = file
            .catalog()
            .ok()?
            .get(b"Lang")
            .ok()?
            .as_str()
            .ok()?
            .to_vec();
        Some(String::from_utf8_lossy(&lang).into_owned())
    }

    fn locale(tag: &str) -> DocumentLocale {
        DocumentLocale::parse(tag).expect("a tag")
    }

    /// `PROSE-IO-032`: the request's language wins, the document's stands
    /// in for a request that names none.
    #[test]
    fn the_file_takes_its_language_from_the_request_then_the_document() {
        let mut state = state();
        let _ = paragraphs(&mut state, 1);
        state.handle_command(ProseCommand::SetDocumentLocale {
            locale: locale("de-CH"),
        });
        assert_eq!(
            language(&export(&mut state, locale("fr-CA"))).as_deref(),
            Some("fr-CA")
        );
        assert_eq!(
            language(&export(&mut state, DocumentLocale::NONE)).as_deref(),
            Some("de-CH")
        );
    }

    /// `PROSE-IO-035`: with no language named anywhere, the file declares
    /// none.
    #[test]
    fn a_file_with_no_language_named_declares_none() {
        let mut state = state();
        let _ = paragraphs(&mut state, 1);
        assert_eq!(language(&export(&mut state, DocumentLocale::NONE)), None);
    }

    /// `PROSE-112`: a layout that measured a script with no face hands out
    /// no page number, so the export refuses with the reason.
    #[test]
    fn a_layout_missing_a_script_exports_no_file() {
        let mut state = state();
        let _ = append(
            &mut state,
            BlockInit::Paragraph,
            &"\u{4e00}\u{4e8c}\u{4e09}".repeat(20),
        );
        assert!(!state.paginate(&[]).missing_scripts.is_empty());
        let outcome = state.handle_converted_in_family(pdf_command(DocumentLocale::NONE));
        assert_eq!(file_of(&outcome), None);
        assert!(
            outcome
                .display
                .iter()
                .any(|event| matches!(event, ProseEvent::Interop(InteropAnswer::Failed { .. })))
        );
    }

    /// `PDF-011`: two exports of one document are the same bytes, and so are
    /// the exports of two peers holding the same faces. A peer holding no
    /// face is the control: it writes another file.
    #[test]
    fn two_peers_holding_the_same_faces_export_one_file() {
        let mut alice = state();
        let _ = paragraphs(&mut alice, 12);
        let snapshot = alice
            .document()
            .export(loro::ExportMode::Snapshot)
            .expect("a snapshot");
        let mut bob = state();
        bob.document()
            .import(&snapshot)
            .expect("the snapshot merges");
        let once = export(&mut alice, DocumentLocale::NONE);
        assert_eq!(once, export(&mut alice, DocumentLocale::NONE));
        assert_eq!(once, export(&mut bob, DocumentLocale::NONE));
        let mut bare = ProseState::with_peer(ambre_core::PeerId::new(3)).expect("a document");
        bare.document()
            .import(&snapshot)
            .expect("the snapshot merges");
        let outcome = bare.handle_converted_in_family(pdf_command(DocumentLocale::NONE));
        assert_ne!(file_of(&outcome), Some(once));
    }

    /// `PDF-013`: a page holding text, a table and a heading records glyphs
    /// and vector rules, and no image.
    #[test]
    fn a_printed_page_paints_no_image() {
        let mut state = state();
        let _ = paragraphs(&mut state, 2);
        let last = *state.root_blocks().last().expect("a root block");
        state.handle_command(ProseCommand::InsertTable {
            parent_id: None,
            after_id: Some(last),
            rows: ambre_core::index::Len::new(2),
            cols: ambre_core::index::Len::new(2),
        });
        let mut job = state
            .print_start(&PdfExport::default(), HashMap::new())
            .expect("the export starts");
        let total = job.total();
        state.print_pages(&mut job, total);
        let (document, _) = job.finish();
        let ops: Vec<&ambre_draw::list::DrawOp> = document.pages.iter().flatten().collect();
        use ambre_draw::list::DrawOp;
        assert!(ops.iter().any(|op| matches!(op, DrawOp::Glyphs { .. })));
        assert!(
            ops.iter()
                .any(|op| matches!(op, DrawOp::StrokeRect { .. } | DrawOp::StrokeLine { .. }))
        );
        assert!(!ops.iter().any(|op| matches!(op, DrawOp::Image { .. })));
        assert!(document.images.is_empty());
    }

    /// `PDF-017`: no slice of an export answers a delta, and the document's
    /// version reads the same after it.
    #[test]
    fn a_pdf_export_leaves_the_oplog_unchanged() {
        let mut state = state();
        let _ = paragraphs(&mut state, 40);
        let before = state.document().oplog_vv();
        let outcome = state.handle_converted_in_family(pdf_command(DocumentLocale::NONE));
        assert!(file_of(&outcome).is_some());
        assert!(outcome.update.is_none());
        assert_eq!(state.document().oplog_vv(), before);
    }

    /// `PROSE-IO-037`, `PROSE-IO-039`: an export places nothing again. It
    /// leaves the view as it was: the Logic thread has no new layout to hand
    /// it, and holds the same runs. The document is long
    /// enough that runs far from the view were dropped, so the export shapes
    /// them for itself.
    #[test]
    fn an_export_lays_nothing_out_again_and_leaves_the_view_alone() {
        let mut state = state();
        let _ = paragraphs(&mut state, 400);
        assert!(state.layout_event().is_some(), "the view takes its layout");
        let held = state.runs_held();
        let laid = state.full_layout().cloned().expect("a layout");
        assert!(
            held < Count::of(laid.blocks.len()),
            "runs far from the view are dropped: {held:?} of {} on {:?}",
            laid.blocks.len(),
            laid.page_count
        );
        let _ = export(&mut state, DocumentLocale::NONE);
        assert!(state.layout_event().is_none(), "the view gets nothing new");
        assert_eq!(state.full_layout().map(|l| &l.blocks), Some(&laid.blocks));
        assert_eq!(state.runs_held(), held);
    }

    /// `PROSE-IO-038`: an edit made while an export runs lands after it: the
    /// file holds the document as the export began.
    #[test]
    fn an_edit_during_an_export_does_not_reach_the_file() {
        let mut state = state();
        let blocks = paragraphs(&mut state, 40);
        let converter = Converter::load(
            ambre_core::ConverterFamily::Pdf,
            &ambre_core::Payload::new(Vec::new()),
        )
        .expect("the host links the engine");
        let first = state.convert_in_place(pdf_command(DocumentLocale::NONE), Ok(&converter));
        assert!(state.has_pending_work(), "the export runs in slices");
        let (last, _) = blocks.last().expect("a paragraph");
        let typed = state.handle_command(ProseCommand::InsertText {
            block_id: *last,
            pos: Pos::<DocChar>::ZERO,
            text: "typedwhilewriting ".to_owned(),
        });
        let mut outcome = state.drained(first, &converter);
        outcome.display.extend(typed.display);
        let bytes = file_of(&outcome).expect("the export finished");
        let read: String = page_texts(&bytes).concat();
        assert!(read.contains("marker039"), "the file holds the document");
        assert!(
            !read.contains("typedwhilewriting"),
            "the edit reached the file"
        );
        assert!(
            state
                .text_of(last)
                .is_some_and(|text| text.starts_with("typedwhilewriting"))
        );
    }

    /// A document of two paragraphs and one picture of `address`, 200 by
    /// 100 pixels, between them.
    fn with_picture(address: &ambre_core::Validated<ambre_core::ContentHash>) -> ProseState {
        let mut state = state();
        let first = paragraphs(&mut state, 2)[0].0;
        state.handle_command(ProseCommand::InsertImage {
            parent_id: None,
            after_id: Some(first),
            content_hash: address.clone(),
            alt: ambre_core::Validated::new("a picture"),
            size: ambre_core::Size::new(ambre_core::Px::new(200.0), ambre_core::Px::new(100.0)),
        });
        state
    }

    /// Print `state` whole over the pictures `fetched`.
    fn printed(
        state: &mut ProseState,
        fetched: HashMap<
            ambre_core::Validated<ambre_core::ContentHash>,
            ambre_core::Payload<ambre_core::MediaBytes>,
        >,
    ) -> (PrintDocument, ConversionReport) {
        let mut job = state
            .print_start(&PdfExport::default(), fetched)
            .expect("the export starts");
        let total = job.total();
        state.print_pages(&mut job, total);
        job.finish()
    }

    fn address() -> ambre_core::Validated<ambre_core::ContentHash> {
        ambre_core::Validated::<ambre_core::ContentHash>::parse("d".repeat(64)).expect("a hash")
    }

    /// `PROSE-IO-107`: a PDF export draws a picture whose bytes came back in
    /// the box its page gives it. It draws the stored file, which the written
    /// PDF holds as an image.
    #[test]
    fn a_printed_page_draws_a_fetched_picture() {
        use ambre_draw::list::DrawOp;
        let mut state = with_picture(&address());
        let bytes = crate::logic::tests::png(40, 20);
        let fetched = [(address(), ambre_core::Payload::new(bytes.clone()))]
            .into_iter()
            .collect();
        let (document, report) = printed(&mut state, fetched);
        let dests: Vec<Rect> = document
            .pages
            .iter()
            .flatten()
            .filter_map(|op| match op {
                DrawOp::EncodedImage { dest, .. } => Some(*dest),
                _ => None,
            })
            .collect();
        assert_eq!(
            dests,
            [Rect::new(0.0, 0.0, 1.0, 1.0)],
            "one picture, drawn once"
        );
        assert_eq!(document.encoded.len(), 1);
        assert_eq!(document.encoded[0].bytes.as_bytes(), bytes.as_slice());
        assert!(
            report
                .entries()
                .iter()
                .all(|e| e.outcome != Outcome::Approximated)
        );
        let converter = ambre_convert::Converter::load(
            ambre_core::ConverterFamily::Pdf,
            &ambre_core::Payload::new(Vec::new()),
        )
        .expect("the host links the engine");
        let outcome = ProseState::write_pdf(document, report, &converter);
        let file = file_of(&outcome).expect("a file");
        let pdf = parsed(&file);
        let images = pdf
            .objects
            .values()
            .filter_map(|object| object.as_stream().ok())
            .filter(|stream| {
                stream
                    .dict
                    .get(b"Subtype")
                    .and_then(lopdf::Object::as_name)
                    .is_ok_and(|name| name == b"Image")
            })
            .count();
        assert!(images >= 1, "the file holds the picture");
    }

    /// `PROSE-FEAT-328`, `PROSE-IO-107`: a JPEG stored on its side is drawn
    /// turned, through the transform its orientation states.
    #[test]
    fn a_printed_picture_stands_as_it_was_taken() {
        use ambre_draw::list::DrawOp;
        let mut state = with_picture(&address());
        let stored = crate::picture::stripped(&crate::picture::tests::media(
            &crate::picture::tests::jpeg_with_metadata(),
        ))
        .expect("a JPEG strips");
        let fetched = [(address(), stored)].into_iter().collect();
        let (document, _) = printed(&mut state, fetched);
        let ops: Vec<&DrawOp> = document.pages.iter().flatten().collect();
        let at = ops
            .iter()
            .position(|op| matches!(op, DrawOp::EncodedImage { .. }))
            .expect("the picture is drawn");
        let Some(DrawOp::PushTransform(turn)) = ops.get(at - 1) else {
            panic!("a transform before the picture");
        };
        let [a, b, c, d, _, _] = turn.as_coeffs();
        assert!(a.abs() < 1e-9 && d.abs() < 1e-9, "a quarter turn: {turn:?}");
        assert!(b > 0.0 && c < 0.0, "clockwise: {turn:?}");
        assert_eq!(document.encoded[0].format, ambre_draw::EncodedFormat::Jpeg);
    }

    /// `PROSE-IO-008`: a picture whose bytes did not come back is a
    /// placeholder on its page and a line in the report.
    #[test]
    fn a_picture_without_its_bytes_is_a_placeholder_in_the_pdf() {
        use ambre_draw::list::DrawOp;
        let mut state = with_picture(&address());
        let (document, report) = printed(&mut state, HashMap::new());
        assert!(
            !document
                .pages
                .iter()
                .flatten()
                .any(|op| matches!(op, DrawOp::EncodedImage { .. }))
        );
        assert_eq!(
            report
                .count(
                    ProseConstruct::Feature(crate::message::UnsupportedFeature::PictureMissing),
                    Outcome::Approximated
                )
                .get(),
            1
        );
    }

    /// The characters of the runs `job` holds for the next page.
    fn held_chars(job: &PrintJob) -> Vec<Len<RunChar>> {
        job.shaped
            .iter()
            .filter_map(|(_, run)| Some(run.as_ref()?.data().len()))
            .collect()
    }

    /// `PROSE-IO-103`: a page far from the view is shaped over slices. Each
    /// shapes within its room or one block alone. The slice that draws the
    /// page shapes nothing, and a page near the view takes one slice.
    #[test]
    fn a_far_page_is_shaped_over_slices_then_drawn_alone() {
        let mut state = state();
        let _ = paragraphs(&mut state, 400);
        assert!(state.layout_event().is_some(), "the view takes its layout");
        let mut job = state
            .print_start(&PdfExport::default(), HashMap::new())
            .expect("the export starts");
        let mut slices_per_page = vec![0_usize];
        while job.printed() < job.total() {
            let (printed, before) = (job.printed(), held_chars(&job));
            state.print_slice(&mut job);
            let after = held_chars(&job);
            *slices_per_page.last_mut().expect("a page") += 1;
            if job.printed() > printed {
                assert!(
                    after.len() <= before.len(),
                    "a drawing slice shapes nothing"
                );
                slices_per_page.push(0);
                continue;
            }
            let shaped = after
                .get(before.len()..)
                .expect("a shaping slice adds runs");
            let chars = shaped
                .iter()
                .fold(Len::<RunChar>::ZERO, |sum, len| sum + *len);
            assert!(!shaped.is_empty(), "a slice shapes or draws");
            assert!(
                shaped.len() == 1 || chars <= PRINT_SHAPE_ROOM,
                "{chars:?} shaped in one slice"
            );
        }
        assert_eq!(slices_per_page.first(), Some(&1), "page 0 is near the view");
        let far = slices_per_page.get(20).copied().unwrap_or_default();
        assert!(far >= 3, "a far page takes {far} slices");
    }

    /// `PROSE-IO-104`: an export in slices writes the file a whole export
    /// writes. So does an export that lands whole one slice into a far page.
    #[test]
    fn an_export_in_slices_writes_the_file_a_whole_export_writes() {
        let mut state = state();
        let _ = paragraphs(&mut state, 400);
        assert!(state.layout_event().is_some(), "the view takes its layout");
        let converter = Converter::load(
            ambre_core::ConverterFamily::Pdf,
            &ambre_core::Payload::new(Vec::new()),
        )
        .expect("the host links the engine");
        let sliced = export(&mut state, DocumentLocale::NONE);
        let first = state.convert_in_place(pdf_command(DocumentLocale::NONE), Ok(&converter));
        let mut whole = state.settle_in_place(Some(&converter));
        whole.display.extend(first.display);
        assert_eq!(file_of(&whole), Some(sliced.clone()));
        let _ = state.convert_in_place(pdf_command(DocumentLocale::NONE), Ok(&converter));
        let progress = |outcome: &ProseOutcome| {
            outcome.display.iter().find_map(|event| match event {
                ProseEvent::Interop(InteropAnswer::Progress(progress)) => Some(*progress),
                _ => None,
            })
        };
        // Up to a far page, then one slice into it: some of its runs shaped.
        while progress(&state.resume_in_place(Some(&converter)))
            .is_some_and(|p| p.done().get() < 20)
        {}
        let mid = state.resume_in_place(Some(&converter));
        assert_eq!(progress(&mid).map(|p| p.done().get()), Some(20));
        let landed = state.settle_in_place(Some(&converter));
        assert_eq!(file_of(&landed), Some(sliced));
    }
}
