// ambre-chrome/src/context_panel.rs
// ============================================================================
// The context panel: one frame for Prose, Trame and Toile
// (`ambre-spec-shell.md` section 2, `CHROME-190`).
//
// The frame is the shell's. A header names the selection, sections fold, and
// the panel collapses to a rail of the section icons. An application hands the
// frame its sections and writes their content alone, from the kit (`kit`).
//
// Collapsing hides the body and keeps it in the page, so the open sections,
// the scroll and an edit in progress outlive a collapse (`CHROME-177`). The
// frame writes nothing to a document: it holds view state only.
//
// The side, the width and the fold are signals the shell hands in. The frame
// never touches a store: the shell reads and keeps them on the device
// (`ambre-spec-shell.md` section 5), and a store that fails still boots.
// ============================================================================

use std::collections::HashSet;

use ambre_commands::{KeyBinding, shared};
use ambre_core::Flag as _;
use ambre_core::{Css, PanelSections, Px, StaticKey};
use leptos::prelude::*;
use leptos::wasm_bindgen::JsCast;

/// The side of the window the context panel docks to (`CHROME-193`). One
/// setting for the three applications (`CHROME-194`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DockSide {
    /// After the canvas: the default.
    #[default]
    Right,
    /// Before the canvas.
    Left,
}

impl DockSide {
    /// The word the stylesheet reads off `data-side`, and the store keeps.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Right => "right",
            Self::Left => "left",
        }
    }

    /// The side a stored word names. An unknown word or no word is the
    /// default, so a damaged store still boots.
    pub fn from_stored(word: Option<&str>) -> Self {
        match word {
            Some("left") => Self::Left,
            _ => Self::Right,
        }
    }

    /// The other side.
    #[must_use]
    pub fn flipped(self) -> Self {
        match self {
            Self::Right => Self::Left,
            Self::Left => Self::Right,
        }
    }
}

/// Whether the panel shows its sections or its rail alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fold {
    /// The header and the sections show.
    #[default]
    Open,
    /// The rail of the section icons shows alone.
    Rail,
}

impl Fold {
    /// The word the stylesheet reads off `data-fold`, and the store keeps.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Rail => "rail",
        }
    }

    /// The fold a stored word names. No word, or an unknown one, is open.
    pub fn from_stored(word: Option<&str>) -> Self {
        match word {
            Some("rail") => Self::Rail,
            _ => Self::Open,
        }
    }

    /// The other fold.
    #[must_use]
    pub fn toggled(self) -> Self {
        match self {
            Self::Open => Self::Rail,
            Self::Rail => Self::Open,
        }
    }
}

/// The width of the open panel, held between the narrowest and the widest
/// width of `ambre-spec-shell.md` section 2 (`CHROME-197`).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct PanelWidth(Px<Css>);

impl PanelWidth {
    /// The narrowest width.
    pub const MIN: Self = Self(Px::<Css>::new(240.0));
    /// The widest width.
    pub const MAX: Self = Self(Px::<Css>::new(480.0));
    /// The width of a first boot.
    pub const DEFAULT: Self = Self(Px::<Css>::new(300.0));

    /// The width `px` asks for, held between the two bounds.
    #[must_use]
    pub fn clamped(px: Px<Css>) -> Self {
        Self(Px::<Css>::new(
            px.get().clamp(Self::MIN.0.get(), Self::MAX.0.get()),
        ))
    }

    /// The width in CSS pixels.
    #[must_use]
    pub fn get(self) -> Px<Css> {
        self.0
    }

    /// The width a stored number names, held between the bounds. A word that
    /// is no number is the default, so a damaged store still boots.
    pub fn from_stored(word: Option<&str>) -> Self {
        word.and_then(|w| w.trim().parse::<f32>().ok())
            .filter(|v| v.is_finite())
            .map_or(Self::DEFAULT, |v| Self::clamped(Px::<Css>::new(v)))
    }

    /// The number the store keeps, in whole CSS pixels.
    pub fn to_stored(self) -> String {
        format!("{:.0}", self.0.get())
    }
}

