//! The strength floor of a new passphrase (`ambre-spec-zero-knowledge.md`,
//! `ZK-035`).
//!
//! The relay stores the salt and the Master Seed sealed under the
//! passphrase, so the relay, or a thief of its database, can guess offline.
//! Argon2id sets the price of one guess, and this module refuses a passphrase
//! that too few guesses find. The floor and its arithmetic live in section 2
//! of the zero-knowledge spec; this module holds the estimator.
//!
//! The estimator is compact on purpose: a short list of common passwords,
//! the patterns a guess tool tries first, and a length model. It credits a
//! word with the bits of a draw from a list of 8 192 words, whatever its
//! length. So it never credits a phrase with more than a word-list attack
//! needs, and its errors fall on the side of a refusal.
//!
//! The estimate never leaves this crate. A refusal names the weakness that
//! dominated it, and nothing else. The one copy of the passphrase the
//! estimator makes is a [`Secret`], wiped when the estimate ends.

use ambre_core::{Confidential, Count, NarrowCount, Passphrase, PassphraseWeakness, Secret};

/// Hundredths of a bit of guess entropy: integer arithmetic, so the estimate
/// reads the same on every target.
enum Centibits {}

impl NarrowCount for Centibits {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("Centibits");
}

/// Characters of the passphrase.
enum Characters {}

impl NarrowCount for Characters {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("Characters");
}

/// How many times a unit of the passphrase repeats, or a word list draws.
enum Draws {}

impl NarrowCount for Draws {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> = ambre_core::StaticKey::new("Draws");
}

/// The floor: 48 bits. Section 2 of `ambre-spec-zero-knowledge.md` derives it
/// from the Argon2id cost and the attacker it names.
const FLOOR: Count<Centibits> = Count::new(4_800);

/// One character drawn from the 26 letters: log2(26).
const LETTER: Count<Centibits> = Count::new(470);
/// One digit: log2(10).
const DIGIT: Count<Centibits> = Count::new(332);
/// One character drawn from the 33 printable ASCII symbols: log2(33).
const SYMBOL: Count<Centibits> = Count::new(504);
/// One character of a run that reads as words: about the entropy of English
/// text, and less than one random letter.
const WORD_CHAR: Count<Centibits> = Count::new(250);
/// One word drawn from a list of 8 192 words: log2(8192).
const WORD: Count<Centibits> = Count::new(1_300);
/// The longest word of the EFF list a diceware roll draws from. A run of
/// words with no separator divides by it to count its words, so three words
/// written together never count as four.
const WORD_LEN: Count<Characters> = Count::new(9);
/// One more choice of two: the case of a word, the separator of a phrase, a
/// token said again, a substitution of digits for letters.
const ONE_BIT: Count<Centibits> = Count::new(100);
/// A separator the phrase has not used before: one of about four.
const NEW_SEPARATOR: Count<Centibits> = Count::new(200);
/// A year of the last two centuries: log2(150).
const YEAR: Count<Centibits> = Count::new(720);
/// A date of six or eight digits: log2(366 * 100).
const DATE: Count<Centibits> = Count::new(1_520);

/// The passwords a guess list tries first, most common first, one space
/// between two. The rank of a match is its cost. One text, not a slice of
/// texts, saves a pointer and a length per entry in the Sync worker. Each entry is lower case ASCII letters: a match folds
/// the case and the common digit-for-letter substitutions first.
const COMMON: &str = concat!(
    "password qwerty azerty letmein iloveyou admin welcome monkey dragon master ",
    "sunshine princess football baseball soccer shadow superman batman trustno hello ",
    "freedom whatever secret login access starwars pokemon hunter ninja mustang ",
    "michael jennifer jordan charlie computer internet summer winter spring autumn ",
    "flower cookie chocolate changeme default guest root user test love motdepasse ",
    "soleil bonjour loulou doudou chouchou marseille passwort hallo contrasena ambre ",
    "google facebook apple orange banana killer pepper ginger tigger maggie buster ",
    "daniel thomas robert matrix zaq abc",
);

