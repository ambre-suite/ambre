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
//   - **No page loads before its user asks** (`ZK-048`). A frame
//     that loads tells the site who watches the board, and when. So an embed
//     whose site holds no choice shows a prompt instead of a frame, and the
//     choice stays in this browser's storage alone.
// ============================================================================

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use ambre_core::Validated;
use ambre_core::id::{Element, Id};
use ambre_toile::message::WebAddress;
use ambre_toile::ui::embed::{
    FrameLoad, LoadOnce, SiteChoice, SiteHost, VisibleWebEmbed, frame_load, site_of,
};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

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
    frames: HashMap<Id<Element>, web_sys::HtmlIFrameElement>,
    /// The address each frame was last pointed at, so a board redraw does not
    /// reload a page that has not changed. Reloading on every frame would make
    /// an embedded page unusable.
    urls: HashMap<Id<Element>, Validated<WebAddress>>,
    /// The prompt over each visible embed that loads no page.
    prompts: HashMap<Id<Element>, Prompt>,
    /// The embeds whose page the user loaded once, for this session.
    once: HashSet<Id<Element>>,
    /// The choice of each site met, read once from the device store.
    choices: HashMap<Validated<SiteHost>, SiteChoice>,
    /// The answers the prompt buttons gave since the last pass. A click lands
    /// outside the pass, so the button queues it and the pass applies it.
    answers: Rc<RefCell<Vec<Answer>>>,
}

/// One prompt over a placeholder: the node, what it shows, and the click
/// handlers its buttons hold, dropped with it.
struct Prompt {
    node: web_sys::HtmlElement,
    shown: FrameLoad,
    site: Validated<SiteHost>,
    _clicks: Vec<Closure<dyn FnMut()>>,
}

/// A button of a prompt, pressed.
#[derive(Clone)]
struct Answer {
    element: Id<Element>,
    site: Validated<SiteHost>,
    pick: Pick,
}

/// The four answers a prompt offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pick {
    /// Load this embed's page, for this session alone.
    LoadOnce,
    /// Load every page of the site, from now on.
    Always,
    /// Load no page of the site, and ask no more.
    Never,
    /// Forget the refusal of the site, so its embeds ask again.
    AskAgain,
}

impl WebFrames {
    /// Reconcile the mounted frames against the culled set: mount what entered,
    /// move what stayed, drop what left. `layer` is the positioned container the
    /// launcher put over the canvas.
    pub fn sync(&mut self, layer: &web_sys::HtmlElement, embeds: &[VisibleWebEmbed]) {
        self.apply_answers();
        // Drop what is no longer drawn, taking its page with it.
        let live: HashSet<&Id<Element>> = embeds.iter().map(|e| &e.element).collect();
        self.frames.retain(|id, frame| {
            let keep = live.contains(id);
            if !keep {
                let _ = layer.remove_child(frame);
            }
            keep
        });
        self.urls.retain(|id, _| live.contains(id));
        self.prompts.retain(|id, prompt| {
            let keep = live.contains(id);
            if !keep {
                let _ = layer.remove_child(&prompt.node);
            }
            keep
        });

        for e in embeds {
            // An address with no site never passed the rule of a web address,
            // so it gets neither a frame nor a prompt.
            let Some(site) = site_of(&e.url) else {
                continue;
            };
            let once = if self.once.contains(&e.element) {
                LoadOnce::Given
            } else {
                LoadOnce::NotGiven
            };
            let decision = frame_load(self.choice(&site), once);
            if decision != FrameLoad::Load {
                self.hold(layer, e, site, decision);
                continue;
            }
            if let Some(prompt) = self.prompts.remove(&e.element) {
                let _ = layer.remove_child(&prompt.node);
            }
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
            let input = if e.focused == ambre_toile::message::InputFocus::Entered {
                PointerInput::Taken
            } else {
                PointerInput::FallsThrough
            };
            set_pointer_input(&frame, input);
            place(&frame, e);
        }
    }

