//! The three forms a cell's text takes (`TRAME-228`, `trame-sheets`).
//!
//! The user writes a formula in its source form. The CRDT stores it in a
//! canonical form, and the formula bar shows it in a display form. Each is a
//! type of its own, so no call reads one where another is due. Plain text, and
//! a formula that fails to parse, read the same in all three.

/// One cell-text form. Every text is a valid one, and each is kept verbatim:
/// plain text, a blank cell's empty text, a formula that fails to parse.
macro_rules! cell_text {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Default, PartialEq, Eq, Hash)]
        pub struct $name(String);

        // A cell's text is the author's: a log carries its length alone.
        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_tuple(stringify!($name))
                    .field(&ambre_core::char_len(&self.0))
                    .finish()
            }
        }

        impl $name {
            /// The text `text`, in this form.
            pub fn new(text: String) -> Self {
                Self(text)
            }

            /// The text as it reads.
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// The text, given up.
            pub fn into_string(self) -> String {
                self.0
            }

            /// Whether the cell is blank.
            pub fn is_empty(&self) -> bool {
                self.0.is_empty()
            }

            /// The formula after its `=`, or `None` for plain text.
            pub fn formula_body(&self) -> Option<&str> {
                self.0.strip_prefix('=')
            }

            /// Whether the text is a formula, one that starts with `=`.
            pub fn is_formula(&self) -> bool {
                self.0.starts_with('=')
            }
        }
    };
}

cell_text! {
    /// A cell's text with each reference at its position: what the user types,
    /// what a fill or a paste shifts, and what a copy carries. A qualifier
    /// names its sheet by name or by id; a write binds it to the id.
    FormulaSource
}

cell_text! {
    /// A cell's text as the CRDT holds it. Each qualifier is bound to its
    /// sheet id, and each reference to the row and column it names. A rename,
    /// a reorder, an insert or a delete never moves it.
    StoredFormula
}

cell_text! {
    /// A cell's text as the formula bar shows it: each reference at its
    /// position, each qualifier through the sheet's current name.
    DisplayFormula
}