impl Default for PanelWidth {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The view state of one panel, which the shell owns and keeps on the
/// device. `Copy`: each field is a signal.
#[derive(Debug, Clone, Copy)]
pub struct PanelPrefs {
    /// The side, shared by the three applications.
    pub side: RwSignal<DockSide>,
    /// The width of the open panel.
    pub width: RwSignal<PanelWidth>,
    /// The fold, per application (`CHROME-195`).
    pub fold: RwSignal<Fold>,
}

impl PanelPrefs {
    /// Prefs from three values, as the shell read them.
    pub fn new(side: DockSide, width: PanelWidth, fold: Fold) -> Self {
        Self {
            side: RwSignal::new(side),
            width: RwSignal::new(width),
            fold: RwSignal::new(fold),
        }
    }
}

/// One section of the panel: an icon and a label for the rail and the
/// header of the section, and the content its application writes.
#[derive(Clone)]
pub struct PanelSection {
    /// The key the rail and the fold state name the section by.
    pub key: StaticKey<PanelSections>,
    /// The label a person reads in the header of the section.
    pub label: &'static str,
    /// The icon of the section on the rail (`CHROME-191`).
    pub icon: icondata::Icon,
    /// The content, written by the application from the kit.
    pub body: ViewFn,
}

impl PanelSection {
    /// A section of `key`, its label, its icon and its content.
    pub fn new(
        key: StaticKey<PanelSections>,
        label: &'static str,
        icon: icondata::Icon,
        body: impl Fn() -> AnyView + Send + Sync + 'static,
    ) -> Self {
        Self {
            key,
            label,
            icon,
            body: ViewFn::from(body),
        }
    }
}

/// What the panel shows now: a title that names the selection, and the
/// sections the booted application writes for it. Each application fills
/// both from the selection its view publishes.
#[derive(Clone, Copy)]
pub struct PanelContent {
    /// The header of the panel.
    pub title: Signal<String>,
    /// The sections, in the order the panel shows them.
    pub sections: Signal<Vec<PanelSection>>,
}

impl PanelContent {
    /// A panel that names nothing and shows no section.
    pub fn empty() -> Self {
        Self {
            title: Signal::stored(String::new()),
            sections: Signal::stored(Vec::new()),
        }
    }
}

/// How far one arrow key moves the edge of the panel.
const KEY_STEP: Px<Css> = Px::<Css>::new(16.0);

/// The chord the tooltips of the frame name: the first of the toggle chords.
const PANEL_CHORD: KeyBinding = KeyBinding::primary(ambre_commands::KeyChar::new('.'));

/// The icon of the button that leaves `fold`, on `side`: it points the way
/// the panel moves.
fn side_icon(side: DockSide, fold: Fold) -> icondata::Icon {
    match (side, fold) {
        (DockSide::Right, Fold::Open) | (DockSide::Left, Fold::Rail) => icondata::LuPanelRightClose,
        (DockSide::Left, Fold::Open) | (DockSide::Right, Fold::Rail) => icondata::LuPanelRightOpen,
    }
}

/// Whether `chord` collapses or opens the panel.
fn is_toggle(chord: KeyBinding) -> bool {
    shared::PANEL_TOGGLE_CHORDS.contains(&chord)
}

/// The context panel. `title` names the selection, and `sections` lists the
/// sections it shows now; both follow the selection. `prefs` holds the side,
/// the width and the fold the shell keeps.
#[component]
pub fn ContextPanel(
    prefs: PanelPrefs,
    #[prop(into)] title: Signal<String>,
    #[prop(into)] sections: Signal<Vec<PanelSection>>,
) -> impl IntoView {
    let PanelPrefs { side, width, fold } = prefs;
    // The sections a person folded. A key outlives the section it names, so a
    // section that leaves and comes back keeps its fold.
    let folded = RwSignal::new(HashSet::<StaticKey<PanelSections>>::new());
    let panel_ref = NodeRef::<leptos::html::Aside>::new();

    // `Ctrl+.` collapses an open panel and opens a collapsed one
    // (`CHROME-175`, `CHROME-176`).
    let keys = window_event_listener(leptos::ev::keydown, move |ev| {
        if crate::keybinding_from_event(&ev).is_some_and(is_toggle) {
            ev.prevent_default();
            fold.update(|f| *f = f.toggled());
        }
    });
    on_cleanup(move || keys.remove());

    // Open the panel at one section: unfold it, then bring it into view
    // (`CHROME-192`).
    let open_at = move |key: StaticKey<PanelSections>| {
        folded.update(|f| {
            f.remove(&key);
        });
        fold.set(Fold::Open);
        if let Some(panel) = panel_ref.get_untracked() {
            let selector = format!(".ambre-ctx-section[data-section=\"{}\"]", key.as_str());
            if let Ok(Some(section)) = panel.query_selector(&selector) {
                section.scroll_into_view();
            }
        }
    };

    // A drag of the inner edge sets the width (`CHROME-197`).
    let dragging = StoredValue::new(None::<i32>);
    let on_edge_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        if let Some(target) = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
        {
            let _ = target.set_pointer_capture(ev.pointer_id());
        }
        dragging.set_value(Some(ev.pointer_id()));
    };
    let on_edge_move = move |ev: web_sys::PointerEvent| {
        if dragging.get_value() != Some(ev.pointer_id()) {
            return;
        }
        let Some(panel) = panel_ref.get_untracked() else {
            return;
        };
        let rect = panel.get_bounding_client_rect();
        let x = f64::from(ev.client_x());
        let asked = match side.get_untracked() {
            DockSide::Right => rect.right() - x,
            DockSide::Left => x - rect.left(),
        };
        width.set(PanelWidth::clamped(Px::<Css>::new(asked as f32)));
    };
    let on_edge_up = move |_: web_sys::PointerEvent| dragging.set_value(None);
    let on_edge_key = move |ev: web_sys::KeyboardEvent| {
        // The arrow toward the canvas narrows the panel, whatever its side.
        let toward_canvas = match side.get_untracked() {
            DockSide::Right => "ArrowRight",
            DockSide::Left => "ArrowLeft",
        };
        let away = match side.get_untracked() {
            DockSide::Right => "ArrowLeft",
            DockSide::Left => "ArrowRight",
        };
        let step = match ev.key().as_str() {
            k if k == away => KEY_STEP.get(),
            k if k == toward_canvas => -KEY_STEP.get(),
            _ => return,
        };
        ev.prevent_default();
        width.update(|w| *w = PanelWidth::clamped(Px::<Css>::new(w.get().get() + step)));
    };

