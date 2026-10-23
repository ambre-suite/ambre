//! Access-token signing and verification (EdDSA / Ed25519).
//!
//! The access token is a stateless JWT. The server verifies its signature
//! locally and reads the identity straight from the claims, with **no Redis or
//! Postgres lookup** on the request path. It carries exactly what the old Redis
//! session hash held (`user_id`, `signing_pubkey`), so every authenticated
//! handler has what it needs from the token alone.
//!
//! Revocation lives on the *refresh* token (opaque, stored in Redis, see
//! [`crate::redis_ops`]), not here: an access token is valid until its `exp`.
//!
//! Signing uses a single server Ed25519 key. In a multi-instance deployment the
//! seed MUST be shared (`JWT_SIGNING_SEED`) so any instance can verify a token
//! minted by another.

use std::collections::HashSet;

use ambre_protocol::ws::AccessToken;
use ed25519_dalek::SigningKey;
use ed25519_dalek::pkcs8::EncodePrivateKey;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

/// Access-token claims. `sub` is the user id, as its hyphenated text.
/// `spk` is the hex Ed25519 signing public key (used server-side to verify WS
/// message signatures without a DB hit). `iat`/`exp` are standard Unix-seconds timestamps.
///
/// `spk` is absent on the session of an account that has not registered its
/// keys yet. Such a session carries no identity to sign under (`SRV-027`).
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    #[serde(with = "ambre_core::serde_id::user_id")]
    pub sub: ambre_core::UserId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spk: Option<ambre_core::SigningKeyHex>,
    pub iat: ambre_core::UnixSeconds,
    pub exp: ambre_core::UnixSeconds,
}

/// The 32-byte Ed25519 seed the access tokens are signed under, read from
/// `JWT_SIGNING_SEED` or drawn at a dev startup.
///
/// Wiped on drop and never cloned (`SRV-235`). Its `Debug` form prints the
/// length and none of the bytes (`SRV-236`).
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct JwtSeed([u8; 32]);

/// Why a `JWT_SIGNING_SEED` value is refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JwtSeedRefusal {
    #[error("JWT_SIGNING_SEED is not valid hex: {0}")]
    NotHex(String),
    #[error("JWT_SIGNING_SEED must be 32 bytes (64 hex chars), not {0}")]
    Length(usize),
}

impl JwtSeed {
    /// The seed `bytes` name, for a fixed seed in a test.
    #[cfg(test)]
    pub const fn from_array(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Reads the seed from its hex text. The decoded bytes are wiped too,
    /// whether the seed is accepted or refused.
    ///
    /// # Errors
    ///
    /// [`JwtSeedRefusal::NotHex`] when the text is not hex, and
    /// [`JwtSeedRefusal::Length`] when it decodes to other than 32 bytes.
    pub fn from_hex(text: &str) -> Result<Self, JwtSeedRefusal> {
        let bytes = Zeroizing::new(
            hex::decode(text.trim()).map_err(|e| JwtSeedRefusal::NotHex(e.to_string()))?,
        );
        let mut seed = Self([0u8; 32]);
        if bytes.len() != seed.0.len() {
            return Err(JwtSeedRefusal::Length(bytes.len()));
        }
        seed.0.copy_from_slice(&bytes);
        Ok(seed)
    }

    /// Draws a fresh seed from the platform CSPRNG.
    ///
    /// # Errors
    ///
    /// The error of `getrandom` when the platform has no CSPRNG.
    pub fn generate() -> Result<Self, getrandom::Error> {
        let mut seed = Self([0u8; 32]);
        getrandom::fill(&mut seed.0)?;
        Ok(seed)
    }
}

impl std::fmt::Debug for JwtSeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "JwtSeed({} bytes)", self.0.len())
    }
}

/// Server signing/verifying material, derived once from the seed at startup.
pub struct JwtKeys {
    encoding: EncodingKey,
    decoding: DecodingKey,
    validation: Validation,
}

