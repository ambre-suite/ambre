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

use ed25519_dalek::SigningKey;
use ed25519_dalek::pkcs8::EncodePrivateKey;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

/// Access-token claims. `sub` is the user id, and `spk` the hex Ed25519
/// signing public key (used server-side to verify WS message signatures
/// without a DB hit). `iat`/`exp` are standard Unix-seconds timestamps.
///
/// `spk` is absent on the session of an account that has not registered its
/// keys yet. Such a session carries no identity to sign under (`SRV-027`).
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spk: Option<String>,
    pub iat: u64,
    pub exp: u64,
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
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        let signing = SigningKey::from_bytes(seed);
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
    pub fn mint(&self, claims: &Claims) -> Result<String, jsonwebtoken::errors::Error> {
        jsonwebtoken::encode(&Header::new(Algorithm::EdDSA), claims, &self.encoding)
    }

    /// Verify a token's signature and expiry, returning its claims.
    pub fn verify(&self, token: &str) -> Result<Claims, jsonwebtoken::errors::Error> {
        Ok(jsonwebtoken::decode::<Claims>(token, &self.decoding, &self.validation)?.claims)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    #[test]
    fn mint_then_verify_roundtrips() {
        let keys = JwtKeys::from_seed(&[7u8; 32]);
        let iat = now();
        let claims = Claims {
            sub: "11111111-1111-7111-8111-111111111111".into(),
            spk: Some("ab".repeat(32)),
            iat,
            exp: iat + 900,
        };
        let token = keys.mint(&claims).expect("mint");
        let decoded = keys.verify(&token).expect("verify");
        assert_eq!(decoded.sub, claims.sub);
        assert_eq!(decoded.spk, claims.spk);
    }

    #[test]
    fn expired_token_is_rejected() {
        let keys = JwtKeys::from_seed(&[9u8; 32]);
        let iat = now() - 10_000;
        let claims = Claims {
            sub: "x".into(),
            spk: Some("y".into()),
            iat,
            exp: iat + 1, // long past
        };
        let token = keys.mint(&claims).expect("mint");
        assert!(keys.verify(&token).is_err());
    }

    #[test]
    fn wrong_key_is_rejected() {
        let a = JwtKeys::from_seed(&[1u8; 32]);
        let b = JwtKeys::from_seed(&[2u8; 32]);
        let iat = now();
        let token = a
            .mint(&Claims {
                sub: "s".into(),
                spk: Some("p".into()),
                iat,
                exp: iat + 900,
            })
            .expect("mint");
        assert!(b.verify(&token).is_err());
    }
}
