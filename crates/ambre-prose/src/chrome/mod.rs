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

use ambre_core::index::Pos;
use std::collections::HashMap;

use ambre_core::id::{Comment, Id};
use ambre_core::{Css, Point, Px, View};
use leptos::prelude::*;

mod handler;
mod menu;
mod notify;
pub use handler::ProseHandler;
pub use menu::context_entries;
pub use notify::notify_event;

use ambre_chrome::{FormatBar, FormatControl, Pressed};
use ambre_commands::CommandId;

use crate::events::{SuggestionKind, TriState};
use crate::ui::ProseUiState;
use crate::view::annotate::{AnnotationIntent, HoveredSuggestion, LostReply, PanelThread};
use crate::view::ruler::{RulerDrag, RulerHandle, RulerRelease, RulerState};
use crate::view::slash::SlashMenuState;
use crate::view::toolbar::{ToolbarState, state_of};

ambre_core::define_flag! {
    /// Whether the pointer rests on a link's text, which picks the canvas
    /// cursor (`PROSE-UI-153`).
    pub enum LinkHover {
        /// The pointer is off every link: the text cursor shows.
        Elsewhere,
        /// The pointer rests on a link: the link cursor shows.
        OverLink,
    }
}

/// The signals the chrome and the shell share with the view.
///
/// The view publishes its positions in pixels of the view, from the canvas's
/// corner. The host publishes where that corner sits on the page, and the
/// chrome moves each position onto the page by it.
#[derive(Clone, Copy)]
pub struct ProseSignals {
    /// The canvas's top-left corner on the page.
    pub canvas_origin: RwSignal<Point<Px<Css>>>,
    /// The threads the panel lists, each at its anchor's height.
    pub threads: RwSignal<Vec<PanelThread>>,
    /// The reply being typed in each thread's field, by thread. It outlives
    /// the card, so a thread that moves keeps it (`PROSE-UI-231`).
    pub drafts: RwSignal<HashMap<Id<Comment>, String>>,
    /// The suggestion under the pointer, for the popover.
    pub hovered: RwSignal<Option<HoveredSuggestion>>,
    /// What the chrome asked for, until the view drains it.
    pub intent: RwSignal<Option<AnnotationIntent>>,
    /// The pointer rests on a link: the canvas shows the link cursor
    /// (`PROSE-UI-153`).
    pub over_link: RwSignal<LinkHover>,
    /// A link target to open, until the shell takes it (`PROSE-UI-071`).
    pub followed: RwSignal<Option<ambre_core::LinkUrl>>,
    /// The format bar, its anchor in CSS pixels (`PROSE-UI-053`).
    pub toolbar: RwSignal<ToolbarState>,
    /// A formatting command a control or a chord invoked, until the view
    /// drains it. Both write the same id here (`PROSE-UI-056`).
    pub format: RwSignal<Option<CommandId>>,
    /// The slash menu, its anchor in CSS pixels (`PROSE-UI-090`).
    pub slash: RwSignal<SlashMenuState>,
    /// The menu item picked with the pointer, until the view drains it.
    pub slash_pick: RwSignal<Option<Pos<crate::view::slash::SlashChoice>>>,
    /// The ruler (`PROSE-UI-064`).
    pub ruler: RwSignal<RulerState>,
    /// A released ruler handle, until the view drains it (`PROSE-UI-063`).
    pub ruler_release: RwSignal<Option<RulerRelease>>,
    /// Whether the local history holds a step to undo, and one to redo, as
    /// the Logic thread last reported it. The chrome counts nothing
    /// (`UNDO-006`).
    pub history: RwSignal<History>,
}

/// What the local history holds, as the Logic thread last reported it. A
/// fresh document holds neither step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct History {
    pub undo: ambre_core::Availability,
    pub redo: ambre_core::Availability,
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
            canvas_origin: RwSignal::new(Point::default()),
            threads: RwSignal::new(Vec::new()),
            drafts: RwSignal::new(HashMap::new()),
            hovered: RwSignal::new(None),
            intent: RwSignal::new(None),
            over_link: RwSignal::new(LinkHover::Elsewhere),
            followed: RwSignal::new(None),
            toolbar: RwSignal::new(ToolbarState::default()),
            format: RwSignal::new(None),
            slash: RwSignal::new(SlashMenuState::default()),
            slash_pick: RwSignal::new(None),
            ruler: RwSignal::new(RulerState::default()),
            ruler_release: RwSignal::new(None),
            history: RwSignal::new(History::default()),
        }
    }

    /// Take in the history's availability from a Prose
    /// [`UndoState`](crate::events::ProseEvent::UndoState). Every other event
    /// leaves it as it was (`UNDO-006`).
    pub fn mirror_event(&self, ev: &ambre_bus::UiEvent) {
        if let ambre_bus::UiEvent::Viewport {
            event: ambre_bus::AppEvent::Prose(crate::events::ProseEvent::UndoState { undo, redo }),
            ..
        } = ev
        {
            self.history.set(History {
                undo: *undo,
                redo: *redo,
            });
        }
    }

    /// Drop each reply draft whose thread left the panel, and answer the
    /// way each that held text was lost, so its notice is owed
    /// (`PROSE-UI-232`).
    pub fn drop_lost_drafts(&self, view: &ProseUiState) -> Vec<LostReply> {
        let lost = self
            .drafts
            .with_untracked(|drafts| view.lost_drafts(drafts));
        if lost.is_empty() {
            return Vec::new();
        }
        self.drafts.update(|drafts| {
            for (id, _) in &lost {
                drafts.remove(id);
            }
        });
        lost.into_iter().filter_map(|(_, notice)| notice).collect()
    }
}

