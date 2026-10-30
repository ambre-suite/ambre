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

use ambre_core::{FontSize, Rgba, Slant, Strikethrough, Underline, Weight};
use std::collections::BTreeMap;

use crate::doc::DocDefaults;
use crate::events::{BlockType, HeadingLevel};
use ambre_bus::app::prose::MarkKey;

use crate::mark::{self, InlineCode, MarkRoot, Script};

/// The marks one run carries, key to stored value.
pub type MarkSet = BTreeMap<MarkKey, StoredMarkValue>;

/// A mark value as the document holds it, never checked. A peer on another
/// build may have written any text, and [`resolve`] parses each one it reads
/// and drops what fails. A value may be a link or a family the user typed, so
/// `Debug` prints its length alone.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct StoredMarkValue(String);

impl StoredMarkValue {
    /// The value `text` the document holds.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    /// The value as stored, for the parse that reads it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for StoredMarkValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("StoredMarkValue")
            .field(&crate::redact::Redacted(&self.0))
            .finish()
    }
}

/// The block a run sits in, which is the level between the mark and the
/// document defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockContext {
    pub block_type: BlockType,
    /// The heading's level, when the block is one.
    pub heading_level: Option<HeadingLevel>,
}

/// The result of the cascade for one run of text.
///
/// Every field is decided: a flag is on or off, and the two colours are a
/// decided presence or a decided absence (`PROSE-FEAT-177`).
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedStyle {
    pub bold: Weight,
    pub italic: Slant,
    pub underline: Underline,
    pub strikethrough: Strikethrough,
    /// Raised, lowered or neither: never both (`PROSE-FEAT-174`).
    pub script: Script,
    pub code_inline: InlineCode,
    pub color: Option<Rgba>,
    pub background: Option<Rgba>,
    pub font_size: FontSize,
    pub font_family: Option<ambre_core::FontFamily>,
    pub link: Option<ambre_core::LinkUrl>,
}

/// How much larger than the body a heading of `level` is drawn.
///
/// The built-in level of the cascade, and the one a named style would sit above
/// once one can be written (section 6 of the feature).
const fn heading_scale(level: HeadingLevel) -> f32 {
    match level {
        HeadingLevel::H1 => 2.0,
        HeadingLevel::H2 => 1.5,
        HeadingLevel::H3 => 1.25,
        HeadingLevel::H4 => 1.1,
        HeadingLevel::H5 => 1.0,
        HeadingLevel::H6 => 0.9,
    }
}

/// The family a code block and an inline code run are drawn in.
const MONOSPACE: ambre_core::FontFamily = ambre_core::FontFamily::Mono;

