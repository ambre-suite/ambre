//! The Logic half of the view boundary (`UI-THREAD-064`).
//!
//! `UI-THREAD-020` keeps the document out of the view code. This module
//! holds the other half. The Logic code reaches a view module or the content
//! rendering crate through a path [`LOGIC_VIEW_REACH`] lists, with its reason. The Logic thread renders an
//! export or a thumbnail through the same painters the screen uses, so some
//! paths are by design. Each new one is a decision a reviewer reads.
//!
//! The view side mirrors the paths the view gate reads. It is an item of
//! `ambre-canvas`, or an item of an application module under one of
//! [`VIEW_MODULES`]. The Logic side is every
//! other source file of an application crate, and the runtime crate
//! `ambre-logic`. The crate root and the files of [`UI_OUTSIDE_VIEW`] are
//! set aside.
//!
//! The driver refuses an unlisted path in the unit that writes it. It also
//! records each listed path it meets, and each entry of the list, as facts.
//! The judge joins them over every unit and refuses an entry no unit
//! reaches, so the list only shrinks on its own.

use rustc_hir::def::DefKind;
use rustc_hir::def_id::DefId;
use rustc_middle::ty::TyCtxt;
use rustc_span::Span;

use crate::Gate;
use crate::bounds::{VIEW_MODULES, application_crate, application_source, view_module};
use crate::checks::shown_paths;
use crate::place;

/// The fact of an entry of [`LOGIC_VIEW_REACH`], written by each unit of the
/// crate the entry names.
pub const LISTED: &str = "logic-view-listed";

/// The fact of a listed path a unit reaches.
pub const REACHED: &str = "logic-view-reached";

/// The content rendering crate, which runs on the UI thread whole.
const CANVAS: &str = "ambre_canvas";

/// The runtime crate of the Logic thread.
const LOGIC_RUNTIME: &str = "crates/ambre-logic/src/";

/// Each path from a Logic file to a view module or to `ambre-canvas`. An
/// entry holds the file from the workspace root and the module that declares
/// the reached item. Its third field says why the Logic thread reaches it.
pub const LOGIC_VIEW_REACH: &[(&str, &str, &str)] = &[
    (
        "crates/ambre-toile/src/logic/interop/pdf_export.rs",
        "ambre_canvas::text_cache",
        "the PDF export shapes text through the text cache the screen walk takes, so a page prints the glyphs the board shows (CANVAS-074)",
    ),
    (
        "crates/ambre-toile/src/logic/interop/pdf_export.rs",
        "ambre_canvas::theme_gen",
        "the palette the board walk paints with: the export prints the light theme",
    ),
    (
        "crates/ambre-toile/src/logic/interop/pdf_export.rs",
        "ambre_toile::ui::grid",
        "the board background the screen paints under the elements, drawn once per page",
    ),
    (
        "crates/ambre-toile/src/logic/interop/pdf_export.rs",
        "ambre_toile::ui::viewport",
        "the camera of a page: the export frames each page with the transform the screen camera uses",
    ),
    (
        "crates/ambre-toile/src/logic/interop/png_export.rs",
        "ambre_toile::ui::viewport",
        "the camera of the image: the PNG export frames its region with the transform the screen camera uses",
    ),
    (
        "crates/ambre-toile/src/logic/interop/pdf_export.rs",
        "ambre_toile::ui::walk",
        "the one board walk the screen and the export share (CANVAS-074)",
    ),
    (
        "crates/ambre-toile/src/logic/interop/pptx_export.rs",
        "ambre_toile::ui::label",
        "the size of a connector label, which the deck writes as the frame of the label text",
    ),
    (
        "crates/ambre-toile/src/logic/state/boolean.rs",
        "ambre_toile::ui::walk",
        "the outline of a triangle, which a boolean operation takes as the shape the board draws",
    ),
    (
        "crates/ambre-toile/src/logic/state/connectors.rs",
        "ambre_toile::ui::label",
        "the width of a connector label, which bounds how far a label sits from its connector",
    ),
    (
        "crates/ambre-toile/src/logic/state/export.rs",
        "ambre_canvas::theme",
        "the image export renders the board in the light theme",
    ),
    (
        "crates/ambre-toile/src/logic/state/export.rs",
        "ambre_toile::ui::walk",
        "the decoded image the board walk draws, which the image export decodes once per picture (CANVAS-074)",
    ),
    (
        "crates/ambre-toile/src/logic/state/presentation.rs",
        "ambre_toile::ui::viewport",
        "a slide frame fits the page with the padding and the transform of the present glide on screen",
    ),
    (
        "crates/ambre-prose/src/logic/print.rs",
        "ambre_prose::ui",
        "the print export holds a view state of its own, which lays out and paints the printed pages as the screen does",
    ),
    (
        "crates/ambre-prose/src/logic/print.rs",
        "ambre_prose::view",
        "the span a link answers, which the print export turns into a link area of the PDF",
    ),
    (
        "crates/ambre-prose/src/logic/print.rs",
        "ambre_prose::view::print",
        "the printed pages and their link areas, which the view's print path builds for the PDF export",
    ),
    (
        "crates/ambre-trame/src/logic/print.rs",
        "ambre_canvas::text_cache",
        "the PDF export shapes cell text through the text cache the screen walk takes, so a page prints the glyphs the sheet shows (CANVAS-074)",
    ),
    (
        "crates/ambre-trame/src/logic/print.rs",
        "ambre_canvas::theme",
        "the palette the sheet walk paints with: the export prints the light theme",
    ),
    (
        "crates/ambre-trame/src/logic/print.rs",
        "ambre_canvas::view",
        "the count of cells the sheet walk drew, which the export reads as the walk the screen takes",
    ),
    (
        "crates/ambre-trame/src/logic/print.rs",
        "ambre_trame::render::content",
        "the content of a cell as the screen paints it, which the export packs for each page",
    ),
    (
        "crates/ambre-trame/src/logic/print.rs",
        "ambre_trame::render::walk",
        "the one sheet walk the screen and the export share, so a page holds what the screen shows",
    ),
    (
        "crates/ambre-trame/src/logic/print.rs",
        "ambre_trame::ui::grid",
        "the cell data the viewport packer builds, which the export packs once per page",
    ),
];

