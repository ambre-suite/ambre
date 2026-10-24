// ambre-app/src/host/web_embed.rs
// ============================================================================
// The sandboxed frames behind a board's web `embed`s (`toile-embeds`,
// `TOILE-088`).
//
// A web page is DOM. The board is WebGPU inside one `<canvas>`, which has no way
// to hold a page. So the board paints a placeholder box and publishes where the
// page goes (`ToileView::visible_web_embeds`). This module reconciles one
// `<iframe>` per visible embed into a layer positioned over the canvas.
//
// Two consequences the corpus should not discover later:
//
//   - **The stacking order is the DOM's.** An iframe sits over the canvas, so a
//     shape the board draws ON TOP of an embed still paints beneath its page.
//     The board has one paint order and the document has another, and they do
//     not interleave. Irreducible short of painting the whole board in DOM.
//   - **The clip is the viewport's, not the embed's.** A CSS `clip-path` is
//     resolved in the element's own box before its transform, so it cannot
//     express a screen-space rectangle. The layer clips instead, which bounds
//     every frame to the visible board. An embed sitting inside a board frame
//     that should crop it is not cropped ("Clip limitation").
// ============================================================================

use std::collections::HashMap;

use ambre_bus::app::toile::{ElementId, WebAddress};
use ambre_toile::ui::embed::VisibleWebEmbed;
use wasm_bindgen::JsCast;

/// What an embedded page is allowed to do (`TOILE-088`).
///
/// **`allow-same-origin` is absent, and its absence is the rule.** With it, a
/// page served from this very origin could reach `parent.document` and take its
/// own sandbox off. Embedding Ambre's own address would be exactly that.
/// Without it every frame runs in an opaque origin. There is no shared storage,
/// no access to the board, nothing to read from the document that hosts it. The
/// price is that a page needing its own storage degrades, and that is the trade
/// `TOILE-088` names.
const SANDBOX: &str = "allow-scripts allow-forms allow-popups";

/// The Permissions-Policy the frame inherits: nothing. A board is not a reason
/// for a third-party page to ask for a camera, a microphone or a location.
const PERMISSIONS: &str = "";

/// The live `<iframe>` behind each visible web embed, keyed by the board element
/// that carries the address. One frame per embed, like one child view per embed
/// (`TOILE-UI-097`).
#[derive(Default)]
pub struct WebFrames {
    frames: HashMap<ElementId, web_sys::HtmlIFrameElement>,
    /// The address each frame was last pointed at, so a board redraw does not
    /// reload a page that has not changed. Reloading on every frame would make
    /// an embedded page unusable.
    urls: HashMap<ElementId, WebAddress>,
}

impl WebFrames {
    /// Reconcile the mounted frames against the culled set: mount what entered,
    /// move what stayed, drop what left. `layer` is the positioned container the
    /// launcher put over the canvas.
    pub fn sync(&mut self, layer: &web_sys::HtmlElement, embeds: &[VisibleWebEmbed]) {
        // Drop what is no longer drawn, taking its page with it.
        let live: std::collections::HashSet<&ElementId> =
            embeds.iter().map(|e| &e.element).collect();
        self.frames.retain(|id, frame| {
            let keep = live.contains(id);
            if !keep {
                let _ = layer.remove_child(frame);
            }
            keep
        });
        self.urls.retain(|id, _| live.contains(id));

        for e in embeds {
            let frame = match self.frames.get(&e.element) {
                Some(f) => f.clone(),
                None => {
                    let Some(f) = mount(layer) else { continue };
                    self.frames.insert(e.element.clone(), f.clone());
                    f
                }
            };
            // Point it somewhere only when the address actually changed: `src`
            // is a navigation, and a per-frame navigation is a page that never
            // finishes loading.
            if self.urls.get(&e.element) != Some(&e.url) {
                frame.set_src(e.url.as_str());
                self.urls.insert(e.element.clone(), e.url.clone());
            }
            // Until the embed is entered the frame lets the pointer through, so
            // the board's own move and resize stay reachable. Entering it hands
            // the page its events directly (`TOILE-UI-091`, `TOILE-UI-324`).
            let input = if e.focused {
                PointerInput::Taken
            } else {
                PointerInput::FallsThrough
            };
            set_pointer_input(&frame, input);
            place(&frame, e);
        }
    }
}

