// ambre-app/src/host/murmure.rs
// The page's side of a Murmure message region. The view lives in ambre-murmure
// and reads the page through closures. They give the pane, a member's name,
// and the time in the reader's language and zone.

use std::rc::Rc;

use ambre_bus::Timestamp;
use ambre_murmure::chrome::MurmureRelay;
use ambre_murmure::view::PageReads;
use leptos::prelude::GetUntracked;
use wasm_bindgen::JsValue;

/// What the page tells a Murmure region, read from the shell's relay.
pub fn page_reads(relay: MurmureRelay) -> PageReads {
    let clock = LocalTime::new();
    PageReads {
        pane: Rc::new(move || relay.pane.get_untracked()),
        name_of: Rc::new(move |user| relay.member_name(user)),
        time_of: Rc::new(move |ts| clock.format(ts)),
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

    /// The page reads the pane the shell shows now, and the same clock.
    #[wasm_bindgen_test]
    fn the_page_reads_follow_the_relay() {
        let relay = MurmureRelay::new();
        let reads = page_reads(relay);
        assert_eq!((reads.pane)(), MurmurePane::ChannelList);
        relay.pane.set(MurmurePane::Members {
            channel: ambre_bus::ChannelId::new(),
        });
        assert!(matches!((reads.pane)(), MurmurePane::Members { .. }));
        let ts = Timestamp(1_700_000_000_000);
        assert_eq!((reads.time_of)(ts), LocalTime::new().format(ts));
    }
}