/// The rows a guess tool walks, one space between two: the alphabet, the
/// digits and the keyboard rows of the QWERTY, AZERTY and QWERTZ layouts. A
/// run of three or more of one, in either direction, is a sequence.
const SEQUENCES: &str = concat!(
    "abcdefghijklmnopqrstuvwxyz qwertyuiop asdfghjkl zxcvbnm azertyuiop qsdfghjklm ",
    "wxcvbn qwertzuiop yxcvbnm 01234567890",
);

/// A new passphrase that the strength floor admits. Only [`Self::check`]
/// builds one, and only a chosen passphrase derives the key that seals the
/// Master Seed for storage (`crate::passphrase::derive_sealing_key`).
pub struct ChosenPassphrase(Confidential<Passphrase>);

impl ChosenPassphrase {
    /// Admits `passphrase` when its estimate reaches the floor.
    ///
    /// # Errors
    ///
    /// The [`PassphraseWeakness`] that dominated the estimate, when it falls
    /// under the floor. The passphrase is dropped, and so wiped, then.
    pub fn check(passphrase: Confidential<Passphrase>) -> Result<Self, PassphraseWeakness> {
        let estimate = estimate(&passphrase);
        if estimate.bits >= FLOOR {
            Ok(Self(passphrase))
        } else {
            Err(estimate.weakness())
        }
    }

    /// The admitted passphrase, for the derivation that seals.
    pub(crate) fn passphrase(&self) -> &Confidential<Passphrase> {
        &self.0
    }
}

/// The bits a guess needs, and the strongest-ranked pattern met on the way:
/// the weakness a refusal names. [`PassphraseWeakness`] ranks its variants in
/// the order a refusal prefers them.
#[derive(Clone, Copy)]
struct Estimate {
    bits: Count<Centibits>,
    met: Option<PassphraseWeakness>,
}

impl Estimate {
    const NOTHING: Self = Self::plain(Count::ZERO);

    const fn plain(bits: Count<Centibits>) -> Self {
        Self { bits, met: None }
    }

    fn with(self, met: PassphraseWeakness) -> Self {
        self.plus(Self {
            bits: Count::ZERO,
            met: Some(met),
        })
    }

    fn plus(self, other: Self) -> Self {
        let met = match (self.met, other.met) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        Self {
            bits: self.bits.plus(other.bits),
            met,
        }
    }

    fn weakness(self) -> PassphraseWeakness {
        self.met.unwrap_or(PassphraseWeakness::TooShort)
    }
}

/// The estimate of `passphrase`: the lower of the text read as typed and the
/// text with its substitutions folded back to letters. A digit or a symbol
/// folds only beside a letter, so a run of digits keeps its digits.
fn estimate(passphrase: &Confidential<Passphrase>) -> Estimate {
    let typed = Chars::of(passphrase);
    let as_typed = typed.estimate();
    let folded = typed.folded();
    if folded.0.expose() == typed.0.expose() {
        return as_typed;
    }
    let as_folded = folded.estimate().plus(Estimate::plain(ONE_BIT));
    if as_folded.bits < as_typed.bits {
        as_folded
    } else {
        as_typed
    }
}

/// The characters of a passphrase, in a buffer that wipes itself on drop.
struct Chars(Secret<Vec<char>>);

impl Chars {
    fn of(passphrase: &Confidential<Passphrase>) -> Self {
        Self(Secret::new(passphrase.secret().expose().chars().collect()))
    }

