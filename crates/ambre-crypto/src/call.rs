//! The sender keys of a call and the seal of each chunk
//! (`ambre-spec-call-media.md section 3`, the chunk format).
//!
//! A chunk is one encoded picture or audio packet of a track. The transform seals it under the sender key
//! of its track, in the format the spec writes byte by byte:
//!
//! ```text
//! clear prefix | 0xFF | key identifier, 8 bytes | counter, 8 bytes | ciphertext | tag, 16 bytes
//! ```
//!
//! The header is the one of RFC 9605 with both lengths fixed at 8 bytes. So every chunk
//! carries a header of one size. The key schedule is the one of RFC 9605 over
//! HKDF-SHA256, for the private suite `0xF001`: ChaCha20-Poly1305 with a 16-byte tag. The
//! clear prefix and the header are the additional data. An audio chunk carries no prefix,
//! and its plaintext is padded to one size: a big-endian `u16` length, the clear chunk, then
//! zeros.
//!
//! A sender key is drawn from the generator of the OS. A join moves it one generation up by
//! HKDF-SHA256. A wrap seals it to one recipient under a key that X25519 and HKDF-SHA256
//! derive. No door builds one from a channel key or a document key (`CALL-002`).

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use ambre_core::id::{Channel, Id, Meeting, User};
use ambre_core::{
    Authored, ChunkCounter, ClearChunk, ExchangePublicKey, FixedBytes, Instant, Issuer,
    KeyGeneration, KeyIdentifier, Octets, Payload, PublicKey, SENDER_INDEX_LIMIT, SealedChunk,
    Secret, SenderIndex, Seq, Signature, Track, Transcript,
};
use chacha20poly1305::aead::AeadInPlace;
use chacha20poly1305::{ChaCha20Poly1305, KeyInit};
use ed25519_dalek::SigningKey;
use hkdf::Hkdf;
use sha2::Sha256;
use thiserror::Error;
use x25519_dalek::{PublicKey as ExchangePoint, StaticSecret};

use crate::error::CryptoError;
use crate::keys::DocumentKey;
use crate::random;

/// The private suite of RFC 9605 the chunks use: ChaCha20-Poly1305, HKDF-SHA256.
const SUITE: u16 = 0xF001;
/// The config byte of the header: an 8-byte key identifier, then an 8-byte counter.
const CONFIG_BYTE: u8 = 0xFF;
/// The length of the header: the config byte, the key identifier, the counter.
const HEADER_LEN: usize = 17;
/// The length of the Poly1305 tag.
const TAG_LEN: usize = 16;
/// The length of the ChaCha20-Poly1305 nonce, and of the salt of RFC 9605.
const SALT_LEN: usize = 12;
/// The largest clear audio chunk a sealed chunk carries. Opus at a constant 64 kbit/s writes 160 bytes
/// per 20 ms.
const AUDIO_CLEAR_CAP: usize = 160;
/// The padded plaintext of an audio chunk: the length, then the clear chunk and its padding.
const AUDIO_BODY_LEN: usize = 2 + AUDIO_CLEAR_CAP;
/// The most ratchets a receiver derives to reach the generation a chunk names.
const MAX_RATCHETS: ambre_core::Count<ambre_core::Ratchets> = ambre_core::Count::new(64);
/// The most openers a receiver keeps per sender, the newest ones.
const OPENERS_PER_SENDER: ambre_core::Count<Openers> = ambre_core::Count::new(8);

/// A number of chunk openers a receiver keeps.
enum Openers {}

impl ambre_core::CountKind for Openers {
    type Repr = u64;
    const NAME: &'static str = "Openers";
}

/// The size of every sealed audio chunk, in every call (`CALL-007`).
pub const AUDIO_CHUNK_SIZE: ambre_core::Count<Octets> =
    ambre_core::Count::new((HEADER_LEN + AUDIO_BODY_LEN + TAG_LEN) as u64);

/// The label of the ratchet of a sender key.
const RATCHET_LABEL: &[u8] = b"ambre-call-ratchet-v1";
/// The domain of the signed join of a call.
const JOIN_DOMAIN: &[u8] = b"ambre-call-join-v1";
/// The domain of the key that wraps a sender key, and the additional data of the wrap.
const WRAP_DOMAIN: &[u8] = b"ambre-call-wrap-v1";

