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

/// The units a name may not end on, lowercase. A time span, a rate, a size
/// and a frequency, each with the spellings a contributor writes for it.
/// `seconds`, `minutes` and `hours` stay out: they also name the parts of a
/// clock reading, which are no span. A length unit such as `px`, `pt` or `mm`
/// stays out: a conversion factor ends on one, as `PX_PER_PT` does.
pub const UNITS: [&str; 15] = [
    "ms", "us", "ns", "sec", "secs", "millis", "micros", "nanos", "bps", "kbps", "mbps", "bits",
    "bytes", "hz", "khz",
];

/// The unit that the name of the item path `item` ends on, if any.
///
/// A `param` site names its parameter, and a `ret` site names its `fn`. Any
/// other site names the last segment of its path: a field, a constant.
pub fn unit_of(item: &str) -> Option<&'static str> {
    let last = segments(name(item)).pop()?;
    UNITS
        .into_iter()
        .find(|unit| unit.eq_ignore_ascii_case(&last))
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