/// Build one sandboxed frame and attach it to `layer`. `None` if the document is
/// gone from under us, which the caller treats as "no frame this pass".
fn mount(layer: &web_sys::HtmlElement) -> Option<web_sys::HtmlIFrameElement> {
    let doc = web_sys::window()?.document()?;
    let frame: web_sys::HtmlIFrameElement = doc.create_element("iframe").ok()?.dyn_into().ok()?;
    frame.set_attribute("sandbox", SANDBOX).ok()?;
    frame.set_attribute("allow", PERMISSIONS).ok()?;
    // The board's own address is not a third party's business.
    frame.set_attribute("referrerpolicy", "no-referrer").ok()?;
    let style = frame.style();
    let _ = style.set_property("position", "absolute");
    let _ = style.set_property("border", "0");
    // The page is laid out at the element's world size and moved by a matrix, so
    // the transform must scale about the frame's own corner.
    let _ = style.set_property("transform-origin", "0 0");
    set_pointer_input(&frame, PointerInput::FallsThrough);
    layer.append_child(&frame).ok()?;
    Some(frame)
}

/// What a frame does with a pointer event over it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PointerInput {
    /// The frame takes it: the page inside is focused.
    Taken,
    /// It falls through to the canvas beneath.
    FallsThrough,
}

/// Let this frame take pointer events, or let them fall through to the
/// canvas beneath it.
fn set_pointer_input(frame: &web_sys::HtmlIFrameElement, input: PointerInput) {
    let value = match input {
        PointerInput::Taken => "auto",
        PointerInput::FallsThrough => "none",
    };
    let _ = frame.style().set_property("pointer-events", value);
}

/// Size and place one frame. The size is the element's world size, in CSS
/// pixels, so the page is laid out once. The camera lives entirely in the
/// transform, so a zoom moves the frame instead of reflowing the page.
fn place(frame: &web_sys::HtmlIFrameElement, e: &VisibleWebEmbed) {
    let style = frame.style();
    let _ = style.set_property("width", &format!("{}px", e.layout_size.width));
    let _ = style.set_property("height", &format!("{}px", e.layout_size.height));
    // The board's screen space is logical, which is CSS pixels: the matrix
    // carries over as is. Its linear part holds the zoom and the rotation.
    let c = e.to_screen.as_coeffs();
    let _ = style.set_property(
        "transform",
        &format!(
            "matrix({}, {}, {}, {}, {}, {})",
            c[0], c[1], c[2], c[3], c[4], c[5]
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::WebFrames;
    use ambre_bus::app::toile::{ElementId, WebAddress};
    use ambre_draw::{Affine, Rect, Size};
    use ambre_toile::ui::embed::VisibleWebEmbed;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::wasm_bindgen_test;

    /// `TOILE-088`: the frame behind a web embed runs in an opaque origin,
    /// with no permission and no referrer. Without `allow-same-origin` the page
    /// reaches neither the board, nor its storage, nor the keys it holds.
    #[wasm_bindgen_test]
    fn a_web_embed_runs_sandboxed_in_an_opaque_origin() {
        let document = web_sys::window()
            .and_then(|w| w.document())
            .expect("a page");
        let layer: web_sys::HtmlElement = document
            .create_element("div")
            .expect("a div")
            .dyn_into()
            .expect("an element");
        let embed = VisibleWebEmbed {
            element: ElementId("page".to_owned()),
            // `.invalid` never resolves, so the test reaches no network.
            url: WebAddress::parse("https://example.invalid").expect("a safe address"),
            screen_rect: Rect::new(0.0, 0.0, 200.0, 100.0),
            layout_size: Size::new(200.0, 100.0),
            to_screen: Affine::IDENTITY,
            focused: false,
        };
        let mut frames = WebFrames::default();
        frames.sync(&layer, std::slice::from_ref(&embed));
        assert_eq!(layer.child_element_count(), 1, "one frame per embed");
        let frame = layer.first_element_child().expect("the frame");
        let sandbox = frame.get_attribute("sandbox").expect("a sandbox");
        let tokens: Vec<&str> = sandbox.split_whitespace().collect();
        assert!(tokens.contains(&"allow-scripts"), "{sandbox}");
        assert!(
            !tokens.contains(&"allow-same-origin"),
            "the page shares the board's origin"
        );
        assert!(
            !tokens.contains(&"allow-top-navigation"),
            "the page can navigate the board"
        );
        assert_eq!(frame.get_attribute("allow").as_deref(), Some(""));
        assert_eq!(
            frame.get_attribute("referrerpolicy").as_deref(),
            Some("no-referrer")
        );
        frames.sync(&layer, &[]);
        assert_eq!(layer.child_element_count(), 0);
    }
}
