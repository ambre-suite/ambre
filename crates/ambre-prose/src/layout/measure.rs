//! Measuring one block: the effective style it is set in, and the height its
//! shaped lines take (`features/ambre-spec-prose-pagination.md section 2`).
//!
//! Placement reads heights and never text, so this is the one module that
//! holds a shaper. It measures and returns; it writes nothing back to the
//! document it read (`PROSE-079`).

use parley::{
    Alignment, AlignmentOptions, FontContext, FontFamily, FontStyle, FontWeight, Layout,
    LayoutContext, LineHeight, StyleProperty,
};

use ambre_bus::app::prose::{Align, DocOffset, LineHeight as StoredLineHeight, ParagraphStyle};
use ambre_core::{ByteDelta, ByteOffset, ByteSpan, DocPx, Pt, char_to_byte};

use ambre_draw::{ParleyBrush, ScriptSet};
use loro::{LoroDoc, LoroText, LoroTree, LoroValue, TextDelta, TreeID};

use crate::doc;
use crate::layout::fonts;
use crate::layout::run::{
    LocalLineCount, ParleyRunData, Resume, RunLine, RunLineCount, RunLineSpan, ShapedRun,
};

/// How many shaped stretches a run may hold before an edit shapes it whole
/// again (`PROSE-128`). Typing at the end of a paragraph adds one per line,
/// and an edit inside it at most two. Each holds a layout alive, so the cap
/// bounds the memory a run keeps.
const MAX_SEGMENTS: usize = 16;

/// A byte range of a block's text and the inline marks the shaper reads on
/// it. A mark either changes the glyphs (`PROSE-UI-207`) or draws a line
/// across them (`PROSE-UI-209`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MarkedSpan {
    pub start: ByteOffset,
    pub end: ByteOffset,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
}

/// The text of a block and the spans of it that carry bold or italic, in
/// one walk over its runs.
#[must_use]
pub fn marked_text(text: &LoroText) -> (String, Vec<MarkedSpan>) {
    let mut out = String::new();
    let mut spans = Vec::new();
    for delta in text.to_delta() {
        let TextDelta::Insert { insert, attributes } = delta else {
            continue;
        };
        // A flag mark is on when present with a value: an unmarked range may
        // keep the key with a null (`ambre-spec-prose-schema.md section 3`).
        let on = |key: &str| {
            attributes
                .as_ref()
                .and_then(|marks| marks.get(key))
                .is_some_and(|value| !matches!(value, LoroValue::Null))
        };
        let span = MarkedSpan {
            start: ByteOffset::from_usize(out.len()),
            end: ByteOffset::from_usize(out.len() + insert.len()),
            bold: on("b"),
            italic: on("i"),
            underline: on("u"),
            strike: on("s"),
        };
        out.push_str(&insert);
        if span.bold || span.italic || span.underline || span.strike {
            spans.push(span);
        }
    }
    (out, spans)
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
    pub font_size_px: DocPx,
    pub line_height: LineHeight,
    /// The floor an `AtLeast` line height sets, in pixels. `None` for the two
    /// modes parley expresses on its own.
    pub at_least_floor_px: Option<DocPx>,
    pub bold: bool,
    pub italic: bool,
    pub align: Align,
    pub indent_first_px: DocPx,
    pub indent_left_px: DocPx,
    pub indent_right_px: DocPx,
    pub space_before_px: DocPx,
    pub space_after_px: DocPx,
    pub keep_together: bool,
    pub keep_with_next: bool,
}

/// The weight a block's text is set in before its marks apply: a heading
/// is bold.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Weight {
    #[default]
    Regular,
    Bold,
}

impl EffectiveStyle {
    /// The style a block is measured in: the document defaults, with every
    /// paragraph property the block sets written over them.
    #[must_use]
    pub fn resolve(
        defaults: &doc::DocDefaults,
        tree: &LoroTree,
        id: &TreeID,
        weight: Weight,
    ) -> Self {
        let paragraph = doc::read_paragraph_style(tree, id);
        Self::from_parts(defaults, &paragraph, weight)
    }

    /// The same resolution, from parts a caller already holds.
    #[must_use]
    pub fn from_parts(
        defaults: &doc::DocDefaults,
        paragraph: &ParagraphStyle,
        weight: Weight,
    ) -> Self {
        // A stored size outside the range of a font size lays out at zero
        // rather than at a negative height.
        let font_size_px = DocPx::new(defaults.font_size.to_px());
        Self {
            font_family: defaults.font_family.clone(),
            font_size_px,
            line_height: line_height_of(paragraph, defaults),
            at_least_floor_px: at_least_floor_of(paragraph),
            bold: weight == Weight::Bold,
            italic: false,
            align: paragraph.align.unwrap_or(Align::Left),
            indent_first_px: Pt::new(paragraph.indent_first.unwrap_or(0.0)).to_doc_px(),
            indent_left_px: Pt::new(paragraph.indent_left.unwrap_or(0.0)).to_doc_px(),
            indent_right_px: Pt::new(paragraph.indent_right.unwrap_or(0.0)).to_doc_px(),
            space_before_px: Pt::new(paragraph.space_before.unwrap_or(0.0)).to_doc_px(),
            space_after_px: Pt::new(paragraph.space_after.unwrap_or(0.0)).to_doc_px(),
            keep_together: paragraph.keep_together.unwrap_or(false),
            keep_with_next: paragraph.keep_with_next.unwrap_or(false),
        }
    }

