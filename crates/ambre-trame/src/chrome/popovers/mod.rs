// ambre-trame/src/chrome/popovers.rs
// ============================================================================
// The composed popover controls of the formatting chrome. The borders popover is
// a pen + preset grid, and the colour popover a palette + reset + custom. The
// shared filter popover is by-values / by-condition. The borders and colour
// popovers are `pub(crate)` so the context panel can mount them.
// ============================================================================

use crate::message::{
    BorderEdit, BorderPreset, BorderStyle, CellFormat, FilterKind, FilterOp, HAlign, StylePatch,
    VAlign,
};
use ambre_bus::Col;
use leptos::html;
use leptos::prelude::*;
use leptos_icons::Icon;
use wasm_bindgen::JsCast;

use super::commands::CLEAR_FILTER;
use super::style::StyleState;

// The colour popover now lives in the shared `ambre-chrome` toolkit. It was
// promoted so Toile's style inspector reuses the identical control
// (`toile-styles`). It is re-exported here so the context panel and the borders
// popover mount it unchanged.
pub use ambre_chrome::ColorPopover;

// Popover components split by concern; re-globbed so `popovers::*` paths hold.
mod filter;
mod style;

pub use filter::*;
pub use style::*;
