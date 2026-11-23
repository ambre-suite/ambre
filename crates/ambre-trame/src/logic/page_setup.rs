//! The page setup a sheet stores (`features/ambre-spec-trame-pdf-export.md
//! section 3`).
//!
//! `page_setup_{sheet}` holds one key per field. A key that is absent, or
//! that holds a value no build writes, reads as the default of its field. A
//! sheet that stores nothing therefore prints on A4, portrait, 12.7 mm on
//! each edge (`TRAME-IO-084`). A command writes the fields it changes alone,
//! so two peers that change two fields at once keep both (`TRAME-IO-086`).

use ambre_core::{Mm, PrintLength};
use loro::LoroValue;

use super::schema::{Fields, PageSetupKey, SheetContainer, SheetRoot};
use super::*;
use crate::message::{
    PageMargins, PageOrientation, PageSetup, PageSetupPatch, PaperSize, TrameRefusal,
};

/// The smallest side of the printable area: one inch. Margins that leave
/// less on either axis print nothing useful, so the setup refuses them
/// (`TRAME-IO-085`).
pub(crate) const MIN_PRINTABLE: PrintLength<Mm> = PrintLength::<Mm>::new(25.4);

/// The size of a sheet of paper, as it lies under the page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PrintPaper {
    /// The width of the sheet of paper.
    pub(crate) width: PrintLength<Mm>,
    /// The height of the sheet of paper.
    pub(crate) height: PrintLength<Mm>,
}

impl PaperSize {
    /// The paper upright: the short edge is the width.
    fn portrait(self) -> PrintPaper {
        let (width, height) = match self {
            Self::A4 => (210.0, 297.0),
            Self::A3 => (297.0, 420.0),
            Self::Letter => (215.9, 279.4),
            Self::Legal => (215.9, 355.6),
        };
        PrintPaper {
            width: PrintLength::new(width),
            height: PrintLength::new(height),
        }
    }
}

impl PageSetup {
    /// The paper the setup prints on, turned by its orientation.
    #[must_use]
    pub(crate) fn sheet(self) -> PrintPaper {
        let upright = self.paper.portrait();
        match self.orientation {
            PageOrientation::Portrait => upright,
            PageOrientation::Landscape => PrintPaper {
                width: upright.height,
                height: upright.width,
            },
        }
    }

    /// Whether each margin is a finite length of zero or more, and the
    /// margins leave at least [`MIN_PRINTABLE`] on each axis.
    #[must_use]
    pub(crate) fn is_printable(self) -> bool {
        let PageMargins {
            top,
            right,
            bottom,
            left,
        } = self.margins;
        let sound = [top, right, bottom, left]
            .into_iter()
            .all(|band| band.is_finite() && band >= PrintLength::<Mm>::ZERO);
        let paper = self.sheet();
        sound
            && paper.width - left - right >= MIN_PRINTABLE
            && paper.height - top - bottom >= MIN_PRINTABLE
    }

    /// The setup itself when it is printable, or its paper and orientation
    /// with the default margins (`TRAME-IO-085`). Each field is valid alone,
    /// so two peers that blend two fields can reach margins too wide for the
    /// paper. Every peer reads that blend back the same way.
    #[must_use]
    pub(crate) fn printable(self) -> Self {
        if self.is_printable() {
            self
        } else {
            Self {
                margins: PageMargins::NORMAL,
                ..self
            }
        }
    }
}

/// A stored margin, or `None` for a key that holds no finite length of zero
/// or more.
fn read_margin(value: Option<LoroValue>) -> Option<PrintLength<Mm>> {
    let number = match value? {
        LoroValue::Double(number) => number,
        LoroValue::I64(number) => number as f64,
        _ => return None,
    };
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a margin in millimetres is far inside the range of an f32"
    )]
    let band = PrintLength::<Mm>::new(number as f32);
    (band.is_finite() && band >= PrintLength::<Mm>::ZERO).then_some(band)
}

/// The paper whose word `value` holds, or `None` for any other value.
fn read_paper(value: Option<LoroValue>) -> Option<PaperSize> {
    let LoroValue::String(stored) = value? else {
        return None;
    };
    PaperSize::ALL
        .into_iter()
        .find(|size| size.word() == stored.as_str())
}

