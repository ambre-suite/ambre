//! The gallery page of the visual system, behind the `chrome-gallery`
//! feature (`ambre-spec-testing.md` section 2, the computed-style snapshot).
//!
//! At `/gallery` the launcher mounts the gallery of the chrome toolkit alone,
//! under the token sheet, and starts no thread. The browser tier reads its
//! computed styles; no release build turns the feature on.

/// Whether the launcher mounted the gallery in place of the application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mounted {
    /// The page is elsewhere: the application boots.
    No,
    /// The gallery holds the page: the boot stops.
    Yes,
}

/// The path the gallery answers at.
const GALLERY_PATH: &str = "/gallery";

/// Mount the gallery when the page is at its path, and say so. The boot then
/// stops: the gallery reaches no thread and opens no document.
pub(crate) fn mount_if_asked() -> Mounted {
    let Some(window) = web_sys::window() else {
        return Mounted::No;
    };
    if window.location().pathname().ok().as_deref() != Some(GALLERY_PATH) {
        return Mounted::No;
    }
    crate::platform::inject_theme(crate::platform::boot_theme(crate::platform::system_scheme()));
    if let Some(loading) = window
        .document()
        .and_then(|d| d.get_element_by_id("loading"))
    {
        loading.remove();
    }
    leptos::mount::mount_to_body(ambre_chrome::Gallery);
    Mounted::Yes
}
