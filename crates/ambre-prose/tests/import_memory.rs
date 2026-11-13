// ambre-prose import memory test (tier `host`, `PROSE-IO-111`).
// ============================================================================
// The ceiling of the `Prose import, transient` row of `ambre-spec-budgets.md
// section 6`, read as the peak resident size of this process. The test is
// alone in its binary, so no other test shares the process it measures.
// Linux alone reports the peak (`VmHWM`), and CI runs on Linux.
// ============================================================================
#![cfg(target_os = "linux")]

use ambre_bus::interop::{InteropAnswer, InteropRequest};
use ambre_prose::events::ProseEvent;
use ambre_prose::interop::markdown::MAX_SIZE;
use ambre_prose::logic::ProseState;
use ambre_prose::message::{ImportFormat, ProseCommand, ProseImport, TimeZone};

/// The ceiling, in multiples of the file. The row measures about 17 times.
/// The rehearsal on a scratch document held about 23.
const CEILING: usize = 20;

/// One kibibyte, the unit `/proc/self/status` reports in.
const KIB: usize = 1024;

/// The peak and the current resident size of this process, in bytes, or
/// `None` when the status names neither.
fn resident() -> Option<(usize, usize)> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let field = |name: &str| {
        status
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .and_then(|rest| {
                rest.trim()
                    .trim_end_matches("kB")
                    .trim()
                    .parse::<usize>()
                    .ok()
            })
            .map(|kib| kib * KIB)
    };
    Some((field("VmHWM:")?, field("VmRSS:")?))
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

/// `PROSE-IO-111`: an import of Markdown at the bound holds less than
/// `CEILING` times its file. The peak is read above the resident size the
/// process had before the import. The import lands one slice per pass, as the
/// Logic thread lands it. Each slice's delta is dropped, as the Logic thread
/// hands it to the Sync worker.
#[test]
fn an_import_holds_less_than_the_ceiling_times_its_file() {
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
    // Writing 5 to `clear_refs` sets the peak back to the current size, so
    // the peak read after the import is the import's own.
    std::fs::write("/proc/self/clear_refs", "5").expect("Linux resets the peak");
    let (_, before) = resident().expect("Linux reports the resident size");
    let mut last = state.handle_conversion(command, Ok(&converter));
    while state.has_pending_work() {
        last = state.resume_conversion(Some(&converter));
    }
    let (peak, _) = resident().expect("Linux reports the peak");
    assert!(
        last.display.iter().any(|event| matches!(
            event,
            ProseEvent::Interop(InteropAnswer::ImportCompleted { .. })
        )),
        "the import lands whole"
    );
    let held = peak.saturating_sub(before);
    assert!(
        held < CEILING * size,
        "an import of {size} bytes held {held} bytes, {} times its file",
        held / size
    );
    eprintln!("an import of {size} bytes held {held} bytes");
}
