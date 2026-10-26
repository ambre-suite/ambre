//! A session token of the Sync worker: an access or a refresh token.
//!
//! It wipes its bytes when the last owner drops it, so a sign-out leaves no
//! token in the worker's memory (`SYNC-100`). Its `Debug` prints no byte of
//! it, so no log can carry one (`SYNC-061`). It lives here because the text
//! is read through [`ambre_core::Secret::expose`], which the primitive gate
//! admits in this crate alone (`ARCH-045`). The worker hands it to a foreign
//! call through [`SecretToken::present`], and never holds the text itself.

use std::sync::Arc;

use ambre_core::Secret;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A session token, serialized and parsed as the bare string the relay
/// exchanges. The token store and each request share one value, so a clone
/// copies the pointer, never the text.
#[derive(Clone, Debug, Default)]
pub struct SecretToken(Arc<Secret<String>>);

impl SecretToken {
    /// The wrapper, for a test that checks the token wipes on drop.
    pub fn secret(&self) -> &Secret<String> {
        &self.0
    }

    /// Hands the token's text to `read`, the one foreign call that carries
    /// it: a socket frame, a query. Nothing of the text outlives the call
    /// but what `read` copies.
    pub fn present<R>(&self, read: impl FnOnce(&str) -> R) -> R {
        read(self.0.expose())
    }

    /// Hands the `Authorization` header value, `Bearer` and the token, to
    /// `read`. The value is wiped once `read` returns.
    pub fn present_bearer<R>(&self, read: impl FnOnce(&str) -> R) -> R {
        const SCHEME: &str = "Bearer ";
        let token = self.0.expose();
        let header = Secret::filled(|header: &mut String| {
            header.reserve_exact(SCHEME.len() + token.len());
            header.push_str(SCHEME);
            header.push_str(token);
        });
        read(header.expose())
    }
}

impl From<String> for SecretToken {
    fn from(token: String) -> Self {
        Self(Arc::new(Secret::new(token)))
    }
}

impl Serialize for SecretToken {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.expose().as_str().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SecretToken {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::from)
    }
}

#[cfg(test)]
mod tests {
    use super::SecretToken;

    #[test]
    fn a_token_prints_nothing_and_travels_as_its_text() {
        let token = SecretToken::from("acc-s3cr3t".to_owned());
        assert_eq!(format!("{token:?}"), "SecretToken(Secret([REDACTED]))");
        assert_eq!(token.present(str::to_owned), "acc-s3cr3t");
        assert_eq!(token.present_bearer(str::to_owned), "Bearer acc-s3cr3t");
        let wire = serde_json::to_string(&token).unwrap();
        assert_eq!(wire, "\"acc-s3cr3t\"");
        let back: SecretToken = serde_json::from_str(&wire).unwrap();
        assert_eq!(back.present(str::to_owned), "acc-s3cr3t");
    }
}
