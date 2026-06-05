// ambre-canvas/src/text_input.rs
// ============================================================================
// The transparent `<textarea>` keyboard/IME/clipboard pipe (ambre-spec-chrome
// 2.C). It renders NOTHING -- the canvas draws caret, selection and the IME
// composition string. The element exists only to capture key events, drive the
// platform IME and carry the clipboard.
//
// Ownership: created and positioned by the host (host.rs) under the canvas's
// positioned container; the per-app `DocView` drives WHERE it sits (caret
// follow) and WHEN it has focus through the `TextInput` handle below. Event
// listeners are installed in host.rs (they need the `FrameHost`), funnelling
// into `InputEvent::{Text, CompositionUpdate, CompositionEnd}`.
//
// wasm32-only: it is pure web_sys DOM glue.
// ============================================================================

use std::cell::Cell;
use std::rc::Rc;

use wasm_bindgen::JsCast;
use web_sys::{Document, Element, HtmlTextAreaElement};

/// Handle to the transparent input pipe. Cheap to clone (an element ref plus a
/// shared composing flag); lives on the UI thread only, so it stays `!Send`.
#[derive(Clone)]
pub struct TextInput {
    textarea: HtmlTextAreaElement,
    /// True between `compositionstart` and `compositionend`. Shared with the
    /// host's `input` listener so it skips the half-formed value mid-composition
    /// (the canvas shows the preview from the composition events instead).
    composing: Rc<Cell<bool>>,
}

impl TextInput {
    /// Build the `<textarea class="ambre-input">` pipe and attach it under
    /// `parent` (the canvas's `position: relative` cell). Styling -- transparent
    /// text/caret/background, `pointer-events: none` -- lives in the app stylesheet
    /// so the chrome owns the look; here we only guarantee the element exists,
    /// is keyboard-reachable, and never autocorrects.
    pub fn create(document: &Document, parent: &Element) -> Self {
        let textarea: HtmlTextAreaElement = document
            .create_element("textarea")
            .expect("create textarea")
            .dyn_into()
            .expect("textarea cast");
        textarea.set_class_name("ambre-input");
        // Kill the mobile keyboard's "helpful" rewriting -- the model is the
        // source of truth, not the textarea buffer.
        for (k, v) in [
            ("autocapitalize", "off"),
            ("autocomplete", "off"),
            ("autocorrect", "off"),
            ("spellcheck", "false"),
            ("aria-hidden", "true"),
            ("tabindex", "-1"),
            ("wrap", "off"),
        ] {
            let _ = textarea.set_attribute(k, v);
        }
        parent.append_child(&textarea).expect("attach textarea");
        Self { textarea, composing: Rc::new(Cell::new(false)) }
    }

    /// The underlying element (host listeners attach here).
    pub fn element(&self) -> &HtmlTextAreaElement {
        &self.textarea
    }

    /// Shared composing flag, handed to the host's `input` listener.
    pub fn composing(&self) -> Rc<Cell<bool>> {
        Rc::clone(&self.composing)
    }

    /// Place the pipe at a caret position, in CSS pixels relative to its
    /// container, and size it to one text line. `transform` (never `top/left`)
    /// keeps it off the layout/paint path and is where the IME popup anchors.
    pub fn set_caret(&self, css_x: f64, css_y: f64, line_height: f64) {
        let style = self.textarea.style();
        let _ = style.set_property("height", &format!("{line_height}px"));
        let _ = style.set_property("transform", &format!("translate({css_x}px, {css_y}px)"));
    }

    /// Give the pipe keyboard focus so typing and IME land here. Called from the
    /// canvas pointer handler (a click in the content focuses the pipe) and when
    /// an edit session opens.
    pub fn focus(&self) {
        let _ = self.textarea.focus();
    }

    /// Empty the buffer. The host clears after each committed `Text`, and an
    /// edit session clears on open, so the textarea never accumulates state.
    pub fn clear(&self) {
        self.textarea.set_value("");
    }
}
