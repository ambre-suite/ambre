//! The Markdown reader of the markup module: text to its events.
//!
//! It reads GFM, which CommonMark is a subset of, so one reader serves both
//! flavours. Every string of text is some Markdown document, so it never
//! fails. Prose maps the events onto its intermediate form.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::markup::{MdCodeBlock, MdEvent, MdHeading, MdList, MdTag, MdTagEnd};

/// The events of `text`, in document order.
pub(super) fn read(text: &str) -> Vec<MdEvent> {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES;
    Parser::new_ext(text, options).map(event).collect()
}

fn event(event: Event<'_>) -> MdEvent {
    match event {
        Event::Start(tag) => MdEvent::Start(start(tag)),
        Event::End(tag) => MdEvent::End(end(tag)),
        Event::Text(text) => MdEvent::Text(text.into_string()),
        Event::Code(text) => MdEvent::Code(text.into_string()),
        Event::SoftBreak => MdEvent::SoftBreak,
        Event::HardBreak => MdEvent::HardBreak,
        Event::Rule => MdEvent::Rule,
        Event::TaskListMarker(done) => MdEvent::TaskListMarker(done),
        Event::FootnoteReference(_) => MdEvent::FootnoteReference,
        Event::Html(_) | Event::InlineHtml(_) => MdEvent::Html,
        Event::InlineMath(text) | Event::DisplayMath(text) => MdEvent::Math(text.into_string()),
    }
}

fn start(tag: Tag<'_>) -> MdTag {
    match tag {
        Tag::Paragraph => MdTag::Paragraph,
        Tag::Heading { level, .. } => MdTag::Heading(heading(level)),
        Tag::CodeBlock(CodeBlockKind::Fenced(info)) => {
            MdTag::CodeBlock(MdCodeBlock::Fenced(info.into_string()))
        }
        Tag::CodeBlock(CodeBlockKind::Indented) => MdTag::CodeBlock(MdCodeBlock::Indented),
        Tag::List(Some(_)) => MdTag::List(MdList::Ordered),
        Tag::List(None) => MdTag::List(MdList::Bullet),
        Tag::Item => MdTag::Item,
        Tag::Table(_) => MdTag::Table,
        Tag::TableHead => MdTag::TableHead,
        Tag::TableRow => MdTag::TableRow,
        Tag::TableCell => MdTag::TableCell,
        Tag::BlockQuote(_) => MdTag::BlockQuote,
        Tag::FootnoteDefinition(_) => MdTag::FootnoteDefinition,
        Tag::HtmlBlock => MdTag::HtmlBlock,
        Tag::Emphasis => MdTag::Emphasis,
        Tag::Strong => MdTag::Strong,
        Tag::Strikethrough => MdTag::Strikethrough,
        Tag::Link { dest_url, .. } => MdTag::Link(dest_url.into_string()),
        Tag::Image { .. } => MdTag::Image,
        _ => MdTag::Other,
    }
}

fn end(tag: TagEnd) -> MdTagEnd {
    match tag {
        TagEnd::Paragraph => MdTagEnd::Paragraph,
        TagEnd::Heading(_) => MdTagEnd::Heading,
        TagEnd::CodeBlock => MdTagEnd::CodeBlock,
        TagEnd::List(_) => MdTagEnd::List,
        TagEnd::Item => MdTagEnd::Item,
        TagEnd::Table => MdTagEnd::Table,
        TagEnd::TableHead => MdTagEnd::TableHead,
        TagEnd::TableRow => MdTagEnd::TableRow,
        TagEnd::TableCell => MdTagEnd::TableCell,
        TagEnd::BlockQuote(_) => MdTagEnd::BlockQuote,
        TagEnd::FootnoteDefinition => MdTagEnd::FootnoteDefinition,
        TagEnd::HtmlBlock => MdTagEnd::HtmlBlock,
        TagEnd::Emphasis => MdTagEnd::Emphasis,
        TagEnd::Strong => MdTagEnd::Strong,
        TagEnd::Strikethrough => MdTagEnd::Strikethrough,
        TagEnd::Link => MdTagEnd::Link,
        TagEnd::Image => MdTagEnd::Image,
        _ => MdTagEnd::Other,
    }
}

const fn heading(level: HeadingLevel) -> MdHeading {
    match level {
        HeadingLevel::H1 => MdHeading::H1,
        HeadingLevel::H2 => MdHeading::H2,
        HeadingLevel::H3 => MdHeading::H3,
        HeadingLevel::H4 => MdHeading::H4,
        HeadingLevel::H5 => MdHeading::H5,
        HeadingLevel::H6 => MdHeading::H6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fenced_block_keeps_its_info_and_a_task_its_box() {
        let events = read("```rust x\nfn f() {}\n```\n\n- [x] done\n");
        assert_eq!(
            events.first(),
            Some(&MdEvent::Start(MdTag::CodeBlock(MdCodeBlock::Fenced(
                "rust x".to_owned()
            ))))
        );
        assert!(events.contains(&MdEvent::TaskListMarker(true)));
        assert!(events.contains(&MdEvent::Start(MdTag::List(MdList::Bullet))));
    }
}
