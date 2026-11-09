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
use ambre_core::id::{Id, User};

use crate::secrets::SeedText;
use ambre_core::CallAdmission;
use ambre_core::{Instant, UnixSec};
use ambre_protocol::signaling::Admission;
use ambre_protocol::ws::{AccessToken, Credential};
use ed25519_dalek::Signer;
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
    #[serde(with = "ambre_core::serde_id::text")]
    pub sub: Id<User>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spk: Option<ambre_core::FixedBytes<ambre_core::PublicKey>>,
    pub iat: Instant<UnixSec>,
    pub exp: Instant<UnixSec>,
}

/// The Ed25519 key the access tokens are signed under, from the 32-byte seed
/// `JWT_SIGNING_SEED` names, or drawn at a dev startup.
///
/// It holds the key, not the seed. `ed25519_dalek::SigningKey` wipes its
/// bytes on drop and derives no `Clone` here, so the relay never reads the
/// seed after the key is built (`SRV-235`). The key sits in a `Box`, so a
/// move of a `JwtSeed` copies the pointer alone. The build of the key moves
/// it on the stack before it is boxed, and nothing wipes those stack copies.
/// Its `Debug` form prints none of the bytes and not their length
/// (`SRV-236`).
pub struct JwtSeed(Box<SigningKey>);

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
    #[error("JWT_SIGNING_SEED must be 32 bytes (64 hex chars), not {}", .0.get())]
    Length(ambre_core::Count<ambre_core::Octets>),
}

/// Why the relay signs no admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AdmissionUnsigned {
    /// `CALL_ADMISSION_SEED` set no key.
    #[error("no admission key is set")]
    NoKey,
    /// The admission does not encode.
    #[error("an admission does not encode")]
    Encode,
}

/// Builds a signing key from a seed `fill` writes. The seed is written into a
/// [`Secret`] and wiped when the key is built, or when `fill` fails. No read
/// of the wrapper is needed: the key is built inside the fill.
pub(crate) fn signing_key<E>(
    fill: impl FnOnce(&mut [u8; 32]) -> Result<(), E>,
) -> Result<SigningKey, E> {
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
        Self(Box::new(SigningKey::from_bytes(&bytes)))
    }

    /// Reads the seed from `JWT_SIGNING_SEED`, whose hex text is a secret
    /// too. The text goes straight into a [`SeedText`], so no bare copy of
    /// it leaves this function, and it is wiped once the key is built.
    /// `None` when the variable is not set.
    ///
    /// # Errors
    ///
    /// As [`JwtSeed::from_text`], and [`JwtSeedRefusal::NotHex`] when the
    /// value is not Unicode. That value is dropped unwiped, since
    /// `std::env::var` hands it back as an `OsString`.
    pub fn from_env() -> Result<Option<Self>, JwtSeedRefusal> {
        match std::env::var("JWT_SIGNING_SEED") {
            Ok(text) => Self::from_text(&SeedText::new(text)).map(Some),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(std::env::VarError::NotUnicode(_)) => Err(JwtSeedRefusal::NotHex),
        }
    }

    /// Reads the seed of the admission key from `CALL_ADMISSION_SEED`, as
    /// [`JwtSeed::from_env`] reads the access-token seed. `None` when the variable is not set.
    ///
    /// # Errors
    ///
    /// As [`JwtSeed::from_env`]. A refusal names `JWT_SIGNING_SEED`, so the caller names the
    /// variable it read.
    pub fn admission_from_env() -> Result<Option<Self>, JwtSeedRefusal> {
        match std::env::var("CALL_ADMISSION_SEED") {
            Ok(text) => Self::from_text(&SeedText::new(text)).map(Some),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(std::env::VarError::NotUnicode(_)) => Err(JwtSeedRefusal::NotHex),
        }
    }

    /// Reads the seed from its hex text. The bytes decode in place into a
    /// wrapper, so no copy of them is left behind, whether the seed is
    /// accepted or refused.
    ///
    /// # Errors
    ///
    /// [`JwtSeedRefusal::NotHex`] when the text is not hex, and
    /// [`JwtSeedRefusal::Length`] when it decodes to other than 32 bytes.
    pub fn from_text(text: &SeedText) -> Result<Self, JwtSeedRefusal> {
        text.seed_key().map(|key| Self(Box::new(key)))
    }

    /// Draws a fresh seed from the platform CSPRNG, in place.
    ///
    /// # Errors
    ///
    /// The error of `getrandom` when the platform has no CSPRNG.
    pub fn generate() -> Result<Self, getrandom::Error> {
        signing_key(|seed| getrandom::fill(seed)).map(|key| Self(Box::new(key)))
    }
}

