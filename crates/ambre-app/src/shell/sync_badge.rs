// ambre-app/src/shell/sync_badge.rs
// ============================================================================
// The sync state in the header: a dot and a label, both read from one signal.
// The stylesheet hides the label below the tablet width (`CHROME-058`), so a
// phone shows the dot alone.
// ============================================================================

use super::super::ChromeStatus;
use leptos::prelude::*;

/// The header's sync state: a coloured dot and its label.
#[component]
pub(super) fn SyncBadge(status: Signal<ChromeStatus>) -> impl IntoView {
    view! {
        <span class="ambre-sync">
            <span class=move || status.get().dot_class()></span>
            <span class="label">{move || status.get().label()}</span>
        </span>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_chrome::Kbd;
    use ambre_chrome::commands::KeyBinding;
    use leptos::mount::mount_to;
    use leptos::wasm_bindgen::JsCast;
    use wasm_bindgen_test::wasm_bindgen_test;

    /// The shell stylesheet, as the page loads it.
    const STYLES: &str = include_str!("../../styles.css");

    fn text(root: &web_sys::Element, selector: &str) -> String {
        root.query_selector(selector)
            .unwrap()
            .unwrap_or_else(|| panic!("no `{selector}` on screen"))
            .text_content()
            .unwrap_or_default()
    }

    /// A signal change reaches the DOM after one reactive tick.
    ///
    /// The label and the dot follow the link state. A badge that read its
    /// status once would keep showing the first one.
    #[wasm_bindgen_test]
    async fn the_badge_follows_the_link_after_a_tick() {
        let status = RwSignal::new(ChromeStatus {
            hydrating: false,
            connected: true,
        });
        let container = document().create_element("div").unwrap();
        document().body().unwrap().append_child(&container).unwrap();
        let target: web_sys::HtmlElement = container.clone().dyn_into().unwrap();
        mount_to(target, move || view! { <SyncBadge status=status.into() /> }).forget();
        assert_eq!(text(&container, ".label"), "Synced");

        status.set(ChromeStatus {
            hydrating: false,
            connected: false,
        });
        leptos::task::tick().await;
        assert_eq!(text(&container, ".label"), "Offline");
        let dot = container.query_selector(".dot").unwrap().unwrap();
        assert_eq!(dot.class_name(), "dot offline");
        container.remove();
    }

    /// Mount the badge and a chord chip in a frame of `width` pixels, with the
    /// shell stylesheet. Return whether the label and the chip are displayed.
    ///
    /// A width query reads the frame's viewport, not the test page's, so the
    /// frame is how a test sets the width. Visibility is read rather than a
    /// display value, which a flex parent changes.
    fn shown_at(width: u32) -> (bool, bool) {
        let frame: web_sys::HtmlIFrameElement = document()
            .create_element("iframe")
            .unwrap()
            .dyn_into()
            .unwrap();
        let style = format!("width:{width}px;height:200px;border:0");
        frame.set_attribute("style", &style).unwrap();
        document().body().unwrap().append_child(&frame).unwrap();
        let inner = frame.content_document().unwrap();
        let sheet = inner.create_element("style").unwrap();
        sheet.set_text_content(Some(STYLES));
        inner.head().unwrap().append_child(&sheet).unwrap();

        let status = Signal::stored(ChromeStatus::default());
        mount_to(inner.body().unwrap(), move || {
            view! {
                <SyncBadge status=status />
                <Kbd binding=KeyBinding::primary('k') />
            }
        })
        .forget();
        // Settle the outer layout, so the frame has its width when read.
        let _ = frame.offset_width();

        let view = frame.content_window().unwrap();
        let shown = |selector: &str| {
            let el = inner.query_selector(selector).unwrap().unwrap();
            view.get_computed_style(&el)
                .unwrap()
                .unwrap()
                .get_property_value("display")
                .unwrap()
                != "none"
        };
        let out = (shown(".ambre-sync .label"), shown(".ambre-kbd"));
        frame.remove();
        out
    }

    /// A phone shows the sync dot without its label and no chord
    /// (`CHROME-057`, `CHROME-058`).
    ///
    /// A tablet restores the label, and only a desktop restores the chord.
    /// Each side of both thresholds is read.
    #[wasm_bindgen_test]
    fn the_sync_label_and_the_chord_appear_with_the_width() {
        assert_eq!(shown_at(400), (false, false), "a phone");
        assert_eq!(shown_at(599), (false, false), "the widest phone");
        assert_eq!(shown_at(600), (true, false), "the narrowest tablet");
        assert_eq!(shown_at(1024), (true, false), "the widest tablet");
        assert_eq!(shown_at(1025), (true, true), "the narrowest desktop");
    }
}
