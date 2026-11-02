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

/// The id of a `<canvas>` element in the page. The shell mounts the element
/// with this id, and the canvas host finds it by the same id. Each id is a
/// constant of the code, so a distinct kind keeps it apart from page text.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Canvases {}

impl sealed::Sealed for Canvases {}
impl KeyKind for Canvases {
    const NAME: &'static str = "CanvasId";
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
    use super::{Canvases, Commands, StaticKey};

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
    fn a_constant_key_stands_in_a_pattern() {
        const BOLD: StaticKey<Commands> = StaticKey::<Commands>::new("bold");
        assert!(matches!(StaticKey::<Commands>::new("bold"), BOLD));
    }
}
