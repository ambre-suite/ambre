// ambre-app/src/host/murmure.rs
// The page's side of a Murmure message region. The view lives in ambre-murmure
// and reads the page through closures. They give the pane, a member's name,
// and the time in the reader's language and zone.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use ambre_bus::{Attachment, DraftKey, RowPlace, SealedRow, Timestamp, UserId};
use ambre_canvas::TextInput;
use ambre_logic::persistence::PersistenceStore;
use ambre_murmure::chrome::MurmureRelay;
use ambre_murmure::drafts::{DraftRow, DraftShelf, DraftWrite, user_range};
use ambre_murmure::uploads::opens_as;
use ambre_murmure::view::PageReads;
use leptos::prelude::{GetUntracked, Set, Update};
use wasm_bindgen::JsValue;

use crate::vault::Vault;

/// What the page tells a Murmure region, read from the shell's relay. `warn`
/// puts a warning in front of the user.
pub fn page_reads(relay: MurmureRelay, pipe: TextInput, warn: Rc<dyn Fn(&str)>) -> PageReads {
    let clock = LocalTime::new();
    PageReads {
        pane: Rc::new(move || relay.pane.get_untracked()),
        name_of: Rc::new(move |user| relay.member_name(user)),
        time_of: Rc::new(move |ts| clock.format(ts)),
        now_ms: Rc::new(super::probe::now_ms),
        // `Date.now()` is a whole, non-negative count of milliseconds.
        wall_clock: Rc::new(|| ambre_bus::Timestamp(js_sys::Date::now() as u64)),
        motion: Rc::new({
            // The query is asked once; its answer follows the user's setting.
            let query = web_sys::window()
                .and_then(|w| w.match_media("(prefers-reduced-motion: reduce)").ok())
                .flatten();
            move || {
                if query.as_ref().is_some_and(web_sys::MediaQueryList::matches) {
                    ambre_murmure::pictures::Motion::Reduced
                } else {
                    ambre_murmure::pictures::Motion::Full
                }
            }
        }),
        announce: Rc::new({
            let pipe = pipe.clone();
            move |text| pipe.announce(text)
        }),
        act_on: Rc::new(move |target| relay.target.set(target)),
        take_edit: Rc::new(move |region| relay.take_edit(region)),
        back: Rc::new(move || relay.pane.update(|pane| *pane = pane.back())),
        me: Rc::new(move || relay.me()),
        pipe,
        members_of: Rc::new(move |channel| relay.members_of(channel)),
        show_mentions: Rc::new(move |region, list| relay.show_mentions(region, list)),
        take_pick: Rc::new(move |region| relay.take_pick(region)),
        show_bar: Rc::new(move |region, bar| relay.show_bar(region, bar)),
        take_press: Rc::new(move |region| relay.take_press(region)),
        take_returned: Rc::new(move |region| relay.take_returned(region)),
        open_link: Rc::new(super::open_address),
        drafts: relay.drafts(),
        ask_emoji: Rc::new(move |region| relay.ask_emoji(region)),
        take_emoji: Rc::new(move |region| relay.take_emoji(region)),
        take_files: Rc::new(move || relay.take_files()),
        save_file: Rc::new(save_file),
        warn,
    }
}

/// How long a tab has to read the address of an image it opened before the
/// page lets the bytes go.
const OPENED_FOR: ambre_core::Millis = ambre_core::Millis(60_000);