/// Why a chunk was not sealed or not opened. The receiver drops a chunk on any of them,
/// and tells no sender which.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ChunkError {
    /// The bytes are no chunk of this format: too short, or another config byte.
    #[error("malformed chunk")]
    Malformed,
    /// No key the receiver holds, or derives, matches the key identifier.
    #[error("no key for the chunk")]
    UnknownKey,
    /// The counter falls outside the replay window of its key, or the key saw it already.
    #[error("replayed chunk")]
    Replayed,
    /// The tag does not verify: another key, another header or changed bytes.
    #[error("chunk does not open")]
    Unopened,
    /// A clear audio chunk larger than the fixed size.
    #[error("chunk too large")]
    ChunkTooLarge,
    /// A track this version seals nothing of: video waits for the clear prefix rule.
    #[error("track not carried")]
    TrackNotCarried,
    /// The key identifier names a field past the width of the header.
    #[error("key identifier out of range")]
    KeyIdentifierOutOfRange,
}

/// The sender key of one participant session, at one generation.
///
/// The bytes are drawn from the generator of the OS, ratcheted or unwrapped. No other door
/// builds one, so no sender key derives from a channel key or a document key (`CALL-002`).
/// The Sync worker and the call worker share one key, so it holds one [`Secret`] behind an
/// [`Arc`], wiped when the last owner drops it.
#[derive(Clone, Debug)]
pub struct SenderKey(Arc<Secret<[u8; 32]>>);

impl SenderKey {
    /// A fresh key from the generator of the OS.
    #[must_use]
    pub fn generate() -> Self {
        Self(Arc::new(Secret::filled(|key: &mut [u8; 32]| {
            random::fill(key);
        })))
    }

    /// The key one generation up: HKDF-SHA256 over this key. A holder of the new key
    /// derives nothing of this one.
    #[must_use]
    pub fn ratcheted(&self) -> Self {
        Self(Arc::new(expand_32(self.0.expose(), RATCHET_LABEL)))
    }

    /// The raw key, for this module alone.
    fn bytes(&self) -> &[u8; 32] {
        self.0.expose()
    }
}

/// HKDF-SHA256 with an empty salt over `ikm`, expanded to 32 bytes under `info`.
#[expect(
    clippy::expect_used,
    reason = "HKDF-SHA256 expands up to 8160 bytes, and 32 is far below that"
)]
fn expand_32(ikm: &[u8], info: &[u8]) -> Secret<[u8; 32]> {
    let hk = Hkdf::<Sha256>::new(None, ikm);
    Secret::filled(|out: &mut [u8; 32]| {
        hk.expand(info, out)
            .expect("HKDF-SHA256 expand of 32 bytes is within the output limit");
    })
}

/// The 64-bit spelling of a key identifier in a header, or `None` for a field past its
/// width.
fn kid_number(kid: KeyIdentifier) -> Option<u64> {
    (kid.sender < SENDER_INDEX_LIMIT && kid.generation.fits_a_key_identifier()).then(|| {
        (kid.generation.get() << 16) | (kid.sender.get() << 2) | u64::from(kid.track.bits())
    })
}

/// The key identifier a header spells, or `None` for the track value no track holds.
fn kid_of(number: u64) -> Option<KeyIdentifier> {
    let track = Track::from_bits(u8::try_from(number & 0b11).ok()?)?;
    Some(KeyIdentifier {
        track,
        sender: Seq::new((number >> 2) & 0x3FFF),
        generation: Seq::new(number >> 16),
    })
}

/// The key and the salt of one key identifier, by the key schedule of RFC 9605, wiped
/// when dropped.
struct ChunkKey(Secret<([u8; 32], [u8; SALT_LEN])>);

