//! The HTML reader of the markup module: a document to its tree.
//!
//! Clipboard HTML is routinely malformed. Only a WHATWG tree builder closes
//! the implied tags as a browser does, so it reads the fragment as the page
//! did. The module answers the tree. Prose walks it and keeps what its
//! allowlist names (`features/ambre-spec-prose-clipboard.md`).

use std::cell::{Cell, RefCell};

use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{
    BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};
use html5ever::{LocalName, ParseOpts, local_name, parse_document};

use super::html_sink::Arena;
use crate::markup::HtmlTree;

/// The tree of `html`. A document nested deeper than [`DEPTH_MAX`] reads as
/// the document alone.
pub(super) fn read(html: &str) -> HtmlTree {
    if too_deep(html) {
        return HtmlTree::empty();
    }
    parse_document(Arena::new(), ParseOpts::default()).one(html)
}

/// The tree of `html`, or `None` for a document nested deeper than
/// [`DEPTH_MAX`]: an import refuses it, where a paste reads it as nothing.
pub(super) fn read_bounded(html: &str) -> Option<HtmlTree> {
    (!too_deep(html)).then(|| parse_document(Arena::new(), ParseOpts::default()).one(html))
}

/// The deepest nesting of HTML elements the markup module reads.
///
/// The tree builder walks the open elements to check an element's scope. A
/// tree `n` deep thus costs `n` squared: 350 KB of nested lists takes seconds.
/// A list nested nine levels in Google Docs opens about forty elements, so a
/// cap four times that keeps every real fragment. Chrome caps its own tree at
/// 512, which the budget of a paste cannot afford.
const DEPTH_MAX: usize = 128;

/// The elements the tree builder closes on its own, and the void ones: none
/// of them deepens the tree.
const UNCOUNTED: [LocalName; 31] = [
    local_name!("p"),
    local_name!("li"),
    local_name!("dt"),
    local_name!("dd"),
    local_name!("tr"),
    local_name!("td"),
    local_name!("th"),
    local_name!("thead"),
    local_name!("tbody"),
    local_name!("tfoot"),
    local_name!("option"),
    local_name!("optgroup"),
    local_name!("rb"),
    local_name!("rt"),
    local_name!("rp"),
    local_name!("colgroup"),
    local_name!("caption"),
    local_name!("area"),
    local_name!("base"),
    local_name!("br"),
    local_name!("col"),
    local_name!("embed"),
    local_name!("hr"),
    local_name!("img"),
    local_name!("input"),
    local_name!("link"),
    local_name!("meta"),
    local_name!("param"),
    local_name!("source"),
    local_name!("track"),
    local_name!("wbr"),
];

/// Counts the nesting of `html` in one pass of the tokenizer, which is
/// linear, before the tree builder sees it.
#[derive(Default)]
struct Depth {
    /// The counted elements open, never more than [`DEPTH_MAX`] of them.
    open: RefCell<Vec<LocalName>>,
    /// Whether an element opened past [`DEPTH_MAX`].
    reach: Cell<Reach>,
}

/// Whether the elements of a paste stay within [`DEPTH_MAX`] of nesting.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Reach {
    #[default]
    Within,
    Exceeded,
}

impl TokenSink for Depth {
    type Handle = ();

    fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
        let Token::TagToken(tag) = token else {
            return TokenSinkResult::Continue;
        };
        let mut open = self.open.borrow_mut();
        match tag.kind {
            TagKind::StartTag => {
                let raw = match tag.name {
                    local_name!("script") => Some(RawKind::ScriptData),
                    local_name!("style")
                    | local_name!("xmp")
                    | local_name!("iframe")
                    | local_name!("noembed")
                    | local_name!("noframes") => Some(RawKind::Rawtext),
                    local_name!("title") | local_name!("textarea") => Some(RawKind::Rcdata),
                    _ => None,
                };
                // `/>` closes an element inside SVG and MathML alone: an HTML
                // `<div/>` opens a `div`.
                let foreign = open
                    .iter()
                    .any(|n| *n == local_name!("svg") || *n == local_name!("math"));
                let closed = tag.self_closing && foreign;
                if raw.is_none() && !closed && !UNCOUNTED.contains(&tag.name) {
                    if open.len() >= DEPTH_MAX {
                        self.reach.set(Reach::Exceeded);
                    } else {
                        open.push(tag.name.clone());
                    }
                }
                if let Some(kind) = raw {
                    return TokenSinkResult::RawData(kind);
                }
            }
            TagKind::EndTag => {
                if let Some(at) = open.iter().rposition(|n| *n == tag.name) {
                    open.truncate(at);
                }
            }
        }
        TokenSinkResult::Continue
    }
}

fn too_deep(html: &str) -> bool {
    let tokenizer = Tokenizer::new(Depth::default(), TokenizerOpts::default());
    let input = BufferQueue::default();
    input.push_back(StrTendril::from_slice(html));
    let _ = tokenizer.feed(&input);
    tokenizer.end();
    tokenizer.sink.reach.get() == Reach::Exceeded
}
