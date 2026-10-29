// ambre-trame/src/chrome/bars.rs
// ============================================================================
// The `FormatBar` formatting toolbar (the full chrome `Toolbar` slot).
// Bold/Italic/Undo/Redo/Merge go through the shared command path (`on_command`);
// the rich-formatting controls emit a `StylePatch` through `on_style`.
// ============================================================================

use ambre_bus::{DecimalsStep, StylePatch};
use ambre_chrome::commands::{CommandId, Locale};
use ambre_chrome::{Availability, FontSizeSelect, FormatToggle};
use ambre_core::{FontSize, Rgb};
use leptos::prelude::*;
use leptos_icons::Icon;

use super::color::{theme_bg_default, theme_fg_default};
use super::commands::{
    BOLD, COPY, CUT, EXPORT_CSV, FIND, IMPORT_CSV, ITALIC, MERGE, PASTE, REDO, SAVE_FILE, UNDO,
    UNMERGE,
};
use super::icons;
use crate::ui::commands as registry;

/// The label a control shows for `id`. Reads the registry and never a literal
/// (`UI-THREAD-019`), so the toolbar and the context menu name a command alike.
fn label(id: CommandId) -> String {
    registry::title(id, &Locale::En).unwrap_or_default()
}

/// The glyph `id` declares, resolved to its asset.
fn glyph(id: CommandId) -> Option<icondata::Icon> {
    registry::icon(id).and_then(icons::resolve)
}

/// The command the merge toggle sends: a merged cell splits, any other cell
/// merges the selection.
fn merge_command(merged: ambre_bus::Merge) -> CommandId {
    match merged {
        ambre_bus::Merge::Single => MERGE,
        ambre_bus::Merge::Merged => UNMERGE,
    }
}
use super::popovers::{
    BordersPopover, ColorPopover, FontPopover, HAlignPopover, NumberFormatPopover, VAlignPopover,
};
use super::style::{StyleState, Toggles};

