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

mod menu;
pub use menu::context_entries;

use ambre_commands::CommandId;

use crate::events::{SuggestionKind, TriState};
use crate::view::annotate::{AnnotationIntent, HoveredSuggestion, PanelThread};
use crate::view::slash::SlashMenuState;
use crate::view::toolbar::{ToolbarState, state_of};

/// The signals the chrome and the shell share with the view. Every position
/// is in CSS pixels of the window.
#[derive(Clone, Copy)]
pub struct ProseSignals {
    /// The threads the panel lists, each at its anchor's height.
    pub threads: RwSignal<Vec<PanelThread>>,
    /// The suggestion under the pointer, for the popover.
    pub hovered: RwSignal<Option<HoveredSuggestion>>,
    /// What the chrome asked for, until the view drains it.
    pub intent: RwSignal<Option<AnnotationIntent>>,
    /// The pointer rests on a link: the canvas shows the link cursor
    /// (`PROSE-UI-153`).
    pub over_link: RwSignal<bool>,
    /// A link target to open, until the shell takes it (`PROSE-UI-071`).
    pub followed: RwSignal<Option<String>>,
    /// The format bar, its anchor in CSS pixels (`PROSE-UI-053`).
    pub toolbar: RwSignal<ToolbarState>,
    /// A formatting command a control or a chord invoked, until the view
    /// drains it. Both write the same id here (`PROSE-UI-056`).
    pub format: RwSignal<Option<CommandId>>,
    /// The slash menu, its anchor in CSS pixels (`PROSE-UI-090`).
    pub slash: RwSignal<SlashMenuState>,
    /// The menu item picked with the pointer, until the view drains it.
    pub slash_pick: RwSignal<Option<usize>>,
}

impl Default for ProseSignals {
    fn default() -> Self {
        Self::new()
    }
}

impl ProseSignals {
    #[must_use]
    pub fn new() -> Self {
        Self {
            threads: RwSignal::new(Vec::new()),
            hovered: RwSignal::new(None),
            intent: RwSignal::new(None),
            over_link: RwSignal::new(false),
            followed: RwSignal::new(None),
            toolbar: RwSignal::new(ToolbarState::default()),
            format: RwSignal::new(None),
            slash: RwSignal::new(SlashMenuState::default()),
            slash_pick: RwSignal::new(None),
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
pub fn ProseCommentsPanel(signals: ProseSignals) -> impl IntoView {
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
pub fn ProseSuggestionPopover(signals: ProseSignals) -> impl IntoView {
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

/// How far above the selection's first line the format bar sits, in CSS
/// pixels.
const BAR_LIFT_PX: f32 = 40.0;

/// The format bar over a non-empty selection
/// (`features/ambre-spec-prose-toolbar.md section 4`).
///
/// It holds no document state: it renders what the view published, and a
/// control hands its command id back to the view (`PROSE-UI-057`,
/// `PROSE-UI-098`).
#[component]
pub fn ProseFormatBar(signals: ProseSignals) -> impl IntoView {
    move || {
        let bar = signals.toolbar.get();
        let (x, y) = bar.anchor?;
        let style = format!(
            "position: fixed; left: {x}px; top: {}px; transform: translateX(-50%); z-index: 62;",
            (y - BAR_LIFT_PX).max(0.0)
        );
        let controls = crate::commands::FORMAT_BAR.map(|(id, key)| {
            let pressed = match bar.active.as_ref().map(|m| state_of(m, key)) {
                Some(TriState::On) => "true",
                Some(TriState::Mixed) => "mixed",
                _ => "false",
            };
            control(signals, id, pressed)
        });
        Some(view! {
            <div class="ambre-prose-format-bar" style=style role="toolbar" aria-label="Format">
                {controls.into_iter().collect_view()}
            </div>
        })
    }
}

/// One control. A press keeps the selection: the canvas never loses focus to
/// the bar.
fn control(signals: ProseSignals, id: CommandId, pressed: &'static str) -> impl IntoView {
    let label = ambre_commands::find(crate::commands::COMMANDS, id)
        .map(|command| (command.title)(&ambre_commands::Locale::En))
        .unwrap_or_default();
    let text = label.clone();
    view! {
        <button
            class="ambre-prose-format-control"
            aria-pressed=pressed
            aria-label=label
            on:mousedown=|ev: web_sys::MouseEvent| ev.prevent_default()
            on:click=move |_| signals.format.set(Some(id))
        >
            {text}
        </button>
    }
}

/// How far below the slash's line the menu sits, in CSS pixels.
const MENU_DROP_PX: f32 = 4.0;

/// The slash menu under the caret
/// (`features/ambre-spec-prose-slash-command.md section 4`).
///
/// It renders what the view published. A picked item goes back to the view
/// by its place in the list: the menu sends nothing itself (`PROSE-UI-088`,
/// `PROSE-UI-089`).
#[component]
pub fn ProseSlashMenu(signals: ProseSignals) -> impl IntoView {
    move || {
        let menu = signals.slash.get();
        let (x, y) = menu.anchor?;
        let style = format!(
            "position: fixed; left: {x}px; top: {}px; z-index: 62;",
            y + MENU_DROP_PX
        );
        let items = menu.items.iter().enumerate().map(|(index, label)| {
            let selected = if index == menu.selected {
                "true"
            } else {
                "false"
            };
            view! {
                <li
                    class="ambre-prose-slash-item"
                    role="option"
                    aria-selected=selected
                    on:mousedown=|ev: web_sys::MouseEvent| ev.prevent_default()
                    on:click=move |_| signals.slash_pick.set(Some(index))
                >
                    {*label}
                </li>
            }
        });
        Some(view! {
            <ul class="ambre-prose-slash-menu" style=style role="listbox" aria-label="Insert">
                {items.collect_view()}
            </ul>
        })
    }
}
