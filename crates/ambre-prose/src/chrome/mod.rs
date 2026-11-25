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
use ambre_core::{Commands, StaticKey};
use std::collections::HashMap;

use ambre_core::Flag as _;
use ambre_core::id::{Comment, Id};
use ambre_core::{Css, Point, Px, View};
use leptos::prelude::*;

mod handler;
mod menu;
mod notify;
mod panel;
pub use handler::ProseHandler;
pub use menu::context_entries;
pub use notify::notify_event;
pub use panel::prose_panel;

use ambre_chrome::{FormatBar, FormatControl, Pressed};

use crate::events::{SuggestionKind, TriState};
use crate::ui::ProseUiState;
use crate::view::annotate::{
    AnnotationIntent, HoveredSuggestion, LostReply, Offered, PanelThread, SpellingPopover,
};
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
    /// The suggestion the popover sits on: the one under the pointer, else
    /// the one at the caret (`PROSE-UI-148`, `PROSE-UI-272`).
    pub hovered: RwSignal<Option<HoveredSuggestion>>,
    /// The suggestion at the caret, which the context panel and the palette
    /// accept or reject (`PROSE-UI-273`).
    pub at_caret: RwSignal<Option<HoveredSuggestion>>,
    /// The spelling popover a press on a misspelled word opened
    /// (`PROSE-UI-248`).
    pub spelling: RwSignal<Option<SpellingPopover>>,
    /// What the chrome asked for, until the view drains it.
    pub intent: RwSignal<Option<AnnotationIntent>>,
    /// The pointer rests on a link: the canvas shows the link cursor
    /// (`PROSE-UI-153`).
    pub over_link: RwSignal<LinkHover>,
    /// A link target to open, until the shell takes it (`PROSE-UI-071`).
    pub followed: RwSignal<Option<ambre_core::LinkUrl>>,
    /// Where a menu key opens the page menu, on the canvas, until the menu
    /// closes (`PROSE-UI-237`). `None` for a right-click, whose own event
    /// names its point.
    pub menu: RwSignal<Option<Point<Px<View>>>>,
    /// The format bar, its anchor in CSS pixels (`PROSE-UI-053`).
    pub toolbar: RwSignal<ToolbarState>,
    /// A formatting command a control or a chord invoked, until the view
    /// drains it. Both write the same id here (`PROSE-UI-056`).
    pub format: RwSignal<Option<StaticKey<Commands>>>,
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
    /// The page format the layout was measured on, which the context panel
    /// shows (`PROSE-UI-253`). `None` before the first layout.
    pub page: RwSignal<Option<crate::doc::PageFormat>>,
    /// A page format a control of the context panel asked for, until the
    /// view drains it into one command (`PROSE-UI-252`).
    pub page_write: RwSignal<Option<crate::doc::PageFormat>>,
    /// Whether this session suggests, as the Logic thread last reported it.
    /// The control of the panel and the status chip read it
    /// (`PROSE-UI-275`).
    pub mode: RwSignal<crate::events::EditMode>,
    /// A text, paragraph or default font write a control of the context
    /// panel asked for, until the view drains it into one command
    /// (`PROSE-UI-098`).
    pub text_write: RwSignal<Option<crate::view::panel::TextWrite>>,
    /// The selection the view holds, which a pick copies into its write
    /// (`PROSE-UI-281`). `None` with no text selected.
    pub selection: RwSignal<Option<crate::view::panel::PickedSelection>>,
    /// The two default fonts the layout was measured with, which the
    /// default styles section shows (`PROSE-UI-261`).
    pub fonts: RwSignal<Option<crate::doc::DocFonts>>,
    /// The field of the format bar a link or a comment press opened.
    pub prompt: RwSignal<Option<crate::view::toolbar::BarPrompt>>,
    /// The words of the selected text, which the subtitle of the context
    /// panel shows (`CHROME-241`). `None` with no selection.
    pub selection_words: RwSignal<Option<ambre_core::Count<crate::events::Words>>>,
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
            at_caret: RwSignal::new(None),
            spelling: RwSignal::new(None),
            intent: RwSignal::new(None),
            over_link: RwSignal::new(LinkHover::Elsewhere),
            followed: RwSignal::new(None),
            menu: RwSignal::new(None),
            toolbar: RwSignal::new(ToolbarState::default()),
            format: RwSignal::new(None),
            slash: RwSignal::new(SlashMenuState::default()),
            slash_pick: RwSignal::new(None),
            ruler: RwSignal::new(RulerState::default()),
            ruler_release: RwSignal::new(None),
            history: RwSignal::new(History::default()),
            page: RwSignal::new(None),
            page_write: RwSignal::new(None),
            mode: RwSignal::new(crate::events::EditMode::Editing),
            text_write: RwSignal::new(None),
            selection: RwSignal::new(None),
            fonts: RwSignal::new(None),
            prompt: RwSignal::new(None),
            selection_words: RwSignal::new(None),
        }
    }

    /// Hand the view a write of `value` over the selection shown now. The
    /// write keeps that selection while it waits for the outbox
    /// (`PROSE-UI-281`).
    pub fn pick_text_write(&self, value: crate::view::panel::TextValue) {
        let selection = self.selection.get_untracked();
        self.text_write
            .set(Some(crate::view::panel::TextWrite { value, selection }));
    }

    /// The command of the text write the context panel asked for, taken
    /// out of its signal, over the selection captured at its pick. `None`
    /// when no write waits, or when the pick held no selection the write
    /// needs.
    pub fn take_text_write(&self, view: &ProseUiState) -> Option<crate::message::ProseCommand> {
        let write = self.text_write.get_untracked()?;
        self.text_write.set(None);
        view.text_write(write)
    }

    /// Open the field of the format bar a command id names: the link or
    /// the comment. Answers the field opened, or `None` for any other id,
    /// which the view formats.
    pub fn open_prompt(&self, id: StaticKey<Commands>) -> Option<crate::view::toolbar::BarPrompt> {
        let prompt = crate::view::toolbar::BarPrompt::of(id)?;
        self.prompt.set(Some(prompt));
        Some(prompt)
    }

    /// The command of the page format the context panel asked for, taken
    /// out of its signal. One command carries the whole format, so one undo
    /// takes it back (`PROSE-UI-252`). `None` when no write waits.
    pub fn take_page_write(&self) -> Option<crate::message::ProseCommand> {
        let format = self.page_write.get_untracked()?;
        self.page_write.set(None);
        Some(crate::message::ProseCommand::SetPageFormat { format })
    }

    /// Take in the history's availability from a Prose
    /// [`UndoState`](crate::events::ProseEvent::UndoState). Every other event
    /// leaves it as it was (`UNDO-006`).
    pub fn mirror_event<
        S: ambre_bus::Embed<crate::events::ProseEvent>,
        P: ambre_bus::ChannelPlane,
    >(
        &self,
        ev: &ambre_bus::UiEvent<S, P>,
    ) {
        if let Some(crate::events::ProseEvent::UndoState { undo, redo }) = ev.viewport_event() {
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
            style:position="fixed"
            style:top="0"
            style:right="8px"
            style:bottom="0"
            style:width=move || format!("{}px", PANEL_WIDTH.get())
            style:pointer-events="none"
            style:z-index="60"
            style:display=move || signals.threads.with(Vec::is_empty).then_some("none")
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
            style:position="absolute"
            style:top=move || format!(
                "{}px",
                entry.with(|e| e.as_ref().map_or(0.0, |e| {
                    e.y.on_page(signals.canvas_origin.get().y).get()
                }))
            )
            style:left="0"
            style:right="0"
            style:pointer-events="auto"
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
                    placeholder=ambre_commands::tr!("Reply", "R\u{e9}pondre")
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
                <button class="ambre-prose-thread-send" on:click=move |_| post()>{ambre_commands::tr!("Send", "Envoyer")}</button>
                <button
                    class="ambre-prose-thread-resolve"
                    on:click=move |_| {
                        // The user closes the thread: the draft goes with it.
                        take_draft();
                        send(AnnotationIntent::Resolve { comment_id });
                    }
                >
                    ambre_commands::tr!("Resolve", "R\u{e9}soudre")
                </button>
            </div>
        </div>
    }
}

