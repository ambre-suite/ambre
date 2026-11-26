//! The recovery phrase of a master seed: 24 words of the BIP39 English list
//! (`ambre-spec-zero-knowledge.md section 2`, `ZK-043`).
//!
//! Only the BIP39 mapping between entropy and words is used. The 256 bits of
//! the seed and the first 8 bits of their SHA-256 digest make 264 bits. 24
//! words of 11 bits each spell them. The BIP39 step from a phrase to a seed,
//! PBKDF2 over the words, is not used. The words spell the master seed
//! itself, and the key hierarchy of the spec derives from that seed.

use ambre_core::{Confidential, RecoveryWords, RecoveryWordsRefusal, Secret};
use sha2::{Digest, Sha256};

use super::MasterSeed;

/// The BIP39 English word list, verbatim from `bitcoin/bips`
/// (`bip-0039/english.txt`): 2048 words, one per line, sorted. A test pins
/// its SHA-256 digest, so an edit of one byte fails the build.
const WORD_LIST: &str = include_str!("bip39-english.txt");

/// The words of a phrase: 264 bits at 11 bits per word.
const PHRASE_WORDS: usize = 24;

/// The bits that one word carries: the list holds 2^11 words.
const WORD_BITS: usize = 11;

impl MasterSeed {
    /// The recovery phrase of the seed: 24 BIP39 English words, lowercase,
    /// one space apart, as the screen of a new account shows them once. The
    /// one way out of the root seed, and it hands out a secret that is wiped
    /// in turn.
    pub fn recovery_words(&self) -> Confidential<RecoveryWords> {
        let seed = self.0.expose();
        // The checksum of BIP39 for 256 bits: the first byte of their SHA-256
        // digest. The rest of the digest stays on the stack, unwiped, as the
        // working state of a primitive does (spec section 2).
        let check = Sha256::digest(seed).first().copied().unwrap_or_default();
        // Sized once for 24 words of at most 8 letters and their spaces.
        // No reallocation leaves a copy behind, and the move into the wrapper
        // moves the buffer, not the bytes.
        let mut text = String::with_capacity(PHRASE_WORDS * 9);
        for word in 0..PHRASE_WORDS {
            // The 264 bits the words spell: the seed, then the checksum.
            let index = (0..WORD_BITS).fold(0_usize, |index, bit| {
                let place = word * WORD_BITS + bit;
                let byte = seed.get(place / 8).copied().unwrap_or(check);
                (index << 1) | usize::from((byte >> (7 - place % 8)) & 1)
            });
            if let Some(entry) = WORD_LIST.lines().nth(index) {
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(entry);
            }
        }
        Confidential::<RecoveryWords>::new(text)
    }

