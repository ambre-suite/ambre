//! Prose's command registry: one `Command` per affordance the shell offers
//! (`ambre-spec-commands.md`, `UI-THREAD-019`).
//!
//! Only the commands the shell routes live here. The text chords (typing,
//! moving the caret) are decoded by the view from the input pipe, and no
//! table names them. Undo and redo are described here for their menu rows,
//! and the view decodes their chords too: the shell binds none of them.

use ambre_commands::{Command, Home, KeyBinding, KeyChar, Keymap};
use ambre_core::{Categories, Commands, Icons, StaticKey};

use crate::mark::MarkRoot;

/// Taking the document out as a file.
const FILE: StaticKey<Categories> = ambre_commands::shared::FILE;
/// Inline formatting: the format bar and its chords.
const FORMAT: StaticKey<Categories> = StaticKey::<Categories>::new("format");
/// Moving text through the system clipboard.
const CLIPBOARD: StaticKey<Categories> = StaticKey::<Categories>::new("clipboard");
/// Commands that belong to the application shell rather than the document.
const GENERAL: StaticKey<Categories> = StaticKey::<Categories>::new("general");
/// Stepping through the local history.
const HISTORY: StaticKey<Categories> = StaticKey::<Categories>::new("history");
/// Adding a new object to the page, such as a picture.
const INSERT: StaticKey<Categories> = ambre_commands::shared::INSERT;
/// Ruling on the suggestions of a review.
const REVIEW: StaticKey<Categories> = StaticKey::<Categories>::new("review");

/// Accept the suggestion at the caret (`PROSE-UI-273`).
pub const ACCEPT_SUGGESTION: StaticKey<Commands> =
    StaticKey::<Commands>::new("prose.review.accept");
/// Reject the suggestion at the caret (`PROSE-UI-273`).
pub const REJECT_SUGGESTION: StaticKey<Commands> =
    StaticKey::<Commands>::new("prose.review.reject");
/// Turn suggestion mode on when it is off, and off when it is on
/// (`PROSE-UI-274`).
pub const SUGGEST_CHANGES: StaticKey<Commands> = StaticKey::<Commands>::new("prose.review.suggest");

/// Undo this peer's last change (`UNDO-001`).
pub const UNDO: StaticKey<Commands> = StaticKey::<Commands>::new("prose.history.undo");
/// Redo the last change undone.
pub const REDO: StaticKey<Commands> = StaticKey::<Commands>::new("prose.history.redo");

/// Open the command palette: a UI-only intent the shell intercepts
/// (`CHROME-008`).
pub const PALETTE: StaticKey<Commands> = StaticKey::<Commands>::new("prose.palette.open");

/// Save the document to a native `.ambre` file (`PROSE-IO-028`).
pub const SAVE_FILE: StaticKey<Commands> = StaticKey::<Commands>::new("prose.file.save");

/// Bring a Markdown file in after the last block (`PROSE-IO-027`).
pub const IMPORT_MARKDOWN: StaticKey<Commands> =
    StaticKey::<Commands>::new("prose.file.import_markdown");

/// Take the document out as GFM Markdown.
pub const EXPORT_MARKDOWN: StaticKey<Commands> =
    StaticKey::<Commands>::new("prose.file.export_markdown");

/// Bring a Word file in after the last block, through the Office module
/// (`PROSE-IO-060`).
pub const IMPORT_WORD: StaticKey<Commands> = StaticKey::<Commands>::new("prose.file.import_word");

/// Take the document out as a Word package, through the Office module, with
/// no comment and no suggestion (`PROSE-IO-014`, `PROSE-IO-081`).
pub const EXPORT_WORD: StaticKey<Commands> = StaticKey::<Commands>::new("prose.file.export_word");

/// Take the document out as a Word package with its comments and its
/// suggestions: publishing the review is an act of its own (`PROSE-IO-081`).
pub const EXPORT_WORD_REVIEWED: StaticKey<Commands> =
    StaticKey::<Commands>::new("prose.file.export_word_reviewed");

