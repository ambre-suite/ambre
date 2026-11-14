//! The links of the document as the view lists them
//! (`features/ambre-spec-prose-selection.md section 2`).
//!
//! A link is the `a` mark over a run of text, its value the target. The view
//! reads the list to show the link cursor and to follow a target; nothing in
//! it writes to the document.

use crate::message::DocChar;
use ambre_core::index::Pos;
use loro::{LoroDoc, LoroValue, TextDelta};

use crate::comment::text_blocks;
use crate::doc;
use crate::events::{LinkSpan, block_id};
use crate::mark::MarkRoot;
use ambre_core::crdt::CrdtName as _;

/// Every run of a link mark, in document order. Two runs side by side with
/// one target read as one link.
#[must_use]
pub fn links(document: &LoroDoc) -> Vec<LinkSpan> {
    let mut out: Vec<LinkSpan> = Vec::new();
    for id in text_blocks(document) {
        let block = block_id(id);
        let mut offset = Pos::<DocChar>::ZERO;
        for delta in doc::get_block_text(document, &id).to_delta() {
            let TextDelta::Insert { insert, attributes } = delta else {
                continue;
            };
            let length = ambre_core::char_len(&insert).as_axis::<DocChar>();
            let end = offset + length;
            let url = attributes
                .as_ref()
                .and_then(|attributes| attributes.get(MarkRoot::Link.as_str()))
                .and_then(|value| match value {
                    LoroValue::String(url) if !url.is_empty() => {
                        Some(ambre_core::LinkUrl::new(url.to_string()))
                    }
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

// The rule a link must pass to reach the host lives with the form. The
// Markdown writer of the markup module thus reads the same one.
pub use ambre_prose_form::link::{HREF_MAX, openable, within_href_max};

#[cfg(test)]
mod tests {
    use crate::events::{BlockInit, LinkSpan, ProseCommand, ProseEvent};
    use crate::logic::ProseState;
    use crate::message::DocChar;
    use crate::view::tests::append;
    use ambre_core::index::{Pos, Span};

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
                range: Span::<DocChar>::new(
                    Pos::<DocChar>::new(range.0),
                    Pos::<DocChar>::new(range.1),
                ),
                key: key.into(),
                value: Some(crate::mark::test_value(value)),
            })
            .display
    }

    #[test]
    fn a_link_is_listed_whole_across_the_marks_inside_it() {
        let mut state =
            ProseState::with_peer(ambre_core::PeerId::new(1)).expect("a document is built");
        let block = append(&mut state, BlockInit::Paragraph, "see the site now");
        let events = mark(&mut state, block, (4, 12), "a", "https://example.org/");
        let expected = vec![LinkSpan {
            block_id: block,
            start: Pos::<DocChar>::new(4),
            end: Pos::<DocChar>::new(12),
            url: "https://example.org/".into(),
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
            assert!(super::openable(&url.into()), "{url}");
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
            assert!(!super::openable(&url.into()), "{url}");
        }
    }
}
