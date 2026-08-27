// ambre-toile/src/ui/palette.rs
// ============================================================================
// Command surface for the Toile tool palette / primary toolbar
// (`toile-tools`). The host-visible half: the `CommandId`s the palette
// contributes, the pure `CommandId -> Tool` map, the bare-key tool chords, and
// the pure focus-guard predicate the launcher applies before stealing a key.
//
// Kept in the always-compiled `ui` module (not the wasm-only `chrome`) so it is
// host-unit-tested, exactly like `ui::find` and `ui::menu`. The wasm-only
// Leptos component that renders these (`chrome::ToolPalette`) consumes them.
//
// The active tool is UI-only, canvas-local state (`toile-tools`, "Local
// view"): it never becomes a `ToileCommand` and never reaches the Logic thread.
// A tool `CommandId` is therefore NOT mapped through `command_to_app`; the
// palette (and the keyboard) write a shared `RwSignal<Tool>` the view applies.
// ============================================================================

use std::borrow::Cow;

use ambre_bus::app::toile::{STICKY_DEFAULT_FILL, ShapeKind};
use ambre_commands::{CommandId, Keymap};

use super::tool::{Handle, Tool};

// -- Command identities --------------------------------------------------------
// String-keyed ids (`ambre-commands`), one per tool. The identity is the string,
// so the shell can round-trip an id through a signal without carrying the
// `Command` table.

/// Select / move / resize / rotate / marquee (`toile-tools`).
pub const TOOL_SELECT: CommandId = CommandId("toile.tool.select");
/// Vector pen (Bezier path).
pub const TOOL_PEN: CommandId = CommandId("toile.tool.pen");
/// Shape drag-rect (defaults to a rectangle; the kind sub-option is a local
/// palette signal folded into `CreateElement`, `toile-tools`).
pub const TOOL_SHAPE: CommandId = CommandId("toile.tool.shape");
/// Connector (anchor to anchor).
pub const TOOL_CONNECTOR: CommandId = CommandId("toile.tool.connector");
/// Freehand draw.
pub const TOOL_DRAW: CommandId = CommandId("toile.tool.draw");
/// Sticky note.
pub const TOOL_STICKY: CommandId = CommandId("toile.tool.sticky");
/// Text element.
pub const TOOL_TEXT: CommandId = CommandId("toile.tool.text");
/// Pan-only hand.
pub const TOOL_HAND: CommandId = CommandId("toile.tool.hand");

/// Every tool `CommandId`, in palette order. The `chrome::ToolPalette` renders
/// one button per entry; the launcher intercepts these ids (UI-only intents).
pub const TOOL_COMMANDS: &[CommandId] = &[
    TOOL_SELECT,
    TOOL_PEN,
    TOOL_SHAPE,
    TOOL_CONNECTOR,
    TOOL_DRAW,
    TOOL_STICKY,
    TOOL_TEXT,
    TOOL_HAND,
];

/// Map a tool `CommandId` to the [`Tool`] it selects, or `None` for any id that
/// is not a tool (a non-tool Toile command such as `FIND`, or an unknown id).
/// `TOOL_SHAPE` selects the rectangle by default (`toile-tools`).
pub fn tool_for_command(id: CommandId) -> Option<Tool> {
    match id {
        TOOL_SELECT => Some(Tool::Select),
        TOOL_PEN => Some(Tool::Pen),
        TOOL_SHAPE => Some(Tool::Shape(ShapeKind::Rect)),
        TOOL_CONNECTOR => Some(Tool::Connector),
        TOOL_DRAW => Some(Tool::Draw),
        // The bare command / keyboard chord arms the default sticky yellow; the
        // palette's colour picker arms Sticky(colour) directly.
        TOOL_STICKY => Some(Tool::Sticky(STICKY_DEFAULT_FILL)),
        TOOL_TEXT => Some(Tool::Text),
        TOOL_HAND => Some(Tool::Hand),
        _ => None,
    }
}