    /// Reads a master seed back from its recovery phrase. Words match the
    /// list without regard to ASCII case, and any run of whitespace parts
    /// them.
    ///
    /// The phrase is checked whole before the seed leaves this function, so a
    /// refused phrase derives no key (`ZK-043`).
    ///
    /// # Errors
    ///
    /// * [`RecoveryWordsRefusal::WrongWordCount`] when the phrase does not
    ///   hold 24 words.
    /// * [`RecoveryWordsRefusal::UnknownWord`] when a word is not on the list.
    /// * [`RecoveryWordsRefusal::BadChecksum`] when the last 8 bits do not
    ///   match the digest of the first 256.
    pub fn from_recovery_words(
        words: &Confidential<RecoveryWords>,
    ) -> Result<Self, RecoveryWordsRefusal> {
        let phrase = words.secret().expose();
        if phrase.split_whitespace().count() != PHRASE_WORDS {
            return Err(RecoveryWordsRefusal::WrongWordCount);
        }
        let mut check = 0_u8;
        // The seed is written in place inside its secret, so a refusal wipes
        // the bits read so far.
        let seed = Secret::<[u8; 32]>::try_filled(|seed| {
            for (word, text) in phrase.split_whitespace().enumerate() {
                let index = WORD_LIST
                    .lines()
                    .position(|entry| entry.eq_ignore_ascii_case(text))
                    .ok_or(RecoveryWordsRefusal::UnknownWord)?;
                for bit in 0..WORD_BITS {
                    if (index >> (WORD_BITS - 1 - bit)) & 1 == 1 {
                        let place = word * WORD_BITS + bit;
                        let mask = 0x80_u8 >> (place % 8);
                        match seed.get_mut(place / 8) {
                            Some(byte) => *byte |= mask,
                            None => check |= mask,
                        }
                    }
                }
            }
            Ok(())
        })?;
        if Sha256::digest(seed.expose()).first() != Some(&check) {
            return Err(RecoveryWordsRefusal::BadChecksum);
        }
        Ok(Self(seed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Holds the 24-word vectors of the reference implementation
    /// (`trezor/python-mnemonic`, `vectors.json`): the entropy in hex, then
    /// the phrase. The PBKDF2 seed of each vector is left out: Ambre does
    /// not use it.
    const VECTORS: [(&str, &str); 8] = [
        (
            "0000000000000000000000000000000000000000000000000000000000000000",
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art",
        ),
        (
            "7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f",
            "legal winner thank year wave sausage worth useful legal winner thank year wave sausage worth useful legal winner thank year wave sausage worth title",
        ),
        (
            "8080808080808080808080808080808080808080808080808080808080808080",
            "letter advice cage absurd amount doctor acoustic avoid letter advice cage absurd amount doctor acoustic avoid letter advice cage absurd amount doctor acoustic bless",
        ),
        (
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo vote",
        ),
        (
            "68a79eaca2324873eacc50cb9c6eca8cc68ea5d936f98787c60c7ebc74e6ce7c",
            "hamster diagram private dutch cause delay private meat slide toddler razor book happy fancy gospel tennis maple dilemma loan word shrug inflict delay length",
        ),
        (
            "9f6a2878b2520799a44ef18bc7df394e7061a224d2c33cd015b157d746869863",
            "panda eyebrow bullet gorilla call smoke muffin taste mesh discover soft ostrich alcohol speed nation flash devote level hobby quick inner drive ghost inside",
        ),
        (
            "066dca1a2bb7e8a1db2832148ce9933eea0f3ac9548d793112d9a95c9407efad",
            "all hour make first leader extend hole alien behind guard gospel lava path output census museum junior mass reopen famous sing advance salt reform",
        ),
        (
            "f585c11aec520db57dd353c69554b21a89b20fb0650966fa0a9d6f74fd989d8f",
            "void come effort suffer camp survey warrior heavy shoot primary clutch crush open amazing screen patrol group space point ten exist slush involve unfold",
        ),
    ];

    fn seed_of(hex: &str) -> MasterSeed {
        let bytes = data_encoding::HEXLOWER.decode(hex.as_bytes()).unwrap();
        MasterSeed::from_array(bytes.try_into().unwrap())
    }

    fn words(text: &str) -> Confidential<RecoveryWords> {
        Confidential::<RecoveryWords>::new(text.to_owned())
    }

    /// The list is the official one: its SHA-256 digest, 2048 words, sorted
    /// and unique, so the binary search of a reader elsewhere agrees.
    #[test]
    fn the_word_list_is_the_bip39_english_list() {
        assert_eq!(
            data_encoding::HEXLOWER.encode(&Sha256::digest(WORD_LIST.as_bytes())),
            "2f5eed53a4727b4bf8880d8f3f199efc90e58503646d9ff8eff3a2ed3b24dbda"
        );
        let entries: Vec<&str> = WORD_LIST.lines().collect();
        assert_eq!(entries.len(), 1 << WORD_BITS);
        assert!(entries.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(entries.iter().all(|entry| entry.len() <= 8));
    }

    /// `ZK-043`: the seed spells the phrase of the official vectors, and the
    /// phrase reads back to the same seed.
    #[test]
    fn the_recovery_words_match_the_bip39_vectors() {
        for (entropy, phrase) in VECTORS {
            let seed = seed_of(entropy);
            assert_eq!(seed.recovery_words().secret().expose(), phrase);
            let back = MasterSeed::from_recovery_words(&words(phrase)).unwrap();
            assert_eq!(back.0.expose(), seed.0.expose(), "{entropy}");
        }
    }

    /// The reader takes any ASCII case and any run of whitespace.
    #[test]
    fn the_recovery_words_read_back_in_any_case_and_spacing() {
        let (entropy, phrase) = VECTORS[5];
        let loose = format!("  {}\n", phrase.to_ascii_uppercase().replace(' ', " \t "));
        let back = MasterSeed::from_recovery_words(&words(&loose)).unwrap();
        assert_eq!(back.0.expose(), seed_of(entropy).0.expose());
    }

    /// `ZK-043`: a word off the list is refused, whatever its place.
    #[test]
    fn a_mistyped_recovery_word_is_refused() {
        let (_, phrase) = VECTORS[4];
        let mut list: Vec<&str> = phrase.split(' ').collect();
        for place in [0, 11, 23] {
            let kept = list[place];
            list[place] = "hamstr";
            let refusal = MasterSeed::from_recovery_words(&words(&list.join(" "))).err();
            assert_eq!(refusal, Some(RecoveryWordsRefusal::UnknownWord), "{place}");
            list[place] = kept;
        }
    }

    /// `ZK-043`: a valid word in the wrong place breaks the checksum.
    #[test]
    fn a_recovery_phrase_with_a_bad_checksum_is_refused() {
        let (_, phrase) = VECTORS[0];
        let refusal =
            MasterSeed::from_recovery_words(&words(&phrase.replace("art", "abandon"))).err();
        assert_eq!(refusal, Some(RecoveryWordsRefusal::BadChecksum));
        let (_, phrase) = VECTORS[6];
        let swapped = phrase.replacen("all hour", "hour all", 1);
        let refusal = MasterSeed::from_recovery_words(&words(&swapped)).err();
        assert_eq!(refusal, Some(RecoveryWordsRefusal::BadChecksum));
    }

    /// A phrase of 23 or 25 words is refused, and so is the old hex form.
    #[test]
    fn a_recovery_phrase_of_the_wrong_length_is_refused() {
        let (_, phrase) = VECTORS[1];
        let short = phrase.rsplit_once(' ').unwrap().0;
        let long = format!("{phrase} title");
        for text in [short, long.as_str(), &"00".repeat(32), ""] {
            let refusal = MasterSeed::from_recovery_words(&words(text)).err();
            assert_eq!(refusal, Some(RecoveryWordsRefusal::WrongWordCount));
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    proptest::proptest! {
        /// `ZK-043`: every seed spells 24 listed words that read back to it.
        #[test]
        fn any_seed_round_trips_through_its_recovery_words(bytes: [u8; 32]) {
            let seed = MasterSeed::from_array(bytes);
            let phrase = seed.recovery_words();
            proptest::prop_assert_eq!(phrase.secret().expose().split(' ').count(), PHRASE_WORDS);
            let back = MasterSeed::from_recovery_words(&phrase).unwrap();
            proptest::prop_assert_eq!(back.0.expose(), &bytes);
        }
    }
}