    /// The width the text is broken against: the content width, less the two
    /// side indents. Never below zero, so a pair of indents wider than the page
    /// still breaks one word per line rather than looping.
    #[must_use]
    pub fn text_width_px(&self, content_width_px: DocPx) -> DocPx {
        (content_width_px - self.indent_left_px - self.indent_right_px).max(DocPx::ZERO)
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
        Some(height) => match height {
            StoredLineHeight::Multiple(factor) => LineHeight::FontSizeRelative(factor),
            StoredLineHeight::Exact(points) => LineHeight::Absolute(points.to_doc_px().get()),
            StoredLineHeight::AtLeast(_) => LineHeight::MetricsRelative(1.0),
        },
    }
}

/// What one block's text measures, once broken to a width.
#[derive(Clone, Debug, PartialEq)]
pub struct TextMetrics {
    /// The shaped text alone, with neither margin included.
    pub text_height_px: DocPx,
    pub line_count: u32,
    /// The top of each line, relative to the top of the text. A break is only
    /// ever taken at one of these (`PROSE-071`).
    pub line_tops: Vec<DocPx>,
}

impl TextMetrics {
    /// The height the first `lines` lines take. Zero for none, the whole text
    /// for every line, and the top of the next line in between.
    #[must_use]
    pub fn height_of_first(&self, lines: u32) -> DocPx {
        if lines == 0 {
            return DocPx::ZERO;
        }
        match self.line_tops.get(lines as usize) {
            Some(top) => *top,
            None => self.text_height_px,
        }
    }

