//! A name composed for display: the placeholder of an unnamed document
//! (`CHROME-079`), or a name joined to the kind label (`CHROME-043`).
//!
//! Such a name MUST NOT be stored (`CHROME-080`). So it is a type of its own
//! that converts to no `String`.

use leptos::prelude::IntoView;

/// What an unnamed document shows. No creatable kind's word, so the
/// placeholder never reads as the kind (`CHROME-079`).
const UNTITLED: &str = "Untitled";

/// The tab of a page with no document open.
const BRAND: &str = "Ambre";

/// A name composed for display alone (`CHROME-080`).
///
/// It has no `Debug`, no `Display`, no `AsRef<str>` and no `Into<String>`,
/// and this crate takes no `serde`. So a rename, a store or a log line built
/// from one does not compile. Its text leaves through two sinks: the tab title
/// and a view. Each block below fails on the conversion alone, since this one
/// builds the same name:
///
/// ```
/// let name = ambre_chrome::ComposedName::or_placeholder(None);
/// let shown = name.clone() == name;
/// ```
///
/// ```compile_fail
/// let name = ambre_chrome::ComposedName::or_placeholder(None);
/// let stored: String = name.into();
/// ```
///
/// ```compile_fail
/// let name = ambre_chrome::ComposedName::or_placeholder(None);
/// let stored = name.to_string();
/// ```
///
/// ```compile_fail
/// let name = ambre_chrome::ComposedName::or_placeholder(None);
/// let logged = format!("{name:?}");
/// ```
///
/// ```compile_fail
/// let name = ambre_chrome::ComposedName::or_placeholder(None);
/// let read: &str = name.as_ref();
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct ComposedName(String);

impl ComposedName {
    /// A document's name, or the placeholder when it has none or only blanks.
    pub fn or_placeholder(name: Option<&str>) -> Self {
        match name.map(str::trim) {
            Some(n) if !n.is_empty() => Self(n.to_owned()),
            _ => Self(UNTITLED.to_owned()),
        }
    }

    /// The browser tab's title. A tab carries no kind icon, so the name is
    /// joined to the generic kind `label` (`CHROME-043`). `None` is a page
    /// with no document open.
    pub fn tab_title(doc: Option<(&'static str, Option<&str>)>) -> Self {
        match doc {
            Some((label, name)) => {
                let Self(name) = Self::or_placeholder(name);
                Self(format!("{name} - {label}"))
            }
            None => Self(BRAND.to_owned()),
        }
    }

    /// Set the browser tab's title to this name.
    pub fn set_as_title(&self, document: &web_sys::Document) {
        document.set_title(&self.0);
    }

    /// The name as a text node of a view.
    pub fn into_view(self) -> impl IntoView {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tab joins the name to the kind label, and a blank name reads as the
    /// placeholder (`CHROME-043`).
    #[test]
    fn a_tab_title_composes_the_name_with_the_kind_label() {
        let title = |doc| ComposedName::tab_title(doc).0;
        assert_eq!(
            title(Some(("Spreadsheet", Some("Budget")))),
            "Budget - Spreadsheet"
        );
        assert_eq!(
            title(Some(("Spreadsheet", Some("  ")))),
            "Untitled - Spreadsheet"
        );
        assert_eq!(title(Some(("Spreadsheet", None))), "Untitled - Spreadsheet");
        assert_eq!(title(None), "Ambre");
    }

    /// A name keeps its text, and only a missing or blank one is the
    /// placeholder.
    #[test]
    fn only_a_missing_or_blank_name_is_the_placeholder() {
        let shown = |name| ComposedName::or_placeholder(name).0;
        assert_eq!(shown(Some(" Budget ")), "Budget");
        assert_eq!(shown(Some(" ")), UNTITLED);
        assert_eq!(shown(None), UNTITLED);
    }
}
