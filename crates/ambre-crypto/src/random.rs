//! CSPRNG helper.
//!
//! Uses `getrandom`, which maps to the OS entropy source natively and to
//! `crypto.getRandomValues` on `wasm32-unknown-unknown` (feature `wasm_js`).

/// Fills `buf` with cryptographically secure random bytes.
///
/// Panics only if the platform entropy source is unavailable, which on our
/// targets (browser, native) is not a recoverable condition.
#[expect(
    clippy::expect_used,
    reason = "a key drawn from a failed entropy source must never be used, so stopping is the safe end"
)]
pub(crate) fn fill(buf: &mut [u8]) {
    getrandom::fill(buf).expect("platform CSPRNG unavailable");
}

/// Returns `N` cryptographically secure random bytes.
pub(crate) fn bytes<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    fill(&mut out);
    out
}
