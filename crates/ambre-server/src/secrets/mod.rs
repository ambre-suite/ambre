//! The one module of the relay that reads a secret (`ARCH-048`).
//!
//! The relay holds secrets that it must hand on:
//!
//!   * the access token, which it verifies;
//!   * the refresh token and the sign-in state, which name a record of its
//!     store;
//!   * the OIDC client secret and PKCE verifier, which it presents to the
//!     provider, and the code, which it trades there;
//!   * the hex text of the token signing seed, which it decodes once.
//!
//! Each sits in an [`ambre_core::Secret`]. The primitive gate admits a read
//! of one here, and nowhere else in the relay. The rest of the relay calls
//! the typed functions below. The relay links no `ambre-crypto` (`ZK-004`),
//! so no key of a document is read here. This module reads a credential only
//! to hand it to the one foreign call that needs it.
//!
//! Each function below is a sink (`ARCH-049`). The gate admits its name in
//! the one file that makes its foreign call, and in its own file here. So
//! no other file of the relay reaches the text through it, and no bare
//! `String` of a secret leaves this module.
//!
//! The foreign call owns its copy. `jsonwebtoken`, `openidconnect` and
//! `rustis` take a `&str` or a `String`, and do not wipe it. A reviewer
//! holds that each function below hands the text to one such call and to
//! nothing else (`ARCH-047`).

pub mod oidc;
pub mod seed;
pub mod token;

pub use oidc::{OidcSecret, OidcSecretSinks};
pub use seed::SeedText;

#[cfg(test)]
mod tests;
