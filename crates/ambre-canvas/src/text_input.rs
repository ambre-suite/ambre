// ambre-canvas/src/text_input.rs
// ============================================================================
// The transparent `<textarea>` keyboard/IME/clipboard pipe
// (`ambre-spec-text-input.md`) and the content a11y mirror
// (`ambre-spec-accessibility.md`). The textarea renders NOTHING: the canvas
// draws caret, selection and the IME composition string. But it is the real
// keyboard FOCUS, so it doubles as the accessibility entry point to the canvas
// content. That means an accessible textbox plus a paired polite live region
// that announces the focused unit as the selection moves. Each application
// names the box after its own kind of document, and writes its own unit.
//
// On the web the accessibility tree IS the DOM. Content a11y therefore means
// mirroring the focused content into the DOM rather than pushing a tree to a
// platform adapter. Scope is deliberately the focused unit, not a full 100k-row
// mirror (`ambre-spec-accessibility.md`).
//
// Ownership: created and positioned by the launcher's frame host under the
// canvas's positioned container. The per-app view (an `AppView` variant) drives
// WHERE it sits (caret follow), WHEN it has focus, and WHAT it announces through
// the handle below.
//
// DOM glue on wasm32. A host test builds a detached pipe instead, so a view
// that owns one can be driven without a browser.
// ============================================================================

use std::cell::Cell;
use std::rc::Rc;

use ambre_core::{Point, Px, View};

use wasm_bindgen::JsCast;
use web_sys::{Document, Element, HtmlTextAreaElement};

/// Whether a view stands alone or sits in another document. The standalone
/// view takes the text pipe on construction; an embedded one leaves it to its
/// host until the user clicks in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Embedding {
    /// A top-level view: it owns the pipe from the start.
    #[default]
    Standalone,
    /// A `doc_embed` child: the document around it owns the pipe.
    Embedded,
}

/// The kind of document the content holds. It names the pipe to assistive
/// technology, so each application describes its own content (`CHROME-024`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentKind {
    /// A Trame grid: the pipe stands for the active cell.
    Spreadsheet,
    /// A Toile board: the pipe stands for the selection.
    Board,
    /// A Prose page: the pipe stands for the text at the caret.
    Page,
    /// A Murmure conversation: the pipe stands for the focused message.
    Conversation,
}

impl ContentKind {
    /// The accessible name of the pipe.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Spreadsheet => "Spreadsheet cell",
            Self::Board => "Whiteboard",
            Self::Page => "Document text",
            Self::Conversation => "Messages",
        }
    }

    /// What a screen reader says the pipe is, in place of a text box.
    #[must_use]
    pub fn role_description(self) -> &'static str {
        match self {
            Self::Spreadsheet => "spreadsheet cell",
            Self::Board => "whiteboard",
            Self::Page => "document",
            Self::Conversation => "conversation",
        }
    }
}

ambre_core::define_flag! {
    /// Whether an IME composition is under way in the pipe: between
    /// `compositionstart` and `compositionend`.
    pub enum Composition {
        /// No composition: the `input` listener reads the value.
        Idle,
        /// A composition is under way: the value is half-formed.
        Composing,
    }
}

/// Handle to the transparent input pipe + content a11y mirror. Cheap to clone
/// (element refs plus a shared flag); lives on the UI thread only, so `!Send`.
#[derive(Clone)]
pub struct TextInput {
    /// The DOM half. `None` for a detached pipe, which a host test builds:
    /// every call that would touch the DOM is then a no-op.
    dom: Option<Pipe>,
    /// `Composing` between `compositionstart` and `compositionend`. Shared
    /// with the host's `input` listener so it skips the half-formed value
    /// mid-composition (the canvas shows the preview from the composition
    /// events instead).
    composing: Rc<Cell<Composition>>,
    /// How many times a view asked for the focus. A host test reads it to state
    /// that a gesture left the focus where it was.
    focus_requests: Rc<Cell<u32>>,
    /// Whether a key on the pipe is typed text. Mirrored on the element as
    /// [`TYPING_ATTR`], which the keymap listener reads off the key's target.
    typing: Rc<Cell<Typing>>,
}

/// The attribute that says a key on the pipe is not typed text, when it reads
/// `false`. A pipe without it is typed text.
/// What a key on the pipe is (`CHROME-103`): a command a keymap reads, or
/// typed text an editor takes. A pipe is typed text by default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Typing {
    /// A bare key reaches the keymap: a board with no editor open.
    Commands,
    /// A key is typed text, as in a page, a cell or an open editor.
    #[default]
    Text,
}

impl Typing {
    /// `Text` when `typing` holds, `Commands` otherwise.
    pub fn text_if(typing: bool) -> Self {
        if typing {
            Typing::Text
        } else {
            Typing::Commands
        }
    }
}

pub const TYPING_ATTR: &str = "data-typing";

#[derive(Clone)]
struct Pipe {
    textarea: HtmlTextAreaElement,
    /// Visually-hidden `aria-live="polite"` region announcing the focused cell
    /// to screen readers as the selection moves (`ambre-spec-accessibility.md`,
    /// CHROME-023).
    live: Element,
}

impl TextInput {
    /// A pipe with no DOM behind it, so a view can be built and driven by a
    /// host test. Focus, clear, naming and announcing do nothing.
    pub fn detached() -> Self {
        Self {
            dom: None,
            composing: Rc::new(Cell::new(Composition::Idle)),
            focus_requests: Rc::new(Cell::new(0)),
            typing: Rc::new(Cell::new(Typing::Text)),
        }
    }

