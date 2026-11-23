//! The look of the visual system, read from the gallery page of a test build
//! (`ambre-spec-testing.md` section 2, the computed-style snapshot).
//!
//! The build of the belt turns on `ambre-app/chrome-gallery`, which serves
//! every control primitive in each of its states at `/gallery`. A test forces
//! the pointer and focus states through the DevTools Protocol. It reads the
//! computed style of each probe, light then dark, and holds the text against
//! one snapshot. A reviewer reads the diff and accepts it by hand.
//!
//! The snapshot keeps what the visual system sets. That is a control height,
//! a padding, a border, a radius, a ground, a colour, a type size, a focus
//! ring and a shadow. A width that follows a text, and the family of a face,
//! differ between two machines, so neither is read.
//!
//! A second test, ignored by default, writes the screenshots of the three
//! panels. They are at a device scale of 2, light and dark, for a person to
//! look at. CI keeps them as an artifact and compares nothing.

use std::path::PathBuf;

use serde_json::{Value, json};

use crate::driver::Browser;
use crate::page::{BOOT, STEP};
use crate::relay::Exchange;
use crate::serve;

/// The elements whose style the snapshot reads, under each specimen.
const PROBES: &str = "[data-specimen] :is(.ambre-icon-button, .ambre-select-trigger, \
    .ambre-segments, .ambre-segments > button, .ambre-stepper, .ambre-stepper-step, \
    .ambre-stepper > input, .ambre-stepper-unit, .ambre-swatch, .ambre-slider > input, \
    .ambre-slider-readout, .ambre-kit-command, .ambre-ctx-head, .ambre-ctx-title, \
    .ambre-ctx-section-head, .ambre-ctx-section-body, .ambre-kit-label, \
    .ambre-icon-button > svg, .ambre-kit-command > svg)";

/// The probes a pointer or a focus moves to another state.
const INTERACTIVE: &str = "[data-specimen] :is(.ambre-icon-button, .ambre-select-trigger, \
    .ambre-segments > button, .ambre-stepper-step, .ambre-swatch, .ambre-slider > input, \
    .ambre-kit-command, .ambre-ctx-section-head)";

/// The pseudo-states a test forces on an interactive probe, besides rest.
const FORCED: [&str; 3] = ["hover", "focus-visible", "active"];

/// The script that reads one probe: its place in the selector is the
/// argument. It answers one line, so a diff names the control it moved. The
/// line holds the specimen, the class, the place in the specimen and the
/// state. Each property the visual system sets follows.
const READ: &str = r#"
const [selector, at, state] = arguments;
const el = document.querySelectorAll(selector)[at];
const box = el.closest('[data-specimen]');
const place = [...box.querySelectorAll(selector)].indexOf(el);
const name = el.tagName === 'svg' ? 'svg' : (el.className.baseVal ?? el.className).split(' ')[0] || el.tagName.toLowerCase();
const s = getComputedStyle(el);
const parts = [];
if (el.matches('.ambre-icon-button, .ambre-swatch, .ambre-stepper-step, svg')) parts.push(`w ${s.width}`);
if (el.matches('.ambre-icon-button, .ambre-swatch, .ambre-stepper-step, svg, .ambre-select-trigger, .ambre-stepper')) parts.push(`h ${s.height}`);
if (s.minHeight !== 'auto' && s.minHeight !== '0px') parts.push(`min-h ${s.minHeight}`);
if (el.tagName !== 'svg') {
  parts.push(`pad ${s.paddingTop} ${s.paddingRight} ${s.paddingBottom} ${s.paddingLeft}`);
  parts.push(s.borderTopStyle === 'none' ? 'border none' : `border ${s.borderTopWidth} ${s.borderTopStyle} ${s.borderTopColor}`);
  parts.push(`radius ${s.borderTopLeftRadius}`, `bg ${s.backgroundColor}`, `font ${s.fontSize}/${s.fontWeight}`);
  if (s.boxShadow !== 'none') parts.push(`shadow ${s.boxShadow}`);
}
parts.push(`fg ${s.color}`);
if (s.opacity !== '1') parts.push(`opacity ${s.opacity}`);
if (s.outlineStyle !== 'none') parts.push(`outline ${s.outlineWidth} ${s.outlineStyle} ${s.outlineColor} +${s.outlineOffset}`);
return [`${box.dataset.specimen} ${name}#${place} ${state} | ${parts.join(' | ')}`];
"#;

