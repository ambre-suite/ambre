// ambre-toile/src/chrome/comments.rs
// ============================================================================
// The comments panel and the drop composer (toile-comments). Two Leptos
// components over the shared signals `ui::comments::CommentSignals` carries:
//
//   * `CommentsPanel` -- the board-wide thread list, filterable by open /
//     resolved, and the one thread it shows whole. Every word it renders was
//     read back from the board (`TOILE-UI-023`); it answers nothing itself, and
//     re-reads on the view's cue (`TOILE-UI-310`).
//   * `CommentComposer` -- the field the comment tool opens where the user
//     clicked (`TOILE-UI-307`). The thread is born with its first message, so
//     nothing is written until the user posts.
//
// Every action leaves as a `CommentIntent` the canvas view drains: the DOM
// names no camera and holds no command queue.
// ============================================================================

use crate::message::{CommentSummary, CommentThread};
use leptos::html;
use leptos::prelude::*;

use crate::ui::PanelState;
use crate::ui::comments::{CommentDraft, CommentIntent, ThreadFilter};
use ambre_core::{Authored, Instant};

/// The comments panel. `state` says whether it shows; the View menu, a pin click and
/// a fresh thread all set it. `list` and `open` are what the view read back
/// from the board; `on_intent` carries every action out.
#[component]
pub fn CommentsPanel(
    state: RwSignal<PanelState>,
    list: RwSignal<Vec<CommentSummary>>,
    open: RwSignal<Option<CommentThread>>,
    on_intent: Callback<CommentIntent>,
) -> impl IntoView {
    let filter = RwSignal::new(ThreadFilter::default());
    let reply = RwSignal::new(String::new());

    let close = move || {
        state.set(PanelState::Closed);
        open.set(None);
    };

    let send_reply = move || {
        let Some(thread) = open.with_untracked(|t| t.as_ref().map(|t| t.id.clone())) else {
            return;
        };
        let body = reply.get_untracked();
        if body.is_empty() {
            return;
        }
        reply.set(String::new());
        on_intent.run(CommentIntent::Reply { thread, body });
    };

    move || {
        state.get().is_open().then(|| {
            view! {
                <div class="ambre-comments-panel" style="position: fixed; top: 8px; right: 8px; z-index: 60;">
                    <div class="ambre-comments-title">
                        <span class="ambre-comments-heading">"Comments"</span>
                        <button class="ambre-comments-close" aria-label="Close" on:click=move |_| close()>
                            "x"
                        </button>
                    </div>

                    <div class="ambre-comments-filter">
                        {[
                            (ThreadFilter::All, "All"),
                            (ThreadFilter::Open, "Open"),
                            (ThreadFilter::Resolved, "Resolved"),
                        ]
                            .into_iter()
                            .map(|(f, label)| view! {
                                <button
                                    class="ambre-comments-filter-btn"
                                    aria-pressed=move || ambre_chrome::Pressed::from_bool(filter.get() == f).aria()
                                    on:click=move |_| filter.set(f)
                                >
                                    {label}
                                </button>
                            })
                            .collect_view()}
                    </div>

                    <ul class="ambre-comments-list">
                        {move || {
                            let f = filter.get();
                            list.get()
                                .into_iter()
                                .filter(|t| f.keeps(t.resolved))
                                .map(|t| {
                                    let anchor = t.anchor.clone();
                                    let id = t.id.clone();
                                    view! {
                                        <li class="ambre-comments-row">
                                            <button
                                                class="ambre-comments-row-btn"
                                                on:click=move |_| on_intent.run(CommentIntent::Focus {
                                                    thread: id.clone(),
                                                    anchor: anchor.clone(),
                                                })
                                            >
                                                <span class="ambre-comments-preview">{t.preview.clone()}</span>
                                                <span class="ambre-comments-count">{t.messages.get().to_string()}</span>
                                                {(t.resolved == crate::ui::comments::ThreadState::Resolved).then(|| view! {
                                                    <span class="ambre-comments-resolved">"resolved"</span>
                                                })}
                                            </button>
                                        </li>
                                    }
                                })
                                .collect_view()
                        }}
                    </ul>

                    {move || open.get().map(|thread| {
                        let id = thread.id.clone();
                        let resolved = thread.resolved;
                        view! {
                            <div class="ambre-comments-thread">
                                <div class="ambre-comments-messages">
                                    {thread.messages.iter().map(|m| view! {
                                        <div class="ambre-comments-message">
                                            <span class="ambre-comments-author">{ambre_core::display_handle(&m.author).into_string()}</span>
                                            <span class="ambre-comments-time">{clock_label(m.ts)}</span>
                                            <p class="ambre-comments-body">{m.body.clone()}</p>
                                        </div>
                                    }).collect_view()}
                                </div>
                                <div class="ambre-comments-reply">
                                    <input
                                        type="text"
                                        class="ambre-comments-input"
                                        placeholder="Reply"
                                        prop:value=move || reply.get()
                                        on:input=move |ev| reply.set(event_target_value(&ev))
                                        on:keydown=move |ev: web_sys::KeyboardEvent| {
                                            if ev.key() == "Enter" {
                                                ev.prevent_default();
                                                send_reply();
                                            }
                                        }
                                    />
                                    <button class="ambre-comments-send" on:click=move |_| send_reply()>
                                        "Send"
                                    </button>
                                    <button
                                        class="ambre-comments-resolve"
                                        on:click=move |_| on_intent.run(CommentIntent::Resolve {
                                            thread: id.clone(),
                                            state: match resolved {
                                                crate::ui::comments::ThreadState::Open => crate::ui::comments::ThreadState::Resolved,
                                                crate::ui::comments::ThreadState::Resolved => crate::ui::comments::ThreadState::Open,
                                            },
                                        })
                                    >
                                        {if resolved == crate::ui::comments::ThreadState::Resolved { "Reopen" } else { "Resolve" }}
                                    </button>
                                </div>
                            </div>
                        }
                    })}
                </div>
            }
        })
    }
}