impl ChunkKey {
    /// Derives the key and the salt of `kid` from the sender key, or `None` for a key
    /// identifier past the width of the header.
    #[expect(
        clippy::expect_used,
        reason = "HKDF-SHA256 expands up to 8160 bytes, and 32 is far below that"
    )]
    fn derive(sender_key: &SenderKey, kid: KeyIdentifier) -> Option<Self> {
        let number = kid_number(kid)?;
        // The pseudorandom key stays in the HKDF state, which the primitive does not wipe,
        // as `ambre-spec-zero-knowledge.md section 2` states of every derivation.
        let (_, hk) = Hkdf::<Sha256>::extract(None, sender_key.bytes());
        let label = |name: &[u8]| {
            let mut info = Vec::with_capacity(name.len() + 10);
            info.extend_from_slice(name);
            info.extend_from_slice(&number.to_be_bytes());
            info.extend_from_slice(&SUITE.to_be_bytes());
            info
        };
        Some(Self(Secret::filled(
            |(key, salt): &mut ([u8; 32], [u8; SALT_LEN])| {
                hk.expand(&label(b"SFrame 1.0 Secret key "), key)
                    .expect("HKDF-SHA256 expand of 32 bytes is within the output limit");
                hk.expand(&label(b"SFrame 1.0 Secret salt "), salt)
                    .expect("HKDF-SHA256 expand of 12 bytes is within the output limit");
            },
        )))
    }

    /// The nonce of `counter`: the salt XOR the counter, left-padded to 12 bytes.
    fn nonce(&self, counter: Seq<ChunkCounter>) -> chacha20poly1305::Nonce {
        let mut nonce = self.0.expose().1;
        for (byte, count) in nonce
            .iter_mut()
            .skip(SALT_LEN - 8)
            .zip(counter.get().to_be_bytes())
        {
            *byte ^= count;
        }
        nonce.into()
    }

    fn cipher(&self) -> ChaCha20Poly1305 {
        ChaCha20Poly1305::new((&self.0.expose().0).into())
    }
}

/// The header of a chunk: what the SFU and the receiver read before any key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChunkHeader {
    /// Which key sealed the chunk.
    pub kid: KeyIdentifier,
    /// The counter of the chunk under that key.
    pub counter: Seq<ChunkCounter>,
}

impl ChunkHeader {
    /// Reads the header of an audio chunk, which carries no clear prefix.
    ///
    /// # Errors
    ///
    /// [`ChunkError::Malformed`] for bytes too short to hold a header and a tag, another
    /// config byte, or a key identifier with no track.
    pub fn read_audio(chunk: &Payload<SealedChunk>) -> Result<Self, ChunkError> {
        let bytes = chunk.as_bytes();
        let (&config, rest) = bytes.split_first().ok_or(ChunkError::Malformed)?;
        let (kid, rest) = rest.split_first_chunk::<8>().ok_or(ChunkError::Malformed)?;
        let (counter, rest) = rest.split_first_chunk::<8>().ok_or(ChunkError::Malformed)?;
        if config != CONFIG_BYTE || rest.len() < TAG_LEN {
            return Err(ChunkError::Malformed);
        }
        Ok(Self {
            kid: kid_of(u64::from_be_bytes(*kid)).ok_or(ChunkError::Malformed)?,
            counter: Seq::new(u64::from_be_bytes(*counter)),
        })
    }
}

/// The one door from a clear chunk to a sealed chunk, for one key identifier.
///
/// It holds the counter of its key and issues each one once, so one key never seals two
/// chunks under one counter (`CALL-005`). No caller hands it a counter.
pub struct ChunkSealer {
    kid: KeyIdentifier,
    key: ChunkKey,
    counters: Issuer<ChunkCounter>,
}

impl ChunkSealer {
    /// A sealer of the chunks of `kid` under `sender_key`, at the generation `kid` names.
    ///
    /// # Errors
    ///
    /// [`ChunkError::KeyIdentifierOutOfRange`] for a sender index or a generation past the
    /// width of the header, and [`ChunkError::TrackNotCarried`] for a video track.
    pub fn new(sender_key: &SenderKey, kid: KeyIdentifier) -> Result<Self, ChunkError> {
        if kid.track != Track::Audio {
            return Err(ChunkError::TrackNotCarried);
        }
        Ok(Self {
            kid,
            key: ChunkKey::derive(sender_key, kid).ok_or(ChunkError::KeyIdentifierOutOfRange)?,
            counters: Issuer::default(),
        })
    }