/// Hand the browser a file a member fetched. A raster image opens in a tab of
/// its own. Any other file saves, typed as a plain stream of bytes. So no
/// file a member sent runs in this page's origin.
fn save_file(file: &Attachment, data: Vec<u8>) {
    use wasm_bindgen::JsCast;
    let opens = opens_as(&file.media_type);
    let parts = js_sys::Array::new();
    parts.push(&js_sys::Uint8Array::from(data.as_slice()).buffer());
    let options = web_sys::BlobPropertyBag::new();
    options.set_type(
        opens
            .unwrap_or(ambre_core::KnownMediaType::OctetStream)
            .as_str(),
    );
    let url = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options)
        .and_then(|blob| web_sys::Url::create_object_url_with_blob(&blob));
    let url = match url {
        Ok(url) => url,
        Err(e) => {
            tracing::warn!("Murmure: the file could not open: {e:?}");
            return;
        }
    };
    let Some(window) = web_sys::window() else {
        return;
    };
    if opens.is_some() {
        let _ = window.open_with_url_and_target_and_features(&url, "_blank", "noopener,noreferrer");
        let revoke = wasm_bindgen::closure::Closure::once_into_js(move || {
            let _ = web_sys::Url::revoke_object_url(&url);
        });
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
            revoke.unchecked_ref(),
            OPENED_FOR.to_timer_i32(),
        );
        return;
    }
    let anchor = window
        .document()
        .and_then(|d| d.create_element("a").ok())
        .and_then(|e| e.dyn_into::<web_sys::HtmlAnchorElement>().ok());
    if let Some(anchor) = anchor {
        anchor.set_href(&url);
        anchor.set_download(&file.name);
        anchor.click();
    }
    let _ = web_sys::Url::revoke_object_url(&url);
}

/// The page's drafts in the local store (`MURMURE-210`). The store opens and
/// the session names the user in either order. The user's rows load once both
/// did, and every write of the shelf lands in the store from then on.
///
/// Each row lands sealed by the Sync worker, and opens through it when read
/// (`RESID-025`). A row that does not open leaves the store (`RESID-027`).
#[derive(Clone)]
pub struct DraftKeeper(Rc<RefCell<Keeper>>);

struct Keeper {
    shelf: DraftShelf,
    store: Option<PersistenceStore>,
    user: Option<UserId>,
    /// The user whose rows were read, so a second sign-in reads its own.
    loaded: Option<UserId>,
    vault: Vault,
}

/// The rows on their way to their seal, by key: the number of the latest
/// write each waits for. A write that is no longer the latest of its key
/// lands nothing, so a delete made meanwhile stays a delete.
#[derive(Default)]
struct Sealing {
    next: u64,
    latest: HashMap<DraftKey, u64>,
}

impl DraftKeeper {
    pub fn new(shelf: DraftShelf, vault: Vault) -> Self {
        Self(Rc::new(RefCell::new(Keeper {
            shelf,
            store: None,
            user: None,
            loaded: None,
            vault,
        })))
    }

    /// The store opened: the shelf writes into it.
    pub fn store_opened(&self, store: PersistenceStore) {
        let sink = store.clone();
        let vault = self.0.borrow().vault.clone();
        let sealing = Rc::new(RefCell::new(Sealing::default()));
        self.0
            .borrow()
            .shelf
            .set_sink(Rc::new(move |write| match write {
                DraftWrite::Put { key, row } => {
                    let number = {
                        let mut sealing = sealing.borrow_mut();
                        sealing.next += 1;
                        let number = sealing.next;
                        sealing.latest.insert(key.clone(), number);
                        number
                    };
                    let (sink, sealing) = (sink.clone(), Rc::clone(&sealing));
                    let place = RowPlace::Draft(key.clone());
                    let plaintext = ambre_bus::SecretBytes::new(row.into_bytes());
                    vault.seal(place, plaintext, move |sealed| {
                        let mut sealing = sealing.borrow_mut();
                        if sealing.latest.get(&key) != Some(&number) {
                            return;
                        }
                        sealing.latest.remove(&key);
                        match sealed {
                            Some(sealed) => sink.save_draft(&key, &sealed),
                            None => tracing::warn!("Drafts: no session seals the draft"),
                        }
                    });
                }
                DraftWrite::Delete { key } => {
                    sealing.borrow_mut().latest.remove(&key);
                    sink.delete_draft(&key);
                }
                DraftWrite::DeleteRange { lower, upper } => {
                    sealing
                        .borrow_mut()
                        .latest
                        .retain(|key, _| !(&lower..=&upper).contains(&key));
                    sink.delete_drafts(&lower, &upper);
                }
                DraftWrite::DeleteAll => {
                    sealing.borrow_mut().latest.clear();
                    sink.clear_drafts();
                }
            }));
        self.0.borrow_mut().store = Some(store);
        self.load();
    }

    /// The session named `user`.
    pub fn signed_in(&self, user: UserId) {
        self.0.borrow_mut().user = Some(user);
        self.load();
    }

