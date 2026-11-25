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
use ambre_client_core::{PanelApplication, PanelFold, PanelSide, SettingChange, UserSettings};
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
///
/// The side and the fold follow the account once its settings answer
/// (`SETTINGS-041`). Until then the device's own words hold them, so the panel
/// does not move at boot. A change the person makes is written to the account
/// through `outbox`. A value that came from the account is never written back.
pub(crate) fn panel_prefs(
    kind: DocType,
    account: RwSignal<Option<UserSettings>>,
    outbox: crate::outbox::ShellOutbox,
) -> PanelPrefs {
    let prefs = PanelPrefs::new(
        DockSide::from_stored(read(SIDE_KEY).as_deref()),
        PanelWidth::from_stored(read(WIDTH_KEY).as_deref()),
        Fold::from_stored(read(fold_key(kind)).as_deref()),
    );
    let PanelPrefs { side, width, fold } = prefs;
    let application = application_of(kind);
    // What the account holds, as the panel last applied it.
    let applied = StoredValue::new(None::<(DockSide, Fold)>);
    Effect::new(move |_| {
        let Some(settings) = account.get() else {
            return;
        };
        let held = (
            dock_side(settings.panel_side),
            application.map_or(fold.get_untracked(), |app| {
                panel_fold(settings.panel_fold(app))
            }),
        );
        applied.set_value(Some(held));
        side.set(held.0);
        fold.set(held.1);
    });
    let outbox = StoredValue::new_local(outbox);
    Effect::new(move |_| {
        let now = (side.get(), fold.get());
        keep(SIDE_KEY, now.0.as_str());
        keep(fold_key(kind), now.1.as_str());
        let Some(held) = applied.get_value() else {
            return;
        };
        let mut changes = Vec::new();
        if now.0 != held.0 {
            changes.push(SettingChange::PanelSide(panel_side(now.0)));
        }
        if let Some(app) = application
            && now.1 != held.1
        {
            changes.push(SettingChange::PanelFold(app, panel_fold_of(now.1)));
        }
        applied.set_value(Some(now));
        outbox.with_value(|outbox| {
            for change in changes {
                outbox.send(ambre_logic::app::LogicCommand::ChangeSetting { change });
            }
        });
    });
    Effect::new(move |_| keep(WIDTH_KEY, &width.get().to_stored()));
    prefs
}

/// The application whose fold the account keeps, or `None` for a page that
/// lays out no context panel.
const fn application_of(kind: DocType) -> Option<PanelApplication> {
    match kind {
        DocType::Prose => Some(PanelApplication::Prose),
        DocType::Trame => Some(PanelApplication::Trame),
        DocType::Toile => Some(PanelApplication::Toile),
        DocType::Murmure | DocType::Settings => None,
    }
}

/// The side the panel docks to for the side the account holds.
const fn dock_side(side: PanelSide) -> DockSide {
    match side {
        PanelSide::Left => DockSide::Left,
        PanelSide::Right => DockSide::Right,
    }
}

/// The side the account holds for the side the panel docks to.
const fn panel_side(side: DockSide) -> PanelSide {
    match side {
        DockSide::Left => PanelSide::Left,
        DockSide::Right => PanelSide::Right,
    }
}

/// The fold the panel shows for the fold the account holds.
const fn panel_fold(fold: PanelFold) -> Fold {
    match fold {
        PanelFold::Open => Fold::Open,
        PanelFold::Folded => Fold::Rail,
    }
}

/// The fold the account holds for the fold the panel shows.
const fn panel_fold_of(fold: Fold) -> PanelFold {
    match fold {
        Fold::Open => PanelFold::Open,
        Fold::Rail => PanelFold::Folded,
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

    /// The prefs of `kind` on a page whose account has not answered, its
    /// writes dropped.
    fn prefs(kind: DocType) -> PanelPrefs {
        let (tx, rx) = ambre_bus::conduit::local(ambre_core::Bounded::<
            ambre_bus::conduit::ChannelSlots,
        >::of::<8>());
        std::mem::forget(rx);
        panel_prefs(
            kind,
            RwSignal::new(None),
            crate::outbox::ShellOutbox::new(tx),
        )
    }

    /// `SETTINGS-041`: the side and the fold follow the account once it
    /// answers. A change of the person reaches the account, and a value of
    /// the account is never written back.
    #[wasm_bindgen_test]
    async fn the_side_and_the_fold_follow_the_account() {
        clear();
        let (tx, rx) = ambre_bus::conduit::local(ambre_core::Bounded::<
            ambre_bus::conduit::ChannelSlots,
        >::of::<8>());
        let account = RwSignal::new(None);
        let outbox = crate::outbox::ShellOutbox::new(tx);
        let trame = owned(move || panel_prefs(DocType::Trame, account, outbox)).await;
        let changes = || -> Vec<SettingChange> {
            rx.try_iter()
                .filter_map(|command| match command {
                    ambre_logic::app::LogicCommand::ChangeSetting { change } => Some(change),
                    _ => None,
                })
                .collect()
        };
        trame.side.set(DockSide::Left);
        leptos::task::tick().await;
        assert!(changes().is_empty(), "no account answered yet");
        account.set(Some(UserSettings {
            panel_fold_trame: PanelFold::Folded,
            ..UserSettings::default()
        }));
        leptos::task::tick().await;
        assert_eq!(trame.side.get_untracked(), DockSide::Right);
        assert_eq!(trame.fold.get_untracked(), Fold::Rail);
        assert!(changes().is_empty(), "the account is not written back");
        trame.fold.set(Fold::Open);
        leptos::task::tick().await;
        assert_eq!(
            changes(),
            vec![SettingChange::PanelFold(
                PanelApplication::Trame,
                PanelFold::Open
            )]
        );
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
        let prose = owned(|| prefs(DocType::Prose)).await;
        assert_eq!(
            prose.side.get_untracked(),
            DockSide::Right,
            "the right by default"
        );
        prose.side.set(DockSide::Left);
        leptos::task::tick().await;
        for kind in [DocType::Trame, DocType::Toile] {
            let other = owned(move || prefs(kind)).await;
            assert_eq!(other.side.get_untracked(), DockSide::Left);
        }
    }

    #[wasm_bindgen_test]
    async fn a_fold_in_one_application_leaves_the_others_open() {
        clear();
        let trame = owned(|| prefs(DocType::Trame)).await;
        trame.fold.set(Fold::Rail);
        leptos::task::tick().await;
        for kind in [DocType::Prose, DocType::Toile] {
            let other = owned(move || prefs(kind)).await;
            assert_eq!(other.fold.get_untracked(), Fold::Open);
        }
    }

    #[wasm_bindgen_test]
    async fn the_panel_opens_as_it_was_last_left_in_that_application() {
        clear();
        let first = owned(|| prefs(DocType::Toile)).await;
        first.fold.set(Fold::Rail);
        first
            .width
            .set(PanelWidth::clamped(ambre_core::Px::<ambre_core::Css>::new(
                420.0,
            )));
        leptos::task::tick().await;
        let again = owned(|| prefs(DocType::Toile)).await;
        assert_eq!(again.fold.get_untracked(), Fold::Rail);
        assert_eq!(again.width.get_untracked(), first.width.get_untracked());
        again.fold.set(Fold::Open);
        leptos::task::tick().await;
        let third = owned(|| prefs(DocType::Toile)).await;
        assert_eq!(third.fold.get_untracked(), Fold::Open);
    }
}