    /// Apply the answers the prompt buttons queued: a load once marks its
    /// embed, and a site choice goes to the device store.
    fn apply_answers(&mut self) {
        let answers: Vec<Answer> = self.answers.borrow_mut().drain(..).collect();
        for answer in answers {
            let choice = match answer.pick {
                Pick::LoadOnce => {
                    self.once.insert(answer.element);
                    continue;
                }
                Pick::Always => SiteChoice::Always,
                Pick::Never => SiteChoice::Never,
                Pick::AskAgain => SiteChoice::Ask,
            };
            store_choice(&answer.site, choice);
            self.choices.insert(answer.site, choice);
        }
    }

    /// The choice `site` holds, read from the device store the first time
    /// the site is met.
    fn choice(&mut self, site: &Validated<SiteHost>) -> SiteChoice {
        *self
            .choices
            .entry(site.clone())
            .or_insert_with(|| read_choice(site))
    }

    /// Show the prompt `decision` asks for over `e`, and no frame. A frame
    /// mounted before goes, with its page: a site refused since then loads
    /// nothing more.
    fn hold(
        &mut self,
        layer: &web_sys::HtmlElement,
        e: &VisibleWebEmbed,
        site: Validated<SiteHost>,
        decision: FrameLoad,
    ) {
        if let Some(frame) = self.frames.remove(&e.element) {
            let _ = layer.remove_child(&frame);
        }
        self.urls.remove(&e.element);
        let current = self
            .prompts
            .get(&e.element)
            .is_some_and(|prompt| prompt.shown == decision && prompt.site == site);
        if !current {
            if let Some(old) = self.prompts.remove(&e.element) {
                let _ = layer.remove_child(&old.node);
            }
            let Some(prompt) = mount_prompt(layer, &e.element, site, decision, &self.answers)
            else {
                return;
            };
            self.prompts.insert(e.element.clone(), prompt);
        }
        if let Some(prompt) = self.prompts.get(&e.element) {
            place_prompt(&prompt.node, e);
        }
    }
}

/// The choice the device store holds for `site`. A store the browser
/// refuses, or a word this build does not know, reads as asking.
fn read_choice(site: &Validated<SiteHost>) -> SiteChoice {
    let stored = web_sys::window()
        .and_then(|window| window.local_storage().ok().flatten())
        .and_then(|storage| {
            storage
                .get_item(&format!("ambre.embed-site.{}", site.as_str()))
                .ok()
                .flatten()
        });
    SiteChoice::from_stored(stored.as_deref())
}

/// Keep `choice` for `site` in this browser alone. Asking
/// again removes the entry. A store that refuses keeps nothing, and the
/// site asks again at the next session.
fn store_choice(site: &Validated<SiteHost>, choice: SiteChoice) {
    let Some(storage) = web_sys::window().and_then(|window| window.local_storage().ok().flatten())
    else {
        return;
    };
    let key = format!("ambre.embed-site.{}", site.as_str());
    let _ = match choice.stored() {
        Some(word) => storage.set_item(&key, word),
        None => storage.remove_item(&key),
    };
}

/// Build the prompt over one placeholder: a line that names the site, and
/// one button per answer. `None` if the
/// document is gone, which the caller treats as "no prompt this pass".
fn mount_prompt(
    layer: &web_sys::HtmlElement,
    element: &Id<Element>,
    site: Validated<SiteHost>,
    decision: FrameLoad,
    answers: &Rc<RefCell<Vec<Answer>>>,
) -> Option<Prompt> {
    let doc = web_sys::window()?.document()?;
    let node: web_sys::HtmlElement = doc.create_element("div").ok()?.dyn_into().ok()?;
    node.set_class_name("ambre-web-embed-ask");
    let (line, picks): (String, &[(Pick, &str)]) = match decision {
        FrameLoad::Refused => (
            format!("Pages from {} do not load on this device.", site.as_str()),
            &[(Pick::AskAgain, "Ask again")][..],
        ),
        FrameLoad::Ask | FrameLoad::Load => (
            format!(
                "This page loads from {}. The site then learns that you view it.",
                site.as_str()
            ),
            &[
                (Pick::LoadOnce, "Load once"),
                (Pick::Always, "Always for this site"),
                (Pick::Never, "Never for this site"),
            ][..],
        ),
    };
    let text = doc.create_element("p").ok()?;
    text.set_text_content(Some(&line));
    node.append_child(&text).ok()?;
    let mut clicks = Vec::with_capacity(picks.len());
    for &(pick, label) in picks {
        let button: web_sys::HtmlElement = doc.create_element("button").ok()?.dyn_into().ok()?;
        button.set_attribute("type", "button").ok()?;
        button.set_text_content(Some(label));
        // The prompt lets the pointer through, and a child inherits that.
        let _ = button.style().set_property("pointer-events", "auto");
        let queue = Rc::clone(answers);
        let answer = Answer {
            element: element.clone(),
            site: site.clone(),
            pick,
        };
        let click = Closure::<dyn FnMut()>::new(move || {
            queue.borrow_mut().push(answer.clone());
        });
        button.set_onclick(Some(click.as_ref().unchecked_ref()));
        node.append_child(&button).ok()?;
        clicks.push(click);
    }
    let style = node.style();
    let _ = style.set_property("position", "absolute");
    // The prompt lets the pointer through, so the board's move and resize of
    // the embed stay reachable. Its buttons alone take a click.
    let _ = style.set_property("pointer-events", "none");
    layer.append_child(&node).ok()?;
    Some(Prompt {
        node,
        shown: decision,
        site,
        _clicks: clicks,
    })
}

