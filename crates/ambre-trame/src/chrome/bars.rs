// ambre-trame/src/chrome/bars.rs
// ============================================================================
// The `FormatBar` formatting toolbar (the full chrome `Toolbar` slot).
// Bold/Italic/Undo/Redo/Merge go through the shared command path (`on_command`);
// the rich-formatting controls emit a `StylePatch` through `on_style`.
// ============================================================================

use ambre_bus::StylePatch;
use ambre_chrome::commands::CommandId;
use ambre_chrome::{FontSizeSelect, FormatToggle};
use leptos::prelude::*;
use leptos_icons::Icon;

use super::color::{theme_bg_default, theme_fg_default};
use super::commands::{
    BOLD, COPY, CUT, EXPORT_CSV, FIND, IMPORT_CSV, ITALIC, MERGE, PASTE, REDO, UNDO, UNMERGE,
};
use super::popovers::{
    BordersPopover, ColorPopover, FontPopover, HAlignPopover, NumberFormatPopover, VAlignPopover,
};
use super::style::{StyleState, Toggles};

/// The Trame format toolbar (the launcher mounts it inside the chrome `Toolbar`
/// slot). Bold/Italic/Undo/Redo invoke their `CommandId` through `on_command`
/// (the shared command path); the rich-formatting controls (underline, font,
/// size, colours, alignment, number format, borders, wrap) emit a [`StylePatch`]
/// through `on_style`, routed as `SetCellStyle` (`trame-cell-styles`).
#[component]
pub fn FormatBar(
    toggles: Toggles,
    style: StyleState,
    /// The active light/dark theme, so the colour pickers' "Automatic" /
    /// "Transparent" defaults match what the canvas actually renders.
    theme: RwSignal<ambre_canvas::Theme>,
    on_command: Callback<CommandId>,
    on_style: Callback<StylePatch>,
) -> impl IntoView {
    view! {
        // Clipboard (`trame-clipboard`): the launcher drives the browser
        // clipboard from these intents (copy/cut read the viewport cache, paste
        // reads the OS clipboard). Ctrl+C/X/V are serviced directly by the
        // DOM clipboard listeners, not through these buttons.
        //
        // `mousedown` preventDefault keeps DOM focus on the canvas text pipe: a
        // button stealing focus both broke the next grid click (needed a second
        // click to reselect) and made `navigator.clipboard.read()` throw "Document
        // is not focused", so the Paste button did nothing.
        <button
            aria-label="Copy"
            on:mousedown=move |ev| ev.prevent_default()
            on:click=move |_| on_command.run(COPY)
        >
            <Icon icon=icondata::MdiContentCopy />
        </button>
        <button
            aria-label="Cut"
            on:mousedown=move |ev| ev.prevent_default()
            on:click=move |_| on_command.run(CUT)
        >
            <Icon icon=icondata::MdiContentCut />
        </button>
        <button
            aria-label="Paste"
            on:mousedown=move |ev| ev.prevent_default()
            on:click=move |_| on_command.run(PASTE)
        >
            <Icon icon=icondata::MdiContentPaste />
        </button>
        <span class="sep"></span>

        // Find & Replace (`trame-find-replace`): opens the DOM panel in find
        // mode. The launcher intercepts FIND (UI-only) like FILTER_COLUMN.
        <button
            aria-label="Find and replace"
            on:mousedown=move |ev| ev.prevent_default()
            on:click=move |_| on_command.run(FIND)
        >
            <Icon icon=icondata::AiSearchOutlined />
        </button>
        <span class="sep"></span>

        // Interop (`trame-import-export`): import a CSV into a new sheet, or
        // export the active sheet to CSV. UI-only intents the launcher services
        // (IMPORT_CSV opens a file picker, EXPORT_CSV triggers the download).
        <button
            aria-label="Import CSV"
            on:mousedown=move |ev| ev.prevent_default()
            on:click=move |_| on_command.run(IMPORT_CSV)
        >
            <Icon icon=icondata::BiImportRegular />
        </button>
        <button
            aria-label="Export CSV"
            on:mousedown=move |ev| ev.prevent_default()
            on:click=move |_| on_command.run(EXPORT_CSV)
        >
            <Icon icon=icondata::BiExportRegular />
        </button>
        <span class="sep"></span>

        // Bold / Italic / Underline / Strikethrough -- shared toggles
        // (`ambre_chrome::FormatToggle`), same cluster Toile's inspector / selection
        // bar use. Bold/Italic route through the shared command path; underline /
        // strikethrough emit a `StylePatch` (they have no `CommandId`).
        <FormatToggle
            label="Bold"
            icon=icondata::BsTypeBold
            pressed=style.bold.into()
            on_toggle=Callback::new(move |()| on_command.run(BOLD))
        />
        <FormatToggle
            label="Italic"
            icon=icondata::BsTypeItalic
            pressed=style.italic.into()
            on_toggle=Callback::new(move |()| on_command.run(ITALIC))
        />
        <FormatToggle
            label="Underline"
            icon=icondata::BsTypeUnderline
            pressed=style.underline.into()
            on_toggle=Callback::new(move |()| on_style.run(StylePatch {
                underline: Some(!style.underline.get_untracked()),
                ..Default::default()
            }))
        />
        <FormatToggle
            label="Strikethrough"
            icon=icondata::BsTypeStrikethrough
            pressed=style.strikethrough.into()
            on_toggle=Callback::new(move |()| on_style.run(StylePatch {
                strikethrough: Some(!style.strikethrough.get_untracked()),
                ..Default::default()
            }))
        />
        <span class="sep"></span>

        // Font family (custom popover: each family previewed in its own
        // typeface) + size (shared `ambre_chrome::FontSizeSelect`; "Default" =
        // the body default the cell follows when it carries no explicit size).
        <FontPopover style=style on_style=on_style />
        <FontSizeSelect
            current=style.size.into()
            on_pick=Callback::new(move |p: Option<u16>| {
                on_style.run(StylePatch { size: Some(p), ..Default::default() });
            })
            auto_label="Default"
        />
        <span class="sep"></span>

        // Text colour: composed picker (palette + "Automatic" reset + custom).
        <ColorPopover
            title="Text colour"
            icon=icondata::BiFontColorRegular
            current=style.fg
            default=Signal::derive(move || theme_fg_default(theme.get()))
            reset_label="Automatic"
            on_pick=Callback::new(move |c| {
                on_style.run(StylePatch { fg: Some(c), ..Default::default() })
            })
        />
        // Fill colour: composed picker (palette + "No fill" reset + custom).
        <ColorPopover
            title="Fill colour"
            icon=icondata::BiColorFillSolid
            current=style.bg
            default=Signal::derive(move || theme_bg_default(theme.get()))
            reset_label="No fill (transparent)"
            on_pick=Callback::new(move |c| {
                on_style.run(StylePatch { bg: Some(c), ..Default::default() })
            })
        />
        <span class="sep"></span>

        // Horizontal alignment: a single button (the active alignment's icon)
        // opening a context-menu popover listing the three alignments.
        <HAlignPopover style=style on_style=on_style />
        // Vertical alignment: same composed popover, vertical variants.
        <VAlignPopover style=style on_style=on_style />
        <span class="sep"></span>

        // Number format: a single button (the "123" icon) opening a context-menu
        // popover that lists the families, with a check on the active one.
        <NumberFormatPopover style=style on_style=on_style />
        <button
            aria-label="Decrease decimals"
            disabled=move || !style.format.get().has_decimals()
            on:mousedown=move |ev| ev.prevent_default()
            on:click=move |_| on_style.run(StylePatch { decimals: Some(Some(0)), ..Default::default() })
        >
            <Icon icon=icondata::LuDecimalsArrowLeft />
        </button>
        <button
            aria-label="Increase decimals"
            disabled=move || !style.format.get().has_decimals()
            on:mousedown=move |ev| ev.prevent_default()
            on:click=move |_| on_style.run(StylePatch { decimals: Some(Some(2)), ..Default::default() })
        >
            <Icon icon=icondata::LuDecimalsArrowRight />
        </button>
        <span class="sep"></span>

        // Borders: a single button opening a composed popover (pen + presets).
        <BordersPopover style=style on_style=on_style />
        <span class="sep"></span>

        // Wrap text.
        <button
            aria-label="Wrap text"
            aria-pressed=move || style.wrap.get().then_some("true")
            on:mousedown=move |ev| ev.prevent_default()
            on:click=move |_| on_style.run(StylePatch {
                wrap: Some(!style.wrap.get_untracked()),
                ..Default::default()
            })
        >
            <Icon icon=icondata::BsTextWrap />
        </button>

        // Merge / unmerge cells: a stateful toggle. Pressed when the active cell
        // is part of a merge, in which case clicking splits it (UNMERGE); on a
        // plain selection it merges the range (MERGE). The Logic thread guards
        // both (single-cell merge is a no-op, non-empty covered cell rejects).
        <button
            aria-label="Merge cells"
            aria-pressed=move || style.merged.get().then_some("true")
            on:mousedown=move |ev| ev.prevent_default()
            on:click=move |_| {
                on_command.run(if style.merged.get_untracked() { UNMERGE } else { MERGE })
            }
        >
            <Icon icon=icondata::AiMergeCellsOutlined />
        </button>
        <span class="sep"></span>

        <button
            aria-label="Undo"
            disabled=move || !toggles.undo_enabled.get()
            on:mousedown=move |ev| ev.prevent_default()
            on:click=move |_| on_command.run(UNDO)
        >
            <Icon icon=icondata::BiUndoRegular />
        </button>
        <button
            aria-label="Redo"
            disabled=move || !toggles.redo_enabled.get()
            on:mousedown=move |ev| ev.prevent_default()
            on:click=move |_| on_command.run(REDO)
        >
            <Icon icon=icondata::BiRedoRegular />
        </button>
    }
}
