// ambre-toile/src/ui/palette.rs
// ============================================================================
// Command surface for the Toile tool palette / primary toolbar
// (`toile-tools`). The host-visible half: the `StaticKey<Commands>`s the palette
// contributes, the pure `StaticKey<Commands> -> Tool` map and the bare-key tool chords.
// Also the pure focus-guard predicate the launcher applies before stealing a
// key.
//
// Kept in the always-compiled `ui` module (not the wasm-only `chrome`) so it is
// host-unit-tested, exactly like `ui::find` and `ui::menu`. The wasm-only
// Leptos component that renders these (`chrome::ToolPalette`) consumes them.
//
// The active tool is UI-only, canvas-local state (`toile-tools`, "Local
// view"): it never becomes a `ToileCommand` and never reaches the Logic thread.
// A tool `StaticKey<Commands>` is therefore NOT mapped through `command_to_app`; the
// palette (and the keyboard) write a shared `RwSignal<Tool>` the view applies.
// ============================================================================

use ambre_commands::{Commands, Keymap, StaticKey};
use ambre_core::Rgba;
use std::borrow::Cow;

use ambre_bus::app::toile::{STICKY_DEFAULT_FILL, ShapeKind};

use super::tool::{Handle, Tool};

// -- Command identities --------------------------------------------------------
// String-keyed ids (`ambre-commands`), one per tool. The identity is the string,
// so the shell can round-trip an id through a signal without carrying the
// `Command` table.

/// Select / move / resize / rotate / marquee (`toile-tools`).
pub const TOOL_SELECT: StaticKey<Commands> = StaticKey::<Commands>::new("toile.tool.select");
/// Vector pen (Bezier path).
pub const TOOL_PEN: StaticKey<Commands> = StaticKey::<Commands>::new("toile.tool.pen");
/// Shape drag-rect (defaults to a rectangle; the kind sub-option is a local
/// palette signal folded into `CreateElement`, `toile-tools`).
pub const TOOL_SHAPE: StaticKey<Commands> = StaticKey::<Commands>::new("toile.tool.shape");
/// Connector (anchor to anchor).
pub const TOOL_CONNECTOR: StaticKey<Commands> = StaticKey::<Commands>::new("toile.tool.connector");
/// Freehand draw.
pub const TOOL_DRAW: StaticKey<Commands> = StaticKey::<Commands>::new("toile.tool.draw");
/// Sticky note.
pub const TOOL_STICKY: StaticKey<Commands> = StaticKey::<Commands>::new("toile.tool.sticky");
/// Text element.
pub const TOOL_TEXT: StaticKey<Commands> = StaticKey::<Commands>::new("toile.tool.text");
/// Drop a comment thread (`toile-comments`, `TOILE-UI-042`).
pub const TOOL_COMMENT: StaticKey<Commands> = StaticKey::<Commands>::new("toile.tool.comment");
/// The highlighter (`toile-presence`, `TOILE-UI-074`): a trail the other peers
/// see and the board never keeps.
pub const TOOL_LASER: StaticKey<Commands> = StaticKey::<Commands>::new("toile.tool.laser");
/// Pan-only hand.
pub const TOOL_HAND: StaticKey<Commands> = StaticKey::<Commands>::new("toile.tool.hand");

/// Every tool `StaticKey<Commands>`, in palette order. The `chrome::ToolPalette` renders
/// one button per entry; the launcher intercepts these ids (UI-only intents).
pub const TOOL_COMMANDS: &[StaticKey<Commands>] = &[
    TOOL_SELECT,
    TOOL_PEN,
    TOOL_SHAPE,
    TOOL_CONNECTOR,
    TOOL_DRAW,
    TOOL_STICKY,
    TOOL_TEXT,
    TOOL_COMMENT,
    TOOL_LASER,
    TOOL_HAND,
];