/// Bring an OpenDocument text in after the last block, through the Office
/// module, as the same text from Word would land (`PROSE-IO-088`).
pub const IMPORT_ODT: StaticKey<Commands> = StaticKey::<Commands>::new("prose.file.import_odt");

/// Take the document out as an OpenDocument text, with no comment and no
/// suggestion (`PROSE-IO-089`, `PROSE-IO-081`).
pub const EXPORT_ODT: StaticKey<Commands> = StaticKey::<Commands>::new("prose.file.export_odt");

/// Take the document out as an OpenDocument text with its comments and its
/// suggestions (`PROSE-IO-081`).
pub const EXPORT_ODT_REVIEWED: StaticKey<Commands> =
    StaticKey::<Commands>::new("prose.file.export_odt_reviewed");

/// Bring an HTML document in after the last block, through the walk a rich
/// paste runs (`PROSE-IO-025`).
pub const IMPORT_HTML: StaticKey<Commands> = StaticKey::<Commands>::new("prose.file.import_html");

/// Take the document out as a standalone HTML document, with no comment and
/// no suggestion (`PROSE-IO-022`).
pub const EXPORT_HTML: StaticKey<Commands> = StaticKey::<Commands>::new("prose.file.export_html");

/// Take the document out as an HTML document with its comments and its
/// suggestions (`PROSE-IO-022`).
pub const EXPORT_HTML_REVIEWED: StaticKey<Commands> =
    StaticKey::<Commands>::new("prose.file.export_html_reviewed");

/// Put a picture from a file after the block the caret sits in
/// (`PROSE-FEAT-316`).
pub const INSERT_PICTURE: StaticKey<Commands> = StaticKey::<Commands>::new("prose.insert.picture");

/// Put a paragraph of text after the block the caret sits in (`CHROME-254`).
pub const INSERT_TEXT: StaticKey<Commands> = StaticKey::<Commands>::new("prose.insert.text");
/// Put a table after the block the caret sits in (`CHROME-254`).
pub const INSERT_TABLE: StaticKey<Commands> = StaticKey::<Commands>::new("prose.insert.table");
/// Put a separator after the block the caret sits in (`CHROME-254`).
pub const INSERT_SEPARATOR: StaticKey<Commands> =
    StaticKey::<Commands>::new("prose.insert.separator");

/// Take the document out as a PDF of the pages the screen shows
/// (`PROSE-IO-033`).
pub const EXPORT_PDF: StaticKey<Commands> = StaticKey::<Commands>::new("prose.file.export_pdf");

/// The clipboard intents carry the shared identities: a cut is one command in
/// every application (`UI-THREAD-011`).
pub const CUT: StaticKey<Commands> = ambre_commands::shared::CUT;
pub const COPY: StaticKey<Commands> = ambre_commands::shared::COPY;
pub const PASTE: StaticKey<Commands> = ambre_commands::shared::PASTE;
/// Paste the text alone, every richer format dropped (`PROSE-UI-227`).
pub const PASTE_PLAIN: StaticKey<Commands> = ambre_commands::shared::PASTE_PLAIN;

/// The clipboard entries of the page's context menu, which it lists first
/// (`CHROME-179`).
pub const MENU_CLIPBOARD: [StaticKey<Commands>; 4] = [CUT, COPY, PASTE, PASTE_PLAIN];

/// The entries of the page's context menu, in the order it shows them: the
/// clipboard, then the formatting the context panel also holds
/// (`CHROME-182`, `CHROME-209`).
pub const CONTEXT_MENU: [StaticKey<Commands>; 9] = [
    CUT,
    COPY,
    PASTE,
    PASTE_PLAIN,
    BOLD,
    ITALIC,
    UNDERLINE,
    STRIKETHROUGH,
    CLEAR_FORMATTING,
];