    /// The key identifier this sealer writes.
    #[must_use]
    pub const fn kid(&self) -> KeyIdentifier {
        self.kid
    }

    /// Seals one clear audio chunk, padded to [`AUDIO_CHUNK_SIZE`] (`CALL-007`), under the next
    /// counter.
    ///
    /// # Errors
    ///
    /// [`ChunkError::ChunkTooLarge`] for a clear chunk past the fixed size, which is not
    /// sent. [`ChunkError::Unopened`] when the cipher refuses, which a chunk this size
    /// never reaches.
    pub fn seal(
        &mut self,
        clear: &Payload<ClearChunk>,
    ) -> Result<Payload<SealedChunk>, ChunkError> {
        let clear = clear.as_bytes();
        let length = u16::try_from(clear.len())
            .ok()
            .filter(|_| clear.len() <= AUDIO_CLEAR_CAP)
            .ok_or(ChunkError::ChunkTooLarge)?;
        let counter = self.counters.issue();
        let mut chunk = Vec::with_capacity(HEADER_LEN + AUDIO_BODY_LEN + TAG_LEN);
        chunk.push(CONFIG_BYTE);
        chunk.extend_from_slice(&kid_number(self.kid).unwrap_or_default().to_be_bytes());
        chunk.extend_from_slice(&counter.get().to_be_bytes());
        chunk.extend_from_slice(&length.to_be_bytes());
        chunk.extend_from_slice(clear);
        chunk.resize(HEADER_LEN + AUDIO_BODY_LEN, 0);
        let (aad, body) = chunk.split_at_mut(HEADER_LEN);
        let tag = self
            .key
            .cipher()
            .encrypt_in_place_detached(&self.key.nonce(counter), aad, body)
            .map_err(|_| ChunkError::Unopened)?;
        chunk.extend_from_slice(&tag);
        Ok(Payload::new(chunk))
    }
}

/// The counters one key has opened: the highest, and a bit per counter below it, as far
/// as the 128 bits of the map reach.
#[derive(Default)]
struct ReplayWindow {
    highest: Seq<ChunkCounter>,
    seen: u128,
}

impl ReplayWindow {
    /// Whether `counter` may still open: above the highest, or inside the window and
    /// unseen (`CALL-026`).
    fn admits(&self, counter: Seq<ChunkCounter>) -> bool {
        if counter > self.highest {
            return true;
        }
        let behind = self.highest.get() - counter.get();
        behind < u64::from(u128::BITS) && self.seen & (1u128 << behind) == 0
    }

    /// Records `counter` once it opened. Never before: a forged chunk moves no window.
    fn mark(&mut self, counter: Seq<ChunkCounter>) {
        if counter > self.highest {
            let shift = counter.get() - self.highest.get();
            self.seen = if shift >= u64::from(u128::BITS) {
                0
            } else {
                self.seen << shift
            };
            self.highest = counter;
        }
        self.seen |= 1u128 << (self.highest.get() - counter.get());
    }
}

/// The one door from a sealed chunk to a clear chunk, for one key identifier.
struct ChunkOpener {
    key: ChunkKey,
    window: ReplayWindow,
}

impl ChunkOpener {
    /// Opens an audio chunk this opener's key identifier sealed. The clear chunk comes back only
    /// once the tag verifies (`CALL-022`).
    fn open(
        &mut self,
        header: ChunkHeader,
        chunk: Payload<SealedChunk>,
    ) -> Result<Payload<ClearChunk>, ChunkError> {
        if !self.window.admits(header.counter) {
            return Err(ChunkError::Replayed);
        }
        let mut bytes = chunk.into_bytes();
        let tag_at = bytes
            .len()
            .checked_sub(TAG_LEN)
            .filter(|&at| at >= HEADER_LEN)
            .ok_or(ChunkError::Malformed)?;
        let tag = bytes.split_off(tag_at);
        let (aad, body) = bytes.split_at_mut(HEADER_LEN);
        self.key
            .cipher()
            .decrypt_in_place_detached(
                &self.key.nonce(header.counter),
                aad,
                body,
                chacha20poly1305::Tag::from_slice(&tag),
            )
            .map_err(|_| ChunkError::Unopened)?;
        self.window.mark(header.counter);
        let (length, clear) = body.split_first_chunk::<2>().ok_or(ChunkError::Malformed)?;
        let length = usize::from(u16::from_be_bytes(*length));
        let clear = clear.get(..length).ok_or(ChunkError::Malformed)?.to_vec();
        Ok(Payload::new(clear))
    }
}

