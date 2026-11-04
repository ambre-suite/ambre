//! The options of a Trame find or replace (`trame-find-replace`): the four
//! checkbox states of the panel. A [`FindQuery`](crate::app::find::FindQuery)
//! carries them in [`TrameCommand::FindNext`], [`TrameCommand::ReplaceOne`]
//! and [`TrameCommand::ReplaceAll`].
//!
//! [`TrameCommand::FindNext`]: super::TrameCommand::FindNext
//! [`TrameCommand::ReplaceOne`]: super::TrameCommand::ReplaceOne
//! [`TrameCommand::ReplaceAll`]: super::TrameCommand::ReplaceAll

pub use ambre_core::CaseMatch;

/// How much of a cell a match must cover.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MatchExtent {
    /// The query may sit anywhere in the cell, and each occurrence is replaced.
    #[default]
    Substring,
    /// The query must equal the cell, and a replace swaps the whole content.
    WholeCell,
}

/// Which text of a formula cell a replace may read and rewrite.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SearchScope {
    /// Formula cells are skipped, and a plain cell matches on its value.
    #[default]
    Values,
    /// A formula cell also matches, on its positional source text.
    Formulas,
}

/// Which sheets a replace-all walks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SheetScope {
    #[default]
    Active,
    All,
}

/// The options of a find or a replace (`trame-find-replace`). A find reads
/// only `case` and `extent`, and only a replace-all reads `sheets`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FindOptions {
    pub case: CaseMatch,
    pub extent: MatchExtent,
    pub scope: SearchScope,
    pub sheets: SheetScope,
}