/// The five controls of the format bar. A control and its chord invoke the
/// same id, and the id resolves to one mark key (`PROSE-UI-056`).
pub const BOLD: StaticKey<Commands> = StaticKey::<Commands>::new("prose.format.bold");
pub const ITALIC: StaticKey<Commands> = StaticKey::<Commands>::new("prose.format.italic");
pub const UNDERLINE: StaticKey<Commands> = StaticKey::<Commands>::new("prose.format.underline");
pub const STRIKETHROUGH: StaticKey<Commands> =
    StaticKey::<Commands>::new("prose.format.strikethrough");
pub const CODE: StaticKey<Commands> = StaticKey::<Commands>::new("prose.format.code");

/// Superscript and subscript, two marks of the context panel
/// (`PROSE-UI-191`).
pub const SUPERSCRIPT: StaticKey<Commands> = StaticKey::<Commands>::new("prose.format.superscript");
pub const SUBSCRIPT: StaticKey<Commands> = StaticKey::<Commands>::new("prose.format.subscript");
/// Remove every formatting mark of the selection (`PROSE-UI-191`,
/// `PROSE-UI-254`).
pub const CLEAR_FORMATTING: StaticKey<Commands> = StaticKey::<Commands>::new("prose.format.clear");
/// Link the selection to an address, or take its link off
/// (`PROSE-UI-188`). The press opens the address field of the bar.
pub const LINK: StaticKey<Commands> = StaticKey::<Commands>::new("prose.format.link");
/// Open a discussion thread over the selection. A thread is a new object,
/// so the creation bar homes it (`CHROME-180`). The press opens the comment
/// field under the format bar, which shows over a selection alone.
pub const COMMENT: StaticKey<Commands> = StaticKey::<Commands>::new("prose.comment.add");

/// The three marks the floating bar shows, in the mockup's order. The bar
/// adds the link and the paragraph style list after them.
pub const BAR_MARKS: [(StaticKey<Commands>, MarkRoot); 3] = [
    (BOLD, MarkRoot::Bold),
    (ITALIC, MarkRoot::Italic),
    (UNDERLINE, MarkRoot::Underline),
];

/// Each control of the format bar, in the order the bar shows them, with the
/// mark key it applies or removes.
pub const FORMAT_BAR: [(StaticKey<Commands>, MarkRoot); 5] = [
    (BOLD, MarkRoot::Bold),
    (ITALIC, MarkRoot::Italic),
    (UNDERLINE, MarkRoot::Underline),
    (STRIKETHROUGH, MarkRoot::Strike),
    (CODE, MarkRoot::Code),
];

/// The two marks the context panel adds to the five of the bar.
pub const PANEL_MARKS: [(StaticKey<Commands>, MarkRoot); 2] = [
    (SUPERSCRIPT, MarkRoot::Superscript),
    (SUBSCRIPT, MarkRoot::Subscript),
];

/// The mark key a formatting command applies, or `None` for any other id.
#[must_use]
pub fn mark_key(id: StaticKey<Commands>) -> Option<MarkRoot> {
    FORMAT_BAR
        .iter()
        .chain(PANEL_MARKS.iter())
        .find(|(bar, _)| *bar == id)
        .map(|(_, key)| *key)
}

/// Whether `id` formats the selection: a mark, or clearing every mark. The
/// view drains such an id from the one signal a control, a chord and a menu
/// entry each set (`PROSE-UI-056`, `CHROME-214`).
#[must_use]
pub fn formats_selection(id: StaticKey<Commands>) -> bool {
    id == CLEAR_FORMATTING || mark_key(id).is_some()
}

const SUPERSCRIPT_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Superscript",
    fr: "Exposant",
};

const SUBSCRIPT_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Subscript",
    fr: "Indice",
};

const CLEAR_FORMATTING_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Clear formatting",
    fr: "Effacer la mise en forme",
};

const LINK_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Link",
    fr: "Lien",
};

