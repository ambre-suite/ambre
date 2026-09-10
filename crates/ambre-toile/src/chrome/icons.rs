// ambre-toile/src/chrome/icons.rs
// ============================================================================
// `IconId` -> `icondata::Icon` for Toile's command registry
// (`ambre-spec-commands.md`, UI-THREAD-019).
//
// The registry carries an opaque string handle, so `ambre-commands` keeps an
// empty dependency table (UI-THREAD-027) and the icon set of one application
// stays out of the shared crate. This is the one place that maps a handle to
// the asset, and it is a rendering concern: the icon a command declares is
// still declared once, in `ui::commands`.
// ============================================================================

use ambre_commands::IconId;

/// The icon asset for `id`, or `None` for a handle no Toile command declares.
///
/// A missing handle draws no icon rather than failing the menu: an entry with
/// its label is still usable, an absent entry is not.
pub fn resolve(id: IconId) -> Option<icondata::Icon> {
    let icon = match id.0 {
        "content-cut" => icondata::MdiContentCut,
        "content-copy" => icondata::MdiContentCopy,
        "content-paste" => icondata::MdiContentPaste,
        "content-duplicate" => icondata::MdiContentDuplicate,
        "delete" => icondata::MdiDelete,
        "group" => icondata::MdiGroup,
        "ungroup" => icondata::MdiUngroup,
        "arrange-bring-to-front" => icondata::MdiArrangeBringToFront,
        "arrange-bring-forward" => icondata::MdiArrangeBringForward,
        "arrange-send-backward" => icondata::MdiArrangeSendBackward,
        "arrange-send-to-back" => icondata::MdiArrangeSendToBack,
        "lock" => icondata::MdiLock,
        "lock-open-variant" => icondata::MdiLockOpenVariant,
        "align-horizontal-left" => icondata::MdiAlignHorizontalLeft,
        "align-horizontal-center" => icondata::MdiAlignHorizontalCenter,
        "align-horizontal-right" => icondata::MdiAlignHorizontalRight,
        "align-vertical-top" => icondata::MdiAlignVerticalTop,
        "align-vertical-center" => icondata::MdiAlignVerticalCenter,
        "align-vertical-bottom" => icondata::MdiAlignVerticalBottom,
        "align-horizontal-distribute" => icondata::MdiAlignHorizontalDistribute,
        "align-vertical-distribute" => icondata::MdiAlignVerticalDistribute,
        "image-frame" => icondata::MdiImageFrame,
        "selection-drag" => icondata::MdiSelectionDrag,
        "select-all" => icondata::MdiSelectAll,
        "thumb-up" => icondata::MdiThumbUp,
        "heart" => icondata::MdiHeart,
        "check" => icondata::MdiCheck,
        "party-popper" => icondata::MdiPartyPopper,
        "close" => icondata::MdiClose,
        "plus" => icondata::MdiPlus,
        "minus" => icondata::MdiMinus,
        "fit-to-screen-outline" => icondata::MdiFitToScreenOutline,
        "selection-search" => icondata::MdiSelectionSearch,
        "cursor-default" => icondata::MdiCursorDefault,
        "vector-polyline" => icondata::MdiVectorPolyline,
        "shape-outline" => icondata::MdiShapeOutline,
        "vector-line" => icondata::MdiVectorLine,
        "draw" => icondata::MdiDraw,
        "note-outline" => icondata::MdiNoteOutline,
        "format-text" => icondata::MdiFormatText,
        "comment-outline" => icondata::MdiCommentOutline,
        "cursor-pointer" => icondata::MdiCursorPointer,
        "comment-multiple-outline" => icondata::MdiCommentMultipleOutline,
        "hand-back-right" => icondata::MdiHandBackRight,
        "view-carousel-outline" => icondata::MdiViewCarouselOutline,
        "file-import-outline" => icondata::MdiFileImportOutline,
        "file-export-outline" => icondata::MdiFileExportOutline,
        "file-pdf-box" => icondata::MdiFilePdfBox,
        _ => return None,
    };
    Some(icon)
}