/// The width of the panel, in CSS pixels.
const PANEL_WIDTH: Px<Css> = Px::<Css>::new(260.0);
/// How far above the suggestion the popover sits, in CSS pixels.
const POPOVER_LIFT: Px<Css> = Px::<Css>::new(30.0);

/// The comment panel, right of the page: each open thread at the height of
/// its anchor. A click on a thread's text brings the anchor into view.
///
/// A card is keyed by its thread, so a move restyles it and never rebuilds
/// it. The field keeps its focus, and its text lives in `drafts`
/// (`PROSE-UI-231`).
#[component]
pub fn ProseCommentsPanel(signals: ProseSignals) -> impl IntoView {
    view! {
        <div
            class="ambre-prose-comments"
            style=move || format!(
                "position: fixed; top: 0; right: 8px; bottom: 0; width: {}px; \
                 pointer-events: none; z-index: 60;{}",
                PANEL_WIDTH.get(),
                if signals.threads.with(Vec::is_empty) { " display: none;" } else { "" }
            )
        >
            <For
                each=move || signals.threads.get()
                key=|entry| entry.thread.comment_id
                children=move |entry| thread_card(entry.thread.comment_id, signals)
            />
        </div>
    }
}

/// One thread: its replies, a field to answer, and a button to resolve it.
/// It reads its thread from the published list, so a move or a new reply
/// updates it in place.
fn thread_card(comment_id: Id<Comment>, signals: ProseSignals) -> impl IntoView {
    let send = move |intent| signals.intent.set(Some(intent));
    let entry = Memo::new(move |_| {
        signals.threads.with(|threads| {
            threads
                .iter()
                .find(|e| e.thread.comment_id == comment_id)
                .cloned()
        })
    });
    let take_draft = move || {
        signals
            .drafts
            .try_update(|drafts| drafts.remove(&comment_id))
            .flatten()
            .unwrap_or_default()
    };
    let post = move || {
        let text = take_draft();
        send(AnnotationIntent::Reply { comment_id, text });
    };
    view! {
        <div
            class="ambre-prose-thread"
            style=move || format!(
                "position: absolute; top: {}px; left: 0; right: 0; pointer-events: auto;",
                entry.with(|e| e.as_ref().map_or(0.0, |e| {
                    e.y.on_page(signals.canvas_origin.get().y).get()
                }))
            )
        >
            <button
                class="ambre-prose-thread-focus"
                on:click=move |_| send(AnnotationIntent::Focus { comment_id })
            >
                {move || entry.with(|e| {
                    e.iter().flat_map(|e| &e.thread.replies).map(|r| view! {
                        <div class="ambre-prose-reply">
                            <span class="ambre-prose-reply-author">{r.author.as_ref().map(|name| name.as_str().to_owned()).unwrap_or_default()}</span>
                            <p class="ambre-prose-reply-text">{r.text.clone()}</p>
                        </div>
                    }).collect_view()
                })}
            </button>
            <div class="ambre-prose-thread-actions">
                <input
                    type="text"
                    class="ambre-prose-thread-input"
                    placeholder="Reply"
                    prop:value=move || signals.drafts.with(|drafts| {
                        drafts.get(&comment_id).cloned().unwrap_or_default()
                    })
                    on:input=move |ev| {
                        let text = event_target_value(&ev);
                        signals.drafts.update(|drafts| {
                            if text.is_empty() {
                                drafts.remove(&comment_id);
                            } else {
                                drafts.insert(comment_id, text);
                            }
                        });
                    }
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
                    on:click=move |_| {
                        // The user closes the thread: the draft goes with it.
                        take_draft();
                        send(AnnotationIntent::Resolve { comment_id });
                    }
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
        let origin = signals.canvas_origin.get();
        signals.hovered.get().map(|hovered| {
            let suggestion_id = hovered.suggestion_id;
            let what = match hovered.kind {
                SuggestionKind::Insertion => "insertion",
                SuggestionKind::Deletion => "deletion",
            };
            let style = format!(
                "position: fixed; left: {}px; top: {}px; z-index: 61;",
                hovered.rect.x0.on_page(origin.x).get(),
                (hovered.rect.y0.on_page(origin.y) - POPOVER_LIFT)
                    .max(Px::<Css>::ZERO)
                    .get()
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

/// The format bar over a non-empty selection
/// (`features/ambre-spec-prose-toolbar.md section 4`), drawn by the shared
/// bar (`CHROME-105`).
///
/// It holds no document state: it renders what the view published, and a
/// control hands its command id back to the view (`PROSE-UI-057`,
/// `PROSE-UI-098`).
#[component]
pub fn ProseFormatBar(signals: ProseSignals) -> impl IntoView {
    move || {
        let bar = signals.toolbar.get();
        let anchor = bar.anchor?.on_page(signals.canvas_origin.get());
        let controls = crate::commands::FORMAT_BAR
            .map(|(id, key)| {
                let pressed = match bar.active.as_ref().map(|m| state_of(m, key)) {
                    Some(TriState::On) => Pressed::On,
                    Some(TriState::Mixed) => Pressed::Mixed,
                    _ => Pressed::Off,
                };
                control(signals, id, pressed)
            })
            .to_vec();
        Some(view! { <FormatBar anchor=anchor controls=controls /> })
    }
}

/// One control, announced by its command's title.
fn control(signals: ProseSignals, id: CommandId, pressed: Pressed) -> FormatControl {
    let label = ambre_commands::find(crate::commands::COMMANDS, id)
        .map(|command| (command.title)(&ambre_commands::Locale::En))
        .unwrap_or_default();
    FormatControl {
        label,
        pressed,
        on_press: Callback::new(move |()| signals.format.set(Some(id))),
    }
}

/// How far below the slash's line the menu sits, in CSS pixels.
const MENU_DROP: Px<Css> = Px::<Css>::new(4.0);

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
        let at = menu.anchor?.on_page(signals.canvas_origin.get());
        let style = format!(
            "position: fixed; left: {}px; top: {}px; z-index: 62;",
            at.x.get(),
            (at.y + MENU_DROP).get()
        );
        let items = menu.items.iter_enumerated().map(|(index, label)| {
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

/// The height of the ruler, in CSS pixels.
const RULER_HEIGHT: Px<Css> = Px::<Css>::new(22.0);

/// Where a pointer event falls on the canvas whose corner sits at `origin`.
fn pointer_x(ev: &web_sys::PointerEvent, origin: Point<Px<Css>>) -> Px<View> {
    Px::<Css>::new(ev.client_x() as f32).in_canvas(origin.x)
}

/// The ruler above the page (`features/ambre-spec-prose-ruler.md section 4`).
///
/// It renders what the view published. A handle in motion is painted at the
/// pointer and writes nothing (`PROSE-UI-076`). Its release goes back to the
/// view as one value, and the view sends one command (`PROSE-UI-063`).
#[component]
pub fn ProseRuler(signals: ProseSignals) -> impl IntoView {
    let drag = RwSignal::new(None::<RulerDrag>);
    let release = move |ev: web_sys::PointerEvent| {
        let Some(mut held) = drag.get_untracked() else {
            return;
        };
        held.move_to(pointer_x(&ev, signals.canvas_origin.get_untracked()));
        drag.set(None);
        let ruler = signals.ruler.get_untracked();
        signals.ruler_release.set(Some(held.release(&ruler)));
    };
    move || {
        let ruler = signals.ruler.get();
        if ruler.content_width <= Px::<View>::ZERO {
            return None;
        }
        let origin = signals.canvas_origin.get();
        let style = format!(
            "position: fixed; left: {}px; top: {}px; width: {}px; height: {}px; \
             z-index: 59;",
            ruler.page_left.on_page(origin.x).get(),
            origin.y.get(),
            ruler.content_width.get(),
            RULER_HEIGHT.get()
        );
        let handles = ruler.visible.as_bool().then(|| {
            [
                (RulerHandle::IndentFirst, "First-line indent"),
                (RulerHandle::IndentLeft, "Left indent"),
                (RulerHandle::IndentRight, "Right indent"),
            ]
            .map(|(handle, label)| {
                let x = move || {
                    drag.get()
                        .filter(|d| d.handle == handle)
                        .map_or_else(|| ruler.handle_x(handle), |d| d.at)
                        - ruler.page_left
                };
                view! {
                    <div
                        class="ambre-prose-ruler-handle"
                        role="slider"
                        aria-label=label
                        style=move || format!(
                            "position: absolute; left: {}px; top: 0; transform: translateX(-50%); \
                             touch-action: none; cursor: ew-resize;",
                            x().get()
                        )
                        on:mousedown=|ev: web_sys::MouseEvent| ev.prevent_default()
                        on:pointerdown=move |ev: web_sys::PointerEvent| {
                            ev.prevent_default();
                            let target = event_target::<web_sys::Element>(&ev);
                            let _ = target.set_pointer_capture(ev.pointer_id());
                            drag.set(Some(RulerDrag::press(handle, pointer_x(&ev, origin))));
                        }
                        on:pointermove=move |ev: web_sys::PointerEvent| {
                            drag.update(|d| {
                                if let Some(d) = d.as_mut().filter(|d| d.handle == handle) {
                                    d.move_to(pointer_x(&ev, origin));
                                }
                            });
                        }
                        on:pointerup=release
                        on:pointercancel=move |_| drag.set(None)
                    >
                        "\u{25BC}"
                    </div>
                }
            })
            .into_iter()
            .collect_view()
        });
        Some(view! {
            <div class="ambre-prose-ruler" style=style role="group" aria-label="Ruler">
                {handles}
            </div>
        })
    }
}