const COMMENT_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Comment",
    fr: "Commentaire",
};

const BOLD_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Bold",
    fr: "Gras",
};

const ITALIC_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Italic",
    fr: "Italique",
};

const UNDERLINE_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Underline",
    fr: "Soulign\u{e9}",
};

const STRIKETHROUGH_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Strikethrough",
    fr: "Barr\u{e9}",
};

const CODE_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Inline code",
    fr: "Code en ligne",
};

const CUT_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Cut",
    fr: "Couper",
};

const COPY_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Copy",
    fr: "Copier",
};

const PASTE_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Paste",
    fr: "Coller",
};

const PASTE_PLAIN_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Paste as plain text",
    fr: "Coller en texte brut",
};

const ACCEPT_SUGGESTION_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Accept the suggestion",
    fr: "Accepter la suggestion",
};

const REJECT_SUGGESTION_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Reject the suggestion",
    fr: "Rejeter la suggestion",
};

const SUGGEST_CHANGES_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Suggest changes",
    fr: "Sugg\u{e9}rer des modifications",
};

const UNDO_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Undo",
    fr: "Annuler",
};

const REDO_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Redo",
    fr: "R\u{e9}tablir",
};

const PALETTE_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Command palette",
    fr: "Palette de commandes",
};

const IMPORT_MARKDOWN_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Import a Markdown file",
    fr: "Importer un fichier Markdown",
};

const EXPORT_MARKDOWN_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Export to Markdown (.md)",
    fr: "Exporter en Markdown (.md)",
};

const IMPORT_WORD_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Import a Word file",
    fr: "Importer un fichier Word",
};

const EXPORT_WORD_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Export to Word (.docx)",
    fr: "Exporter en Word (.docx)",
};

const EXPORT_WORD_REVIEWED_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Export to Word with comments and suggestions (.docx)",
    fr: "Exporter en Word avec commentaires et suggestions (.docx)",
};

const IMPORT_ODT_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Import an OpenDocument text",
    fr: "Importer un texte OpenDocument",
};

const EXPORT_ODT_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Export to OpenDocument (.odt)",
    fr: "Exporter en OpenDocument (.odt)",
};

const EXPORT_ODT_REVIEWED_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Export to OpenDocument with comments and suggestions (.odt)",
    fr: "Exporter en OpenDocument avec commentaires et suggestions (.odt)",
};

const INSERT_PICTURE_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Insert a picture",
    fr: "Ins\u{e9}rer une image",
};

const INSERT_TEXT_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Insert text",
    fr: "Ins\u{e9}rer du texte",
};

const INSERT_TABLE_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Insert a table",
    fr: "Ins\u{e9}rer un tableau",
};

const INSERT_SEPARATOR_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Insert a separator",
    fr: "Ins\u{e9}rer un s\u{e9}parateur",
};

const IMPORT_HTML_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Import an HTML document",
    fr: "Importer un document HTML",
};

const EXPORT_HTML_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Export to HTML (.html)",
    fr: "Exporter en HTML (.html)",
};

const EXPORT_HTML_REVIEWED_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Export to HTML with comments and suggestions (.html)",
    fr: "Exporter en HTML avec commentaires et suggestions (.html)",
};

const EXPORT_PDF_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Export to PDF (.pdf)",
    fr: "Exporter en PDF (.pdf)",
};

// The file is not sealed, and the label says so (`DOCFILE-009`).
const SAVE_FILE_TITLE: ambre_commands::TitleWords = ambre_commands::TitleWords {
    en: "Save to a file (.ambre, not encrypted)",
    fr: "Enregistrer dans un fichier (.ambre, non chiffr\u{e9})",
};

