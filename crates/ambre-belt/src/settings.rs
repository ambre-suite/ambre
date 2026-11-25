//! The settings of a user, followed across two devices of one account
//! (`ambre-spec-user-settings.md`).
//!
//! Each device is a browser with a profile of its own, so it shares no local
//! storage with the other. The development sign-in derives the identity from
//! the name, so both browsers sign in to one account. A setting moves from one
//! to the other through the relay alone.

use serde_json::{Value, json};

use crate::page::{Page, STEP};

/// The theme the page draws, as its stylesheet reads it.
async fn theme(page: &Page) -> String {
    let theme = page
        .browser
        .script(
            "return document.documentElement.getAttribute('data-theme') || '';",
            json!([]),
        )
        .await;
    theme.as_str().unwrap_or_default().to_owned()
}

/// Waits until the page draws `theme`.
async fn wait_theme(page: &Page, theme: &str) {
    page.browser
        .wait_for(
            &format!("the {theme} theme"),
            &format!(
                "return document.documentElement.getAttribute('data-theme') === {};",
                Value::String(theme.to_owned())
            ),
            STEP,
        )
        .await;
}

/// Switches the theme from the account menu, as a person does.
async fn switch_theme(page: &Page) {
    page.browser
        .wait_for(
            "the theme item of the account menu",
            "const b = document.querySelector('.ambre-account-menu .ambre-header-menu-button'); \
             if (!b) return false; \
             if (b.getAttribute('aria-expanded') !== 'true') b.click(); \
             const item = document.querySelector('.ambre-menu-theme'); \
             if (!item) return false; \
             item.click(); \
             return true;",
            STEP,
        )
        .await;
}

/// Clicks the control of the context panel named `label`, once it shows.
async fn press(page: &Page, label: &str) {
    page.browser
        .wait_for(
            label,
            &format!(
                "const b = document.querySelector('.ambre-ctx-panel [aria-label=' + JSON.stringify({}) + ']'); \
                 if (!b) return false; b.click(); return true;",
                Value::String(label.to_owned())
            ),
            STEP,
        )
        .await;
}

/// Waits until the context panel shows `attribute` at `value`: its side or
/// its fold.
async fn wait_panel(page: &Page, attribute: &str, value: &str) {
    page.browser
        .wait_for(
            &format!("the panel {attribute} {value}"),
            &format!(
                "const p = document.querySelector('.ambre-ctx-panel'); \
                 return !!p && p.getAttribute({}) === {};",
                Value::String(attribute.to_owned()),
                Value::String(value.to_owned())
            ),
            STEP,
        )
        .await;
}

/// Docks the panel to the left, as a person does.
async fn dock_left(page: &Page) {
    press(page, "Dock the panel to the left").await;
}

/// Collapses the panel of the application the page shows.
async fn collapse(page: &Page) {
    press(page, "Collapse the panel").await;
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use crate::relay::Relay;

    /// `SETTINGS-012`, `SETTINGS-041`: a theme switched on one device of an
    /// account shows on the other, through the relay, and a switch back on
    /// the second shows on the first. The side of the panel follows the
    /// account in every application, and its fold in one application alone.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build, chromedriver and the services; run just test-browser"
    )]
    async fn the_settings_follow_the_account_across_two_devices() {
        let name = format!("belt-{}", uuid::Uuid::new_v4());
        let laptop = Page::home(Arc::new(Relay::start().await)).await;
        laptop.sign_in(&name).await;
        laptop.open_new("prose").await;
        let phone = Page::home(laptop.relay()).await;
        phone.sign_in(&name).await;
        phone.open_new("trame").await;

        let first = theme(&laptop).await;
        assert_eq!(theme(&phone).await, first, "both start at one theme");
        switch_theme(&laptop).await;
        let switched = if first == "dark" { "light" } else { "dark" };
        wait_theme(&laptop, switched).await;
        wait_theme(&phone, switched).await;

        switch_theme(&phone).await;
        wait_theme(&phone, &first).await;
        wait_theme(&laptop, &first).await;

        // One side for every application (`SETTINGS-041`).
        wait_panel(&phone, "data-side", "right").await;
        dock_left(&laptop).await;
        wait_panel(&laptop, "data-side", "left").await;
        wait_panel(&phone, "data-side", "left").await;

        // One fold per application: Trame folds on the phone, and Prose stays
        // open on the laptop until it opens Trame.
        collapse(&phone).await;
        wait_panel(&phone, "data-fold", "rail").await;
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        wait_panel(&laptop, "data-fold", "open").await;
        laptop.open_new("trame").await;
        wait_panel(&laptop, "data-fold", "rail").await;
        wait_panel(&laptop, "data-side", "left").await;
    }
}
