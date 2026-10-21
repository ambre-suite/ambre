// ambre-toile/src/chrome/palette.rs
// ============================================================================ The
// tool palette / primary toolbar (`toile-tools`), wasm-only. A floating vertical
// toolbar with one button per Toile tool (Select, Pen, Shape, Connector, Draw,
// Sticky, Text, Hand). Exactly one button reads active, reflecting the current
// `Tool`; clicking switches the tool.
//
// The Sticky button is a split affordance. Clicking it opens a small colour popover
// of pastel swatches, and picking a swatch arms `Tool::Sticky(colour)`. The next
// click then places notes in that colour (folded into `CreateElement.fill`,
// `toile-elements`). This mirrors the Miro / Figma-whiteboard sticky flow.
//
// Styling is SHARED with the Trame command bar. The container carries the
// `.ambre-commandbar` class. So buttons, hover, the `[aria-pressed="true"]` active
// fill, `.sep` separators and the `aria-label` tooltip all come from the launcher
// CSS, pixel-identical to Trame. An `.ambre-tool-palette` modifier only re-docks it
// as a floating vertical bar. Icons come from `icondata` rendered by `leptos-icons`,
// exactly like the Trame `FormatBar` and the Toile context menu (`chrome::menu`).
//
// The active tool is UI-only, canvas-local state (`toile-tools`): it is NOT a
// `ToileCommand`. The palette writes a shared `RwSignal<Tool>` that the canvas
// `ToileView` applies each frame; the keyboard chords (`ui::palette::bind_tool_keys`)
// write the very same signal. The pure command surface (ids, key chords, the id ->
// Tool map, the swatch set) lives in the always-compiled, host-tested `ui::palette`.
// ============================================================================

use ambre_chrome::commands::{CommandId, Locale};
use leptos::html;
use leptos::prelude::*;
use leptos_icons::Icon;
use wasm_bindgen::JsCast;

use crate::ui::Tool;
use crate::ui::commands as registry;
use crate::ui::palette::{
    STICKY_SWATCHES, TOOL_COMMENT, TOOL_CONNECTOR, TOOL_DRAW, TOOL_HAND, TOOL_LASER, TOOL_PEN,
    TOOL_SELECT, TOOL_SHAPE, TOOL_STICKY, TOOL_TEXT, tool_for_command,
};

use super::icons;

/// One palette button. `aria-pressed` compares the active tool by variant
/// (`mem::discriminant`). So any `Shape(kind)` reads pressed while the Shape tool
/// is active, regardless of the current kind sub-option (`toile-tools`). The
/// `mousedown` `prevent_default` keeps DOM focus on the canvas text pipe. A
/// button stealing focus broke the next canvas click and the clipboard, the same
/// hazard documented on the Trame `FormatBar`.
///
/// `command` names the tool. Its tooltip and its glyph come from the registry
/// (`UI-THREAD-019`), and `tool_for_command` says which tool the click arms.
/// The button, the keyboard chord and the command palette then all agree.
fn tool_button(active: RwSignal<Tool>, command: CommandId) -> impl IntoView {
    let label = registry::title(command, &Locale::En).unwrap_or_default();
    let icon = registry::icon(command).and_then(icons::resolve);
    // An id absent from the tool map arms nothing rather than failing the bar.
    let tool = tool_for_command(command).unwrap_or(Tool::Select);
    let is_active = move || std::mem::discriminant(&active.get()) == std::mem::discriminant(&tool);
    view! {
        <button
            aria-label=label
            aria-pressed=move || ambre_chrome::aria_bool(is_active())
            on:mousedown=move |ev| ev.prevent_default()
            on:click=move |_| active.set(tool)
        >
            {icon.map(|ic| view! { <Icon icon=ic /> })}
        </button>
    }
}