/// The accept and reject buttons over the suggestion under the pointer, or
/// at the caret, and over no other (`PROSE-UI-148`, `PROSE-UI-272`).
#[component]
pub fn ProseSuggestionPopover(signals: ProseSignals) -> impl IntoView {
    move || {
        let origin = signals.canvas_origin.get();
        signals.hovered.get().map(|hovered| {
            let suggestion_id = hovered.suggestion_id;
            let (accept, reject) = match hovered.kind {
                SuggestionKind::Insertion => (
                    ambre_commands::tr!("Accept the insertion", "Accepter l'insertion"),
                    ambre_commands::tr!("Reject the insertion", "Rejeter l'insertion"),
                ),
                SuggestionKind::Deletion => (
                    ambre_commands::tr!("Accept the deletion", "Accepter la suppression"),
                    ambre_commands::tr!("Reject the deletion", "Rejeter la suppression"),
                ),
            };
            let left = format!("{}px", hovered.rect.x0.on_page(origin.x).get());
            let top = format!(
                "{}px",
                (hovered.rect.y0.on_page(origin.y) - POPOVER_LIFT)
                    .max(Px::<Css>::ZERO)
                    .get()
            );
            view! {
                <div
                    class="ambre-prose-suggestion"
                    style:position="fixed"
                    style:left=left
                    style:top=top
                    style:z-index="61"
                    role="group"
                >
                    <button
                        class="ambre-prose-suggestion-accept"
                        aria-label=accept
                        on:click=move |_| signals
                            .intent
                            .set(Some(AnnotationIntent::Accept { suggestion_id }))
                    >
                        ambre_commands::tr!("Accept", "Accepter")
                    </button>
                    <button
                        class="ambre-prose-suggestion-reject"
                        aria-label=reject
                        on:click=move |_| signals
                            .intent
                            .set(Some(AnnotationIntent::Reject { suggestion_id }))
                    >
                        ambre_commands::tr!("Reject", "Rejeter")
                    </button>
                </div>
            }
        })
    }
}

