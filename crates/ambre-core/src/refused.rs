//! The error of a refused value, which echoes it.
//!
//! A door that checks a value hands the caller back what it refused, so the
//! message names it and the caller may read it again. Every such error is one
//! generic type, [`Refused<K>`], over a marker `K` that names the raw type
//! and writes the message. Two refusals of one raw type never stand in for
//! each other:
//!
//! ```compile_fail,E0308
//! use ambre_core::{Members, Refused, StatusWord};
//! let _: Refused<StatusWord> = Refused::<Members>::new(-1);
//! ```
//!
//! A number refused for leaving the bounds of a [`Bounded`](crate::Bounded)
//! quantity is an [`OutOfRange`](crate::OutOfRange) instead: its message
//! comes from the bounds. A `Refused` serves a rule no bounds express, such
//! as a finite number, a byte of a set or a word of a closed list.

use std::fmt;
use std::marker::PhantomData;

/// The marker of one refused value.
///
/// The marker is an uninhabited type: it names what was refused and holds no
/// value. The trait is open, because the check lives in the crate of the
/// value it guards.
pub trait Refuses: 'static {
    /// The raw value the door refused, as the caller handed it.
    type Raw: fmt::Debug;
    /// The name `Debug` prints, as the hand-written error printed its own.
    const NAME: crate::StaticKey<crate::DebugNames>;

    /// Writes the message a person reads about the refused `raw`.
    ///
    /// # Errors
    ///
    /// The formatter's own error.
    fn message(raw: &Self::Raw, f: &mut fmt::Formatter<'_>) -> fmt::Result;
}

/// The error of a value the door of `K` refused, echoing it.
pub struct Refused<K: Refuses> {
    raw: K::Raw,
    kind: PhantomData<fn() -> K>,
}

impl<K: Refuses> Refused<K> {
    /// The error of the refused `raw`. It checks nothing: the door that
    /// refused `raw` builds it.
    #[must_use]
    pub const fn new(raw: K::Raw) -> Self {
        Self {
            raw,
            kind: PhantomData,
        }
    }
}

impl<K: Refuses> Refused<K>
where
    K::Raw: Copy,
{
    /// The refused value, as the error echoes it.
    #[must_use]
    pub const fn get(self) -> K::Raw {
        self.raw
    }
}

impl<K: Refuses<Raw = String>> Refused<K> {
    /// The refused text, as the error echoes it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.raw
    }
}

// By hand, every impl below: a derive would bound the marker, which has no
// value to clone, compare or print.

impl<K: Refuses> Clone for Refused<K>
where
    K::Raw: Clone,
{
    fn clone(&self) -> Self {
        Self::new(self.raw.clone())
    }
}

impl<K: Refuses> Copy for Refused<K> where K::Raw: Copy {}

impl<K: Refuses> PartialEq for Refused<K>
where
    K::Raw: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl<K: Refuses> Eq for Refused<K> where K::Raw: Eq {}

/// `NAME(raw)`, as the derived `Debug` of the hand-written error printed it.
impl<K: Refuses> fmt::Debug for Refused<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple(K::NAME.as_str()).field(&self.raw).finish()
    }
}

/// The message of the marker, around the refused value.
impl<K: Refuses> fmt::Display for Refused<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        K::message(&self.raw, f)
    }
}

impl<K: Refuses> std::error::Error for Refused<K> {}

#[cfg(test)]
mod tests {
    use super::{Refused, Refuses};
    use std::fmt;

    /// A test refusal of a byte, printed in hex.
    enum Separator {}
    impl Refuses for Separator {
        type Raw = u8;
        const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("InvalidSeparator");
        fn message(raw: &Self::Raw, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "byte {raw:#04x} separates nothing")
        }
    }

    /// A test refusal of a word.
    enum Word {}
    impl Refuses for Word {
        type Raw = String;
        const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("UnknownWord");
        fn message(raw: &Self::Raw, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "unknown word: {raw}")
        }
    }

    #[test]
    fn a_refusal_echoes_its_value_in_its_message_and_its_debug_name() {
        let byte = Refused::<Separator>::new(b'"');
        assert_eq!(byte.get(), b'"');
        assert_eq!(byte.to_string(), "byte 0x22 separates nothing");
        assert_eq!(format!("{byte:?}"), "InvalidSeparator(34)");
        assert_eq!(byte, Refused::new(b'"'));
        let word = Refused::<Word>::new("maybe".to_owned());
        assert_eq!(word.as_str(), "maybe");
        assert_eq!(word.to_string(), "unknown word: maybe");
        assert_eq!(format!("{word:?}"), "UnknownWord(\"maybe\")");
        assert_eq!(word.clone(), word);
        let boxed: Box<dyn std::error::Error> = Box::new(word);
        assert_eq!(boxed.to_string(), "unknown word: maybe");
    }
}
