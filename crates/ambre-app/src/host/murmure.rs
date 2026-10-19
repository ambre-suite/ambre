// ambre-app/src/host/murmure.rs
// The page's side of a Murmure message region. The view lives in ambre-murmure
// and reads the page through closures. They give the pane, a member's name,
// and the time in the reader's language and zone.

use std::rc::Rc;

use ambre_bus::Timestamp;
use ambre_canvas::TextInput;
use ambre_murmure::chrome::MurmureRelay;
use ambre_murmure::view::PageReads;
use leptos::prelude::{GetUntracked, Set, Update};
use wasm_bindgen::JsValue;

/// What the page tells a Murmure region, read from the shell's relay.
pub fn page_reads(relay: MurmureRelay, pipe: TextInput) -> PageReads {
    let clock = LocalTime::new();
    PageReads {
        pane: Rc::new(move || relay.pane.get_untracked()),
        name_of: Rc::new(move |user| relay.member_name(user)),
        time_of: Rc::new(move |ts| clock.format(ts)),
        now_ms: Rc::new(super::probe::now_ms),
        announce: Rc::new({
            let pipe = pipe.clone();
            move |text| pipe.announce(text)
        }),
        act_on: Rc::new(move |target| relay.target.set(target)),
        take_edit: Rc::new(move |in_thread| relay.take_edit(in_thread)),
        back: Rc::new(move || relay.pane.update(|pane| *pane = pane.back())),
        me: Rc::new(move || relay.me()),
        pipe,
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
        let reads = page_reads(MurmureRelay::new(), TextInput::create(&document, &cell));
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
        let reads = page_reads(relay, TextInput::detached());
        assert_eq!((reads.pane)(), MurmurePane::ChannelList);
        relay.pane.set(MurmurePane::Members {
            channel: ambre_bus::ChannelId::new(),
        });
        assert!(matches!((reads.pane)(), MurmurePane::Members { .. }));
        let ts = Timestamp(1_700_000_000_000);
        assert_eq!((reads.time_of)(ts), LocalTime::new().format(ts));
    }
}
