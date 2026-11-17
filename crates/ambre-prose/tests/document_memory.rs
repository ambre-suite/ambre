// ambre-prose document memory test (tier `host`, `PROSE-IO-119`).
// ============================================================================
// The ceiling of the `Prose document, held after an import` row of
// `ambre-spec-budgets.md section 6`. It reads the bytes the heap holds once
// a document's import ends, as a multiple of its Markdown file.
// The test is alone in its binary, so no other test shares the allocator it
// counts. A counting allocator reads the heap itself: what the C allocator
// keeps of the freed memory, and the pages it never touched, do not count.
// ============================================================================
#![cfg(not(target_arch = "wasm32"))]

use std::alloc::System;

use ambre_bus::interop::{InteropAnswer, InteropRequest};
use ambre_prose::events::ProseEvent;
use ambre_prose::interop::markdown::MAX_SIZE;
use ambre_prose::logic::ProseState;
use ambre_prose::message::{ImportFormat, ProseCommand, ProseImport, TimeZone};
use cap::Cap;

/// The allocator of this binary: the system one, with a count of the bytes
/// it holds and no limit.
#[global_allocator]
static HEAP: Cap<System> = Cap::new(System, usize::MAX);

/// The ceiling, in multiples of the file. On 2026-11-17 this test reads
/// 13.9 times, as a heap profile of the same import does.
/// Before the step of the import kept no write into a block it created,
/// the profile read 15.8 times.
const CEILING: usize = 15;

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
/// less than `CEILING` times its file once the import ends. The heap is
/// counted before the import and after it, the file and every answer
/// dropped. The difference is what the document holds, its undo history
/// included.
#[test]
fn a_document_holds_less_than_the_ceiling_times_its_file() {
    let converter = ambre_convert::Converter::load(
        ambre_core::ConverterFamily::Markup,
        &ambre_core::Payload::new(Vec::new()),
    )
    .expect("the host links the markup engine");
    let mut state = ProseState::new().expect("a fresh document builds");
    // The count starts before the file exists. The import frees the file,
    // so a count taken after it would take the file off the document.
    let before = HEAP.allocated();
    let file = novel(MAX_SIZE.saturating_len().to_usize());
    let size = file.len();
    let command = ProseCommand::Interop(InteropRequest::Import(ProseImport {
        format: ImportFormat::Markdown,
        data: ambre_core::Payload::new(file),
        now: ambre_core::Instant::new(1_800_000_000_000),
        zone: TimeZone::UTC,
    }));
    // The read runs in place, as the converter thread runs it.
    let mut last = state.handle_conversion(command, Ok(&converter));
    for (ticket, conversion) in state.take_file_requests() {
        if let Some(next) = state.handle_file_converted(ticket, converter.convert(&conversion)) {
            last = next;
        }
    }
    while state.has_pending_work() {
        last = state.resume_conversion();
    }
    assert!(
        last.display.iter().any(|event| matches!(
            event,
            ProseEvent::Interop(InteropAnswer::ImportCompleted { .. })
        )),
        "the import lands whole"
    );
    drop(last);
    let after = HEAP.allocated();
    let held = after.saturating_sub(before);
    assert!(
        held < CEILING * size,
        "a document of {size} bytes of Markdown held {held} bytes, {} times its file",
        held / size
    );
    eprintln!("a document of {size} bytes of Markdown held {held} bytes");
    drop(state);
}