/// The sticky-note colour swatches offered by the palette's colour picker
/// (`toile-tools`), ARGB. The first entry is [`STICKY_DEFAULT_FILL`]
/// (the bare-key / default colour); the rest are the standard whiteboard pastels
/// (blue, green, pink, orange, purple, grey). Shared with the chrome popover so
/// the offered set is host-tested here rather than buried in wasm-only code.
pub const STICKY_SWATCHES: &[u32] = &[
    STICKY_DEFAULT_FILL, // yellow (default)
    0xFFA5_D8FF,         // blue
    0xFFB2_F2BB,         // green
    0xFFFF_C9C9,         // pink
    0xFFFF_D8A8,         // orange
    0xFFD0_BFFF,         // purple
    0xFFDE_E2E6,         // grey
];

/// Bind the bare-key tool chords into the launcher keymap. These are the de-facto
/// whiteboard letters (V/P/R/C/D/N/T/H); `toile-tools` promises the palette click
/// and the keyboard resolve to the same tool selection. The launcher guards them
/// with [`is_editable_tag`] so a letter typed into an inline editor is not stolen.
pub fn bind_tool_keys(keymap: &mut Keymap) {
    for &id in TOOL_COMMANDS {
        if let Some(chord) = crate::ui::commands::chord(id) {
            keymap.bind(chord, id);
        }
    }
}

/// A note-shaped CSS cursor (inlined SVG data URI) shown while the Sticky tool is
/// armed, so it is obvious a click will drop a note (`toile-tools`,
/// feedback). `%23` encodes `#` inside the URI; the hotspot is the note's
/// top-left corner. Falls back to `crosshair` if the data URI is unsupported.
pub const STICKY_CURSOR: &str = "url(\"data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' width='28' height='28' viewBox='0 0 28 28'><rect x='4' y='4' width='18' height='18' fill='%23FEF08A' stroke='%23333333' stroke-width='1'/><path d='M17 22 L22 22 L22 17 Z' fill='%23E4CE5C'/></svg>\") 4 4, crosshair";

/// The CSS `cursor` value for the canvas while `tool` is active
/// (`toile-tools`, "Cursor feedback"): a note-shaped cursor while the Sticky tool
/// is armed, a crosshair for the other creation tools (so a click visibly means
/// "place here"), a grab hand for the pan-only Hand tool, and the default arrow
/// for Select. Pure so the chrome can bind it to the active-tool signal without
/// any canvas round-trip, and host-tested.
pub fn cursor_css(tool: Tool) -> &'static str {
    match tool {
        Tool::Select => "default",
        Tool::Hand => "grab",
        Tool::Sticky(_) => STICKY_CURSOR,
        Tool::Text | Tool::Shape(_) | Tool::Pen | Tool::Connector | Tool::Draw => "crosshair",
    }
}

/// A rotation cursor (inlined SVG data URI) shown on the rotate handle so it is
/// obvious the drag will rotate rather than resize (`toile-tools`).
/// Built from the shared `icondata::LuRotateCw` glyph (Lucide "rotate-cw", a
/// clockwise circular arrow): its paths are wrapped in a full `<svg>` and painted
/// twice -- a white under-stroke then a dark top-stroke -- so it stays legible on
/// any board colour. The glyph's `"` are swapped for `'` so the markup survives
/// inside the CSS `url("…")`, and `#` is percent-encoded. Hotspot at the icon
/// centre; falls back to `grab` if the data URI is unsupported. Lazily built once.
pub static ROTATE_CURSOR: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    // Swap `"` for `'` (so the markup survives inside `url("…")`) and collapse all
    // whitespace to single spaces -- the glyph data spans two lines, and a raw
    // newline is a control char that invalidates the whole data URI / cursor.
    let paths = icondata::LuRotateCw
        .data
        .replace('"', "'")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let view_box = icondata::LuRotateCw.view_box.unwrap_or("0 0 24 24");
    format!(
        "url(\"data:image/svg+xml;utf8,\
         <svg xmlns='http://www.w3.org/2000/svg' width='28' height='28' viewBox='{view_box}' \
         fill='none' stroke-linecap='round' stroke-linejoin='round'>\
         <g stroke='%23FFFFFF' stroke-width='4'>{paths}</g>\
         <g stroke='%23222222' stroke-width='2'>{paths}</g>\
         </svg>\") 14 14, grab"
    )
});

