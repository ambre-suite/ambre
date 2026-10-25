//! The file an export hands the browser: its name and its bytes.
//!
//! An export name derives from a document title or a sheet name, so it is
//! user input. The browser saves the download under that name. On a host
//! that honours it, a separator or a `..` names a place outside the download
//! folder. So the name is a validated type (`DOCFILE-019`, `DOCFILE-020`).

use crate::media::ByteSize;

/// The extension an export file carries, one per format the suite writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExportExtension {
    /// The native file, plaintext or sealed.
    Ambre,
    /// A sheet as comma-separated values.
    Csv,
    /// A text document as Markdown.
    Markdown,
    /// A deck as a PDF.
    Pdf,
    /// A board as an SVG document.
    Svg,
}

impl ExportExtension {
    /// The extension as the file name spells it, without its dot.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ambre => "ambre",
            Self::Csv => "csv",
            Self::Markdown => "md",
            Self::Pdf => "pdf",
            Self::Svg => "svg",
        }
    }
}

/// Why a text is no export file name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ExportFilenameError {
    /// The name is empty.
    #[error("an export file name is empty")]
    Empty,
    /// The name is `.` or `..`, which name a folder and never a file.
    #[error("an export file name is a folder reference")]
    FolderReference,
    /// The name holds a `/` or a `\`, which a host reads as a folder step.
    #[error("an export file name holds a path separator")]
    PathSeparator,
    /// The name holds a control character.
    #[error("an export file name holds a control character")]
    ControlCharacter,
    /// The name holds an invisible format character, such as a bidirectional
    /// override that shows `exe.pdf` for a name that ends in `.exe`.
    #[error("an export file name holds a format character")]
    FormatCharacter,
    /// The name is longer than [`ExportFilename::MAX_SIZE`].
    #[error("an export file name is too long")]
    TooLong,
}

/// The name an export file is saved under. It is never empty, never `.` or
/// `..`, and holds no path separator, no control character and no format
/// character (`DOCFILE-019`). It holds at most [`MAX_SIZE`](Self::MAX_SIZE) bytes
/// (`DOCFILE-020`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ExportFilename(String);

impl ExportFilename {
    /// The most bytes a name holds, extension included: the limit of most
    /// file systems.
    pub const MAX_SIZE: ByteSize = ByteSize(255);

    /// The stem an export takes when its title leaves nothing to name it by.
    const FALLBACK_STEM: &'static str = "export";

    /// The name `text` spells, or the first rule it breaks.
    ///
    /// # Errors
    ///
    /// Any [`ExportFilenameError`]: the name is refused whole, never repaired.
    pub fn new(text: &str) -> Result<Self, ExportFilenameError> {
        if text.is_empty() {
            return Err(ExportFilenameError::Empty);
        }
        if matches!(text, "." | "..") {
            return Err(ExportFilenameError::FolderReference);
        }
        if text.contains(['/', '\\']) {
            return Err(ExportFilenameError::PathSeparator);
        }
        if text.chars().any(char::is_control) {
            return Err(ExportFilenameError::ControlCharacter);
        }
        if text.chars().any(is_format_character) {
            return Err(ExportFilenameError::FormatCharacter);
        }
        if ByteSize::of_len(text.len()) > Self::MAX_SIZE {
            return Err(ExportFilenameError::TooLong);
        }
        Ok(Self(text.to_owned()))
    }

    /// The name of a file the suite names itself, such as `board.svg`.
    #[must_use]
    pub fn named(stem: &'static str, extension: ExportExtension) -> Self {
        Self::from_title(stem, Self::FALLBACK_STEM, extension)
    }

    /// The name an export of a document titled `title` takes.
    ///
    /// The title loses each character a host refuses in a name, then the
    /// spaces around it. A stem cut to the length bound keeps the extension
    /// whole. A title that leaves nothing takes the `fallback` stem.
    #[must_use]
    pub fn from_title(title: &str, fallback: &'static str, extension: ExportExtension) -> Self {
        let suffix = extension.as_str();
        // The stem's room: the bound, less the dot and the extension.
        let room = Self::MAX_SIZE.as_len().saturating_sub(suffix.len() + 1);
        let stem = [title, fallback, Self::FALLBACK_STEM]
            .into_iter()
            .map(|text| clean_stem(text, room))
            .find(|stem| !stem.is_empty())
            .unwrap_or_else(|| Self::FALLBACK_STEM.to_owned());
        Self(format!("{stem}.{suffix}"))
    }

