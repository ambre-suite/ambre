//! The spelling session against the two dictionaries the host links, served
//! as the converter thread serves them.

use super::*;
use crate::events::{BlockInit, ProseCommand};
use crate::view::tests::append;

/// A document whose locale is `locale`, holding one paragraph per text.
fn document(locale: Option<&str>, texts: &[&str]) -> (ProseState, Vec<BlockId>) {
    let mut state = ProseState::with_peer(ambre_core::PeerId::new(1)).expect("a document is built");
    if let Some(locale) = locale {
        state.handle_command(ProseCommand::SetDocumentLocale {
            locale: ambre_client_core::DocumentLocale::parse(locale).expect("a tag"),
        });
    }
    let ids = texts
        .iter()
        .map(|text| append(&mut state, BlockInit::Paragraph, text))
        .collect();
    state.layout_event();
    (state, ids)
}

/// Run each job `jobs` holds on the engine the host links, as the converter
/// thread does, and hand back every outcome.
fn serve(state: &mut ProseState, jobs: Vec<(Seq<Request>, Conversion)>) -> Vec<ProseOutcome> {
    jobs.into_iter()
        .filter_map(|(ticket, conversion)| {
            let converter = ambre_convert::Converter::load(
                conversion.family(),
                &ambre_core::Payload::new(Vec::new()),
            )
            .expect("the host links the spelling engines");
            let answer = converter
                .convert(&conversion)
                .map_err(|_| JobFailure::Unavailable);
            state.handle_spelled(ticket, &answer)
        })
        .collect()
}

/// One pass, its jobs served: the events the view is told.
fn pass(state: &mut ProseState) -> Vec<ProseEvent> {
    let (jobs, outcome) = state.spell_pass();
    let mut display = outcome.display;
    for outcome in serve(state, jobs) {
        display.extend(outcome.display);
    }
    display
}

/// The ranges each `Misspelled` event names for `block`, the last one
/// standing.
fn underlined(events: &[ProseEvent], block: BlockId) -> Option<Vec<(usize, usize)>> {
    events.iter().rev().find_map(|event| match event {
        ProseEvent::Misspelled { block_id, ranges } if *block_id == block => Some(
            ranges
                .iter()
                .map(|range| (range.start.to_usize(), range.end.to_usize()))
                .collect(),
        ),
        _ => None,
    })
}

fn span(start: usize, end: usize) -> Span<DocChar> {
    Span::<DocChar>::from_usize_range(start..end)
}

/// `PROSE-FEAT-333`: a misspelled word of a paragraph is reported with its
/// range in code points, in English and in French.
#[test]
fn a_misspelled_word_is_reported_with_its_range() {
    let (mut state, ids) = document(Some("en-US"), &["I recieve teh letter."]);
    let events = pass(&mut state);
    assert_eq!(underlined(&events, ids[0]), Some(vec![(2, 9), (10, 13)]));

    let (mut state, ids) = document(Some("fr"), &["L\u{2019}\u{e9}t\u{e9} est chuad."]);
    let events = pass(&mut state);
    assert_eq!(underlined(&events, ids[0]), Some(vec![(10, 15)]));
}

/// `PROSE-FEAT-041`, `PROSE-FEAT-103`: the check leaves as a job of the
/// converter thread. The pass returns with it in flight, nothing is
/// underlined before its answer, and one check at a time is in flight.
#[test]
fn the_check_leaves_for_the_converter_thread_and_nothing_waits() {
    let (mut state, ids) = document(Some("en"), &["A mispeled word."]);
    let (jobs, outcome) = state.spell_pass();
    assert!(outcome.display.is_empty(), "nothing is underlined yet");
    let [(_, Conversion::CheckSpelling(check))] = &jobs[..] else {
        panic!("one check leaves");
    };
    assert_eq!(check.dictionary, Dictionary::English);
    state.handle_command(ProseCommand::InsertText {
        block_id: ids[0],
        pos: Pos::<DocChar>::ZERO,
        text: "Yes. ".to_owned(),
    });
    state.layout_event();
    let (more, _) = state.spell_pass();
    assert!(
        more.is_empty(),
        "the edited block waits for the check in flight"
    );
    // The answer names ranges of the text before the edit. The block is
    // checked again, and the next check underlines the word where it is now.
    let stale = serve(&mut state, jobs);
    assert!(
        stale.iter().all(|outcome| outcome.display.is_empty()),
        "a stale answer underlines nothing"
    );
    let events = pass(&mut state);
    assert_eq!(underlined(&events, ids[0]), Some(vec![(7, 15)]));
}

