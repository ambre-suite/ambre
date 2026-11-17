//! Keys that a `const` table of the code names: a command, an icon, a
//! command category, a canvas element.
//!
//! Every such key is one generic type, [`StaticKey<K>`], over a marker `K`
//! that names its kind. A command key never stands in for an icon key, though
//! both are a static text:
//!
//! ```compile_fail
//! use ambre_core::{Commands, Icons, StaticKey};
//! let _: StaticKey<Icons> = StaticKey::<Commands>::new("prose.format.bold");
//! ```
//!
//! Nor does the name a refusal prints stand in for a command id:
//!
//! ```compile_fail
//! use ambre_core::{CommandNames, Commands, StaticKey};
//! let _: StaticKey<Commands> = StaticKey::<CommandNames>::new("typing");
//! ```

use std::fmt;
use std::marker::PhantomData;

mod sealed {
    /// Closes [`super::KeyKind`].
    pub trait Sealed {}
}

/// The marker of one kind of static key. Sealed: every kind lives in this
/// module.
pub trait KeyKind: sealed::Sealed + 'static {
    /// The name `Debug` prints, as the hand-written type printed its own.
    const NAME: &'static str;
}

/// A key of the kind `K` that a `const` table names.
///
/// The `&'static str` is the identity: two keys are equal iff their strings
/// are equal, which makes them cheap map keys and trivially `Copy`.
///
/// `PartialEq` and `Eq` are derived, not written by hand. A handler matches
/// a key against the `const` keys of its table. Only a derived equality lets
/// a constant stand in a pattern. The derive bounds the marker, so each
/// marker derives the same traits.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct StaticKey<K: KeyKind> {
    text: &'static str,
    kind: PhantomData<fn() -> K>,
}

impl<K: KeyKind> StaticKey<K> {
    /// Wraps a raw value. The constructor checks nothing.
    #[must_use]
    pub const fn new(value: &'static str) -> Self {
        Self {
            text: value,
            kind: PhantomData,
        }
    }

    /// Returns the raw text.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.text
    }
}

/// `NAME("...")`, as the derived `Debug` of the hand-written type printed
/// it.
impl<K: KeyKind> fmt::Debug for StaticKey<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple(K::NAME).field(&self.text).finish()
    }
}

/// Stable, namespaced identifier of a command, for example
/// `StaticKey::<Commands>::new("prose.format.bold")`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Commands {}

impl sealed::Sealed for Commands {}
impl KeyKind for Commands {
    const NAME: &'static str = "CommandId";
}

/// The name a family of the core prints through `Debug` for one marker:
/// `Rows` in `Pos<Rows>(3)`. Each marker of a family names itself by one,
/// and only a log line reads it.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum DebugNames {}

impl sealed::Sealed for DebugNames {}
impl KeyKind for DebugNames {
    const NAME: &'static str = "DebugName";
}

/// Opaque icon handle. The concrete glyph/asset is resolved by the chrome's
/// icon set; the registry only carries the reference.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icons {}

impl sealed::Sealed for Icons {}
impl KeyKind for Icons {
    const NAME: &'static str = "IconId";
}

/// Grouping key for the command palette and for ribbon tabs (one category =
/// one tab), for example `StaticKey::<Categories>::new("format")`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Categories {}

impl sealed::Sealed for Categories {}
impl KeyKind for Categories {
    const NAME: &'static str = "CommandCategory";
}

/// The words a refusal uses for a command, for example "replying to the
/// comment". A [`Commands`] key is a namespaced id a registry looks up, which
/// no author reads, so the two kinds stay apart.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandNames {}

impl sealed::Sealed for CommandNames {}
impl KeyKind for CommandNames {
    const NAME: &'static str = "CommandName";
}

/// The bare words, as a refusal shows them to the author.
impl fmt::Display for StaticKey<CommandNames> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.text)
    }
}

/// The id of a `<canvas>` element in the page. The shell mounts the element
/// with this id, and the canvas host finds it by the same id. Each id is a
/// constant of the code, so a distinct kind keeps it apart from page text.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Canvases {}

impl sealed::Sealed for Canvases {}
impl KeyKind for Canvases {
    const NAME: &'static str = "CanvasId";
}

/// A key of the browser's local or session storage. The code writes the
/// value under the key and reads it back by the same key, so a typo in one
/// place loses the value.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum StorageKeys {}