/// Every command Prose contributes, described once.
pub const COMMANDS: &[Command] = &[
    Command {
        id: BOLD,
        title: BOLD_TITLE,
        icon: Some(StaticKey::<Icons>::new("format-bold")),
        keybindings: &[KeyBinding::primary(KeyChar::B)],
        category: FORMAT,
        home: Home::ContextPanel,
        description: None,
    },
    Command {
        id: ITALIC,
        title: ITALIC_TITLE,
        icon: Some(StaticKey::<Icons>::new("format-italic")),
        keybindings: &[KeyBinding::primary(KeyChar::I)],
        category: FORMAT,
        home: Home::ContextPanel,
        description: None,
    },
    Command {
        id: UNDERLINE,
        title: UNDERLINE_TITLE,
        icon: Some(StaticKey::<Icons>::new("format-underline")),
        keybindings: &[KeyBinding::primary(KeyChar::U)],
        category: FORMAT,
        home: Home::ContextPanel,
        description: None,
    },
    Command {
        id: STRIKETHROUGH,
        title: STRIKETHROUGH_TITLE,
        icon: Some(StaticKey::<Icons>::new("format-strikethrough")),
        keybindings: &[KeyBinding::primary_shift(KeyChar::X)],
        category: FORMAT,
        home: Home::ContextPanel,
        description: None,
    },
    Command {
        id: CODE,
        title: CODE_TITLE,
        icon: Some(StaticKey::<Icons>::new("code-tags")),
        keybindings: &[KeyBinding::primary(KeyChar::E)],
        category: FORMAT,
        home: Home::ContextPanel,
        description: None,
    },
    Command {
        id: SUPERSCRIPT,
        title: SUPERSCRIPT_TITLE,
        icon: Some(StaticKey::<Icons>::new("format-superscript")),
        keybindings: &[],
        category: FORMAT,
        home: Home::ContextPanel,
        description: None,
    },
    Command {
        id: SUBSCRIPT,
        title: SUBSCRIPT_TITLE,
        icon: Some(StaticKey::<Icons>::new("format-subscript")),
        keybindings: &[],
        category: FORMAT,
        home: Home::ContextPanel,
        description: None,
    },
    Command {
        id: CLEAR_FORMATTING,
        title: CLEAR_FORMATTING_TITLE,
        icon: Some(StaticKey::<Icons>::new("format-clear")),
        keybindings: &[KeyBinding::primary(KeyChar::new('\\'))],
        category: FORMAT,
        home: Home::ContextPanel,
        description: None,
    },
    Command {
        id: LINK,
        title: LINK_TITLE,
        icon: Some(StaticKey::<Icons>::new("link")),
        keybindings: &[],
        category: FORMAT,
        home: Home::ContextPanel,
        description: None,
    },
    Command {
        id: SAVE_FILE,
        title: SAVE_FILE_TITLE,
        icon: Some(StaticKey::<Icons>::new("content-save")),
        keybindings: &[],
        category: FILE,
        home: Home::AmbreMenu,
        description: None,
    },
    Command {
        id: IMPORT_MARKDOWN,
        title: IMPORT_MARKDOWN_TITLE,
        icon: Some(StaticKey::<Icons>::new("file-import-outline")),
        keybindings: &[],
        category: FILE,
        home: Home::AmbreMenu,
        description: None,
    },
    Command {
        id: EXPORT_MARKDOWN,
        title: EXPORT_MARKDOWN_TITLE,
        icon: Some(StaticKey::<Icons>::new("file-export-outline")),
        keybindings: &[],
        category: FILE,
        home: Home::AmbreMenu,
        description: None,
    },
    Command {
        id: IMPORT_WORD,
        title: IMPORT_WORD_TITLE,
        icon: Some(StaticKey::<Icons>::new("file-import-outline")),
        keybindings: &[],
        category: FILE,
        home: Home::AmbreMenu,
        description: None,
    },
    Command {
        id: EXPORT_WORD,
        title: EXPORT_WORD_TITLE,
        icon: Some(StaticKey::<Icons>::new("file-export-outline")),
        keybindings: &[],
        category: FILE,
        home: Home::AmbreMenu,
        description: None,
    },
    Command {
        id: EXPORT_WORD_REVIEWED,
        title: EXPORT_WORD_REVIEWED_TITLE,
        icon: Some(StaticKey::<Icons>::new("file-export-outline")),
        keybindings: &[],
        category: FILE,
        home: Home::AmbreMenu,
        description: None,
    },
    Command {
        id: IMPORT_ODT,
        title: IMPORT_ODT_TITLE,
        icon: Some(StaticKey::<Icons>::new("file-import-outline")),
        keybindings: &[],
        category: FILE,
        home: Home::AmbreMenu,
        description: None,
    },
    Command {
        id: EXPORT_ODT,
        title: EXPORT_ODT_TITLE,
        icon: Some(StaticKey::<Icons>::new("file-export-outline")),
        keybindings: &[],
        category: FILE,
        home: Home::AmbreMenu,
        description: None,
    },
    Command {
        id: EXPORT_ODT_REVIEWED,
        title: EXPORT_ODT_REVIEWED_TITLE,
        icon: Some(StaticKey::<Icons>::new("file-export-outline")),
        keybindings: &[],
        category: FILE,
        home: Home::AmbreMenu,
        description: None,
    },
    Command {
        id: IMPORT_HTML,
        title: IMPORT_HTML_TITLE,
        icon: Some(StaticKey::<Icons>::new("file-import-outline")),
        keybindings: &[],
        category: FILE,
        home: Home::AmbreMenu,
        description: None,
    },
    Command {
        id: EXPORT_HTML,
        title: EXPORT_HTML_TITLE,
        icon: Some(StaticKey::<Icons>::new("file-export-outline")),
        keybindings: &[],
        category: FILE,
        home: Home::AmbreMenu,
        description: None,
    },
    Command {
        id: EXPORT_HTML_REVIEWED,
        title: EXPORT_HTML_REVIEWED_TITLE,
        icon: Some(StaticKey::<Icons>::new("file-export-outline")),
        keybindings: &[],
        category: FILE,
        home: Home::AmbreMenu,
        description: None,
    },
    Command {
        id: INSERT_TEXT,
        title: INSERT_TEXT_TITLE,
        icon: Some(StaticKey::<Icons>::new("format-text")),
        keybindings: &[],
        category: INSERT,
        home: Home::CreationBar,
        description: None,
    },
    Command {
        id: INSERT_PICTURE,
        title: INSERT_PICTURE_TITLE,
        icon: Some(StaticKey::<Icons>::new("image-outline")),
        keybindings: &[],
        category: INSERT,
        home: Home::CreationBar,
        description: None,
    },
    Command {
        id: INSERT_TABLE,
        title: INSERT_TABLE_TITLE,
        icon: Some(StaticKey::<Icons>::new("table-plus")),
        keybindings: &[],
        category: INSERT,
        home: Home::CreationBar,
        description: None,
    },
    Command {
        id: INSERT_SEPARATOR,
        title: INSERT_SEPARATOR_TITLE,
        icon: Some(StaticKey::<Icons>::new("border-horizontal")),
        keybindings: &[],
        category: INSERT,
        home: Home::CreationBar,
        description: None,
    },
    Command {
        id: COMMENT,
        title: COMMENT_TITLE,
        icon: Some(StaticKey::<Icons>::new("comment-outline")),
        keybindings: &[],
        category: INSERT,
        home: Home::CreationBar,
        description: None,
    },
    Command {
        id: EXPORT_PDF,
        title: EXPORT_PDF_TITLE,
        icon: Some(StaticKey::<Icons>::new("file-export-outline")),
        keybindings: &[],
        category: FILE,
        home: Home::AmbreMenu,
        description: None,
    },
    Command {
        id: CUT,
        title: CUT_TITLE,
        icon: Some(StaticKey::<Icons>::new("content-cut")),
        keybindings: ambre_commands::shared::CUT_CHORDS,
        category: CLIPBOARD,
        home: Home::ContextMenu,
        description: None,
    },
    Command {
        id: COPY,
        title: COPY_TITLE,
        icon: Some(StaticKey::<Icons>::new("content-copy")),
        keybindings: ambre_commands::shared::COPY_CHORDS,
        category: CLIPBOARD,
        home: Home::ContextMenu,
        description: None,
    },
    Command {
        id: PASTE,
        title: PASTE_TITLE,
        icon: Some(StaticKey::<Icons>::new("content-paste")),
        keybindings: ambre_commands::shared::PASTE_CHORDS,
        category: CLIPBOARD,
        home: Home::ContextMenu,
        description: None,
    },
    Command {
        id: PASTE_PLAIN,
        title: PASTE_PLAIN_TITLE,
        icon: Some(StaticKey::<Icons>::new("content-paste")),
        keybindings: ambre_commands::shared::PASTE_PLAIN_CHORDS,
        category: CLIPBOARD,
        home: Home::ContextMenu,
        description: None,
    },
    Command {
        id: ACCEPT_SUGGESTION,
        title: ACCEPT_SUGGESTION_TITLE,
        icon: None,
        keybindings: &[],
        category: REVIEW,
        home: Home::ContextPanel,
        description: None,
    },
    Command {
        id: REJECT_SUGGESTION,
        title: REJECT_SUGGESTION_TITLE,
        icon: None,
        keybindings: &[],
        category: REVIEW,
        home: Home::ContextPanel,
        description: None,
    },
    Command {
        id: SUGGEST_CHANGES,
        title: SUGGEST_CHANGES_TITLE,
        icon: Some(StaticKey::<Icons>::new("file-document-edit-outline")),
        // The chord Word gives the same mode, so a reviewer's hand knows it.
        keybindings: &[KeyBinding::primary_shift(KeyChar::E)],
        category: REVIEW,
        home: Home::ContextPanel,
        description: None,
    },
    Command {
        id: UNDO,
        title: UNDO_TITLE,
        icon: Some(StaticKey::<Icons>::new("undo")),
        keybindings: ambre_commands::shared::UNDO_CHORDS,
        category: HISTORY,
        home: Home::AmbreMenu,
        description: None,
    },
    Command {
        id: REDO,
        title: REDO_TITLE,
        icon: Some(StaticKey::<Icons>::new("redo")),
        keybindings: ambre_commands::shared::REDO_CHORDS,
        category: HISTORY,
        home: Home::AmbreMenu,
        description: None,
    },
    Command {
        id: PALETTE,
        title: PALETTE_TITLE,
        icon: None,
        keybindings: &[KeyBinding::primary(KeyChar::K)],
        category: GENERAL,
        home: Home::AmbreMenu,
        description: None,
    },
];

