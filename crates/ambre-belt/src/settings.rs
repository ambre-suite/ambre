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

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use crate::relay::Relay;

    /// `SETTINGS-012`, `SETTINGS-023`: a theme switched on one device of an
    /// account shows on the other, through the relay. A switch back on the
    /// second device shows on the first.
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
    }
}
