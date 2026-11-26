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
//! needs, and its errors fall on the side of a refusal. A run of letters
//! reads as random only when it holds no vowel at all.
//!
//! The estimate never leaves this crate. A refusal names the weakness that
//! dominated it, and nothing else. Each copy of the passphrase the estimator
//! makes is a [`Secret`], wiped when the estimate ends. Each one takes its
//! size before its first character, so it never moves.

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

/// The rows a guess tool walks, one space between two. They are the
/// alphabet, the digits, and the keyboard rows of the QWERTY, AZERTY and
/// QWERTZ layouts. The last three are the columns of each layout, read top
/// to bottom, then left to right. A row reads as a ring, in either
/// direction, so a walk longer than its row still follows it. A walk is
/// three characters or more.
const SEQUENCES: &str = concat!(
    "abcdefghijklmnopqrstuvwxyz 0123456789 qwertyuiop asdfghjkl zxcvbnm ",
    "azertyuiop qsdfghjklm wxcvbn qwertzuiop yxcvbnm ",
    "qazwsxedcrfvtgbyhnujmikolp aqwzsxedcrfvtgbyhnujikolpm qaywsxedcrfvtgbzhnujmikolp",
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

/// The estimate of `passphrase`: the lowest of three readings.
/// 1. The text as typed.
/// 2. The text with its substitutions folded back to letters, for one bit
///    more. A digit or a symbol folds only beside a letter.
/// 3. The text read as one run, its separators and its camel case taken
///    out, when the rest is a pattern.
///
/// A passphrase of digits alone stays under the floor (`Chars::capped`).
fn estimate(passphrase: &Confidential<Passphrase>) -> Estimate {
    let typed = Chars::of(passphrase);
    let mut best = typed.estimate();
    let folded = typed.folded();
    if folded.0.expose() != typed.0.expose() {
        best = lower(best, folded.estimate().plus(Estimate::plain(ONE_BIT)));
    }
    if let Some(joined) = typed.joined() {
        best = lower(best, joined);
    }
    typed.capped(best)
}

/// `low` when it costs fewer bits than `kept`, else `kept`: a tie keeps the
/// reading the caller holds.
fn lower(kept: Estimate, low: Estimate) -> Estimate {
    if low.bits < kept.bits { low } else { kept }
}

/// The characters of a passphrase, in a buffer that wipes itself on drop.
struct Chars(Secret<Vec<char>>);

impl Chars {
    /// The buffer takes its whole size before the first character, since a
    /// text holds no more characters than bytes. So it never moves: a move
    /// would free a buffer that holds a prefix of the passphrase, unwiped
    /// (audit `W104`).
    fn of(passphrase: &Confidential<Passphrase>) -> Self {
        let text = passphrase.secret().expose();
        Self(Secret::filled(|chars: &mut Vec<char>| {
            chars.reserve_exact(text.len());
            chars.extend(text.chars());
        }))
    }

    /// The estimate of the text read as one run: its separators taken out,
    /// and its camel case read as one word. It counts when the rest is one
    /// class and reads as a pattern: a common password, a repetition or a
    /// walk. The alphabet spelled one letter at a time is such a pattern.
    /// `None` when the rest holds two classes, or reads as words.
    ///
    /// A separator costs its first choice. A phrase of words joined reads as
    /// fewer words than it holds.
    ///
    /// The reading also runs on a text with no separator. Without it, the
    /// split at each capital cuts a pattern into words. Capitals spread at
    /// random then lift a walk past the floor.
    fn joined(&self) -> Option<Estimate> {
        let text = self.0.expose();
        let separator = if text.iter().any(|&c| Class::of(c) == Class::Separator) {
            NEW_SEPARATOR
        } else {
            Count::ZERO
        };
        // Sized once, as in `Chars::of`, so no prefix is left behind.
        let kept = Self(Secret::filled(|kept: &mut Vec<char>| {
            kept.reserve_exact(text.len());
            kept.extend(
                text.iter()
                    .copied()
                    .filter(|&c| Class::of(c) != Class::Separator),
            );
        }));
        let chars = kept.0.expose();
        let class = Class::of(*chars.first()?);
        if chars.iter().any(|&c| Class::of(c) != class) {
            return None;
        }
        let run = Run {
            class,
            chars,
            standing: Standing::Alone,
        };
        let pattern = run.estimate();
        pattern
            .met
            .map(|_| pattern.plus(Estimate::plain(separator)))
    }

    /// `estimate`, held under the floor when the text is digits alone, its
    /// separators aside. Such a text is a PIN, a phone number or a run of
    /// dates, and a guess list walks those whole, whatever their length.
    fn capped(&self, estimate: Estimate) -> Estimate {
        let text = self.0.expose();
        let digits_alone = text.iter().any(char::is_ascii_digit)
            && text
                .iter()
                .all(|&c| matches!(Class::of(c), Class::Digit | Class::Separator));
        if digits_alone && estimate.bits >= FLOOR {
            Estimate {
                bits: Count::new(FLOOR.get().saturating_sub(1)),
                ..estimate
            }
        } else {
            estimate
        }
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
    ///
    /// The split walks `chunk_by`, which moves forward at each step whatever
    /// its test answers: no change of the test can loop.
    fn estimate(&self) -> Estimate {
        let text = self.0.expose();
        let class_at = |i: Option<usize>| i.and_then(|i| text.get(i)).map(|&c| Class::of(c));
        let apart = |class: Option<Class>| class.is_none_or(|class| class == Class::Separator);
        let one_run = |a: &char, b: &char| {
            Class::of(*a) == Class::of(*b) && !(a.is_ascii_lowercase() && b.is_ascii_uppercase())
        };
        let mut total = Estimate::NOTHING;
        let mut start = 0;
        for chars in text.chunk_by(one_run) {
            let end = start + chars.len();
            let class = chars.first().map_or(Class::Letter, |&c| Class::of(c));
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

/// What a run is, for its cost, before its pieces are read.
enum Shape {
    Common(Count<Draws>),
    SameChar,
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

    /// What the run is as a whole, in the order a guess tool tries the
    /// shapes:
    /// 1. a common password;
    /// 2. one character said again;
    /// 3. words;
    /// 4. random letters.
    ///
    /// A run reads as words when it holds a vowel. Only a run with no vowel
    /// reads as random letters. So a dictionary word or a keyboard walk never
    /// reads as random (audit `W100`). A run of one or two
    /// letters reads as a word when it stands alone. A letter outside ASCII
    /// counts as a vowel, so a script with no vowel reads as words: the
    /// cheaper reading.
    fn shape(&self) -> Shape {
        let len = self.chars.len();
        let spells = |word: &str| {
            len == word.len()
                && self
                    .chars
                    .iter()
                    .zip(word.bytes())
                    .all(|(c, b)| c.to_ascii_lowercase() == char::from(b))
        };
        let vowel = |c: &char| {
            !c.is_ascii() || matches!(c.to_ascii_lowercase(), 'a' | 'e' | 'i' | 'o' | 'u' | 'y')
        };
        let words = self.class == Class::Letter
            && if len < 3 {
                self.standing == Standing::Alone
            } else {
                self.chars.iter().any(vowel)
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
        } else if words {
            Shape::Words
        } else {
            Shape::Random
        }
    }

    /// The run as one unit said again, when it is one. It costs the unit, the
    /// log of the count, and one bit. The unit is the shortest that spells
    /// the run when said again, twice at least. A last unit cut short still
    /// counts as one.
    fn repeated(&self) -> Option<Estimate> {
        let len = self.chars.len();
        let period = (2..=len / 2).find(|&period| {
            self.chars
                .iter()
                .zip(self.chars.iter().skip(period))
                .all(|(a, b)| a.eq_ignore_ascii_case(b))
        })?;
        let unit = Run {
            class: self.class,
            chars: self.chars.get(..period)?,
            standing: self.standing,
        };
        let count = Count::<Draws>::of(len.div_ceil(period));
        Some(
            unit.estimate()
                .plus(Estimate::plain(log2(count).plus(ONE_BIT)))
                .with(PassphraseWeakness::Repetition),
        )
    }

    /// The cheapest reading of the run as a chain of pieces. A piece is a
    /// common password, a walk of three characters or more along a row of
    /// [`SEQUENCES`], or one character at `filler`. A guess tool chains
    /// patterns this way. So two keyboard rows joined cost the two walks and
    /// no more (audit `W100`). `first` is the cost of the character a walk
    /// starts on.
    ///
    /// The walk from each start is the longest along any row, as a ring and
    /// either way. Every shorter walk from that start follows the same row,
    /// so the chain tries each length up to it.
    fn segmented(&self, first: Count<Centibits>, filler: Count<Centibits>) -> Estimate {
        let chars = self.chars;
        let len = chars.len();
        let same = |c: char, b: u8| c.to_ascii_lowercase() == char::from(b);
        let rows: Vec<&[u8]> = SEQUENCES
            .split(' ')
            .map(str::as_bytes)
            .filter(|row| !row.is_empty())
            .collect();
        let walk_from = |start: usize| {
            let rest = chars.get(start..).unwrap_or_default();
            rows.iter()
                .flat_map(|row| {
                    let ring = row.len();
                    (0..ring)
                        .flat_map(|at| [Direction::Forward, Direction::Backward].map(|d| (at, d)))
                        .map(move |(at, direction)| {
                            rest.iter()
                                .enumerate()
                                .take_while(|&(step, &c)| {
                                    let i = match direction {
                                        Direction::Forward => (at + step) % ring,
                                        Direction::Backward => (at + ring - step % ring) % ring,
                                    };
                                    row.get(i).is_some_and(|&b| same(c, b))
                                })
                                .count()
                        })
                })
                .max()
                .unwrap_or(0)
        };
        let mut best: Vec<Option<Estimate>> = vec![None; len + 1];
        if let Some(origin) = best.first_mut() {
            *origin = Some(Estimate::NOTHING);
        }
        for start in 0..len {
            let Some(Some(so_far)) = best.get(start).copied() else {
                continue;
            };
            let mut offer = |end: usize, piece: Estimate| {
                if let Some(slot) = best.get_mut(end) {
                    let next = so_far.plus(piece);
                    if (*slot).is_none_or(|held| next.bits < held.bits) {
                        *slot = Some(next);
                    }
                }
            };
            offer(start + 1, Estimate::plain(filler));
            for walk in 3..=walk_from(start) {
                offer(start + walk, sequence(first, Count::of(walk)));
            }
            if self.class != Class::Letter {
                continue;
            }
            for (rank, word) in COMMON.split(' ').enumerate() {
                let hit = chars
                    .get(start..start + word.len())
                    .is_some_and(|piece| piece.iter().zip(word.bytes()).all(|(&c, b)| same(c, b)));
                if hit {
                    offer(
                        start + word.len(),
                        Estimate::plain(log2(Count::<Draws>::of(rank + 1)).plus(ONE_BIT))
                            .with(PassphraseWeakness::CommonPassword),
                    );
                }
            }
        }
        best.last().copied().flatten().unwrap_or(Estimate::NOTHING)
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

    /// A run of letters: a common password, one letter said again, a unit
    /// said again, or the lower of its whole reading and its pieces. Its case
    /// costs on top, except for a unit said again, whose unit carries it.
    fn letters(&self) -> Estimate {
        let len = self.len();
        let base = match self.shape() {
            Shape::Common(rank) => {
                Estimate::plain(log2(rank).plus(ONE_BIT)).with(PassphraseWeakness::CommonPassword)
            }
            Shape::SameChar => same_char(LETTER, len),
            shape => {
                if let Some(repeated) = self.repeated() {
                    return repeated;
                }
                let (whole, filler) = if matches!(shape, Shape::Words) {
                    let words = Count::<Characters>::new((len.get() / WORD_LEN.get()).max(1));
                    (WORD_CHAR.times(len).min(WORD.times(words)), WORD_CHAR)
                } else {
                    (LETTER.times(len), LETTER)
                };
                lower(Estimate::plain(whole), self.segmented(LETTER, filler))
            }
        };
        base.plus(Estimate::plain(self.case()))
    }

    /// A run of digits: one digit said again, or a unit said again. Else the
    /// lower of the run read whole and its pieces. Read whole, it is a year,
    /// a date or random digits.
    fn digits(&self) -> Estimate {
        let len = self.len();
        if matches!(self.shape(), Shape::SameChar) {
            return same_char(DIGIT, len);
        }
        if let Some(repeated) = self.repeated() {
            return repeated;
        }
        let first_two = (self.chars.first(), self.chars.get(1));
        let a_year = matches!(first_two, (Some('1'), Some('9')) | (Some('2'), Some('0')));
        let whole = if len.get() == 4 && a_year {
            YEAR
        } else if matches!(len.get(), 6 | 8) {
            DATE.min(DIGIT.times(len))
        } else {
            DIGIT.times(len)
        };
        lower(Estimate::plain(whole), self.segmented(DIGIT, DIGIT))
    }

    /// A run of symbols: one symbol said again, a unit said again, or
    /// random symbols. No row holds a symbol, so a run has no walk.
    fn symbols(&self) -> Estimate {
        let len = self.len();
        if matches!(self.shape(), Shape::SameChar) {
            return same_char(SYMBOL, len);
        }
        self.repeated()
            .unwrap_or(Estimate::plain(SYMBOL.times(len)))
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

/// A walk of `len` characters along a row of [`SEQUENCES`]: its first
/// character, the log of its length, and one bit for its direction.
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
            ("m0nk3yf00tb4ll", PassphraseWeakness::CommonPassword),
            ("", PassphraseWeakness::TooShort),
            // Audit `W100`: each input the estimator once admitted.
            ("christchurch", PassphraseWeakness::TooShort),
            ("qazwsxedcrfv", PassphraseWeakness::Sequence),
            ("qwertyasdfgh", PassphraseWeakness::CommonPassword),
            ("qwertyuiopasdfgh", PassphraseWeakness::TooShort),
            ("123456789012345", PassphraseWeakness::Sequence),
            ("123123123123123", PassphraseWeakness::Repetition),
            ("strengths12", PassphraseWeakness::TooShort),
            (
                "a b c d e f g h i j k l m n o p q r s",
                PassphraseWeakness::Sequence,
            ),
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
    #[cfg(not(target_arch = "wasm32"))]
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

    #[cfg(not(target_arch = "wasm32"))]
    fn phrase(picks: &[usize], separator: &str) -> String {
        picks
            .iter()
            .filter_map(|&i| DICEWARE.get(i % DICEWARE.len()).copied())
            .collect::<Vec<_>>()
            .join(separator)
    }

    #[cfg(not(target_arch = "wasm32"))]
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

    /// The estimate of `text` as typed, with no fold of its substitutions.
    fn typed(text: &str) -> Estimate {
        Chars::of(&pass(text)).estimate()
    }

    fn bits(text: &str) -> Count<Centibits> {
        typed(text).bits
    }

    #[test]
    fn a_substitution_folds_back_beside_a_letter_alone() {
        let folded =
            |text: &str| -> String { Chars::of(&pass(text)).folded().0.expose().iter().collect() };
        assert_eq!(folded("b0b1b3b4b@b5b$b7b2"), "bobibebababsbsbtb2");
        assert_eq!(folded("1337 2024 x 1 y"), "1337 2024 x 1 y");
    }

    #[test]
    fn a_common_password_costs_the_log_of_its_rank() {
        // `qwerty` is the second entry, `letmein` the fourth: one bit and two
        // bits of rank, and one bit of the choice to try the list.
        assert_eq!(bits("qwerty"), Count::new(200));
        assert_eq!(bits("letmein"), Count::new(300));
        assert_eq!(
            typed("qwerty").met,
            Some(PassphraseWeakness::CommonPassword)
        );
    }

    #[test]
    fn a_row_read_backward_is_a_sequence() {
        // log2(26) for the first letter, log2(10) for the length, one bit
        // for the direction.
        assert_eq!(bits("poiuytrewq"), Count::new(870));
        assert_eq!(typed("poiuytrewq").met, Some(PassphraseWeakness::Sequence));
    }

    #[test]
    fn a_short_run_reads_as_a_word_when_it_stands_alone() {
        // Two letters alone read as a word; three letters follow the vowel
        // rule, and `tqk` holds no vowel.
        assert_eq!(bits("xq"), Count::new(500));
        assert_eq!(bits("tqk"), Count::new(1_410));
    }

    /// Audit `W100`: a run that holds a vowel reads as words, however its
    /// consonants cluster. Only a run with no vowel reads as random letters.
    #[test]
    fn a_run_with_a_vowel_reads_as_words_and_one_without_as_random() {
        assert_eq!(bits("strngaeiou"), Count::new(1_300));
        assert_eq!(bits("strnaeiou"), Count::new(1_300));
        assert_eq!(bits("christchurch"), Count::new(1_300));
        assert_eq!(bits("qxzvbnwkrtplmhgj"), Count::new(6_410));
    }

    /// Audit `W100`: the estimator refuses each shape a guess tool builds
    /// first, whatever its length.
    /// * a keyboard walk across rows or down columns;
    /// * a row walked past its end;
    /// * a unit said again inside one run;
    /// * a dictionary word with an affix;
    /// * patterns chained in one run.
    #[test]
    fn each_shape_a_guess_tool_builds_first_is_refused() {
        let shapes = [
            // Keyboard walks: rows joined, columns, a layout other than QWERTY.
            "qwertyuiopasdfghjklzxcvbnm",
            "qazwsxedcrfvtgbyhnujm",
            "aqwzsxedcrfvtgbyhnuj",
            "mnbvcxzlkjhgfdsapoiuytrewq",
            // A row walked past its end, as a ring.
            "12345678901234567890",
            "abcdefghijklmnopqrstuvwxyzabcdefgh",
            // Repeats inside one run.
            "abcabcabcabcabcabc",
            "blueblueblueblueblue",
            "passwordpasswordpassword",
            "1212121212121212",
            "!@!@!@!@!@!@!@!@",
            // Dictionary words with affixes.
            "sunflower2024!",
            "Elephant1987$",
            "xxchristchurchxx",
            // Patterns chained in one run.
            "qwertypasswordasdfghdragon",
            "passwordqwertyuiopsunshine",
        ];
        for shape in shapes {
            assert!(refusal(shape).is_some(), "{shape}");
        }
    }

    /// Audit `W100`: a passphrase of digits alone stays under the floor,
    /// however many digits it holds, with or without separators.
    #[test]
    fn a_passphrase_of_digits_alone_stays_under_the_floor() {
        for digits in [
            "583920174652839",
            "58392017465283917364",
            "5839 2017 4652 8391 7364",
            "06-12-48-93-27-15-84",
        ] {
            assert_eq!(
                refusal(digits),
                Some(PassphraseWeakness::TooShort),
                "{digits}"
            );
        }
    }

    /// Audit `W104`: the buffer of the characters takes its whole size before
    /// the first one, so it never moves and leaves no freed prefix behind.
    /// A buffer that grew by reallocation would hold another capacity.
    #[test]
    fn the_character_buffer_is_sized_once_and_never_moves() {
        for text in [
            "anchovy glider ruckus tadpole",
            "x",
            "a passphrase of exactly forty-one chars!!",
        ] {
            let chars = Chars::of(&pass(text));
            assert_eq!(chars.0.expose().capacity(), text.len(), "{text}");
            assert_eq!(chars.0.expose().len(), text.chars().count(), "{text}");
        }
        // A text outside ASCII holds fewer characters than bytes, so the
        // buffer still never grows.
        let accented = "\u{e9}t\u{e9} \u{e0} la plage";
        let chars = Chars::of(&pass(accented));
        assert!(chars.0.expose().capacity() >= chars.0.expose().len());
        assert_eq!(chars.0.expose().capacity(), accented.len());
    }

    #[cfg(not(target_arch = "wasm32"))]
    proptest::proptest! {
        /// Audit `W100`: a chain of four fragments at most never reaches the
        /// floor. A fragment is a common password or a part of a row. A
        /// separator, or nothing, joins two fragments.
        #[test]
        fn no_chain_of_common_or_row_fragments_reaches_the_floor(
            picks in proptest::collection::vec((0_usize..1_000, 0_usize..1_000, 3_usize..12), 1..=4),
            separator in proptest::sample::select(vec!["", " ", "-", "_", "."]),
        ) {
            let common: Vec<&str> = COMMON.split(' ').collect();
            let rows: Vec<&str> = SEQUENCES.split(' ').collect();
            let fragments: Vec<String> = picks
                .iter()
                .map(|&(which, at, len)| {
                    if which % 2 == 0 {
                        common.get(at % common.len()).copied().unwrap_or("abc").to_owned()
                    } else {
                        let row = rows.get(at % rows.len()).copied().unwrap_or("abc");
                        let start = (at / rows.len()) % row.len();
                        row.chars().cycle().skip(start).take(len).collect()
                    }
                })
                .collect();
            let text = fragments.join(separator);
            proptest::prop_assert!(refusal(&text).is_some(), "{}", text);
        }
    }

    #[test]
    fn a_run_of_digits_reads_as_a_year_a_date_or_random_digits() {
        let cases = [
            ("1111", 532),
            ("5821", 1_328),
            ("1984", 720),
            ("2031", 720),
            ("58213", 1_660),
            ("582193", 1_520),
            ("58219374", 1_520),
            ("5821937", 2_324),
        ];
        for (text, centibits) in cases {
            assert_eq!(bits(text), Count::new(centibits), "{text}");
        }
        assert_eq!(typed("1111").met, Some(PassphraseWeakness::Repetition));
    }

    #[test]
    fn a_symbol_said_again_costs_the_log_of_its_count() {
        assert_eq!(bits("!!!!"), Count::new(704));
        assert_eq!(bits("!#%&"), Count::new(2_016));
        assert_eq!(typed("!!!!").met, Some(PassphraseWeakness::Repetition));
    }

    #[test]
    fn the_case_of_a_run_costs_one_bit_or_one_per_capital() {
        let case = |text: &str| {
            let chars: Vec<char> = text.chars().collect();
            Run {
                class: Class::Letter,
                chars: &chars,
                standing: Standing::Alone,
            }
            .case()
        };
        let cases = [
            ("abcd", 0),
            ("Abcd", 100),
            ("ABCD", 100),
            ("aBcd", 100),
            ("AbCd", 200),
            ("aBcD", 200),
        ];
        for (text, centibits) in cases {
            assert_eq!(case(text), Count::new(centibits), "{text}");
        }
    }

    #[test]
    fn a_capital_after_a_small_letter_starts_a_run() {
        // `ab` and `CD` are two runs, two letters each and embedded: random.
        // `ZXqk` is one run of letters, random, with a bit per capital.
        assert_eq!(bits("abCD"), Count::new(940 + 940 + 100));
        assert_eq!(bits("ZXqk"), Count::new(1_880 + 200));
    }

    #[test]
    fn a_tie_between_the_two_readings_keeps_the_text_as_typed() {
        // As typed, `11` is a digit said twice: 4.32 bits and a repetition.
        // Folded, `1istapie` costs one bit more than its 16.32 bits of runs,
        // and meets nothing. Both readings give 17.32 bits.
        let tie = "11stapie";
        assert_eq!(bits(tie), Count::new(1_732));
        assert_eq!(estimate(&pass(tie)).bits, Count::new(1_732));
        assert_eq!(refusal(tie), Some(PassphraseWeakness::Repetition));
    }

    /// `ZK-035`: a pattern with capitals spread inside it stays under the
    /// floor. The split at each capital no longer cuts it into words. The
    /// walk of the three rows costs its three walks, and each capital one
    /// bit: 25.1 and 9 bits. A text with no separator pays no separator.
    #[test]
    fn a_pattern_in_mixed_case_is_refused() {
        assert_eq!(
            estimate(&pass("QweRtyUioPasDfgHjkLzxCvbNm")).bits,
            Count::new(3_410)
        );
        for (text, weakness) in [
            ("QweRtyUioPasDfgHjkLzxCvbNm", PassphraseWeakness::Sequence),
            (
                "IloVeyOuSunsHinePrinCessFootBall",
                PassphraseWeakness::CommonPassword,
            ),
            (
                "PassWordQwerTyDraGonMonKeyLetMeIn",
                PassphraseWeakness::CommonPassword,
            ),
        ] {
            assert_eq!(refusal(text), Some(weakness), "{text}");
        }
    }

    /// `ZK-035`: a row read backward past its first character goes on from
    /// its last, as a ring. `cbazyxwvut` costs what `poiuytrewq` costs. A
    /// walk longer than its row goes round the ring again: `098765432109`
    /// costs the first digit, the log of 12 and one bit.
    #[test]
    fn a_row_read_backward_past_its_start_is_a_sequence() {
        assert_eq!(bits("cbazyxwvut"), Count::new(870));
        assert_eq!(typed("cbazyxwvut").met, Some(PassphraseWeakness::Sequence));
        assert_eq!(bits("098765432109"), Count::new(732));
        assert_eq!(
            typed("098765432109").met,
            Some(PassphraseWeakness::Sequence)
        );
    }

    /// `ZK-035`: each common password of a chain costs the log of its rank
    /// and one bit. `qwerty` is the second entry, `password` the first: two
    /// bits and one bit.
    #[test]
    fn each_common_password_of_a_chain_costs_the_log_of_its_rank() {
        assert_eq!(bits("qwertypassword"), Count::new(300));
    }

    /// `ZK-036`: two chains of one cost keep the chain found first. The walk
    /// `qwertyuio` and `qwerty` then `uio` both cost 8.7 bits. The walk comes
    /// first, so a refusal names a sequence.
    #[test]
    fn a_tie_between_two_chains_keeps_the_chain_found_first() {
        assert_eq!(bits("qwertyuio"), Count::new(870));
        assert_eq!(typed("qwertyuio").met, Some(PassphraseWeakness::Sequence));
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