/// The bold a heading is drawn in, and the monospace of a code block, live here
/// rather than in the caller. The cascade is the one reader of the block type.
fn block_defaults(context: BlockContext, defaults: &DocDefaults) -> ResolvedStyle {
    let mut resolved = ResolvedStyle {
        bold: Weight::Regular,
        italic: Slant::Upright,
        underline: Underline::Plain,
        strikethrough: Strikethrough::Plain,
        script: Script::Baseline,
        code_inline: InlineCode::Prose,
        color: None,
        background: None,
        font_size: defaults.font_size,
        font_family: Some(defaults.font_family.clone()),
        link: None,
    };
    match context.block_type {
        BlockType::Heading => {
            resolved.bold = Weight::Bold;
            let level = context.heading_level.unwrap_or(HeadingLevel::H1);
            resolved.font_size = FontSize::new(defaults.font_size.points() * heading_scale(level))
                .unwrap_or(defaults.font_size);
        }
        BlockType::CodeBlock => {
            resolved.code_inline = InlineCode::Code;
            resolved.font_family = Some(MONOSPACE);
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
    let has = |root: MarkRoot| marks.contains_key(root.as_str());
    let get = |root: MarkRoot| marks.get(root.as_str()).map(StoredMarkValue::as_str);
    if has(MarkRoot::Bold) {
        resolved.bold = Weight::Bold;
    }
    if has(MarkRoot::Italic) {
        resolved.italic = Slant::Italic;
    }
    resolved.underline = Underline::from_bool(has(MarkRoot::Underline));
    resolved.strikethrough = Strikethrough::from_bool(has(MarkRoot::Strike));
    if has(MarkRoot::Code) {
        resolved.code_inline = InlineCode::Code;
    }
    // Both marks are storable on one range, and neither write clears the other
    // (`PROSE-FEAT-173`). The raised one wins, by a priority fixed here rather
    // than by which mark arrived last (`PROSE-FEAT-174`).
    resolved.script = if has(MarkRoot::Superscript) {
        Script::Raised
    } else if has(MarkRoot::Subscript) {
        Script::Lowered
    } else {
        Script::Baseline
    };
    if let Some(raw) = get(MarkRoot::Ink) {
        resolved.color = mark::parse_color(raw).ok();
    }
    if let Some(raw) = get(MarkRoot::Highlight) {
        resolved.background = mark::parse_color(raw).ok();
    }
    if let Some(points) = get(MarkRoot::FontSize).and_then(|raw| mark::parse_font_size(raw).ok()) {
        resolved.font_size = points;
    }
    if let Some(family) = get(MarkRoot::FontFamily).filter(|family| !family.is_empty()) {
        resolved.font_family = Some(ambre_core::FontFamily::from_label(family));
    }
    resolved.link = get(MarkRoot::Link)
        .filter(|url| !url.is_empty())
        .map(|url| ambre_core::LinkUrl::new(url.to_owned()));
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_mark_value_prints_its_length_alone() {
        let value = StoredMarkValue::new("https://private.example/page");
        assert!(crate::redact::tests::hides(
            &value,
            "https://private.example/page"
        ));
    }

    fn marks(pairs: &[(&str, &str)]) -> MarkSet {
        pairs
            .iter()
            .map(|(key, value)| (MarkKey::from(*key), StoredMarkValue::new(*value)))
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
            heading_level: HeadingLevel::new(level),
        }
    }

    /// Each level keeps the scale the level-indexed table gave it, shallowest
    /// first, so no arm of the match swaps two levels.
    #[test]
    fn each_heading_level_is_drawn_at_its_own_scale() {
        let table: [f32; 6] = [2.0, 1.5, 1.25, 1.1, 1.0, 0.9];
        assert_eq!(table.len(), HeadingLevel::ALL.len());
        for (level, scale) in HeadingLevel::ALL.into_iter().zip(table) {
            assert_eq!(heading_scale(level).to_bits(), scale.to_bits(), "{level}");
        }
    }

    /// A property is taken from the first level that carries it
    /// (`PROSE-FEAT-311`).
    #[test]
    fn a_mark_wins_over_the_block_which_wins_over_the_defaults() {
        let defaults = DocDefaults::default();
        // The defaults alone.
        let plain = resolve(&MarkSet::new(), body(), &defaults);
        assert_eq!(plain.font_size, defaults.font_size);
        assert_eq!(plain.bold, Weight::Regular);

        // The block type, above the defaults.
        let titled = resolve(&MarkSet::new(), heading(1), &defaults);
        assert_eq!(
            titled.bold,
            Weight::Bold,
            "a heading is drawn bold by its type"
        );
        assert!(titled.font_size > defaults.font_size);

        // The mark, above both.
        let marked = resolve(&marks(&[("fs", "30")]), heading(1), &defaults);
        assert_eq!(marked.font_size, FontSize::from_whole_points(30));
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
        assert!(resolved.font_size > FontSize::default());
        assert!(resolved.font_family.is_some());
    }

    /// Raised beats lowered by a priority fixed in the reader, whichever order
    /// the two marks are held in (`PROSE-FEAT-174`).
    #[test]
    fn raised_beats_lowered_by_a_fixed_priority() {
        let defaults = DocDefaults::default();
        let both = resolve(&marks(&[("sup", ""), ("sub", "")]), body(), &defaults);
        assert_eq!(both.script, Script::Raised);
        // Lowered alone still resolves as lowered.
        let lowered = resolve(&marks(&[("sub", "")]), body(), &defaults);
        assert_eq!(lowered.script, Script::Lowered);
    }

    /// Two spellings of one opaque colour resolve to one value
    /// (`PROSE-FEAT-175`).
    #[test]
    fn two_spellings_of_one_colour_resolve_to_one_value() {
        let defaults = DocDefaults::default();
        let short = resolve(&marks(&[("fg", "#3366cc")]), body(), &defaults);
        let long = resolve(&marks(&[("fg", "#3366ccff")]), body(), &defaults);
        assert_eq!(short.color, long.color);
        assert_eq!(short.color, Some(Rgba::from_bits(0x3366_ccff)));
    }
}