    /// The characters with each substitution folded back to its letter
    /// beside a letter, as inside a word.
    fn folded(&self) -> Self {
        let typed = self.0.expose();
        let letter = |i: Option<usize>| {
            i.and_then(|i| typed.get(i))
                .is_some_and(|&c| Class::of(c) == Class::Letter)
        };
        Self(Secret::new(
            typed
                .iter()
                .enumerate()
                .map(|(i, &c)| match c {
                    _ if !letter(i.checked_sub(1)) && !letter(Some(i + 1)) => c,
                    '0' => 'o',
                    '1' => 'i',
                    '3' => 'e',
                    '4' | '@' => 'a',
                    '5' | '$' => 's',
                    '7' => 't',
                    other => other,
                })
                .collect(),
        ))
    }

    /// The sum of the runs of the text. A run is a maximal stretch of one
    /// class. A capital after a small letter starts a run too, so each word of
    /// a phrase written in camel case counts as one word.
    fn estimate(&self) -> Estimate {
        let text = self.0.expose();
        let class_at = |i: Option<usize>| i.and_then(|i| text.get(i)).map(|&c| Class::of(c));
        let apart = |class: Option<Class>| class.is_none_or(|class| class == Class::Separator);
        let mut total = Estimate::NOTHING;
        let mut start = 0;
        while let Some(&first) = text.get(start) {
            let class = Class::of(first);
            let mut end = start + 1;
            while let Some(&c) = text.get(end) {
                let camel = text.get(end - 1).is_some_and(char::is_ascii_lowercase)
                    && c.is_ascii_uppercase();
                if Class::of(c) != class || camel {
                    break;
                }
                end += 1;
            }
            let chars = text.get(start..end).unwrap_or_default();
            let standing = if apart(class_at(start.checked_sub(1))) && apart(class_at(Some(end))) {
                Standing::Alone
            } else {
                Standing::Embedded
            };
            let run = Run {
                class,
                chars,
                standing,
            };
            let said_before = text
                .get(..start)
                .is_some_and(|before| before.windows(chars.len()).any(|w| w == chars));
            total = total.plus(if said_before {
                run.said_again()
            } else {
                run.estimate()
            });
            start = end;
        }
        total
    }
}

/// What a character is, for the split into runs. Every character outside
/// ASCII counts as a letter. The Unicode tables of the standard library weigh
/// kilobytes in the Sync worker. A letter is also the cheaper reading of a
/// character the estimator cannot class.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Letter,
    Digit,
    Separator,
    Symbol,
}

impl Class {
    fn of(c: char) -> Self {
        if c.is_ascii_alphabetic() || !c.is_ascii() {
            Self::Letter
        } else if c.is_ascii_digit() {
            Self::Digit
        } else if matches!(c, ' ' | '-' | '_' | '.' | ',' | '/' | '+' | ':' | ';') {
            Self::Separator
        } else {
            Self::Symbol
        }
    }
}

/// Whether a run of letters stands alone between separators or the ends of
/// the text, as a word of a phrase does.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Standing {
    Embedded,
    Alone,
}

/// What a run of letters is, for its cost.
enum Shape {
    Common(Count<Draws>),
    SameChar,
    Sequence,
    Words,
    Random,
}

/// One run of the passphrase: a view into its wiped buffer.
struct Run<'a> {
    class: Class,
    chars: &'a [char],
    standing: Standing,
}

