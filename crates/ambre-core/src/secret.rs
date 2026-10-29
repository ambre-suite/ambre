//! The one wrapper under every secret of the workspace, and the secret
//! newtypes that cross a thread.
//!
//! [`Secret`] carries the hygiene, and a domain newtype keeps the name and
//! the meaning: `pub struct DocumentKey(Arc<Secret<[u8; 32]>>)`. The wrapper
//! holds its value on the heap, so a move copies no byte of it. It wipes the
//! value when it drops, the heap buffer of a `String` or a `Vec` too. Its
//! `Debug` prints no byte and no length, and it has no `Display`.
//!
//! It derives no `Clone`, `Serialize`, `Deserialize`, `PartialEq`, `Hash` or
//! `Ord`. So a type that holds one cannot derive them either. Equality is
//! constant-time through [`subtle::ConstantTimeEq`]. A copy is the explicit
//! [`Secret::clone_secret`]. A type shared by several owners holds an
//! `Arc<Secret<T>>` and derives `Clone` on the `Arc`.
//!
//! [`Secret::expose`] is the one way to the value. The primitive gate refuses
//! a call of it, or of [`Secret::clone_secret`], outside `ambre-crypto` and
//! this module (`ARCH-045`). It refuses `zeroize` named anywhere outside this
//! module too (`ARCH-046`).
//!
//! The newtypes below live here, not in `ambre-crypto`. The bus, the
//! interface, the wire and the relay carry them, and none of these may depend
//! on `ambre-crypto`. Their serde form is written here by hand, for the same
//! reason. Each serde form is a door to the value, and `ARCH-047` names it.
//!
//! Three of them carry what a person types or reads. Five carry a bearer
//! credential: a session token, a presigned link, an OIDC code and state.
//! One macro writes the five, so they share one form.
//!
//! A type that holds a [`Secret`] derives `Debug`, because the wrapper's own
//! form is redacted. It cannot derive `Clone`, `Serialize` or `PartialEq`:
//!
//! ```compile_fail
//! #[derive(Clone)]
//! struct Holder(ambre_core::Secret<[u8; 32]>);
//! ```
//!
//! ```compile_fail
//! #[derive(serde::Serialize)]
//! struct Holder(ambre_core::Secret<[u8; 32]>);
//! ```
//!
//! ```compile_fail
//! #[derive(PartialEq)]
//! struct Holder(ambre_core::Secret<[u8; 32]>);
//! ```
//!
//! The same derives compile on a holder of the bare array, so each refusal
//! above comes from the wrapper and from nothing else:
//!
//! ```
//! #[derive(Clone, Debug, PartialEq, serde::Serialize)]
//! struct Plain([u8; 32]);
//!
//! #[derive(Debug)]
//! struct Holder(ambre_core::Secret<[u8; 32]>);
//! ```

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use subtle::{Choice, ConstantTimeEq};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// A secret value: on the heap, wiped on drop, redacted in `Debug`.
///
/// The value is read through [`Secret::expose`] alone, which the primitive
/// gate admits in `ambre-crypto` and in this module only (`ARCH-045`).
pub struct Secret<T: Zeroize>(Box<T>);

impl<T: Zeroize> Secret<T> {
    /// Moves `value` onto the heap, under the wrapper.
    ///
    /// The place `value` came from is the caller's, and this call does not
    /// wipe it. A value built in place goes through [`Secret::filled`].
    #[must_use]
    pub fn new(value: T) -> Self {
        Self(Box::new(value))
    }

    /// The value, for the one call that reads it.
    #[must_use]
    pub fn expose(&self) -> &T {
        &self.0
    }

    /// Wipes `value` now: a plaintext that no wrapper holds and that its
    /// owner drops, such as the bytes a refused parse hands back. Code
    /// outside this module wipes a value through this call alone
    /// (`ARCH-046`). A copy `value` left elsewhere is out of its reach.
    pub fn wipe(mut value: T) {
        value.zeroize();
    }
}

