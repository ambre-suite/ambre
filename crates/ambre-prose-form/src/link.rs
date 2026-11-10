//! Whether a link may reach the host (`PROSE-UI-186`), and the longest
//! target a writer keeps (`PROSE-IO-069`). Prose reads them through its
//! `link` module, and the Markdown writer of the markup module reads the
//! same rule, so the two cannot drift apart.

/// A scheme a link may hand to the host to open. The set is closed: any
/// other scheme, `javascript:` or `data:` among them, never reaches the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum OpenableScheme {
    Http,
    Https,
    Mailto,
}

impl OpenableScheme {
    /// Every scheme of the set.
    const ALL: [Self; 3] = [Self::Http, Self::Https, Self::Mailto];

    /// The scheme as a target spells it, before its colon.
    fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Https => "https",
            Self::Mailto => "mailto",
        }
    }

    /// The scheme `word` names, case folded. `None` for a scheme outside
    /// the set.
    fn parse(word: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|scheme| word.eq_ignore_ascii_case(scheme.as_str()))
    }
}

/// The longest link target a paste keeps and a copy or an export writes, in
/// bytes (`PROSE-UI-234`, `PROSE-IO-069`).
pub const HREF_MAX: ambre_core::Count<ambre_core::Octets> =
    ambre_core::Count::<ambre_core::Octets>::new(2048);

/// Whether a link's target of `size` fits [`HREF_MAX`]. A writer writes a
/// longer one as its text alone. A paste, a copy and an export all ask here,
/// so the three cannot drift apart.
pub fn within_href_max(size: ambre_core::Count<ambre_core::Octets>) -> bool {
    size <= HREF_MAX
}

/// Whether a link's target may be handed to the host (`PROSE-UI-186`).
///
/// Nothing rules on a target when a peer writes it, so a hostile one may
/// carry `javascript:` or `data:`. Only an [`OpenableScheme`] passes.
/// The scheme is read the way a browser reads it: leading spaces and control
/// characters skipped, tabs and newlines inside dropped, case folded.
#[must_use]
pub fn openable(url: &ambre_core::LinkUrl) -> bool {
    let cleaned: String = url
        .as_str()
        .trim_start_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    cleaned
        .split_once(':')
        .and_then(|(scheme, _)| OpenableScheme::parse(scheme))
        .is_some()
}
