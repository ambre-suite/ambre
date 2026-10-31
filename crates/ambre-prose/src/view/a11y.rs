// ambre-prose/src/view/a11y.rs
// ============================================================================
// What a screen reader hears of the page (`ambre-spec-accessibility.md`,
// `CHROME-023`). The focused unit of a page is the block that holds the
// caret: its kind, then its text.
//
// The host announces it when the caret enters another block. A move inside
// the block says nothing, or each arrow press would read the paragraph again.
// ============================================================================

use ambre_bus::app::prose::BlockType;

use crate::events::BlockId;
use crate::ui::ProseUiState;

/// The longest text an announcement carries. A long paragraph is announced
/// by its start, not read whole each time the caret enters it.
const TEXT_LIMIT: usize = 200;

impl ProseUiState {
    /// The block that holds the caret, and what a screen reader hears of it.
    /// `None` without a caret, or while no layout carries its block yet.
    #[must_use]
    pub fn caret_announcement(&self) -> Option<(BlockId, String)> {
        let block = self.cursor?.block_id;
        let laid = self.block_layout(block).map(|b| b.block_type);
        let in_cell = laid.is_none() && self.cell_run(block).is_some();
        let text = self
            .shown_text(block)
            .or_else(|| self.run_of(block).map(|run| run.text().to_owned()));
        if laid.is_none() && !in_cell && text.is_none() {
            return None;
        }
        let kind = match laid {
            Some(BlockType::Heading) => "Heading",
            Some(BlockType::ListItem) => "List item",
            Some(BlockType::CodeBlock) => "Code",
            Some(BlockType::Image) => "Image",
            Some(BlockType::Toc) => "Table of contents",
            Some(BlockType::Table | BlockType::TableRow | BlockType::TableCell) => "Table cell",
            Some(BlockType::PageBreak | BlockType::HRule) => "Separator",
            Some(BlockType::Paragraph) | None if in_cell => "Table cell",
            Some(BlockType::Paragraph) | None => "Paragraph",
        };
        let text = text.unwrap_or_default();
        let text = text.trim();
        let said = if text.is_empty() {
            format!("{kind}, empty")
        } else {
            let start: String = text.chars().take(TEXT_LIMIT).collect();
            format!("{kind}, {start}")
        };
        Some((block, said))
    }
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use crate::events::HeadingLevel;
    use ambre_bus::app::prose::DocOffset;
    use ambre_bus::app::prose::{BlockInit, CursorPosition};
    use ambre_core::{Px, View};

    use crate::logic::ProseState;
    use crate::ui::ProseUiState;
    use crate::view::tests::append;

    /// `CHROME-023`: a page announces the block at the caret by its kind and
    /// its text. An empty block is announced as empty, and nothing is said
    /// before a caret exists.
    #[test]
    fn a_page_announces_the_block_at_the_caret() {
        let mut logic = ProseState::with_peer(ambre_core::PeerId(1)).expect("a document is built");
        logic.load_faces(&crate::layout::fonts::shipped_faces());
        let title = append(
            &mut logic,
            BlockInit::Heading {
                level: HeadingLevel::H1,
            },
            "Introduction",
        );
        let body = append(&mut logic, BlockInit::Paragraph, "");
        let mut ui = ProseUiState::default();
        ui.resize(Px::<View>::new(1200.0), Px::<View>::new(900.0));
        if let Some(event) = logic.layout_event() {
            ui.apply_event(&event);
        }
        assert_eq!(ui.caret_announcement(), None, "no caret, nothing to say");

        ui.cursor = Some(CursorPosition {
            block_id: title,
            offset: DocOffset::new(3),
            anchor: None,
        });
        assert_eq!(
            ui.caret_announcement(),
            Some((title, "Heading, Introduction".to_owned()))
        );
        ui.cursor = Some(CursorPosition {
            block_id: body,
            offset: DocOffset::ZERO,
            anchor: None,
        });
        assert_eq!(
            ui.caret_announcement(),
            Some((body, "Paragraph, empty".to_owned()))
        );
    }
}