/// The Lucide "move-horizontal" glyph (a horizontal double-headed arrow, `<->`,
/// centred at 12,12 in a 0 0 24 24 box): rotated to any angle it becomes the
/// directional resize cursor for that axis. Its `"` are swapped for `'` and its
/// whitespace collapsed so the markup survives inside the CSS `url("…")`. Built
/// once; shared by every rotated resize cursor.
static MOVE_PATHS: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    icondata::LuMoveHorizontal
        .data
        .replace('"', "'")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
});

/// The base resize-axis angle (radians, screen space y-down) of a handle on the
/// axis-aligned box, or `None` for the rotate handle. A resize direction has
/// 180-degree symmetry, so opposite handles share an angle.
fn resize_axis_base(h: Handle) -> Option<f64> {
    use std::f64::consts::{FRAC_PI_2, FRAC_PI_4};
    match h {
        Handle::E | Handle::W => Some(0.0),
        Handle::Nw | Handle::Se => Some(FRAC_PI_4),
        Handle::N | Handle::S => Some(FRAC_PI_2),
        Handle::Ne | Handle::Sw => Some(3.0 * FRAC_PI_4),
        Handle::Rotate => None,
    }
}

/// Snap a resize-axis angle (radians) to the nearest of the four native CSS resize
/// keywords -- the crisp cursor for an axis-aligned box, and the graceful fallback
/// when a data-URI cursor is unsupported.
fn nearest_resize_keyword(a: f64) -> &'static str {
    use std::f64::consts::{FRAC_PI_4, PI};
    match (a.rem_euclid(PI) / FRAC_PI_4).round() as i64 % 4 {
        0 => "ew-resize",
        1 => "nwse-resize",
        2 => "ns-resize",
        _ => "nesw-resize",
    }
}

/// A directional resize cursor rotated `deg` degrees: the `MOVE_PATHS` double
/// arrow wrapped in a `rotate(deg 12 12)` group, painted twice (white under-stroke
/// then dark top-stroke) so it stays legible on any board colour, hotspot centred.
/// `fallback` is the native keyword shown if the data URI is unsupported.
fn rotated_resize_cursor(deg: f64, fallback: &str) -> String {
    let paths = MOVE_PATHS.as_str();
    let d = deg.rem_euclid(360.0).round() as i64;
    format!(
        "url(\"data:image/svg+xml;utf8,\
         <svg xmlns='http://www.w3.org/2000/svg' width='28' height='28' viewBox='0 0 24 24' \
         fill='none' stroke-linecap='round' stroke-linejoin='round'>\
         <g transform='rotate({d} 12 12)'>\
         <g stroke='%23FFFFFF' stroke-width='4'>{paths}</g>\
         <g stroke='%23222222' stroke-width='2'>{paths}</g>\
         </g></svg>\") 14 14, {fallback}"
    )
}

/// The CSS `cursor` value for hovering / dragging a selection transform handle
/// (`toile-tools`, "Cursor feedback"): a directional double-arrow that
/// matches the axis a corner or side resizes along, and a rotation cursor on the
/// rotate handle. `theta` is the element's rotation (radians). For an axis-aligned
/// element (`theta == 0`) it returns the crisp **native** CSS keyword; for a
/// rotated one it returns a **custom SVG cursor rotated to the exact angle**
/// (`resize_axis_base(h) + theta`), so the arrow tilts continuously with the box
/// -- native resize cursors are fixed OS bitmaps and cannot be rotated. Pure and
/// host-tested.
pub fn resize_cursor(h: Handle, theta: f64) -> Cow<'static, str> {
    let Some(base) = resize_axis_base(h) else {
        return Cow::Borrowed(ROTATE_CURSOR.as_str());
    };
    let angle = base + theta;
    if theta == 0.0 {
        // Axis-aligned: the native keyword is crisper than an SVG cursor.
        Cow::Borrowed(nearest_resize_keyword(angle))
    } else {
        Cow::Owned(rotated_resize_cursor(
            angle.to_degrees(),
            nearest_resize_keyword(angle),
        ))
    }
}

