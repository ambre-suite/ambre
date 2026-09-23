//! Measuring one block: the effective style it is set in, and the height its
//! shaped lines take (`features/ambre-spec-prose-pagination.md section 2`).
//!
//! Placement reads heights and never text, so this is the one module that
//! holds a shaper. It measures and returns; it writes nothing back to the
//! document it read (`PROSE-079`).

use parley::{
    Alignment, AlignmentOptions, FontContext, FontStack, FontStyle, FontWeight, Layout,
    LayoutContext, LineHeight, StyleProperty,
};

use ambre_bus::app::prose::{Align, LineHeightMode, ParagraphStyle};
use ambre_draw::{ParleyBrush, ScriptSet};
use loro::{LoroDoc, LoroTree, TreeID};

use crate::doc;
use crate::layout::fonts;
use crate::layout::run::ShapedRun;

/// Pixels per point at 96 DPI. Every length the document stores in points is
/// converted here and nowhere else.
const PX_PER_PT: f32 = 96.0 / 72.0;

/// The height one point converts to.
#[must_use]
pub fn px(points: f32) -> f32 {
    points * PX_PER_PT
}

/// The style one block is measured in.
///
/// It resolves the document defaults against the block's own paragraph
/// settings. That is the bottom of the four-level cascade, and all of the
/// cascade that exists today. The named built-in styles land with the
/// "Styles, lists and headings" Story (`ambre-roadmap.md` E5), and this is
/// where their sizes will arrive.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectiveStyle {
    pub font_family: String,
    pub font_size_px: f32,
    pub line_height: LineHeight,
    /// The floor an `AtLeast` line height sets, in pixels. `None` for the two
    /// modes parley expresses on its own.
    pub at_least_floor_px: Option<f32>,
    pub bold: bool,
    pub italic: bool,
    pub align: Align,
    pub indent_first_px: f32,
    pub indent_left_px: f32,
    pub indent_right_px: f32,
    pub space_before_px: f32,
    pub space_after_px: f32,
    pub keep_together: bool,
    pub keep_with_next: bool,
}

impl EffectiveStyle {
    /// The style a block is measured in: the document defaults, with every
    /// paragraph property the block sets written over them.
    #[must_use]
    pub fn resolve(defaults: &doc::DocDefaults, tree: &LoroTree, id: &TreeID, bold: bool) -> Self {
        let paragraph = doc::read_paragraph_style(tree, id);
        Self::from_parts(defaults, &paragraph, bold)
    }

    /// The same resolution, from parts a caller already holds.
    #[must_use]
    pub fn from_parts(defaults: &doc::DocDefaults, paragraph: &ParagraphStyle, bold: bool) -> Self {
        let font_size_px = px(defaults.font_size_pt);
        Self {
            font_family: defaults.font_family.clone(),
            font_size_px,
            line_height: line_height_of(paragraph, defaults),
            at_least_floor_px: at_least_floor_of(paragraph),
            bold,
            italic: false,
            align: paragraph.align.unwrap_or(Align::Left),
            indent_first_px: px(paragraph.indent_first.unwrap_or(0.0)),
            indent_left_px: px(paragraph.indent_left.unwrap_or(0.0)),
            indent_right_px: px(paragraph.indent_right.unwrap_or(0.0)),
            space_before_px: px(paragraph.space_before.unwrap_or(0.0)),
            space_after_px: px(paragraph.space_after.unwrap_or(0.0)),
            keep_together: paragraph.keep_together.unwrap_or(false),
            keep_with_next: paragraph.keep_with_next.unwrap_or(false),
        }
    }

    /// The width the text is broken against: the content width, less the two
    /// side indents. Never below zero, so a pair of indents wider than the page
    /// still breaks one word per line rather than looping.
    #[must_use]
    pub fn text_width_px(&self, content_width_px: f32) -> f32 {
        (content_width_px - self.indent_left_px - self.indent_right_px).max(0.0)
    }
}

/// The line-height parley is asked for, from the three modes the document has.
///
/// `AtLeast` has no parley equivalent: it is a floor on a height only shaping
/// knows. It is handed over as the font's own height here, and the floor is
/// applied by [`Measurer::shape`], which has the measured line to compare.
fn line_height_of(paragraph: &ParagraphStyle, defaults: &doc::DocDefaults) -> LineHeight {
    match paragraph.line_height {
        None => LineHeight::FontSizeRelative(defaults.line_height),
        Some(height) => match height.mode {
            LineHeightMode::Multiple => LineHeight::FontSizeRelative(height.value),
            LineHeightMode::Exact => LineHeight::Absolute(px(height.value)),
            LineHeightMode::AtLeast => LineHeight::MetricsRelative(1.0),
        },
    }
}