    /// Build the `<textarea class="ambre-input">` pipe and attach it under `parent`
    /// (the canvas's `position: relative` cell). Styling -- transparent
    /// text/caret/background, `pointer-events: none` -- lives in the app
    /// stylesheet, so the chrome owns the look. Here we only guarantee the element
    /// exists, is keyboard-reachable, and never autocorrects.
    ///
    /// # Panics
    ///
    /// When the DOM refuses to create or attach the `<textarea>`. The canvas has no
    /// keyboard entry point without it, so there is nothing to degrade to.
    #[expect(
        clippy::expect_used,
        reason = "a boot without the keyboard pipe has no input path to degrade to"
    )]
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
            // The element says what the pipe is from the start, so the keymap
            // listener reads a content pipe even where no view sets it.
            (TYPING_ATTR, "true"),
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
            dom: Some(Pipe { textarea, live }),
            composing: Rc::new(Cell::new(Composition::Idle)),
            focus_requests: Rc::new(Cell::new(0)),
            typing: Rc::new(Cell::new(Typing::Text)),
        }
    }

    /// The underlying element (host listeners attach here).
    ///
    /// # Panics
    ///
    /// On a [`detached`](Self::detached) pipe, which has no element to attach to.
    #[expect(
        clippy::expect_used,
        reason = "only the host boot asks for the element, and it holds a created pipe"
    )]
    pub fn element(&self) -> &HtmlTextAreaElement {
        &self
            .dom
            .as_ref()
            .expect("a detached pipe has no element")
            .textarea
    }

    /// Shared composition state, handed to the host's `input` listener.
    pub fn composing(&self) -> Rc<Cell<Composition>> {
        Rc::clone(&self.composing)
    }

    /// Place the pipe at a caret position, in CSS pixels relative to its
    /// container, and size it to one text line. `transform` (never `top/left`)
    /// keeps it off the layout/paint path and is where the IME popup anchors.
    pub fn set_caret(&self, at: Point<Px<View>>, line_height: Px<View>) {
        let Some(dom) = &self.dom else { return };
        let style = dom.textarea.style();
        let (x, y, tall) = (at.x.get(), at.y.get(), line_height.get());
        let _ = style.set_property("height", &format!("{tall}px"));
        let _ = style.set_property("transform", &format!("translate({x}px, {y}px)"));
    }

    /// Give the pipe keyboard focus so typing and IME land here. Called from the
    /// canvas pointer handler (a click in the content focuses the pipe) and when
    /// an edit session opens.
    pub fn focus(&self) {
        self.focus_requests
            .set(self.focus_requests.get().saturating_add(1));
        if let Some(dom) = &self.dom {
            let _ = dom.textarea.focus();
        }
    }

    /// How many times [`focus`](Self::focus) was called on this pipe.
    pub fn focus_requests(&self) -> u32 {
        self.focus_requests.get()
    }

    /// Empty the buffer. The host clears after each committed `Text`, and an
    /// edit session clears on open, so the textarea never accumulates state.
    pub fn clear(&self) {
        if let Some(dom) = &self.dom {
            dom.textarea.set_value("");
        }
    }

    /// Say whether a key on the pipe is typed text (`CHROME-103`). A pipe is
    /// typed text by default, as in a page or a cell. A board says it is not
    /// while no text editor is open, so a bare tool key reaches the keymap.
    pub fn set_typing(&self, typing: Typing) {
        self.typing.set(typing);
        if let Some(dom) = &self.dom {
            let flag = match typing {
                Typing::Commands => "false",
                Typing::Text => "true",
            };
            let _ = dom.textarea.set_attribute(TYPING_ATTR, flag);
        }
    }

    /// What a key on the pipe is, as the view last said.
    pub fn typing(&self) -> Typing {
        self.typing.get()
    }

    /// Name the pipe after the kind of document the view holds (`CHROME-024`).
    /// The top-level view calls it once it owns the pipe. An embedded child
    /// never does, since the pipe still belongs to the document around it.
    pub fn name_content(&self, kind: ContentKind) {
        let Some(dom) = &self.dom else { return };
        let _ = dom.textarea.set_attribute("aria-label", kind.label());
        let _ = dom
            .textarea
            .set_attribute("aria-roledescription", kind.role_description());
    }

    /// Announce the focused content (for example `"B2, 42"`) to screen readers via the
    /// polite live region. Called by the view when the selection moves or a cell
    /// edit commits (`ambre-spec-accessibility.md`). Replacing the text content
    /// of an `aria-live` region is what triggers the announcement.
    pub fn announce(&self, text: &str) {
        if let Some(dom) = &self.dom {
            dom.live.set_text_content(Some(text));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `CHROME-024`: each kind names itself, and only the spreadsheet says it
    /// is a cell. A board or a page announced as a cell is the defect.
    #[test]
    fn only_the_spreadsheet_names_itself_a_cell() {
        let kinds = [
            ContentKind::Spreadsheet,
            ContentKind::Board,
            ContentKind::Page,
            ContentKind::Conversation,
        ];
        for kind in kinds {
            let says_cell = kind.label().to_lowercase().contains("cell")
                || kind.role_description().contains("cell");
            assert_eq!(says_cell, kind == ContentKind::Spreadsheet, "{kind:?}");
        }
        let mut labels: Vec<_> = kinds.iter().map(|k| k.label()).collect();
        labels.dedup();
        assert_eq!(labels.len(), 4, "no two kinds share a name");
    }
}
