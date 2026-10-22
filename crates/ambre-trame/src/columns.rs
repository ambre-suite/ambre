//! Canonical spreadsheet column-label conversions.
//!
//! A Trame grid has up to [`crate::TOTAL_COLS`] columns, well past 26, so a
//! label needs the full bijective base-26 encoding (A..Z, AA..ZZ, AAA..).
//! Previously four near-identical copies of this logic lived across `formula`,
//! `logic`, `model`, and the UI/chrome modules. Several of them were the buggy
//! single-letter `(b'A' + col % 26)` form that wraps at column 26. This module
//! is the single source of truth; every call site delegates here.

use ambre_bus::{Col, Row};

/// Convert a 0-based column index to its spreadsheet letters.
///
/// `0 -> "A"`, `25 -> "Z"`, `26 -> "AA"`, `701 -> "ZZ"`, `702 -> "AAA"`.
pub fn col_to_alpha(col: Col) -> String {
    let mut col = col.get();
    let mut chars = Vec::new();
    loop {
        chars.push((b'A' + (col % 26) as u8) as char);
        if col < 26 {
            break;
        }
        col = col / 26 - 1;
    }
    chars.into_iter().rev().collect()
}

/// Convert spreadsheet column letters back to a 0-based index.
///
/// Inverse of [`col_to_alpha`]. Rejects an empty or non-`A..Z` string.
///
/// # Errors
///
/// A message naming `s` when it is empty or holds a byte outside `A..Z`, and
/// another when the accumulated index overflows a `u32`.
pub fn alpha_to_col(s: &str) -> Result<Col, String> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err(format!("invalid column reference: {s}"));
    }
    let mut col = 0u32;
    for b in s.bytes() {
        col = col
            .checked_mul(26)
            .and_then(|c| c.checked_add((b - b'A') as u32 + 1))
            .ok_or_else(|| format!("column overflow: {s}"))?;
    }
    Ok(Col::new(col.saturating_sub(1)))
}

/// The label a user reads for a 0-based row: its 1-based number.
pub fn row_label(row: Row) -> String {
    (u64::from(row.get()) + 1).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn col_to_alpha_basics() {
        for (col, alpha) in [
            (0, "A"),
            (25, "Z"),
            (26, "AA"),
            (51, "AZ"),
            (52, "BA"),
            (701, "ZZ"),
            (702, "AAA"),
        ] {
            assert_eq!(col_to_alpha(Col::new(col)), alpha);
        }
    }

    #[test]
    fn alpha_to_col_roundtrip() {
        for col in [0, 1, 25, 26, 51, 52, 100, 701, 702, 999] {
            let alpha = col_to_alpha(Col::new(col));
            assert_eq!(
                alpha_to_col(&alpha),
                Ok(Col::new(col)),
                "roundtrip for {alpha}"
            );
        }
    }

    #[test]
    fn alpha_to_col_invalid() {
        assert!(alpha_to_col("").is_err());
        assert!(alpha_to_col("a").is_err());
        assert!(alpha_to_col("A1").is_err());
    }
}
