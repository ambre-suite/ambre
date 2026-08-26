// ambre-canvas/src/text_input.rs
// ============================================================================
// The transparent `<textarea>` keyboard/IME/clipboard pipe
// (`ambre-spec-text-input.md`) and the content a11y mirror
// (`ambre-spec-accessibility.md`). The textarea renders NOTHING -- the
// canvas draws caret, selection and the IME composition string -- but it is the
// real keyboard FOCUS, so it doubles as the accessibility entry point to the
// canvas content: an accessible textbox plus a paired polite live region that
// announces the focused cell as the selection moves.
//
// On the web the accessibility tree IS the DOM, so content a11y means mirroring
// the focused content into the DOM rather than pushing a tree to a platform
// adapter. Scope is deliberately the focused unit, not a full 100k-row mirror
// (`ambre-spec-accessibility.md`).
//
// Ownership: created and positioned by the launcher's frame host under the
// canvas's positioned container; the per-app view (an `AppView` variant) drives
// WHERE it sits (caret follow), WHEN it has focus, and WHAT it announces through
// the handle below.
//
// wasm32-only: it is pure web_sys DOM glue.
// ============================================================================

use std::cell::Cell;
use std::rc::Rc;

use wasm_bindgen::JsCast;
use web_sys::{Document, Element, HtmlTextAreaElement};

/// Handle to the transparent input pipe + content a11y mirror. Cheap to clone
/// (element refs plus a shared flag); lives on the UI thread only, so `!Send`.
#[derive(Clone)]
pub struct TextInput {
    textarea: HtmlTextAreaElement,
    /// Visually-hidden `aria-live="polite"` region announcing the focused cell
    /// to screen readers as the selection moves (`ambre-spec-accessibility.md`,
    /// CHROME-023).
    live: Element,
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
        // source of truth, not the textarea buffer. NOT `aria-hidden`: this is
        // the keyboard focus and the content's a11y entry point, so it must be
        // reachable. `tabindex="0"` puts the content in the page tab order.
        for (k, v) in [
            ("autocapitalize", "off"),
            ("autocomplete", "off"),
            ("autocorrect", "off"),
            ("spellcheck", "false"),
            ("tabindex", "0"),
            ("wrap", "off"),
            ("aria-label", "Spreadsheet cell"),
            ("aria-roledescription", "spreadsheet cell"),
        ] {
            let _ = textarea.set_attribute(k, v);
        }
        parent.append_child(&textarea).expect("attach textarea");

        // Polite live region: announces the focused cell as the selection moves.
        // Visually hidden (`.ambre-a11y-live` in the app stylesheet) but exposed
        // to assistive tech.
        let live = document
            .create_element("div")
            .expect("create a11y live region");
        live.set_class_name("ambre-a11y-live");
        for (k, v) in [
            ("role", "status"),
            ("aria-live", "polite"),
            ("aria-atomic", "true"),
        ] {
            let _ = live.set_attribute(k, v);
        }
        parent.append_child(&live).expect("attach a11y live region");

        Self {
            textarea,
            live,
            composing: Rc::new(Cell::new(false)),
        }
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

    /// Announce the focused content (e.g. `"B2, 42"`) to screen readers via the
    /// polite live region. Called by the view when the selection moves or a cell
    /// edit commits (`ambre-spec-accessibility.md`). Replacing the text content
    /// of an `aria-live` region is what triggers the announcement.
    pub fn announce(&self, text: &str) {
        self.live.set_text_content(Some(text));
    }
}
