// ambre-prose/src/chrome/mod.rs
// ============================================================================
// The Prose annotation chrome: the comment panel and the suggestion popover
// (`features/ambre-spec-prose-comments.md section 4`,
// `features/ambre-spec-prose-track-changes.md section 4`).
//
// Both are DOM, never canvas overlays (`PROSE-UI-050`, `PROSE-UI-052`). They
// render what the Prose view published and hand every action back to it as
// one `AnnotationIntent`: the DOM holds no command channel.
// ============================================================================

use leptos::prelude::*;

use crate::events::SuggestionKind;
use crate::view::annotate::{AnnotationIntent, HoveredSuggestion, PanelThread};

/// The signals the chrome shares with the view. Every position is in CSS
/// pixels of the window.
#[derive(Clone, Copy)]
pub struct ProseAnnotations {
    /// The threads the panel lists, each at its anchor's height.
    pub threads: RwSignal<Vec<PanelThread>>,
    /// The suggestion under the pointer, for the popover.
    pub hovered: RwSignal<Option<HoveredSuggestion>>,
    /// What the chrome asked for, until the view drains it.
    pub intent: RwSignal<Option<AnnotationIntent>>,
}

impl Default for ProseAnnotations {
    fn default() -> Self {
        Self::new()
    }
}

impl ProseAnnotations {
    #[must_use]
    pub fn new() -> Self {
        Self {
            threads: RwSignal::new(Vec::new()),
            hovered: RwSignal::new(None),
            intent: RwSignal::new(None),
        }
    }
}

/// The width of the panel, in CSS pixels.
const PANEL_WIDTH_PX: u32 = 260;
/// How far above the suggestion the popover sits, in CSS pixels.
const POPOVER_LIFT_PX: f32 = 30.0;

/// The comment panel, right of the page: each open thread at the height of
/// its anchor. A click on a thread's text brings the anchor into view.
#[component]
pub fn ProseCommentsPanel(signals: ProseAnnotations) -> impl IntoView {
    let send = move |intent| signals.intent.set(Some(intent));
    move || {
        let threads = signals.threads.get();
        (!threads.is_empty()).then(|| {
            view! {
                <div
                    class="ambre-prose-comments"
                    style=format!(
                        "position: fixed; top: 0; right: 8px; bottom: 0; width: {PANEL_WIDTH_PX}px; \
                         pointer-events: none; z-index: 60;"
                    )
                >
                    {threads.into_iter().map(|entry| thread_card(entry, send)).collect_view()}
                </div>
            }
        })
    }
}

/// One thread: its replies, a field to answer, and a button to resolve it.
fn thread_card(
    entry: PanelThread,
    send: impl Fn(AnnotationIntent) + Copy + 'static,
) -> impl IntoView {
    let comment_id = entry.thread.comment_id;
    let reply = RwSignal::new(String::new());
    let post = move || {
        let text = reply.get_untracked();
        reply.set(String::new());
        send(AnnotationIntent::Reply { comment_id, text });
    };
    view! {
        <div
            class="ambre-prose-thread"
            style=format!(
                "position: absolute; top: {}px; left: 0; right: 0; pointer-events: auto;",
                entry.y_px
            )
        >
            <button
                class="ambre-prose-thread-focus"
                on:click=move |_| send(AnnotationIntent::Focus { comment_id })
            >
                {entry.thread.replies.iter().map(|r| view! {
                    <div class="ambre-prose-reply">
                        <span class="ambre-prose-reply-author">{r.author.clone()}</span>
                        <p class="ambre-prose-reply-text">{r.text.clone()}</p>
                    </div>
                }).collect_view()}
            </button>
            <div class="ambre-prose-thread-actions">
                <input
                    type="text"
                    class="ambre-prose-thread-input"
                    placeholder="Reply"
                    prop:value=move || reply.get()
                    on:input=move |ev| reply.set(event_target_value(&ev))
                    on:keydown=move |ev: web_sys::KeyboardEvent| {
                        if ev.key() == "Enter" {
                            ev.prevent_default();
                            post();
                        }
                    }
                />
                <button class="ambre-prose-thread-send" on:click=move |_| post()>"Send"</button>
                <button
                    class="ambre-prose-thread-resolve"
                    on:click=move |_| send(AnnotationIntent::Resolve { comment_id })
                >
                    "Resolve"
                </button>
            </div>
        </div>
    }
}

/// The accept and reject buttons over the suggestion under the pointer, and
/// over no other (`PROSE-UI-148`).
#[component]
pub fn ProseSuggestionPopover(signals: ProseAnnotations) -> impl IntoView {
    move || {
        signals.hovered.get().map(|hovered| {
            let suggestion_id = hovered.suggestion_id;
            let what = match hovered.kind {
                SuggestionKind::Insertion => "insertion",
                SuggestionKind::Deletion => "deletion",
            };
            let style = format!(
                "position: fixed; left: {}px; top: {}px; z-index: 61;",
                hovered.rect.x0,
                (hovered.rect.y0 - POPOVER_LIFT_PX).max(0.0)
            );
            view! {
                <div class="ambre-prose-suggestion" style=style role="group">
                    <button
                        class="ambre-prose-suggestion-accept"
                        aria-label=format!("Accept the {what}")
                        on:click=move |_| signals
                            .intent
                            .set(Some(AnnotationIntent::Accept { suggestion_id }))
                    >
                        "Accept"
                    </button>
                    <button
                        class="ambre-prose-suggestion-reject"
                        aria-label=format!("Reject the {what}")
                        on:click=move |_| signals
                            .intent
                            .set(Some(AnnotationIntent::Reject { suggestion_id }))
                    >
                        "Reject"
                    </button>
                </div>
            }
        })
    }
}
