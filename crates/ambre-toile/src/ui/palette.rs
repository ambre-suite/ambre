// ambre-toile/src/ui/palette.rs
// ============================================================================
// Command surface for the Toile tool palette / primary toolbar (toile-ui
// SS5quinquies.A). The host-visible half: the `CommandId`s the palette
// contributes, the pure `CommandId -> Tool` map, the bare-key tool chords, and
// the pure focus-guard predicate the launcher applies before stealing a key.
//
// Kept in the always-compiled `ui` module (not the wasm-only `chrome`) so it is
// host-unit-tested (SS10), exactly like `ui::find` and `ui::menu`. The wasm-only
// Leptos component that renders these (`chrome::ToolPalette`) consumes them.
//
// The active tool is UI-only, canvas-local state (toile-ui SS11bis, "Local
// view"): it never becomes a `ToileCommand` and never reaches the Logic thread.
// A tool `CommandId` is therefore NOT mapped through `command_to_app`; the
// palette (and the keyboard) write a shared `RwSignal<Tool>` the view applies.
// ============================================================================

use ambre_bus::app::toile::ShapeKind;
use ambre_commands::{CommandId, Key, KeyBinding, Keymap, Modifiers};

use super::tool::Tool;

// -- Command identities --------------------------------------------------------
// String-keyed ids (`ambre-commands`), one per tool. The identity is the string,
// so the shell can round-trip an id through a signal without carrying the
// `Command` table.

/// Select / move / resize / rotate / marquee (SS5).
pub const TOOL_SELECT: CommandId = CommandId("toile.tool.select");
/// Vector pen (Bezier path).
pub const TOOL_PEN: CommandId = CommandId("toile.tool.pen");
/// Shape drag-rect (defaults to a rectangle; the kind sub-option is a local
/// palette signal folded into `CreateElement`, SS5quinquies.A).
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
/// `TOOL_SHAPE` selects the rectangle by default (SS5quinquies.A).
pub fn tool_for_command(id: CommandId) -> Option<Tool> {
    match id {
        TOOL_SELECT => Some(Tool::Select),
        TOOL_PEN => Some(Tool::Pen),
        TOOL_SHAPE => Some(Tool::Shape(ShapeKind::Rect)),
        TOOL_CONNECTOR => Some(Tool::Connector),
        TOOL_DRAW => Some(Tool::Draw),
        TOOL_STICKY => Some(Tool::Sticky),
        TOOL_TEXT => Some(Tool::Text),
        TOOL_HAND => Some(Tool::Hand),
        _ => None,
    }
}

/// Bind the bare-key tool chords into the launcher keymap. These are the de-facto
/// whiteboard letters (V/P/R/C/D/N/T/H); SS5quinquies.A promises the palette click
/// and the keyboard resolve to the same tool selection. The launcher guards them
/// with [`is_editable_tag`] so a letter typed into an inline editor is not stolen.
pub fn bind_tool_keys(keymap: &mut Keymap) {
    keymap.bind(
        KeyBinding::new(Modifiers::NONE, Key::char('v')),
        TOOL_SELECT,
    );
    keymap.bind(KeyBinding::new(Modifiers::NONE, Key::char('p')), TOOL_PEN);
    keymap.bind(KeyBinding::new(Modifiers::NONE, Key::char('r')), TOOL_SHAPE);
    keymap.bind(
        KeyBinding::new(Modifiers::NONE, Key::char('c')),
        TOOL_CONNECTOR,
    );
    keymap.bind(KeyBinding::new(Modifiers::NONE, Key::char('d')), TOOL_DRAW);
    keymap.bind(
        KeyBinding::new(Modifiers::NONE, Key::char('n')),
        TOOL_STICKY,
    );
    keymap.bind(KeyBinding::new(Modifiers::NONE, Key::char('t')), TOOL_TEXT);
    keymap.bind(KeyBinding::new(Modifiers::NONE, Key::char('h')), TOOL_HAND);
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
        assert_eq!(tool_for_command(TOOL_STICKY), Some(Tool::Sticky));
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
            ('n', TOOL_STICKY, Tool::Sticky),
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
    fn editable_tag_guards_text_entry() {
        assert!(is_editable_tag("INPUT", false));
        assert!(is_editable_tag("textarea", false)); // case-insensitive
        assert!(is_editable_tag("DIV", true)); // contenteditable host
        assert!(!is_editable_tag("DIV", false));
        assert!(!is_editable_tag("CANVAS", false));
    }
}