    let rail = move || {
        sections
            .get()
            .into_iter()
            .map(|section| {
                let key = section.key;
                view! {
                    <crate::IconButton
                        icon=section.icon
                        label=section.label
                        size=crate::ControlSize::Large
                                                on_press=Callback::new(move |()| open_at(key))
                        {..}
                        class:ambre-ctx-rail-icon=true
                        data-section=key.as_str()
                    />
                }
            })
            .collect_view()
    };

    view! {
        <aside
            class="ambre-ctx-panel"
            node_ref=panel_ref
            aria-label="Context panel"
            data-side=move || side.get().as_str()
            data-fold=move || fold.get().as_str()
            style:--ctx-panel-w=move || format!("{}px", width.get().get().get())
        >
            <div
                class="ambre-ctx-edge"
                role="separator"
                aria-orientation="vertical"
                aria-label="Panel width"
                aria-valuemin=PanelWidth::MIN.get().get()
                aria-valuemax=PanelWidth::MAX.get().get()
                aria-valuenow=move || width.get().get().get()
                tabindex="0"
                on:pointerdown=on_edge_down
                on:pointermove=on_edge_move
                on:pointerup=on_edge_up
                on:pointercancel=on_edge_up
                on:keydown=on_edge_key
            ></div>
            <nav class="ambre-ctx-rail" aria-label="Panel sections">
                {move || {
                    view! {
                        <crate::IconButton
                            icon=side_icon(side.get(), Fold::Rail)
                            label="Open the panel"
                            chord=Some(PANEL_CHORD)
                            size=crate::ControlSize::Large
                                                        on_press=Callback::new(move |()| fold.set(Fold::Open))
                            {..}
                            class:ambre-ctx-open=true
                        />
                    }
                }}
                {rail}
            </nav>
            <div class="ambre-ctx-body">
                <header class="ambre-ctx-head">
                    <h2 class="ambre-ctx-title">{move || title.get()}</h2>
                    {move || {
                        view! {
                            <crate::IconButton
                                icon=side_icon(side.get(), Fold::Open)
                                label="Collapse the panel"
                                chord=Some(PANEL_CHORD)
                                                                on_press=Callback::new(move |()| fold.set(Fold::Rail))
                                {..}
                                class:ambre-ctx-collapse=true
                            />
                        }
                    }}
                </header>
                <div class="ambre-ctx-sections">
                    <For
                        each=move || sections.get()
                        key=|section| section.key
                        children=move |section| {
                            let key = section.key;
                            let is_folded = move || folded.with(|f| f.contains(&key));
                            view! {
                                <section
                                    class="ambre-ctx-section"
                                    data-section=key.as_str()
                                    data-folded=move || if is_folded() { "true" } else { "false" }
                                >
                                    <button
                                        class="ambre-ctx-section-head"
                                        aria-expanded=move || crate::Openness::from_bool(!is_folded()).aria()
                                        on:mousedown=move |ev| ev.prevent_default()
                                        on:click=move |_| folded.update(|f| {
                                            if !f.remove(&key) {
                                                f.insert(key);
                                            }
                                        })
                                    >
                                        <leptos_icons::Icon icon=section.icon />
                                        <span>{section.label}</span>
                                    </button>
                                    <div class="ambre-ctx-section-body">{section.body.run()}</div>
                                </section>
                            }
                        }
                    />
                </div>
            </div>
        </aside>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_width_is_held_between_the_two_bounds() {
        assert_eq!(PanelWidth::clamped(Px::<Css>::new(100.0)), PanelWidth::MIN);
        assert_eq!(PanelWidth::clamped(Px::<Css>::new(900.0)), PanelWidth::MAX);
        assert_eq!(
            PanelWidth::clamped(Px::<Css>::new(333.0)).get(),
            Px::<Css>::new(333.0)
        );
    }

    #[test]
    fn a_damaged_store_reads_as_the_defaults() {
        assert_eq!(PanelWidth::from_stored(None), PanelWidth::DEFAULT);
        assert_eq!(PanelWidth::from_stored(Some("wide")), PanelWidth::DEFAULT);
        assert_eq!(PanelWidth::from_stored(Some("NaN")), PanelWidth::DEFAULT);
        assert_eq!(PanelWidth::from_stored(Some("9000")), PanelWidth::MAX);
        assert_eq!(DockSide::from_stored(Some("up")), DockSide::Right);
        assert_eq!(Fold::from_stored(Some("")), Fold::Open);
    }

    #[test]
    fn each_pref_reads_back_what_it_stored() {
        let w = PanelWidth::clamped(Px::<Css>::new(412.0));
        assert_eq!(PanelWidth::from_stored(Some(&w.to_stored())), w);
        for side in [DockSide::Left, DockSide::Right] {
            assert_eq!(DockSide::from_stored(Some(side.as_str())), side);
        }
        for fold in [Fold::Open, Fold::Rail] {
            assert_eq!(Fold::from_stored(Some(fold.as_str())), fold);
        }
    }

    #[test]
    fn the_panel_chord_is_ctrl_period() {
        assert!(is_toggle(KeyBinding::primary(
            ambre_commands::KeyChar::new('.')
        )));
        assert!(!is_toggle(KeyBinding::primary(ambre_commands::KeyChar::K)));
    }
}