    /// Resolves once every write handed to the store so far has landed. A
    /// read starts only after each read-write transaction opened before it
    /// on the same store has ended, so a read is the barrier.
    pub async fn written(&self) {
        let store = self.0.borrow().store.clone();
        if let Some(store) = store {
            let (first, last) = DraftKey::every();
            let _ = store.load_drafts(&first, &last).await;
        }
    }

    /// Read the user's rows into the shelf, once the store and the user are
    /// both known.
    fn load(&self) {
        let mut keeper = self.0.borrow_mut();
        let (Some(store), Some(user)) = (keeper.store.clone(), keeper.user) else {
            return;
        };
        if keeper.loaded == Some(user) {
            return;
        }
        keeper.loaded = Some(user);
        let shelf = keeper.shelf.clone();
        let vault = keeper.vault.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let (lower, upper) = user_range(user);
            let rows = store.load_drafts(&lower, &upper).await.unwrap_or_default();
            let keys: Vec<DraftKey> = rows.iter().map(|(key, _)| key.clone()).collect();
            let sealed = rows
                .into_iter()
                .map(|(key, sealed)| SealedRow {
                    place: RowPlace::Draft(key),
                    sealed,
                })
                .collect();
            vault.open(sealed, move |opened| {
                // No session holds the key: the rows stay for the next.
                let opened = opened.unwrap_or_default();
                let mut drafts = Vec::new();
                for (key, row) in keys.into_iter().zip(opened) {
                    match row.and_then(|row| DraftRow::from_bytes(row.to_vec())) {
                        Some(row) => drafts.push((key, row)),
                        None => store.delete_draft(&key),
                    }
                }
                shelf.load(user, drafts);
            });
        });
    }
}

/// The time of a message as the reader reads it: hours and minutes, in the
/// browser's language and time zone (`MURMURE-418`). Built once per page.
pub struct LocalTime(js_sys::Function);

impl LocalTime {
    pub fn new() -> Self {
        let options = js_sys::Object::new();
        for key in ["hour", "minute"] {
            let _ = js_sys::Reflect::set(&options, &key.into(), &"2-digit".into());
        }
        let format = js_sys::Intl::DateTimeFormat::new(&js_sys::Array::new(), &options);
        Self(format.format())
    }

