//! The page box, and the two bands cut out of it
//! (`features/ambre-spec-prose-pagination.md section 2`).
//!
//! Everything here is derived from `meta/page` on every read. No page number,
//! no break position and no measured height is stored, so no reader can find a
//! stale one (`PROSE-033`, `PROSE-065`).

use crate::logic::Roots;
use ambre_core::{Doc, Px};
use loro::LoroDoc;

use crate::doc;

/// The share of the content height either band may take (`PROSE-016`).
pub(crate) const BAND_SHARE: f32 = 0.25;

/// The page a document is laid out on, in pixels.
///
/// The content box is what the four margins leave. Placement measures against
/// it and never against the sheet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageBox {
    pub page_width: Px<Doc>,
    pub page_height: Px<Doc>,
    pub content_width: Px<Doc>,
    pub content_height: Px<Doc>,
    /// Where the content box starts on the sheet.
    pub margin_left: Px<Doc>,
    pub margin_top: Px<Doc>,
    /// The format the box was read from, in millimetres, which the context
    /// panel shows (`PROSE-UI-253`).
    pub format: doc::PageFormat,
}

impl Default for PageBox {
    /// The page a fresh document opens on: A4 portrait with its four margins.
    fn default() -> Self {
        Self::from_mm(doc::PageFormat::default())
    }
}

impl PageBox {
    /// The page box `meta/page` describes, or the A4 default when the register
    /// is missing or holds a dimension no page can have.
    ///
    /// A document that lost its page register still lays out. The alternative
    /// is a document that opens and cannot be read, which is worse than a
    /// document read on the wrong sheet.
    #[must_use]
    pub fn read(document: &LoroDoc, roots: &Roots) -> Self {
        let format = doc::read_page_format(document, roots);
        Self::from_mm(format)
    }

    /// The pixel box a millimetre format converts to at 96 DPI.
    #[must_use]
    pub fn from_mm(format: doc::PageFormat) -> Self {
        let page_width = format.width.to_doc_px();
        let page_height = format.height.to_doc_px();
        let horizontal = (format.margin_left + format.margin_right).to_doc_px();
        let vertical = (format.margin_top + format.margin_bottom).to_doc_px();
        Self {
            page_width,
            page_height,
            content_width: (page_width - horizontal).max(Px::<Doc>::ZERO),
            content_height: (page_height - vertical).max(Px::<Doc>::ZERO),
            margin_left: format.margin_left.to_doc_px(),
            margin_top: format.margin_top.to_doc_px(),
            format,
        }
    }

    /// The height a header or a footer may take on this page.
    ///
    /// A band measured taller is clipped to this, never allowed to grow the
    /// page (`PROSE-016`). Growing it would contradict the declared format. And
    /// a band as tall as the page leaves placement no height to put a line in,
    /// which is the loop `PROSE-017` forbids.
    #[must_use]
    pub fn band_cap(&self) -> Px<Doc> {
        self.content_height * BAND_SHARE
    }

    /// The height left for the body once both bands are cut out.
    ///
    /// Each band is capped first, so the answer is at least half the content
    /// height whatever the bands measured.
    #[must_use]
    pub fn body_height(&self, header_height: Px<Doc>, footer_height: Px<Doc>) -> Px<Doc> {
        let cap = self.band_cap();
        (self.content_height - header_height.min(cap) - footer_height.min(cap)).max(Px::<Doc>::ZERO)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_core::{Mm, PrintLength};

    /// A4 portrait at 96 DPI, the format `seed` writes.
    fn a4() -> PageBox {
        PageBox::from_mm(doc::PageFormat::default())
    }

    #[test]
    fn the_content_box_is_what_the_four_margins_leave() {
        let page = a4();
        let margins = PrintLength::<Mm>::new(40.0).to_doc_px();
        assert!(
            (page.content_width - (page.page_width - margins))
                .abs()
                .get()
                < 0.01
        );
        assert!(
            (page.content_height - (page.page_height - margins))
                .abs()
                .get()
                < 0.01
        );
    }

    #[test]
    fn a_band_taller_than_a_quarter_is_clipped_and_never_grows_the_page() {
        let page = a4();
        let cap = page.band_cap();
        let tall = page.content_height * 4.0;
        let body = page.body_height(tall, tall);
        assert!((body - (page.content_height - cap * 2.0)).abs().get() < 0.01);
        assert!(body >= page.content_height / 2.0);
    }

    #[test]
    fn a_margin_wider_than_the_sheet_leaves_a_content_box_of_zero() {
        let page = PageBox::from_mm(doc::PageFormat {
            margin_left: PrintLength::<Mm>::new(500.0),
            margin_top: PrintLength::<Mm>::new(500.0),
            ..doc::PageFormat::default()
        });
        assert_eq!(page.content_width, Px::<Doc>::ZERO);
        assert_eq!(page.content_height, Px::<Doc>::ZERO);
        assert_eq!(
            page.body_height(Px::<Doc>::ZERO, Px::<Doc>::ZERO),
            Px::<Doc>::ZERO
        );
    }
}