impl<T: Zeroize + Default> Secret<T> {
    /// A default value on the heap, then written in place by `fill`. No copy
    /// of the result stays on the stack.
    #[must_use]
    pub fn filled(fill: impl FnOnce(&mut T)) -> Self {
        let mut secret = Self::default();
        fill(&mut secret.0);
        secret
    }

    /// As [`Secret::filled`], for a `fill` that may fail. On a failure, the
    /// partial value is wiped as the wrapper drops.
    ///
    /// # Errors
    ///
    /// The error of `fill`, unchanged.
    pub fn try_filled<E>(fill: impl FnOnce(&mut T) -> Result<(), E>) -> Result<Self, E> {
        let mut secret = Self::default();
        fill(&mut secret.0)?;
        Ok(secret)
    }
}

impl<T: Zeroize + Clone> Secret<T> {
    /// An independent copy of the value, on a heap of its own. It is the one
    /// way to copy a secret, and the gate admits it where it admits
    /// [`Secret::expose`] (`ARCH-045`). A type with several owners shares an
    /// `Arc<Secret<T>>` instead. The copy passes through one stack temporary
    /// before it is boxed, and nothing wipes that temporary.
    #[must_use]
    pub fn clone_secret(&self) -> Self {
        Self::new(T::clone(&self.0))
    }
}

impl<T: Zeroize + Default> Default for Secret<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T: Zeroize> Drop for Secret<T> {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl<T: Zeroize> ZeroizeOnDrop for Secret<T> {}

impl<T: Zeroize> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([REDACTED])")
    }
}

/// Equality in a time that depends on the length alone, never on where the
/// first difference sits.
impl<T: Zeroize + AsRef<[u8]>> ConstantTimeEq for Secret<T> {
    fn ct_eq(&self, other: &Self) -> Choice {
        self.0.as_ref().as_ref().ct_eq(other.0.as_ref().as_ref())
    }
}

/// Secret UTF-8 text that crosses a thread. It carries the recovery phrase
/// of a new account from the Sync worker to the screen that shows it once.
#[derive(Clone, Debug, Default)]
pub struct SecretString(Arc<Secret<String>>);

impl SecretString {
    /// Wraps an owned string.
    #[must_use]
    pub fn new(text: String) -> Self {
        Self(Arc::new(Secret::new(text)))
    }

    /// The wrapper, for the crypto code that reads the text.
    #[must_use]
    pub fn secret(&self) -> &Secret<String> {
        &self.0
    }
}

impl From<String> for SecretString {
    fn from(text: String) -> Self {
        Self::new(text)
    }
}

impl Serialize for SecretString {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.expose().as_str().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SecretString {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::new(String::deserialize(deserializer)?))
    }
}

/// Secret bytes that cross the interface, the Logic worker and the Sync
/// worker: a plaintext on its way to or from a seal.
///
/// A `Vec<u8>` would relay the same bytes, print them in a `Debug` line, and
/// leave its copy on the heap after the hop. The bytes are wiped when the last
/// copy drops, in a handler that only binds and drops it too.
#[derive(Clone, Debug, Default)]
pub struct SecretBytes(Arc<Secret<Vec<u8>>>);

impl SecretBytes {
    /// Wraps owned bytes.
    #[must_use]
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(Arc::new(Secret::new(bytes)))
    }

    /// The wrapper, for the crypto code that seals the bytes.
    #[must_use]
    pub fn secret(&self) -> &Secret<Vec<u8>> {
        &self.0
    }

    /// Lends the opened bytes to `read`, with no copy, for the code that
    /// reads them on the way to another form. The bytes stay under the
    /// wrapper, which wipes them when the last copy drops, on every path out
    /// of `read`. What `read` builds from them is its own.
    pub fn with_plaintext<R>(&self, read: impl FnOnce(&[u8]) -> R) -> R {
        read(self.0.expose())
    }

    /// Moves the opened bytes out, with no copy, into a role that keeps them
    /// in the clear by design. A media file and a CRDT state are such roles.
    /// The receiver owns the bytes from here: it keeps them clear by
    /// its role, or it wipes them itself. The wrapper is left empty, so its
    /// own wipe finds nothing. A wrapper that another owner still shares
    /// hands out a copy instead, and that owner keeps and wipes the original.
    #[must_use]
    pub fn into_plaintext(self) -> Vec<u8> {
        match Arc::try_unwrap(self.0) {
            Ok(mut alone) => std::mem::take(&mut *alone.0),
            Err(shared) => shared.expose().clone(),
        }
    }
}

