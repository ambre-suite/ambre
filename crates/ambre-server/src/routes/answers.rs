//! The answer bodies several routes share. Each answer of the relay is a
//! type (`SRV-181`). A renamed field or a new word is then a compile error,
//! not a silent change of the wire.

use ambre_core::WriteStatus;

/// The answer of a write whose outcome is the whole answer.
#[derive(serde::Serialize)]
pub struct Done {
    pub status: WriteStatus,
}

impl Done {
    /// The answer of a write that landed with this outcome.
    pub(crate) fn json(status: WriteStatus) -> axum::Json<Self> {
        axum::Json(Self { status })
    }
}

/// Rows listed per document, under `documents`.
#[derive(serde::Serialize)]
pub struct DocumentRows<T> {
    pub documents: Vec<T>,
}

impl<T> DocumentRows<T> {
    /// The answer that lists these rows.
    pub(crate) fn json(documents: Vec<T>) -> axum::Json<Self> {
        axum::Json(Self { documents })
    }
}
