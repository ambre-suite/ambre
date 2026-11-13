//! A page as a print draws it (`ambre-spec-pdf-export.md`, `PDF-009`).
//!
//! The print resolves a page as the screen resolves it, then runs the
//! screen's own [`PageWalk`] over it. It draws no sheet, no shadow, no caret
//! and no selection: the page of the file is the sheet. The view state it
//! paints from is its own, built on the Logic thread for one export. The
//! view the reader looks at is never touched (`PROSE-IO-039`).
//!
//! A page is drawn from its top-left corner at zoom 1, in document pixels.
//! The caller scales it to points.

use std::collections::HashMap;
use std::sync::Arc;

use ambre_core::index::{Pos, Span};
use ambre_core::{Count, Px, Surface};
use ambre_draw::{Color, DocumentWalk, DrawSurface, Rect};
use loro::TreeID;

use crate::events::{Page, ProseEvent};
use crate::layout::run::{RunChar, ShapedLayout, ShapedRun};
use crate::ui::ProseUiState;
use crate::view::cache::{RunCache, RunKey};
use crate::view::column::PageColumn;
use crate::view::paint::Frame;
use crate::view::walk::{
    Highlights, PageItem, PagePaint, PagePalette, PageView, PageWalk, Placed, to_rect,
};

/// The colours of a print: black text on white paper, whatever the theme
/// the screen is drawn in. A dark theme would print white text on a page
/// no reader prints white.
pub(crate) const PRINT_PALETTE: PagePalette = PagePalette {
    sheet: Color::from_rgba8(255, 255, 255, 255),
    sheet_border: Color::from_rgba8(160, 160, 160, 255),
    shadow: Color::from_rgba8(0, 0, 0, 0),
    text: Color::from_rgba8(0, 0, 0, 255),
    accent: Color::from_rgba8(0, 90, 200, 255),
    warning: Color::from_rgba8(230, 160, 0, 255),
    success: Color::from_rgba8(0, 140, 60, 255),
    error: Color::from_rgba8(200, 30, 30, 255),
};

/// A block a printed page drew text for: its id, its run, and where the
/// run was placed on the page.
pub(crate) struct PrintedBlock {
    pub block: TreeID,
    pub run: ShapedRun,
    pub placed: Placed,
}

/// Where a span of text leads in the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LinkTo {
    /// An address a reader may open.
    Address(ambre_core::LinkUrl),
    /// The top of a block of the document.
    Block(TreeID),
}

/// A span of a block's text that leads somewhere, counted in the run the
/// layout laid out.
#[derive(Clone, Debug)]
pub(crate) struct LinkSource {
    pub span: Span<RunChar>,
    pub to: LinkTo,
}

/// The area a link covers on a page, in document pixels from the page's
/// top-left corner, and where it leads.
#[derive(Clone, Debug)]
pub(crate) struct LinkArea {
    pub area: Rect,
    pub to: LinkTo,
}

impl ProseUiState {
    /// A view that prints: no caret, no selection, no peer and no draft.
    /// It marks the suggestions `suggestions` lists, as the screen marks
    /// them, and no comment and no find match.
    pub(crate) fn printing(suggestions: &ProseEvent) -> Self {
        let mut view = Self::default();
        view.apply_event(suggestions);
        view
    }

    /// Print from `shaped` from now on, whatever its version. A print takes
    /// the windows of one layout in turn, and each carries the same version.
    /// The screen's rule that drops an older layout does not apply.
    ///
    /// The drawing state of the runs of the last window is dropped, so a
    /// print holds the pages of one window at a time.
    pub(crate) fn take_print_window(&mut self, shaped: ShapedLayout) {
        self.column = PageColumn::of(&shaped.layout);
        self.shaped = Arc::new(shaped);
        self.runs = RunCache::default();
    }

    /// Draw `page` alone onto `surface`, from its top-left corner, with the
    /// screen's walk. Answers each block it drew text for.
    pub(crate) fn print_page<S: DrawSurface>(
        &mut self,
        surface: &mut S,
        page: Pos<Page>,
    ) -> Vec<PrintedBlock> {
        let mut frame = Frame::default();
        self.resolve_page(&mut frame, page, Px::<Surface>::ZERO);
        let width = Px::<Surface>::new(f64::from(self.shaped.layout.page_width.get()));
        let height = Px::<Surface>::new(f64::from(self.shaped.layout.page_height.get()));
        let view = PageView {
            palette: &PRINT_PALETTE,
            width,
            height,
            pages: &frame.pages,
            pictures: &self.pictures,
        };
        let mut walk = PageWalk {
            runs: &mut self.runs,
            highlights_walked: Count::<Highlights>::ZERO,
        };
        walk.walk(
            &view,
            Rect::new(0.0, 0.0, width.get(), height.get()),
            surface,
        );
        frame.pages.iter().flat_map(printed_blocks).collect()
    }
}

/// Each block of text a resolved page draws: in the body, in a table cell,
/// in a band and in a footnote.
fn printed_blocks(page: &PagePaint) -> Vec<PrintedBlock> {
    let mut out = Vec::new();
    let mut push = |key: RunKey, run: &ShapedRun, placed: Placed| {
        if let RunKey::Block(block) = key {
            out.push(PrintedBlock {
                block,
                run: run.clone(),
                placed,
            });
        }
    };
    for item in &page.items {
        match item {
            PageItem::Block(block) => push(block.key, &block.run, block.placed),
            PageItem::Table { cells, .. } => {
                for block in cells.iter().flat_map(|cell| cell.blocks.iter()) {
                    push(block.key, &block.run, block.placed);
                }
            }
            PageItem::Shape { .. } | PageItem::Rule { .. } | PageItem::Picture(_) => {}
        }
    }
    out
}

/// The area every link of `sources` covers on the page `printed` came
/// from. A span is cut to the part of its block the page shows. A block
/// broken across two pages links on each page from the part it holds there.
pub(crate) fn link_areas(
    printed: &[PrintedBlock],
    sources: &HashMap<TreeID, Vec<LinkSource>>,
) -> Vec<LinkArea> {
    let mut out = Vec::new();
    for block in printed {
        let Some(links) = sources.get(&block.block) else {
            continue;
        };
        let data = block.run.data();
        for link in links {
            for part in data.range_boxes(link.span) {
                let area = to_rect(&block.placed, &part).intersect(block.placed.clip);
                if area.width() > 0.0 && area.height() > 0.0 {
                    out.push(LinkArea {
                        area,
                        to: link.to.clone(),
                    });
                }
            }
        }
    }
    out
}