/// The sender keys of the other participants of one call, as the receiver holds them.
///
/// It opens a chunk under the key its identifier names alone. So a chunk reaches the
/// decoder with the sender that sealed it, or not at all (`CALL-022`, `CALL-025`).
#[derive(Default)]
pub struct ReceiveKeys {
    senders: HashMap<Seq<SenderIndex>, Received>,
}

/// What a receiver holds of one sender.
#[derive(Default)]
struct Received {
    /// The keys wrapped to the receiver, by generation.
    keys: BTreeMap<Seq<KeyGeneration>, SenderKey>,
    /// The openers of the key identifiers that opened a chunk, the newest ones.
    openers: BTreeMap<(Seq<KeyGeneration>, Track), ChunkOpener>,
    /// No chunk below this generation opens: its opener was dropped with its window.
    floor: Seq<KeyGeneration>,
}

impl ReceiveKeys {
    /// Holds `key` as the key of `sender` at `generation`.
    pub fn insert(
        &mut self,
        sender: Seq<SenderIndex>,
        generation: Seq<KeyGeneration>,
        key: SenderKey,
    ) {
        self.senders
            .entry(sender)
            .or_default()
            .keys
            .insert(generation, key);
    }

    /// Drops every key of `sender`.
    pub fn forget(&mut self, sender: Seq<SenderIndex>) {
        self.senders.remove(&sender);
    }

    /// Opens an audio chunk under the key its identifier names, and names its sender.
    ///
    /// A key it does not hold comes from ratchets of the closest key below it in the same
    /// epoch. The receiver keeps that key only once a chunk opens under it.
    ///
    /// # Errors
    ///
    /// Each [`ChunkError`] of a receiver: the chunk is dropped (`CALL-024`, `CALL-025`,
    /// `CALL-026`).
    pub fn open_audio(
        &mut self,
        chunk: Payload<SealedChunk>,
    ) -> Result<(Seq<SenderIndex>, Payload<ClearChunk>), ChunkError> {
        let header = ChunkHeader::read_audio(&chunk)?;
        let kid = header.kid;
        if kid.track != Track::Audio {
            return Err(ChunkError::TrackNotCarried);
        }
        let received = self
            .senders
            .get_mut(&kid.sender)
            .ok_or(ChunkError::UnknownKey)?;
        if kid.generation < received.floor {
            return Err(ChunkError::Replayed);
        }
        let slot = (kid.generation, kid.track);
        if let Some(opener) = received.openers.get_mut(&slot) {
            return opener.open(header, chunk).map(|clear| (kid.sender, clear));
        }
        let key = received
            .key_at(kid.generation)
            .ok_or(ChunkError::UnknownKey)?;
        let mut opener = ChunkOpener {
            key: ChunkKey::derive(&key, kid).ok_or(ChunkError::Malformed)?,
            window: ReplayWindow::default(),
        };
        let clear = opener.open(header, chunk)?;
        received.keys.insert(kid.generation, key);
        received.openers.insert(slot, opener);
        if ambre_core::Count::<Openers>::of(received.openers.len()) > OPENERS_PER_SENDER
            && let Some(((oldest, _), _)) = received.openers.pop_first()
        {
            received.floor = received.floor.max(oldest);
        }
        Ok((kid.sender, clear))
    }
}

impl Received {
    /// The key at `generation`: held, or ratcheted up from the closest held key below it
    /// in the same epoch, at most [`MAX_RATCHETS`] times.
    fn key_at(&self, generation: Seq<KeyGeneration>) -> Option<SenderKey> {
        let (&base, key) = self.keys.range(..=generation).next_back()?;
        let steps = generation.ratchets_since(base)?.get();
        (steps <= MAX_RATCHETS.get())
            .then(|| (0..steps).fold(key.clone(), |key, _| key.ratcheted()))
    }
}

