//! The unit a `math` line may not name (`ARCH-084`).
//!
//! The category of a line is a claim a reviewer holds (`ARCH-035`). The key
//! alone refutes the claim `math` when the name ends on a unit. The value
//! is a quantity in that unit, so it takes a type. A line costs less than a
//! type. Without this check, one line hides a unit quantity until a reader
//! catches it.
//!
//! The check reads the last segment of the name and nothing else. A unit
//! before the last segment qualifies another word, as in `bits_per_pixel`, a
//! ratio. A name that hides its unit elsewhere passes, and the reviewer holds
//! it.

use std::fmt;

/// A unit a name may not end on. Each one is a time span, a rate, a size or
/// a frequency, in a spelling a contributor writes. `seconds`, `minutes`
/// and `hours` stay out: they also name the parts of a clock reading, which
/// are no span. A length unit such as `px`, `pt` or `mm` stays out: a
/// conversion factor ends on one, as `PX_PER_PT` does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Ms,
    Us,
    Ns,
    Sec,
    Secs,
    Millis,
    Micros,
    Nanos,
    Bps,
    Kbps,
    Mbps,
    Bits,
    Bytes,
    Hz,
    Khz,
}

impl Unit {
    /// Every unit, in the order `ARCH-084` lists them.
    pub const ALL: [Self; 15] = [
        Self::Ms,
        Self::Us,
        Self::Ns,
        Self::Sec,
        Self::Secs,
        Self::Millis,
        Self::Micros,
        Self::Nanos,
        Self::Bps,
        Self::Kbps,
        Self::Mbps,
        Self::Bits,
        Self::Bytes,
        Self::Hz,
        Self::Khz,
    ];

    /// The word a name spells the unit with, lowercase.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ms => "ms",
            Self::Us => "us",
            Self::Ns => "ns",
            Self::Sec => "sec",
            Self::Secs => "secs",
            Self::Millis => "millis",
            Self::Micros => "micros",
            Self::Nanos => "nanos",
            Self::Bps => "bps",
            Self::Kbps => "kbps",
            Self::Mbps => "mbps",
            Self::Bits => "bits",
            Self::Bytes => "bytes",
            Self::Hz => "hz",
            Self::Khz => "khz",
        }
    }
}

impl fmt::Display for Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The unit that the name of the item path `item` ends on, if any.
///
/// A `param` site names its parameter, and a `ret` site names its `fn`. Any
/// other site names the last segment of its path: a field, a constant.
pub fn unit_of(item: &str) -> Option<Unit> {
    let last = segments(name(item)).pop()?;
    Unit::ALL
        .into_iter()
        .find(|unit| unit.as_str().eq_ignore_ascii_case(&last))
}

/// The name an item path gives its value. `Retry::wait(delay_ms)` names
/// `delay_ms`, `Retry::delay_ms()` names `delay_ms`, `Retry.delay_ms` names
/// `delay_ms`.
fn name(item: &str) -> &str {
    let path = match item
        .strip_suffix(')')
        .and_then(|head| head.rsplit_once('('))
    {
        Some((_, param)) if !param.is_empty() => return param,
        Some((path, _)) => path,
        None => item,
    };
    path.rsplit(['.', ':']).next().unwrap_or(path)
}

/// The words of a name. A word ends at each of four places:
///
/// * an `_`;
/// * a change from lower to upper case;
/// * the end of an uppercase run before a capitalised word;
/// * a change between a letter and a digit.
///
/// So `delayMs`, `DELAY_MS` and `delay_10ms` all end on `ms`.
fn segments(name: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let chars: Vec<char> = name.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        let prev = i.checked_sub(1).and_then(|j| chars.get(j)).copied();
        let next = chars.get(i + 1).copied();
        let split = match prev {
            None => false,
            Some(p) => {
                (p.is_lowercase() && c.is_uppercase())
                    || (p.is_uppercase()
                        && c.is_uppercase()
                        && next.is_some_and(char::is_lowercase))
                    || (p.is_ascii_digit() != c.is_ascii_digit())
            }
        };
        if c == '_' || split {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            if c == '_' {
                continue;
            }
        }
        word.push(c);
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

#[cfg(test)]
#[path = "units_tests.rs"]
mod tests;
