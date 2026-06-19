// ambre-trame/src/chrome/bars.rs
// ============================================================================
// The `FormatBar` formatting toolbar (the full chrome `Toolbar` slot).
// Bold/Italic/Undo/Redo/Merge go through the shared command path (`on_command`);
// the rich-formatting controls emit a `StylePatch` through `on_style`.
// ============================================================================

use ambre_bus::{HAlign, StylePatch, VAlign};
use ambre_chrome::commands::CommandId;
use leptos::prelude::*;
use leptos_icons::Icon;

use super::color::{theme_bg_default, theme_fg_default};
use super::commands::{BOLD, ITALIC, MERGE, REDO, UNDO, UNMERGE};
use super::popovers::{BordersPopover, ColorPopover, NumberFormatPopover};
use super::style::{StyleState, Toggles};

/// The font families offered by the toolbar `<select>` (the renderer falls back
/// to its default for any family it does not have loaded).
const FONTS: &[&str] = &["Default", "Sans", "Serif", "Monospace"];

/// The font sizes offered by the toolbar `<select>` (points).
const SIZES: &[u16] = &[8, 9, 10, 11, 12, 14, 16, 18, 24, 36];

/// The grid's default body size (matches `render::BODY_SIZE`), shown in the size
/// picker when a cell has no explicit size.
const DEFAULT_SIZE: u16 = 12;

/// The Trame format toolbar (the launcher mounts it inside the chrome `Toolbar`
/// slot). Bold/Italic/Undo/Redo invoke their `CommandId` through `on_command`
/// (the shared command path); the rich-formatting controls (underline, font,
/// size, colours, alignment, number format, borders, wrap) emit a [`StylePatch`]
/// through `on_style`, which the launcher routes as `SetCellStyle` (sec.5bis).
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
        <button
            title="Bold"
            aria-pressed=move || style.bold.get().then_some("true")
            on:click=move |_| on_command.run(BOLD)
        >
            <Icon icon=icondata::BsTypeBold />
        </button>
        <button
            title="Italic"
            aria-pressed=move || style.italic.get().then_some("true")
            on:click=move |_| on_command.run(ITALIC)
        >
            <Icon icon=icondata::BsTypeItalic />
        </button>
        <button
            title="Underline"
            aria-pressed=move || style.underline.get().then_some("true")
            on:click=move |_| on_style.run(StylePatch {
                underline: Some(!style.underline.get_untracked()),
                ..Default::default()
            })
        >
            <Icon icon=icondata::BsTypeUnderline />
        </button>
        <button
            title="Strikethrough"
            aria-pressed=move || style.strikethrough.get().then_some("true")
            on:click=move |_| on_style.run(StylePatch {
                strikethrough: Some(!style.strikethrough.get_untracked()),
                ..Default::default()
            })
        >
            <Icon icon=icondata::BsTypeStrikethrough />
        </button>
        <span class="sep"></span>

        // Font family + size.
        <select
            title="Font"
            prop:value=move || style.font.get().unwrap_or_else(|| "Default".to_owned())
            on:change=move |ev| {
                let v = event_target_value(&ev);
                let font = (v != "Default").then_some(v);
                on_style.run(StylePatch { font: Some(font), ..Default::default() });
            }
        >
            {FONTS
                .iter()
                .map(|f| view! { <option value=*f>{f.to_string()}</option> })
                .collect_view()}
        </select>
        <select
            title="Font size"
            prop:value=move || {
                style.size.get().map(|s| s.to_string()).unwrap_or_else(|| DEFAULT_SIZE.to_string())
            }
            on:change=move |ev| {
                let sz = event_target_value(&ev).parse::<u16>().ok();
                on_style.run(StylePatch { size: Some(sz), ..Default::default() });
            }
        >
            {SIZES
                .iter()
                .map(|&s| view! { <option value=s.to_string()>{s.to_string()}</option> })
                .collect_view()}
        </select>
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

        // Horizontal alignment.
        <button
            title="Align left"
            aria-pressed=move || (style.h_align.get() == HAlign::Left).then_some("true")
            on:click=move |_| on_style.run(StylePatch { h_align: Some(HAlign::Left), ..Default::default() })
        >
            <Icon icon=icondata::BiAlignLeftRegular />
        </button>
        <button
            title="Align center"
            aria-pressed=move || (style.h_align.get() == HAlign::Center).then_some("true")
            on:click=move |_| on_style.run(StylePatch { h_align: Some(HAlign::Center), ..Default::default() })
        >
            <Icon icon=icondata::BiAlignMiddleRegular />
        </button>
        <button
            title="Align right"
            aria-pressed=move || (style.h_align.get() == HAlign::Right).then_some("true")
            on:click=move |_| on_style.run(StylePatch { h_align: Some(HAlign::Right), ..Default::default() })
        >
            <Icon icon=icondata::BiAlignRightRegular />
        </button>
        <span class="sep"></span>

        // Vertical alignment.
        <button
            title="Align top"
            aria-pressed=move || (style.v_align.get() == VAlign::Top).then_some("true")
            on:click=move |_| on_style.run(StylePatch { v_align: Some(VAlign::Top), ..Default::default() })
        >
            <Icon icon=icondata::AiVerticalAlignTopOutlined />
        </button>
        <button
            title="Align middle"
            aria-pressed=move || (style.v_align.get() == VAlign::Middle).then_some("true")
            on:click=move |_| on_style.run(StylePatch { v_align: Some(VAlign::Middle), ..Default::default() })
        >
            <Icon icon=icondata::AiVerticalAlignMiddleOutlined />
        </button>
        <button
            title="Align bottom"
            aria-pressed=move || (style.v_align.get() == VAlign::Bottom).then_some("true")
            on:click=move |_| on_style.run(StylePatch { v_align: Some(VAlign::Bottom), ..Default::default() })
        >
            <Icon icon=icondata::AiVerticalAlignBottomOutlined />
        </button>
        <span class="sep"></span>

        // Number format: a single button (the "123" icon) opening a context-menu
        // popover that lists the families, with a check on the active one.
        <NumberFormatPopover style=style on_style=on_style />
        <button
            title="Decrease decimals"
            on:click=move |_| on_style.run(StylePatch { decimals: Some(Some(0)), ..Default::default() })
        >
            <Icon icon=icondata::LuDecimalsArrowLeft />
        </button>
        <button
            title="Increase decimals"
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
            title="Wrap text"
            aria-pressed=move || style.wrap.get().then_some("true")
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
            title="Merge cells"
            aria-pressed=move || style.merged.get().then_some("true")
            on:click=move |_| {
                on_command.run(if style.merged.get_untracked() { UNMERGE } else { MERGE })
            }
        >
            <Icon icon=icondata::AiMergeCellsOutlined />
        </button>
        <span class="sep"></span>

        <button
            title="Undo"
            disabled=move || !toggles.undo_enabled.get()
            on:click=move |_| on_command.run(UNDO)
        >
            <Icon icon=icondata::BiUndoRegular />
        </button>
        <button
            title="Redo"
            disabled=move || !toggles.redo_enabled.get()
            on:click=move |_| on_command.run(REDO)
        >
            <Icon icon=icondata::BiRedoRegular />
        </button>
    }
}
