//! BLAKE2b hashing for the snapshot integrity chain.
//!
//! Each snapshot carries a `parent_snapshot_proof` linking it to its parent
//! without decryption. The chain hash folds the parent proof with the new
//! snapshot bytes, so any reordering or substitution breaks the chain.

use ambre_core::FixedBytes;
use blake2::{Blake2b512, Digest};
use data_encoding::BASE32_NOPAD;

/// Length of a chain hash, in bytes.
pub const HASH_LEN: usize = 32;

/// Domain-separation tag for the TOFU public-key fingerprint. Distinct from the
/// key-derivation tags in `keys.rs` so a fingerprint can never collide with a
/// derived key digest over the same input.
const FINGERPRINT_DOMAIN: &[u8] = b"ambre-fingerprint-v1";

/// Bytes of the fingerprint retained after truncation: 20 bytes = 160 bits, far
/// beyond collision reach yet short enough to read aloud over the phone.
const FINGERPRINT_LEN: usize = 20;

/// Human-verifiable fingerprint of a public key for the TOFU trust check
/// (`ambre-spec-key-verification.md`, CHROME-048).
///
/// Computes `BLAKE2b-512(domain || pub_key)`, keeps the first 20 bytes, and
/// base32-encodes them (RFC 4648, no padding). The 32 output characters are
/// grouped into 4-char blocks separated by spaces, so the string reads cleanly
/// aloud for out-of-band verification. The output is stable for a given key: the
/// same key always yields the same string.
///
pub fn fingerprint(
    pub_key: &FixedBytes<ambre_core::ExchangePublicKey>,
) -> ambre_core::Opaque<ambre_core::KeyFingerprints> {
    let mut hasher = Blake2b512::new();
    hasher.update(FINGERPRINT_DOMAIN);
    hasher.update(pub_key.as_array());
    let digest: [u8; FINGERPRINT_LEN] = prefix(&hasher.finalize());
    let encoded = BASE32_NOPAD.encode(&digest);
    // Group into 4-char blocks: "XXXX XXXX ..." (8 groups for 32 chars).
    let mut out = String::with_capacity(encoded.len() + encoded.len() / 4);
    for (i, c) in encoded.chars().enumerate() {
        if i > 0 && i % 4 == 0 {
            out.push(' ');
        }
        out.push(c);
    }
    ambre_core::Opaque::<ambre_core::KeyFingerprints>::new(out)
}

/// The BLAKE2b-256 digest of the converter module's bytes (`ARCH-080`). The
/// build computes it over the module it ships. The Sync worker computes it
/// over the bytes it fetched. Keyless and unsalted, so any copy of the same
/// bytes gives the same digest.
pub fn module_digest<K: ambre_core::PayloadKind>(
    bytes: &ambre_core::Payload<K>,
) -> FixedBytes<ambre_core::ModuleDigest> {
    let digest = blake2::Blake2b::<blake2::digest::consts::U32>::digest(bytes.as_bytes());
    FixedBytes::from_array(prefix(&digest))
}

/// The first `N` bytes of `digest`, zero-filled past its end. Every caller
/// truncates a 64-byte BLAKE2b digest, so the fill never happens; it keeps the
/// copy free of a length check that could panic.
pub(crate) fn prefix<const N: usize>(digest: &[u8]) -> [u8; N] {
    let mut out = [0u8; N];
    for (o, d) in out.iter_mut().zip(digest) {
        *o = *d;
    }
    out
}

/// Domain-separation tag for the digest of a membership log entry.
const ENTRY_DIGEST_DOMAIN: &[u8] = b"ambre-membership-digest-v1";

/// The digest of one encoded membership entry, which the next entry names
/// (`SYNC-102`): `BLAKE2b(domain || entry)`, truncated to 32 bytes.
pub fn entry_digest(
    encoded_entry: &ambre_core::Encoded<ambre_core::EncodedLogEntry>,
) -> FixedBytes<ambre_core::EntryDigest> {
    let mut hasher = Blake2b512::new();
    hasher.update(ENTRY_DIGEST_DOMAIN);
    hasher.update(encoded_entry.as_bytes());
    FixedBytes::from_array(prefix(&hasher.finalize()))
}

/// Domain-separation tag for the pin of an inviter's signing key.
const INVITER_PIN_DOMAIN: &[u8] = b"ambre-inviter-pin-v1";

/// The pin of the signing key `signing_key` that an invite link carries
/// (`SYNC-205`): `BLAKE2b(domain || key)`, truncated to 32 bytes. The tag keeps
/// it apart from the readable fingerprint of an exchange key, so a pin never
/// passes for a fingerprint or the reverse.
pub fn inviter_pin(
    signing_key: &FixedBytes<ambre_core::PublicKey>,
) -> FixedBytes<ambre_core::InviterPin> {
    let mut hasher = Blake2b512::new();
    hasher.update(INVITER_PIN_DOMAIN);
    hasher.update(signing_key.as_array());
    FixedBytes::from_array(prefix(&hasher.finalize()))
}

/// Computes the chain hash `BLAKE2b(len(parent) || parent || snapshot)`,
/// truncated to 32 bytes. The first snapshot of a document has no parent, and
/// its parent field is empty. The length prefix makes the field boundary
/// unambiguous, so a parent and a snapshot never read as another pair.
pub fn chain_hash(
    parent: Option<&FixedBytes<ambre_core::ChainHash>>,
    snapshot: &ambre_core::Payload<ambre_core::Ciphertext>,
) -> FixedBytes<ambre_core::ChainHash> {
    let parent: &[u8] = parent.map_or(&[], |p| p.as_slice());
    FixedBytes::from_array(length_prefixed(Head(parent), Tail(snapshot.as_bytes())))
}

/// The field that [`length_prefixed`] bounds with its length.
struct Head<'a>(&'a [u8]);

/// The bytes that [`length_prefixed`] hashes after the bounded field.
///
/// The chain hash rests on the order of the two inputs. Each takes a type of
/// its own, so a swap does not compile.
struct Tail<'a>(&'a [u8]);

/// `BLAKE2b(len(head) || head || tail)`, truncated to 32 bytes: the one
/// construction behind the chain hash and the two keyless digests below.
fn length_prefixed(head: Head<'_>, tail: Tail<'_>) -> [u8; HASH_LEN] {
    let mut hasher = Blake2b512::new();
    hasher.update((head.0.len() as u64).to_le_bytes());
    hasher.update(head.0);
    hasher.update(tail.0);
    prefix(&hasher.finalize())
}

/// Domain-separation tag of the dev-login seed.
const DEV_SEED_DOMAIN: &[u8] = b"ambre-dev-seed";

/// The seed a dev login derives from a username. The same username always
/// yields the same keys, so a refresh re-derives the identity with no
/// server-side key storage. Dev login only; a real account draws its seed
/// from the CSPRNG.
pub fn dev_seed(
    username: &ambre_core::Validated<ambre_core::DisplayName>,
) -> crate::keys::MasterSeed {
    crate::keys::MasterSeed::from_array(length_prefixed(
        Head(DEV_SEED_DOMAIN),
        Tail(username.as_str().as_bytes()),
    ))
}

/// A keyless 32-byte digest of a whole container file, for the fixture
/// sidecar that catches a flipped byte. Not a link of the snapshot chain: its head is
/// empty, which keeps the digests of the committed corpus.
pub fn file_digest<F: ambre_core::ContainerFile>(file: &F) -> FixedBytes<ambre_core::FileDigest> {
    FixedBytes::from_array(length_prefixed(Head(&[]), Tail(file.as_ref())))
}