impl Run<'_> {
    fn len(&self) -> Count<Characters> {
        Count::of(self.chars.len())
    }

    /// A run said before costs the choice to say it again. A separator said
    /// again costs nothing: the phrase chose it once.
    fn said_again(&self) -> Estimate {
        if self.class == Class::Separator {
            Estimate::NOTHING
        } else {
            Estimate::plain(ONE_BIT).with(PassphraseWeakness::Repetition)
        }
    }

    fn estimate(&self) -> Estimate {
        match self.class {
            Class::Letter => self.letters(),
            Class::Digit => self.digits(),
            Class::Symbol => self.symbols(),
            Class::Separator => Estimate::plain(NEW_SEPARATOR),
        }
    }

    /// What the run is, in the order a guess tool tries the shapes:
    /// 1. a common password;
    /// 2. one character said again;
    /// 3. a row of [`SEQUENCES`], forward or backward;
    /// 4. words;
    /// 5. random letters.
    ///
    /// A run reads as words when it holds a vowel in four letters and no
    /// five consonants in a row. A run of one or two letters reads as a word
    /// when it stands alone. A letter outside ASCII counts as a vowel, so a
    /// script with no vowel reads as words: the cheaper reading.
    fn shape(&self) -> Shape {
        let len = self.chars.len();
        let is = |c: &char, b: u8| c.to_ascii_lowercase() == char::from(b);
        let spells = |word: &str| {
            len == word.len() && self.chars.iter().zip(word.bytes()).all(|(c, b)| is(c, b))
        };
        let along = |window: &[u8], direction: Direction| {
            window.iter().enumerate().all(|(i, &b)| {
                let at = match direction {
                    Direction::Forward => Some(i),
                    Direction::Backward => len.checked_sub(i + 1),
                };
                at.and_then(|at| self.chars.get(at))
                    .is_some_and(|c| is(c, b))
            })
        };
        let in_a_row = len >= 3
            && SEQUENCES.split(' ').any(|row| {
                row.as_bytes().windows(len).any(|window| {
                    along(window, Direction::Forward) || along(window, Direction::Backward)
                })
            });
        let vowel = |c: &char| {
            !c.is_ascii() || matches!(c.to_ascii_lowercase(), 'a' | 'e' | 'i' | 'o' | 'u' | 'y')
        };
        let vowels = self.chars.iter().filter(|c| vowel(c)).count();
        let longest_consonants = self
            .chars
            .split(vowel)
            .map(<[char]>::len)
            .max()
            .unwrap_or(0);
        let words = self.class == Class::Letter
            && if len < 3 {
                self.standing == Standing::Alone
            } else {
                vowels * 4 >= len && longest_consonants < 5
            };
        let one_char = len > 1
            && self
                .chars
                .windows(2)
                .all(|pair| pair.first() == pair.last());
        if let Some(rank) = COMMON.split(' ').position(spells) {
            Shape::Common(Count::of(rank + 1))
        } else if one_char {
            Shape::SameChar
        } else if in_a_row {
            Shape::Sequence
        } else if words {
            Shape::Words
        } else {
            Shape::Random
        }
    }

    /// The bits of the case of a run. Lower case costs none. A capital first
    /// letter, or a run all in capitals, costs one bit. Else each capital
    /// costs one bit.
    fn case(&self) -> Count<Centibits> {
        let capitals = self.chars.iter().filter(|c| c.is_ascii_uppercase()).count();
        let first = self.chars.first().is_some_and(char::is_ascii_uppercase);
        if capitals == 0 {
            Count::ZERO
        } else if (first && capitals == 1) || capitals == self.chars.len() {
            ONE_BIT
        } else {
            ONE_BIT.times(Count::<Characters>::of(capitals))
        }
    }

    fn letters(&self) -> Estimate {
        let len = self.len();
        let base = match self.shape() {
            Shape::Common(rank) => {
                Estimate::plain(log2(rank).plus(ONE_BIT)).with(PassphraseWeakness::CommonPassword)
            }
            Shape::SameChar => same_char(LETTER, len),
            Shape::Sequence => sequence(LETTER, len),
            Shape::Words => {
                let words = Count::<Characters>::new((len.get() / WORD_LEN.get()).max(1));
                Estimate::plain(WORD_CHAR.times(len).min(WORD.times(words)))
            }
            Shape::Random => Estimate::plain(LETTER.times(len)),
        };
        base.plus(Estimate::plain(self.case()))
    }

    fn digits(&self) -> Estimate {
        let len = self.len();
        let first_two = (self.chars.first(), self.chars.get(1));
        let a_year = matches!(first_two, (Some('1'), Some('9')) | (Some('2'), Some('0')));
        match self.shape() {
            Shape::SameChar => same_char(DIGIT, len),
            Shape::Sequence => sequence(DIGIT, len),
            _ if len.get() == 4 && a_year => Estimate::plain(YEAR),
            _ if matches!(len.get(), 6 | 8) => Estimate::plain(DATE.min(DIGIT.times(len))),
            _ => Estimate::plain(DIGIT.times(len)),
        }
    }

    fn symbols(&self) -> Estimate {
        let len = self.len();
        match self.shape() {
            Shape::SameChar => same_char(SYMBOL, len),
            _ => Estimate::plain(SYMBOL.times(len)),
        }
    }
}

