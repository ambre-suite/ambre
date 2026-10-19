// ambre-chrome/src/dialog.rs
// ============================================================================
// Native <dialog> modal (ambre-spec-chrome.md, CHROME-011 / CHROME-012).
//
// `showModal()` gives focus trap, inertness of the rest of the page, Escape to
// close and a `::backdrop` for free. None of it needs reimplementing. The
// component only bridges that native element to a Leptos `open` signal.
//
// A fullscreen modal freezes the canvas: battery saving, plus a free backdrop
// blur. The launcher provides a `ModalHold` once, and every `Dialog` takes it
// on open and gives it back on close or unmount. No caller wires the pause, so
// no caller can forget the resume.
// ============================================================================

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

/// What an open modal does to the frame loop, provided once by the launcher
/// as a context. `true` takes a hold, `false` releases it.
#[derive(Clone)]
pub struct ModalHold(Arc<dyn Fn(bool) + Send + Sync>);

impl ModalHold {
    /// A hold that calls `set(true)` when a modal opens and `set(false)` when
    /// it closes.
    pub fn new(set: impl Fn(bool) + Send + Sync + 'static) -> Self {
        Self(Arc::new(set))
    }

    /// The hold one dialog owns: taken at most once, released at most once.
    fn owned(self) -> OwnedHold {
        OwnedHold {
            hold: self,
            held: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// One dialog's share of the `ModalHold`. The flag pairs every take with one
/// release, so a close event repeated, or a close after an unmount, releases
/// nothing twice.
#[derive(Clone)]
struct OwnedHold {
    hold: ModalHold,
    held: Arc<AtomicBool>,
}

impl OwnedHold {
    fn take(&self) {
        if !self.held.swap(true, Ordering::Relaxed) {
            (self.hold.0)(true);
        }
    }

    fn release(&self) {
        if self.held.swap(false, Ordering::Relaxed) {
            (self.hold.0)(false);
        }
    }
}

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
    /// Fired after the modal opens, for work of the caller's own. The frame
    /// loop is the `ModalHold`'s, not this callback's.
    #[prop(optional, into)]
    on_open: Option<Callback<()>>,
    /// Fired after the modal closes.
    #[prop(optional, into)]
    on_close: Option<Callback<()>>,
    /// Extra class on the `<dialog>` element, for per-dialog styling.
    #[prop(optional, into)]
    class: Option<String>,
    /// The modal's accessible name, for one with no heading to name it.
    #[prop(optional, into)]
    label: Option<String>,
    children: ChildrenFn,
) -> impl IntoView {
    let dialog_ref = NodeRef::<html::Dialog>::new();
    let hold = use_context::<ModalHold>().map(ModalHold::owned);
    // A dialog unmounted while open fires no `close` event: give the hold
    // back here, or the loop stays suspended.
    if let Some(hold) = hold.clone() {
        on_cleanup(move || hold.release());
    }

    // Reflect `open` into the native modal state.
    let open_hold = hold.clone();
    Effect::new(move |_| {
        let want_open = open.get();
        let Some(el) = dialog_ref.get() else { return };
        if want_open && !el.open() {
            let _ = el.show_modal();
            if let Some(hold) = &open_hold {
                hold.take();
            }
            if let Some(cb) = on_open {
                cb.run(());
            }
        } else if !want_open && el.open() {
            el.close(); // fires the native `close` event -> handle_close
        }
    });

    // Native close (Escape, or el.close()): sync the signal and notify. All
    // three are idempotent, so this is safe whatever initiated the close.
    let handle_close = move |_| {
        open.set(false);
        if let Some(hold) = &hold {
            hold.release();
        }
        // The native event is queued: it can land after the owner of the
        // callback is gone, and a disposed callback has no one to notify.
        if let Some(cb) = on_close {
            let _ = cb.try_run(());
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

    let class = class
        .map(|c| format!("ambre-dialog {c}"))
        .unwrap_or_else(|| "ambre-dialog".to_string());

    view! {
        <dialog
            node_ref=dialog_ref
            class=class
            aria-label=label
            on:close=handle_close
            on:click=handle_click
        >
            <div class="ambre-dialog-body">{children()}</div>
        </dialog>
    }
}