impl From<Vec<u8>> for SecretBytes {
    fn from(bytes: Vec<u8>) -> Self {
        Self::new(bytes)
    }
}

impl Serialize for SecretBytes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.expose().as_slice().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SecretBytes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::new(Vec::<u8>::deserialize(deserializer)?))
    }
}

/// The text a person types to derive a key: the account passphrase, or the
/// passphrase of one sealed `.ambre` file.
///
/// It travels from the prompt that reads it to the worker that derives a key
/// from it, and no further. Along that path it is a type of its own, not a
/// `String` or a `&[u8]` beside a salt of the same shape.
#[derive(Clone, Debug, Default)]
pub struct Passphrase(Arc<Secret<String>>);

impl Passphrase {
    /// Wraps the typed text.
    #[must_use]
    pub fn new(text: String) -> Self {
        Self(Arc::new(Secret::new(text)))
    }

    /// The wrapper, for the one derivation that reads the text.
    #[must_use]
    pub fn secret(&self) -> &Secret<String> {
        &self.0
    }

    /// Whether nothing was typed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.expose().is_empty()
    }

    /// Whether `other` holds the same text, in constant time: the check a
    /// sealing prompt makes against its confirmation field.
    #[must_use]
    pub fn matches(&self, other: &Self) -> bool {
        self.0.ct_eq(&other.0).into()
    }
}

impl From<String> for Passphrase {
    fn from(text: String) -> Self {
        Self::new(text)
    }
}

impl Serialize for Passphrase {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.expose().as_str().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Passphrase {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::new(String::deserialize(deserializer)?))
    }
}

/// Writes one bearer credential: a text that grants access to whoever holds
/// it. The text sits in one shared [`Secret`], so a clone copies the pointer,
/// never the text. Equality is constant-time, through `matches`. The serde
/// form is the bare text, as the wire carried it before the wrapper.
macro_rules! define_credential {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Debug)]
        pub struct $name(Arc<Secret<String>>);

        impl $name {
            /// Wraps the credential text.
            #[must_use]
            pub fn new(text: String) -> Self {
                Self(Arc::new(Secret::new(text)))
            }

            /// The wrapper, for the code the gate admits to read the text.
            #[must_use]
            pub fn secret(&self) -> &Secret<String> {
                &self.0
            }

            /// Whether `other` holds the same text, in constant time.
            #[must_use]
            pub fn matches(&self, other: &Self) -> bool {
                self.0.ct_eq(&other.0).into()
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                self.0.expose().as_str().serialize(serializer)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                Ok(Self::new(String::deserialize(deserializer)?))
            }
        }
    };
}

define_credential! {
    /// An access token: the short-lived credential the relay mints, which a
    /// request header and the first frame of the socket present (`SRV-031`).
    AccessToken
}

impl AccessToken {
    /// Takes a wrapper another owner shares, such as the token store of the
    /// Sync worker. No copy of the text is made.
    #[must_use]
    pub fn from_shared(secret: Arc<Secret<String>>) -> Self {
        Self(secret)
    }
}

define_credential! {
    /// A refresh token: the opaque credential `/auth/refresh` trades for a new
    /// pair.
    RefreshToken
}