/// Bind the chords the shell routes for Prose.
pub fn bind_keys(keymap: &mut Keymap) {
    for id in FORMAT_BAR
        .map(|(id, _)| id)
        .into_iter()
        .chain([PALETTE, SUGGEST_CHANGES])
    {
        if let Some(chord) = ambre_commands::find(COMMANDS, id).and_then(Command::chord) {
            keymap.bind(chord, id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each command is described once, with a title in every locale, a
    /// group, no blank icon and no chord another command holds
    /// (`UI-THREAD-019`, `UI-THREAD-037`).
    #[test]
    fn the_table_has_no_defect() {
        assert_eq!(ambre_commands::table_defects(COMMANDS), []);
    }

    /// `CHROME-180`: the creation bar of the page offers a comment, after
    /// the four blocks it inserts.
    #[test]
    fn the_creation_bar_offers_a_comment_after_the_four_inserts() {
        let ids: Vec<StaticKey<Commands>> = COMMANDS
            .iter()
            .filter(|c| c.home == Home::CreationBar)
            .map(|c| c.id)
            .collect();
        assert_eq!(
            ids,
            [
                INSERT_TEXT,
                INSERT_PICTURE,
                INSERT_TABLE,
                INSERT_SEPARATOR,
                COMMENT
            ]
        );
    }

    /// `PROSE-IO-028`, `PROSE-IO-027`: the save, import and export entries
    /// are declared once each, so the palette lists them.
    #[test]
    fn the_save_entry_is_declared_once() {
        for id in [
            SAVE_FILE,
            IMPORT_MARKDOWN,
            EXPORT_MARKDOWN,
            IMPORT_WORD,
            EXPORT_WORD,
            EXPORT_WORD_REVIEWED,
            IMPORT_ODT,
            EXPORT_ODT,
            EXPORT_ODT_REVIEWED,
            IMPORT_HTML,
            EXPORT_HTML,
            EXPORT_HTML_REVIEWED,
            EXPORT_PDF,
            INSERT_PICTURE,
            INSERT_TEXT,
            INSERT_TABLE,
            INSERT_SEPARATOR,
        ] {
            let declared = COMMANDS.iter().filter(|c| c.id == id).count();
            assert_eq!(declared, 1, "{id:?}");
        }
        let mut keymap = Keymap::new();
        bind_keys(&mut keymap);
        assert_eq!(
            keymap.resolve(KeyBinding::primary(KeyChar::K)),
            Some(PALETTE)
        );
    }

    /// `PROSE-UI-056`: each control's chord resolves to the control's own
    /// id, and the id to one mark key.
    #[test]
    fn a_control_and_its_chord_resolve_to_one_command_id() {
        let mut keymap = Keymap::new();
        bind_keys(&mut keymap);
        for (id, key) in FORMAT_BAR {
            let command = ambre_commands::find(COMMANDS, id).expect("the control is declared");
            let chord = command.chord().expect("the control has a chord");
            assert_eq!(keymap.resolve(chord), Some(id));
            assert_eq!(mark_key(id), Some(key));
        }
        assert_eq!(mark_key(PALETTE), None);
    }

    /// `PROSE-UI-227`: each entry of the context menu is declared, and the
    /// keymap leaves the clipboard chords to the browser's own events.
    #[test]
    fn the_context_menu_entries_are_declared_and_left_unbound() {
        let mut keymap = Keymap::new();
        bind_keys(&mut keymap);
        for id in CONTEXT_MENU {
            assert!(
                ambre_commands::find(COMMANDS, id).is_some(),
                "{id:?} is declared"
            );
        }
        for id in MENU_CLIPBOARD {
            let command = ambre_commands::find(COMMANDS, id).expect("the entry is declared");
            for chord in command.keybindings {
                assert_eq!(keymap.resolve(*chord), None);
            }
        }
    }
    /// `CHROME-214`: every entry of the context menu past the clipboard
    /// formats the selection, so the handler hands it to the view. An entry
    /// that formats nothing would close the menu and change nothing.
    #[test]
    fn every_menu_entry_past_the_clipboard_formats_the_selection() {
        for id in CONTEXT_MENU {
            assert!(
                MENU_CLIPBOARD.contains(&id) || formats_selection(id),
                "{id:?} reaches nothing"
            );
        }
        assert!(!formats_selection(PALETTE));
        assert!(!formats_selection(CUT));
    }

    /// `PROSE-UI-276`: the suggestion-mode command is declared once, and its
    /// chord resolves to it, so a control, the chord and the palette run one
    /// id.
    #[test]
    fn the_suggestion_mode_command_is_declared_once_and_its_chord_resolves_to_it() {
        let declared = COMMANDS.iter().filter(|c| c.id == SUGGEST_CHANGES).count();
        assert_eq!(declared, 1);
        let mut keymap = Keymap::new();
        bind_keys(&mut keymap);
        assert_eq!(
            keymap.resolve(KeyBinding::primary_shift(KeyChar::E)),
            Some(SUGGEST_CHANGES)
        );
        assert!(!formats_selection(SUGGEST_CHANGES));
    }
}
