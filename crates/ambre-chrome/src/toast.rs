// ambre-chrome/src/toast.rs
// ============================================================================
// Toasts -- transient notifications in an ARIA live region
// (ambre-spec-accessibility.md, CHROME-006). The host is a `position: fixed`
// container marked `role="status" aria-live="polite"`, so screen readers
// announce each new toast without stealing focus. A visual toast is hidden
// from them, because a live line of the page already says its words. Entrance/exit are CSS
// transform/opacity animations run by the compositor, off the canvas rAF budget:
// zero millisecond stolen from the frame (ambre-spec-chrome.md).
//
// Shared, app-agnostic: an app pushes messages through the `Toasts` handle; the
// `ToastHost` renders + auto-dismisses them. Written once for every Ambre app.
// ============================================================================

use std::time::Duration;

use ambre_core::{Seq, SeqKind};
use leptos::prelude::*;

/// Severity of a toast -- drives its accent stripe (status tokens) and, for
/// errors, a longer dwell so they are not missed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Warning,
    Error,
}

impl ToastKind {
    fn class(self) -> &'static str {
        match self {
            ToastKind::Info => "info",
            ToastKind::Success => "success",
            ToastKind::Warning => "warning",
            ToastKind::Error => "error",
        }
    }

    /// Auto-dismiss delay. Errors linger longer.
    fn dwell(self) -> Duration {
        match self {
            ToastKind::Error => Duration::from_millis(7000),
            _ => Duration::from_millis(4000),
        }
    }
}

/// Whether a screen reader announces a toast. A live line of the page can
/// already announce the words of a toast. That toast is visual alone, so a
/// screen reader does not read the words twice (`CHROME-136`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToastVoice {
    /// The live region announces the toast (`CHROME-006`).
    Announced,
    /// The toast is hidden from assistive technology: another line of the
    /// page announces the same words.
    Visual,
}

/// The number a toast is raised under, so a dismissal names that toast alone.
pub enum ToastId {}

impl SeqKind for ToastId {
    type IssueRight = ();
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("ToastId");
}

#[derive(Clone)]
struct Toast {
    id: Seq<ToastId>,
    message: String,
    kind: ToastKind,
    voice: ToastVoice,
}

/// Shared toast store. Cheap to copy (just signals); created inside the Leptos
/// owner and handed to the `ToastHost` plus anything that raises notifications.
#[derive(Clone, Copy)]
pub struct Toasts {
    items: RwSignal<Vec<Toast>>,
    next_id: RwSignal<Seq<ToastId>>,
}

impl Default for Toasts {
    fn default() -> Self {
        Self::new()
    }
}

impl Toasts {
    pub fn new() -> Self {
        Self {
            items: RwSignal::new(Vec::new()),
            next_id: RwSignal::new(Seq::ZERO),
        }
    }

    pub fn info(self, message: impl Into<String>) {
        self.push(ToastKind::Info, message);
    }
    pub fn success(self, message: impl Into<String>) {
        self.push(ToastKind::Success, message);
    }
    pub fn warning(self, message: impl Into<String>) {
        self.push(ToastKind::Warning, message);
    }
    pub fn error(self, message: impl Into<String>) {
        self.push(ToastKind::Error, message);
    }

    /// Raise an announced toast and schedule its auto-dismiss.
    pub fn push(self, kind: ToastKind, message: impl Into<String>) {
        self.raise(kind, ToastVoice::Announced, message);
    }

    /// Raise a toast under `voice` and schedule its auto-dismiss.
    pub fn raise(self, kind: ToastKind, voice: ToastVoice, message: impl Into<String>) {
        let id = self.next_id.get_untracked();
        self.next_id.set(id.next());
        self.items.update(|v| {
            v.push(Toast {
                id,
                message: message.into(),
                kind,
                voice,
            })
        });

        let items = self.items;
        set_timeout(
            move || items.update(|v| v.retain(|t| t.id != id)),
            kind.dwell(),
        );
    }

    fn dismiss(self, id: Seq<ToastId>) {
        self.items.update(|v| v.retain(|t| t.id != id));
    }
}

/// The live-region container. Mount once near the app root; it renders whatever
/// toasts are in `toasts`. Clicking a toast dismisses it early.
#[component]
pub fn ToastHost(toasts: Toasts) -> impl IntoView {
    view! {
        <div class="ambre-toasts" role="status" aria-live="polite" aria-atomic="false">
            <For each=move || toasts.items.get() key=|t| t.id let:toast>
                <div
                    class=format!("ambre-toast {}", toast.kind.class())
                    aria-hidden=(toast.voice == ToastVoice::Visual).then_some("true")
                    on:click=move |_| toasts.dismiss(toast.id)
                >
                    {toast.message.clone()}
                </div>
            </For>
        </div>
    }
}
