//! The cascade: what one run of text finally looks like
//! (`features/ambre-spec-prose-inline-styles.md section 3`).
//!
//! A property is taken from the first level that carries it. The inline mark on
//! the character comes first, then the built-in settings of the block type,
//! then the document defaults (`PROSE-FEAT-311`). Every field of [`ResolvedStyle`] is a
//! decided value, so a caller draws from it and never looks further up
//! (`PROSE-FEAT-177`).
//!
//! Nothing here writes. The resolution runs once per run per layout, which is
//! the row the budget pays for (`ambre-spec-budgets.md section 1`).

use std::collections::BTreeMap;

use crate::doc::DocDefaults;
use crate::events::BlockType;
use crate::mark;

/// The marks one run carries, key to stored value.
pub type MarkSet = BTreeMap<String, String>;

/// The block a run sits in, which is the level between the mark and the
/// document defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockContext {
    pub block_type: BlockType,
    /// The heading's level, when the block is one.
    pub heading_level: Option<u8>,
}

/// The result of the cascade for one run of text.
///
/// Every field is decided: a `bool` is on or off, and the two colours are a
/// decided presence or a decided absence (`PROSE-FEAT-177`).
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub superscript: bool,
    pub subscript: bool,
    pub code_inline: bool,
    pub color: Option<u32>,
    pub background: Option<u32>,
    /// In points.
    pub font_size: f32,
    pub font_family: Option<String>,
    pub link: Option<String>,
}

/// How much larger than the body a heading of each level is drawn.
///
/// The built-in level of the cascade, and the one a named style would sit above
/// once one can be written (section 6 of the feature).
const HEADING_SCALE: [f32; 6] = [2.0, 1.5, 1.25, 1.1, 1.0, 0.9];

/// The family a code block and an inline code run are drawn in.
const MONOSPACE: &str = "monospace";

/// The bold a heading is drawn in, and the monospace of a code block, live here
/// rather than in the caller. The cascade is the one reader of the block type.
fn block_defaults(context: BlockContext, defaults: &DocDefaults) -> ResolvedStyle {
    let mut resolved = ResolvedStyle {
        bold: false,
        italic: false,
        underline: false,
        strikethrough: false,
        superscript: false,
        subscript: false,
        code_inline: false,
        color: None,
        background: None,
        font_size: defaults.font_size_pt,
        font_family: Some(defaults.font_family.clone()),
        link: None,
    };
    match context.block_type {
        BlockType::Heading => {
            resolved.bold = true;
            let level = usize::from(context.heading_level.unwrap_or(1).clamp(1, 6));
            if let Some(scale) = HEADING_SCALE.get(level - 1) {
                resolved.font_size = defaults.font_size_pt * scale;
            }
        }
        BlockType::CodeBlock => {
            resolved.code_inline = true;
            resolved.font_family = Some(MONOSPACE.to_owned());
        }
        _ => {}
    }
    resolved
}

/// Resolve one run against its block and the document defaults
/// (`PROSE-FEAT-311`).
///
/// A mark the run carries wins over the block, which wins over the defaults.
/// A colour that fails its grammar is dropped rather than resolved to a wrong
/// one. The write path refused it already (`PROSE-FEAT-022`), so a malformed
/// value here came from a document this build did not write.
#[must_use]
pub fn resolve(marks: &MarkSet, context: BlockContext, defaults: &DocDefaults) -> ResolvedStyle {
    let mut resolved = block_defaults(context, defaults);
    let has = |key: &str| marks.contains_key(key);
    resolved.bold |= has("b");
    resolved.italic |= has("i");
    resolved.underline = has("u");
    resolved.strikethrough = has("s");
    resolved.code_inline |= has("c");
    // Both marks are storable on one range, and neither write clears the other
    // (`PROSE-FEAT-173`). The raised one wins, by a priority fixed here rather
    // than by which mark arrived last (`PROSE-FEAT-174`).
    resolved.superscript = has("sup");
    resolved.subscript = has("sub") && !resolved.superscript;
    if let Some(raw) = marks.get("fg") {
        resolved.color = mark::parse_color(raw).ok();
    }
    if let Some(raw) = marks.get("bg") {
        resolved.background = mark::parse_color(raw).ok();
    }
    if let Some(points) = marks
        .get("fs")
        .and_then(|raw| mark::parse_font_size(raw).ok())
    {
        resolved.font_size = points;
    }
    if let Some(family) = marks.get("ff").filter(|family| !family.is_empty()) {
        resolved.font_family = Some(family.clone());
    }
    resolved.link = marks.get("a").filter(|url| !url.is_empty()).cloned();
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marks(pairs: &[(&str, &str)]) -> MarkSet {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn body() -> BlockContext {
        BlockContext {
            block_type: BlockType::Paragraph,
            heading_level: None,
        }
    }

    fn heading(level: u8) -> BlockContext {
        BlockContext {
            block_type: BlockType::Heading,
            heading_level: Some(level),
        }
    }

    /// A property is taken from the first level that carries it
    /// (`PROSE-FEAT-311`).
    #[test]
    fn a_mark_wins_over_the_block_which_wins_over_the_defaults() {
        let defaults = DocDefaults::default();
        // The defaults alone.
        let plain = resolve(&MarkSet::new(), body(), &defaults);
        assert_eq!(plain.font_size, defaults.font_size_pt);
        assert!(!plain.bold);

        // The block type, above the defaults.
        let titled = resolve(&MarkSet::new(), heading(1), &defaults);
        assert!(titled.bold, "a heading is drawn bold by its type");
        assert!(titled.font_size > defaults.font_size_pt);

        // The mark, above both.
        let marked = resolve(&marks(&[("fs", "30")]), heading(1), &defaults);
        assert_eq!(marked.font_size, 30.0);
    }

    /// A property no level carries resolves to a decided absence, and the
    /// reader stops there (`PROSE-FEAT-177`).
    #[test]
    fn a_property_no_level_carries_resolves_to_a_decided_absence() {
        let resolved = resolve(&MarkSet::new(), body(), &DocDefaults::default());
        assert_eq!(resolved.color, None);
        assert_eq!(resolved.background, None);
        assert_eq!(resolved.link, None);
        // The size and the family are always decided: the defaults carry both.
        assert!(resolved.font_size > 0.0);
        assert!(resolved.font_family.is_some());
    }

    /// Raised beats lowered by a priority fixed in the reader, whichever order
    /// the two marks are held in (`PROSE-FEAT-174`).
    #[test]
    fn raised_beats_lowered_by_a_fixed_priority() {
        let defaults = DocDefaults::default();
        let both = resolve(&marks(&[("sup", ""), ("sub", "")]), body(), &defaults);
        assert!(both.superscript);
        assert!(!both.subscript);
        // Lowered alone still resolves as lowered.
        let lowered = resolve(&marks(&[("sub", "")]), body(), &defaults);
        assert!(lowered.subscript);
        assert!(!lowered.superscript);
    }

    /// Two spellings of one opaque colour resolve to one value
    /// (`PROSE-FEAT-175`).
    #[test]
    fn two_spellings_of_one_colour_resolve_to_one_value() {
        let defaults = DocDefaults::default();
        let short = resolve(&marks(&[("fg", "#3366cc")]), body(), &defaults);
        let long = resolve(&marks(&[("fg", "#3366ccff")]), body(), &defaults);
        assert_eq!(short.color, long.color);
        assert_eq!(short.color, Some(0x3366_ccff));
    }
}