impl sealed::Sealed for StorageKeys {}
impl KeyKind for StorageKeys {
    const NAME: &'static str = "StorageKey";
}

/// A class that the code puts on an element and a stylesheet selects. The
/// stylesheet matches the class back, so it is no text a person reads.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum CssClasses {}

impl sealed::Sealed for CssClasses {}
impl KeyKind for CssClasses {
    const NAME: &'static str = "CssClass";
}

/// The id of an element that another element names, as the alert line of a
/// form that each field names as its description. Both sides read one
/// constant, so a typo cannot break the link a screen reader follows.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum ElementIds {}

impl sealed::Sealed for ElementIds {}
impl KeyKind for ElementIds {
    const NAME: &'static str = "ElementId";
}

/// The name of an attribute that the code sets on an element and reads back
/// off it, possibly from another crate.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Attributes {}

impl sealed::Sealed for Attributes {}
impl KeyKind for Attributes {
    const NAME: &'static str = "AttributeName";
}

/// A same-origin path that the page fetches an asset from. The server matches
/// the path back to the file the build copied there, so a typo loses the
/// asset, and no person reads the path.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssetPaths {}

impl sealed::Sealed for AssetPaths {}
impl KeyKind for AssetPaths {
    const NAME: &'static str = "AssetPath";
}

/// The bare path, as a log line names the asset it fetched.
impl fmt::Display for StaticKey<AssetPaths> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.text)
    }
}

/// The name or the help text of a metric, or the name of one of its labels.
/// A `const` table of the relay or of the SFU names each one, so no metric
/// name is built from a runtime value (`ZK-029`).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetricNames {}

impl sealed::Sealed for MetricNames {}
impl KeyKind for MetricNames {
    const NAME: &'static str = "MetricName";
}

/// The value of a metric label. Each one is the spelling of a variant of a
/// closed `enum`. So a label carries no runtime value: no identifier and no
/// address. The count of series stays bounded (`ZK-029`).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum LabelValues {}

impl sealed::Sealed for LabelValues {}
impl KeyKind for LabelValues {
    const NAME: &'static str = "LabelValue";
}

impl StaticKey<Canvases> {
    /// The content canvas, the central one that every application paints.
    pub const CONTENT: Self = Self::new("ambre-canvas");
}

/// The bare id, as the DOM takes it. A canvas id alone prints: a log line
/// names the element it could not find.
impl fmt::Display for StaticKey<Canvases> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.text)
    }
}

#[cfg(test)]
mod tests {
    use super::{Canvases, CommandNames, Commands, StaticKey};

    #[test]
    fn a_command_name_prints_its_bare_words_and_its_old_debug_form() {
        let name = StaticKey::<CommandNames>::new("typing");
        assert_eq!(name.to_string(), "typing");
        assert_eq!(format!("{name:?}"), "CommandName(\"typing\")");
    }

    #[test]
    fn a_key_prints_its_kind_and_a_canvas_id_its_bare_text() {
        assert_eq!(StaticKey::<Canvases>::CONTENT.to_string(), "ambre-canvas");
        assert_eq!(
            format!("{:?}", StaticKey::<Canvases>::new("x")),
            "CanvasId(\"x\")"
        );
        assert_eq!(
            format!("{:?}", StaticKey::<Commands>::new("a.b")),
            "CommandId(\"a.b\")"
        );
    }

    #[test]
    fn each_new_kind_prints_its_own_name() {
        use super::{AssetPaths, Attributes, CssClasses, StorageKeys};
        let asset = StaticKey::<AssetPaths>::new("/fonts/a.ttf");
        assert_eq!(format!("{asset:?}"), "AssetPath(\"/fonts/a.ttf\")");
        assert_eq!(asset.to_string(), "/fonts/a.ttf");
        assert_eq!(
            format!("{:?}", StaticKey::<StorageKeys>::new("a.b")),
            "StorageKey(\"a.b\")"
        );
        assert_eq!(
            format!("{:?}", StaticKey::<CssClasses>::new("c")),
            "CssClass(\"c\")"
        );
        assert_eq!(
            format!("{:?}", StaticKey::<Attributes>::new("data-x")),
            "AttributeName(\"data-x\")"
        );
    }

    #[test]
    fn a_constant_key_stands_in_a_pattern() {
        const BOLD: StaticKey<Commands> = StaticKey::<Commands>::new("bold");
        assert!(matches!(StaticKey::<Commands>::new("bold"), BOLD));
    }
}