/// The Sticky button + its colour popover (`toile-tools`). Clicking the button
/// opens the popover; picking a pastel swatch arms `Tool::Sticky(colour)` (which
/// remembers the colour for the next note) and closes it. `aria-pressed` reads
/// active for any `Sticky(_)` regardless of colour. The popover dismisses on any
/// pointer-down outside the control, such as a click on the canvas or on another
/// tool.
fn sticky_button(active: RwSignal<Tool>) -> impl IntoView {
    let open = RwSignal::new(false);
    // The picked colour tints the button's strip so the palette shows what the
    // next note will be. Seeded from the current tool colour when Sticky is
    // armed, else the default (the first swatch).
    let strip_color = move || match active.get() {
        Tool::Sticky(c) => c,
        _ => STICKY_SWATCHES.first().copied().unwrap_or_default(),
    };
    let is_active = move || matches!(active.get(), Tool::Sticky(_));

    // Close on any pointer-down outside the control (mirrors `ambre_chrome::
    // ColorPopover`): the wrapper node lets us tell inside from outside.
    let root_ref = NodeRef::<html::Span>::new();
    let handle = window_event_listener(leptos::ev::pointerdown, move |ev| {
        if !open.get_untracked() {
            return;
        }
        let inside = root_ref
            .get_untracked()
            .zip(ev.target())
            .and_then(|(root, target)| target.dyn_into::<web_sys::Node>().ok().map(|n| (root, n)))
            .is_some_and(|(root, node)| root.contains(Some(&node)));
        if !inside {
            open.set(false);
        }
    });
    on_cleanup(move || handle.remove());

    view! {
        <span class="ambre-color" node_ref=root_ref>
            <button
                aria-label=registry::title(TOOL_STICKY, &Locale::En).unwrap_or_default()
                aria-haspopup="true"
                aria-expanded=move || ambre_chrome::aria_bool(open.get())
                aria-pressed=move || ambre_chrome::aria_bool(is_active())
                on:mousedown=move |ev| ev.prevent_default()
                on:click=move |_| open.update(|o| *o = !*o)
            >
                <Icon icon=icondata::MdiNoteOutline />
                <span
                    class="swatch-strip"
                    style:background-color=move || format!("#{:06x}", strip_color().rgb().bits())
                ></span>
            </button>
            <Show when=move || open.get() fallback=|| ()>
                <div class="ambre-color-popover">
                    <div class="swatches">
                        {STICKY_SWATCHES
                            .iter()
                            .map(|&argb| {
                                view! {
                                    <button
                                        aria-label=format!("#{:06x}", argb.rgb().bits())
                                        style:background-color=format!(
                                            "#{:06x}",
                                            argb.rgb().bits(),
                                        )
                                        on:mousedown=move |ev| ev.prevent_default()
                                        on:click=move |_| {
                                            active.set(Tool::Sticky(argb));
                                            open.set(false);
                                        }
                                    ></button>
                                }
                            })
                            .collect_view()}
                    </div>
                </div>
            </Show>
        </span>
    }
}

/// The Toile tool palette. `active` is the shared active-tool signal, the same
/// signal the keyboard chords write and the canvas view reads. The buttons write
/// it on click, and read it for their pressed state.
#[component]
pub fn ToolPalette(active: RwSignal<Tool>) -> impl IntoView {
    view! {
        <div class="ambre-commandbar ambre-tool-palette" role="toolbar" aria-label="Tools">
            {tool_button(active, TOOL_SELECT)}
            <span class="sep"></span>
            {tool_button(active, TOOL_PEN)}
            // The Shape tool defaults to a rectangle; the kind sub-picker is a
            // later palette signal folded into `CreateElement` (`toile-tools`).
            {tool_button(active, TOOL_SHAPE)}
            {tool_button(active, TOOL_CONNECTOR)}
            {tool_button(active, TOOL_DRAW)}
            {sticky_button(active)}
            {tool_button(active, TOOL_TEXT)}
            {tool_button(active, TOOL_COMMENT)}
            {tool_button(active, TOOL_LASER)}
            <span class="sep"></span>
            {tool_button(active, TOOL_HAND)}
        </div>
    }
}