define_credential! {
    /// A presigned object-store URL: it reads or writes one object until it
    /// expires.
    PresignedUrl
}

impl PresignedUrl {
    /// Lends the full URL to `send`, the one call that sends the request.
    /// Nothing of the text outlives the call but what `send` copies. The
    /// primitive gate admits this name in the transport alone (`ARCH-049`).
    pub fn present_link<R>(&self, send: impl FnOnce(&str) -> R) -> R {
        send(self.0.expose())
    }

    /// The address without its query, which holds the signature. A log and
    /// an error show this part alone.
    #[must_use]
    pub fn object_address(&self) -> &str {
        let text = self.0.expose();
        text.split_once('?')
            .map_or(text.as_str(), |(address, _)| address)
    }
}

define_credential! {
    /// The authorization code an identity provider hands back to the client.
    OidcCode
}

define_credential! {
    /// The CSRF state that binds a sign-in callback to the sign-in it began.
    OidcState
}

impl OidcState {
    /// Lends the state to `store`, the one call that keeps it until the
    /// callback: the page's session storage. The primitive gate admits this
    /// name in that one file (`ARCH-049`).
    pub fn present_state<R>(&self, store: impl FnOnce(&str) -> R) -> R {
        store(self.0.expose())
    }
}

#[cfg(test)]
mod tests {
    use std::marker::PhantomData;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use subtle::ConstantTimeEq;
    use zeroize::{Zeroize, ZeroizeOnDrop};

    use super::{
        AccessToken, OidcCode, OidcState, Passphrase, PresignedUrl, RefreshToken, Secret,
        SecretBytes, SecretString,
    };

    /// A value that counts the wipes it receives, so a test sees the drop
    /// wipe it without reading freed memory.
    #[derive(Default)]
    struct Spy(Arc<AtomicU32>);

