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
use crate::mark::MarkRoot;

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

/// A scheme a link may hand to the host to open. The set is closed: any
/// other scheme, `javascript:` or `data:` among them, never reaches the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum OpenableScheme {
    Http,
    Https,
    Mailto,
}

impl OpenableScheme {
    /// Every scheme of the set.
    const ALL: [Self; 3] = [Self::Http, Self::Https, Self::Mailto];

    /// The scheme as a target spells it, before its colon.
    fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Https => "https",
            Self::Mailto => "mailto",
        }
    }

    /// The scheme `word` names, case folded. `None` for a scheme outside
    /// the set.
    fn parse(word: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|scheme| word.eq_ignore_ascii_case(scheme.as_str()))
    }
}

/// The longest link target a paste keeps and a copy or an export writes, in
/// bytes (`PROSE-UI-234`, `PROSE-IO-069`).
pub(crate) const HREF_MAX: ambre_core::ByteSize = ambre_core::ByteSize(2048);

/// Whether a link's target fits [`HREF_MAX`]. A writer writes a longer one as
/// its text alone.
pub(crate) fn within_href_max(url: &ambre_core::LinkUrl) -> bool {
    url.size() <= HREF_MAX
}

/// Whether a link's target may be handed to the host (`PROSE-UI-186`).
///
/// Nothing rules on a target when a peer writes it, so a hostile one may
/// carry `javascript:` or `data:`. Only an [`OpenableScheme`] passes.
/// The scheme is read the way a browser reads it: leading spaces and control
/// characters skipped, tabs and newlines inside dropped, case folded.
#[must_use]
pub fn openable(url: &ambre_core::LinkUrl) -> bool {
    let cleaned: String = url
        .as_str()
        .trim_start_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    cleaned
        .split_once(':')
        .and_then(|(scheme, _)| OpenableScheme::parse(scheme))
        .is_some()
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
                key: key.into(),
                value: Some(crate::mark::test_value(value)),
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