    /// How many whole lines fit in a height. Never more than the text has.
    #[must_use]
    pub fn lines_fitting_in(&self, height_px: DocPx) -> u32 {
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
#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn shipped_measurer() -> Measurer {
    let mut measurer = Measurer::new();
    for bytes in fonts::shipped_faces() {
        measurer.register_faces(bytes.into_bytes());
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
        marks: &[MarkedSpan],
        style: &EffectiveStyle,
        width_px: DocPx,
    ) -> Layout<ParleyBrush> {
        self.shaped = self.shaped.saturating_add(1);
        let absent = fonts::missing_scripts(&mut self.font_cx, text);
        self.missing.extend(absent);
        shape_paragraph(
            &mut self.font_cx,
            &mut self.layout_cx,
            (text, marks),
            style,
            width_px,
        )
    }

    /// Shape one run and keep its heights alone.
    ///
    /// For a caller that needs no glyphs. A block the view draws is measured
    /// with its run instead (`PROSE-036`).
    pub fn measure(&mut self, text: &str, style: &EffectiveStyle, width_px: DocPx) -> TextMetrics {
        metrics_of(&self.shape(text, &[], style, width_px))
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
        width_px: DocPx,
    ) -> (TextMetrics, ShapedRun) {
        let layout = self.shape(text, &[], style, width_px);
        (metrics_of(&layout), ShapedRun::styled(text, layout, style))
    }

    /// Shape one run, keep its heights and the run, and name the line each
    /// code-point offset of `offsets` falls on.
    ///
    /// One shape serves both, so a block that calls a note pays nothing more
    /// than one that does not. An offset past the text is on the last line.
    /// The spans of `marks` are shaped bold or italic (`PROSE-UI-207`).
    ///
    /// `previous` is the run the block had before an edit. When it was shaped
    /// alike, only a window of lines from the one before the edit is shaped
    /// again (`PROSE-128`, `PROSE-UI-215`).
    pub fn measure_with_lines(
        &mut self,
        (text, marks): (&str, &[MarkedSpan]),
        style: &EffectiveStyle,
        width_px: DocPx,
        offsets: &[DocOffset],
        previous: Option<&ShapedRun>,
    ) -> (TextMetrics, Vec<u32>, ShapedRun) {
        let run = previous
            .and_then(|old| self.shape_window(old.data(), (text, marks), style, width_px))
            .unwrap_or_else(|| {
                let layout = self.shape(text, marks, style, width_px);
                let mut data = ParleyRunData::new(text, layout, style.indent_left_px);
                data.shaped_in(style, marks, width_px);
                ShapedRun::from_data(data)
            });
        let lines = offsets
            .iter()
            .map(|offset| {
                let byte = char_to_byte(text, offset.as_char());
                u32::try_from(run.data().line_at_byte(byte).to_usize()).unwrap_or(u32::MAX)
            })
            .collect();
        (run_metrics(run.data()), lines, run)
    }

    /// Shape a window of `text` around an edit, keeping the lines of
    /// `previous` on both sides of it (`PROSE-128`, `PROSE-UI-215`). Counted
    /// as one run shaped, however many windows it tried.
    fn shape_window(
        &mut self,
        previous: &ParleyRunData,
        (text, marks): (&str, &[MarkedSpan]),
        style: &EffectiveStyle,
        width_px: DocPx,
    ) -> Option<ShapedRun> {
        let Self {
            font_cx,
            layout_cx,
            missing,
            ..
        } = self;
        let data = reshape_window(
            previous,
            (text, marks),
            style,
            width_px,
            &mut |part, spans, style| {
                missing.extend(fonts::missing_scripts(font_cx, part));
                shape_paragraph(font_cx, layout_cx, (part, spans), style, width_px)
            },
        )?;
        self.shaped = self.shaped.saturating_add(1);
        Some(ShapedRun::from_data(data))
    }

    /// Measure one block of the document, reading its text and its style.
    pub fn measure_block(
        &mut self,
        document: &LoroDoc,
        defaults: &doc::DocDefaults,
        id: &TreeID,
        content_width_px: DocPx,
        weight: Weight,
    ) -> (EffectiveStyle, TextMetrics) {
        let tree = doc::blocks(document);
        let style = EffectiveStyle::resolve(defaults, &tree, id, weight);
        let text = doc::get_block_text(document, id).to_string();
        let metrics = self.measure(&text, &style, style.text_width_px(content_width_px));
        (style, metrics)
    }
}

/// Shape a paragraph broken to `width_px`, as the Logic thread lays it out
/// (`PROSE-UI-201`).
///
/// The one path both threads take, so a paragraph the view shapes itself
/// breaks where the pagination will.
pub fn shape_paragraph(
    font_cx: &mut FontContext,
    layout_cx: &mut LayoutContext<ParleyBrush>,
    (text, marks): (&str, &[MarkedSpan]),
    style: &EffectiveStyle,
    width_px: DocPx,
) -> Layout<ParleyBrush> {
    let family = fonts::resolve(&style.font_family).face_name();
    let shaped = |font_cx: &mut FontContext, layout_cx: &mut _, height| {
        build(
            font_cx,
            layout_cx,
            (text, marks),
            style,
            family,
            height,
            Some(width_px),
        )
    };
    let layout = shaped(font_cx, layout_cx, style.line_height);
    // `AtLeast` is a floor on the height the font asked for, so it takes a
    // measured line to apply. One re-shape, and only for that one mode.
    if let Some(floor_px) = style.at_least_floor_px
        && layout.height() > 0.0
        && let lines = layout.len().max(1) as f32
        && layout.height() / lines < floor_px.get()
    {
        return shaped(font_cx, layout_cx, LineHeight::Absolute(floor_px.get()));
    }
    layout
}

/// Shape a paragraph again after an edit, where `previous` is its run
/// before it (`PROSE-UI-215`). The view takes this path while the layout
/// that carries the edit is on its way, so a key costs a window of lines.
///
/// A window when `previous` allows one, and the whole paragraph otherwise.
pub fn reshape_paragraph(
    font_cx: &mut FontContext,
    layout_cx: &mut LayoutContext<ParleyBrush>,
    previous: &ParleyRunData,
    (text, marks): (&str, &[MarkedSpan]),
    style: &EffectiveStyle,
    width_px: DocPx,
) -> ShapedRun {
    let windowed = reshape_window(
        previous,
        (text, marks),
        style,
        width_px,
        &mut |part, spans, style| {
            shape_paragraph(font_cx, layout_cx, (part, spans), style, width_px)
        },
    );
    match windowed {
        Some(data) => ShapedRun::from_data(data),
        None => {
            let layout = shape_paragraph(font_cx, layout_cx, (text, marks), style, width_px);
            ShapedRun::marked(text, layout, style, marks)
        }
    }
}

/// How far the window an edit is shaped in reaches, as an index into the
/// lines after the edit that it may meet again. A key moves a word or two,
/// which one of the next three lines most often takes back. A window that
/// meets none is followed by the rest of the text, shaped once. A second,
/// longer window would cost more than it saves when it misses too.
const REACH: usize = 3;

/// The share of the text after the edit a window may cover, as a divisor.
/// A window that misses is shaped for nothing, so a wider one is not tried.
/// The rest is shaped at once, and a miss costs a quarter more at most.
const WINDOW_SHARE: usize = 4;

/// What shapes one stretch of text with its marks, in a style, at the width
/// of the paragraph. The Logic thread counts what it shapes, and the view
/// does not.
type ShapeFn<'a> = dyn FnMut(&str, &[MarkedSpan], &EffectiveStyle) -> Layout<ParleyBrush> + 'a;

/// Shape a window of `text` from the line before an edit, and keep the
/// lines of `previous` on both sides of it (`PROSE-128`, `PROSE-UI-215`).
///
/// Lines are broken greedily, so a line ends where it would have ended
/// whatever follows the next one. Above the window, the lines of
/// `previous` stand. Once a line of the window starts where a line of
/// `previous` started after the edit, every line after it breaks as before.
/// The window stops there, and the old lines follow, moved by the bytes
/// the edit added. A window that meets none is followed by the rest of the
/// text, shaped once.
///
/// `None` when the shortcut does not hold. A new width, style or mark ahead
/// of the edit breaks it. So do a floor on the line height, justified or
/// right-to-left text, and too many segments already. A mark changed after
/// the edit only keeps the window from stopping early.
pub(crate) fn reshape_window(
    previous: &ParleyRunData,
    (text, marks): (&str, &[MarkedSpan]),
    style: &EffectiveStyle,
    width_px: DocPx,
    shape: &mut ShapeFn<'_>,
) -> Option<ParleyRunData> {
    let alike = previous.width_px() == Some(width_px)
        && previous.style() == Some(style)
        && style.at_least_floor_px.is_none()
        && style.align != Align::Justify
        && previous.segment_count() < MAX_SEGMENTS;
    if !alike || text.chars().any(right_to_left) {
        return None;
    }
    let old = previous.text();
    let prefix: usize = old
        .chars()
        .zip(text.chars())
        .take_while(|(old, new)| old == new)
        .map(|(c, _)| c.len_utf8())
        .sum();
    // Counted in whole characters from the end, and never into the prefix,
    // so both ends fall on a character boundary of both texts.
    let suffix = old
        .chars()
        .rev()
        .zip(text.chars().rev())
        .take_while(|(old, new)| old == new)
        .map(|(c, _)| c.len_utf8())
        .sum::<usize>()
        .min(old.len() - prefix)
        .min(text.len() - prefix);
    let delta =
        ByteDelta::new(isize::try_from(text.len()).ok()? - isize::try_from(old.len()).ok()?);
    let (old_suffix, new_suffix) = (old.len() - suffix, text.len() - suffix);
    let cut = previous
        .line_at_byte(ByteOffset::from_usize(prefix.saturating_sub(1)))
        .saturating_sub(RunLineCount::ONE);
    let start = previous.line_start(cut)?.to_usize();
    if start > prefix || !text.is_char_boundary(start) {
        return None;
    }
    if clip(previous.marks(), ByteSpan::from_usize_range(0..start))
        != clip(marks, ByteSpan::from_usize_range(0..start))
    {
        return None;
    }
    // The lines of `previous` a window may meet again: past the edit, and
    // with the same marks after it. Each with where it starts now.
    let same_after = clip(
        previous.marks(),
        ByteSpan::from_usize_range(old_suffix..old.len()),
    ) == clip(marks, ByteSpan::from_usize_range(new_suffix..text.len()));
    let meets: Vec<(RunLine, usize)> = if same_after {
        RunLineSpan::new(cut.next(), previous.line_count().as_end())
            .into_iter()
            .filter_map(|line| {
                let at = previous
                    .line_start(line)
                    .filter(|at| at.to_usize() >= old_suffix)?;
                Some((line, at.checked_offset(delta)?.to_usize()))
            })
            .collect()
    } else {
        Vec::new()
    };
    // Only the paragraph's first line is indented, and it is kept unless
    // the window starts there.
    let mut window_style = style.clone();
    if cut > RunLine::ZERO {
        window_style.indent_first_px = DocPx::ZERO;
    }
    if let Some(&(_, end)) = meets
        .get(REACH)
        .filter(|&&(_, end)| (end - start) * WINDOW_SHARE <= text.len() - start)
    {
        let window = shape(
            text.get(start..end)?,
            &clip(marks, ByteSpan::from_usize_range(start..end)),
            &window_style,
        );
        // The window's last line ends where its text ends, not where the
        // paragraph would break it, so it never answers.
        let met = (1..window.len().saturating_sub(1)).find_map(|local| {
            let at = start + window.get(local)?.text_range().start;
            let found = meets.binary_search_by_key(&at, |&(_, now)| now).ok()?;
            let &(line, _) = meets.get(found)?;
            Some(Resume {
                kept: LocalLineCount::from_len(local),
                line,
                delta,
            })
        });
        if met.is_some() {
            return ParleyRunData::with_window(previous, cut, (text, marks), window, met);
        }
    }
    let window = shape(
        text.get(start..)?,
        &clip(marks, ByteSpan::from_usize_range(start..text.len())),
        &window_style,
    );
    ParleyRunData::with_window(previous, cut, (text, marks), window, None)
}

/// The spans of `spans` over `bytes`, cut to them and counted from its
/// start.
fn clip(spans: &[MarkedSpan], bytes: ByteSpan) -> Vec<MarkedSpan> {
    spans
        .iter()
        .filter(|span| span.start < bytes.end && span.end > bytes.start)
        .map(|span| MarkedSpan {
            start: ByteOffset::ZERO + span.start.max(bytes.start).count_from(bytes.start),
            end: ByteOffset::ZERO + span.end.min(bytes.end).count_from(bytes.start),
            ..*span
        })
        .collect()
}

/// Shape a draft the way the block it is typed into was shaped
/// (`PROSE-UI-134`), on the contexts the thread that paints holds.
///
/// It is not broken into lines, since a draft is a few keystrokes long. It is
/// drawn under the caret until the layout that carries it arrives.
pub fn shape_draft(
    font_cx: &mut FontContext,
    layout_cx: &mut LayoutContext<ParleyBrush>,
    (text, marks): (&str, &[MarkedSpan]),
    style: &EffectiveStyle,
) -> Layout<ParleyBrush> {
    let family = fonts::resolve(&style.font_family).face_name();
    build(
        font_cx,
        layout_cx,
        (text, marks),
        style,
        family,
        style.line_height,
        None,
    )
}

/// `text` comes with the spans of it that carry a mark (`PROSE-UI-207`).
fn build(
    font_cx: &mut FontContext,
    layout_cx: &mut LayoutContext<ParleyBrush>,
    (text, marks): (&str, &[MarkedSpan]),
    style: &EffectiveStyle,
    family: &str,
    line_height: LineHeight,
    width_px: Option<DocPx>,
) -> Layout<ParleyBrush> {
    let mut builder = layout_cx.ranged_builder(font_cx, text, 1.0, true);
    builder.push_default(StyleProperty::FontFamily(FontFamily::Source(
        family.to_string().into(),
    )));
    builder.push_default(StyleProperty::FontSize(style.font_size_px.get()));
    builder.push_default(StyleProperty::LineHeight(line_height));
    if style.bold {
        builder.push_default(StyleProperty::FontWeight(FontWeight::BOLD));
    }
    if style.italic {
        builder.push_default(StyleProperty::FontStyle(FontStyle::Italic));
    }
    for span in marks {
        let range = ByteSpan::new(span.start, span.end).to_usize_range();
        if span.bold {
            builder.push(StyleProperty::FontWeight(FontWeight::BOLD), range.clone());
        }
        if span.italic {
            builder.push(StyleProperty::FontStyle(FontStyle::Italic), range.clone());
        }
        if span.underline {
            builder.push(StyleProperty::Underline(true), range.clone());
        }
        if span.strike {
            builder.push(StyleProperty::Strikethrough(true), range);
        }
    }
    // The first-line indent is an empty box ahead of the text, so the line
    // breaks, aligns and hit-tests around it (`PROSE-FEAT-309`). Only a whole
    // paragraph takes it: a draft is a piece typed inside one. A hanging
    // indent has no box that could express it (`PROSE-FEAT-310`).
    if width_px.is_some() && style.indent_first_px > DocPx::ZERO {
        builder.push_inline_box(parley::InlineBox {
            id: 0,
            kind: parley::InlineBoxKind::InFlow,
            index: 0,
            width: style.indent_first_px.get(),
            height: 0.0,
            baseline: None,
            vertical_align: parley::VerticalAlign::BASELINE,
        });
    }
    let mut layout: Layout<ParleyBrush> = builder.build(text);
    layout.break_all_lines(width_px.map(DocPx::get));
    layout.align(alignment_of(style.align), AlignmentOptions::default());
    layout
}

/// The floor an `AtLeast` line height sets, in pixels.
fn at_least_floor_of(paragraph: &ParagraphStyle) -> Option<DocPx> {
    match paragraph.line_height? {
        StoredLineHeight::AtLeast(floor) => Some(floor.to_doc_px()),
        StoredLineHeight::Multiple(_) | StoredLineHeight::Exact(_) => None,
    }
}

/// The line tops and the height a shaped layout carries.
fn metrics_of(layout: &Layout<ParleyBrush>) -> TextMetrics {
    let mut line_tops = Vec::with_capacity(layout.len() + 1);
    let mut top = DocPx::ZERO;
    for line in layout.lines() {
        line_tops.push(top);
        top += DocPx::new(line.metrics().line_height);
    }
    line_tops.push(DocPx::new(layout.height()));
    TextMetrics {
        text_height_px: DocPx::new(layout.height()),

        line_count: u32::try_from(layout.len()).unwrap_or(u32::MAX),
        line_tops,
    }
}

/// The heights of a run that may hold several shaped stretches.
fn run_metrics(run: &ParleyRunData) -> TextMetrics {
    let line_tops = run.line_tops();
    TextMetrics {
        text_height_px: run.height(),
        line_count: u32::try_from(line_tops.len().saturating_sub(1)).unwrap_or(u32::MAX),
        line_tops,
    }
}

/// Whether a character belongs to a right-to-left script. Such text is
/// reordered across the whole paragraph, so it is never shaped in part.
fn right_to_left(c: char) -> bool {
    matches!(
        u32::from(c),
        0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF | 0x1_0800..=0x1_0FFF | 0x1_E800..=0x1_EFFF
    ) || matches!(c, '\u{200F}' | '\u{202B}' | '\u{202E}' | '\u{2067}')
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
    use crate::layout::run::RunCount;
    use crate::layout::run::{RunOffset, RunSpan};
    use parley::BoundingBox;

    fn style() -> EffectiveStyle {
        EffectiveStyle::from_parts(
            &doc::DocDefaults::default(),
            &ParagraphStyle::default(),
            Weight::Regular,
        )
    }

    /// Enough words that no reasonable width holds them on one line.
    const LONG: &str = "The quick brown fox jumps over the lazy dog, and then \
                        it jumps over another one, and over a third after that.";

    /// Every line of `run`: where it starts, its top, its baseline in the
    /// run, and every caret box.
    #[allow(clippy::type_complexity)]
    fn picture(
        run: &ShapedRun,
    ) -> (
        Vec<usize>,
        Vec<f32>,
        Vec<f32>,
        Vec<(u32, [i64; 4], u32)>,
        Vec<[i64; 4]>,
    ) {
        let data = run.data();
        let starts = (0..data.line_count().get())
            .filter_map(|line| data.line_start(crate::layout::run::RunLine::new(line)))
            .map(ByteOffset::to_usize)
            .collect();
        let baselines = data
            .lines()
            .map(|(line, shift)| line.metrics().baseline + shift)
            .collect();
        let carets = (0..=data.len().get())
            .map(|offset| {
                let caret = data.caret_box_at(
                    RunOffset::new(offset),
                    if offset % 2 == 0 {
                        crate::layout::run::Affinity::Upstream
                    } else {
                        crate::layout::run::Affinity::Downstream
                    },
                );
                let back = data.offset_at(ambre_core::DocPoint::new(
                    ambre_core::DocPx::new(caret.x0 as f32 + 0.1),
                    ambre_core::DocPx::new(((caret.y0 + caret.y1) / 2.0) as f32),
                ));
                (offset, rounded(caret), back.get())
            })
            .collect();
        let boxes = data
            .range_boxes(RunSpan::new(
                RunOffset::new(1),
                data.len().saturating_sub(RunCount::ONE).as_end(),
            ))
            .into_iter()
            .map(rounded)
            .collect();
        let tops = data.line_tops().into_iter().map(DocPx::get).collect();
        (starts, tops, baselines, carets, boxes)
    }

    fn rounded(rect: BoundingBox) -> [i64; 4] {
        let round = |v: f64| (v * 100.0).round() as i64;
        [
            round(rect.x0),
            round(rect.y0),
            round(rect.x1),
            round(rect.y1),
        ]
    }

    /// `PROSE-128`: a run shaped again from the line before an edit breaks,
    /// draws and places its caret as a run shaped whole would.
    #[test]
    fn a_run_shaped_from_the_edit_on_matches_one_shaped_whole() {
        let mut indented = style();
        indented.indent_first_px = DocPx::new(40.0);
        let mut centered = style();
        centered.align = Align::Center;
        let bold = |text: &str| {
            vec![MarkedSpan {
                start: ByteOffset::new(20),
                end: ByteOffset::from_usize(text.len() / 2),
                bold: true,
                ..MarkedSpan::default()
            }]
        };
        let plain = |_: &str| Vec::new();
        shaped_in_part_as_whole(&style(), &plain);
        shaped_in_part_as_whole(&indented, &bold);
        shaped_in_part_as_whole(&centered, &bold);
    }

    fn shaped_in_part_as_whole(style: &EffectiveStyle, marks_of: &dyn Fn(&str) -> Vec<MarkedSpan>) {
        let mut measurer = super::shipped_measurer();
        let style = style.clone();
        let width = 300.0;
        let mut text = LONG.repeat(12);
        let marks = marks_of(&text);
        let (_, _, mut run) =
            measurer.measure_with_lines((&text, &marks), &style, DocPx::new(width), &[], None);
        // Type at the end, in the middle, near the start; erase; paste a
        // long word; and keep going past the segment cap.
        let edits: Vec<(usize, &str, usize)> = (0..40)
            .map(|step| match step % 5 {
                0 => (text.len(), "a", 0),
                1 => (text.len() / 2, "b ", 0),
                2 => (90, "c", 0),
                3 => (text.len() / 3, "", 3),
                _ => (text.len() * 3 / 4, " unbreakablewordthatislong ", 0),
            })
            .collect();
        let mut partial = 0;
        for (at, insert, erase) in edits {
            let mut at = at.min(text.len());
            while !text.is_char_boundary(at) {
                at -= 1;
            }
            let end = (at + erase).min(text.len());
            text.replace_range(at..end, insert);
            let marks = marks_of(&text);
            let before = measurer.shaped();
            let (metrics, _, next) = measurer.measure_with_lines(
                (&text, &marks),
                &style,
                DocPx::new(width),
                &[],
                Some(&run),
            );
            let (whole_metrics, _, whole) =
                measurer.measure_with_lines((&text, &marks), &style, DocPx::new(width), &[], None);
            assert_eq!(measurer.shaped(), before + 2);
            if next.data().segment_count() > 1 {
                partial += 1;
            }
            assert_eq!(metrics, whole_metrics, "heights after an edit at {at}");
            assert_eq!(
                picture(&next),
                picture(&whole),
                "lines after an edit at {at}"
            );
            run = next;
        }
        assert!(partial > 10, "the shortcut was taken {partial} times");
    }

    /// `PROSE-128`: an edit near the start of a long paragraph shapes a
    /// few lines around it and keeps the rest, moved by what it added. The
    /// run still breaks, draws and places its caret as a whole shape does.
    #[test]
    fn an_edit_near_the_start_shapes_a_few_lines_and_keeps_the_rest() {
        let mut measurer = super::shipped_measurer();
        let style = style();
        let width = 300.0;
        let base = LONG.repeat(12);
        let bold = |text: &str| {
            vec![MarkedSpan {
                start: ByteOffset::from_usize(text.len() - 200),
                end: ByteOffset::from_usize(text.len() - 100),
                bold: true,
                ..MarkedSpan::default()
            }]
        };
        let (_, _, run) = measurer.measure_with_lines(
            (&base, &bold(&base)),
            &style,
            DocPx::new(width),
            &[],
            None,
        );
        assert!(run.data().line_count().get() > 20);
        // A letter, one in the first line, and two accented ones. A key
        // moves a word or two, and a few lines take it back. An erased
        // word and a pasted phrase can move every break after them, and the
        // rest of the paragraph is shaped.
        let edits: [(usize, &str, usize, usize); 5] = [
            (30, "x", 0, 4),
            (45, "", 6, usize::MAX),
            (3, "q", 0, 4),
            (50, "\u{e9}\u{e8}", 0, 4),
            (60, " averyveryverylongwordindeed ", 0, usize::MAX),
        ];
        for (at, insert, erase, most) in edits {
            let mut text = base.clone();
            text.replace_range(at..at + erase, insert);
            let marks = bold(&text);
            let (metrics, _, windowed) = measurer.measure_with_lines(
                (&text, &marks),
                &style,
                DocPx::new(width),
                &[],
                Some(&run),
            );
            let (whole_metrics, _, whole) =
                measurer.measure_with_lines((&text, &marks), &style, DocPx::new(width), &[], None);
            let fresh = windowed.data().fresh_lines(run.data());
            assert!(fresh <= most, "{fresh} lines shaped for an edit at {at}");
            assert_eq!(metrics, whole_metrics, "heights after an edit at {at}");
            assert_eq!(
                picture(&windowed),
                picture(&whole),
                "lines after an edit at {at}"
            );
        }
    }

    /// `PROSE-128`: a mark changed after the edit is drawn: the window
    /// runs to the end rather than keep lines shaped without it.
    #[test]
    fn a_mark_changed_past_the_edit_is_shaped_again() {
        let mut measurer = super::shipped_measurer();
        let style = style();
        let text = LONG.repeat(12);
        let (_, _, run) =
            measurer.measure_with_lines((&text, &[]), &style, DocPx::new(300.0), &[], None);
        let edited = format!("x{text}");
        let marks = [MarkedSpan {
            start: ByteOffset::from_usize(edited.len() - 300),
            end: ByteOffset::from_usize(edited.len() - 10),
            bold: true,
            ..MarkedSpan::default()
        }];
        let (_, _, windowed) = measurer.measure_with_lines(
            (&edited, &marks),
            &style,
            DocPx::new(300.0),
            &[],
            Some(&run),
        );
        let (_, _, whole) =
            measurer.measure_with_lines((&edited, &marks), &style, DocPx::new(300.0), &[], None);
        assert_eq!(picture(&windowed), picture(&whole));
    }

    /// `PROSE-128`: a style or a width that changed shapes the run whole.
    #[test]
    fn a_new_width_shapes_the_run_whole() {
        let mut measurer = super::shipped_measurer();
        let style = style();
        let text = LONG.repeat(6);
        let (_, _, run) =
            measurer.measure_with_lines((&text, &[]), &style, DocPx::new(300.0), &[], None);
        let longer = format!("{text}x");
        let (_, _, wider) =
            measurer.measure_with_lines((&longer, &[]), &style, DocPx::new(320.0), &[], Some(&run));
        assert_eq!(wider.data().segment_count(), 1);
        let (_, _, same) =
            measurer.measure_with_lines((&longer, &[]), &style, DocPx::new(300.0), &[], Some(&run));
        assert_eq!(same.data().segment_count(), 2);
    }

    #[test]
    fn a_narrow_width_breaks_a_run_into_more_lines_than_a_wide_one() {
        let mut measurer = super::shipped_measurer();
        let style = style();
        let wide = measurer.measure(LONG, &style, DocPx::new(2000.0));
        let narrow = measurer.measure(LONG, &style, DocPx::new(120.0));
        assert!(narrow.line_count > wide.line_count);
        assert!(narrow.text_height_px > wide.text_height_px);
    }

    #[test]
    fn the_line_tops_climb_and_end_at_the_measured_height() {
        let mut measurer = super::shipped_measurer();
        let metrics = measurer.measure(LONG, &style(), DocPx::new(200.0));
        assert!(metrics.line_count >= 2);
        for pair in metrics.line_tops.windows(2) {
            let [first, second] = pair else { continue };
            assert!(second >= first);
        }
        assert_eq!(metrics.line_tops.first(), Some(&DocPx::ZERO));
        assert_eq!(metrics.line_tops.last(), Some(&metrics.text_height_px));
    }

    #[test]
    fn a_height_holding_no_whole_line_fits_none_of_them() {
        let mut measurer = super::shipped_measurer();
        let metrics = measurer.measure(LONG, &style(), DocPx::new(200.0));
        assert_eq!(metrics.lines_fitting_in(DocPx::new(0.0)), 0);
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
        assert!((Pt::new(12.0).to_doc_px().get() - 16.0).abs() < 0.001);
        assert!((Pt::new(72.0).to_doc_px().get() - 96.0).abs() < 0.001);
    }

    #[test]
    fn the_body_size_lays_out_as_every_view_lays_it_out() {
        // Trame and Toile read `FontSize::to_px` too, so twelve points draw at
        // one height in all three applications.
        let defaults = doc::DocDefaults {
            font_size: ambre_core::FontSize::from_whole_points(12),
            ..doc::DocDefaults::default()
        };
        let style =
            EffectiveStyle::from_parts(&defaults, &ParagraphStyle::default(), Weight::Regular);
        let twelve = ambre_core::FontSize::from_whole_points(12);
        assert_eq!(style.font_size_px, DocPx::new(twelve.to_px()));
        assert_eq!(style.font_size_px, DocPx::new(16.0));
    }

    #[test]
    fn two_indents_wider_than_the_page_still_leave_a_width_to_break_against() {
        let mut style = style();
        style.indent_left_px = DocPx::new(900.0);
        style.indent_right_px = DocPx::new(900.0);
        assert_eq!(style.text_width_px(DocPx::new(400.0)), DocPx::new(0.0));
    }

    #[test]
    fn an_exact_line_height_is_the_height_every_line_takes() {
        let mut measurer = super::shipped_measurer();
        let mut style = style();
        style.line_height = LineHeight::Absolute(30.0);
        style.at_least_floor_px = None;
        let metrics = measurer.measure(LONG, &style, DocPx::new(200.0));
        assert!(metrics.line_count >= 2);
        let expected = 30.0 * metrics.line_count as f32;
        assert!((metrics.text_height_px - DocPx::new(expected)).abs() < DocPx::new(0.5));
    }

    #[test]
    fn an_at_least_floor_taller_than_the_font_raises_every_line() {
        let mut measurer = super::shipped_measurer();
        let mut style = style();
        style.line_height = LineHeight::MetricsRelative(1.0);
        style.at_least_floor_px = None;
        let natural = measurer.measure(LONG, &style, DocPx::new(200.0));
        style.at_least_floor_px = Some(DocPx::new(60.0));
        let floored = measurer.measure(LONG, &style, DocPx::new(200.0));
        assert_eq!(floored.line_count, natural.line_count);
        assert!(floored.text_height_px > natural.text_height_px);
    }

    #[test]
    fn an_at_least_floor_below_the_font_leaves_the_lines_alone() {
        let mut measurer = super::shipped_measurer();
        let mut style = style();
        style.line_height = LineHeight::MetricsRelative(1.0);
        style.at_least_floor_px = None;
        let natural = measurer.measure(LONG, &style, DocPx::new(200.0));
        style.at_least_floor_px = Some(DocPx::new(1.0));
        let floored = measurer.measure(LONG, &style, DocPx::new(200.0));
        assert_eq!(floored, natural);
    }
}