/// Server signing/verifying material, derived once from the seed at startup.
pub struct JwtKeys {
    encoding: EncodingKey,
    decoding: DecodingKey,
    validation: Validation,
    /// The key of one purpose that signs the admissions of a call, from `CALL_ADMISSION_SEED`
    /// (`ambre-spec-call-media.md` section 3). It is never the access-token key, so either one
    /// rotates alone, and the SFU holds no key that verifies a session. `None` until
    /// [`JwtKeys::with_admission`] sets it: a call is then refused. It wipes itself on drop.
    admission: Option<Box<SigningKey>>,
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
            admission: None,
        }
    }

    /// The keys, with `seed` signing the admissions of a call. The caller passes the seed of
    /// `CALL_ADMISSION_SEED`, never the access-token seed.
    pub fn with_admission(mut self, seed: &JwtSeed) -> Self {
        self.admission = Some(Box::new(seed.0.as_ref().clone()));
        self
    }

    /// The token that admits one participant to the SFU: `admission`, signed. The relay mints
    /// it, which `SRV-012` allows, and holds no key of the SFU.
    ///
    /// # Errors
    ///
    /// [`AdmissionUnsigned::NoKey`] when no admission key is set, and
    /// [`AdmissionUnsigned::Encode`] for an encoder error, which fixed fields never meet.
    pub fn sign_admission(
        &self,
        admission: &Admission,
    ) -> Result<Credential<CallAdmission>, AdmissionUnsigned> {
        let key = self.admission.as_ref().ok_or(AdmissionUnsigned::NoKey)?;
        let signed = admission
            .signed_bytes()
            .map_err(|_| AdmissionUnsigned::Encode)?;
        let signature = key.sign(signed.as_bytes()).to_bytes();
        admission
            .token(&ambre_core::FixedBytes::from_array(signature))
            .map_err(|_| AdmissionUnsigned::Encode)
    }

    /// The public key the SFU verifies an admission with: `SFU_ADMISSION_KEY` holds it in hex.
    /// `None` when no admission key is set.
    pub fn admission_key(&self) -> Option<ambre_core::FixedBytes<ambre_core::PublicKey>> {
        self.admission
            .as_ref()
            .map(|key| ambre_core::FixedBytes::from_array(key.verifying_key().to_bytes()))
    }

    /// Mint a signed access token for `claims`.
    pub fn mint(
        &self,
        claims: &Claims,
    ) -> Result<Credential<AccessToken>, jsonwebtoken::errors::Error> {
        jsonwebtoken::encode(&Header::new(Algorithm::EdDSA), claims, &self.encoding)
            .map(Credential::new)
    }

    /// Verify a token's signature and expiry, returning its claims. A `sub`
    /// that is not a user id fails the decode.
    pub fn verify(
        &self,
        token: &Credential<AccessToken>,
    ) -> Result<Claims, jsonwebtoken::errors::Error> {
        Ok(
            crate::secrets::token::decode_access::<Claims>(
                token,
                &self.decoding,
                &self.validation,
            )?
            .claims,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use ambre_core::{FixedBytes, Instant, Sec, TimeSpan, UnixSec};

    fn now() -> Instant<UnixSec> {
        crate::s3::unix_now()
    }

    #[test]
    fn mint_then_verify_roundtrips() {
        let keys = JwtKeys::from_seed(&crate::jwt::JwtSeed::from_array([7u8; 32]));
        let iat = now();
        let claims = Claims {
            sub: Id::<User>::parse_str("11111111-1111-7111-8111-111111111111").unwrap(),
            spk: Some(FixedBytes::from_array([0xab; 32])),
            iat,
            exp: iat.plus(TimeSpan::<Sec>::new(900)),
        };
        let token = keys.mint(&claims).expect("mint");
        let decoded = keys.verify(&token).expect("verify");
        assert_eq!(decoded.sub, claims.sub);
        assert_eq!(decoded.spk, claims.spk);
    }

    #[test]
    fn a_jwt_seed_is_wiped_and_never_printed() {
        fn wiped_on_drop<T: zeroize::ZeroizeOnDrop>(_: &T) {}
        fn read(text: String) -> Result<JwtSeed, JwtSeedRefusal> {
            JwtSeed::from_text(&SeedText::new(text))
        }
        let seed = read(" ".to_owned() + &"ab".repeat(32)).expect("64 hex chars");
        wiped_on_drop(&*seed.0);
        assert_eq!(seed.0.to_bytes(), [0xab; 32]);
        assert_eq!(format!("{seed:?}"), "JwtSeed([REDACTED])");
        assert_eq!(
            read("ab".repeat(31)).err(),
            Some(JwtSeedRefusal::Length(ambre_core::Count::<
                ambre_core::Octets,
            >::new(31)))
        );
        assert_eq!(
            JwtSeedRefusal::Length(ambre_core::Count::<ambre_core::Octets>::new(31)).to_string(),
            "JWT_SIGNING_SEED must be 32 bytes (64 hex chars), not 31"
        );
        assert_eq!(read("zz".to_owned()).err(), Some(JwtSeedRefusal::NotHex));
        assert_eq!(read("abc".to_owned()).err(), Some(JwtSeedRefusal::NotHex));
        // The refusal quotes no character of the seed.
        let refused = read("abQ".repeat(21) + "a").err().unwrap();
        assert_eq!(refused.to_string(), "JWT_SIGNING_SEED is not valid hex");
    }

    /// The relay signs the admission of a call under a key of its own, never the access-token
    /// key. It signs none until that key is set (`ambre-spec-call-media.md` section 3).
    #[test]
    fn an_admission_is_signed_under_a_key_of_its_own() {
        use ambre_core::{FixedBytes, Instant as At};
        let admission = Admission {
            room: FixedBytes::from_array([1; 16]),
            participant: FixedBytes::from_array([2; 16]),
            expires: At::<UnixSec>::new(60),
        };
        let token_seed = JwtSeed::from_array([7u8; 32]);
        let keys = JwtKeys::from_seed(&token_seed);
        assert_eq!(keys.admission_key(), None);
        assert_eq!(
            keys.sign_admission(&admission).err(),
            Some(AdmissionUnsigned::NoKey)
        );
        let keys = keys.with_admission(&JwtSeed::from_array([8u8; 32]));
        let admission_key = keys.admission_key().expect("an admission key");
        assert_ne!(
            admission_key.as_array(),
            token_seed.0.verifying_key().as_bytes(),
            "the admission key is not the access-token key"
        );
        assert!(keys.sign_admission(&admission).is_ok());
    }

    #[test]
    fn expired_token_is_rejected() {
        let keys = JwtKeys::from_seed(&crate::jwt::JwtSeed::from_array([9u8; 32]));
        let iat = Instant::<UnixSec>::new(now().get() - 10_000);
        let claims = Claims {
            sub: Id::<User>::mint(),
            spk: Some(FixedBytes::from_array([0x79; 32])),
            iat,
            exp: iat.plus(TimeSpan::<Sec>::new(1)), // long past
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
                sub: Id::<User>::mint(),
                spk: Some(FixedBytes::from_array([0x70; 32])),
                iat,
                exp: iat.plus(TimeSpan::<Sec>::new(900)),
            })
            .expect("mint");
        assert!(b.verify(&token).is_err());
    }
}