/// The orientation whose word `value` holds, or `None` for any other value.
fn read_orientation(value: Option<LoroValue>) -> Option<PageOrientation> {
    let LoroValue::String(stored) = value? else {
        return None;
    };
    PageOrientation::ALL
        .into_iter()
        .find(|side| side.word() == stored.as_str())
}

impl TrameState {
    /// The map that holds the page setup of `sheet`.
    fn page_setup_fields(&self, sheet: &Id<Sheet>) -> Fields<PageSetupKey> {
        Fields::new(
            self.doc
                .get_map(SheetContainer::of(SheetRoot::PageSetup, sheet).as_str()),
        )
    }

    /// The page setup `sheet` stores. An absent or unreadable field reads as
    /// its default (`TRAME-IO-084`). Margins that leave no printable area
    /// read as the default margins (`TRAME-IO-085`).
    #[must_use]
    pub(crate) fn page_setup(&self, sheet: &Id<Sheet>) -> PageSetup {
        let fields = self.page_setup_fields(sheet);
        let fallback = PageSetup::default();
        let margin = |key: PageSetupKey, default: PrintLength<Mm>| {
            read_margin(fields.value(key)).unwrap_or(default)
        };
        PageSetup {
            paper: read_paper(fields.value(PageSetupKey::Paper)).unwrap_or(fallback.paper),
            orientation: read_orientation(fields.value(PageSetupKey::Orientation))
                .unwrap_or(fallback.orientation),
            margins: PageMargins {
                top: margin(PageSetupKey::MarginTop, fallback.margins.top),
                right: margin(PageSetupKey::MarginRight, fallback.margins.right),
                bottom: margin(PageSetupKey::MarginBottom, fallback.margins.bottom),
                left: margin(PageSetupKey::MarginLeft, fallback.margins.left),
            },
        }
        .printable()
    }

    /// Apply `patch` to the page setup of `sheet_id`, the active sheet when
    /// `None` (`TRAME-IO-022`). One commit, so one undo item. Only the fields
    /// that change are written (`TRAME-IO-086`). A patch whose result leaves
    /// no printable area writes nothing and answers its refusal
    /// (`TRAME-IO-085`).
    pub(super) fn set_page_setup(
        &mut self,
        sheet_id: Option<Id<Sheet>>,
        patch: PageSetupPatch,
    ) -> TrameOutcome {
        let sheet = sheet_id.unwrap_or_else(|| self.active_sheet.clone());
        if !self.sheet_order_ids().contains(&sheet) {
            return TrameOutcome::display_only(vec![]);
        }
        let stored = self.page_setup(&sheet);
        let next = PageSetup {
            paper: patch.paper.unwrap_or(stored.paper),
            orientation: patch.orientation.unwrap_or(stored.orientation),
            margins: patch.margins.unwrap_or(stored.margins),
        };
        if !next.is_printable() {
            return TrameOutcome::display_only(vec![TrameEvent::Error {
                refusal: TrameRefusal::NoPrintableArea,
            }]);
        }
        if next == stored {
            return TrameOutcome::display_only(vec![]);
        }
        // A Loro map refuses a write only on a detached handler. A root map
        // is never detached, so the result is dropped, as the locale write does.
        let fields = self.page_setup_fields(&sheet);
        if next.paper != stored.paper {
            let _ = fields.insert(PageSetupKey::Paper, next.paper.word());
        }
        if next.orientation != stored.orientation {
            let _ = fields.insert(PageSetupKey::Orientation, next.orientation.word());
        }
        let bands = [
            (
                PageSetupKey::MarginTop,
                next.margins.top,
                stored.margins.top,
            ),
            (
                PageSetupKey::MarginRight,
                next.margins.right,
                stored.margins.right,
            ),
            (
                PageSetupKey::MarginBottom,
                next.margins.bottom,
                stored.margins.bottom,
            ),
            (
                PageSetupKey::MarginLeft,
                next.margins.left,
                stored.margins.left,
            ),
        ];
        for (key, band, before) in bands {
            if band != before {
                let _ = fields.insert(key, f64::from(band.get()));
            }
        }
        self.doc.commit();
        let update = self.flush_delta();
        // The sheet list carries the setup of the active sheet to the print
        // group of the panel.
        let display = if sheet == self.active_sheet {
            vec![self.sheet_list_event()]
        } else {
            vec![]
        };
        TrameOutcome {
            display,
            update,
            ephemeral: None,
        }
    }
}
