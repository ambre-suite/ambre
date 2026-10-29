//! The emoji the picker offers
//! (`specs/5-Murmure/features/ambre-spec-murmure-emoji.md`).
//!
//! A table compiled into the client, searched by name. Nothing is fetched to
//! show it or to search it, so the picker sends no request outside the client
//! (`MURMURE-141`). Each emoji is one grapheme, which a reaction takes as its
//! key (`MURMURE-092`). The source spells each one by its code points, since
//! the tree is printable ASCII.

use ambre_bus::Emoji;

/// One emoji the picker offers, with the name a screen reader hears for its
/// button and a search matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmojiEntry {
    pub emoji: Emoji,
    pub name: &'static str,
}

/// The entry of `emoji`, named `name`.
const fn entry(emoji: Emoji, name: &'static str) -> EmojiEntry {
    EmojiEntry { emoji, name }
}

/// Every emoji the picker offers, with its name, in the order it shows them.
pub const EMOJI: &[EmojiEntry] = &[
    entry(Emoji::from_static("\u{1F44D}"), "thumbs up"),
    entry(Emoji::from_static("\u{1F44E}"), "thumbs down"),
    entry(Emoji::from_static("\u{2764}\u{FE0F}"), "red heart"),
    entry(Emoji::from_static("\u{1F602}"), "face with tears of joy"),
    entry(Emoji::from_static("\u{1F389}"), "party popper"),
    entry(Emoji::from_static("\u{1F440}"), "eyes"),
    entry(Emoji::from_static("\u{1F64F}"), "folded hands"),
    entry(Emoji::from_static("\u{2705}"), "check mark button"),
    entry(Emoji::from_static("\u{1F525}"), "fire"),
    entry(Emoji::from_static("\u{1F680}"), "rocket"),
    entry(Emoji::from_static("\u{1F600}"), "grinning face"),
    entry(
        Emoji::from_static("\u{1F603}"),
        "grinning face with big eyes",
    ),
    entry(
        Emoji::from_static("\u{1F604}"),
        "grinning face with smiling eyes",
    ),
    entry(
        Emoji::from_static("\u{1F601}"),
        "beaming face with smiling eyes",
    ),
    entry(Emoji::from_static("\u{1F606}"), "grinning squinting face"),
    entry(Emoji::from_static("\u{1F605}"), "grinning face with sweat"),
    entry(
        Emoji::from_static("\u{1F923}"),
        "rolling on the floor laughing",
    ),
    entry(Emoji::from_static("\u{1F642}"), "slightly smiling face"),
    entry(Emoji::from_static("\u{1F643}"), "upside down face"),
    entry(Emoji::from_static("\u{1F609}"), "winking face"),
    entry(
        Emoji::from_static("\u{1F60A}"),
        "smiling face with smiling eyes",
    ),
    entry(Emoji::from_static("\u{1F607}"), "smiling face with halo"),
    entry(Emoji::from_static("\u{1F970}"), "smiling face with hearts"),
    entry(
        Emoji::from_static("\u{1F60D}"),
        "smiling face with heart eyes",
    ),
    entry(Emoji::from_static("\u{1F929}"), "star struck"),
    entry(Emoji::from_static("\u{1F618}"), "face blowing a kiss"),
    entry(Emoji::from_static("\u{1F60B}"), "face savoring food"),
    entry(Emoji::from_static("\u{1F61B}"), "face with tongue"),
    entry(Emoji::from_static("\u{1F61C}"), "winking face with tongue"),
    entry(Emoji::from_static("\u{1F92A}"), "zany face"),
    entry(
        Emoji::from_static("\u{1F917}"),
        "smiling face with open hands",
    ),
    entry(Emoji::from_static("\u{1F914}"), "thinking face"),
    entry(Emoji::from_static("\u{1F92B}"), "shushing face"),
    entry(Emoji::from_static("\u{1F910}"), "zipper mouth face"),
    entry(Emoji::from_static("\u{1F610}"), "neutral face"),
    entry(Emoji::from_static("\u{1F611}"), "expressionless face"),
    entry(Emoji::from_static("\u{1F636}"), "face without mouth"),
    entry(Emoji::from_static("\u{1F60F}"), "smirking face"),
    entry(Emoji::from_static("\u{1F612}"), "unamused face"),
    entry(Emoji::from_static("\u{1F644}"), "face with rolling eyes"),
    entry(Emoji::from_static("\u{1F62C}"), "grimacing face"),
    entry(Emoji::from_static("\u{1F60C}"), "relieved face"),
    entry(Emoji::from_static("\u{1F614}"), "pensive face"),
    entry(Emoji::from_static("\u{1F62A}"), "sleepy face"),
    entry(Emoji::from_static("\u{1F634}"), "sleeping face"),
    entry(Emoji::from_static("\u{1F637}"), "face with medical mask"),
    entry(Emoji::from_static("\u{1F912}"), "face with thermometer"),
    entry(Emoji::from_static("\u{1F92F}"), "exploding head"),
    entry(Emoji::from_static("\u{1F973}"), "partying face"),
    entry(
        Emoji::from_static("\u{1F60E}"),
        "smiling face with sunglasses",
    ),
    entry(Emoji::from_static("\u{1F913}"), "nerd face"),
    entry(Emoji::from_static("\u{1F615}"), "confused face"),
    entry(Emoji::from_static("\u{1F61F}"), "worried face"),
    entry(Emoji::from_static("\u{1F62E}"), "face with open mouth"),
    entry(Emoji::from_static("\u{1F632}"), "astonished face"),
    entry(Emoji::from_static("\u{1F633}"), "flushed face"),
    entry(Emoji::from_static("\u{1F97A}"), "pleading face"),
    entry(Emoji::from_static("\u{1F622}"), "crying face"),
    entry(Emoji::from_static("\u{1F62D}"), "loudly crying face"),
    entry(Emoji::from_static("\u{1F631}"), "face screaming in fear"),
    entry(Emoji::from_static("\u{1F624}"), "face with steam from nose"),
    entry(Emoji::from_static("\u{1F621}"), "enraged face"),
    entry(Emoji::from_static("\u{1F620}"), "angry face"),
    entry(Emoji::from_static("\u{1F480}"), "skull"),
    entry(Emoji::from_static("\u{1F4A9}"), "pile of poo"),
    entry(Emoji::from_static("\u{1F921}"), "clown face"),
    entry(Emoji::from_static("\u{1F47B}"), "ghost"),
    entry(Emoji::from_static("\u{1F47D}"), "alien"),
    entry(Emoji::from_static("\u{1F916}"), "robot"),
    entry(Emoji::from_static("\u{1F63A}"), "grinning cat"),
    entry(Emoji::from_static("\u{1F44B}"), "waving hand"),
    entry(Emoji::from_static("\u{270B}"), "raised hand"),
    entry(Emoji::from_static("\u{1F44C}"), "ok hand"),
    entry(Emoji::from_static("\u{270C}\u{FE0F}"), "victory hand"),
    entry(Emoji::from_static("\u{1F91E}"), "crossed fingers"),
    entry(Emoji::from_static("\u{1F91D}"), "handshake"),
    entry(Emoji::from_static("\u{1F44F}"), "clapping hands"),
    entry(Emoji::from_static("\u{1F64C}"), "raising hands"),
    entry(Emoji::from_static("\u{1F4AA}"), "flexed biceps"),
    entry(
        Emoji::from_static("\u{1F449}"),
        "backhand index pointing right",
    ),
    entry(
        Emoji::from_static("\u{1F448}"),
        "backhand index pointing left",
    ),
    entry(
        Emoji::from_static("\u{1F446}"),
        "backhand index pointing up",
    ),
    entry(
        Emoji::from_static("\u{1F447}"),
        "backhand index pointing down",
    ),
    entry(Emoji::from_static("\u{1FAE1}"), "saluting face"),
    entry(Emoji::from_static("\u{1F9E1}"), "orange heart"),
    entry(Emoji::from_static("\u{1F49B}"), "yellow heart"),
    entry(Emoji::from_static("\u{1F49A}"), "green heart"),
    entry(Emoji::from_static("\u{1F499}"), "blue heart"),
    entry(Emoji::from_static("\u{1F49C}"), "purple heart"),
    entry(Emoji::from_static("\u{1F5A4}"), "black heart"),
    entry(Emoji::from_static("\u{1F494}"), "broken heart"),
    entry(Emoji::from_static("\u{1F4AF}"), "hundred points"),
    entry(Emoji::from_static("\u{2728}"), "sparkles"),
    entry(Emoji::from_static("\u{2B50}"), "star"),
    entry(Emoji::from_static("\u{1F31F}"), "glowing star"),
    entry(Emoji::from_static("\u{26A1}"), "high voltage"),
    entry(Emoji::from_static("\u{1F4A1}"), "light bulb"),
    entry(Emoji::from_static("\u{1F4CC}"), "pushpin"),
    entry(Emoji::from_static("\u{1F4CE}"), "paperclip"),
    entry(Emoji::from_static("\u{1F4DD}"), "memo"),
    entry(Emoji::from_static("\u{1F4C5}"), "calendar"),
    entry(Emoji::from_static("\u{23F0}"), "alarm clock"),
    entry(Emoji::from_static("\u{23F3}"), "hourglass not done"),
    entry(Emoji::from_static("\u{1F512}"), "locked"),
    entry(Emoji::from_static("\u{1F511}"), "key"),
    entry(Emoji::from_static("\u{1F41B}"), "bug"),
    entry(Emoji::from_static("\u{1F6E0}\u{FE0F}"), "hammer and wrench"),
    entry(Emoji::from_static("\u{2699}\u{FE0F}"), "gear"),
    entry(Emoji::from_static("\u{1F4E6}"), "package"),
    entry(Emoji::from_static("\u{1F4BB}"), "laptop"),
    entry(Emoji::from_static("\u{1F4F1}"), "mobile phone"),
    entry(Emoji::from_static("\u{2615}"), "hot beverage"),
    entry(Emoji::from_static("\u{1F355}"), "pizza"),
    entry(Emoji::from_static("\u{1F370}"), "shortcake"),
    entry(Emoji::from_static("\u{1F37A}"), "beer mug"),
    entry(Emoji::from_static("\u{1F382}"), "birthday cake"),
    entry(Emoji::from_static("\u{1F381}"), "wrapped gift"),
    entry(Emoji::from_static("\u{1F3C6}"), "trophy"),
    entry(Emoji::from_static("\u{1F3AF}"), "direct hit"),
    entry(Emoji::from_static("\u{1F308}"), "rainbow"),
    entry(Emoji::from_static("\u{2600}\u{FE0F}"), "sun"),
    entry(Emoji::from_static("\u{1F319}"), "crescent moon"),
    entry(Emoji::from_static("\u{2744}\u{FE0F}"), "snowflake"),
    entry(Emoji::from_static("\u{1F331}"), "seedling"),
    entry(Emoji::from_static("\u{1F338}"), "cherry blossom"),
    entry(Emoji::from_static("\u{1F436}"), "dog face"),
    entry(Emoji::from_static("\u{1F431}"), "cat face"),
    entry(Emoji::from_static("\u{1F98A}"), "fox"),
    entry(Emoji::from_static("\u{1F43C}"), "panda"),
    entry(Emoji::from_static("\u{1F984}"), "unicorn"),
    entry(Emoji::from_static("\u{1F422}"), "turtle"),
    entry(Emoji::from_static("\u{274C}"), "cross mark"),
    entry(Emoji::from_static("\u{26A0}\u{FE0F}"), "warning"),
    entry(Emoji::from_static("\u{2753}"), "red question mark"),
    entry(Emoji::from_static("\u{2757}"), "red exclamation mark"),
    entry(Emoji::from_static("\u{2795}"), "plus"),
    entry(Emoji::from_static("\u{2796}"), "minus"),
    entry(Emoji::from_static("\u{1F197}"), "ok button"),
    entry(Emoji::from_static("\u{1F534}"), "red circle"),
    entry(Emoji::from_static("\u{1F7E2}"), "green circle"),
    entry(Emoji::from_static("\u{1F535}"), "blue circle"),
];