/// The source files of an application that run on the UI thread outside
/// its view modules, and why. Prose holds its modules side by side under
/// `src/`, so its path names no thread. Such a file is no Logic code. The
/// view gate does not read it either.
pub const UI_OUTSIDE_VIEW: &[(&str, &str)] = &[
    (
        "crates/ambre-prose/src/input.rs",
        "what a key, a press or typed text does to the view: the host calls it on the UI thread",
    ),
    (
        "crates/ambre-prose/src/probe.rs",
        "the keystroke latency probes, which read the draft buffer of the view on the UI thread",
    ),
];

/// Whether a file, from the workspace root, is Logic code: an application
/// source file outside its view modules, or the runtime crate. The crate
/// root serves both threads, as `UI-THREAD-062` sets it aside too.
pub fn logic_file(f: &str) -> bool {
    f.starts_with(LOGIC_RUNTIME)
        || application_source(f).is_some_and(|rest| {
            rest != "lib.rs" && !view_module(f) && !UI_OUTSIDE_VIEW.iter().any(|(u, _)| *u == f)
        })
}

/// The module `def` is declared in, named the same way from every crate:
/// `ambre_toile::ui::render`. `None` for an item outside a module.
fn declaring_module(tcx: TyCtxt<'_>, def: DefId) -> Option<(String, Vec<String>)> {
    let mut module = def;
    while !matches!(tcx.def_kind(module), DefKind::Mod) {
        module = tcx.opt_parent(module)?;
    }
    let segments: Vec<String> = tcx
        .def_path(module)
        .data
        .iter()
        .filter_map(|data| data.data.get_opt_name())
        .map(|name| name.to_string())
        .collect();
    Some((tcx.crate_name(def.krate).to_string(), segments))
}

/// The module of the view side that `def` belongs to, or `None` for an item
/// the Logic code reaches freely. A module on the way is no item, and a
/// generic parameter belongs to no module.
fn view_target(tcx: TyCtxt<'_>, def: DefId) -> Option<String> {
    if matches!(
        tcx.def_kind(def),
        DefKind::Mod | DefKind::TyParam | DefKind::ConstParam | DefKind::LifetimeParam
    ) {
        return None;
    }
    let (krate, segments) = declaring_module(tcx, def)?;
    let view = krate == CANVAS
        || (application_crate(&krate)
            && segments
                .first()
                .is_some_and(|first| VIEW_MODULES.contains(&first.as_str())));
    view.then(|| {
        [krate]
            .into_iter()
            .chain(segments)
            .collect::<Vec<_>>()
            .join("::")
    })
}

impl Gate {
    /// Writes one fact per entry of [`LOGIC_VIEW_REACH`] in this unit's
    /// package. The judge then finds an entry no unit reaches
    /// (`UI-THREAD-064`).
    pub fn logic_view_listed(&mut self) {
        if self.test {
            return;
        }
        let package = format!("crates/{}/", self.unit.krate);
        let listed: Vec<String> = LOGIC_VIEW_REACH
            .iter()
            .filter(|(file, _, _)| file.starts_with(&package))
            .map(|(file, module, _)| format!("{file} -> {module}"))
            .collect();
        for key in listed {
            self.record(&["fact", LISTED, &key, "-"]);
        }
    }

    /// Refuses a Logic file that reaches a view module or `ambre-canvas`
    /// through a path [`LOGIC_VIEW_REACH`] does not list (`UI-THREAD-064`).
    /// `def` is what the name at `span` resolves to.
    pub fn logic_view_reach(&mut self, tcx: TyCtxt<'_>, def: DefId, span: Span) {
        if self.test {
            return;
        }
        let Some(module) = view_target(tcx, def) else {
            return;
        };
        let at = place::position(tcx, &self.unit, span);
        let files = shown_paths(&self.unit.cwd, &at.path);
        let Some(file) = files.iter().find(|f| logic_file(f)) else {
            return;
        };
        let key = format!("{file} -> {module}");
        let listed = LOGIC_VIEW_REACH
            .iter()
            .any(|(f, m, _)| f == file && *m == module);
        if listed {
            self.record(&["fact", REACHED, &key, &at.at()]);
            return;
        }
        if !self
            .bounded
            .insert(("UI-THREAD-064", format!("{} {module}", at.at())))
        {
            return;
        }
        let path = tcx.def_path_str(def);
        self.refuse(
            "boundary",
            format!(
                "{}: a Logic file reaches the view code by a path no list names: `{path}` in \
                 `{module}` (UI-THREAD-064)",
                at.at()
            ),
        );
    }
}
