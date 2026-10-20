//! The emoji the picker offers
//! (`specs/5-Murmure/features/ambre-spec-murmure-emoji.md`).
//!
//! A table compiled into the client, searched by name. Nothing is fetched to
//! show it or to search it, so the picker sends no request outside the client
//! (`MURMURE-141`). Each emoji is one grapheme, which a reaction takes as its
//! key (`MURMURE-092`). The source spells each one by its code points, since
//! the tree is printable ASCII.

/// Every emoji the picker offers, with its name, in the order it shows them.
pub const EMOJI: &[(&str, &str)] = &[
    ("\u{1F44D}", "thumbs up"),
    ("\u{1F44E}", "thumbs down"),
    ("\u{2764}\u{FE0F}", "red heart"),
    ("\u{1F602}", "face with tears of joy"),
    ("\u{1F389}", "party popper"),
    ("\u{1F440}", "eyes"),
    ("\u{1F64F}", "folded hands"),
    ("\u{2705}", "check mark button"),
    ("\u{1F525}", "fire"),
    ("\u{1F680}", "rocket"),
    ("\u{1F600}", "grinning face"),
    ("\u{1F603}", "grinning face with big eyes"),
    ("\u{1F604}", "grinning face with smiling eyes"),
    ("\u{1F601}", "beaming face with smiling eyes"),
    ("\u{1F606}", "grinning squinting face"),
    ("\u{1F605}", "grinning face with sweat"),
    ("\u{1F923}", "rolling on the floor laughing"),
    ("\u{1F642}", "slightly smiling face"),
    ("\u{1F643}", "upside down face"),
    ("\u{1F609}", "winking face"),
    ("\u{1F60A}", "smiling face with smiling eyes"),
    ("\u{1F607}", "smiling face with halo"),
    ("\u{1F970}", "smiling face with hearts"),
    ("\u{1F60D}", "smiling face with heart eyes"),
    ("\u{1F929}", "star struck"),
    ("\u{1F618}", "face blowing a kiss"),
    ("\u{1F60B}", "face savoring food"),
    ("\u{1F61B}", "face with tongue"),
    ("\u{1F61C}", "winking face with tongue"),
    ("\u{1F92A}", "zany face"),
    ("\u{1F917}", "smiling face with open hands"),
    ("\u{1F914}", "thinking face"),
    ("\u{1F92B}", "shushing face"),
    ("\u{1F910}", "zipper mouth face"),
    ("\u{1F610}", "neutral face"),
    ("\u{1F611}", "expressionless face"),
    ("\u{1F636}", "face without mouth"),
    ("\u{1F60F}", "smirking face"),
    ("\u{1F612}", "unamused face"),
    ("\u{1F644}", "face with rolling eyes"),
    ("\u{1F62C}", "grimacing face"),
    ("\u{1F60C}", "relieved face"),
    ("\u{1F614}", "pensive face"),
    ("\u{1F62A}", "sleepy face"),
    ("\u{1F634}", "sleeping face"),
    ("\u{1F637}", "face with medical mask"),
    ("\u{1F912}", "face with thermometer"),
    ("\u{1F92F}", "exploding head"),
    ("\u{1F973}", "partying face"),
    ("\u{1F60E}", "smiling face with sunglasses"),
    ("\u{1F913}", "nerd face"),
    ("\u{1F615}", "confused face"),
    ("\u{1F61F}", "worried face"),
    ("\u{1F62E}", "face with open mouth"),
    ("\u{1F632}", "astonished face"),
    ("\u{1F633}", "flushed face"),
    ("\u{1F97A}", "pleading face"),
    ("\u{1F622}", "crying face"),
    ("\u{1F62D}", "loudly crying face"),
    ("\u{1F631}", "face screaming in fear"),
    ("\u{1F624}", "face with steam from nose"),
    ("\u{1F621}", "enraged face"),
    ("\u{1F620}", "angry face"),
    ("\u{1F480}", "skull"),
    ("\u{1F4A9}", "pile of poo"),
    ("\u{1F921}", "clown face"),
    ("\u{1F47B}", "ghost"),
    ("\u{1F47D}", "alien"),
    ("\u{1F916}", "robot"),
    ("\u{1F63A}", "grinning cat"),
    ("\u{1F44B}", "waving hand"),
    ("\u{270B}", "raised hand"),
    ("\u{1F44C}", "ok hand"),
    ("\u{270C}\u{FE0F}", "victory hand"),
    ("\u{1F91E}", "crossed fingers"),
    ("\u{1F91D}", "handshake"),
    ("\u{1F44F}", "clapping hands"),
    ("\u{1F64C}", "raising hands"),
    ("\u{1F4AA}", "flexed biceps"),
    ("\u{1F449}", "backhand index pointing right"),
    ("\u{1F448}", "backhand index pointing left"),
    ("\u{1F446}", "backhand index pointing up"),
    ("\u{1F447}", "backhand index pointing down"),
    ("\u{1FAE1}", "saluting face"),
    ("\u{1F9E1}", "orange heart"),
    ("\u{1F49B}", "yellow heart"),
    ("\u{1F49A}", "green heart"),
    ("\u{1F499}", "blue heart"),
    ("\u{1F49C}", "purple heart"),
    ("\u{1F5A4}", "black heart"),
    ("\u{1F494}", "broken heart"),
    ("\u{1F4AF}", "hundred points"),
    ("\u{2728}", "sparkles"),
    ("\u{2B50}", "star"),
    ("\u{1F31F}", "glowing star"),
    ("\u{26A1}", "high voltage"),
    ("\u{1F4A1}", "light bulb"),
    ("\u{1F4CC}", "pushpin"),
    ("\u{1F4CE}", "paperclip"),
    ("\u{1F4DD}", "memo"),
    ("\u{1F4C5}", "calendar"),
    ("\u{23F0}", "alarm clock"),
    ("\u{23F3}", "hourglass not done"),
    ("\u{1F512}", "locked"),
    ("\u{1F511}", "key"),
    ("\u{1F41B}", "bug"),
    ("\u{1F6E0}\u{FE0F}", "hammer and wrench"),
    ("\u{2699}\u{FE0F}", "gear"),
    ("\u{1F4E6}", "package"),
    ("\u{1F4BB}", "laptop"),
    ("\u{1F4F1}", "mobile phone"),
    ("\u{2615}", "hot beverage"),
    ("\u{1F355}", "pizza"),
    ("\u{1F370}", "shortcake"),
    ("\u{1F37A}", "beer mug"),
    ("\u{1F382}", "birthday cake"),
    ("\u{1F381}", "wrapped gift"),
    ("\u{1F3C6}", "trophy"),
    ("\u{1F3AF}", "direct hit"),
    ("\u{1F308}", "rainbow"),
    ("\u{2600}\u{FE0F}", "sun"),
    ("\u{1F319}", "crescent moon"),
    ("\u{2744}\u{FE0F}", "snowflake"),
    ("\u{1F331}", "seedling"),
    ("\u{1F338}", "cherry blossom"),
    ("\u{1F436}", "dog face"),
    ("\u{1F431}", "cat face"),
    ("\u{1F98A}", "fox"),
    ("\u{1F43C}", "panda"),
    ("\u{1F984}", "unicorn"),
    ("\u{1F422}", "turtle"),
    ("\u{274C}", "cross mark"),
    ("\u{26A0}\u{FE0F}", "warning"),
    ("\u{2753}", "red question mark"),
    ("\u{2757}", "red exclamation mark"),
    ("\u{2795}", "plus"),
    ("\u{2796}", "minus"),
    ("\u{1F197}", "ok button"),
    ("\u{1F534}", "red circle"),
    ("\u{1F7E2}", "green circle"),
    ("\u{1F535}", "blue circle"),
];

