//! The three forms a cell's text takes (`TRAME-228`, `trame-sheets`).
//!
//! The user writes a formula in its source form. The CRDT stores it in a
//! canonical form, and the formula bar shows it in a display form. Each is a
//! type of its own, so no call reads one where another is due. Plain text, and
//! a formula that fails to parse, read the same in all three.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

use ambre_core::Redacted;

/// The marker of one cell-text form. Sealed: the three forms live in this
/// module.
pub trait TextForm: sealed::Sealed + 'static {
    /// The name `Debug` prints, as the hand-written type printed its own.
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames>;
}

mod sealed {
    /// Closes [`super::TextForm`].
    pub trait Sealed {}
}

/// A cell's text in the form `F`. Every text is a valid one, and each is kept
/// verbatim: plain text, a blank cell's empty text, a formula that fails to
/// parse. A cell's text is the author's: a log carries its length alone.
pub struct CellText<F: TextForm> {
    text: Redacted<String>,
    form: PhantomData<fn() -> F>,
}

impl<F: TextForm> CellText<F> {
    /// The text `text`, in this form.
    pub fn new(text: String) -> Self {
        Self {
            text: Redacted::new(text),
            form: PhantomData,
        }
    }

    /// The text as it reads.
    pub fn as_str(&self) -> &str {
        self.text.as_str()
    }

    /// The text, given up.
    pub fn into_string(self) -> String {
        self.text.into_inner()
    }

    /// Whether the cell is blank.
    pub fn is_empty(&self) -> bool {
        self.as_str().is_empty()
    }

    /// The formula after its `=`, or `None` for plain text.
    pub fn formula_body(&self) -> Option<&str> {
        self.as_str().strip_prefix('=')
    }

    /// Whether the text is a formula, one that starts with `=`.
    pub fn is_formula(&self) -> bool {
        self.as_str().starts_with('=')
    }
}

// By hand, every impl below: a derive would bound the marker, which has no
// value to clone, compare or hash.

impl<F: TextForm> Clone for CellText<F> {
    fn clone(&self) -> Self {
        Self::new(self.as_str().to_owned())
    }
}

impl<F: TextForm> Default for CellText<F> {
    fn default() -> Self {
        Self::new(String::new())
    }
}

impl<F: TextForm> PartialEq for CellText<F> {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
    }
}

impl<F: TextForm> Eq for CellText<F> {}

impl<F: TextForm> Hash for CellText<F> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text.hash(state);
    }
}

/// `NAME(N chars)`, as the hand-written type printed itself.
impl<F: TextForm> fmt::Debug for CellText<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple(F::NAME.as_str()).field(&self.text).finish()
    }
}

/// A cell's text with each reference at its position: what the user types,
/// what a fill or a paste shifts, and what a copy carries. A qualifier
/// names its sheet by name or by id; a write binds it to the id.
pub enum SourceForm {}

impl sealed::Sealed for SourceForm {}
impl TextForm for SourceForm {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("FormulaSource");
}

/// A cell's text as the CRDT holds it. Each qualifier is bound to its
/// sheet id, and each reference to the row and column it names. A rename,
/// a reorder, an insert or a delete never moves it.
pub enum StoredForm {}

impl sealed::Sealed for StoredForm {}
impl TextForm for StoredForm {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("StoredFormula");
}

/// A cell's text as the formula bar shows it: each reference at its
/// position, each qualifier through the sheet's current name.
pub enum DisplayForm {}

impl sealed::Sealed for DisplayForm {}
impl TextForm for DisplayForm {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("DisplayFormula");
}
