//! The one module of the relay that reads a secret (`ARCH-048`).
//!
//! The relay holds secrets that it must hand on:
//!
//!   * the access token, which it verifies;
//!   * the refresh token and the sign-in state, which name a record of its
//!     store;
//!   * the OIDC client secret and PKCE verifier, which it presents to the
//!     provider, and the code, which it trades there.
//!
//! Each sits in an [`ambre_core::Secret`]. The primitive gate admits a read
//! of one here, and nowhere else in the relay. The rest of the relay calls
//! the typed functions below. The relay links no `ambre-crypto` (`ZK-004`),
//! so no key of a document is read here: this module reads a credential only
//! to hand it to the one foreign call that needs it.
//!
//! The foreign call owns its copy. `jsonwebtoken`, `openidconnect` and
//! `rustis` take a `&str` or a `String`, and do not wipe it. A store key that
//! carries a token is such a copy too. A reviewer holds that each function
//! below hands the text to one such call and to nothing else (`ARCH-047`).

mod oidc;
mod token;

pub use oidc::{OidcSecret, authorization_code};
pub use token::{decode_access, oidc_state_key, refresh_key};

#[cfg(test)]
mod tests;
