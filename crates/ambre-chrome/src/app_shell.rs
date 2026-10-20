// ambre-chrome/src/app_shell.rs
// ============================================================================
// AppShell -- the CSS-grid chrome layout (ambre-spec-chrome.md, CHROME-009).
//
// Rows: header, command bar (contextual toolbar), body, status bar. The body
// row holds an optional sidebar, the content and an optional panel, and the
// content hosts the wgpu `<canvas>` cell. The shell OWNS that element, so the
// DOM chrome and the canvas content share one layout. The canvas resizes to its
// grid cell, not the viewport. Which body regions sit side by side at a width
// is the stylesheet's alone (`CHROME-056`): the shell only names the focus.
//
// The shell is app-agnostic: every region is a slot the app fills. ambre-chrome
// knows nothing about Trame, Prose or any other app -- it lays out whatever
// chrome content the app provides. This mirrors the renderer, which dispatches
// only over `AppView`.
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
/// (app-provided content -- for example Trame's sheet tabs). Optional: apps with no
/// tabbed content (Prose) omit it and the row collapses.
#[slot]
pub struct Tabs {
    children: ChildrenFn,
}

/// Status bar: cursor position, zoom, sync state, counters (app-provided).
/// Optional: an application that reports nothing omits it, and the row is absent.
#[slot]
pub struct StatusBar {
    children: ChildrenFn,
}

/// Sidebar: the navigation region before the content (app-provided). It folds
/// to a strip from its own control, and stays on screen (`CHROME-033`).
#[slot]
pub struct Sidebar {
    /// The accessible name of the region, such as "Channels".
    #[prop(into)]
    label: String,
    children: ChildrenFn,
}

/// Panel: the region after the content (app-provided), such as an open thread.
#[slot]
pub struct Panel {
    /// The accessible name of the region, such as "Thread". It follows what
    /// the panel holds.
    #[prop(into)]
    label: Signal<String>,
    children: ChildrenFn,
}

/// The region a narrow window shows when it cannot show them all. The
/// stylesheet decides, at each width, which regions sit beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellRegion {
    /// The sidebar.
    Sidebar,
    /// The content, between the sidebar and the panel.
    Content,
    /// The panel.
    Panel,
}

impl ShellRegion {
    /// The word the stylesheet reads off `data-focus`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sidebar => "sidebar",
            Self::Content => "content",
            Self::Panel => "panel",
        }
    }
}

/// A canvas of its own beside the central one, filling the rest of the region
/// that holds it. The canvas host binds it by `id` while it is in the page
/// (`CANVAS-070`), so a view can show a second region on a canvas.
///
/// The canvas takes the focus and handles its own keys. So it is an
/// application to assistive technology, which then hands it the arrows. What
/// it shows reaches a screen reader through the live region (`CHROME-023`).
#[component]
pub fn CanvasCell(
    /// Id of the canvas element, the one the canvas host looks up.
    id: &'static str,
    /// The name a screen reader gives the region.
    label: &'static str,
) -> impl IntoView {
    view! {
        <div class="ambre-canvas-cell">
            <canvas id=id class="ambre-canvas" tabindex="0" role="application" aria-label=label>
            </canvas>
        </div>
    }
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
    #[prop(optional)] status_bar: Option<StatusBar>,
    #[prop(optional)] sidebar: Option<Sidebar>,
    #[prop(optional)] panel: Option<Panel>,
    /// The region in focus, for the widths that show one region alone. The
    /// content when unset.
    #[prop(optional, into)]
    focus: Option<Signal<ShellRegion>>,
    /// Whether the panel is open. A closed panel leaves the DOM. Open when unset.
    #[prop(optional, into)]
    panel_open: Option<Signal<bool>>,
) -> impl IntoView {
    let canvas_id = canvas_id.unwrap_or_else(|| "ambre-canvas".to_owned());
    let focus = focus.unwrap_or_else(|| Signal::stored(ShellRegion::Content));
    let panel_open = panel_open.unwrap_or_else(|| Signal::stored(true));
    let folded = RwSignal::new(false);
    view! {
        <div class="ambre-shell">
            <header class="ambre-header">{(header.children)()}</header>
            <div class="ambre-commandbar">{(toolbar.children)()}</div>
            {formula_bar.map(|fb| view! { <div class="ambre-formulabar">{(fb.children)()}</div> })}
            <div class="ambre-body" data-focus=move || focus.get().as_str()>
                {sidebar.map(|sidebar| {
                    view! {
                        <nav
                            class="ambre-sidebar"
                            aria-label=sidebar.label
                            data-folded=move || crate::aria_bool(folded.get())
                        >
                            <button
                                class="icon ambre-sidebar-fold"
                                aria-label="Fold the sidebar"
                                aria-expanded=move || crate::aria_bool(!folded.get())
                                on:click=move |_| folded.update(|f| *f = !*f)
                            >
                                {move || if folded.get() { "\u{203a}" } else { "\u{2039}" }}
                            </button>
                            <div class="ambre-sidebar-body">{(sidebar.children)()}</div>
                        </nav>
                    }
                })}
                <main class="ambre-content">
                    // The canvas host (ambre-canvas) attaches wgpu to this element
                    // by id and sizes its backing store to the cell.
                    <canvas id=canvas_id class="ambre-canvas" tabindex="0"></canvas>
                    // The layer a board's sandboxed web embeds are mounted into
                    // (`toile-embeds`, `TOILE-088`). Empty for every other app and
                    // for a board with no web embed on screen. It sits over the
                    // canvas and clips to it. Each frame decides for itself whether
                    // it takes the pointer, so an embed nobody has entered leaves
                    // the board's own gestures reachable.
                    <div id="ambre-web-embeds" class="ambre-web-embeds"></div>
                </main>
                {panel.map(|panel| {
                    let label = panel.label;
                    let children = panel.children;
                    move || {
                        panel_open.get().then(|| {
                            view! {
                                <aside class="ambre-panel" aria-label=move || label.get()>
                                    {children()}
                                </aside>
                            }
                        })
                    }
                })}
            </div>
            {tabs.map(|tabs| view! { <div class="ambre-tabstrip">{(tabs.children)()}</div> })}
            {status_bar.map(|sb| view! { <footer class="ambre-statusbar">{(sb.children)()}</footer> })}
        </div>
    }
}