/// Map a tool `StaticKey<Commands>` to the [`Tool`] it selects, or `None` for any id that
/// is not a tool. That covers a non-tool Toile command such as `FIND`, and an
/// unknown id. `TOOL_SHAPE` selects the rectangle by default (`toile-tools`).
pub fn tool_for_command(id: StaticKey<Commands>) -> Option<Tool> {
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
        TOOL_COMMENT => Some(Tool::Comment),
        TOOL_LASER => Some(Tool::Laser),
        TOOL_HAND => Some(Tool::Hand),
        _ => None,
    }
}

/// The sticky-note colour swatches offered by the palette's colour picker
/// (`toile-tools`), RGBA. The first entry is [`STICKY_DEFAULT_FILL`]
/// (the bare-key / default colour); the rest are the standard whiteboard pastels
/// (blue, green, pink, orange, purple, grey). Shared with the chrome popover so
/// the offered set is host-tested here rather than buried in wasm-only code.
pub const STICKY_SWATCHES: &[Rgba] = &[
    STICKY_DEFAULT_FILL,          // yellow (default)
    Rgba::from_bits(0xA5D8_FFFF), // blue
    Rgba::from_bits(0xB2F2_BBFF), // green
    Rgba::from_bits(0xFFC9_C9FF), // pink
    Rgba::from_bits(0xFFD8_A8FF), // orange
    Rgba::from_bits(0xD0BF_FFFF), // purple
    Rgba::from_bits(0xDEE2_E6FF), // grey
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
/// armed. It is then obvious a click will drop a note (`toile-tools`, feedback).
/// `%23` encodes `#` inside the URI; the hotspot is the note's top-left corner.
/// Falls back to `crosshair` if the data URI is unsupported.
pub const STICKY_CURSOR: &str = "url(\"data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' width='28' height='28' viewBox='0 0 28 28'><rect x='4' y='4' width='18' height='18' fill='%23FEF08A' stroke='%23333333' stroke-width='1'/><path d='M17 22 L22 22 L22 17 Z' fill='%23E4CE5C'/></svg>\") 4 4, crosshair";

/// A CSS `cursor` value the board view asks of the canvas element over the
/// tool's own (`toile-tools`): a keyword, or an inlined SVG cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanvasCursor(Cow<'static, str>);

impl CanvasCursor {
    /// Wraps a raw value. The constructor checks nothing.
    pub const fn new(value: Cow<'static, str>) -> Self {
        Self(value)
    }

    /// Returns the raw text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The CSS `cursor` value for the canvas while `tool` is active (`toile-tools`,
/// "Cursor feedback"). A note-shaped cursor while the Sticky tool is armed, and a
/// crosshair for the other creation tools: a click visibly means "place here". A
/// grab hand for the pan-only Hand tool, and the default arrow for Select. Pure
/// so the chrome can bind it to the active-tool signal without any canvas
/// round-trip, and host-tested.
pub fn cursor_css(tool: Tool) -> CanvasCursor {
    CanvasCursor(Cow::Borrowed(match tool {
        Tool::Select => "default",
        Tool::Hand => "grab",
        Tool::Sticky(_) => STICKY_CURSOR,
        Tool::Text
        | Tool::Shape(_)
        | Tool::Pen
        | Tool::Connector
        | Tool::Draw
        | Tool::Comment
        | Tool::Laser => "crosshair",
    }))
}

/// A rotation cursor (inlined SVG data URI) shown on the rotate handle so it is
/// obvious the drag will rotate rather than resize (`toile-tools`).
///
/// Built from the shared `icondata::LuRotateCw` glyph (Lucide "rotate-cw", a
/// clockwise circular arrow). Its paths are wrapped in a full `<svg>` and
/// painted twice: a white under-stroke then a dark top-stroke. It stays legible
/// on any board colour.
///
/// The glyph's `"` are swapped for `'` so the markup survives inside the CSS
/// `url("...")`, and `#` is percent-encoded. Hotspot at the icon centre; falls
/// back to `grab` if the data URI is unsupported. Lazily built once.
pub static ROTATE_CURSOR: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    // Swap `"` for `'` (so the markup survives inside `url("...")`) and collapse all
    // whitespace to single spaces. The glyph data spans two lines, and a raw newline
    // is a control char that invalidates the whole data URI / cursor.
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

/// The Lucide "move-horizontal" glyph: a horizontal double-headed arrow, `<->`,
/// centred at 12,12 in a 0 0 24 24 box. Rotated to any angle it becomes the
/// directional resize cursor for that axis. Its `"` are swapped for `'` and its
/// whitespace collapsed so the markup survives inside the CSS `url("...")`. Built
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
fn resize_axis_base(h: Handle) -> Option<ambre_core::Radians> {
    use std::f64::consts::{FRAC_PI_2, FRAC_PI_4};
    let base = match h {
        Handle::E | Handle::W => 0.0,
        Handle::Nw | Handle::Se => FRAC_PI_4,
        Handle::N | Handle::S => FRAC_PI_2,
        Handle::Ne | Handle::Sw => 3.0 * FRAC_PI_4,
        Handle::Rotate => return None,
    };
    Some(ambre_core::Radians::new(base))
}

/// Snap a resize-axis angle (radians) to the nearest of the four native CSS resize
/// keywords. That is the crisp cursor for an axis-aligned box, and the graceful
/// fallback when a data-URI cursor is unsupported.
fn nearest_resize_keyword(a: ambre_core::Radians) -> &'static str {
    use std::f64::consts::{FRAC_PI_4, PI};
    match (a.get().rem_euclid(PI) / FRAC_PI_4).round() as i64 % 4 {
        0 => "ew-resize",
        1 => "nwse-resize",
        2 => "ns-resize",
        _ => "nesw-resize",
    }
}

/// A directional resize cursor rotated to `angle`: the `MOVE_PATHS` double
/// arrow wrapped in a `rotate(deg 12 12)` group, in whole degrees, hotspot
/// centred. It is painted twice (white under-stroke then dark top-stroke) so it
/// stays legible on any board colour. `fallback` is the native keyword shown if
/// the data URI is unsupported.
fn rotated_resize_cursor(angle: ambre_core::Radians, fallback: &str) -> String {
    let paths = MOVE_PATHS.as_str();
    let d = angle.to_degrees().rem_euclid(360.0).round() as i64;
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
/// (`toile-tools`, "Cursor feedback"). A directional double-arrow matches the
/// axis a corner or side resizes along, and a rotation cursor sits on the
/// rotate handle. `theta` is the element's rotation (radians).
///
/// For an axis-aligned element (`theta == 0`) it returns the crisp **native**
/// CSS keyword. For a rotated one it returns a **custom SVG cursor rotated to
/// the exact angle** (`resize_axis_base(h) + theta`), so the arrow tilts
/// continuously with the box. Native resize cursors are fixed OS bitmaps and
/// cannot be rotated. Pure and host-tested.
pub fn resize_cursor(h: Handle, theta: ambre_core::Radians) -> Cow<'static, str> {
    let Some(base) = resize_axis_base(h) else {
        return Cow::Borrowed(ROTATE_CURSOR.as_str());
    };
    let angle = base + theta;
    if theta == ambre_core::Radians::ZERO {
        // Axis-aligned: the native keyword is crisper than an SVG cursor.
        Cow::Borrowed(nearest_resize_keyword(angle))
    } else {
        Cow::Owned(rotated_resize_cursor(angle, nearest_resize_keyword(angle)))
    }
}

/// What the DOM says of an event target's editing host: a plain element, or
/// one whose `isContentEditable` holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Host {
    Plain,
    ContentEditable,
}