/// The X25519 key a participant draws for one call, to receive the wraps of the others.
pub struct CallExchangeKey(StaticSecret);

impl std::fmt::Debug for CallExchangeKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CallExchangeKey([REDACTED])")
    }
}

impl CallExchangeKey {
    /// A fresh key from the generator of the OS.
    #[must_use]
    pub fn generate() -> Self {
        let secret = Secret::filled(|bytes: &mut [u8; 32]| random::fill(bytes));
        Self(StaticSecret::from(*secret.expose()))
    }

    /// The public half, which the signed join carries.
    #[must_use]
    pub fn public(&self) -> FixedBytes<ExchangePublicKey> {
        FixedBytes::from_array(ExchangePoint::from(&self.0).to_bytes())
    }
}

/// What a participant claims when it joins a call, before its signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinClaim {
    /// The call joined.
    pub call: Id<Meeting>,
    /// The channel the call belongs to.
    pub channel: Id<Channel>,
    /// The member who joins.
    pub member: Id<User>,
    /// The Ed25519 identity of the member, which signs the claim.
    pub identity: FixedBytes<PublicKey>,
    /// The time of the join, on the clock of the joiner.
    pub at: Instant<Authored>,
    /// The index of the participant session.
    pub sender: Seq<SenderIndex>,
    /// The X25519 key of this join, drawn for this call alone.
    pub exchange: FixedBytes<ExchangePublicKey>,
}

/// The bytes a join signature covers: the domain, then each field of the claim.
struct JoinTranscript(Vec<u8>);

impl Transcript for JoinTranscript {
    fn covered_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl JoinClaim {
    fn transcript(&self) -> JoinTranscript {
        let mut bytes = Vec::with_capacity(JOIN_DOMAIN.len() + 136);
        bytes.extend_from_slice(JOIN_DOMAIN);
        bytes.extend_from_slice(self.call.as_bytes());
        bytes.extend_from_slice(self.channel.as_bytes());
        bytes.extend_from_slice(self.member.as_bytes());
        bytes.extend_from_slice(self.identity.as_slice());
        bytes.extend_from_slice(&self.at.get().to_be_bytes());
        bytes.extend_from_slice(&self.sender.get().to_be_bytes());
        bytes.extend_from_slice(self.exchange.as_slice());
        JoinTranscript(bytes)
    }
}

/// A join claim and the signature of its member's identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedJoin {
    claim: JoinClaim,
    signature: FixedBytes<Signature>,
}

impl SignedJoin {
    /// Signs `claim` with `identity`, whose public half the claim names.
    ///
    /// # Errors
    ///
    /// [`CryptoError::InvalidSignature`] when `identity` is not the key the claim names.
    pub fn sign(identity: &SigningKey, claim: JoinClaim) -> Result<Self, CryptoError> {
        if identity.verifying_key().to_bytes() != *claim.identity.as_array() {
            return Err(CryptoError::InvalidSignature);
        }
        let signature = crate::signature::sign_message(identity, &claim.transcript());
        Ok(Self { claim, signature })
    }

    /// The claim, once the identity it names verifies the signature.
    ///
    /// # Errors
    ///
    /// [`CryptoError::InvalidSignature`] when the signature does not verify.
    pub fn verified(&self) -> Result<&JoinClaim, CryptoError> {
        crate::signature::verify_message(
            &self.claim.identity,
            &self.claim.transcript(),
            &self.signature,
        )?;
        Ok(&self.claim)
    }

    /// A copy whose claim another participant rewrote, for a test of the refusal.
    #[cfg(test)]
    pub(crate) fn with_claim(&self, claim: JoinClaim) -> Self {
        Self {
            claim,
            signature: self.signature,
        }
    }
}

/// Who a wrap goes from and to, inside one call. The wrap binds all three.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WrapBinding {
    /// The call.
    pub call: Id<Meeting>,
    /// The participant whose sender key the wrap carries.
    pub sender: Seq<SenderIndex>,
    /// The participant the wrap is for.
    pub recipient: Seq<SenderIndex>,
}

/// A sender key wrapped to one recipient: a nonce, then the sealed generation and key.
/// The relay forwards it and learns its size alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WrappedSenderKey {
    nonce: FixedBytes<ambre_core::Nonce>,
    ciphertext: Payload<ambre_core::Ciphertext>,
}

