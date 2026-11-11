//! The file an export hands the browser: its name and its bytes.
//!
//! An export name derives from a document title or a sheet name, so it is
//! user input. The browser saves the download under that name. On a host
//! that honours it, a separator or a `..` names a place outside the download
//! folder. So the name is a validated type (`DOCFILE-019`, `DOCFILE-020`).

use crate::Redacted;
use crate::count::Count;
use crate::index::Len;
use crate::media::Octets;
use crate::payload::{Payload, PayloadKind};
use crate::text_offset::Byte;

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
    /// A text document as a Word package.
    Docx,
    /// A workbook as an Excel package.
    Xlsx,
    /// A deck as a PowerPoint package.
    Pptx,
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
            Self::Docx => "docx",
            Self::Xlsx => "xlsx",
            Self::Pptx => "pptx",
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
    /// The name holds a deceptive character: a bidirectional control, or an
    /// invisible character with no linguistic role. An override, for example,
    /// shows `exe.pdf` for a name that ends in `.exe`.
    #[error("an export file name holds a deceptive character")]
    DeceptiveCharacter,
    /// The name is longer than [`ExportFilename::MAX_SIZE`].
    #[error("an export file name is too long")]
    TooLong,
}

/// The name an export file is saved under. It is never empty, never `.` or
/// `..`, and holds no path separator, no control character and no deceptive
/// character (`DOCFILE-019`). It holds at most [`MAX_SIZE`](Self::MAX_SIZE) bytes
/// (`DOCFILE-020`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExportFilename(Redacted<String>);

impl ExportFilename {
    /// The most UTF-8 bytes a name holds, extension included: the limit of
    /// most file systems.
    pub const MAX_SIZE: Len<Byte> = Len::<Byte>::new(255);

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
        if text.chars().any(is_deceptive_character) {
            return Err(ExportFilenameError::DeceptiveCharacter);
        }
        if Len::<Byte>::of(text.len()) > Self::MAX_SIZE {
            return Err(ExportFilenameError::TooLong);
        }
        Ok(Self(Redacted::new(text.to_owned())))
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
        let room = Self::MAX_SIZE.saturating_sub(Len::<Byte>::of(suffix.len() + 1));
        let stem = [title, fallback, Self::FALLBACK_STEM]
            .into_iter()
            .map(|text| clean_stem(text, room))
            .find(|stem| !stem.is_empty())
            .unwrap_or_else(|| Self::FALLBACK_STEM.to_owned());
        Self(Redacted::new(format!("{stem}.{suffix}")))
    }

    /// The longest extension a received name keeps.
    const MAX_EXTENSION: Len<Byte> = Len::<Byte>::new(16);

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
        let room = Self::MAX_SIZE.saturating_sub(Len::<Byte>::of(
            extension.map_or(0, |extension| extension.len() + 1),
        ));
        let stem = [stem, fallback, Self::FALLBACK_STEM]
            .into_iter()
            .map(|text| clean_stem(text, room))
            .find(|stem| !stem.is_empty())
            .unwrap_or_else(|| Self::FALLBACK_STEM.to_owned());
        match extension {
            Some(extension) => Self(Redacted::new(format!("{stem}.{extension}"))),
            None => Self(Redacted::new(stem)),
        }
    }

    /// The name, for the browser call that saves the file under it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl std::fmt::Display for ExportFilename {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.as_str())
    }
}