    impl Zeroize for Spy {
        fn zeroize(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Reads which traits `T` implements: the inherent constant applies when
    /// the bound holds, the trait constant otherwise.
    struct Probe<T>(PhantomData<T>);

    trait Lacks {
        const DISPLAY: bool = false;
        const CLONE: bool = false;
        const SERIALIZE: bool = false;
        const PARTIAL_EQ: bool = false;
    }

    impl<T> Lacks for Probe<T> {}

    impl<T: std::fmt::Display> Probe<T> {
        const DISPLAY: bool = true;
    }

    impl<T: Clone> Probe<T> {
        const CLONE: bool = true;
    }

    impl<T: serde::Serialize> Probe<T> {
        const SERIALIZE: bool = true;
    }

    impl<T: PartialEq> Probe<T> {
        const PARTIAL_EQ: bool = true;
    }

    type Key = Secret<[u8; 32]>;

    #[test]
    fn a_secret_prints_neither_its_bytes_nor_its_length() {
        let secret = Secret::new("abandon ability".to_owned());
        assert_eq!(format!("{secret:?}"), "Secret([REDACTED])");
        let words = SecretString::new("abandon ability".to_owned());
        assert_eq!(format!("{words:?}"), "SecretString(Secret([REDACTED]))");
        let typed = Passphrase::new("correct horse".to_owned());
        assert_eq!(format!("{typed:?}"), "Passphrase(Secret([REDACTED]))");
        let plain = SecretBytes::new(b"plain".to_vec());
        assert_eq!(format!("{plain:?}"), "SecretBytes(Secret([REDACTED]))");
    }

    #[test]
    fn a_secret_wipes_its_value_once_when_it_drops() {
        let wipes = Arc::new(AtomicU32::new(0));
        let secret = Secret::new(Spy(Arc::clone(&wipes)));
        assert_eq!(wipes.load(Ordering::Relaxed), 0);
        drop(secret);
        assert_eq!(wipes.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_shared_secret_wipes_when_its_last_owner_drops() {
        let wipes = Arc::new(AtomicU32::new(0));
        let first = Arc::new(Secret::new(Spy(Arc::clone(&wipes))));
        let second = Arc::clone(&first);
        drop(first);
        assert_eq!(wipes.load(Ordering::Relaxed), 0);
        drop(second);
        assert_eq!(wipes.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_failed_fill_wipes_the_partial_value() {
        let wipes = Arc::new(AtomicU32::new(0));
        let seen = Arc::clone(&wipes);
        let failed = Secret::<Spy>::try_filled(|spy| {
            spy.0 = seen;
            Err("refused")
        });
        assert!(failed.is_err());
        assert_eq!(wipes.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_secret_is_zeroize_on_drop() {
        fn wiped_on_drop<T: ZeroizeOnDrop>(_: &T) {}
        wiped_on_drop(&Key::default());
        wiped_on_drop(&Secret::new(String::new()));
    }

    #[test]
    fn equality_is_constant_time_and_reads_every_byte() {
        let a = Key::new([7; 32]);
        let mut last = [7; 32];
        last[31] = 8;
        assert!(bool::from(a.ct_eq(&Key::new([7; 32]))));
        assert!(!bool::from(a.ct_eq(&Key::new(last))));
        let typed = Passphrase::new("correct horse".to_owned());
        assert!(typed.matches(&Passphrase::new("correct horse".to_owned())));
        assert!(!typed.matches(&Passphrase::new("correct horsf".to_owned())));
        assert!(!typed.matches(&Passphrase::new("correct".to_owned())));
    }

    #[test]
    fn expose_reads_the_value_and_clone_secret_copies_it() {
        let secret = Secret::filled(|key: &mut [u8; 32]| key[0] = 9);
        assert_eq!(secret.expose()[0], 9);
        let copy = secret.clone_secret();
        drop(secret);
        assert_eq!(copy.expose()[0], 9);
    }

    #[test]
    fn a_secret_has_no_display_clone_serialize_or_partial_eq() {
        // The probe answers true for a type that has each trait.
        assert_eq!(
            [
                Probe::<String>::DISPLAY,
                Probe::<String>::CLONE,
                Probe::<String>::SERIALIZE,
                Probe::<String>::PARTIAL_EQ,
            ],
            [true; 4]
        );
        assert_eq!(
            [
                Probe::<Secret<String>>::DISPLAY,
                Probe::<Secret<String>>::CLONE,
                Probe::<Secret<String>>::SERIALIZE,
                Probe::<Secret<String>>::PARTIAL_EQ,
            ],
            [false; 4]
        );
        assert_eq!(
            [
                Probe::<Passphrase>::DISPLAY,
                Probe::<Passphrase>::PARTIAL_EQ
            ],
            [false; 2]
        );
    }

    #[test]
    fn a_secret_newtype_travels_as_its_text_or_its_bytes() {
        let words = SecretString::new("words".to_owned());
        let bytes = postcard::to_allocvec(&words).unwrap();
        assert_eq!(bytes, postcard::to_allocvec("words").unwrap());
        let back: SecretString = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(back.secret().expose(), "words");

        let typed = Passphrase::new("correct horse".to_owned());
        let wire = serde_json::to_string(&typed).unwrap();
        assert_eq!(wire, "\"correct horse\"");
        let back: Passphrase = serde_json::from_str(&wire).unwrap();
        assert!(back.matches(&typed));

        let plain = SecretBytes::new(b"plain".to_vec());
        let bytes = postcard::to_allocvec(&plain).unwrap();
        assert_eq!(bytes, postcard::to_allocvec(b"plain".as_slice()).unwrap());
        let back: SecretBytes = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(back.into_plaintext(), b"plain");
    }

    #[test]
    fn opened_bytes_are_lent_with_no_copy() {
        let plain = SecretBytes::new(b"plain".to_vec());
        let held = plain.secret().expose().as_ptr();
        let (lent, read) = plain.with_plaintext(|bytes| (bytes.as_ptr(), bytes.to_vec()));
        assert_eq!(lent, held);
        assert_eq!(read, b"plain");
    }

    #[test]
    fn opened_bytes_move_out_with_no_copy() {
        let plain = SecretBytes::new(b"plain".to_vec());
        let held = plain.secret().expose().as_ptr();
        let moved = plain.into_plaintext();
        assert_eq!(moved.as_ptr(), held);
        assert_eq!(moved, b"plain");
    }

    #[test]
    fn shared_opened_bytes_move_out_as_a_copy_and_stay_with_the_other_owner() {
        let plain = SecretBytes::new(b"plain".to_vec());
        let other = plain.clone();
        let held = other.secret().expose().as_ptr();
        let copy = plain.into_plaintext();
        assert_ne!(copy.as_ptr(), held);
        assert_eq!(copy, b"plain");
        assert_eq!(other.secret().expose().as_ptr(), held);
        assert_eq!(other.secret().expose(), b"plain");
    }

    #[test]
    fn a_value_handed_to_wipe_is_wiped_once() {
        let wipes = Arc::new(AtomicU32::new(0));
        Secret::wipe(Spy(Arc::clone(&wipes)));
        assert_eq!(wipes.load(Ordering::Relaxed), 1);
    }

    /// The form each credential had on the wire before its wrapper: a
    /// newtype over the bare text, as serde derived it.
    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
    struct Before(String);

    #[test]
    fn a_credential_travels_as_the_text_it_had_before_its_wrapper() {
        let text = "https://store.test/o?sig=s3cr3t".to_owned();
        let before = Before(text.clone());
        let json = serde_json::to_string(&before).unwrap();
        let bytes = postcard::to_allocvec(&before).unwrap();

        macro_rules! round_trip {
            ($($name:ident),+) => {$(
                let value = $name::new(text.clone());
                assert_eq!(serde_json::to_string(&value).unwrap(), json);
                assert_eq!(postcard::to_allocvec(&value).unwrap(), bytes);
                let back: $name = serde_json::from_str(&json).unwrap();
                assert!(back.matches(&value));
                let back: $name = postcard::from_bytes(&bytes).unwrap();
                assert_eq!(back.secret().expose(), &text);
                assert_eq!(
                    serde_json::from_str::<Before>(&serde_json::to_string(&back).unwrap())
                        .unwrap(),
                    before
                );
            )+};
        }
        round_trip!(AccessToken, RefreshToken, PresignedUrl, OidcCode, OidcState);
    }

    #[test]
    fn a_credential_prints_no_byte_and_compares_in_constant_time() {
        let url = PresignedUrl::new("https://store.test/o?sig=s3cr3t".to_owned());
        assert_eq!(format!("{url:?}"), "PresignedUrl(Secret([REDACTED]))");
        assert_eq!(url.object_address(), "https://store.test/o");
        assert_eq!(
            url.present_link(str::to_owned),
            "https://store.test/o?sig=s3cr3t"
        );
        let state = OidcState::new("st4te".to_owned());
        assert!(state.matches(&OidcState::new("st4te".to_owned())));
        assert!(!state.matches(&OidcState::new("st4tf".to_owned())));
        assert_eq!(state.present_state(str::to_owned), "st4te");
        assert_eq!(
            [
                Probe::<AccessToken>::PARTIAL_EQ,
                Probe::<RefreshToken>::PARTIAL_EQ,
                Probe::<PresignedUrl>::PARTIAL_EQ,
                Probe::<OidcCode>::PARTIAL_EQ,
                Probe::<OidcState>::PARTIAL_EQ,
                Probe::<AccessToken>::DISPLAY,
            ],
            [false; 6]
        );
    }

    #[test]
    fn a_shared_access_token_is_the_same_wrapper() {
        let shared = Arc::new(Secret::new("acc".to_owned()));
        let token = AccessToken::from_shared(Arc::clone(&shared));
        assert!(std::ptr::eq(token.secret(), shared.as_ref()));
    }
}