    /// The longest extension a received name keeps.
    const MAX_EXTENSION: ByteSize = ByteSize(16);

    /// The name a file another user sent saves under, from the `name` its
    /// sender gave it.
    ///
    /// The name is untrusted input, so it obeys the rules of
    /// [`from_title`](Self::from_title). A path keeps its last step alone. An
    /// extension of letters and digits survives whole; any other stays in the
    /// stem. A name that leaves nothing takes the `fallback` stem.
    #[must_use]
    pub fn from_received(name: &str, fallback: &'static str) -> Self {
        let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
        let (stem, extension) = match base.rsplit_once('.') {
            Some((stem, extension)) if is_kept_extension(extension) => (stem, Some(extension)),
            _ => (base, None),
        };
        // The stem's room: the bound, less the dot and the extension.
        let room = Self::MAX_SIZE
            .as_len()
            .saturating_sub(extension.map_or(0, |extension| extension.len() + 1));
        let stem = [stem, fallback, Self::FALLBACK_STEM]
            .into_iter()
            .map(|text| clean_stem(text, room))
            .find(|stem| !stem.is_empty())
            .unwrap_or_else(|| Self::FALLBACK_STEM.to_owned());
        match extension {
            Some(extension) => Self(format!("{stem}.{extension}")),
            None => Self(stem),
        }
    }

