// ambre-trame/src/chrome/icons.rs
// ============================================================================
// `StaticKey<Icons>` -> `icondata::Icon` for Trame's command registry
// (`ambre-spec-commands.md`, UI-THREAD-019).
//
// The registry carries an opaque string handle. So `ambre-commands` keeps an
// empty dependency table (UI-THREAD-027), and the icon set of one application
// stays out of the shared crate. This is the one place that maps a handle to the
// asset, and it is a rendering concern. The icon a command declares is still
// declared once, in `ui::commands`.
// ============================================================================

use ambre_commands::{Icons, StaticKey};

/// The icon asset for `id`, or `None` for a handle no Trame command declares.
///
/// A missing handle draws no icon rather than failing the menu: an entry with
/// its label is still usable, an absent entry is not.
pub fn resolve(id: StaticKey<Icons>) -> Option<icondata::Icon> {
    let icon = match id.as_str() {
        "table-row-plus-before" => icondata::MdiTableRowPlusBefore,
        "table-row-plus-after" => icondata::MdiTableRowPlusAfter,
        "table-column-plus-before" => icondata::MdiTableColumnPlusBefore,
        "table-column-plus-after" => icondata::MdiTableColumnPlusAfter,
        "table-row-remove" => icondata::MdiTableRowRemove,
        "table-column-remove" => icondata::MdiTableColumnRemove,
        "table-merge-cells" => icondata::MdiTableMergeCells,
        "table-split-cell" => icondata::MdiTableSplitCell,
        "sort-ascending" => icondata::MdiSortAscending,
        "sort-descending" => icondata::MdiSortDescending,
        "sort" => icondata::MdiSort,
        "sort-variant" => icondata::MdiSortVariant,
        "filter" => icondata::MdiFilter,
        "filter-off" => icondata::MdiFilterOff,
        "content-cut" => icondata::MdiContentCut,
        "content-copy" => icondata::MdiContentCopy,
        "content-paste" => icondata::MdiContentPaste,
        // Toolbar-only glyphs, from families the grid menu does not use.
        "format-bold" => icondata::BsTypeBold,
        "format-italic" => icondata::BsTypeItalic,
        "undo" => icondata::BiUndoRegular,
        "redo" => icondata::BiRedoRegular,
        "search" => icondata::AiSearchOutlined,
        "import" => icondata::BiImportRegular,
        "export" => icondata::BiExportRegular,
        _ => return None,
    };
    Some(icon)
}