/// What one block's text measures, once broken to a width.
#[derive(Clone, Debug, PartialEq)]
pub struct TextMetrics {
    /// The shaped text alone, with neither margin included.
    pub text_height_px: f32,
    pub line_count: u32,
    /// The top of each line, relative to the top of the text. A break is only
    /// ever taken at one of these (`PROSE-071`).
    pub line_tops: Vec<f32>,
}

impl TextMetrics {
    /// The height the first `lines` lines take. Zero for none, the whole text
    /// for every line, and the top of the next line in between.
    #[must_use]
    pub fn height_of_first(&self, lines: u32) -> f32 {
        if lines == 0 {
            return 0.0;
        }
        match self.line_tops.get(lines as usize) {
            Some(top) => *top,
            None => self.text_height_px,
        }
    }

    /// How many whole lines fit in a height. Never more than the text has.
    #[must_use]
    pub fn lines_fitting_in(&self, height_px: f32) -> u32 {
        let mut fitting = 0;
        for line in 1..=self.line_count {
            if self.height_of_first(line) > height_px {
                break;
            }
            fitting = line;
        }
        fitting
    }
}

/// A measurer holding the faces the application ships.
///
/// Tests only: it stands in for the `LoadFonts` delivery a running peer gets.
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) fn shipped_measurer() -> Measurer {
    let mut measurer = Measurer::new();
    for bytes in fonts::shipped_faces() {
        measurer.register_faces(bytes);
    }
    measurer
}

/// The shaper, and the two contexts parley reuses between blocks.
///
/// One per layout pass. Holding it across a pass is what keeps a font from
/// being parsed once per paragraph.
pub struct Measurer {
    font_cx: FontContext,
    layout_cx: LayoutContext<ParleyBrush>,
    /// How many runs have been shaped. It is what makes a reflow that
    /// re-shapes an untouched block observable (`PROSE-083`).
    shaped: u64,
    /// The scripts met since the last full run that no loaded face covers
    /// (`PROSE-110`). A layout carries it so a consumer can tell a settled
    /// measurement from one still waiting for a face.
    missing: ScriptSet,
}

impl Default for Measurer {
    fn default() -> Self {
        Self::new()
    }
}

impl Measurer {
    /// A measurer over a sealed context: no face is in it until one is
    /// registered (`PROSE-114`).
    ///
    /// This is the only way to build one, so no caller can hand in a context
    /// that reads the device.
    #[must_use]
    pub fn new() -> Self {
        Self {
            font_cx: ambre_draw::sealed_font_context(),
            layout_cx: LayoutContext::new(),
            shaped: 0,
            missing: ScriptSet::new(),
        }
    }

    /// How many runs this measurer has shaped since it was built.
    #[must_use]
    pub fn shaped(&self) -> u64 {
        self.shaped
    }

    /// The scripts this measurer met and had no face for (`PROSE-110`).
    ///
    /// It accumulates over every run shaped since the last [`forget_missing`]
    /// call. Empty means every measurement had the faces it asked for.
    ///
    /// [`forget_missing`]: Self::forget_missing
    #[must_use]
    pub fn missing(&self) -> ScriptSet {
        self.missing
    }

    /// Drop what the measurer recorded as missing, before a full run.
    pub fn forget_missing(&mut self) {
        self.missing = ScriptSet::new();
    }

    /// Load a face into the measurer, from the bytes of a font file.
    ///
    /// The path a lazily fetched script face takes to the paginator
    /// (`CANVAS-006`). The layout measured before it arrived is stale, and
    /// the next reflow recomputes in full (`PROSE-111`).
    pub fn register_faces(&mut self, bytes: Vec<u8>) {
        fonts::register_faces(&mut self.font_cx, bytes);
    }

    /// Whether a face has arrived for a script a past run was missing.
    ///
    /// The layout it produced is stale the moment one does (`PROSE-111`).
    pub fn coverage_grew(&mut self, missing: ScriptSet) -> bool {
        fonts::coverage_grew(&mut self.font_cx, missing)
    }

    /// Shape one run to a width, and hand back the layout.
    ///
    /// The family is resolved to a shipped face before the text reaches the
    /// shaper (`PROSE-034`, `PROSE-108`). A label no shipped face carries is
    /// replaced by one that has glyphs, rather than shaped into nothing.
    ///
    /// A script no loaded face covers is recorded rather than hidden: the run
    /// still shapes, into tofu, and [`missing`](Self::missing) names it.
    pub fn shape(
        &mut self,
        text: &str,
        style: &EffectiveStyle,
        width_px: f32,
    ) -> Layout<ParleyBrush> {
        self.shaped = self.shaped.saturating_add(1);
        let absent = fonts::missing_scripts(&mut self.font_cx, text);
        self.missing.extend(absent);
        let family = fonts::resolve(&style.font_family).face_name();
        let mut layout = self.build(text, style, family, style.line_height, width_px);
        // `AtLeast` is a floor on the height the font asked for, so it takes a
        // measured line to apply. One re-shape, and only for that one mode.
        if let Some(floor_px) = style.at_least_floor_px
            && layout.height() > 0.0
            && let lines = layout.len().max(1) as f32
            && layout.height() / lines < floor_px
        {
            layout = self.build(
                text,
                style,
                family,
                LineHeight::Absolute(floor_px),
                width_px,
            );
        }
        layout
    }

