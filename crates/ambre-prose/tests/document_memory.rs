// ambre-prose document memory test (tier `host`, `PROSE-IO-119`).
// ============================================================================
// The ceiling of the `Prose document, held after an import` row of
// `ambre-spec-budgets.md section 6`. It reads the resident size a document
// keeps once its import ends, as a multiple of its Markdown file.
// The test is alone in its binary, so no other test shares the process it
// measures. Linux alone reports the resident size, and CI runs on Linux.
// ============================================================================
#![cfg(target_os = "linux")]

use ambre_bus::interop::{InteropAnswer, InteropRequest};
use ambre_prose::events::ProseEvent;
use ambre_prose::interop::markdown::MAX_SIZE;
use ambre_prose::logic::ProseState;
use ambre_prose::message::{ImportFormat, ProseCommand, ProseImport, TimeZone};

/// The ceiling, in multiples of the file. This test reads about 16.4 times:
/// the 14.8 times a counting allocator reads, and what the allocator keeps
/// of the import's freed memory. Before the undo step dropped its copy of
/// the text, it read about 18 times.
const CEILING: usize = 17;

/// One kibibyte, the unit `/proc/self/status` reports in.
const KIB: usize = 1024;

/// The current resident size of this process, in bytes, or `None` when the
/// status does not name it.
fn resident() -> Option<usize> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|rest| {
            rest.trim()
                .trim_end_matches("kB")
                .trim()
                .parse::<usize>()
                .ok()
        })
        .map(|kib| kib * KIB)
}

/// A Markdown file of 400-character paragraphs, the file the row measures.
fn novel(size: usize) -> Vec<u8> {
    let words = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod \
                 tempor incididunt ut labore et dolore magna aliqua. ";
    let paragraph: String = words.chars().cycle().take(400).collect();
    let mut text = String::with_capacity(size);
    while text.len() + paragraph.len() + 2 <= size {
        text.push_str(&paragraph);
        text.push_str("\n\n");
    }
    text.into_bytes()
}

/// `PROSE-IO-119`: a document imported from Markdown at the bound keeps
/// less than `CEILING` times its file once the import ends. The resident
/// size is read before the import and after it, the file and every answer
/// dropped. What the allocator keeps of the freed memory counts
/// against the document, so the figure is an upper bound.
#[test]
fn a_document_holds_less_than_the_ceiling_times_its_file() {
    let converter = ambre_convert::Converter::load(
        ambre_core::ConverterFamily::Markup,
        &ambre_core::Payload::new(Vec::new()),
    )
    .expect("the host links the markup engine");
    let mut state = ProseState::new().expect("a fresh document builds");
    let file = novel(MAX_SIZE.saturating_len().to_usize());
    let size = file.len();
    let command = ProseCommand::Interop(InteropRequest::Import(ProseImport {
        format: ImportFormat::Markdown,
        data: ambre_core::Payload::new(file),
        now: ambre_core::Instant::new(1_800_000_000_000),
        zone: TimeZone::UTC,
    }));
    let before = resident().expect("Linux reports the resident size");
    // The read runs in place, as the converter thread runs it.
    let mut last = state.handle_conversion(command, Ok(&converter));
    for (ticket, conversion) in state.take_file_requests() {
        if let Some(next) = state.handle_file_converted(ticket, converter.convert(&conversion)) {
            last = next;
        }
    }
    while state.has_pending_work() {
        last = state.resume_conversion(Some(&converter));
    }
    assert!(
        last.display.iter().any(|event| matches!(
            event,
            ProseEvent::Interop(InteropAnswer::ImportCompleted { .. })
        )),
        "the import lands whole"
    );
    drop(last);
    let after = resident().expect("Linux reports the resident size");
    let held = after.saturating_sub(before);
    assert!(
        held < CEILING * size,
        "a document of {size} bytes of Markdown held {held} bytes, {} times its file",
        held / size
    );
    eprintln!("a document of {size} bytes of Markdown held {held} bytes");
    drop(state);
}