impl Host {
    /// `ContentEditable` when the DOM's `isContentEditable` holds.
    pub fn content_editable_if(editable: bool) -> Self {
        if editable {
            Host::ContentEditable
        } else {
            Host::Plain
        }
    }
}

/// Whether a keyboard event targeting an element with this tag and editing
/// host should be treated as "typing". A bare tool chord must then NOT be
/// stolen. True for form text fields (`INPUT`, `TEXTAREA`) and any
/// contenteditable host. The tag is compared case-insensitively
/// (`Element.tagName` is upper-case in HTML, but be defensive). The launcher's
/// `target_is_editable` reads `tagName` and `isContentEditable` off the event
/// target and delegates here so the decision stays pure and host-tested.
pub fn is_editable_tag(tag_name: &str, host: Host) -> bool {
    if host == Host::ContentEditable {
        return true;
    }
    tag_name.eq_ignore_ascii_case("input") || tag_name.eq_ignore_ascii_case("textarea")
}

#[cfg(test)]
mod tests {
    use super::*;
    // The chord types are the tests' business now: the module itself reads
    // every binding from the registry.
    use ambre_commands::{Key, KeyBinding, KeyChar, Modifiers};

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

    /// `TOILE-UI-409`: an id that names no tool arms no tool.
    #[test]
    fn tool_for_command_rejects_non_tool_ids() {
        // A non-tool Toile command (the Find panel opener) and an unknown id both
        // resolve to `None`, so the launcher never switches tools for them.
        assert_eq!(
            tool_for_command(StaticKey::<Commands>::new("toile.find.open")),
            None
        );
        assert_eq!(
            tool_for_command(StaticKey::<Commands>::new("nonsense")),
            None
        );
    }