/// The drop composer (`TOILE-UI-307`): a field at the click, whose first post
/// opens the thread. Escape abandons it, and nothing is written.
#[component]
pub fn CommentComposer(
    draft: RwSignal<Option<CommentDraft>>,
    on_intent: Callback<CommentIntent>,
) -> impl IntoView {
    let body = RwSignal::new(String::new());
    let field = NodeRef::<html::Input>::new();

    // Focus the field as the composer opens: the user types where they clicked.
    Effect::new(move |_| {
        if draft.get().is_some()
            && let Some(input) = field.get()
        {
            let _ = input.focus();
        }
    });

    let post = move || {
        let text = body.get_untracked();
        body.set(String::new());
        on_intent.run(CommentIntent::Post(text));
    };
    let discard = move || {
        body.set(String::new());
        on_intent.run(CommentIntent::Discard);
    };

    move || {
        draft.get().map(|d| {
            let style = format!(
                "position: fixed; left: {}px; top: {}px; z-index: 61;",
                d.at.x, d.at.y
            );
            view! {
                <div class="ambre-comment-composer" style=style>
                    <input
                        type="text"
                        class="ambre-comment-composer-input"
                        placeholder="Comment"
                        node_ref=field
                        prop:value=move || body.get()
                        on:input=move |ev| body.set(event_target_value(&ev))
                        on:keydown=move |ev: web_sys::KeyboardEvent| {
                            match ev.key().as_str() {
                                "Enter" => { ev.prevent_default(); post(); }
                                "Escape" => { ev.prevent_default(); discard(); }
                                _ => {}
                            }
                        }
                    />
                    <button class="ambre-comment-composer-post" on:click=move |_| post()>
                        "Post"
                    </button>
                </div>
            }
        })
    }
}

/// The wall clock a message carries, rendered for the reader. Nothing orders by
/// it (`TOILE-FEAT-012`): it is shown and used for nothing else.
fn clock_label(ts: Instant<Authored>) -> String {
    let date = js_sys::Date::new(&(ts.get() as f64).into());
    date.to_locale_time_string("en-US")
        .as_string()
        .unwrap_or_default()
}