/// `text` without the characters a host refuses in a name, trimmed, and cut
/// on a character boundary to fit `room`.
///
/// The refused set is the Windows one, which holds the POSIX one: every
/// control character, both separators, and `: * ? " < > |`. Every deceptive
/// character goes too. A stem of dots alone is dropped, so `..` never
/// reaches a name.
fn clean_stem(text: &str, room: Len<Byte>) -> String {
    let kept: String = text
        .chars()
        .filter(|&c| {
            !c.is_control()
                && !is_deceptive_character(c)
                && !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
        .collect();
    let mut stem = String::new();
    for c in kept.trim().chars() {
        if Len::<Byte>::of(stem.len() + c.len_utf8()) > room {
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

/// Whether `c` deceives the eye in a name: a bidirectional control, or an
/// invisible character with no linguistic role.
///
/// A bidirectional control reorders what a name shows: the right-to-left
/// override U+202E shows `facture<U+202E>fdp.exe` as `facture` then
/// `exe.pdf`. An invisible character, such as the zero-width space, makes two
/// different names look the same. Other format characters stay. The
/// zero-width non-joiner U+200C spells Persian and Indic words. The
/// zero-width joiner U+200D joins emoji sequences. The tag characters
/// U+E0020 to U+E007F spell subdivision flags.
const fn is_deceptive_character(c: char) -> bool {
    matches!(
        c,
        // The bidirectional controls: the Arabic letter mark, the two
        // direction marks, the embeddings and overrides, and the isolates.
        '\u{61c}'
            | '\u{200e}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
            // The invisible characters with no linguistic role: the soft
            // hyphen, the zero-width space, the word joiner, the invisible
            // operators and the byte order mark.
            | '\u{ad}'
            | '\u{200b}'
            | '\u{2060}'..='\u{2064}'
            | '\u{feff}'
    )
}

/// Whether a received name's `extension` survives whole: letters and digits
/// alone, at most [`ExportFilename::MAX_EXTENSION`] bytes. No such extension
/// holds a character a host refuses.
fn is_kept_extension(extension: &str) -> bool {
    !extension.is_empty()
        && Len::<Byte>::of(extension.len()) <= ExportFilename::MAX_EXTENSION
        && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

/// The role of the bytes of a file crossing an import or an export:
/// `Payload<FileBytes>`. They are what the browser read from a picked file,
/// or what an export writes for it to save.
///
/// The bytes are the user's content, so `Debug` prints their length alone.
pub enum FileBytes {}

impl PayloadKind for FileBytes {
    const NAME: &'static str = "FileBytes";
}

impl Payload<FileBytes> {
    /// The size of the file.
    #[must_use]
    pub fn size(&self) -> Count<Octets> {
        self.len()
    }
}

/// A plaintext `.ambre` export saves the whole container, header included.
impl From<Payload<crate::PlaintextContainer>> for Payload<FileBytes> {
    fn from(container: Payload<crate::PlaintextContainer>) -> Self {
        Self::new(container.into_bytes())
    }
}

/// A sealed `.ambre` export saves the whole sealed container.
impl From<Payload<crate::SealedContainer>> for Payload<FileBytes> {
    fn from(container: Payload<crate::SealedContainer>) -> Self {
        Self::new(container.into_bytes())
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

    /// Every deceptive character. The bidirectional controls come first,
    /// then the invisible characters with no linguistic role.
    const DECEPTIVE_CHARACTERS: [char; 20] = [
        '\u{61c}', '\u{200e}', '\u{200f}', '\u{202a}', '\u{202b}', '\u{202c}', '\u{202d}',
        '\u{202e}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}', '\u{ad}', '\u{200b}',
        '\u{2060}', '\u{2061}', '\u{2062}', '\u{2063}', '\u{2064}', '\u{feff}',
    ];

    /// Texts whose format characters carry meaning. The first is a Persian
    /// word with a zero-width non-joiner. The second is an emoji family with
    /// zero-width joiners. The third is the flag of England, which tag
    /// characters spell.
    const MEANINGFUL_TEXTS: [&str; 3] = [
        "\u{645}\u{6cc}\u{200c}\u{62e}\u{648}\u{627}\u{647}\u{645}",
        "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}",
        "\u{1f3f4}\u{e0067}\u{e0062}\u{e0065}\u{e006e}\u{e0067}\u{e007f}",
    ];

    /// `DOCFILE-019`: a deceptive character is refused, so a right-to-left
    /// override never shows `exe.pdf` for a name ending in `.exe`.
    #[test]
    fn a_deceptive_character_is_refused() {
        for c in DECEPTIVE_CHARACTERS {
            let text = format!("facture{c}fdp.exe");
            assert_eq!(
                ExportFilename::new(&text),
                Err(ExportFilenameError::DeceptiveCharacter),
                "{text:?}"
            );
        }
    }

    /// `DOCFILE-019`: a joiner or a tag character is no deceptive character,
    /// so the checked constructor takes a name that holds one as written.
    #[test]
    fn a_joiner_or_a_tag_is_taken_as_written() {
        for text in MEANINGFUL_TEXTS {
            let text = format!("{text}.md");
            let name = ExportFilename::new(&text).expect("a valid name");
            assert_eq!(name.as_str(), text);
        }
    }

    /// `DOCFILE-019`: a title loses its deceptive characters.
    #[test]
    fn a_title_loses_its_deceptive_characters() {
        for c in DECEPTIVE_CHARACTERS {
            let title = format!("{c}plan{c}Q3{c}");
            let name = ExportFilename::from_title(&title, "x", ExportExtension::Markdown);
            assert_eq!(name.as_str(), "planQ3.md", "{title:?}");
        }
    }

    /// `DOCFILE-019`: a title keeps its joiners and its tag characters, so a
    /// Persian word, an emoji family and a flag survive in the name.
    #[test]
    fn a_title_keeps_its_joiners_and_tags() {
        for title in MEANINGFUL_TEXTS {
            let name = ExportFilename::from_title(title, "x", ExportExtension::Markdown);
            assert_eq!(name.as_str(), format!("{title}.md"), "{title:?}");
        }
    }

    /// `DOCFILE-019`: a received name loses its deceptive characters, so the
    /// extension it shows is the extension it has.
    #[test]
    fn a_received_name_loses_its_deceptive_characters() {
        for c in DECEPTIVE_CHARACTERS {
            let sent = format!("facture{c}fdp.exe");
            let name = ExportFilename::from_received(&sent, "attachment");
            assert_eq!(name.as_str(), "facturefdp.exe", "sent as {sent:?}");
            let sent = format!("{c}");
            let name = ExportFilename::from_received(&sent, "attachment");
            assert_eq!(name.as_str(), "attachment", "sent as {sent:?}");
        }
    }

    /// `DOCFILE-019`: a received name keeps its joiners and its tag
    /// characters, with its extension.
    #[test]
    fn a_received_name_keeps_its_joiners_and_tags() {
        for text in MEANINGFUL_TEXTS {
            let sent = format!("{text}.pdf");
            let name = ExportFilename::from_received(&sent, "attachment");
            assert_eq!(name.as_str(), sent, "sent as {sent:?}");
        }
    }

    /// `DOCFILE-020`: a name past the bound is refused, and one at it is
    /// taken.
    #[test]
    fn a_name_past_the_bound_is_refused() {
        let at = "a".repeat(ExportFilename::MAX_SIZE.to_usize());
        assert!(ExportFilename::new(&at).is_ok());
        let past = "a".repeat(ExportFilename::MAX_SIZE.to_usize() + 1);
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
        assert!(Len::<Byte>::of(name.as_str().len()) <= ExportFilename::MAX_SIZE);
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
        assert!(Len::<Byte>::of(name.as_str().len()) <= ExportFilename::MAX_SIZE);
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
        let bytes = Payload::<FileBytes>::new(b"secret".to_vec());
        assert_eq!(format!("{bytes:?}"), "FileBytes(6 bytes)");
        assert_eq!(bytes.size(), Count::<Octets>::new(6));
    }

    /// A name holds the document's title: its `Debug` form prints the length
    /// alone.
    #[test]
    fn a_file_name_prints_its_length_alone() {
        let name = ExportFilename::new("Q3 layoffs.pdf").expect("a valid name");
        assert_eq!(format!("{name:?}"), "ExportFilename(14 chars)");
    }
}