    /// Shape one run and keep its heights alone.
    ///
    /// For a caller that needs no glyphs. A block the view draws is measured
    /// with its run instead (`PROSE-036`).
    pub fn measure(&mut self, text: &str, style: &EffectiveStyle, width_px: f32) -> TextMetrics {
        metrics_of(&self.shape(text, style, width_px))
    }

    /// Shape one run, keep its heights and the shaped run itself.
    ///
    /// The run is what the view draws and hit-tests, so the text is shaped
    /// once, here, and never again on the thread that paints
    /// (`PROSE-UI-006`).
    pub fn measure_run(
        &mut self,
        text: &str,
        style: &EffectiveStyle,
        width_px: f32,
    ) -> (TextMetrics, ShapedRun) {
        let layout = self.shape(text, style, width_px);
        (
            metrics_of(&layout),
            ShapedRun::new(text, layout, style.indent_left_px),
        )
    }

    /// Shape one run, keep its heights and the run, and name the line each
    /// code-point offset of `offsets` falls on.
    ///
    /// One shape serves both, so a block that calls a note pays nothing more
    /// than one that does not. An offset past the text is on the last line.
    pub fn measure_with_lines(
        &mut self,
        text: &str,
        style: &EffectiveStyle,
        width_px: f32,
        offsets: &[u32],
    ) -> (TextMetrics, Vec<u32>, ShapedRun) {
        let layout = self.shape(text, style, width_px);
        let last = layout.len().saturating_sub(1);
        let lines = offsets
            .iter()
            .map(|offset| {
                let byte = text
                    .char_indices()
                    .nth(*offset as usize)
                    .map_or(text.len(), |(at, _)| at);
                let line = layout
                    .lines()
                    .position(|line| line.text_range().contains(&byte))
                    .unwrap_or(last);
                u32::try_from(line).unwrap_or(u32::MAX)
            })
            .collect();
        (
            metrics_of(&layout),
            lines,
            ShapedRun::new(text, layout, style.indent_left_px),
        )
    }

    /// Measure one block of the document, reading its text and its style.
    pub fn measure_block(
        &mut self,
        document: &LoroDoc,
        defaults: &doc::DocDefaults,
        id: &TreeID,
        content_width_px: f32,
        bold: bool,
    ) -> (EffectiveStyle, TextMetrics) {
        let tree = document.get_tree(doc::BLOCKS);
        let style = EffectiveStyle::resolve(defaults, &tree, id, bold);
        let text = doc::get_block_text(document, id).to_string();
        let metrics = self.measure(&text, &style, style.text_width_px(content_width_px));
        (style, metrics)
    }

    fn build(
        &mut self,
        text: &str,
        style: &EffectiveStyle,
        family: &str,
        line_height: LineHeight,
        width_px: f32,
    ) -> Layout<ParleyBrush> {
        let mut builder = self
            .layout_cx
            .ranged_builder(&mut self.font_cx, text, 1.0, true);
        builder.push_default(StyleProperty::FontStack(FontStack::Source(
            family.to_string().into(),
        )));
        builder.push_default(StyleProperty::FontSize(style.font_size_px));
        builder.push_default(StyleProperty::LineHeight(line_height));
        if style.bold {
            builder.push_default(StyleProperty::FontWeight(FontWeight::BOLD));
        }
        if style.italic {
            builder.push_default(StyleProperty::FontStyle(FontStyle::Italic));
        }
        let mut layout: Layout<ParleyBrush> = builder.build(text);
        layout.break_all_lines(Some(width_px));
        layout.align(
            Some(width_px),
            alignment_of(style.align),
            AlignmentOptions::default(),
        );
        layout
    }
}

/// The floor an `AtLeast` line height sets, in pixels.
fn at_least_floor_of(paragraph: &ParagraphStyle) -> Option<f32> {
    let height = paragraph.line_height?;
    match height.mode {
        LineHeightMode::AtLeast => Some(px(height.value)),
        LineHeightMode::Multiple | LineHeightMode::Exact => None,
    }
}