/// The Trame format toolbar (the launcher mounts it inside the chrome `Toolbar`
/// slot). Bold/Italic/Undo/Redo invoke their `CommandId` through `on_command`,
/// the shared command path. The rich-formatting controls (underline, font, size,
/// colours, alignment, number format, borders, wrap) emit a [`StylePatch`]
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
        // Clipboard (`trame-clipboard`): the launcher drives the browser clipboard
        // from these intents (copy/cut read the viewport cache, paste reads the OS
        // clipboard). Ctrl+C/X/V are serviced directly by the DOM clipboard
        // listeners, not through these buttons.
        //
        // `mousedown` preventDefault keeps DOM focus on the canvas text pipe. A
        // button stealing focus broke the next grid click, which needed a second
        // click to reselect. It also made `navigator.clipboard.read()` throw
        // "Document is not focused", so the Paste button did nothing.
        <button
            aria-label=label(COPY)
            on:mousedown=move |ev| ev.prevent_default()
            on:click=move |_| on_command.run(COPY)
        >
            {glyph(COPY).map(|ic| view! { <Icon icon=ic /> })}
        </button>
        <button
            aria-label=label(CUT)
            on:mousedown=move |ev| ev.prevent_default()
            on:click=move |_| on_command.run(CUT)
        >
            {glyph(CUT).map(|ic| view! { <Icon icon=ic /> })}
        </button>
        <button
            aria-label=label(PASTE)
            on:mousedown=move |ev| ev.prevent_default()
            on:click=move |_| on_command.run(PASTE)
        >
            {glyph(PASTE).map(|ic| view! { <Icon icon=ic /> })}
        </button>
        <span class="sep"></span>

        // Find & Replace (`trame-find-replace`): opens the DOM panel in find
        // mode. The launcher intercepts FIND (UI-only) like FILTER_COLUMN.
        <button
            aria-label=label(FIND)
            on:mousedown=move |ev| ev.prevent_default()
            on:click=move |_| on_command.run(FIND)
        >
            {glyph(FIND).map(|ic| view! { <Icon icon=ic /> })}
        </button>
        <span class="sep"></span>

        // Interop (`trame-import-export`): import a CSV into a new sheet, or
        // export the active sheet to CSV. UI-only intents the launcher services
        // (IMPORT_CSV opens a file picker, EXPORT_CSV triggers the download).
        <button
            aria-label=label(IMPORT_CSV)
            on:mousedown=move |ev| ev.prevent_default()
            on:click=move |_| on_command.run(IMPORT_CSV)
        >
            {glyph(IMPORT_CSV).map(|ic| view! { <Icon icon=ic /> })}
        </button>
        <button
            aria-label=label(EXPORT_CSV)
            on:mousedown=move |ev| ev.prevent_default()
            on:click=move |_| on_command.run(EXPORT_CSV)
        >
            {glyph(EXPORT_CSV).map(|ic| view! { <Icon icon=ic /> })}
        </button>
        // The native file, next to the two lossy paths and distinguished by its
        // own label: it keeps everything and it is not encrypted
        // (`DOCFILE-001`, `DOCFILE-009`).
        <button
            aria-label=label(SAVE_FILE)
            on:mousedown=move |ev| ev.prevent_default()
            on:click=move |_| on_command.run(SAVE_FILE)
        >
            {glyph(SAVE_FILE).map(|ic| view! { <Icon icon=ic /> })}
        </button>
        <span class="sep"></span>

        // Bold / Italic / Underline / Strikethrough -- shared toggles
        // (`ambre_chrome::FormatToggle`), same cluster Toile's inspector / selection
        // bar use. Bold/Italic route through the shared command path; underline /
        // strikethrough emit a `StylePatch` (they have no `CommandId`).
        <FormatToggle
            label=label(BOLD)
            icon=glyph(BOLD)
            pressed=Signal::derive(move || ambre_chrome::Pressed::from_bool(style.bold.get().as_bool()))
            on_toggle=Callback::new(move |()| on_command.run(BOLD))
        />
        <FormatToggle
            label=label(ITALIC)
            icon=glyph(ITALIC)
            pressed=Signal::derive(move || ambre_chrome::Pressed::from_bool(style.italic.get().as_bool()))
            on_toggle=Callback::new(move |()| on_command.run(ITALIC))
        />
        <FormatToggle
            label="Underline"
            icon=icondata::BsTypeUnderline
            pressed=Signal::derive(move || ambre_chrome::Pressed::from_bool(style.underline.get().as_bool()))
            on_toggle=Callback::new(move |()| on_style.run(StylePatch {
                underline: Some(ambre_bus::Underline::from_bool(
                    !style.underline.get_untracked().as_bool(),
                )),
                ..Default::default()
            }))
        />
        <FormatToggle
            label="Strikethrough"
            icon=icondata::BsTypeStrikethrough
            pressed=Signal::derive(move || ambre_chrome::Pressed::from_bool(style.strikethrough.get().as_bool()))
            on_toggle=Callback::new(move |()| on_style.run(StylePatch {
                strikethrough: Some(ambre_bus::Strikethrough::from_bool(
                    !style.strikethrough.get_untracked().as_bool(),
                )),
                ..Default::default()
            }))
        />
        <span class="sep"></span>

        // Font family (custom popover: each family previewed in its own
        // typeface) + size (shared `ambre_chrome::FontSizeSelect`). "Default"
        // is the body default the cell follows when it carries no explicit
        // size.
        <FontPopover style=style on_style=on_style />
        <FontSizeSelect
            current=Signal::derive(move || style.size.get())
            on_pick=Callback::new(move |size: Option<FontSize>| {
                on_style.run(StylePatch { size: Some(size), ..Default::default() });
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
            on_pick=Callback::new(move |fg: Option<Rgb>| {
                on_style.run(StylePatch { fg: Some(fg), ..Default::default() })
            })
        />
        // Fill colour: composed picker (palette + "No fill" reset + custom).
        <ColorPopover
            title="Fill colour"
            icon=icondata::BiColorFillSolid
            current=style.bg
            default=Signal::derive(move || theme_bg_default(theme.get()))
            reset_label="No fill (transparent)"
            on_pick=Callback::new(move |bg: Option<Rgb>| {
                on_style.run(StylePatch { bg: Some(bg), ..Default::default() })
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
            on:click=move |_| on_style.run(StylePatch { decimals_step: Some(DecimalsStep::FEWER), ..Default::default() })
        >
            <Icon icon=icondata::LuDecimalsArrowLeft />
        </button>
        <button
            aria-label="Increase decimals"
            disabled=move || !style.format.get().has_decimals()
            on:mousedown=move |ev| ev.prevent_default()
            on:click=move |_| on_style.run(StylePatch { decimals_step: Some(DecimalsStep::MORE), ..Default::default() })
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
            aria-pressed=move || ambre_chrome::Pressed::from_bool(style.wrap.get().as_bool()).aria()
            on:mousedown=move |ev| ev.prevent_default()
            on:click=move |_| on_style.run(StylePatch {
                wrap: Some(ambre_bus::WordBreak::from_bool(!style.wrap.get_untracked().as_bool())),
                ..Default::default()
            })
        >
            <Icon icon=icondata::BsTextWrap />
        </button>

        // Merge / unmerge cells: a stateful toggle. Pressed when the active cell
        // is part of a merge, in which case clicking splits it (UNMERGE). On a
        // plain selection it merges the range (MERGE). The Logic thread guards
        // both (single-cell merge is a no-op, non-empty covered cell rejects).
        <button
            // The label names the command the click will send, so a merged cell
            // announces "Unmerge cells" instead of the action it will not do.
            aria-label=move || label(merge_command(style.merged.get()))
            aria-pressed=move || ambre_chrome::Pressed::from_bool(style.merged.get().as_bool()).aria()
            on:mousedown=move |ev| ev.prevent_default()
            on:click=move |_| on_command.run(merge_command(style.merged.get_untracked()))
        >
            // One glyph for both states: the button is a toggle, so it must not
            // change shape under the pointer.
            {glyph(MERGE).map(|ic| view! { <Icon icon=ic /> })}
        </button>
        <span class="sep"></span>

        <button
            aria-label=label(UNDO)
            disabled=move || toggles.undo_enabled.get() == Availability::Disabled
            on:mousedown=move |ev| ev.prevent_default()
            on:click=move |_| on_command.run(UNDO)
        >
            {glyph(UNDO).map(|ic| view! { <Icon icon=ic /> })}
        </button>
        <button
            aria-label=label(REDO)
            disabled=move || toggles.redo_enabled.get() == Availability::Disabled
            on:mousedown=move |ev| ev.prevent_default()
            on:click=move |_| on_command.run(REDO)
        >
            {glyph(REDO).map(|ic| view! { <Icon icon=ic /> })}
        </button>
    }
}
