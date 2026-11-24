// ambre-app/src/shell/panel.rs
// The view state of the context panel, kept on this device
// (`ambre-spec-shell.md` section 5). The side is one setting for the three
// applications (`CHROME-194`), as the width is. The fold is one per
// application (`CHROME-195`, `CHROME-196`).
//
// A store the browser refuses reads as the defaults and keeps nothing, so a
// private window or a full quota still boots the panel.

use ambre_bus::DocType;
use ambre_chrome::{DockSide, Fold, PanelPrefs, PanelWidth};
use ambre_core::{StaticKey, StorageKeys};
use leptos::prelude::*;

/// The side of the panel, shared by Prose, Trame and Toile.
const SIDE_KEY: StaticKey<StorageKeys> = StaticKey::new("ambre.panel.side");

/// The width of the open panel, shared as the side is.
const WIDTH_KEY: StaticKey<StorageKeys> = StaticKey::new("ambre.panel.width");

/// The fold of the panel in one application.
fn fold_key(kind: DocType) -> StaticKey<StorageKeys> {
    match kind {
        DocType::Prose => StaticKey::new("ambre.panel.fold.prose"),
        DocType::Trame => StaticKey::new("ambre.panel.fold.trame"),
        DocType::Toile => StaticKey::new("ambre.panel.fold.toile"),
        DocType::Murmure => StaticKey::new("ambre.panel.fold.murmure"),
        // No route boots the settings document, so no panel reads this key.
        DocType::Settings => StaticKey::new("ambre.panel.fold.settings"),
    }
}

/// The word stored under `key` on this device, or `None`.
fn read(key: StaticKey<StorageKeys>) -> Option<String> {
    crate::platform::stored_word(key)
}

/// Keep `word` under `key` on this device. A refused write keeps nothing,
/// and the panel goes on with the value it shows.
fn keep(key: StaticKey<StorageKeys>, word: &str) {
    crate::platform::keep_word(key, word);
}

/// The prefs of the panel of `kind`, read from this device, each kept back
/// whenever it changes. Called once per page, inside the reactive owner of
/// the shell.
pub(crate) fn panel_prefs(kind: DocType) -> PanelPrefs {
    let prefs = PanelPrefs::new(
        DockSide::from_stored(read(SIDE_KEY).as_deref()),
        PanelWidth::from_stored(read(WIDTH_KEY).as_deref()),
        Fold::from_stored(read(fold_key(kind)).as_deref()),
    );
    let PanelPrefs { side, width, fold } = prefs;
    Effect::new(move |_| keep(SIDE_KEY, side.get().as_str()));
    Effect::new(move |_| keep(WIDTH_KEY, &width.get().to_stored()));
    Effect::new(move |_| keep(fold_key(kind), fold.get().as_str()));
    prefs
}

/// The control that moves the panel to the other side
/// (`ambre-spec-shell.md` section 2). It sits beside the theme switch, the
/// one place of the header that holds a setting of the person.
#[component]
pub(crate) fn DockSideSwitch(side: RwSignal<DockSide>) -> impl IntoView {
    let label = move || match side.get() {
        DockSide::Right => "Dock the panel to the left",
        DockSide::Left => "Dock the panel to the right",
    };
    view! {
        <button
            class="icon ambre-dock-side"
            title=label
            aria-label=label
            on:click=move |_| side.update(|s| *s = s.flipped())
        >
            {move || {
                let icon = match side.get() {
                    DockSide::Right => icondata::LuPanelRight,
                    DockSide::Left => icondata::LuPanelLeft,
                };
                view! { <leptos_icons::Icon icon=icon /> }
            }}
        </button>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    /// Run `f` inside a mounted view, which starts the executor the effects
    /// run on, then let the effects write.
    async fn owned<T: 'static>(f: impl FnOnce() -> T + 'static) -> T {
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let put = slot.clone();
        let doc = web_sys::window().unwrap().document().unwrap();
        let host = doc.create_element("div").unwrap();
        doc.body().unwrap().append_child(&host).unwrap();
        let host: web_sys::HtmlElement = leptos::wasm_bindgen::JsCast::dyn_into(host).unwrap();
        leptos::mount::mount_to(host, move || {
            *put.borrow_mut() = Some(f());
        })
        .forget();
        leptos::task::tick().await;
        slot.borrow_mut().take().unwrap()
    }

    fn clear() {
        let s = web_sys::window().unwrap().local_storage().unwrap().unwrap();
        for key in [SIDE_KEY, WIDTH_KEY] {
            s.remove_item(key.as_str()).unwrap();
        }
        for kind in [DocType::Prose, DocType::Trame, DocType::Toile] {
            s.remove_item(fold_key(kind).as_str()).unwrap();
        }
    }

    #[wasm_bindgen_test]
    async fn the_side_is_one_setting_for_the_three_applications() {
        clear();
        let prose = owned(|| panel_prefs(DocType::Prose)).await;
        assert_eq!(
            prose.side.get_untracked(),
            DockSide::Right,
            "the right by default"
        );
        prose.side.set(DockSide::Left);
        leptos::task::tick().await;
        for kind in [DocType::Trame, DocType::Toile] {
            let other = owned(move || panel_prefs(kind)).await;
            assert_eq!(other.side.get_untracked(), DockSide::Left);
        }
    }

    #[wasm_bindgen_test]
    async fn a_fold_in_one_application_leaves_the_others_open() {
        clear();
        let trame = owned(|| panel_prefs(DocType::Trame)).await;
        trame.fold.set(Fold::Rail);
        leptos::task::tick().await;
        for kind in [DocType::Prose, DocType::Toile] {
            let other = owned(move || panel_prefs(kind)).await;
            assert_eq!(other.fold.get_untracked(), Fold::Open);
        }
    }

    #[wasm_bindgen_test]
    async fn the_panel_opens_as_it_was_last_left_in_that_application() {
        clear();
        let first = owned(|| panel_prefs(DocType::Toile)).await;
        first.fold.set(Fold::Rail);
        first
            .width
            .set(PanelWidth::clamped(ambre_core::Px::<ambre_core::Css>::new(
                420.0,
            )));
        leptos::task::tick().await;
        let again = owned(|| panel_prefs(DocType::Toile)).await;
        assert_eq!(again.fold.get_untracked(), Fold::Rail);
        assert_eq!(again.width.get_untracked(), first.width.get_untracked());
        again.fold.set(Fold::Open);
        leptos::task::tick().await;
        let third = owned(|| panel_prefs(DocType::Toile)).await;
        assert_eq!(third.fold.get_untracked(), Fold::Open);
    }
}