    #[test]
    fn tool_keys_round_trip_key_to_command_to_tool() {
        let mut keymap = Keymap::new();
        bind_tool_keys(&mut keymap);
        // Each chord resolves to its command, and the command maps back to the
        // expected tool. That is the full key -> command -> tool wiring, no
        // DOM.
        let cases = [
            ('v', TOOL_SELECT, Tool::Select),
            ('p', TOOL_PEN, Tool::Pen),
            ('r', TOOL_SHAPE, Tool::Shape(ShapeKind::Rect)),
            ('c', TOOL_CONNECTOR, Tool::Connector),
            ('d', TOOL_DRAW, Tool::Draw),
            ('n', TOOL_STICKY, Tool::Sticky(STICKY_DEFAULT_FILL)),
            ('t', TOOL_TEXT, Tool::Text),
            ('m', TOOL_COMMENT, Tool::Comment),
            ('l', TOOL_LASER, Tool::Laser),
            ('h', TOOL_HAND, Tool::Hand),
        ];
        for (c, id, tool) in cases {
            let chord = KeyBinding::new(Modifiers::NONE, Key::Char(KeyChar::new(c)));
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
        let mut ids: Vec<StaticKey<Commands>> = ['v', 'p', 'r', 'c', 'd', 'n', 't', 'm', 'l', 'h']
            .into_iter()
            .filter_map(|c| {
                keymap.resolve(KeyBinding::new(Modifiers::NONE, Key::Char(KeyChar::new(c))))
            })
            .collect();
        assert_eq!(ids.len(), TOOL_COMMANDS.len());
        ids.sort_by_key(|id| id.as_str());
        ids.dedup();
        assert_eq!(ids.len(), TOOL_COMMANDS.len(), "tool chords collide");
    }

    #[test]
    fn cursor_reflects_the_active_tool() {
        assert_eq!(cursor_css(Tool::Select).0, "default");
        assert_eq!(cursor_css(Tool::Hand).0, "grab");
        assert_eq!(
            cursor_css(Tool::Sticky(STICKY_DEFAULT_FILL)).0,
            STICKY_CURSOR
        );
        assert!(STICKY_CURSOR.starts_with("url("));
        assert_eq!(cursor_css(Tool::Text).0, "crosshair");
        assert_eq!(cursor_css(Tool::Shape(ShapeKind::Rect)).0, "crosshair");
        assert_eq!(cursor_css(Tool::Pen).0, "crosshair");
        assert_eq!(cursor_css(Tool::Connector).0, "crosshair");
        assert_eq!(cursor_css(Tool::Draw).0, "crosshair");
    }

    #[test]
    fn resize_cursor_uses_native_keywords_when_axis_aligned() {
        // Axis-aligned element (theta = 0): each handle keeps its native keyword.
        assert_eq!(
            resize_cursor(Handle::Nw, ambre_core::Radians::ZERO).as_ref(),
            "nwse-resize"
        );
        assert_eq!(
            resize_cursor(Handle::Se, ambre_core::Radians::ZERO).as_ref(),
            "nwse-resize"
        );
        assert_eq!(
            resize_cursor(Handle::Ne, ambre_core::Radians::ZERO).as_ref(),
            "nesw-resize"
        );
        assert_eq!(
            resize_cursor(Handle::Sw, ambre_core::Radians::ZERO).as_ref(),
            "nesw-resize"
        );
        assert_eq!(
            resize_cursor(Handle::N, ambre_core::Radians::ZERO).as_ref(),
            "ns-resize"
        );
        assert_eq!(
            resize_cursor(Handle::S, ambre_core::Radians::ZERO).as_ref(),
            "ns-resize"
        );
        assert_eq!(
            resize_cursor(Handle::E, ambre_core::Radians::ZERO).as_ref(),
            "ew-resize"
        );
        assert_eq!(
            resize_cursor(Handle::W, ambre_core::Radians::ZERO).as_ref(),
            "ew-resize"
        );
        assert_eq!(
            resize_cursor(Handle::Rotate, ambre_core::Radians::ZERO).as_ref(),
            ROTATE_CURSOR.as_str()
        );
        assert!(ROTATE_CURSOR.starts_with("url(\""));
        assert!(ROTATE_CURSOR.ends_with("grab"));
        // The Lucide glyph made it in, and its `"` were swapped for `'`. The only
        // double quotes left are the two wrapping the `url(...)` value.
        assert!(ROTATE_CURSOR.contains("<path"));
        assert_eq!(ROTATE_CURSOR.matches('"').count(), 2);
    }

    #[test]
    fn resize_cursor_rotates_continuously_when_the_element_is_rotated() {
        use std::f64::consts::{FRAC_PI_2, FRAC_PI_6};
        // A rotated element yields a custom SVG cursor tilted to the exact angle,
        // not a native keyword. The E handle at a quarter turn points along 90 deg.
        let c = resize_cursor(Handle::E, ambre_core::Radians::new(FRAC_PI_2));
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
        let c = resize_cursor(Handle::E, ambre_core::Radians::new(FRAC_PI_6));
        assert!(c.contains("rotate(30 12 12)"), "{c}");
        // The rotate handle stays the rotation cursor regardless of angle.
        assert_eq!(
            resize_cursor(Handle::Rotate, ambre_core::Radians::new(FRAC_PI_6)).as_ref(),
            ROTATE_CURSOR.as_str()
        );
    }

    /// `TOILE-UI-410`: the note swatches are distinct opaque colours, the
    /// default colour first.
    #[test]
    fn sticky_swatches_lead_with_the_default_and_are_distinct() {
        assert_eq!(STICKY_SWATCHES.first(), Some(&STICKY_DEFAULT_FILL));
        let mut seen = STICKY_SWATCHES.to_vec();
        seen.sort_unstable_by_key(|c| c.bits());
        seen.dedup();
        assert_eq!(seen.len(), STICKY_SWATCHES.len(), "swatch colours collide");
        // Every swatch is fully opaque (alpha = 0xFF), so a note is never
        // accidentally transparent.
        assert!(STICKY_SWATCHES.iter().all(|c| c.channels()[3] == 0xFF));
    }

    #[test]
    fn editable_tag_guards_text_entry() {
        assert!(is_editable_tag("INPUT", Host::Plain));
        assert!(is_editable_tag("textarea", Host::Plain)); // case-insensitive
        assert!(is_editable_tag("DIV", Host::ContentEditable)); // contenteditable host
        assert!(!is_editable_tag("DIV", Host::Plain));
        assert!(!is_editable_tag("CANVAS", Host::Plain));
    }
}
