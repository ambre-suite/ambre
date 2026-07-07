// ambre-trame/src/chrome/popovers.rs
// ============================================================================
// The composed popover controls of the formatting chrome: the borders popover
// (pen + preset grid), the colour popover (palette + reset + custom), and the
// shared filter popover (by-values / by-condition). The borders and colour
// popovers are `pub(crate)` so the `FormatBar` toolbar can mount them.
// ============================================================================

use ambre_bus::{
    BorderEdit, BorderPreset, BorderStyle, CellFormat, FilterKind, FilterOp, HAlign, StylePatch,
    VAlign,
};
use ambre_chrome::commands::CommandId;
use leptos::html;
use leptos::prelude::*;
use leptos_icons::Icon;
use wasm_bindgen::JsCast;

use super::color::{color_hex, parse_hex_color};
use super::commands::CLEAR_FILTER;
use super::style::StyleState;

// Popover components split by concern; re-globbed so `popovers::*` paths hold.
mod filter;
mod style;

pub use filter::*;
pub use style::*;