impl WrappedSenderKey {
    /// The bytes the relay forwards: the nonce, then the ciphertext.
    #[must_use]
    pub fn size(&self) -> ambre_core::Count<Octets> {
        ambre_core::Count::new(
            (self.nonce.as_slice().len() as u64).saturating_add(self.ciphertext.len().get()),
        )
    }
}

/// The key that wraps a sender key from `sender` to `recipient`: X25519, then
/// HKDF-SHA256 over the binding and both public keys.
fn wrap_key(
    shared: &crate::ecdh::SharedSecret,
    binding: WrapBinding,
    sender: &FixedBytes<ExchangePublicKey>,
    recipient: &FixedBytes<ExchangePublicKey>,
) -> DocumentKey {
    let mut info = Vec::with_capacity(WRAP_DOMAIN.len() + 96);
    info.extend_from_slice(WRAP_DOMAIN);
    info.extend_from_slice(binding.call.as_bytes());
    info.extend_from_slice(&binding.sender.get().to_be_bytes());
    info.extend_from_slice(&binding.recipient.get().to_be_bytes());
    info.extend_from_slice(sender.as_slice());
    info.extend_from_slice(recipient.as_slice());
    DocumentKey::from_secret(expand_32(shared.as_bytes(), &info))
}

/// Wraps `key` at `generation` from the holder of `own` to the holder of `recipient`
/// alone (`CALL-016`).
///
/// # Errors
///
/// [`CryptoError::WeakPublicKey`] for a low-order `recipient`, and
/// [`CryptoError::EncryptionFailed`] from the AEAD.
pub fn wrap_sender_key(
    own: &CallExchangeKey,
    recipient: &FixedBytes<ExchangePublicKey>,
    binding: WrapBinding,
    generation: Seq<KeyGeneration>,
    key: &SenderKey,
) -> Result<WrappedSenderKey, CryptoError> {
    let shared = crate::ecdh::shared_secret(&own.0, recipient)?;
    let kek = wrap_key(&shared, binding, &own.public(), recipient);
    let plaintext = Secret::filled(|plain: &mut Vec<u8>| {
        plain.reserve_exact(40);
        plain.extend_from_slice(&generation.get().to_be_bytes());
        plain.extend_from_slice(key.bytes());
    });
    let (nonce, ciphertext) =
        crate::aead::seal(plaintext.expose(), &ambre_core::Aad::of(WRAP_DOMAIN), &kek)?;
    Ok(WrappedSenderKey { nonce, ciphertext })
}

/// Opens a wrap the holder of `sender` made for the holder of `own`.
///
/// # Errors
///
/// [`CryptoError::WeakPublicKey`] for a low-order `sender`, [`CryptoError::AeadFailed`]
/// for a wrap made for another binding or another recipient, and
/// [`CryptoError::InvalidKeyLength`] for a plaintext of another size.
pub fn unwrap_sender_key(
    own: &CallExchangeKey,
    sender: &FixedBytes<ExchangePublicKey>,
    binding: WrapBinding,
    wrapped: &WrappedSenderKey,
) -> Result<(Seq<KeyGeneration>, SenderKey), CryptoError> {
    let shared = crate::ecdh::shared_secret(&own.0, sender)?;
    let kek = wrap_key(&shared, binding, sender, &own.public());
    let opened = crate::aead::open(
        &wrapped.nonce,
        &wrapped.ciphertext,
        &ambre_core::Aad::of(WRAP_DOMAIN),
        &kek,
    )?;
    let plain = opened.secret().expose();
    let (generation, key) = plain
        .split_first_chunk::<8>()
        .filter(|(_, key)| key.len() == 32)
        .ok_or(CryptoError::InvalidKeyLength)?;
    let key = Secret::try_filled(|out: &mut [u8; 32]| {
        out.copy_from_slice(key);
        Ok::<(), CryptoError>(())
    })?;
    Ok((
        Seq::new(u64::from_be_bytes(*generation)),
        SenderKey(Arc::new(key)),
    ))
}

#[cfg(test)]
#[path = "call_tests.rs"]
mod tests;
