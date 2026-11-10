//! The relationship types a Word package carries beside the two the package
//! layer names itself (`ambre_opc::RelKind`). Each URI is the transitional
//! one the writer gives, and a strict package reads the same through its
//! last segment.

use crate::opc::RelKind;

pub const STYLES: RelKind =
    RelKind::new("http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles");
pub const NUMBERING: RelKind =
    RelKind::new("http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering");
pub const SETTINGS: RelKind =
    RelKind::new("http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings");
pub const FOOTNOTES: RelKind =
    RelKind::new("http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes");
pub const ENDNOTES: RelKind =
    RelKind::new("http://schemas.openxmlformats.org/officeDocument/2006/relationships/endnotes");
pub const COMMENTS: RelKind =
    RelKind::new("http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments");
pub const COMMENTS_EXTENDED: RelKind =
    RelKind::new("http://schemas.microsoft.com/office/2011/relationships/commentsExtended");
pub const HEADER: RelKind =
    RelKind::new("http://schemas.openxmlformats.org/officeDocument/2006/relationships/header");
pub const FOOTER: RelKind =
    RelKind::new("http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer");
pub const HYPERLINK: RelKind =
    RelKind::new("http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink");
pub const IMAGE: RelKind =
    RelKind::new("http://schemas.openxmlformats.org/officeDocument/2006/relationships/image");
/// A macro project. The reader names it and never reads it.
pub const VBA_PROJECT: RelKind =
    RelKind::new("http://schemas.microsoft.com/office/2006/relationships/vbaProject");
