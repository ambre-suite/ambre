//! Padded base64 of RFC 4648, in its standard and its URL-safe alphabet.
//!
//! The `base64` crate cost the Sync worker about 6 KB of wasm for two calls.
//! They encode the sealed bytes of [`crate::Encoded`] and the checksum header
//! of an object upload. This codec writes the text that crate writes. It refuses
//! every text that the padded engines of that crate refuse. The padding is
//! required and canonical, and the unused bits of the last symbol are zero.
//! A property test holds both halves against the crate.

/// The 64 symbols of one alphabet, in the order of their values.
const STANDARD_SYMBOLS: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const URL_SAFE_SYMBOLS: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
/// The symbol that fills a group short of three bytes.
const PAD: u8 = b'=';

/// The alphabet of a base64 text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Alphabet {
    /// `+` and `/`: the header an object store reads.
    Standard,
    /// `-` and `_`: the JSON wire of the relay.
    UrlSafe,
}

impl Alphabet {
    const fn symbols(self) -> &'static [u8; 64] {
        match self {
            Self::Standard => STANDARD_SYMBOLS,
            Self::UrlSafe => URL_SAFE_SYMBOLS,
        }
    }

    /// The symbol of the low six bits of `sextet`.
    fn symbol(self, sextet: u32) -> char {
        // The mask keeps the index under 64, so the lookup never misses.
        let index = usize::try_from(sextet & 0x3f).unwrap_or(0);
        char::from(self.symbols().get(index).copied().unwrap_or(PAD))
    }

    /// The value of `symbol`, or `None` when the alphabet lacks it.
    fn value(self, symbol: u8) -> Option<u32> {
        let value = match symbol {
            b'A'..=b'Z' => symbol - b'A',
            b'a'..=b'z' => symbol - b'a' + 26,
            b'0'..=b'9' => symbol - b'0' + 52,
            b'+' if self == Self::Standard => 62,
            b'/' if self == Self::Standard => 63,
            b'-' if self == Self::UrlSafe => 62,
            b'_' if self == Self::UrlSafe => 63,
            _ => return None,
        };
        Some(u32::from(value))
    }
}

/// Why a text is not base64.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum Base64Error {
    /// The length is not a multiple of four symbols.
    #[error("the length is not a multiple of four")]
    Length,
    /// A byte is not a symbol of the alphabet, or a pad sits before the end.
    #[error("a byte is not a symbol of the alphabet")]
    Symbol,
    /// The last symbol carries bits that no byte holds.
    #[error("the last symbol carries trailing bits")]
    TrailingBits,
}

/// The padded text of `bytes` in `alphabet`.
pub(crate) fn encode(alphabet: Alphabet, bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len().div_ceil(3).saturating_mul(4));
    let (groups, remainder) = bytes.as_chunks::<3>();
    for &[a, b, c] in groups {
        let word = u32::from(a) << 16 | u32::from(b) << 8 | u32::from(c);
        text.extend([18, 12, 6, 0].map(|shift| alphabet.symbol(word >> shift)));
    }
    match *remainder {
        [a] => {
            let word = u32::from(a) << 16;
            text.extend([18, 12].map(|shift| alphabet.symbol(word >> shift)));
            text.extend([char::from(PAD); 2]);
        }
        [a, b] => {
            let word = u32::from(a) << 16 | u32::from(b) << 8;
            text.extend([18, 12, 6].map(|shift| alphabet.symbol(word >> shift)));
            text.push(char::from(PAD));
        }
        _ => {}
    }
    text
}

