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

use ambre_core::Secret;
use ambre_protocol::ws::AccessToken;
use ed25519_dalek::SigningKey;
use ed25519_dalek::pkcs8::EncodePrivateKey;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

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

/// The Ed25519 key the access tokens are signed under, from the 32-byte seed
/// `JWT_SIGNING_SEED` names, or drawn at a dev startup.
///
/// It holds the key, not the seed. `ed25519_dalek::SigningKey` wipes its
/// bytes on drop and derives no `Clone` here, so the relay never reads the
/// seed after the key is built (`SRV-235`). Its `Debug` form prints none of
/// the bytes and not their length (`SRV-236`).
pub struct JwtSeed(SigningKey);

impl std::fmt::Debug for JwtSeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("JwtSeed([REDACTED])")
    }
}

/// Why a `JWT_SIGNING_SEED` value is refused. Neither case quotes the text,
/// since the text is the seed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JwtSeedRefusal {
    #[error("JWT_SIGNING_SEED is not valid hex")]
    NotHex,
    #[error("JWT_SIGNING_SEED must be 32 bytes (64 hex chars), not {0}")]
    Length(usize),
}

/// Builds a signing key from a seed `fill` writes. The seed is written into a
/// [`Secret`] and wiped when the key is built, or when `fill` fails. No read
/// of the wrapper is needed: the key is built inside the fill.
fn signing_key<E>(fill: impl FnOnce(&mut [u8; 32]) -> Result<(), E>) -> Result<SigningKey, E> {
    // The placeholder is replaced before the key is returned; it wipes on
    // that drop as any signing key does.
    let mut key = SigningKey::from_bytes(&[0; 32]);
    Secret::try_filled(|seed: &mut [u8; 32]| {
        fill(seed)?;
        key = SigningKey::from_bytes(seed);
        Ok(())
    })?;
    Ok(key)
}

impl JwtSeed {
    /// The key a fixed seed `bytes` names, for a test.
    #[cfg(test)]
    pub fn from_array(bytes: [u8; 32]) -> Self {
        Self(SigningKey::from_bytes(&bytes))
    }

    /// Reads the seed from its hex text, which is a secret too. The bytes
    /// decode in place into a wrapper, and the text is wiped once it is read.
    /// So no copy of either is left behind, whether the seed is accepted or
    /// refused.
    ///
    /// # Errors
    ///
    /// [`JwtSeedRefusal::NotHex`] when the text is not hex, and
    /// [`JwtSeedRefusal::Length`] when it decodes to other than 32 bytes.
    pub fn from_hex(text: String) -> Result<Self, JwtSeedRefusal> {
        let key = signing_key(|seed| {
            let text = text.trim();
            hex::decode_to_slice(text, seed).map_err(|e| match e {
                hex::FromHexError::InvalidStringLength
                    if text.bytes().all(|b| b.is_ascii_hexdigit()) =>
                {
                    JwtSeedRefusal::Length(text.len() / 2)
                }
                _ => JwtSeedRefusal::NotHex,
            })
        });
        drop(Secret::new(text));
        key.map(Self)
    }

    /// Draws a fresh seed from the platform CSPRNG, in place.
    ///
    /// # Errors
    ///
    /// The error of `getrandom` when the platform has no CSPRNG.
    pub fn generate() -> Result<Self, getrandom::Error> {
        signing_key(|seed| getrandom::fill(seed)).map(Self)
    }
}

/// Server signing/verifying material, derived once from the seed at startup.
pub struct JwtKeys {
    encoding: EncodingKey,
    decoding: DecodingKey,
    validation: Validation,
}

impl JwtKeys {
    /// Build the key pair from the Ed25519 signing key.
    ///
    /// `jsonwebtoken` (ring) wants the private key as PKCS#8 DER and the public
    /// key as raw 32 bytes, so we convert via ed25519-dalek's `pkcs8` support.
    /// The DER document wipes itself, but `EncodingKey` keeps its own copy of
    /// the bytes, unwiped, for the life of the process (`ZK-019`).
    ///
    /// # Panics
    ///
    /// Never: an Ed25519 key always encodes to PKCS#8.
    #[expect(
        clippy::expect_used,
        reason = "an Ed25519 key always encodes to PKCS#8, so the conversion cannot fail"
    )]
    pub fn from_seed(seed: &JwtSeed) -> Self {
        let signing = &seed.0;
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
            crate::secrets::decode_access::<Claims>(token, &self.decoding, &self.validation)?
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
        let seed = JwtSeed::from_hex(" ".to_owned() + &"ab".repeat(32)).expect("64 hex chars");
        wiped_on_drop(&seed.0);
        assert_eq!(seed.0.to_bytes(), [0xab; 32]);
        assert_eq!(format!("{seed:?}"), "JwtSeed([REDACTED])");
        assert_eq!(
            JwtSeed::from_hex("ab".repeat(31)).err(),
            Some(JwtSeedRefusal::Length(31))
        );
        assert_eq!(
            JwtSeed::from_hex("zz".to_owned()).err(),
            Some(JwtSeedRefusal::NotHex)
        );
        assert_eq!(
            JwtSeed::from_hex("abc".to_owned()).err(),
            Some(JwtSeedRefusal::NotHex)
        );
        // The refusal quotes no character of the seed.
        let refused = JwtSeed::from_hex("abQ".repeat(21) + "a").err().unwrap();
        assert_eq!(refused.to_string(), "JWT_SIGNING_SEED is not valid hex");
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