/// The gallery of the served test build, open in a browser at `/gallery`.
async fn open_gallery() -> Browser {
    let downloads = std::env::temp_dir().join(format!("belt-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&downloads).expect("a download directory");
    // The gallery reaches no relay: the address only fills the server's slot.
    let origin = serve::spawn(
        serve::dist_dir(),
        "http://127.0.0.1:9".to_owned(),
        Exchange::StandIn,
    )
    .await;
    let browser = Browser::start(&downloads).await;
    browser.goto(&format!("{origin}/gallery")).await;
    browser
        .wait_for(
            "the gallery",
            "return !!document.querySelector('.ambre-gallery [data-specimen=\"frame\"] .ambre-ctx-head');",
            BOOT,
        )
        .await;
    browser
}

/// The CDP node ids of every element `selector` matches, in document order.
async fn nodes(browser: &Browser, selector: &str) -> Vec<i64> {
    let root = browser.cdp("DOM.getDocument", json!({ "depth": -1 })).await;
    let root = root["root"]["nodeId"].as_i64().expect("a document node");
    let found = browser
        .cdp(
            "DOM.querySelectorAll",
            json!({ "nodeId": root, "selector": selector }),
        )
        .await;
    found["nodeIds"]
        .as_array()
        .expect("node ids")
        .iter()
        .filter_map(Value::as_i64)
        .collect()
}

/// The lines of one probe in one state.
async fn read(browser: &Browser, selector: &str, at: usize, state: &str) -> Vec<String> {
    let lines = browser.script(READ, json!([selector, at, state])).await;
    lines
        .as_array()
        .expect("the lines of a probe")
        .iter()
        .filter_map(|l| l.as_str().map(str::to_owned))
        .collect()
}

/// The whole record of the gallery in the theme `theme`: every probe at
/// rest, then every interactive probe in each forced state.
async fn record(browser: &Browser, theme: &str) -> Vec<String> {
    browser
        .script(
            "document.documentElement.setAttribute('data-theme', arguments[0]); return true;",
            json!([theme]),
        )
        .await;
    let mut out = vec![format!("== theme {theme}")];
    let probes = nodes(browser, PROBES).await;
    for at in 0..probes.len() {
        out.extend(read(browser, PROBES, at, "rest").await);
    }
    let interactive = nodes(browser, INTERACTIVE).await;
    for (at, node) in interactive.iter().enumerate() {
        for state in FORCED {
            browser
                .cdp(
                    "CSS.forcePseudoState",
                    json!({ "nodeId": node, "forcedPseudoClasses": [state] }),
                )
                .await;
            out.extend(read(browser, INTERACTIVE, at, state).await);
        }
        browser
            .cdp(
                "CSS.forcePseudoState",
                json!({ "nodeId": node, "forcedPseudoClasses": [] }),
            )
            .await;
    }
    out
}

/// Where the screenshots of the applications land: an ignored directory CI
/// uploads as an artifact.
fn shots_dir() -> PathBuf {
    let dir = std::env::var_os("BELT_SHOTS").map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/belt-shots"),
        PathBuf::from,
    );
    std::fs::create_dir_all(&dir).expect("a screenshot directory");
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `CHROME-223`: every primitive of the gallery and the frame of the
    /// panel, in each state, light and dark, keep the look a reviewer last
    /// accepted. The snapshot is the record; `cargo insta review` accepts it.
    #[tokio::test]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a Trunk build with chrome-gallery and a chromedriver; run just test-browser"
    )]
    #[cfg_attr(
        feature = "firefox",
        ignore = "forcing a pseudo-state is the DevTools Protocol, which Firefox does not speak"
    )]
    async fn the_gallery_keeps_the_look_a_reviewer_accepted() {
        let browser = open_gallery().await;
        browser.cdp("DOM.enable", json!({})).await;
        browser.cdp("CSS.enable", json!({})).await;
        let mut lines = record(&browser, "light").await;
        lines.extend(record(&browser, "dark").await);
        insta::assert_snapshot!("gallery", lines.join("\n"));
    }

    /// Writes a screenshot of each application's panel, at a device scale of
    /// 2, light and dark, for a person to look at. It asserts nothing: CI
    /// keeps the files as an artifact and compares none of them.
    #[tokio::test]
    #[ignore = "writes screenshots for a person; CI runs it apart and keeps the files"]
    async fn write_the_screenshots_of_the_three_panels() {
        use std::sync::Arc;

        use crate::page::Page;
        use crate::relay::Relay;

        let dir = shots_dir();
        let relay = Arc::new(Relay::start().await);
        for theme in ["light", "dark"] {
            let page = Page::home(Arc::clone(&relay)).await;
            page.browser
                .cdp(
                    "Emulation.setDeviceMetricsOverride",
                    json!({ "width": 1280, "height": 800, "deviceScaleFactor": 2, "mobile": false }),
                )
                .await;
            page.browser
                .cdp(
                    "Emulation.setEmulatedMedia",
                    json!({ "features": [{ "name": "prefers-color-scheme", "value": theme }] }),
                )
                .await;
            page.browser.goto(&format!("{}/", page.origin)).await;
            page.sign_in(&format!("shots-{}", uuid::Uuid::new_v4()))
                .await;
            for kind in ["prose", "trame", "toile"] {
                page.open_new(kind).await;
                page.browser
                    .wait_for(
                        "the panel",
                        "return !!document.querySelector('.ambre-ctx-panel .ambre-ctx-head');",
                        STEP,
                    )
                    .await;
                let png = page.browser.screenshot().await;
                std::fs::write(dir.join(format!("{kind}-{theme}.png")), png)
                    .expect("a screenshot written");
            }
        }
    }
}