    /// The name, for the browser call that saves the file under it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ExportFilename {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// `text` without the characters a host refuses in a name, trimmed, and cut
/// on a character boundary to at most `room` bytes.
///
/// The refused set is the Windows one, which holds the POSIX one: every
/// control character, both separators, and `: * ? " < > |`. Every format
/// character goes too. A stem of dots alone is dropped, so `..` never
/// reaches a name.
fn clean_stem(text: &str, room: usize) -> String {
    let kept: String = text
        .chars()
        .filter(|&c| {
            !c.is_control()
                && !is_format_character(c)
                && !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
        .collect();
    let mut stem = String::new();
    for c in kept.trim().chars() {
        if stem.len() + c.len_utf8() > room {
            break;
        }
        stem.push(c);
    }
    let stem = stem.trim_end().to_owned();
    if stem.chars().all(|c| c == '.') {
        String::new()
    } else {
        stem
    }
}

/// Whether `c` is a format character, the Unicode general category `Cf`.
///
/// A format character is invisible, yet it changes how a name shows: the
/// right-to-left override U+202E shows `facture<U+202E>fdp.exe` as
/// `facture` then `exe.pdf`. The standard library has no predicate for the
/// category, so the ranges below list the `Cf` set of Unicode 15.1 by hand.
const fn is_format_character(c: char) -> bool {
    matches!(
        c,
        '\u{ad}'
            | '\u{600}'..='\u{605}'
            | '\u{61c}'
            | '\u{6dd}'
            | '\u{70f}'
            | '\u{890}'..='\u{891}'
            | '\u{8e2}'
            | '\u{180e}'
            | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206f}'
            | '\u{feff}'
            | '\u{fff9}'..='\u{fffb}'
            | '\u{110bd}'
            | '\u{110cd}'
            | '\u{13430}'..='\u{1343f}'
            | '\u{1bca0}'..='\u{1bca3}'
            | '\u{1d173}'..='\u{1d17a}'
            | '\u{e0001}'
            | '\u{e0020}'..='\u{e007f}'
    )
}

/// Whether a received name's `extension` survives whole: letters and digits
/// alone, at most [`ExportFilename::MAX_EXTENSION`] bytes. No such extension
/// holds a character a host refuses.
fn is_kept_extension(extension: &str) -> bool {
    !extension.is_empty()
        && ByteSize::of_len(extension.len()) <= ExportFilename::MAX_EXTENSION
        && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

/// The bytes of a file crossing an import or an export. They are what the
/// browser read from a picked file, or what an export writes for it to save.
///
/// The bytes are the user's content, so `Debug` prints their length alone.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct FileBytes(Vec<u8>);

impl FileBytes {
    /// Wraps the bytes of one file.
    #[must_use]
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// The bytes, for the parser or the browser call that takes them.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The bytes, owned.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    /// The size of the file.
    #[must_use]
    pub fn size(&self) -> ByteSize {
        ByteSize::of_len(self.0.len())
    }

    /// Whether the file holds no byte.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for FileBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FileBytes({} bytes)", self.0.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `DOCFILE-019`: a plain name is taken as written.
    #[test]
    fn a_plain_name_is_taken_as_written() {
        let name = ExportFilename::new("Q3 plan.md").expect("a valid name");
        assert_eq!(name.as_str(), "Q3 plan.md");
    }

    /// `DOCFILE-019`: an empty name is refused.
    #[test]
    fn an_empty_name_is_refused() {
        assert_eq!(ExportFilename::new(""), Err(ExportFilenameError::Empty));
    }

    /// `DOCFILE-019`: `.` and `..` name a folder, so both are refused.
    #[test]
    fn a_folder_reference_is_refused() {
        for text in [".", ".."] {
            assert_eq!(
                ExportFilename::new(text),
                Err(ExportFilenameError::FolderReference),
                "{text:?}"
            );
        }
    }

    /// `DOCFILE-019`: either separator is refused, wherever it sits.
    #[test]
    fn a_path_separator_is_refused() {
        for text in ["../plan.md", "a/b.md", "a\\b.md", "plan.md/"] {
            assert_eq!(
                ExportFilename::new(text),
                Err(ExportFilenameError::PathSeparator),
                "{text:?}"
            );
        }
    }

    /// `DOCFILE-019`: a control character is refused.
    #[test]
    fn a_control_character_is_refused() {
        for text in ["plan\n.md", "plan\0.md", "plan\u{7f}.md"] {
            assert_eq!(
                ExportFilename::new(text),
                Err(ExportFilenameError::ControlCharacter),
                "{text:?}"
            );
        }
    }

    /// The format characters a spoofed name hides: the bidirectional
    /// overrides, embeddings and isolates, the zero-width characters, the
    /// direction marks and the byte order mark.
    const SPOOFING_CHARACTERS: [char; 12] = [
        '\u{202e}', '\u{202d}', '\u{200b}', '\u{200c}', '\u{200d}', '\u{200e}', '\u{200f}',
        '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}', '\u{feff}',
    ];

    /// `DOCFILE-019`: a format character is refused, so a right-to-left
    /// override never shows `exe.pdf` for a name ending in `.exe`.
    #[test]
    fn a_format_character_is_refused() {
        for c in SPOOFING_CHARACTERS {
            let text = format!("facture{c}fdp.exe");
            assert_eq!(
                ExportFilename::new(&text),
                Err(ExportFilenameError::FormatCharacter),
                "{text:?}"
            );
        }
    }

    /// `DOCFILE-019`: a title loses its format characters.
    #[test]
    fn a_title_loses_its_format_characters() {
        for c in SPOOFING_CHARACTERS {
            let title = format!("{c}plan{c}Q3{c}");
            let name = ExportFilename::from_title(&title, "x", ExportExtension::Markdown);
            assert_eq!(name.as_str(), "planQ3.md", "{title:?}");
        }
    }

    /// `DOCFILE-019`: a received name loses its format characters, so the
    /// extension it shows is the extension it has.
    #[test]
    fn a_received_name_loses_its_format_characters() {
        for c in SPOOFING_CHARACTERS {
            let sent = format!("facture{c}fdp.exe");
            let name = ExportFilename::from_received(&sent, "attachment");
            assert_eq!(name.as_str(), "facturefdp.exe", "sent as {sent:?}");
            let sent = format!("{c}");
            let name = ExportFilename::from_received(&sent, "attachment");
            assert_eq!(name.as_str(), "attachment", "sent as {sent:?}");
        }
    }

    /// `DOCFILE-020`: a name past the bound is refused, and one at it is
    /// taken.
    #[test]
    fn a_name_past_the_bound_is_refused() {
        let at = "a".repeat(ExportFilename::MAX_SIZE.as_len());
        assert!(ExportFilename::new(&at).is_ok());
        let past = "a".repeat(ExportFilename::MAX_SIZE.as_len() + 1);
        assert_eq!(
            ExportFilename::new(&past),
            Err(ExportFilenameError::TooLong)
        );
    }

    /// `DOCFILE-019`: a title loses its separators, its control characters
    /// and the characters a host refuses, then keeps the extension.
    #[test]
    fn a_title_loses_what_a_host_refuses() {
        let name = ExportFilename::from_title(" a/b\\c:d*e?\"<>|\n ", "x", ExportExtension::Csv);
        assert_eq!(name.as_str(), "abcde.csv");
        assert!(ExportFilename::new(name.as_str()).is_ok());
    }

    /// `DOCFILE-019`: a title of dots alone never becomes `..`; the fallback
    /// stem names the file instead.
    #[test]
    fn a_title_of_dots_takes_the_fallback() {
        for title in ["..", ".", "...", "/..", "  "] {
            let name = ExportFilename::from_title(title, "document", ExportExtension::Markdown);
            assert_eq!(name.as_str(), "document.md", "{title:?}");
        }
    }

    /// `DOCFILE-020`: a long title is cut on a character boundary, and the
    /// extension survives the cut whole.
    #[test]
    fn a_long_title_is_cut_and_keeps_its_extension() {
        let title = "\u{e9}".repeat(400);
        let name = ExportFilename::from_title(&title, "x", ExportExtension::Markdown);
        assert!(ByteSize::of_len(name.as_str().len()) <= ExportFilename::MAX_SIZE);
        assert!(name.as_str().ends_with(".md"));
        assert!(ExportFilename::new(name.as_str()).is_ok());
    }

    /// `DOCFILE-019`: a received name keeps the last step of its path and
    /// loses what a host refuses; a name of dots takes the fallback stem.
    #[test]
    fn a_received_name_keeps_its_last_step() {
        let cases = [
            ("../../.ssh/authorized_keys", "authorized_keys"),
            ("..\\..\\boot.ini", "boot.ini"),
            ("..", "attachment"),
            ("a/..", "attachment"),
            ("a\0b\nc.pdf", "abc.pdf"),
            ("notes.tar.gz", "notes.tar.gz"),
            ("photo.j:pg", "photo.jpg"),
            ("report.pdf", "report.pdf"),
        ];
        for (sent, saved) in cases {
            let name = ExportFilename::from_received(sent, "attachment");
            assert_eq!(name.as_str(), saved, "sent as {sent:?}");
            assert!(ExportFilename::new(name.as_str()).is_ok());
        }
    }

    /// `DOCFILE-020`: a long received name is cut to the bound, and its
    /// extension survives the cut whole.
    #[test]
    fn a_long_received_name_keeps_its_extension() {
        let sent = format!("{}.pdf", "\u{e9}".repeat(400));
        let name = ExportFilename::from_received(&sent, "attachment");
        assert!(ByteSize::of_len(name.as_str().len()) <= ExportFilename::MAX_SIZE);
        assert!(name.as_str().ends_with(".pdf"));
        assert!(ExportFilename::new(name.as_str()).is_ok());
    }

    /// A suite-chosen name is its stem and its extension.
    #[test]
    fn a_suite_name_is_its_stem_and_extension() {
        assert_eq!(
            ExportFilename::named("board", ExportExtension::Svg).as_str(),
            "board.svg"
        );
    }

    /// The bytes of a file print their length alone, never their content.
    #[test]
    fn file_bytes_print_their_length_alone() {
        let bytes = FileBytes::new(b"secret".to_vec());
        assert_eq!(format!("{bytes:?}"), "FileBytes(6 bytes)");
        assert_eq!(bytes.size(), ByteSize(6));
    }
}
