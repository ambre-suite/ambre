//! The showcase bundle (`toile-import-export` section 1.7).
//!
//! One HTML page that a browser draws with no Ambre, no relay and no network.
//! The page is a picture alone: the SVG export of the board, inline. It embeds
//! no board state, so a reader learns what the picture shows and nothing more
//! (`TOILE-IO-018`).
//!
//! The page is plaintext for anyone who holds it. It is the one deliberate
//! exit from the zero-knowledge boundary (`ZK-037`), and the gesture that
//! makes it says so (`TOILE-IO-096`).

use super::svg_export::SvgExport;

/// Write the showcase page around `svg`.
///
/// The page names no address a browser loads. A picture is a `data:` address
/// inside the SVG export (`TOILE-IO-087`). The content policy is the first
/// thing the head declares after the encoding. It refuses every fetch, every
/// script and every frame, so a writer bug that adds an address still loads
/// nothing (`TOILE-IO-095`).
pub fn showcase_page(svg: &SvgExport) -> ambre_core::Payload<ambre_core::FileBytes> {
    let mut page = String::with_capacity(svg.document.len() + 1024);
    page.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
    // `default-src 'none'` refuses every fetch the page could make. Only an
    // inline picture and the inline stylesheet below pass.
    page.push_str(
        "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; \
         img-src data:; style-src 'unsafe-inline'\">\n",
    );
    page.push_str("<meta name=\"referrer\" content=\"no-referrer\">\n");
    page.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    // The board's own title is a launcher concern: the Logic thread holds a
    // document, not its listing entry. The page therefore names what it is.
    page.push_str("<title>Board</title>\n");
    page.push_str(
        "<style>html,body{margin:0;height:100%;background:#fff}\
         body>svg{display:block;width:100%;height:100%}</style>\n",
    );
    page.push_str("</head>\n<body>\n");
    page.push_str(&svg.document);
    page.push_str("</body>\n</html>\n");
    ambre_core::Payload::<ambre_core::FileBytes>::new(page.into_bytes())
}