/// The bytes that `text` spells in `alphabet`.
///
/// # Errors
///
/// * [`Base64Error::Length`] when the text is not whole groups of four.
/// * [`Base64Error::Symbol`] on a byte outside the alphabet, or on a pad
///   anywhere but the end of the last group.
/// * [`Base64Error::TrailingBits`] when the last symbol is not canonical.
pub(crate) fn decode(alphabet: Alphabet, text: &str) -> Result<Vec<u8>, Base64Error> {
    let text = text.as_bytes();
    if !text.len().is_multiple_of(4) {
        return Err(Base64Error::Length);
    }
    let (groups, _) = text.as_chunks::<4>();
    let mut bytes = Vec::with_capacity(groups.len().saturating_mul(3));
    let sextet = |symbol: u8| alphabet.value(symbol).ok_or(Base64Error::Symbol);
    for (index, &[a, b, c, d]) in groups.iter().enumerate() {
        let last = index + 1 == groups.len();
        let head = sextet(a)? << 18 | sextet(b)? << 12;
        match (c, d) {
            (PAD, PAD) if last => {
                let [_, first, rest @ ..] = head.to_be_bytes();
                if rest != [0, 0] {
                    return Err(Base64Error::TrailingBits);
                }
                bytes.push(first);
            }
            (c, PAD) if last => {
                let [_, first, second, rest] = (head | sextet(c)? << 6).to_be_bytes();
                if rest != 0 {
                    return Err(Base64Error::TrailingBits);
                }
                bytes.extend([first, second]);
            }
            (c, d) => {
                let [_, group @ ..] = (head | sextet(c)? << 6 | sextet(d)?).to_be_bytes();
                bytes.extend(group);
            }
        }
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use base64::engine::general_purpose::{STANDARD, URL_SAFE};
    use proptest::prelude::*;

    use super::{Alphabet, decode, encode};

    /// The crate engine this codec replaces, for each alphabet.
    fn engine(alphabet: Alphabet) -> base64::engine::GeneralPurpose {
        match alphabet {
            Alphabet::Standard => STANDARD,
            Alphabet::UrlSafe => URL_SAFE,
        }
    }

    fn alphabet() -> impl Strategy<Value = Alphabet> {
        prop_oneof![Just(Alphabet::Standard), Just(Alphabet::UrlSafe)]
    }

    /// A text near base64: symbols of both alphabets, the pad, and bytes
    /// outside every alphabet, so each refusal of the crate is drawn.
    fn near_text() -> impl Strategy<Value = String> {
        proptest::collection::vec(
            prop_oneof![
                8 => proptest::sample::select(
                    b"AQgw/+_-=".iter().map(|&b| char::from(b)).collect::<Vec<_>>()
                ),
                4 => proptest::char::range('A', 'z'),
                1 => Just('\n'),
                1 => Just('\u{e9}'),
            ],
            0..24,
        )
        .prop_map(|chars| chars.into_iter().collect())
    }

    /// The text of some bytes with one symbol overwritten, so a pad, a
    /// trailing bit or a foreign symbol lands inside an otherwise valid text.
    fn mutated_text() -> impl Strategy<Value = String> {
        (
            alphabet(),
            proptest::collection::vec(any::<u8>(), 1..20),
            any::<prop::sample::Index>(),
            proptest::sample::select(b"AB/+-_=*".to_vec()),
        )
            .prop_map(|(alphabet, bytes, at, symbol)| {
                let mut text = encode(alphabet, &bytes).into_bytes();
                let at = at.index(text.len());
                if let Some(slot) = text.get_mut(at) {
                    *slot = symbol;
                }
                String::from_utf8(text).unwrap_or_default()
            })
    }

    proptest! {
        /// The codec writes, byte for byte, the text the crate writes.
        #[test]
        fn the_codec_writes_the_text_of_the_crate(
            alphabet in alphabet(),
            bytes in proptest::collection::vec(any::<u8>(), 0..200),
        ) {
            prop_assert_eq!(encode(alphabet, &bytes), engine(alphabet).encode(&bytes));
        }

        /// The codec reads back what it writes.
        #[test]
        fn the_codec_reads_what_it_writes(
            alphabet in alphabet(),
            bytes in proptest::collection::vec(any::<u8>(), 0..200),
        ) {
            prop_assert_eq!(decode(alphabet, &encode(alphabet, &bytes)), Ok(bytes));
        }

        /// The codec refuses each text the crate refuses, and reads the
        /// bytes the crate reads from every other.
        #[test]
        fn the_codec_reads_and_refuses_as_the_crate(
            alphabet in alphabet(),
            text in prop_oneof![near_text(), mutated_text()],
        ) {
            prop_assert_eq!(
                decode(alphabet, &text).ok(),
                engine(alphabet).decode(text.as_bytes()).ok()
            );
        }
    }

    /// The edges a random draw reaches seldom, each against the crate.
    #[test]
    fn the_codec_meets_the_crate_on_each_edge() {
        let edges = [
            "", "=", "==", "====", "A===", "AA==", "AB==", "AAA=", "AAB=", "AAAA", "AA=A", "A=AA",
            "AA==AAAA", "AAA=AAAA", "AAAAAA", "AA", "AAA", "AA===", "Zg==", "Zm8=", "Zm9v",
        ];
        for alphabet in [Alphabet::Standard, Alphabet::UrlSafe] {
            for text in edges {
                assert_eq!(
                    decode(alphabet, text).ok(),
                    engine(alphabet).decode(text).ok(),
                    "{alphabet:?} on {text:?}"
                );
            }
        }
    }
}