/// Lay the prompt over the screen box of its placeholder.
fn place_prompt(node: &web_sys::HtmlElement, e: &VisibleWebEmbed) {
    let style = node.style();
    let r = e.screen_rect;
    let _ = style.set_property("left", &format!("{}px", r.x0));
    let _ = style.set_property("top", &format!("{}px", r.y0));
    let _ = style.set_property("width", &format!("{}px", r.width()));
    let _ = style.set_property("height", &format!("{}px", r.height()));
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
    use super::{Answer, Pick, WebFrames};
    use ambre_core::Validated;
    use ambre_core::id::{Element, Id};
    use ambre_draw::{Affine, Rect, Size};
    use ambre_toile::message::WebAddress;
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
            element: Id::<Element>::from_text("page".to_owned()),
            // `.invalid` never resolves, so the test reaches no network.
            url: Validated::<WebAddress>::parse("https://example.invalid").expect("a safe address"),
            screen_rect: Rect::new(0.0, 0.0, 200.0, 100.0),
            layout_size: Size::new(200.0, 100.0),
            to_screen: Affine::IDENTITY,
            focused: ambre_toile::message::InputFocus::Board,
        };
        let mut frames = WebFrames::default();
        // The site holds a choice on this pass alone: the store stays clean.
        if let Some(site) = ambre_toile::ui::embed::site_of(&embed.url) {
            frames
                .choices
                .insert(site, ambre_toile::ui::embed::SiteChoice::Always);
        }
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

    /// A layer and one web embed on it, of the address `url`.
    fn layer_and_embed(url: &str, element: &str) -> (web_sys::HtmlElement, VisibleWebEmbed) {
        let document = web_sys::window()
            .and_then(|w| w.document())
            .expect("a page");
        let layer: web_sys::HtmlElement = document
            .create_element("div")
            .expect("a div")
            .dyn_into()
            .expect("an element");
        let embed = VisibleWebEmbed {
            element: Id::<Element>::from_text(element.to_owned()),
            url: Validated::<WebAddress>::parse(url).expect("a safe address"),
            screen_rect: Rect::new(0.0, 0.0, 200.0, 100.0),
            layout_size: Size::new(200.0, 100.0),
            to_screen: Affine::IDENTITY,
            focused: ambre_toile::message::InputFocus::Board,
        };
        (layer, embed)
    }

    /// Whether `layer` holds a frame pointed at a page.
    fn loads_a_page(layer: &web_sys::HtmlElement) -> bool {
        layer.query_selector("iframe[src]").ok().flatten().is_some()
    }

    /// Queue the answer `pick` for `embed`, as its prompt button does.
    fn press(frames: &WebFrames, embed: &VisibleWebEmbed, pick: Pick) {
        let site = ambre_toile::ui::embed::site_of(&embed.url).expect("a site");
        frames.answers.borrow_mut().push(Answer {
            element: embed.element.clone(),
            site,
            pick,
        });
    }

    /// `ZK-048`: an embed whose site holds no choice
    /// mounts no frame and points nothing at the page. It shows a prompt with
    /// three answers. Once the user picks "Always", the frame mounts and
    /// loads, and the choice is in the store of this browser alone.
    #[wasm_bindgen_test]
    fn no_page_loads_before_its_site_is_chosen() {
        let (layer, embed) = layer_and_embed("https://consent-always.invalid/a", "always");
        let mut frames = WebFrames::default();
        frames.sync(&layer, std::slice::from_ref(&embed));
        assert!(!loads_a_page(&layer), "a page loaded before any answer");
        let buttons = layer
            .query_selector_all(".ambre-web-embed-ask button")
            .expect("a query");
        assert_eq!(buttons.length(), 3, "load once, always, never");
        // A second pass, as the next frame makes, still loads nothing.
        frames.sync(&layer, std::slice::from_ref(&embed));
        assert!(!loads_a_page(&layer));

        press(&frames, &embed, Pick::Always);
        frames.sync(&layer, std::slice::from_ref(&embed));
        assert!(loads_a_page(&layer), "the chosen site loads its page");
        assert!(
            layer
                .query_selector(".ambre-web-embed-ask")
                .ok()
                .flatten()
                .is_none(),
            "the prompt left with the answer"
        );
        let stored = web_sys::window()
            .and_then(|w| w.local_storage().ok().flatten())
            .and_then(|s| {
                s.get_item("ambre.embed-site.consent-always.invalid")
                    .ok()
                    .flatten()
            });
        assert_eq!(stored.as_deref(), Some("always"));

        // A fresh session reads the choice back and loads at once.
        let mut later = WebFrames::default();
        let (other_layer, other) = layer_and_embed("https://consent-always.invalid/b", "other");
        later.sync(&other_layer, std::slice::from_ref(&other));
        assert!(loads_a_page(&other_layer), "the choice holds for the site");

        // Leave the store as the test found it.
        press(&later, &other, Pick::AskAgain);
        later.sync(&other_layer, std::slice::from_ref(&other));
        assert!(!loads_a_page(&other_layer), "asking again unloads the page");
    }

    /// "Load once" loads this embed alone, and keeps nothing in
    /// the store. Another embed of the same site still asks.
    #[wasm_bindgen_test]
    fn a_page_loaded_once_leaves_the_site_asking() {
        let (layer, embed) = layer_and_embed("https://consent-once.invalid", "once");
        let (_, twin) = layer_and_embed("https://consent-once.invalid/twin", "twin");
        let both = [embed.clone(), twin];
        let mut frames = WebFrames::default();
        frames.sync(&layer, &both);
        press(&frames, &embed, Pick::LoadOnce);
        frames.sync(&layer, &both);
        let loaded = layer.query_selector_all("iframe[src]").expect("a query");
        assert_eq!(loaded.length(), 1, "the answered embed alone loads");
        assert_eq!(
            layer
                .query_selector_all(".ambre-web-embed-ask")
                .expect("a query")
                .length(),
            1,
            "the other embed still asks"
        );
        let stored = web_sys::window()
            .and_then(|w| w.local_storage().ok().flatten())
            .and_then(|s| {
                s.get_item("ambre.embed-site.consent-once.invalid")
                    .ok()
                    .flatten()
            });
        assert_eq!(stored, None, "a load once is not remembered");
    }

    /// A refused site loads nothing and says so, with one way
    /// back. Asking again brings the three answers back.
    #[wasm_bindgen_test]
    fn a_refused_site_loads_nothing_until_asked_again() {
        let (layer, embed) = layer_and_embed("https://consent-never.invalid", "never");
        let mut frames = WebFrames::default();
        frames.sync(&layer, std::slice::from_ref(&embed));
        press(&frames, &embed, Pick::Never);
        frames.sync(&layer, std::slice::from_ref(&embed));
        assert!(!loads_a_page(&layer));
        let buttons = layer
            .query_selector_all(".ambre-web-embed-ask button")
            .expect("a query");
        assert_eq!(buttons.length(), 1, "ask again, and nothing else");
        press(&frames, &embed, Pick::AskAgain);
        frames.sync(&layer, std::slice::from_ref(&embed));
        assert!(!loads_a_page(&layer));
        let buttons = layer
            .query_selector_all(".ambre-web-embed-ask button")
            .expect("a query");
        assert_eq!(buttons.length(), 3);
    }
}