/// Which way a run reads along a row.
#[derive(Clone, Copy)]
enum Direction {
    Forward,
    Backward,
}

/// Bits per thing, times a count of things.
trait Times {
    fn times<K: NarrowCount>(self, count: Count<K>) -> Self;
}

impl Times for Count<Centibits> {
    fn times<K: NarrowCount>(self, count: Count<K>) -> Self {
        Count::new(self.get().saturating_mul(count.get()))
    }
}

/// log2 of a count, rounded down, as bits.
fn log2<K: NarrowCount>(count: Count<K>) -> Count<Centibits> {
    Count::new(count.get().checked_ilog2().unwrap_or(0).saturating_mul(100))
}

/// A run of one character said `len` times.
fn same_char(first: Count<Centibits>, len: Count<Characters>) -> Estimate {
    Estimate::plain(first.plus(log2(len))).with(PassphraseWeakness::Repetition)
}

/// A run that a row of [`SEQUENCES`] holds.
fn sequence(first: Count<Centibits>, len: Count<Characters>) -> Estimate {
    Estimate::plain(first.plus(log2(len)).plus(ONE_BIT)).with(PassphraseWeakness::Sequence)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pass(text: &str) -> Confidential<Passphrase> {
        Confidential::<Passphrase>::new(text.to_owned())
    }

    fn refusal(text: &str) -> Option<PassphraseWeakness> {
        ChosenPassphrase::check(pass(text)).err()
    }

    #[test]
    fn a_known_weak_password_is_refused_with_its_weakness() {
        let cases = [
            ("password", PassphraseWeakness::CommonPassword),
            ("P@ssw0rd!", PassphraseWeakness::CommonPassword),
            ("Password123", PassphraseWeakness::CommonPassword),
            ("iloveyou", PassphraseWeakness::CommonPassword),
            ("motdepasse2024", PassphraseWeakness::CommonPassword),
            ("qwertyuiop", PassphraseWeakness::Sequence),
            ("azerty123", PassphraseWeakness::CommonPassword),
            ("asdfgh123", PassphraseWeakness::Sequence),
            ("abcdefghijklmnop", PassphraseWeakness::Sequence),
            ("1234567890", PassphraseWeakness::Sequence),
            ("aaaaaaaaaaaaaaaaaaaa", PassphraseWeakness::Repetition),
            ("blue blue blue blue blue", PassphraseWeakness::Repetition),
            ("Tr0ub4dor&3", PassphraseWeakness::TooShort),
            ("Summer2024!", PassphraseWeakness::CommonPassword),
            ("correct horse", PassphraseWeakness::TooShort),
            ("x7#kQ2", PassphraseWeakness::TooShort),
            ("m0nk3yf00tb4ll", PassphraseWeakness::TooShort),
            ("", PassphraseWeakness::TooShort),
        ];
        for (text, weakness) in cases {
            assert_eq!(refusal(text), Some(weakness), "{text}");
        }
    }

    #[test]
    fn three_diceware_words_fall_under_the_floor_and_four_reach_it() {
        // Words of the EFF large list, 12.9 bits each: three give 38.8 bits,
        // under the floor, and four give 51.7, over it.
        for three in [
            "crane motion velvet",
            "anchovy-glider-ruckus",
            "unsorted_payable_cubicle",
            "CraneMotionVelvet",
            "ruckuspayableglider",
            "marmaladetadpoleoutdoors",
        ] {
            assert_eq!(
                refusal(three),
                Some(PassphraseWeakness::TooShort),
                "{three}"
            );
        }
        for four in [
            "correct horse battery staple",
            "anchovy-glider-ruckus-tadpole",
            "unsorted payable cubicle mammal",
            "Crane Motion Velvet Pasture",
            "CraneMotionVelvetPasture",
            "glider ruckus payable cubicle tadpole",
        ] {
            assert_eq!(refusal(four), None, "{four}");
        }
    }

    /// Words of the EFF large list, the list a diceware roll draws from.
    const DICEWARE: &[&str] = &[
        "anchovy",
        "glider",
        "ruckus",
        "tadpole",
        "payable",
        "cubicle",
        "mammal",
        "velvet",
        "motion",
        "pasture",
        "crane",
        "unsorted",
        "battery",
        "staple",
        "harmony",
        "lettuce",
        "gravity",
        "nebula",
        "pelican",
        "outdoors",
        "wobbly",
        "sizable",
        "unloved",
        "clamor",
        "dolphin",
        "ladle",
        "marmalade",
        "opponent",
        "rotunda",
        "sapling",
        "tribune",
        "voucher",
    ];

    fn phrase(picks: &[usize], separator: &str) -> String {
        picks
            .iter()
            .filter_map(|&i| DICEWARE.get(i % DICEWARE.len()).copied())
            .collect::<Vec<_>>()
            .join(separator)
    }

    proptest::proptest! {
        #[test]
        fn any_four_distinct_diceware_words_reach_the_floor(
            picks in proptest::sample::subsequence((0..DICEWARE.len()).collect::<Vec<_>>(), 4),
            separator in proptest::sample::select(vec![" ", "-", "_", "."]),
        ) {
            proptest::prop_assert_eq!(refusal(&phrase(&picks, separator)), None);
        }

        #[test]
        fn any_three_diceware_words_fall_under_the_floor(
            picks in proptest::collection::vec(0..DICEWARE.len(), 3),
            separator in proptest::sample::select(vec![" ", "-", "_", ".", ""]),
        ) {
            proptest::prop_assert!(refusal(&phrase(&picks, separator)).is_some());
        }
    }

    #[test]
    fn a_random_password_of_enough_characters_reaches_the_floor() {
        for random in [
            "k8#Qz!v2Lm@9",
            "x9TqLw3vRz7P",
            "Zp4!nR8@kw2#Xy",
            "qxzvbnwkrtplmhgj",
        ] {
            assert_eq!(refusal(random), None, "{random}");
        }
    }

    #[test]
    fn a_word_is_credited_no_more_than_a_draw_from_a_word_list() {
        // One long word reads as at most three draws, under the floor.
        assert_eq!(
            refusal("internationalization"),
            Some(PassphraseWeakness::TooShort)
        );
        assert!(estimate(&pass("internationalization")).bits <= WORD.times(Count::<Draws>::new(3)));
    }

    #[test]
    fn a_substitution_never_raises_the_estimate_past_the_letters_it_hides() {
        let plain_text = estimate(&pass("monkeyfootball")).bits;
        let folded = estimate(&pass("m0nk3yf00tb4ll")).bits;
        assert!(folded <= plain_text.plus(ONE_BIT));
    }

    #[test]
    fn a_refused_passphrase_is_dropped_and_an_admitted_one_kept_whole() {
        let chosen = ChosenPassphrase::check(pass("anchovy glider ruckus tadpole")).ok();
        assert_eq!(
            chosen.map(|c| c.passphrase().secret().expose().clone()),
            Some("anchovy glider ruckus tadpole".to_owned())
        );
    }
}