/// The emoji whose name holds every word of `query`, in table order. A query
/// with no word offers the whole table. The search is the table's alone, so it
/// asks nothing of anyone (`MURMURE-141`).
pub fn search(query: &str) -> Vec<&'static EmojiEntry> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    EMOJI
        .iter()
        .filter(|entry| words.iter().all(|word| entry.name.contains(word.as_str())))
        .collect()
}

/// The name of `emoji`, which a screen reader hears for its button. `None`
/// for an emoji the table does not hold.
pub fn name_of(emoji: &Emoji) -> Option<&'static str> {
    EMOJI
        .iter()
        .find(|entry| entry.emoji == *emoji)
        .map(|entry| entry.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every emoji of the table is one a reaction takes, and no two entries
    /// repeat one. The table is compiled in, so the picker asks nothing of the
    /// network (`MURMURE-141`).
    #[test]
    fn every_emoji_of_the_table_is_one_reaction_key() {
        let mut seen = std::collections::HashSet::new();
        for EmojiEntry { emoji, name } in EMOJI {
            let glyph = emoji.as_str();
            assert!(crate::reaction::is_emoji(emoji), "{glyph} ({name})");
            assert!(seen.insert(glyph), "{glyph} repeats");
            assert_eq!(*name, name.to_lowercase(), "{name}");
        }
    }

    /// A query keeps the emoji whose name holds each of its words, whatever
    /// their case, and a blank one keeps them all.
    #[test]
    fn a_query_keeps_the_emoji_whose_name_holds_its_words() {
        assert_eq!(search("  ").len(), EMOJI.len());
        let glyphs =
            |query| -> Vec<&str> { search(query).iter().map(|e| e.emoji.as_str()).collect() };
        assert_eq!(glyphs("Thumbs"), ["\u{1F44D}", "\u{1F44E}"]);
        assert_eq!(glyphs("heart broken"), ["\u{1F494}"]);
        assert!(search("no such emoji").is_empty());
        assert_eq!(name_of(&Emoji::from("\u{1F680}")), Some("rocket"));
        assert_eq!(name_of(&Emoji::from("a")), None);
    }
}