/// The line tops and the height a shaped layout carries.
fn metrics_of(layout: &Layout<ParleyBrush>) -> TextMetrics {
    let mut line_tops = Vec::with_capacity(layout.len() + 1);
    let mut top = 0.0;
    for line in layout.lines() {
        line_tops.push(top);
        top += line.metrics().line_height;
    }
    line_tops.push(layout.height());
    TextMetrics {
        text_height_px: layout.height(),
        line_count: u32::try_from(layout.len()).unwrap_or(u32::MAX),
        line_tops,
    }
}

/// The parley alignment one document alignment asks for.
fn alignment_of(align: Align) -> Alignment {
    match align {
        Align::Left => Alignment::Left,
        Align::Center => Alignment::Center,
        Align::Right => Alignment::Right,
        Align::Justify => Alignment::Justify,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style() -> EffectiveStyle {
        EffectiveStyle::from_parts(
            &doc::DocDefaults::default(),
            &ParagraphStyle::default(),
            false,
        )
    }

    /// Enough words that no reasonable width holds them on one line.
    const LONG: &str = "The quick brown fox jumps over the lazy dog, and then \
                        it jumps over another one, and over a third after that.";

    #[test]
    fn a_narrow_width_breaks_a_run_into_more_lines_than_a_wide_one() {
        let mut measurer = super::shipped_measurer();
        let style = style();
        let wide = measurer.measure(LONG, &style, 2000.0);
        let narrow = measurer.measure(LONG, &style, 120.0);
        assert!(narrow.line_count > wide.line_count);
        assert!(narrow.text_height_px > wide.text_height_px);
    }

    #[test]
    fn the_line_tops_climb_and_end_at_the_measured_height() {
        let mut measurer = super::shipped_measurer();
        let metrics = measurer.measure(LONG, &style(), 200.0);
        assert!(metrics.line_count >= 2);
        for pair in metrics.line_tops.windows(2) {
            let [first, second] = pair else { continue };
            assert!(second >= first);
        }
        assert_eq!(metrics.line_tops.first(), Some(&0.0));
        assert_eq!(metrics.line_tops.last(), Some(&metrics.text_height_px));
    }

    #[test]
    fn a_height_holding_no_whole_line_fits_none_of_them() {
        let mut measurer = super::shipped_measurer();
        let metrics = measurer.measure(LONG, &style(), 200.0);
        assert_eq!(metrics.lines_fitting_in(0.0), 0);
        assert_eq!(
            metrics.lines_fitting_in(metrics.text_height_px),
            metrics.line_count
        );
        let two_lines = metrics.height_of_first(2);
        assert_eq!(metrics.lines_fitting_in(two_lines), 2);
    }

    #[test]
    fn a_font_size_in_points_is_measured_in_pixels() {
        // 96 DPI: twelve points is sixteen pixels, and nothing rounds on the
        // way (`features/ambre-spec-prose-pagination.md section 2`).
        assert!((px(12.0) - 16.0).abs() < 0.001);
        assert!((px(72.0) - 96.0).abs() < 0.001);
    }

    #[test]
    fn two_indents_wider_than_the_page_still_leave_a_width_to_break_against() {
        let mut style = style();
        style.indent_left_px = 900.0;
        style.indent_right_px = 900.0;
        assert_eq!(style.text_width_px(400.0), 0.0);
    }

    #[test]
    fn an_exact_line_height_is_the_height_every_line_takes() {
        let mut measurer = super::shipped_measurer();
        let mut style = style();
        style.line_height = LineHeight::Absolute(30.0);
        style.at_least_floor_px = None;
        let metrics = measurer.measure(LONG, &style, 200.0);
        assert!(metrics.line_count >= 2);
        let expected = 30.0 * metrics.line_count as f32;
        assert!((metrics.text_height_px - expected).abs() < 0.5);
    }

    #[test]
    fn an_at_least_floor_taller_than_the_font_raises_every_line() {
        let mut measurer = super::shipped_measurer();
        let mut style = style();
        style.line_height = LineHeight::MetricsRelative(1.0);
        style.at_least_floor_px = None;
        let natural = measurer.measure(LONG, &style, 200.0);
        style.at_least_floor_px = Some(60.0);
        let floored = measurer.measure(LONG, &style, 200.0);
        assert_eq!(floored.line_count, natural.line_count);
        assert!(floored.text_height_px > natural.text_height_px);
    }

    #[test]
    fn an_at_least_floor_below_the_font_leaves_the_lines_alone() {
        let mut measurer = super::shipped_measurer();
        let mut style = style();
        style.line_height = LineHeight::MetricsRelative(1.0);
        style.at_least_floor_px = None;
        let natural = measurer.measure(LONG, &style, 200.0);
        style.at_least_floor_px = Some(1.0);
        let floored = measurer.measure(LONG, &style, 200.0);
        assert_eq!(floored, natural);
    }
}
