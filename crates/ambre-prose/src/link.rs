//! The links of the document as the view lists them
//! (`features/ambre-spec-prose-selection.md section 2`).
//!
//! A link is the `a` mark over a run of text, its value the target. The view
//! reads the list to show the link cursor and to follow a target; nothing in
//! it writes to the document.

use ambre_bus::app::prose::{DocCount, DocOffset};
use loro::{LoroDoc, LoroValue, TextDelta};

use crate::comment::text_blocks;
use crate::doc;
use crate::events::{LinkSpan, block_id};

/// The mark key a link is written under.
const LINK_KEY: &str = "a";

/// Every run of a link mark, in document order. Two runs side by side with
/// one target read as one link.
#[must_use]
pub fn links(document: &LoroDoc) -> Vec<LinkSpan> {
    let mut out: Vec<LinkSpan> = Vec::new();
    for id in text_blocks(document) {
        let block = block_id(id);
        let mut offset = DocOffset::ZERO;
        for delta in doc::get_block_text(document, &id).to_delta() {
            let TextDelta::Insert { insert, attributes } = delta else {
                continue;
            };
            let length = DocCount::of(&insert);
            let end = offset + length;
            let url = attributes
                .as_ref()
                .and_then(|attributes| attributes.get(LINK_KEY))
                .and_then(|value| match value {
                    LoroValue::String(url) if !url.is_empty() => Some(url.to_string()),
                    _ => None,
                });
            if let Some(url) = url {
                match out.last_mut() {
                    Some(last)
                        if last.block_id == block && last.end == offset && last.url == url =>
                    {
                        last.end = end;
                    }
                    _ => out.push(LinkSpan {
                        block_id: block,
                        start: offset,
                        end,
                        url,
                    }),
                }
            }
            offset = end;
        }
    }
    out
}

/// The schemes a link may hand to the host to open.
const OPENABLE: [&str; 3] = ["http", "https", "mailto"];

/// Whether a link's target may be handed to the host (`PROSE-UI-186`).
///
/// Nothing rules on a target when a peer writes it, so a hostile one may
/// carry `javascript:` or `data:`. Only a scheme from [`OPENABLE`] passes.
/// The scheme is read the way a browser reads it: leading spaces and control
/// characters skipped, tabs and newlines inside dropped, case folded.
#[must_use]
pub fn openable(url: &str) -> bool {
    let cleaned: String = url
        .trim_start_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    let Some((scheme, _)) = cleaned.split_once(':') else {
        return false;
    };
    OPENABLE
        .iter()
        .any(|allowed| scheme.eq_ignore_ascii_case(allowed))
}

#[cfg(test)]
mod tests {
    use crate::events::{BlockInit, LinkSpan, ProseCommand, ProseEvent};
    use crate::logic::ProseState;
    use crate::view::tests::append;
    use ambre_bus::app::prose::{DocOffset, DocSpan};

    fn mark(
        state: &mut ProseState,
        block: crate::events::BlockId,
        range: (u32, u32),
        key: &str,
        value: &str,
    ) -> Vec<ProseEvent> {
        state
            .handle_command(ProseCommand::ApplyMark {
                block_id: block,
                range: DocSpan::new(DocOffset::new(range.0), DocOffset::new(range.1)),
                key: key.to_owned(),
                value: Some(value.to_owned()),
            })
            .display
    }

    #[test]
    fn a_link_is_listed_whole_across_the_marks_inside_it() {
        let mut state = ProseState::with_peer(ambre_core::PeerId(1)).expect("a document is built");
        let block = append(&mut state, BlockInit::Paragraph, "see the site now");
        let events = mark(&mut state, block, (4, 12), "a", "https://example.org/");
        let expected = vec![LinkSpan {
            block_id: block,
            start: DocOffset::new(4),
            end: DocOffset::new(12),
            url: "https://example.org/".to_owned(),
        }];
        assert!(events.contains(&ProseEvent::LinksUpdated {
            links: expected.clone()
        }));
        // Bold over part of it splits the text's runs, and not the link.
        let events = mark(&mut state, block, (6, 9), "b", "true");
        assert!(
            events
                .iter()
                .all(|e| !matches!(e, ProseEvent::LinksUpdated { .. }))
        );
        assert_eq!(super::links(state.document()), expected);
    }

    #[test]
    fn only_a_web_or_mail_target_may_be_opened() {
        for url in [
            "https://example.org/",
            "http://example.org/",
            "HTTPS://example.org/",
            "mailto:someone@example.org",
        ] {
            assert!(super::openable(url), "{url}");
        }
        for url in [
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            " \u{1}javascript:alert(1)",
            "java\tscript:alert(1)",
            "java\nscript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "vbscript:msgbox(1)",
            "file:///etc/passwd",
            "blob:https://example.org/id",
            "about:blank",
            "example.org",
            "",
        ] {
            assert!(!super::openable(url), "{url}");
        }
    }
}
