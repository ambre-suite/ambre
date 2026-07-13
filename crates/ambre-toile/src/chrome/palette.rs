// ambre-toile/src/chrome/palette.rs
// ============================================================================
// The tool palette / primary toolbar (toile-ui SS5quinquies.A), wasm-only. A
// floating vertical toolbar with one button per Toile tool (Select, Pen, Shape,
// Connector, Draw, Sticky, Text, Hand -- SS5). Exactly one button reads active,
// reflecting the current `Tool`; clicking switches the tool.
//
// Styling is SHARED with the Trame command bar: the container carries the
// `.ambre-commandbar` class (so buttons, hover, the `[aria-pressed="true"]`
// active fill, `.sep` separators and the `aria-label` tooltip all come from the
// launcher CSS, pixel-identical to Trame) plus an `.ambre-tool-palette` modifier
// that only re-docks it as a floating vertical bar. Icons come from `icondata`
// rendered by `leptos-icons`, exactly like the Trame `FormatBar` and the Toile
// context menu (`chrome::menu`).
//
// The active tool is UI-only, canvas-local state (toile-ui SS11bis): it is NOT a
// `ToileCommand`. The palette writes a shared `RwSignal<Tool>` that the canvas
// `ToileView` applies each frame; the keyboard chords (`ui::palette::bind_tool_keys`)
// write the very same signal. The pure command surface (ids, key chords, the
// id -> Tool map) lives in the always-compiled, host-tested `ui::palette`.
// ============================================================================

use ambre_bus::app::toile::ShapeKind;
use leptos::prelude::*;
use leptos_icons::Icon;

use crate::ui::Tool;

/// One palette button. `aria-pressed` compares the active tool by variant
/// (`mem::discriminant`), so any `Shape(kind)` reads pressed while the Shape tool
/// is active regardless of the current kind sub-option (SS5quinquies.A). The
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
            // later palette signal folded into `CreateElement` (SS5quinquies.A).
            {tool_button(active, Tool::Shape(ShapeKind::Rect), "Shape", icondata::MdiShapeOutline)}
            {tool_button(active, Tool::Connector, "Connector", icondata::MdiVectorLine)}
            {tool_button(active, Tool::Draw, "Draw", icondata::MdiDraw)}
            {tool_button(active, Tool::Sticky, "Sticky note", icondata::MdiNoteOutline)}
            {tool_button(active, Tool::Text, "Text", icondata::MdiFormatText)}
            <span class="sep"></span>
            {tool_button(active, Tool::Hand, "Hand", icondata::MdiHandBackRight)}
        </div>
    }
}