/// The emoji whose name holds every word of `query`, in table order. A query
/// with no word offers the whole table. The search is the table's alone, so it
/// asks nothing of anyone (`MURMURE-141`).
pub fn search(query: &str) -> Vec<&'static str> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    EMOJI
        .iter()
        .filter(|(_, name)| words.iter().all(|word| name.contains(word.as_str())))
        .map(|&(emoji, _)| emoji)
        .collect()
}

/// The name of `emoji`, which a screen reader hears for its button. `None`
/// for an emoji the table does not hold.
pub fn name_of(emoji: &str) -> Option<&'static str> {
    EMOJI
        .iter()
        .find(|&&(e, _)| e == emoji)
        .map(|&(_, name)| name)
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
        for &(emoji, name) in EMOJI {
            assert!(crate::reaction::is_emoji(emoji), "{emoji} ({name})");
            assert!(seen.insert(emoji), "{emoji} repeats");
            assert_eq!(name, name.to_lowercase(), "{name}");
        }
    }

    /// A query keeps the emoji whose name holds each of its words, whatever
    /// their case, and a blank one keeps them all.
    #[test]
    fn a_query_keeps_the_emoji_whose_name_holds_its_words() {
        assert_eq!(search("  ").len(), EMOJI.len());
        assert_eq!(search("Thumbs"), ["\u{1F44D}", "\u{1F44E}"]);
        assert_eq!(search("heart broken"), ["\u{1F494}"]);
        assert!(search("no such emoji").is_empty());
        assert_eq!(name_of("\u{1F680}"), Some("rocket"));
        assert_eq!(name_of("a"), None);
    }
}