/// Whether a keyboard event targeting an element with this tag / contenteditable
/// flag should be treated as "typing", so a bare tool chord must NOT be stolen.
/// True for form text fields (`INPUT`, `TEXTAREA`) and any contenteditable host.
/// The tag is compared case-insensitively (`Element.tagName` is upper-case in
/// HTML, but be defensive). The launcher's `target_is_editable` reads `tagName`
/// and `isContentEditable` off the event target and delegates here so the
/// decision stays pure and host-tested.
pub fn is_editable_tag(tag_name: &str, content_editable: bool) -> bool {
    if content_editable {
        return true;
    }
    tag_name.eq_ignore_ascii_case("input") || tag_name.eq_ignore_ascii_case("textarea")
}

#[cfg(test)]
mod tests {
    use super::*;
    // The chord types are the tests' business now: the module itself reads
    // every binding from the registry.
    use ambre_commands::{Key, KeyBinding, Modifiers};

    #[test]
    fn tool_for_command_maps_every_tool_id() {
        assert_eq!(tool_for_command(TOOL_SELECT), Some(Tool::Select));
        assert_eq!(tool_for_command(TOOL_PEN), Some(Tool::Pen));
        assert_eq!(
            tool_for_command(TOOL_SHAPE),
            Some(Tool::Shape(ShapeKind::Rect))
        );
        assert_eq!(tool_for_command(TOOL_CONNECTOR), Some(Tool::Connector));
        assert_eq!(tool_for_command(TOOL_DRAW), Some(Tool::Draw));
        assert_eq!(
            tool_for_command(TOOL_STICKY),
            Some(Tool::Sticky(STICKY_DEFAULT_FILL))
        );
        assert_eq!(tool_for_command(TOOL_TEXT), Some(Tool::Text));
        assert_eq!(tool_for_command(TOOL_HAND), Some(Tool::Hand));
    }

    #[test]
    fn tool_for_command_rejects_non_tool_ids() {
        // A non-tool Toile command (the Find panel opener) and an unknown id both
        // resolve to `None`, so the launcher never switches tools for them.
        assert_eq!(tool_for_command(CommandId("toile.find.open")), None);
        assert_eq!(tool_for_command(CommandId("nonsense")), None);
    }

    #[test]
    fn tool_keys_round_trip_key_to_command_to_tool() {
        let mut keymap = Keymap::new();
        bind_tool_keys(&mut keymap);
        // Each chord resolves to its command, and the command maps back to the
        // expected tool -- the full key -> command -> tool wiring, no DOM.
        let cases = [
            ('v', TOOL_SELECT, Tool::Select),
            ('p', TOOL_PEN, Tool::Pen),
            ('r', TOOL_SHAPE, Tool::Shape(ShapeKind::Rect)),
            ('c', TOOL_CONNECTOR, Tool::Connector),
            ('d', TOOL_DRAW, Tool::Draw),
            ('n', TOOL_STICKY, Tool::Sticky(STICKY_DEFAULT_FILL)),
            ('t', TOOL_TEXT, Tool::Text),
            ('h', TOOL_HAND, Tool::Hand),
        ];
        for (c, id, tool) in cases {
            let chord = KeyBinding::new(Modifiers::NONE, Key::char(c));
            assert_eq!(keymap.resolve(chord), Some(id), "chord {c} -> command");
            assert_eq!(tool_for_command(id), Some(tool), "command {c} -> tool");
        }
    }

    #[test]
    fn tool_chords_are_distinct() {
        // No two tools share a chord (a collision would silently drop a tool from
        // the keyboard).
        let mut keymap = Keymap::new();
        bind_tool_keys(&mut keymap);
        let mut ids: Vec<CommandId> = ['v', 'p', 'r', 'c', 'd', 'n', 't', 'h']
            .into_iter()
            .filter_map(|c| keymap.resolve(KeyBinding::new(Modifiers::NONE, Key::char(c))))
            .collect();
        assert_eq!(ids.len(), TOOL_COMMANDS.len());
        ids.sort_by_key(|id| id.0);
        ids.dedup();
        assert_eq!(ids.len(), TOOL_COMMANDS.len(), "tool chords collide");
    }

    #[test]
    fn cursor_reflects_the_active_tool() {
        assert_eq!(cursor_css(Tool::Select), "default");
        assert_eq!(cursor_css(Tool::Hand), "grab");
        assert_eq!(cursor_css(Tool::Sticky(STICKY_DEFAULT_FILL)), STICKY_CURSOR);
        assert!(STICKY_CURSOR.starts_with("url("));
        assert_eq!(cursor_css(Tool::Text), "crosshair");
        assert_eq!(cursor_css(Tool::Shape(ShapeKind::Rect)), "crosshair");
        assert_eq!(cursor_css(Tool::Pen), "crosshair");
        assert_eq!(cursor_css(Tool::Connector), "crosshair");
        assert_eq!(cursor_css(Tool::Draw), "crosshair");
    }