/// The corrections of the misspelled word a press opened, then an Ignore
/// button, under the word (`PROSE-UI-248`). Each button hands its pick back
/// to the view as one intent: the DOM holds no command channel.
#[component]
pub fn ProseSpellingPopover(signals: ProseSignals) -> impl IntoView {
    move || {
        let origin = signals.canvas_origin.get();
        signals.spelling.get().map(|open| {
            let (block_id, range) = (open.block_id, open.range);
            let left = format!("{}px", open.rect.x0.on_page(origin.x).get());
            let top = format!("{}px", open.rect.y1.on_page(origin.y).get() + 4.0);
            let choices = match open.suggestions {
                Offered::Waiting => view! {
                    <span class="ambre-prose-spelling-waiting">{ambre_commands::tr!("Looking up", "Recherche en cours")}</span>
                }
                .into_any(),
                Offered::Ready(found) if found.is_empty() => view! {
                    <span class="ambre-prose-spelling-none">{ambre_commands::tr!("No suggestion", "Aucune suggestion")}</span>
                }
                .into_any(),
                Offered::Ready(found) => found
                    .into_iter()
                    .map(|word| {
                        let label = word.as_str().to_owned();
                        view! {
                            <button
                                class="ambre-prose-spelling-pick"
                                on:click=move |_| signals.intent.set(Some(
                                    AnnotationIntent::Correct {
                                        block_id,
                                        range,
                                        replacement: word.clone(),
                                    },
                                ))
                            >
                                {label}
                            </button>
                        }
                    })
                    .collect_view()
                    .into_any(),
            };
            view! {
                <div
                    class="ambre-prose-spelling"
                    style:position="fixed"
                    style:left=left
                    style:top=top
                    style:z-index="61"
                    role="group"
                    aria-label=ambre_commands::tr!("Spelling", "Orthographe")
                >
                    {choices}
                    <button
                        class="ambre-prose-spelling-ignore"
                        on:click=move |_| signals
                            .intent
                            .set(Some(AnnotationIntent::Ignore { block_id, range }))
                    >
                        ambre_commands::tr!("Ignore", "Ignorer")
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
/// `PROSE-UI-098`). It publishes its reach into `cover` (`CHROME-171`).
#[component]
pub fn ProseFormatBar(
    signals: ProseSignals,
    cover: RwSignal<ambre_client_core::cover::Cover>,
) -> impl IntoView {
    // The text typed in the field of the bar. It outlives a rebuild of the
    // bar, which each new reading of the selection causes. `None` shows the
    // text the field opens on.
    let draft = RwSignal::new(None::<String>);
    move || {
        let bar = signals.toolbar.get();
        let Some(anchor) = bar.anchor else {
            // The selection went: a field it opened goes with it.
            signals.prompt.set(None);
            return None;
        };
        let anchor = anchor.on_page(signals.canvas_origin.get());
        let mut controls: Vec<FormatControl> = crate::commands::BAR_MARKS
            .map(|(id, key)| {
                let pressed = match bar.active.as_ref().map(|m| state_of(m, key)) {
                    Some(TriState::On) => Pressed::On,
                    Some(TriState::Mixed) => Pressed::Mixed,
                    _ => Pressed::Off,
                };
                control(signals, id, pressed)
            })
            .to_vec();
        let linked = bar.active.as_ref().is_some_and(|m| m.link.is_some());
        controls.push(control(
            signals,
            crate::commands::LINK,
            Pressed::from_bool(linked),
        ));
        let kind = bar
            .active
            .as_ref()
            .map_or(ambre_commands::Reading::Uniform(None), |m| m.values.kind);
        Some(view! {
            <FormatBar anchor=anchor controls=controls cover=cover>
                <span class="ambre-format-sep" aria-hidden="true"></span>
                {bar_kind_list(signals, kind)}
                {bar_prompt(signals, draft, bar.active.as_ref().and_then(|m| m.link.clone()))}
            </FormatBar>
        })
    }
}

/// The paragraph style list of the bar: body, three headings, quote
/// (`PROSE-UI-262`). A pick reaches the view as one write.
fn bar_kind_list(
    signals: ProseSignals,
    kind: ambre_commands::Reading<Option<crate::message::ParagraphKind>>,
) -> impl IntoView {
    use crate::view::panel::{BAR_KINDS, KIND, TextValue};
    let choices = BAR_KINDS
        .iter()
        .map(|(_, label)| ambre_chrome::ChoiceView::text(ambre_commands::shown(label)))
        .collect::<Vec<_>>();
    let selected = match kind {
        ambre_commands::Reading::Uniform(Some(kind)) => BAR_KINDS
            .iter()
            .position(|(k, _)| *k == kind)
            .map(Pos::<ambre_chrome::ChoiceRow>::from_usize),
        _ => None,
    };
    let pick = Callback::new(move |p: Pos<ambre_chrome::ChoiceRow>| {
        if let Some((kind, _)) = p.to_index().and_then(|i| BAR_KINDS.get(i)) {
            signals.pick_text_write(TextValue::Kind(*kind));
        }
    });
    view! {
        <ambre_chrome::Select
            label=ambre_commands::tr!("Paragraph style", "Style de paragraphe")
            choices=choices
            selected=Signal::stored(selected)
            on_pick=pick
            placeholder=ambre_commands::tr!("Heading", "Titre")
            reach=ambre_commands::Reach::Property(KIND)
        />
    }
}

/// The field a link press or the comment of the creation bar opened, under
/// the bar. Enter hands
/// its text to the view; Escape closes it. An empty address takes the link
/// off (`PROSE-UI-188`).
fn bar_prompt(
    signals: ProseSignals,
    draft: RwSignal<Option<String>>,
    link: Option<ambre_core::LinkUrl>,
) -> impl IntoView {
    use crate::view::toolbar::BarPrompt;
    move || {
        let Some(prompt) = signals.prompt.get() else {
            draft.set(None);
            return None;
        };
        let (placeholder, start) = match prompt {
            BarPrompt::Link => (
                ambre_commands::tr!(
                    "Paste or type an address",
                    "Collez ou saisissez une adresse"
                ),
                link.as_ref()
                    .map(|l| l.as_str().to_owned())
                    .unwrap_or_default(),
            ),
            BarPrompt::Comment => (
                ambre_commands::tr!("Write a comment", "\u{c9}crivez un commentaire"),
                String::new(),
            ),
        };
        let field = NodeRef::<leptos::html::Input>::new();
        Effect::new(move |_| {
            if let Some(input) = field.get() {
                let _ = input.focus();
            }
        });
        let shown = draft.get_untracked().unwrap_or(start);
        let submit = move |text: String| {
            draft.set(None);
            signals.prompt.set(None);
            match prompt {
                BarPrompt::Link => signals.pick_text_write(crate::view::panel::TextValue::Link(
                    Some(text)
                        .filter(|t| !t.trim().is_empty())
                        .map(ambre_core::LinkUrl::new),
                )),
                BarPrompt::Comment => signals.intent.set(Some(AnnotationIntent::Comment { text })),
            }
        };
        Some(view! {
            <input
                node_ref=field
                type="text"
                class="ambre-format-prompt"
                aria-label=placeholder
                placeholder=placeholder
                prop:value=shown
                on:input=move |ev| draft.set(Some(event_target_value(&ev)))
                on:keydown=move |ev: web_sys::KeyboardEvent| match ev.key().as_str() {
                    "Enter" => {
                        ev.prevent_default();
                        submit(event_target_value(&ev));
                    }
                    "Escape" => {
                        ev.prevent_default();
                        draft.set(None);
                        signals.prompt.set(None);
                    }
                    _ => {}
                }
            />
        })
    }
}

/// The glyph a command of the page declares, resolved to its asset.
#[must_use]
pub fn icon_of(command: &ambre_commands::Command) -> Option<icondata::Icon> {
    Some(match command.icon?.as_str() {
        "format-bold" => icondata::BsTypeBold,
        "format-italic" => icondata::BsTypeItalic,
        "format-underline" => icondata::BsTypeUnderline,
        "format-strikethrough" => icondata::BsTypeStrikethrough,
        "code-tags" => icondata::MdiCodeTags,
        "format-superscript" => icondata::MdiFormatSuperscript,
        "format-subscript" => icondata::MdiFormatSubscript,
        "format-clear" => icondata::MdiFormatClear,
        "file-document-edit-outline" => icondata::MdiFileDocumentEditOutline,
        "format-text" => icondata::MdiFormatText,
        "image-outline" => icondata::MdiImageOutline,
        "table-plus" => icondata::MdiTablePlus,
        "border-horizontal" => icondata::MdiBorderHorizontal,
        "link" => icondata::MdiLink,
        "comment-outline" => icondata::MdiCommentOutline,
        _ => return None,
    })
}

/// The suggestion-mode chip of the status bar, shown while this session
/// suggests (`PROSE-UI-275`). The context panel folds, and the status bar
/// never does, so the mode stays in view. The chip is a reach of the
/// command, never a second home: a press turns the mode off.
#[component]
pub fn ProseModeChip(signals: ProseSignals) -> impl IntoView {
    let command = ambre_commands::find(crate::commands::COMMANDS, crate::commands::SUGGEST_CHANGES);
    let on_command = Callback::new(move |id| {
        if let Some(intent) = crate::view::annotate::review_intent(id, None) {
            signals.intent.set(Some(intent));
        }
    });
    move || {
        let command = command?;
        (signals.mode.get() == crate::events::EditMode::Suggesting).then(|| {
            view! {
                <span class="statusbar-mode" role="status">
                    <ambre_chrome::CommandButton
                        command=command
                        icon=icon_of(command)
                        on_command=on_command
                        pressed=Signal::derive(|| Pressed::On)
                        labelled=ambre_chrome::Labelled::Text
                    />
                </span>
            }
        })
    }
}

/// One control, announced by its command's title, chord and description
/// (`CHROME-186`). An id the table does not describe shows an empty label.
fn control(signals: ProseSignals, id: StaticKey<Commands>, pressed: Pressed) -> FormatControl {
    let on_press = Callback::new(move |()| signals.format.set(Some(id)));
    match ambre_commands::find(crate::commands::COMMANDS, id) {
        Some(command) => FormatControl::of(command, icon_of(command), pressed, on_press),
        None => FormatControl {
            label: String::new(),
            pressed,
            on_press,
            icon: None,
            chord: None,
            description: None,
            reach: None,
        },
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
        let left = format!("{}px", at.x.get());
        let top = format!("{}px", (at.y + MENU_DROP).get());
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
            <ul
                class="ambre-prose-slash-menu"
                style:position="fixed"
                style:left=left
                style:top=top
                style:z-index="62"
                role="listbox"
                aria-label=ambre_commands::tr!("Insert", "Ins\u{e9}rer")
            >
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
        let left = format!("{}px", ruler.page_left.on_page(origin.x).get());
        let top = format!("{}px", origin.y.get());
        let width = format!("{}px", ruler.content_width.get());
        let height = format!("{}px", RULER_HEIGHT.get());
        let handles = ruler.visible.as_bool().then(|| {
            [
                (
                    RulerHandle::IndentFirst,
                    ambre_commands::tr!("First-line indent", "Retrait de premi\u{e8}re ligne"),
                    "first",
                ),
                (
                    RulerHandle::IndentLeft,
                    ambre_commands::tr!("Left indent", "Retrait gauche"),
                    "left",
                ),
                (
                    RulerHandle::IndentRight,
                    ambre_commands::tr!("Right indent", "Retrait droit"),
                    "right",
                ),
            ]
            .map(|(handle, label, side)| {
                let x = move || {
                    drag.get()
                        .filter(|d| d.handle == handle)
                        .map_or_else(|| ruler.handle_x(handle), |d| d.at)
                        - ruler.page_left
                };
                // The sheet draws the marker from the tokens, and its hover and
                // drag states (`PROSE-UI-257`).
                let dragging = move || {
                    drag.get()
                        .is_some_and(|d| d.handle == handle)
                        .then_some("true")
                };
                view! {
                    <div
                        class="ambre-prose-ruler-handle"
                        role="slider"
                        aria-label=label
                        data-handle=side
                        data-dragging=dragging
                        style:position="absolute"
                        style:left=move || format!("{}px", x().get())
                        style:transform="translateX(-50%)"
                        style:touch-action="none"
                        style:cursor="ew-resize"
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
                    ></div>
                }
            })
            .into_iter()
            .collect_view()
        });
        Some(view! {
            <div
                class="ambre-prose-ruler"
                style:position="fixed"
                style:left=left
                style:top=top
                style:width=width
                style:height=height
                style:z-index="59"
                role="group"
                aria-label=ambre_commands::tr!("Ruler", "R\u{e8}gle")
            >
                {handles}
            </div>
        })
    }
}
