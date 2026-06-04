// ambre-chrome/src/dialog.rs
// ============================================================================
// Native <dialog> modal (ambre-spec-chrome.md, 2.A/2.B).
//
// `showModal()` gives focus trap, inertness of the rest of the page, Escape to
// close and a `::backdrop` for free -- no need to reimplement any of it. The
// component only bridges that native element to a Leptos `open` signal and
// fires `on_open`/`on_close` so the launcher can suspend/resume the canvas rAF
// (a fullscreen modal freezes the canvas: battery saving + free backdrop blur).
// ============================================================================

use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

/// A modal dialog driven by an `open` signal.
///
/// Opening calls `show_modal()`; closing (Escape, backdrop click, or `open`
/// set false) calls `close()`. The component keeps `open` in sync with the
/// native state in every direction, so callers can both command and observe it.
#[component]
pub fn Dialog(
    /// Visibility. Set `false` to close; the component flips it back on Escape
    /// and backdrop click so it always reflects the true state.
    open: RwSignal<bool>,
    /// Fired after the modal opens (e.g. to pause the canvas rAF).
    #[prop(optional, into)]
    on_open: Option<Callback<()>>,
    /// Fired after the modal closes (e.g. to resume the canvas rAF).
    #[prop(optional, into)]
    on_close: Option<Callback<()>>,
    children: ChildrenFn,
) -> impl IntoView {
    let dialog_ref = NodeRef::<html::Dialog>::new();

    // Reflect `open` into the native modal state.
    Effect::new(move |_| {
        let want_open = open.get();
        let Some(el) = dialog_ref.get() else { return };
        if want_open && !el.open() {
            let _ = el.show_modal();
            if let Some(cb) = on_open {
                cb.run(());
            }
        } else if !want_open && el.open() {
            el.close(); // fires the native `close` event -> handle_close
        }
    });

    // Native close (Escape, or el.close()): sync the signal and notify. Both
    // are idempotent, so this is safe whatever initiated the close.
    let handle_close = move |_| {
        open.set(false);
        if let Some(cb) = on_close {
            cb.run(());
        }
    };

    // A click whose target is the <dialog> element itself is a backdrop click
    // (clicks on the body/children target those nodes instead). Body padding
    // carries the inset, so the dialog element is only directly hit on the
    // backdrop.
    let handle_click = move |ev: web_sys::MouseEvent| {
        let on_backdrop = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlDialogElement>().ok())
            .is_some();
        if on_backdrop {
            open.set(false);
        }
    };

    view! {
        <dialog
            node_ref=dialog_ref
            class="ambre-dialog"
            on:close=handle_close
            on:click=handle_click
        >
            <div class="ambre-dialog-body">{children()}</div>
        </dialog>
    }
}