    #[test]
    fn resize_cursor_uses_native_keywords_when_axis_aligned() {
        // Axis-aligned element (theta = 0): each handle keeps its native keyword.
        assert_eq!(resize_cursor(Handle::Nw, 0.0).as_ref(), "nwse-resize");
        assert_eq!(resize_cursor(Handle::Se, 0.0).as_ref(), "nwse-resize");
        assert_eq!(resize_cursor(Handle::Ne, 0.0).as_ref(), "nesw-resize");
        assert_eq!(resize_cursor(Handle::Sw, 0.0).as_ref(), "nesw-resize");
        assert_eq!(resize_cursor(Handle::N, 0.0).as_ref(), "ns-resize");
        assert_eq!(resize_cursor(Handle::S, 0.0).as_ref(), "ns-resize");
        assert_eq!(resize_cursor(Handle::E, 0.0).as_ref(), "ew-resize");
        assert_eq!(resize_cursor(Handle::W, 0.0).as_ref(), "ew-resize");
        assert_eq!(
            resize_cursor(Handle::Rotate, 0.0).as_ref(),
            ROTATE_CURSOR.as_str()
        );
        assert!(ROTATE_CURSOR.starts_with("url(\""));
        assert!(ROTATE_CURSOR.ends_with("grab"));
        // The Lucide glyph made it in, and its `"` were swapped for `'` so the only
        // double quotes left are the two wrapping the `url(...)` value.
        assert!(ROTATE_CURSOR.contains("<path"));
        assert_eq!(ROTATE_CURSOR.matches('"').count(), 2);
    }

    #[test]
    fn resize_cursor_rotates_continuously_when_the_element_is_rotated() {
        use std::f64::consts::{FRAC_PI_2, FRAC_PI_6};
        // A rotated element yields a custom SVG cursor tilted to the exact angle,
        // not a native keyword: the E handle at a quarter turn points along 90 deg.
        let c = resize_cursor(Handle::E, FRAC_PI_2);
        assert!(
            c.starts_with("url(\""),
            "rotated resize cursor is a data URI"
        );
        assert!(
            c.contains("rotate(90 12 12)"),
            "tilted to the exact angle: {c}"
        );
        // It carries the move-arrow glyph and falls back to the nearest keyword.
        assert!(c.contains("<path"));
        assert!(c.ends_with("ns-resize"), "graceful native fallback: {c}");
        // Only the two `url("...")` quotes survive (paths were re-quoted to `'`).
        assert_eq!(c.matches('"').count(), 2);
        // An arbitrary (non-45) angle is honoured exactly: 30 deg for the E axis.
        let c = resize_cursor(Handle::E, FRAC_PI_6);
        assert!(c.contains("rotate(30 12 12)"), "{c}");
        // The rotate handle stays the rotation cursor regardless of angle.
        assert_eq!(
            resize_cursor(Handle::Rotate, FRAC_PI_6).as_ref(),
            ROTATE_CURSOR.as_str()
        );
    }

    #[test]
    fn sticky_swatches_lead_with_the_default_and_are_distinct() {
        assert_eq!(STICKY_SWATCHES.first(), Some(&STICKY_DEFAULT_FILL));
        let mut seen = STICKY_SWATCHES.to_vec();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), STICKY_SWATCHES.len(), "swatch colours collide");
        // Every swatch is fully opaque (alpha = 0xFF), so a note is never
        // accidentally transparent.
        assert!(STICKY_SWATCHES.iter().all(|c| c >> 24 == 0xFF));
    }

    #[test]
    fn editable_tag_guards_text_entry() {
        assert!(is_editable_tag("INPUT", false));
        assert!(is_editable_tag("textarea", false)); // case-insensitive
        assert!(is_editable_tag("DIV", true)); // contenteditable host
        assert!(!is_editable_tag("DIV", false));
        assert!(!is_editable_tag("CANVAS", false));
    }
}