    /// `ts`, written for the reader.
    pub fn format(&self, ts: Timestamp) -> String {
        let date = js_sys::Date::new(&JsValue::from_f64(ts.0 as f64));
        self.0
            .call1(&JsValue::NULL, &date)
            .ok()
            .and_then(|written| written.as_string())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use ambre_murmure::MurmurePane;
    use leptos::prelude::Set;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    /// The time reads in the browser's own language and time zone, hours
    /// and minutes (`MURMURE-418`).
    #[wasm_bindgen_test]
    fn the_time_reads_in_the_readers_language_and_zone() {
        let ms = 1_700_000_000_000.0;
        let options = js_sys::Object::new();
        for key in ["hour", "minute"] {
            js_sys::Reflect::set(&options, &key.into(), &"2-digit".into()).unwrap();
        }
        let date = js_sys::Date::new(&JsValue::from_f64(ms));
        let expected: String = date
            .to_locale_time_string_with_options("default", &options)
            .into();
        let written = LocalTime::new().format(Timestamp(ms as u64));
        assert_eq!(written, expected);
        assert!(written.chars().any(|c| c.is_ascii_digit()));
    }

    /// An announcement is the text of the content's live region, and the
    /// focus stays on what held it (`MURMURE-208`, `MURMURE-209`).
    #[wasm_bindgen_test]
    fn an_announcement_writes_the_live_region_and_leaves_the_focus() {
        let document = web_sys::window().unwrap().document().unwrap();
        let body = document.body().unwrap();
        let cell = document.create_element("div").unwrap();
        body.append_child(&cell).unwrap();
        let button: web_sys::HtmlElement =
            document.create_element("button").unwrap().unchecked_into();
        body.append_child(&button).unwrap();
        button.focus().unwrap();
        let reads = page_reads(
            MurmureRelay::new(),
            TextInput::create(&document, &cell),
            Rc::new(|_| {}),
        );
        (reads.announce)("Ada, 10:42: hello");
        let live = cell.query_selector(".ambre-a11y-live").unwrap().unwrap();
        assert_eq!(live.text_content().as_deref(), Some("Ada, 10:42: hello"));
        assert_eq!(live.get_attribute("aria-live").as_deref(), Some("polite"));
        let focused = document.active_element().unwrap();
        assert!(
            focused.is_same_node(Some(&button)),
            "the focus did not move"
        );
        cell.remove();
        button.remove();
    }

    /// The page reads the pane the shell shows now, and the same clock.
    #[wasm_bindgen_test]
    fn the_page_reads_follow_the_relay() {
        let relay = MurmureRelay::new();
        let reads = page_reads(relay, TextInput::detached(), Rc::new(|_| {}));
        assert_eq!((reads.pane)(), MurmurePane::ChannelList);
        relay.pane.set(MurmurePane::Members {
            channel: ambre_bus::ChannelId::new(),
        });
        assert!(matches!((reads.pane)(), MurmurePane::Members { .. }));
        let ts = Timestamp(1_700_000_000_000);
        assert_eq!((reads.time_of)(ts), LocalTime::new().format(ts));
    }

    /// `MURMURE-567`: a clicked link opens in a fresh tab with no opener and
    /// no referrer. The page's own `open` is swapped for a recorder, so the
    /// test opens no tab.
    #[wasm_bindgen_test]
    fn a_clicked_link_opens_in_a_fresh_tab_with_no_opener() {
        let window = web_sys::window().expect("a window");
        let key = JsValue::from_str("open");
        let original = js_sys::Reflect::get(&window, &key).expect("window.open");
        let recorder = js_sys::Function::new_with_args(
            "url, target, features",
            "globalThis.__murmureOpened = [url, target, features]; return null;",
        );
        js_sys::Reflect::set(&window, &key, &recorder).expect("a writable open");
        let reads = page_reads(MurmureRelay::new(), TextInput::detached(), Rc::new(|_| {}));
        (reads.open_link)(&"https://ambre.dev/a".into());
        js_sys::Reflect::set(&window, &key, &original).expect("open restored");
        let opened = js_sys::Reflect::get(&window, &"__murmureOpened".into()).expect("a record");
        let opened: Vec<String> = js_sys::Array::from(&opened)
            .iter()
            .filter_map(|v| v.as_string())
            .collect();
        assert_eq!(
            opened,
            ["https://ambre.dev/a", "_blank", "noopener,noreferrer"]
        );
    }

    /// A fetched raster image opens in a fresh tab from a blob address, with
    /// no opener. An SVG never opens, since its script would run in this
    /// page's origin.
    #[wasm_bindgen_test]
    fn a_fetched_image_opens_in_a_tab_and_an_svg_never_does() {
        let window = web_sys::window().expect("a window");
        let key = JsValue::from_str("open");
        let original = js_sys::Reflect::get(&window, &key).expect("window.open");
        let recorder = js_sys::Function::new_with_args(
            "url, target, features",
            "(globalThis.__murmureFiles ||= []).push([url, target, features]); return null;",
        );
        js_sys::Reflect::set(&window, &key, &recorder).expect("a writable open");
        let file = |media_type: &str| Attachment {
            media_ref: ambre_bus::MediaRef::parse(&"0".repeat(64)).expect("an address"),
            name: "a".to_owned(),
            media_type: ambre_core::MediaType::from(media_type),
            size: ambre_bus::ByteSize(1),
        };
        save_file(&file("image/png"), vec![1]);
        js_sys::Reflect::set(&window, &key, &original).expect("open restored");
        let opened = js_sys::Reflect::get(&window, &"__murmureFiles".into()).expect("a record");
        let opened: Vec<Vec<String>> = js_sys::Array::from(&opened)
            .iter()
            .map(|call| {
                js_sys::Array::from(&call)
                    .iter()
                    .filter_map(|v| v.as_string())
                    .collect()
            })
            .collect();
        let [call] = opened.as_slice() else {
            panic!("one tab, got {opened:?}");
        };
        assert!(call[0].starts_with("blob:"), "a blob address: {}", call[0]);
        assert_eq!(call[1..], ["_blank", "noopener,noreferrer"]);
        assert_eq!(opens_as(&file("image/svg+xml").media_type), None);
    }
}
