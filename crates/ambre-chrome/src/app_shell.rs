// ambre-chrome/src/app_shell.rs
// ============================================================================
// AppShell -- the CSS-grid chrome layout (ambre-spec-chrome.md, CHROME-009).
//
// Rows: header, command bar (contextual toolbar), content, status bar. The
// content row hosts the wgpu `<canvas>` cell; the shell OWNS that element so
// the DOM chrome and the canvas content share one layout (the canvas resizes
// to its grid cell, not the viewport).
//
// The shell is app-agnostic: every region is a slot the app fills. ambre-chrome
// knows nothing about Trame/Prose/etc. -- it lays out whatever chrome content
// the app provides, exactly as the renderer dispatches only over `AppView`.
// ============================================================================

use leptos::prelude::*;

/// Header region: title, sync state, avatar (app-provided content).
#[slot]
pub struct Header {
    children: ChildrenFn,
}

/// Command bar: the contextual toolbar (app-provided content). One bar showing
/// commands relevant to the selection; the app builds it from the registry.
#[slot]
pub struct Toolbar {
    children: ChildrenFn,
}

/// Formula bar: name box (cell reference) + content input (app-provided).
#[slot]
pub struct FormulaBar {
    children: ChildrenFn,
}

/// Tab strip: a thin band just below the content, above the status bar
/// (app-provided content -- e.g. Trame's sheet tabs). Optional: apps with no
/// tabbed content (Prose) omit it and the row collapses.
#[slot]
pub struct Tabs {
    children: ChildrenFn,
}

/// Status bar: cursor position, zoom, sync state, counters (app-provided).
#[slot]
pub struct StatusBar {
    children: ChildrenFn,
}

/// The application shell. Renders the chrome grid around a central `<canvas>`
/// whose `id` defaults to `"ambre-canvas"` (the id the canvas host looks up).
#[component]
pub fn AppShell(
    /// Id of the central canvas element. Defaults to `"ambre-canvas"`.
    #[prop(optional, into)]
    canvas_id: Option<String>,
    header: Header,
    toolbar: Toolbar,
    #[prop(optional)] formula_bar: Option<FormulaBar>,
    #[prop(optional)] tabs: Option<Tabs>,
    status_bar: StatusBar,
) -> impl IntoView {
    let canvas_id = canvas_id.unwrap_or_else(|| "ambre-canvas".to_owned());
    view! {
        <div class="ambre-shell">
            <header class="ambre-header">{(header.children)()}</header>
            <div class="ambre-commandbar">{(toolbar.children)()}</div>
            {if let Some(fb) = formula_bar {
                Some(view! { <div class="ambre-formulabar">{(fb.children)()}</div> })
            } else {
                None
            }}
            <main class="ambre-content">
                // The canvas host (ambre-canvas) attaches wgpu to this element
                // by id and sizes its backing store to the cell.
                <canvas id=canvas_id class="ambre-canvas" tabindex="0"></canvas>
            </main>
            {if let Some(tabs) = tabs {
                Some(view! { <div class="ambre-tabstrip">{(tabs.children)()}</div> })
            } else {
                None
            }}
            <footer class="ambre-statusbar">{(status_bar.children)()}</footer>
        </div>
    }
}