/// `PROSE-FEAT-106`, `PROSE-FEAT-335`: the errors live in the reader's
/// session. A check writes no delta and leaves the document as it was.
#[test]
fn a_check_writes_nothing_to_the_document() {
    let (mut state, _) = document(Some("en"), &["A mispeled word."]);
    let before = state.export_snapshot();
    let (jobs, outcome) = state.spell_pass();
    assert!(outcome.update.is_none() && outcome.ephemeral.is_none());
    for outcome in serve(&mut state, jobs) {
        assert!(outcome.update.is_none() && outcome.ephemeral.is_none());
    }
    assert_eq!(state.export_snapshot(), before);
}

/// `PROSE-FEAT-105`: the suggestions of a misspelled word leave as a job of
/// their own, and come back at most five, the right one among them.
#[test]
fn a_suggestion_comes_back_at_most_five() {
    let (mut state, ids) = document(Some("en"), &["I recieve it."]);
    pass(&mut state);
    let outcome = state.handle_command(ProseCommand::SuggestSpelling {
        block_id: ids[0],
        range: span(2, 9),
    });
    assert!(outcome.update.is_none());
    let events = pass(&mut state);
    let Some(ProseEvent::SpellingSuggestions { suggestions, .. }) = events
        .iter()
        .find(|event| matches!(event, ProseEvent::SpellingSuggestions { .. }))
    else {
        panic!("the suggestions come back: {events:?}");
    };
    assert!(suggestions.len() <= 5);
    assert!(suggestions.iter().any(|word| word.as_str() == "receive"));
}

/// `PROSE-FEAT-109`, `PROSE-FEAT-110`: a correction replaces the word as an
/// ordinary edit, and a range that names no error is refused untouched.
#[test]
fn a_correction_is_an_edit_of_the_word_alone() {
    let (mut state, ids) = document(Some("en"), &["I recieve it."]);
    pass(&mut state);
    let receive = Validated::<SpellWord>::parse("receive").expect("a word");
    let refused = state.handle_command(ProseCommand::CorrectSpelling {
        block_id: ids[0],
        range: span(0, 1),
        replacement: receive.clone(),
    });
    assert!(refused.update.is_none(), "no error sits there");
    assert_eq!(state.text_of(&ids[0]).as_deref(), Some("I recieve it."));
    let outcome = state.handle_command(ProseCommand::CorrectSpelling {
        block_id: ids[0],
        range: span(2, 9),
        replacement: receive,
    });
    assert!(outcome.update.is_some(), "peers receive the edit");
    assert_eq!(state.text_of(&ids[0]).as_deref(), Some("I receive it."));
    state.layout_event();
    let events = pass(&mut state);
    assert_eq!(underlined(&events, ids[0]), Some(Vec::new()));
}

/// `PROSE-FEAT-108`, `PROSE-FEAT-334`: an ignored word stops being
/// underlined everywhere in the document, and nothing is written.
#[test]
fn an_ignored_word_is_underlined_nowhere_and_nothing_is_written() {
    let (mut state, ids) = document(Some("en"), &["Ambre is here.", "We like Ambre."]);
    pass(&mut state);
    let before = state.export_snapshot();
    let outcome = state.handle_command(ProseCommand::IgnoreSpelling {
        block_id: ids[0],
        range: span(0, 5),
    });
    assert!(outcome.update.is_none() && outcome.ephemeral.is_none());
    assert_eq!(underlined(&outcome.display, ids[0]), Some(Vec::new()));
    assert_eq!(underlined(&outcome.display, ids[1]), Some(Vec::new()));
    assert_eq!(state.export_snapshot(), before);
    state.handle_command(ProseCommand::InsertText {
        block_id: ids[1],
        pos: Pos::<DocChar>::ZERO,
        text: "Ambre. ".to_owned(),
    });
    state.layout_event();
    let events = pass(&mut state);
    assert_eq!(
        underlined(&events, ids[1]),
        None,
        "still nothing underlined"
    );
}

/// `PROSE-FEAT-111`: the language is the document's locale, else the
/// reader's own, else none. A locale set checks the document again under
/// the new dictionary.
#[test]
fn the_language_follows_the_document_then_the_reader() {
    let (mut state, ids) = document(None, &["Une maisn."]);
    assert!(pass(&mut state).is_empty(), "no language, no check");
    state.set_spelling_reader(Validated::<LangTag>::parse("fr-FR").ok());
    assert_eq!(underlined(&pass(&mut state), ids[0]), Some(vec![(4, 9)]));
    state.handle_command(ProseCommand::SetDocumentLocale {
        locale: ambre_client_core::DocumentLocale::parse("en").expect("a tag"),
    });
    let events = pass(&mut state);
    assert_eq!(
        underlined(&events, ids[0]),
        Some(vec![(0, 3), (4, 9)]),
        "checked again in English"
    );
}