impl JwtKeys {
    /// Build the key pair from a 32-byte Ed25519 seed.
    ///
    /// `jsonwebtoken` (ring) wants the private key as PKCS#8 DER and the public
    /// key as raw 32 bytes, so we convert via ed25519-dalek's `pkcs8` support.
    ///
    /// # Panics
    ///
    /// Never: an Ed25519 key always encodes to PKCS#8.
    #[expect(
        clippy::expect_used,
        reason = "an Ed25519 key always encodes to PKCS#8, so the conversion cannot fail"
    )]
    pub fn from_seed(seed: &JwtSeed) -> Self {
        let signing = SigningKey::from_bytes(&seed.0);
        let pkcs8 = signing
            .to_pkcs8_der()
            .expect("ed25519 signing key to PKCS#8 DER");
        let encoding = EncodingKey::from_ed_der(pkcs8.as_bytes());
        let decoding = DecodingKey::from_ed_der(signing.verifying_key().as_bytes());

        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.required_spec_claims = HashSet::from(["exp".to_string()]);
        validation.validate_exp = true;
        // No audience/issuer checks: this is a single-tenant access token.
        validation.validate_aud = false;

        Self {
            encoding,
            decoding,
            validation,
        }
    }

    /// Mint a signed access token for `claims`.
    pub fn mint(&self, claims: &Claims) -> Result<AccessToken, jsonwebtoken::errors::Error> {
        jsonwebtoken::encode(&Header::new(Algorithm::EdDSA), claims, &self.encoding)
            .map(AccessToken::new)
    }

    /// Verify a token's signature and expiry, returning its claims. A `sub`
    /// that is not a user id fails the decode.
    pub fn verify(&self, token: &AccessToken) -> Result<Claims, jsonwebtoken::errors::Error> {
        Ok(
            jsonwebtoken::decode::<Claims>(token.expose(), &self.decoding, &self.validation)?
                .claims,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use ambre_core::{Seconds, SigningKeyHex, UnixSeconds};

    fn now() -> UnixSeconds {
        crate::s3::unix_now()
    }

    #[test]
    fn mint_then_verify_roundtrips() {
        let keys = JwtKeys::from_seed(&crate::jwt::JwtSeed::from_array([7u8; 32]));
        let iat = now();
        let claims = Claims {
            sub: ambre_core::UserId::parse_str("11111111-1111-7111-8111-111111111111").unwrap(),
            spk: Some(SigningKeyHex("ab".repeat(32))),
            iat,
            exp: iat.plus(Seconds(900)),
        };
        let token = keys.mint(&claims).expect("mint");
        let decoded = keys.verify(&token).expect("verify");
        assert_eq!(decoded.sub, claims.sub);
        assert_eq!(decoded.spk, claims.spk);
    }

    #[test]
    fn a_jwt_seed_is_wiped_and_never_printed() {
        fn wiped_on_drop<T: zeroize::ZeroizeOnDrop>(_: &T) {}
        let seed = JwtSeed::from_hex(&"ab".repeat(32)).expect("64 hex chars");
        wiped_on_drop(&seed);
        assert_eq!(format!("{seed:?}"), "JwtSeed(32 bytes)");
        assert_eq!(
            JwtSeed::from_hex(&"ab".repeat(31)).err(),
            Some(JwtSeedRefusal::Length(31))
        );
        assert!(matches!(
            JwtSeed::from_hex("zz"),
            Err(JwtSeedRefusal::NotHex(_))
        ));
    }

    #[test]
    fn expired_token_is_rejected() {
        let keys = JwtKeys::from_seed(&crate::jwt::JwtSeed::from_array([9u8; 32]));
        let iat = UnixSeconds(now().0 - 10_000);
        let claims = Claims {
            sub: ambre_core::UserId::new(),
            spk: Some(SigningKeyHex("y".into())),
            iat,
            exp: iat.plus(Seconds(1)), // long past
        };
        let token = keys.mint(&claims).expect("mint");
        assert!(keys.verify(&token).is_err());
    }

    #[test]
    fn wrong_key_is_rejected() {
        let a = JwtKeys::from_seed(&crate::jwt::JwtSeed::from_array([1u8; 32]));
        let b = JwtKeys::from_seed(&crate::jwt::JwtSeed::from_array([2u8; 32]));
        let iat = now();
        let token = a
            .mint(&Claims {
                sub: ambre_core::UserId::new(),
                spk: Some(SigningKeyHex("p".into())),
                iat,
                exp: iat.plus(Seconds(900)),
            })
            .expect("mint");
        assert!(b.verify(&token).is_err());
    }
}
