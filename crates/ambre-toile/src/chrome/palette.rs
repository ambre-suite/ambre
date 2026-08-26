// ambre-toile/src/chrome/palette.rs
// ============================================================================
// The tool palette / primary toolbar (`toile-tools`), wasm-only. A
// floating vertical toolbar with one button per Toile tool (Select, Pen, Shape,
// Connector, Draw, Sticky, Text, Hand). Exactly one button reads active,
// reflecting the current `Tool`; clicking switches the tool.
//
// The Sticky button is a split affordance: clicking it opens a small colour
// popover of pastel swatches; picking a swatch arms `Tool::Sticky(colour)` so
// the next click places notes in that colour (folded into `CreateElement.fill`,
// `toile-elements`). This mirrors the Miro / Figma-whiteboard sticky flow.
//
// Styling is SHARED with the Trame command bar: the container carries the
// `.ambre-commandbar` class (so buttons, hover, the `[aria-pressed="true"]`
// active fill, `.sep` separators and the `aria-label` tooltip all come from the
// launcher CSS, pixel-identical to Trame) plus an `.ambre-tool-palette` modifier
// that only re-docks it as a floating vertical bar. Icons come from `icondata`
// rendered by `leptos-icons`, exactly like the Trame `FormatBar` and the Toile
// context menu (`chrome::menu`).
//
// The active tool is UI-only, canvas-local state (`toile-tools`): it is NOT a
// `ToileCommand`. The palette writes a shared `RwSignal<Tool>` that the canvas
// `ToileView` applies each frame; the keyboard chords (`ui::palette::bind_tool_keys`)
// write the very same signal. The pure command surface (ids, key chords, the
// id -> Tool map, the swatch set) lives in the always-compiled, host-tested
// `ui::palette`.
// ============================================================================

use ambre_bus::app::toile::ShapeKind;
use leptos::html;
use leptos::prelude::*;
use leptos_icons::Icon;
use wasm_bindgen::JsCast;

use crate::ui::Tool;
use crate::ui::palette::STICKY_SWATCHES;

/// One palette button. `aria-pressed` compares the active tool by variant
/// (`mem::discriminant`), so any `Shape(kind)` reads pressed while the Shape tool
/// is active regardless of the current kind sub-option (`toile-tools`). The
/// `mousedown` `prevent_default` keeps DOM focus on the canvas text pipe -- a
/// button stealing focus broke the next canvas click and the clipboard (the same
/// hazard documented on the Trame `FormatBar`).
fn tool_button(
    active: RwSignal<Tool>,
    tool: Tool,
    label: &'static str,
    icon: icondata::Icon,
) -> impl IntoView {
    let is_active = move || std::mem::discriminant(&active.get()) == std::mem::discriminant(&tool);
    view! {
        <button
            aria-label=label
            aria-pressed=move || is_active().then_some("true")
            on:mousedown=move |ev| ev.prevent_default()
            on:click=move |_| active.set(tool)
        >
            <Icon icon=icon />
        </button>
    }
}

/// The Sticky button + its colour popover (`toile-tools`). Clicking the button
/// opens the popover; picking a pastel swatch arms `Tool::Sticky(colour)` (which
/// remembers the colour for the next note) and closes it. `aria-pressed` reads
/// active for any `Sticky(_)` regardless of colour. The popover dismisses on any
/// pointer-down outside the control (a click on the canvas, another tool, etc.).
fn sticky_button(active: RwSignal<Tool>) -> impl IntoView {
    let open = RwSignal::new(false);
    // The picked colour tints the button's strip so the palette shows what the
    // next note will be. Seeded from the current tool colour when Sticky is
    // armed, else the default (the first swatch).
    let strip_color = move || match active.get() {
        Tool::Sticky(c) => c,
        _ => STICKY_SWATCHES[0],
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
                aria-label="Sticky note"
                aria-haspopup="true"
                aria-expanded=move || open.get().then_some("true")
                aria-pressed=move || is_active().then_some("true")
                on:mousedown=move |ev| ev.prevent_default()
                on:click=move |_| open.update(|o| *o = !*o)
            >
                <Icon icon=icondata::MdiNoteOutline />
                <span
                    class="swatch-strip"
                    style:background-color=move || format!("#{:06x}", strip_color() & 0x00ff_ffff)
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
                                        aria-label=format!("#{:06x}", argb & 0x00ff_ffff)
                                        style:background-color=format!(
                                            "#{:06x}",
                                            argb & 0x00ff_ffff,
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

/// The Toile tool palette. `active` is the shared active-tool signal (the same
/// signal the keyboard chords write and the canvas view reads); the buttons write
/// it on click and read it for their pressed state.
#[component]
pub fn ToolPalette(active: RwSignal<Tool>) -> impl IntoView {
    view! {
        <div class="ambre-commandbar ambre-tool-palette" role="toolbar" aria-label="Tools">
            {tool_button(active, Tool::Select, "Select", icondata::MdiCursorDefault)}
            <span class="sep"></span>
            {tool_button(active, Tool::Pen, "Pen", icondata::MdiVectorPolyline)}
            // The Shape tool defaults to a rectangle; the kind sub-picker is a
            // later palette signal folded into `CreateElement` (`toile-tools`).
            {tool_button(active, Tool::Shape(ShapeKind::Rect), "Shape", icondata::MdiShapeOutline)}
            {tool_button(active, Tool::Connector, "Connector", icondata::MdiVectorLine)}
            {tool_button(active, Tool::Draw, "Draw", icondata::MdiDraw)}
            {sticky_button(active)}
            {tool_button(active, Tool::Text, "Text", icondata::MdiFormatText)}
            <span class="sep"></span>
            {tool_button(active, Tool::Hand, "Hand", icondata::MdiHandBackRight)}
        </div>
    }
}
